//! oil.nvim style file explorer: a directory listing in an editable buffer. `ret` opens the
//! entry under the cursor, `-` opens the parent directory and `:write` applies the edits.
//! Entries are tracked through the edit history: an edited line renames its entry, a removed
//! line deletes it and a new line creates one (a trailing `/` makes it a directory).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::key;

/// Opens an editable listing of `dir` in the current view with the cursor on the `focus` entry.
pub fn open(editor: &mut Editor, dir: PathBuf, focus: Option<&str>) {
    let dir = helix_stdx::path::normalize(dir);
    let entries = match ui::directory_content(&dir, editor) {
        Ok(entries) => entries,
        Err(err) => return editor.set_error(format!("{}: {err}", dir.display())),
    };
    let mut listing = String::new();
    for (path, is_dir) in entries {
        listing.push_str(&path.strip_prefix(&dir).unwrap_or(&path).to_string_lossy());
        if is_dir {
            listing.push('/');
        }
        listing.push('\n');
    }
    let line = focus
        .and_then(|focus| listing.lines().position(|line| line == focus))
        .unwrap_or(0);

    let mut doc = Document::from(
        Rope::from(listing.as_str()),
        None,
        editor.config.clone(),
        editor.syn_loader.clone(),
    );
    doc.explorer = Some((dir, listing));
    update_icons(&mut doc);
    // An unmodified explorer buffer we replace here is dropped by `switch` like an empty scratch
    // buffer; a modified one stays open so its edits can still be written.
    editor.new_file_from_document(Action::Replace, doc);
    let (view, doc) = current!(editor);
    let pos = doc.text().line_to_char(line);
    doc.set_selection(view.id, Selection::point(pos));
}

pub(crate) fn register_hooks() {
    use helix_view::events::DocumentDidChange;
    helix_event::register_hook!(move |event: &mut DocumentDidChange<'_>| {
        update_icons(event.doc);
        Ok(())
    });
}

/// Recomputes the icons shown before the entries of an explorer buffer.
fn update_icons(doc: &mut Document) {
    use helix_core::text_annotations::InlineAnnotation;

    if doc.explorer.is_none() {
        return;
    }
    let text = doc.text().slice(..);
    let mut layers: Vec<(Option<helix_core::syntax::Highlight>, Vec<InlineAnnotation>)> =
        Vec::new();
    for (line, content) in text.lines().enumerate() {
        let name: Cow<str> = content.into();
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let (icon, color) = icon(name);
        let highlight = color.map(|[r, g, b]| helix_view::Theme::rgb_highlight(r, g, b));
        let icon = InlineAnnotation::new(text.line_to_char(line), format!("{icon} "));
        match layers.iter_mut().find(|(layer, _)| *layer == highlight) {
            Some((_, icons)) => icons.push(icon),
            None => layers.push((highlight, vec![icon])),
        }
    }
    doc.explorer_icons = layers;
}

/// The Nerd Font icon of an entry and its color, `None` for directories (themed instead).
fn icon(name: &str) -> (&'static str, Option<[u8; 3]>) {
    if name.ends_with('/') {
        return ("\u{f07b}", None);
    }
    let name = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let by_name = match name.as_str() {
        "cargo.toml" | "cargo.lock" => Some(("\u{e7a8}", [0xde, 0xa5, 0x84])),
        "dockerfile" | "containerfile" => Some(("\u{f308}", [0x45, 0x8e, 0xe6])),
        "makefile" | "justfile" => Some(("\u{e779}", [0x6d, 0x80, 0x86])),
        "license" | "license.md" | "license.txt" => Some(("\u{e60a}", [0xd0, 0xbf, 0x41])),
        ".gitignore" | ".gitattributes" | ".gitmodules" => {
            Some(("\u{e702}", [0xf5, 0x4d, 0x27]))
        }
        _ => None,
    };
    let (icon, color) = by_name.unwrap_or_else(|| {
        match name.rsplit_once('.').map_or("", |(_, extension)| extension) {
            "rs" => ("\u{e7a8}", [0xde, 0xa5, 0x84]),
            "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" => ("\u{e615}", [0x6d, 0x80, 0x86]),
            "json" | "jsonc" | "json5" => ("\u{e60b}", [0xcb, 0xcb, 0x41]),
            "md" | "markdown" => ("\u{f48a}", [0xdd, 0xdd, 0xdd]),
            "txt" => ("\u{f15c}", [0x89, 0xe0, 0x51]),
            "js" | "mjs" | "cjs" | "jsx" => ("\u{e74e}", [0xcb, 0xcb, 0x41]),
            "ts" | "mts" | "cts" | "tsx" => ("\u{e628}", [0x51, 0x9a, 0xba]),
            "py" => ("\u{e606}", [0xff, 0xbc, 0x03]),
            "go" => ("\u{e627}", [0x51, 0x9a, 0xba]),
            "c" => ("\u{e61e}", [0x59, 0x9e, 0xff]),
            "h" | "hpp" | "hh" => ("\u{f0fd}", [0xa0, 0x74, 0xc4]),
            "cpp" | "cc" | "cxx" => ("\u{e61d}", [0x51, 0x9a, 0xba]),
            "java" => ("\u{e738}", [0xcc, 0x3e, 0x44]),
            "kt" | "kts" => ("\u{e634}", [0x7f, 0x52, 0xff]),
            "swift" => ("\u{e755}", [0xe3, 0x79, 0x33]),
            "rb" => ("\u{e739}", [0x70, 0x15, 0x16]),
            "php" => ("\u{e73d}", [0xa0, 0x74, 0xc4]),
            "lua" => ("\u{e620}", [0x51, 0xa0, 0xcf]),
            "zig" => ("\u{e6a9}", [0xf6, 0x9a, 0x1b]),
            "hs" => ("\u{e777}", [0xa0, 0x74, 0xc4]),
            "ex" | "exs" => ("\u{e62d}", [0xa0, 0x74, 0xc4]),
            "dart" => ("\u{e798}", [0x03, 0x58, 0x9c]),
            "nix" => ("\u{f313}", [0x7e, 0xba, 0xe4]),
            "sh" | "bash" | "zsh" | "fish" => ("\u{e795}", [0x89, 0xe0, 0x51]),
            "vim" => ("\u{e62b}", [0x01, 0x98, 0x33]),
            "html" | "htm" => ("\u{e736}", [0xe4, 0x4d, 0x26]),
            "css" => ("\u{e749}", [0x42, 0xa5, 0xf5]),
            "scss" | "sass" => ("\u{e603}", [0xf5, 0x53, 0x85]),
            "xml" => ("\u{f05c0}", [0xe3, 0x79, 0x33]),
            "sql" => ("\u{e706}", [0xda, 0xd8, 0xd8]),
            "lock" => ("\u{f023}", [0xbb, 0xbb, 0xbb]),
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" => {
                ("\u{f1c5}", [0xa0, 0x74, 0xc4])
            }
            "mp3" | "wav" | "flac" | "ogg" => ("\u{f001}", [0x66, 0xd8, 0xef]),
            "mp4" | "mkv" | "webm" | "mov" => ("\u{f03d}", [0xfd, 0x97, 0x1f]),
            "pdf" => ("\u{f1c1}", [0xb3, 0x0b, 0x00]),
            "zip" | "tar" | "gz" | "xz" | "zst" | "7z" | "rar" => ("\u{f410}", [0xec, 0xa5, 0x17]),
            _ => ("\u{f15b}", [0x6d, 0x80, 0x86]),
        }
    });
    (icon, Some(color))
}

/// Handles `Enter` in an explorer buffer when the keymap leaves it unbound. Returns whether `key`
/// was used.
pub fn handle_key(editor: &mut Editor, mode: Mode, key: KeyEvent) -> bool {
    if mode != Mode::Normal {
        return false;
    }
    let Some((dir, _)) = doc!(editor).explorer.clone() else {
        return false;
    };
    match key {
        key!(Enter) => {
            let name = current_line(editor);
            if name.is_empty() {
                return true;
            }
            let path = dir.join(&name);
            if path.is_dir() {
                open(editor, path, None);
            } else if let Err(err) = editor.open(&path, Action::Replace) {
                editor.set_error(format!("{}: {err}", path.display()));
            }
        }
        _ => return false,
    }
    true
}

/// Applies the edits of all modified explorer buffers to the file system, asking first if any
/// entry gets deleted. They are applied together so that an entry deleted in one directory and
/// added under the same name in another is moved there.
pub fn write(cx: &mut compositor::Context) {
    let (view, doc) = current!(cx.editor);
    if doc.explorer.is_none() {
        return;
    }
    let current = doc.id();
    doc.append_changes_to_history(view);
    let docs: Vec<DocumentId> = cx
        .editor
        .documents()
        .filter(|doc| doc.explorer.is_some() && (doc.id() == current || doc.is_modified()))
        .map(|doc| doc.id())
        .collect();

    let mut ops = Ops::default();
    for id in &docs {
        let doc = doc_mut!(cx.editor, id);
        if let Some((dir, listing)) = &doc.explorer {
            plan(dir, listing, doc.history.get_mut().path_from_root(), &mut ops);
        }
    }
    ops.deletes.retain(|from| {
        let Some(i) = ops.creates.iter().position(|(to, is_dir)| {
            to.file_name() == from.file_name() && *is_dir == from.is_dir()
        }) else {
            return true;
        };
        let (to, _) = ops.creates.remove(i);
        ops.renames.push((from.clone(), to));
        false
    });

    if ops.deletes.is_empty() {
        return apply(cx.editor, &docs, &ops);
    }
    let deletes: Vec<_> = ops
        .deletes
        .iter()
        .map(|path| helix_stdx::path::get_relative_path(path).display().to_string())
        .collect();
    let label = format!("delete {}? (y/n):", deletes.join(", "));
    cx.jobs.callback(async move {
        let call: job::Callback =
            job::Callback::EditorCompositor(Box::new(move |_editor, compositor| {
                let prompt = ui::Prompt::new(
                    label.into(),
                    None,
                    ui::completers::none,
                    move |cx: &mut compositor::Context, input: &str, event: PromptEvent| {
                        if event == PromptEvent::Validate && input == "y" {
                            apply(cx.editor, &docs, &ops);
                        }
                    },
                );
                compositor.push(Box::new(prompt))
            }));
        Ok(call)
    });
}

/// Re-lists the current explorer buffer, discarding its edits.
pub fn reload(editor: &mut Editor) {
    let Some((dir, _)) = doc!(editor).explorer.clone() else {
        return;
    };
    doc_mut!(editor).reset_modified();
    let focus = current_line(editor);
    open(editor, dir, Some(&focus));
}

fn current_line(editor: &Editor) -> String {
    let (view, doc) = current_ref!(editor);
    let text = doc.text().slice(..);
    let line = text.char_to_line(doc.selection(view.id).primary().cursor(text));
    text.line(line).to_string().trim().to_string()
}

#[derive(Default)]
struct Ops {
    /// Paths to create, and whether each is a directory.
    creates: Vec<(PathBuf, bool)>,
    renames: Vec<(PathBuf, PathBuf)>,
    deletes: Vec<PathBuf>,
}

/// Adds the file operations for the edits of the `listing` of `dir` to `ops`, following each
/// entry through the `transactions` of the edit history (see [`step`]). An entry whose name is
/// also on a line that holds no entry (e.g. it was cut and pasted) counts as moved there, so it
/// is neither deleted nor renamed.
fn plan<'a>(
    dir: &Path,
    listing: &str,
    transactions: impl Iterator<Item = &'a Transaction>,
    ops: &mut Ops,
) {
    fn key(name: &str) -> &str {
        name.trim_end_matches('/')
    }
    let old_names: Vec<&str> = listing.lines().collect();
    let mut text = Rope::from(listing);
    // The entry each line holds; the line after the final newline holds none.
    let mut ids: Vec<Option<usize>> = (0..old_names.len()).map(Some).chain([None]).collect();
    for transaction in transactions {
        let before = text.clone();
        transaction.apply(&mut text);
        ids = step(&ids, &before, transaction.changes(), &text);
    }

    let new_names: Vec<String> = text
        .lines()
        .map(|line| line.to_string().trim().to_string())
        .collect();
    let old_set: HashSet<&str> = old_names.iter().map(|name| key(name)).collect();
    let untracked: HashSet<&str> = new_names
        .iter()
        .zip(&ids)
        .filter(|(_, id)| id.is_none())
        .map(|(name, _)| key(name))
        .collect();
    let mut lines = vec![None; old_names.len()];
    for (line, id) in ids.iter().enumerate() {
        if let Some(id) = id {
            lines[*id] = Some(line);
        }
    }

    let path = |name: &str| dir.join(key(name));
    for (old, line) in old_names.iter().zip(lines) {
        let new = line
            .map(|line| new_names[line].as_str())
            .filter(|name| !name.is_empty());
        let moved = untracked.contains(key(old));
        match new {
            None if !moved => ops.deletes.push(path(old)),
            Some(new) if key(new) != key(old) && !moved => {
                ops.renames.push((path(old), path(new)))
            }
            Some(new) if key(new) != key(old) && !old_set.contains(key(new)) => {
                ops.creates.push((path(new), new.ends_with('/')))
            }
            _ => {}
        }
    }
    for (name, id) in new_names.iter().zip(&ids) {
        if id.is_none() && !name.is_empty() && !old_set.contains(key(name)) {
            ops.creates.push((path(name), name.ends_with('/')));
        }
    }
}

/// Carries the entry `ids` of the lines of `before` over to the lines of `after`. A line keeps
/// its entry on the line its first surviving char (newline included) ends up on. Lines removed
/// by the edit pass their entries in order to the new lines it inserted text into, so replacing
/// a whole line in one edit (`xc`) is a rename, while deleting it and inserting a line in
/// separate edits is not.
fn step(
    ids: &[Option<usize>],
    before: &Rope,
    changes: &helix_core::ChangeSet,
    after: &Rope,
) -> Vec<Option<usize>> {
    use helix_core::Operation;

    // Retained runs as (old pos, new pos, len), inserted text as new ranges.
    let mut runs = Vec::new();
    let mut inserts = Vec::new();
    let (mut p, mut q) = (0, 0);
    for op in changes.changes() {
        match op {
            Operation::Retain(n) => {
                runs.push((p, q, *n));
                p += n;
                q += n;
            }
            Operation::Delete(n) => p += n,
            Operation::Insert(text) => {
                let len = text.chars().count();
                if len > 0 {
                    inserts.push(q..q + len);
                }
                q += len;
            }
        }
    }
    // An empty change set retains everything implicitly.
    if p < before.len_chars() {
        runs.push((p, q, before.len_chars() - p));
    }

    let mut next = vec![None; after.len_lines()];
    let mut dead = vec![true; ids.len()];
    for (p, q, n) in runs {
        let mut line = before.char_to_line(p);
        while line < ids.len() && before.line_to_char(line) < p + n {
            if std::mem::replace(&mut dead[line], false) {
                let first = before.line_to_char(line).max(p);
                let target = after.char_to_line(q + first - p);
                if next[target].is_none() {
                    next[target] = ids[line];
                }
            }
            line += 1;
        }
    }
    let removed = (0..ids.len()).filter(|&line| dead[line] && ids[line].is_some());
    let mut inserted: Vec<usize> = inserts
        .into_iter()
        .flat_map(|range| after.char_to_line(range.start)..=after.char_to_line(range.end - 1))
        .filter(|&line| next[line].is_none())
        .collect();
    inserted.dedup();
    for (old, new) in removed.zip(inserted) {
        next[new] = ids[old];
    }
    next
}

fn apply(editor: &mut Editor, docs: &[DocumentId], ops: &Ops) {
    let result = run(editor, ops);
    // Re-list the current directory so the buffer shows the actual result, also after a
    // failure. The other written explorer buffers are stale, so close them.
    let current = doc!(editor).id();
    for &id in docs {
        if id != current {
            let _ = editor.close_document(id, true);
        }
    }
    if docs.contains(&current) {
        reload(editor);
    }
    if let Err(err) = result {
        editor.set_error(format!("{err:#}"));
    }
}

fn run(editor: &mut Editor, ops: &Ops) -> anyhow::Result<()> {
    // Renames onto the source of another rename (swaps, cycles) first move their source out
    // of the way, and finish after the other renames.
    let sources: HashSet<&Path> = ops.renames.iter().map(|(from, _)| from.as_path()).collect();
    let (cycles, direct): (Vec<_>, Vec<_>) = ops
        .renames
        .iter()
        .partition(|(_, to)| sources.contains(to.as_path()));
    let mut staged = Vec::new();
    for (from, to) in cycles {
        let mut tmp = from.clone().into_os_string();
        tmp.push(".~explorer~");
        let tmp = PathBuf::from(tmp);
        rename(editor, from, &tmp)?;
        staged.push((tmp, to));
    }
    for (from, to) in direct {
        rename(editor, from, to)?;
    }
    for (from, to) in &staged {
        rename(editor, from, to)?;
    }

    for (path, is_dir) in &ops.creates {
        let created = if *is_dir {
            fs::create_dir_all(path)
        } else {
            path.parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|_| {
                    fs::File::options()
                        .create(true)
                        .append(true)
                        .open(path)
                        .map(drop)
                })
        };
        created.with_context(|| format!("failed to create {}", path.display()))?;
    }

    for path in &ops.deletes {
        if path.is_dir() && !path.is_symlink() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .with_context(|| format!("failed to delete {}", path.display()))?;
    }
    Ok(())
}

fn rename(editor: &mut Editor, from: &Path, to: &Path) -> anyhow::Result<()> {
    let context = || format!("failed to rename {} to {}", from.display(), to.display());
    if to.exists() {
        return Err(anyhow!("{} already exists", to.display())).with_context(context);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(context)?;
    }
    editor.move_path(from, to).with_context(context)
}
