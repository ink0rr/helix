//! LSP semantic tokens layered on top of tree-sitter highlights (like Zed's "combined" mode).
//!
//! Tokens are mapped to theme scopes and rendered as an overlay, so tree-sitter highlighting stays
//! in place wherever the server sends nothing or the token has no mapping.

use std::{collections::HashSet, time::Duration};

use helix_core::{
    syntax::{
        config::{LanguageServerFeature, SemanticTokenRule},
        Highlight,
    },
    Assoc,
};
use helix_event::{cancelable_future, register_hook};
use helix_lsp::{lsp, util::lsp_pos_to_pos, LanguageServerId};
use helix_view::{
    document::{SemanticToken, SemanticTokens},
    events::{
        ConfigDidChange, DocumentDidChange, DocumentDidOpen, LanguageServerExited,
        LanguageServerInitialized,
    },
    graphics::{Color, Modifier},
    handlers::{lsp::SemanticTokensEvent, Handlers},
    theme::Theme,
    DocumentId, Editor,
};
use tokio::time::Instant;

use crate::job;

#[derive(Default)]
pub(super) struct SemanticTokensHandler {
    docs: HashSet<DocumentId>,
}

const DOCUMENT_CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);

impl helix_event::AsyncHook for SemanticTokensHandler {
    type Event = SemanticTokensEvent;

    fn handle_event(&mut self, event: Self::Event, _timeout: Option<Instant>) -> Option<Instant> {
        let SemanticTokensEvent(doc_id) = event;
        self.docs.insert(doc_id);
        Some(Instant::now() + DOCUMENT_CHANGE_DEBOUNCE)
    }

    fn finish_debounce(&mut self) {
        let docs = std::mem::take(&mut self.docs);

        job::dispatch_blocking(move |editor, _compositor| {
            for doc in docs {
                request_semantic_tokens(editor, doc);
            }
        });
    }
}

/// Re-request tokens for every document attached to the given server
/// (`workspace/semanticTokens/refresh`).
pub fn request_semantic_tokens_for_language_server(
    editor: &mut Editor,
    server_id: LanguageServerId,
) {
    let doc_ids: Vec<_> = editor
        .documents()
        .filter(|doc| doc.supports_language_server(server_id))
        .map(|doc| doc.id())
        .collect();

    for doc_id in doc_ids {
        request_semantic_tokens(editor, doc_id);
    }
}

fn request_semantic_tokens(editor: &mut Editor, doc_id: DocumentId) {
    if !editor.config().lsp.semantic_tokens {
        return;
    }
    let Some(doc) = editor.document_mut(doc_id) else {
        return;
    };

    // Like syntax highlighting, only the first server supporting the feature is used.
    let Some(language_server) = doc
        .language_servers_with_feature(LanguageServerFeature::SemanticTokens)
        .next()
    else {
        return;
    };
    let Some(legend) = language_server.semantic_tokens_legend().cloned() else {
        return;
    };
    let Some(future) =
        language_server.text_document_semantic_tokens_full(doc.identifier(), None)
    else {
        return;
    };
    let offset_encoding = language_server.offset_encoding();
    let rules = doc
        .language_config()
        .map(|config| compile_rules(&config.semantic_token_rules, &legend))
        .unwrap_or_default();
    let text = doc.text().clone();
    let cancel = doc.semantic_tokens_controller.restart();

    tokio::spawn(async move {
        let data = match cancelable_future(future, &cancel).await {
            Some(Ok(Some(lsp::SemanticTokensResult::Tokens(tokens)))) => tokens.data,
            Some(Ok(Some(lsp::SemanticTokensResult::Partial(tokens)))) => tokens.data,
            Some(Ok(None)) => Vec::new(),
            Some(Err(err)) => {
                log::error!("semantic tokens request failed: {err}");
                return;
            }
            None => return,
        };

        let mut tokens = Vec::with_capacity(data.len());
        let (mut line, mut character) = (0, 0);
        for token in data {
            if token.delta_line == 0 {
                character += token.delta_start;
            } else {
                line += token.delta_line;
                character = token.delta_start;
            }
            let Some(token_type) = legend.token_types.get(token.token_type as usize) else {
                continue;
            };
            let scope = standard_scope(&legend, token_type.as_str(), token.token_modifiers_bitset);
            let start = lsp::Position::new(line, character);
            let end = lsp::Position::new(line, character + token.length);
            let (Some(start), Some(end)) = (
                lsp_pos_to_pos(&text, start, offset_encoding),
                lsp_pos_to_pos(&text, end, offset_encoding),
            ) else {
                continue;
            };
            if start < end {
                let (mut rule_fg, mut rule_modifiers) = (None, Modifier::empty());
                for rule in rules.iter().filter(|rule| rule.matches(&token)) {
                    rule_fg = rule.fg.or(rule_fg);
                    rule_modifiers |= rule.style_modifiers;
                }
                tokens.push(SemanticToken {
                    start,
                    end,
                    token_type: token.token_type,
                    modifiers: token.token_modifiers_bitset,
                    scope,
                    rule_fg,
                    rule_modifiers: (!rule_modifiers.is_empty())
                        .then(|| Theme::modifier_highlight(rule_modifiers)),
                });
            }
        }
        let semantic_tokens = SemanticTokens {
            type_scopes: legend
                .token_types
                .iter()
                .map(|token_type| format!("lsp.type.{}", token_type.as_str()))
                .collect(),
            modifier_scopes: legend
                .token_modifiers
                .iter()
                .take(32)
                .map(|modifier| format!("lsp.mod.{}", modifier.as_str()))
                .collect(),
            tokens,
        };

        job::dispatch(move |editor, _| {
            // The document changed while tokens were being converted.
            if cancel.is_canceled() {
                return;
            }
            if let Some(doc) = editor.documents.get_mut(&doc_id) {
                doc.semantic_tokens = semantic_tokens;
            }
        })
        .await;
    });
}

/// A `semantic-token-rules` entry resolved against a server's legend.
struct Rule {
    token_type: Option<u32>,
    /// Bitset of required token modifiers.
    token_modifiers: u32,
    fg: Option<Highlight>,
    style_modifiers: Modifier,
}

impl Rule {
    fn matches(&self, token: &lsp::SemanticToken) -> bool {
        self.token_type.is_none_or(|t| t == token.token_type)
            && token.token_modifiers_bitset & self.token_modifiers == self.token_modifiers
    }
}

/// Rules naming a type or modifier the server's legend lacks can never match and are dropped.
fn compile_rules(rules: &[SemanticTokenRule], legend: &lsp::SemanticTokensLegend) -> Vec<Rule> {
    rules
        .iter()
        .filter_map(|rule| {
            let token_type = match &rule.token_type {
                Some(name) => Some(
                    legend
                        .token_types
                        .iter()
                        .position(|token_type| token_type.as_str() == name)?
                        as u32,
                ),
                None => None,
            };
            let mut token_modifiers = 0;
            for name in &rule.token_modifiers {
                let i = legend
                    .token_modifiers
                    .iter()
                    .take(32)
                    .position(|modifier| modifier.as_str() == name)?;
                token_modifiers |= 1 << i;
            }
            let fg = match rule.fg.as_deref().map(Color::from_hex) {
                None => None,
                Some(Ok(Color::Rgb(r, g, b))) => Some(Theme::rgb_highlight(r, g, b)),
                Some(_) => {
                    log::warn!("semantic-token-rules: invalid hex color {:?}", rule.fg);
                    None
                }
            };
            let mut style_modifiers = Modifier::empty();
            for modifier in &rule.modifiers {
                match modifier.parse::<Modifier>() {
                    Ok(modifier) => style_modifiers |= modifier,
                    Err(err) => log::warn!("semantic-token-rules: {err}: {modifier}"),
                }
            }
            Some(Rule {
                token_type,
                token_modifiers,
                fg,
                style_modifiers,
            })
        })
        .collect()
}

/// Maps standard (and common rust-analyzer) token types to Helix scopes. Non-standard types are
/// styled by the theme's `lsp.type.<name>` scopes or the language's `semantic-token-rules`.
fn standard_scope(
    legend: &lsp::SemanticTokensLegend,
    token_type: &str,
    modifiers: u32,
) -> Option<&'static str> {
    let has = |modifier: &str| {
        legend
            .token_modifiers
            .iter()
            .take(32)
            .enumerate()
            .any(|(i, m)| modifiers & (1 << i) != 0 && m.as_str() == modifier)
    };

    let scope = match token_type {
        "namespace" => "namespace",
        "type" | "class" | "struct" | "interface" | "typeAlias" | "union" => {
            if has("defaultLibrary") {
                "type.builtin"
            } else {
                "type"
            }
        }
        "builtinType" => "type.builtin",
        "enum" => "type.enum",
        "typeParameter" => "type.parameter",
        "enumMember" => "constructor",
        "parameter" => "variable.parameter",
        "variable" if has("constant") || (has("readonly") && has("static")) => "constant",
        "variable" if has("defaultLibrary") => "variable.builtin",
        "variable" => "variable",
        "const" | "constParameter" => "constant",
        "selfKeyword" => "variable.builtin",
        "property" => "variable.other.member",
        "function" if has("defaultLibrary") => "function.builtin",
        "function" => "function",
        "method" => "function.method",
        "macro" => "function.macro",
        "decorator" => "attribute",
        "label" => "label",
        "keyword" => "keyword",
        "modifier" => "keyword.storage.modifier",
        "comment" if has("documentation") => "comment.block.documentation",
        "comment" => "comment",
        "string" => "string",
        "regexp" => "string.regexp",
        "number" => "constant.numeric",
        "operator" => "operator",
        _ => return None,
    };
    Some(scope)
}

pub(super) fn register_hooks(handlers: &Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        request_semantic_tokens(event.editor, event.doc);
        Ok(())
    });

    let tx = handlers.semantic_tokens.clone();
    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        // Keep stale tokens roughly in place until the server answers.
        event
            .changes
            .update_positions(event.doc.semantic_tokens.tokens.iter_mut().flat_map(|token| {
                [(&mut token.start, Assoc::After), (&mut token.end, Assoc::Before)]
            }));

        if !event.ghost_transaction {
            event.doc.semantic_tokens_controller.cancel();
            helix_event::send_blocking(&tx, SemanticTokensEvent(event.doc.id()));
        }

        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerInitialized<'_>| {
        request_semantic_tokens_for_language_server(event.editor, event.server_id);
        Ok(())
    });

    register_hook!(move |event: &mut LanguageServerExited<'_>| {
        let doc_ids: Vec<_> = event
            .editor
            .documents_mut()
            .filter(|doc| doc.supports_language_server(event.server_id))
            .map(|doc| {
                doc.semantic_tokens = SemanticTokens::default();
                doc.id()
            })
            .collect();

        // Another server may provide tokens now.
        for doc_id in doc_ids {
            request_semantic_tokens(event.editor, doc_id);
        }
        Ok(())
    });

    register_hook!(move |event: &mut ConfigDidChange<'_>| {
        match (event.old.lsp.semantic_tokens, event.new.lsp.semantic_tokens) {
            (false, true) => {
                let doc_ids: Vec<_> = event.editor.documents().map(|doc| doc.id()).collect();
                for doc_id in doc_ids {
                    request_semantic_tokens(event.editor, doc_id);
                }
            }
            (true, false) => {
                for doc in event.editor.documents_mut() {
                    doc.semantic_tokens_controller.cancel();
                    doc.semantic_tokens = SemanticTokens::default();
                }
            }
            _ => {}
        }
        Ok(())
    });
}
