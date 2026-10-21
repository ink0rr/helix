//! Minimap and scrollbar drawn on the right edge of each editor view.

use helix_core::{
    diagnostic::Severity,
    syntax::{config::LanguageServerFeature, HighlightEvent, Loader},
};
use helix_view::{
    graphics::{Color, Rect, Style},
    Document, Theme, View,
};
use tui::buffer::Buffer as Surface;

/// Document lines per minimap row: a braille cell is 4 dots tall (and 2 wide).
const LINES_PER_ROW: usize = 4;
/// Braille bit for the dot at index `dy * 2 + dx`.
const BRAILLE_BITS: [u32; 8] = [0x01, 0x08, 0x02, 0x10, 0x04, 0x20, 0x40, 0x80];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    Minimap,
    Scrollbar,
}

/// Vertical document metrics in document lines (soft-wrap is ignored).
struct Metrics {
    total: usize,
    top: usize,
    visible: usize,
}

impl Metrics {
    fn new(doc: &Document, view: &View) -> Self {
        let text = doc.text().slice(..);
        let anchor = doc.view_offset(view.id).anchor.min(text.len_chars());
        Self {
            total: text.len_lines(),
            top: text.char_to_line(anchor),
            visible: view.inner_height(),
        }
    }

    /// First line shown by a minimap holding `capacity` lines. Documents longer than that
    /// scroll the minimap proportionally to the view.
    fn minimap_first_line(&self, capacity: usize) -> usize {
        if self.total <= capacity {
            return 0;
        }
        let range = self.total.saturating_sub(self.visible).max(1);
        self.top.min(range) * (self.total - capacity) / range
    }
}

/// The bar under the given screen position.
pub fn bar_at(view: &View, doc: &Document, row: u16, column: u16) -> Option<Bar> {
    let (minimap, scrollbar) = view.bar_areas(doc);
    let hit = |area: Option<Rect>| {
        area.is_some_and(|a| {
            (a.left()..a.right()).contains(&column) && (a.top()..a.bottom()).contains(&row)
        })
    };
    if hit(minimap) {
        Some(Bar::Minimap)
    } else if hit(scrollbar) {
        Some(Bar::Scrollbar)
    } else {
        None
    }
}

/// The top line that centers the document line under screen `row` of the bar in the view.
/// `None` when the bar is hidden.
pub fn top_line_at(doc: &Document, view: &View, bar: Bar, row: u16) -> Option<usize> {
    let (minimap, scrollbar) = view.bar_areas(doc);
    let area = match bar {
        Bar::Minimap => minimap,
        Bar::Scrollbar => scrollbar,
    }?;
    let m = Metrics::new(doc, view);
    let row = row.saturating_sub(area.y).min(area.height.saturating_sub(1)) as usize;
    let half = m.visible / 2;
    let top = match bar {
        Bar::Scrollbar => ((2 * row + 1) * m.total / (2 * area.height as usize)).saturating_sub(half),
        Bar::Minimap => {
            let capacity = area.height as usize * LINES_PER_ROW;
            let line = row * LINES_PER_ROW + LINES_PER_ROW / 2;
            if m.total <= capacity {
                line.saturating_sub(half)
            } else {
                // Inverse of `minimap_first_line`, so the viewport band follows the pointer.
                let range = m.total.saturating_sub(m.visible);
                (line.saturating_sub(half) * range / capacity.saturating_sub(m.visible).max(1))
                    .min(range)
            }
        }
    };
    Some(top.min(m.total.saturating_sub(1)))
}

pub fn render(doc: &Document, view: &View, theme: &Theme, loader: &Loader, surface: &mut Surface) {
    let (minimap, scrollbar) = view.bar_areas(doc);
    if minimap.is_none() && scrollbar.is_none() {
        return;
    }
    let m = Metrics::new(doc, view);
    if let Some(area) = minimap {
        render_minimap(doc, theme, loader, &m, area, surface);
    }
    if let Some(area) = scrollbar {
        render_scrollbar(doc, view, theme, &m, area, surface);
    }
}

fn render_minimap(
    doc: &Document,
    theme: &Theme,
    loader: &Loader,
    m: &Metrics,
    area: Rect,
    surface: &mut Surface,
) {
    let text = doc.text().slice(..);
    let width = area.width as usize;
    let rows = area.height as usize;
    let capacity = rows * LINES_PER_ROW;
    let first = m.minimap_first_line(capacity);
    let end = (first + capacity).min(m.total);
    let dot_columns = width * 2;
    let tab_width = doc.tab_width().max(1);

    let base = theme
        .try_get_exact("ui.minimap")
        .unwrap_or_else(|| theme.get("ui.background"));
    let band = base.patch(
        theme
            .try_get_exact("ui.minimap.viewport")
            .unwrap_or_else(|| theme.get("ui.cursorline.primary")),
    );
    let default_fg = base
        .fg
        .or(theme.get("ui.text").fg)
        .unwrap_or(Color::Reset);

    // One color sample per dot, 8 dots per cell; `None` is an empty dot.
    let mut dots: Vec<Option<Color>> = vec![None; width * rows * 8];
    let range = text.line_to_byte(first) as u32..text.line_to_byte(end) as u32;
    let mut highlighter = doc
        .syntax()
        .map(|syntax| syntax.highlighter(text, loader, range));
    let mut style = Style::default();

    for line in first..end {
        let offset = line - first;
        let (row, dy) = (offset / LINES_PER_ROW, offset % LINES_PER_ROW);
        let mut byte = text.line_to_byte(line) as u32;
        let mut col = 0;
        for ch in text.line(line).chars() {
            if col >= dot_columns || ch == '\n' || ch == '\r' {
                break;
            }
            if let Some(highlighter) = highlighter.as_mut() {
                while highlighter.next_event_offset() <= byte {
                    let (event, highlights) = highlighter.advance();
                    let base = match event {
                        HighlightEvent::Refresh => Style::default(),
                        HighlightEvent::Push => style,
                    };
                    style = highlights.fold(base, |acc, h| acc.patch(theme.highlight(h)));
                }
            }
            match ch {
                '\t' => col = (col / tab_width + 1) * tab_width,
                ch if ch.is_whitespace() => col += 1,
                _ => {
                    let cell = row * width + col / 2;
                    dots[cell * 8 + dy * 2 + col % 2] = Some(style.fg.unwrap_or(default_fg));
                    col += 1;
                }
            }
            byte += ch.len_utf8() as u32;
        }
    }

    let band_start = m.top.max(first);
    let band_end = (m.top + m.visible).min(end);
    let band_rows = if band_start < band_end {
        (band_start - first) / LINES_PER_ROW..(band_end - 1 - first) / LINES_PER_ROW + 1
    } else {
        0..0
    };

    for row in 0..rows {
        let style = if band_rows.contains(&row) { band } else { base };
        for x in 0..width {
            let cell_dots = &dots[(row * width + x) * 8..][..8];
            let cell = &mut surface[(area.x + x as u16, area.y + row as u16)];
            cell.reset();
            cell.set_style(style);
            if let Some(color) = most_frequent(cell_dots) {
                let bits = cell_dots
                    .iter()
                    .zip(BRAILLE_BITS)
                    .filter(|(dot, _)| dot.is_some())
                    .fold(0, |acc, (_, bit)| acc | bit);
                cell.set_char(char::from_u32(0x2800 + bits).unwrap_or(' '))
                    .set_fg(color);
            }
        }
    }
}

/// The most frequent color among the dots, ties going to the first one.
fn most_frequent(dots: &[Option<Color>]) -> Option<Color> {
    let mut best = None;
    let mut best_count = 0;
    for color in dots.iter().flatten() {
        let count = dots.iter().filter(|dot| **dot == Some(*color)).count();
        if count > best_count {
            best = Some(*color);
            best_count = count;
        }
    }
    best
}

fn render_scrollbar(
    doc: &Document,
    view: &View,
    theme: &Theme,
    m: &Metrics,
    area: Rect,
    surface: &mut Surface,
) {
    let height = area.height as usize;
    if height == 0 {
        return;
    }
    let text = doc.text().slice(..);
    let total = m.total.max(1);
    let row_of = |line: usize| line.min(total - 1) * height / total;

    let menu = theme.get("ui.menu.scroll");
    let track = theme
        .try_get_exact("ui.scrollbar")
        .and_then(|style| style.bg)
        .or(menu.bg)
        .unwrap_or(Color::Reset);
    let thumb = theme
        .try_get_exact("ui.scrollbar.thumb")
        .and_then(|style| style.bg)
        .or(menu.fg)
        .unwrap_or(Color::Reset);
    let thumb_start = row_of(m.top);
    let thumb_end = ((m.top + m.visible) * height)
        .div_ceil(total)
        .clamp(thumb_start + 1, height);

    // Highest priority marker per row.
    let mut markers: Vec<Option<(u8, Color)>> = vec![None; height];
    let mut mark = |row: usize, priority: u8, color: Option<Color>| {
        let slot = &mut markers[row];
        if let Some(color) = color {
            if slot.is_none_or(|(p, _)| p < priority) {
                *slot = Some((priority, color));
            }
        }
    };

    let (error, warning, info, hint) = (
        theme.get("error").fg,
        theme.get("warning").fg,
        theme.get("info").fg,
        theme.get("hint").fg,
    );
    for diagnostic in doc.diagnostics().iter().filter(|d| {
        d.provider.language_server_id().is_none_or(|id| {
            doc.language_servers_with_feature(LanguageServerFeature::Diagnostics)
                .any(|ls| ls.id() == id)
        })
    }) {
        let (priority, color) = match diagnostic.severity {
            Some(Severity::Error) => (5, error),
            Some(Severity::Warning) | None => (4, warning),
            Some(Severity::Info) => (1, info),
            Some(Severity::Hint) => (1, hint),
        };
        mark(row_of(diagnostic.line), priority, color);
    }

    if let Some(handle) = doc.diff_handle() {
        let hunks = handle.load();
        let (plus, minus, delta) = (
            theme.get("diff.plus").fg,
            theme.get("diff.minus").fg,
            theme.get("diff.delta").fg,
        );
        for i in 0..hunks.len() {
            let hunk = hunks.nth_hunk(i);
            let color = if hunk.is_pure_insertion() {
                plus
            } else if hunk.is_pure_removal() {
                minus
            } else {
                delta
            };
            let start = hunk.after.start as usize;
            let last = (hunk.after.end as usize).saturating_sub(1).max(start);
            for row in row_of(start)..=row_of(last) {
                mark(row, 3, color);
            }
        }
    }

    let cursor = theme.get("ui.cursor.primary");
    for range in doc.selection(view.id).iter() {
        let line = text.char_to_line(range.cursor(text));
        mark(row_of(line), 2, cursor.bg.or(cursor.fg));
    }

    for (row, marker) in markers.iter().enumerate() {
        let cell = &mut surface[(area.x, area.y + row as u16)];
        cell.reset();
        cell.set_bg(if (thumb_start..thumb_end).contains(&row) {
            thumb
        } else {
            track
        });
        if let Some((_, color)) = marker {
            cell.set_symbol("━").set_fg(*color);
        }
    }
}
