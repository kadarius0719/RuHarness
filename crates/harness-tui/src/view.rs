//! Drawing (docs/COCKPIT-WRAPPER-DESIGN.md §1, §3, §5, §6): the Files tree,
//! the View of the selection, the two activity rows, the hint bar and the
//! overlays (menu, dialog, details, help, verdict, diff). Every piece of
//! target-, model- or harness-derived text goes through the display filter
//! ([`crate::display`]) before it reaches the terminal; widths are measured
//! per grapheme exactly as ratatui draws them; only visible rows are built;
//! the view never reads the ledger itself. Each clickable region is recorded
//! in [`App::hits`] as it is drawn (the mouse of Build B uses them).

use crate::app::{
    attempt_tags, provenance_words, shell_line, short_id, App, CodeLine, Confirm, Focus, Hit,
    LayoutMode, Menu, Mode, PairView, Tone,
};
use crate::display::{self, Sanitizer};
use crate::files::{self, FileState, UnitState};
use crate::highlight::{Class, Pieces};
use crate::menu::MODEL_SEPARATOR;
use crate::model::UnitView;
use crate::narrate::{check_words, elapsed_words};
use crate::tree::{Row, RowKind, Selection};
use ratatui::buffer::CellWidth;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Below this many columns one pane shows at a time (Files or View).
pub const SINGLE_PANE_BELOW: u16 = 80;
/// From this many columns the Files pane is [`FILES_WIDE`] wide.
pub const WIDE_FROM: u16 = 120;
/// The Files pane at [`WIDE_FROM`] columns or more.
pub const FILES_WIDE: u16 = 32;
/// The Files pane at 80–119 columns.
pub const FILES_NARROW: u16 = 24;
/// The C and the Rust sit side by side when the View is at least this wide.
pub const SPLIT_VIEW_MIN: u16 = 78;
/// The dialog's width (narrower terminals get what they have).
pub const DIALOG_COLUMNS: u16 = 76;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn tone(t: Tone) -> Style {
    match t {
        Tone::Plain => Style::default(),
        Tone::Good => Style::default().fg(Color::Green),
        Tone::Bad => Style::default().fg(Color::Red),
        Tone::Warn => Style::default().fg(Color::Yellow),
        Tone::Dim => dim(),
    }
}

fn class_style(class: Class) -> Style {
    match class {
        Class::Plain | Class::Name => Style::default(),
        Class::Attribute => Style::default().fg(Color::Magenta),
        Class::Comment => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        Class::Constant => Style::default().fg(Color::Cyan),
        Class::Function => Style::default().fg(Color::Blue),
        Class::Keyword => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        Class::Punctuation => Style::default(),
        Class::String => Style::default().fg(Color::Green),
        Class::Type => Style::default().fg(Color::Yellow),
    }
}

/// A state glyph's colour (a glyph AND a word are always shown: colour is
/// never the only signal).
fn glyph_style(glyph: &str) -> Style {
    match glyph {
        "✗" | "?" => Style::default().fg(Color::Red),
        "⚠" | "!" | "◐" => Style::default().fg(Color::Yellow),
        "✓" => Style::default().fg(Color::Green),
        "✓?" => Style::default().fg(Color::Cyan),
        "+" => Style::default().fg(Color::Cyan),
        _ => dim(),
    }
}

/// Every grapheme of already-filtered `text` with the cells ratatui draws
/// it in, so a padded column is exactly as wide on screen as measured here;
/// `f` returns `false` to stop.
fn graphemes(text: &str, mut f: impl FnMut(&str, usize) -> bool) {
    let span = Span::raw(text);
    for g in span.styled_graphemes(Style::default()) {
        if !f(g.symbol, usize::from(g.symbol.cell_width())) {
            break;
        }
    }
}

/// `raw` through the display filter, cut to `width` display columns.
fn safe(raw: &str, width: usize) -> String {
    clip(&display::line(raw), width)
}

/// Already-filtered text cut to `width` display columns.
fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0;
    graphemes(text, |g, w| {
        if col + w > width {
            return false;
        }
        out.push_str(g);
        col += w;
        true
    });
    out
}

/// `raw` filtered and cut to `width`, ending in `…` when it was cut.
fn ellipsis(raw: &str, width: usize) -> String {
    let text = display::line(raw);
    if width_of(&text) <= width {
        return text;
    }
    let mut out = clip(&text, width.saturating_sub(1));
    out.push('…');
    out
}

fn width_of(text: &str) -> usize {
    let mut n = 0;
    graphemes(text, |_, w| {
        n += w;
        true
    });
    n
}

/// Already-filtered `text` cut into rows of at most `width` cells.
fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = vec![String::new()];
    let mut col = 0;
    graphemes(text, |g, w| {
        if col > 0 && col + w > width {
            rows.push(String::new());
            col = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push_str(g);
        }
        col += w;
        true
    });
    rows
}

/// `raw` filtered, then word-wrapped to `width` (long words hard-wrapped).
fn word_wrap(raw: &str, width: usize) -> Vec<String> {
    let text = display::line(raw);
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let candidate = if row.is_empty() {
            word.to_string()
        } else {
            format!("{row} {word}")
        };
        if width_of(&candidate) <= width {
            row = candidate;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if width_of(word) > width {
            let mut parts = hard_wrap(word, width);
            row = parts.pop().unwrap_or_default();
            rows.extend(parts);
        } else {
            row = word.to_string();
        }
    }
    rows.push(row);
    rows
}

/// Raw text → filtered, word-wrapped rows of `width`, all in `style`.
fn wrapped(raw: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    word_wrap(raw, width)
        .into_iter()
        .map(|row| Line::from(Span::styled(row, style)))
        .collect()
}

/// Highlighted raw pieces of one line → spans at most `width` columns wide,
/// the first `skip` columns scrolled away (horizontal scroll). A line cut at
/// the right ends with a dim `›`. Returns the spans and the columns used.
fn fit(pieces: &Pieces, width: usize, skip: usize) -> (Vec<Span<'static>>, usize) {
    let mut san = Sanitizer::default();
    let mut budget = display::MAX_LINE_BYTES;
    let mut col = 0; // columns of the line seen so far
    let mut used = 0; // columns drawn
    let mut spans = Vec::new();
    let mut cut = false;
    'pieces: for (class, raw) in pieces {
        if budget == 0 {
            break;
        }
        let mut end = raw.len().min(budget);
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        budget -= end;
        let text = san.push(&raw[..end]);
        let mut piece = String::new();
        let mut stop = false;
        graphemes(&text, |g, w| {
            if col < skip {
                // A wide grapheme straddling the edge shows as spaces.
                if col + w > skip {
                    piece.push_str(&" ".repeat(col + w - skip));
                    used += col + w - skip;
                }
                col += w;
                return true;
            }
            if used + w > width {
                stop = true;
                return false;
            }
            piece.push_str(g);
            used += w;
            col += w;
            true
        });
        if !piece.is_empty() {
            spans.push(Span::styled(piece, class_style(*class)));
        }
        if stop {
            cut = true;
            break 'pieces;
        }
    }
    if cut && width > 0 {
        // Make room for the cut mark.
        while used + 1 > width {
            let Some(last) = spans.pop() else { break };
            let text = last.content.to_string();
            let keep = clip(&text, width_of(&text).saturating_sub(used + 1 - width));
            used = used - width_of(&text) + width_of(&keep);
            if !keep.is_empty() {
                spans.push(Span::styled(keep, last.style));
            }
        }
        spans.push(Span::styled("›", dim()));
        used += 1;
    }
    (spans, used)
}

/// The spans of one cell of a side: gutter + code, a link, a note, or the
/// filler of the shorter side. Always exactly `width` columns.
fn cell(line: Option<&CodeLine>, gutter: usize, width: usize, skip: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let used = match line {
        None => {
            spans.push(Span::styled(clip("~", width), dim()));
            width.min(1)
        }
        Some(CodeLine::Code { number, pieces }) => {
            let g = clip(
                &format!("{number:>w$} ", w = gutter.saturating_sub(1)),
                width,
            );
            let gw = width_of(&g);
            spans.push(Span::styled(g, dim()));
            let (code, cw) = fit(pieces, width.saturating_sub(gw), skip);
            spans.extend(code);
            gw + cw
        }
        Some(CodeLine::Link(text)) => {
            let t = safe(text, width);
            let w = width_of(&t);
            spans.push(Span::styled(
                t,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::ITALIC),
            ));
            w
        }
        Some(CodeLine::Note(text)) => {
            let t = safe(text, width);
            let w = width_of(&t);
            spans.push(Span::styled(
                t,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC),
            ));
            w
        }
    };
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    spans
}

fn gutter_of(lines: &[CodeLine]) -> usize {
    lines
        .iter()
        .filter_map(|l| match l {
            CodeLine::Code { number, .. } => Some(*number),
            _ => None,
        })
        .max()
        .map_or(0, |n| n.to_string().len() + 1)
}

/// Rows one pair takes: a heading, the body, a rule (wide); two headings,
/// both sides, a rule (stacked).
fn pair_height(p: &PairView, wide: bool) -> usize {
    if wide {
        2 + p.c.len().max(p.rust.len())
    } else {
        3 + p.c.len() + p.rust.len()
    }
}

/// Row `r` (0-based, `< pair_height`) of one pair.
fn pair_row(
    p: &PairView,
    (gc, gr): (usize, usize),
    r: usize,
    width: usize,
    wide: bool,
    skip: usize,
) -> Line<'static> {
    let last = pair_height(p, wide) - 1;
    if r == last {
        return Line::from(Span::styled("─".repeat(width), dim()));
    }
    if wide {
        let left = (width.saturating_sub(3)) / 2;
        let right = width.saturating_sub(3 + left);
        if r == 0 {
            let lt = ellipsis(&format!("C  {}", p.c_title), left);
            let lw = width_of(&lt);
            return Line::from(vec![
                Span::styled(lt, bold()),
                Span::raw(" ".repeat(left.saturating_sub(lw))),
                Span::styled(" ⇄ ", dim()),
                Span::styled(ellipsis(&format!("Rust  {}", p.rust_title), right), bold()),
            ]);
        }
        let i = r - 1;
        let mut spans = cell(p.c.get(i), gc, left, skip);
        spans.push(Span::styled(" │ ", dim()));
        spans.extend(cell(p.rust.get(i), gr, right, skip));
        return Line::from(spans);
    }
    let heading = |lead: &'static str, text: &str| {
        Line::from(vec![
            Span::styled(lead, dim()),
            Span::styled(ellipsis(text, width.saturating_sub(5)), bold()),
        ])
    };
    let c_end = 1 + p.c.len();
    if r == 0 {
        heading("C    ", &p.c_title)
    } else if r < c_end {
        Line::from(cell(p.c.get(r - 1), gc, width, skip))
    } else if r == c_end {
        heading("Rust ", &p.rust_title)
    } else {
        Line::from(cell(p.rust.get(r - c_end - 1), gr, width, skip))
    }
}

/// The pairs' rows `[from, from + height)`, built only for those rows, plus
/// the first row of each pair and the total row count.
pub fn pair_window(
    pairs: &[PairView],
    width: usize,
    wide: bool,
    from: usize,
    height: usize,
    skip: usize,
) -> (Vec<Line<'static>>, Vec<usize>, usize) {
    if pairs.is_empty() {
        let rows = if from == 0 && height > 0 {
            vec![Line::from(Span::styled("no function pairs", dim()))]
        } else {
            Vec::new()
        };
        return (rows, Vec::new(), 1);
    }
    let mut starts = Vec::with_capacity(pairs.len());
    let mut total = 0;
    for p in pairs {
        starts.push(total);
        total += pair_height(p, wide);
    }
    let end = from.saturating_add(height).min(total);
    let mut rows = Vec::with_capacity(end.saturating_sub(from));
    let mut i = starts.partition_point(|s| *s <= from).saturating_sub(1);
    let mut row = from;
    let mut gutters = None;
    while row < end && i < pairs.len() {
        let local = row - starts[i];
        if local < pair_height(&pairs[i], wide) {
            let g =
                *gutters.get_or_insert_with(|| (gutter_of(&pairs[i].c), gutter_of(&pairs[i].rust)));
            rows.push(pair_row(&pairs[i], g, local, width, wide, skip));
            row += 1;
        } else {
            i += 1;
            gutters = None;
        }
    }
    (rows, starts, total)
}

/// The first index of a `rows`-long window over `len` items that keeps
/// `sel` in view, moving as little as possible from `offset`.
fn follow(len: usize, rows: usize, sel: usize, offset: usize) -> usize {
    if len <= rows || rows == 0 {
        return 0;
    }
    let offset = offset.min(len - rows);
    if sel < offset {
        sel
    } else if sel >= offset + rows {
        sel + 1 - rows
    } else {
        offset
    }
}

// ----- the Files pane ---------------------------------------------------------

/// A symbol's name without the scanner's `<file>::` prefix of a static.
pub fn short_symbol(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// A node's name in the tree.
fn node_name(app: &App, sel: &Selection) -> String {
    match sel {
        Selection::Project => app
            .config
            .target
            .file_name()
            .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned()),
        Selection::Dir(d) => format!("{}/", d.rsplit('/').next().unwrap_or(d)),
        Selection::File(p) => p.rsplit('/').next().unwrap_or(p).to_string(),
        Selection::Function(_, name) => format!("{}()", short_symbol(name)),
        Selection::Units => format!("Units ({})", app.snapshot.units.len()),
        Selection::Unit(id) => id.clone(),
        Selection::Crate(_) => "crate".into(),
        Selection::Attempt(_, id) => short_id(id),
    }
}

/// A row's glyph and state word (a rollup for the project and directories).
fn row_label(app: &App, sel: &Selection) -> (&'static str, String) {
    match sel {
        Selection::Project => ("", app.files.rollup("").text()),
        Selection::Dir(d) => ("", app.files.rollup(d).text()),
        Selection::Units => ("", String::new()),
        Selection::Attempt(u, a) => {
            let (_, word) = app.node_label(sel);
            let glyph = match app
                .snapshot
                .unit(u)
                .and_then(|x| x.attempt(a))
                .map(|x| x.record.outcome.as_str())
            {
                Some("green") => "✓",
                Some("in-progress") => "◐",
                Some(_) => "✗",
                None => "",
            };
            (glyph, word)
        }
        _ => app.node_label(sel),
    }
}

fn tree_row(app: &App, row: &Row, width: usize, selected: bool) -> Line<'static> {
    let indent = "  ".repeat(row.depth);
    let sel = match &row.kind {
        RowKind::Note(text) => {
            return Line::from(Span::styled(
                ellipsis(&format!("{indent}  {text}"), width),
                Style::default().fg(Color::Yellow),
            ));
        }
        RowKind::Node(sel) => sel,
    };
    let marker = if !row.expandable {
        "  "
    } else if row.open {
        "▾ "
    } else {
        "▸ "
    };
    let (glyph, word) = row_label(app, sel);
    let internal = matches!(sel, Selection::Function(..)) && word == "internal";
    let lead = format!("{indent}{marker}");
    let glyph_text = if glyph.is_empty() {
        String::new()
    } else {
        format!("{glyph} ")
    };
    let name = node_name(app, sel);
    let head = width_of(&lead) + width_of(&glyph_text);
    let room = width.saturating_sub(head);
    let word_w = width_of(&display::line(&word));
    let name_w = width_of(&display::line(&name));
    // The word shows at the right edge when it fits; the selected row
    // always shows it — cut first, the name keeping at least half the row
    // (review USE-8).
    // A state with no glyph (the fallback) shows its word always: it is its
    // only sign (review USE-6).
    let show_word = !word.is_empty()
        && !internal
        && (name_w + 2 + word_w <= room
            || selected
            || (glyph.is_empty() && matches!(sel, Selection::Unit(_) | Selection::File(_))));
    let (name_text, pad, word_text) = if show_word {
        let name_keep = name_w.min(room / 2);
        let word_room = room.saturating_sub(name_keep + 1).max(1);
        let word_text = ellipsis(&word, word_room.min(word_w));
        let name_room = room.saturating_sub(width_of(&word_text) + 1);
        let name_text = ellipsis(&name, name_room);
        let pad = room.saturating_sub(width_of(&name_text) + width_of(&word_text));
        (name_text, pad, word_text)
    } else {
        (ellipsis(&name, room), 0, String::new())
    };
    let mut name_style = if internal { dim() } else { Style::default() };
    if matches!(sel, Selection::Project | Selection::Units) {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    let mut spans = vec![
        Span::styled(lead, dim()),
        Span::styled(glyph_text, glyph_style(glyph)),
        Span::styled(name_text, name_style),
    ];
    if show_word {
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(word_text, dim()));
    }
    Line::from(spans)
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    let mut block = Block::default().borders(Borders::ALL);
    if focused {
        block = block
            .border_style(bold())
            .title(Span::styled(title, bold().add_modifier(Modifier::REVERSED)));
    } else {
        block = block.border_style(dim()).title(Span::styled(title, dim()));
    }
    block
}

fn draw_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused =
        app.focus == Focus::Files && matches!(app.mode, Mode::Normal | Mode::Details { .. });
    let title = if app.loading {
        " Files · reading… ".to_string()
    } else {
        " Files ".to_string()
    };
    let block = pane_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.push((area, Hit::Pane(Focus::Files)));
    let height = inner.height as usize;
    app.layout.tree_page = height;
    let cursor = app.cursor().unwrap_or(0);
    // The wheel lets the selected row scroll away until the next key or
    // click (§7); otherwise the tree keeps it in view.
    app.tree_offset = if app.tree_follow {
        follow(app.rows.len(), height, cursor, app.tree_offset)
    } else {
        app.tree_offset.min(app.rows.len().saturating_sub(height))
    };
    let width = inner.width as usize;
    let mut lines = Vec::new();
    for (i, row) in app
        .rows
        .iter()
        .enumerate()
        .skip(app.tree_offset)
        .take(height)
    {
        let selected = i == cursor && row.selection() == Some(&app.selection);
        let mut line = tree_row(app, row, width, selected);
        if selected {
            line = line.style(if focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::UNDERLINED)
            });
        }
        lines.push(line);
        if let Some(sel) = row.selection() {
            let y = inner.y + (i - app.tree_offset) as u16;
            app.hits
                .push((Rect::new(inner.x, y, inner.width, 1), Hit::Row(sel.clone())));
            // The ▸/▾ marker folds or opens the row (review USE-B-1).
            let x = 2 * row.depth;
            if row.expandable && x + 2 <= width {
                app.hits.push((
                    Rect::new(inner.x + x as u16, y, 2, 1),
                    Hit::Fold(sel.clone()),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// ----- the View ------------------------------------------------------------------

/// The View's title: the node and its state (the selected row's word).
fn view_title(app: &App) -> String {
    let sel = &app.selection;
    let name = match sel {
        Selection::File(p) | Selection::Dir(p) => p.clone(),
        Selection::Function(p, n) => format!("{}() in {p}", short_symbol(n)),
        Selection::Crate(u) => format!("{u}'s crate"),
        Selection::Attempt(u, a) => format!("attempt {} of {u}", short_id(a)),
        _ => node_name(app, sel),
    };
    let (_, word) = row_label(app, sel);
    let mut title = if word.is_empty() {
        name
    } else {
        format!("{name} · {word}")
    };
    if let Some(u) = app.unit_view() {
        if matches!(sel, Selection::File(_) | Selection::Function(..)) {
            title = format!("{title} ({})", u.unit.id);
        }
    }
    format!(" {} ", display::line(&title))
}

fn unit_state<'a>(app: &'a App, unit: &UnitView) -> Option<&'a UnitState> {
    app.snapshot
        .units
        .iter()
        .position(|u| u.unit.id == unit.unit.id)
        .and_then(|i| app.files.units.get(i))
        .map(|i| &i.state)
}

/// The lines above a unit's pairs: its state (and cause), its crate's
/// origin, an attempt's record.
fn unit_header(app: &App, unit: &UnitView, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let state = unit_state(app, unit);
    if let Some(s) = state {
        let mut spans = vec![
            Span::styled(
                if s.glyph().is_empty() {
                    String::new()
                } else {
                    format!("{} ", s.glyph())
                },
                glyph_style(s.glyph()),
            ),
            Span::styled(format!("{} ", unit.unit.id), bold()),
            Span::raw(s.word()),
            Span::styled(format!(" · status {}", unit.report.status), dim()),
        ];
        if let Some(h) = &unit.report.write_in_flight {
            spans.push(Span::styled(
                format!(" · being written by `{}`", h.command),
                Style::default().fg(Color::Yellow),
            ));
        }
        lines.push(Line::from(clipped(spans, width)));
        if let UnitState::Attention(cause) = s {
            lines.extend(wrapped(
                &format!("Needs attention: {}", cause.words()),
                width,
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    match app.shown_attempt() {
        Some(a) => {
            let r = &a.record;
            let mut text = format!(
                "Attempt {} · {} · provider {} · model {}",
                r.id, r.outcome, r.provider, r.model
            );
            let tags = attempt_tags(unit, a);
            if !tags.is_empty() {
                text.push_str(&format!(" · {}", tags.join(" ")));
            }
            lines.extend(wrapped(&text, width, Style::default()));
            if let Some(note) = &r.steer_note {
                lines.extend(wrapped(&format!("Steer note: {note}"), width, dim()));
            }
            if let Some(note) = &r.note {
                lines.extend(wrapped(&format!("Hand-edit note: {note}"), width, dim()));
            }
            if !r.turns.is_empty() {
                let turns: Vec<String> = r
                    .turns
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{} {} → {}", i + 1, t.kind, t.result))
                    .collect();
                lines.extend(wrapped(
                    &format!("Turns: {}", turns.join(" · ")),
                    width,
                    dim(),
                ));
            }
            if app
                .awaiting
                .iter()
                .any(|aw| aw.attempt.as_deref() == Some(r.id.as_str()))
            {
                lines.extend(wrapped(
                    "Waiting for your answer to the hand-off; then choose Resume (R).",
                    width,
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        None => {
            let verdict = match (
                &unit.report.verdict.green,
                unit.report.verdict.stale.is_empty(),
            ) {
                (Some(true), true) => "verdict green, fresh".to_string(),
                (Some(false), true) => "verdict RED".to_string(),
                (Some(g), false) => format!(
                    "verdict {} but out of date ({})",
                    if *g { "green" } else { "red" },
                    unit.report.verdict.stale.join(", ")
                ),
                (None, _) => "no verdict yet".to_string(),
            };
            let origin = if unit.crate_dir.is_some() {
                format!("Crate {} · {verdict}", provenance_words(unit))
            } else {
                "No crate yet".to_string()
            };
            lines.extend(wrapped(&origin, width, dim()));
        }
    }
    lines
}

/// The spans filtered and clipped, one after the other, to `width`.
fn clipped(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut used = 0;
    spans
        .into_iter()
        .map(|span| {
            let text = safe(&span.content, width.saturating_sub(used));
            used += width_of(&text);
            Span::styled(text, span.style)
        })
        .collect()
}

/// The checks strip in `rows` rows: FAILED checks first, in words, so a cut
/// strip never hides a failure, and `+N` for whatever did not fit.
fn checks_lines(app: &App, width: usize, rows: usize) -> Vec<Line<'static>> {
    let Some(v) = app.shown_verdict() else {
        return vec![Line::from(Span::styled(
            clip("Checks  none yet for what is shown", width),
            dim(),
        ))];
    };
    let rows = rows.max(1);
    let mut chips: Vec<(String, Style)> = vec![(
        if v.green {
            "Checks ".into()
        } else {
            "Checks RED ".into()
        },
        if v.green {
            dim()
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        },
    )];
    // Failed first; checks with the same words (the whole-program samples)
    // collapse into one chip with a count.
    let mut grouped: Vec<(bool, String, usize)> = Vec::new();
    for check in v
        .checks
        .iter()
        .filter(|c| !c.passed)
        .chain(v.checks.iter().filter(|c| c.passed))
    {
        let words = check_words(&check.name);
        match grouped
            .iter_mut()
            .find(|(p, w, _)| *p == check.passed && *w == words)
        {
            Some((_, _, n)) => *n += 1,
            None => grouped.push((check.passed, words, 1)),
        }
    }
    for (passed, words, n) in grouped {
        let count = if n > 1 {
            format!(" ×{n}")
        } else {
            String::new()
        };
        chips.push((
            display::line(&format!(
                " {} {words}{count} ",
                if passed { "✓" } else { "✗" }
            )),
            if passed {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ));
    }
    let total = chips.len();
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0;
    let mut placed = 0;
    for (i, (text, style)) in chips.iter().enumerate() {
        let w = width_of(text);
        let hidden_after = total - i - 1;
        let marker = if hidden_after > 0 { 6 } else { 0 };
        let last_row = lines.len() == rows;
        if used + w > width || (last_row && used + w + marker > width && hidden_after > 0) {
            if last_row {
                break;
            }
            lines.push(vec![Span::raw("       ")]);
            used = 7;
        }
        if let Some(line) = lines.last_mut() {
            line.push(Span::styled(clip(text, width), *style));
        }
        used += w;
        placed += 1;
    }
    if placed < total {
        if let Some(line) = lines.last_mut() {
            line.push(Span::styled(format!("+{}", total - placed), dim()));
        }
    }
    lines.into_iter().map(Line::from).collect()
}

/// The project summary (§3): the Next step first, as a fact; the facts; the
/// units by state; the lock holder; the last command; the files that need
/// action, each a link.
fn summary(app: &App, width: usize, links: &mut Vec<(usize, Selection)>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some((step, _)) = app.next_step() {
        lines.extend(wrapped(&format!("Next step: {step}"), width, bold()));
        lines.push(Line::from(""));
    }
    match &app.snapshot.facts_state {
        None => lines.extend(wrapped("Nothing is scanned yet.", width, dim())),
        Some(s) => {
            let count = |st: FileState| app.files.files.iter().filter(|f| f.state == st).count();
            let mut text = format!("{} files scanned", s.files);
            for (n, what) in [
                // The stale paths hold the missing files too (ENG-6).
                (s.stale.saturating_sub(count(FileState::Missing)), "changed"),
                (count(FileState::New), "new"),
                (count(FileState::Missing), "missing"),
            ] {
                if n > 0 {
                    text.push_str(&format!(" · {n} {what}"));
                }
            }
            lines.extend(wrapped(&text, width, Style::default()));
        }
    }
    if !app.snapshot.units.is_empty() {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for info in &app.files.units {
            let key = format!("{} {}", info.state.glyph(), info.state.word())
                .trim()
                .to_string();
            match counts.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => counts.push((key, 1)),
            }
        }
        let text = counts
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(" · ");
        lines.extend(wrapped(
            &format!("Units ({}): {text}", app.snapshot.units.len()),
            width,
            Style::default(),
        ));
    }
    // (The read model's own note — "no plan — run `harness plan`" — is the
    // CLI's wording; the Next step above says it the cockpit's way.)
    if let Some(h) = &app.holder {
        lines.extend(wrapped(
            &format!("Busy: `{}` holds the writer lock", h.command),
            width,
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(last) = &app.last {
        lines.extend(wrapped(&format!("Last: {last}"), width, dim()));
    }
    type Pick = fn(&App, &files::FileInfo) -> bool;
    let groups: [(&str, Pick); 4] = [
        (
            "Failing",
            |app, f| matches!(f.state, FileState::Owned(u) if matches!(app.files.units[u].state, UnitState::Failing)),
        ),
        (
            "Needs attention",
            |app, f| matches!(f.state, FileState::Owned(u) if matches!(app.files.units[u].state, UnitState::Attention(_))),
        ),
        ("Changed since the scan", |_, f| {
            matches!(f.state, FileState::Changed | FileState::Missing)
        }),
        ("New (not scanned yet)", |_, f| f.state == FileState::New),
    ];
    for (title, pick) in groups {
        let picked: Vec<&files::FileInfo> =
            app.files.files.iter().filter(|f| pick(app, f)).collect();
        if picked.is_empty() {
            continue;
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("{title}:"), bold())));
        for f in picked {
            let (glyph, word) = files::file_label(&app.files, &f.state);
            links.push((lines.len(), Selection::File(f.path.clone())));
            lines.push(Line::from(clipped(
                vec![
                    Span::raw("  "),
                    Span::styled(format!("{glyph} "), glyph_style(glyph)),
                    Span::raw(f.path.clone()),
                    Span::styled(format!("  {word}"), dim()),
                ],
                width,
            )));
        }
    }
    lines
}

/// A directory's View: its files and their states, each a link.
fn dir_view(
    app: &App,
    dir: &str,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let rollup = app.files.rollup(dir).text();
    if !rollup.is_empty() {
        lines.push(Line::from(Span::styled(safe(&rollup, width), dim())));
    }
    let prefix = format!("{dir}/");
    for f in app
        .files
        .files
        .iter()
        .filter(|f| f.path.starts_with(&prefix))
    {
        let (glyph, word) = files::file_label(&app.files, &f.state);
        links.push((lines.len(), Selection::File(f.path.clone())));
        lines.push(Line::from(clipped(
            vec![
                Span::styled(format!("{glyph:<2} "), glyph_style(glyph)),
                Span::raw(f.path[prefix.len()..].to_string()),
                Span::styled(format!("  {word}"), dim()),
            ],
            width,
        )));
    }
    lines
}

/// Draw `lines` (a list with links) scrolled, the link cursor shown.
fn draw_list(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    lines: Vec<Line<'static>>,
    links: Vec<(usize, Selection)>,
) {
    let height = area.height as usize;
    app.layout.total_rows = lines.len();
    app.layout.page = height;
    app.layout.pair_rows = Vec::new();
    let focused = app.focus == Focus::View;
    let link_row = if focused {
        app.link.and_then(|l| links.get(l)).map(|(row, _)| *row)
    } else {
        None
    };
    if let Some(row) = link_row.filter(|_| app.view_follow) {
        app.scroll = follow(lines.len(), height, row, app.scroll);
    } else {
        app.scroll = app.scroll.min(lines.len().saturating_sub(1));
    }
    let shown: Vec<Line<'static>> = lines
        .into_iter()
        .enumerate()
        .skip(app.scroll)
        .take(height)
        .map(|(i, l)| {
            if Some(i) == link_row {
                l.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect();
    for (i, (row, _)) in links.iter().enumerate() {
        if *row >= app.scroll && *row < app.scroll + height {
            let y = area.y + (*row - app.scroll) as u16;
            app.hits
                .push((Rect::new(area.x, y, area.width, 1), Hit::Link(i)));
        }
    }
    app.links = links.into_iter().map(|(_, s)| s).collect();
    frame.render_widget(Paragraph::new(shown), area);
}

fn is_wide(app: &App, width: u16) -> bool {
    match app.config.layout {
        LayoutMode::Split => true,
        LayoutMode::Stacked => false,
        LayoutMode::Auto => width >= SPLIT_VIEW_MIN,
    }
}

fn draw_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused =
        app.focus == Focus::View && matches!(app.mode, Mode::Normal | Mode::Details { .. });
    let block = pane_block(view_title(app), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.push((area, Hit::Pane(Focus::View)));
    let width = inner.width as usize;
    let sel = app.selection.clone();
    // Lists: the project summary, a directory, the units group.
    let mut links = Vec::new();
    let list = match &sel {
        Selection::Project => Some(summary(app, width, &mut links)),
        Selection::Dir(d) => Some(dir_view(app, d, width, &mut links)),
        Selection::Units => {
            let mut lines = Vec::new();
            for (u, info) in app.snapshot.units.iter().zip(&app.files.units) {
                links.push((lines.len(), Selection::Unit(u.unit.id.clone())));
                lines.push(Line::from(clipped(
                    vec![
                        Span::styled(
                            format!("{:<2} ", info.state.glyph()),
                            glyph_style(info.state.glyph()),
                        ),
                        Span::raw(u.unit.id.clone()),
                        Span::styled(format!("  {}", info.state.word()), dim()),
                    ],
                    width,
                )));
            }
            Some(lines)
        }
        _ => None,
    };
    if let Some(lines) = list {
        draw_list(frame, app, inner, lines, links);
        return;
    }
    app.links.clear();
    // C source: a file no unit owns, a header, an internal function.
    if let Some(source) = app.source.clone() {
        let mut head: Vec<Line<'static>> = Vec::new();
        if let Selection::Function(_, name) = &sel {
            if app.unit_view().is_some() {
                head.extend(wrapped(
                    &format!(
                        "{}() is internal: compared through the unit's exported functions",
                        short_symbol(name)
                    ),
                    width,
                    dim(),
                ));
            }
        }
        if let Some(note) = &source.note {
            head.extend(wrapped(note, width, Style::default().fg(Color::Yellow)));
        }
        let gutter = gutter_of(&source.lines);
        let body_h = (inner.height as usize).saturating_sub(head.len());
        app.layout.total_rows = source.lines.len();
        app.layout.page = body_h;
        app.layout.pair_rows = Vec::new();
        app.scroll = app.scroll.min(source.lines.len().saturating_sub(1));
        let mut lines = head;
        for l in source.lines.iter().skip(app.scroll).take(body_h) {
            lines.push(Line::from(cell(Some(l), gutter, width, app.hscroll)));
        }
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }
    // A unit's screen: header, pairs, checks.
    let Some(unit) = app.unit_view().cloned() else {
        frame.render_widget(
            Paragraph::new(wrapped("nothing to show", width, dim())),
            inner,
        );
        return;
    };
    let mut head = unit_header(app, &unit, width);
    if let Selection::File(p) = &sel {
        let internal: Vec<String> = app
            .files
            .file(p)
            .map(|f| {
                f.functions
                    .iter()
                    .filter(|x| !x.in_unit)
                    .map(|x| format!("{}()", short_symbol(&x.name)))
                    .collect()
            })
            .unwrap_or_default();
        if !internal.is_empty() {
            head.extend(wrapped(
                &format!(
                    "Internal: {} (compared through the unit's exported functions)",
                    internal.join(", ")
                ),
                width,
                dim(),
            ));
        }
    }
    let checks_rows = 2u16.min(inner.height.saturating_sub(head.len() as u16 + 1));
    let head_h = (head.len() as u16).min(inner.height.saturating_sub(checks_rows + 1));
    let [head_area, pairs_area, checks_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(head_h),
            Constraint::Min(1),
            Constraint::Length(checks_rows),
        ])
        .areas(inner);
    frame.render_widget(Paragraph::new(head), head_area);
    let wide = is_wide(app, pairs_area.width);
    let (pw, ph) = (pairs_area.width as usize, pairs_area.height as usize);
    let (_, _, total) = pair_window(&app.pairs, pw, wide, 0, 0, 0);
    app.scroll = app.scroll.min(total.saturating_sub(1));
    let (visible, starts, total) = pair_window(&app.pairs, pw, wide, app.scroll, ph, app.hscroll);
    app.layout.pair_rows = starts;
    app.layout.total_rows = total;
    app.layout.page = ph;
    frame.render_widget(Paragraph::new(visible), pairs_area);
    frame.render_widget(
        Paragraph::new(checks_lines(
            app,
            checks_area.width as usize,
            checks_rows as usize,
        )),
        checks_area,
    );
}

// ----- the activity panel ----------------------------------------------------------

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Row 1: the running command in words, or "Ready. Last: …"; its buttons on
/// the right.
fn activity_row(app: &mut App, area: Rect) -> Line<'static> {
    let width = area.width as usize;
    let mut buttons: Vec<(&'static str, &'static str)> = Vec::new();
    let (lead, lead_style) = match (&app.run, app.running) {
        (Some(run), true) => {
            let elapsed = run.started.elapsed();
            let frame = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
            buttons.push(("[Cancel x]", "x"));
            buttons.push(("[Details c]", "c"));
            (
                format!(
                    "{frame} {}: {} · {}",
                    run.narrator.label,
                    run.narrator.step(),
                    elapsed_words(elapsed)
                ),
                Style::default().fg(Color::Cyan),
            )
        }
        _ => {
            if app.try_again.is_some() {
                buttons.push(("[Try again t]", "t"));
            }
            if app.run.is_some() {
                buttons.push(("[Details c]", "c"));
            }
            let text = match &app.last {
                Some(last) => format!("Ready. Last: {last}"),
                None => "Ready.".to_string(),
            };
            (text, Style::default())
        }
    };
    let right: String = buttons.iter().map(|(b, _)| format!(" {b}")).collect();
    let right_w = width_of(&right);
    let lead = ellipsis(&format!(" {lead}"), width.saturating_sub(right_w));
    let pad = width.saturating_sub(width_of(&lead) + right_w);
    let mut x = area.x + (width_of(&lead) + pad) as u16;
    for (b, key) in &buttons {
        x += 1;
        let w = width_of(b) as u16;
        // Only a button drawn whole is clickable (review ENG-B-5).
        if x + w <= area.x + area.width {
            app.hits
                .push((Rect::new(x, area.y, w, 1), Hit::Activity(key)));
        }
        x += w;
    }
    Line::from(vec![
        Span::styled(lead, lead_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, bold()),
    ])
}

/// Row 2: the notice, else the plan summary, else the hand-offs' state.
fn notice_row(app: &App, width: usize) -> Line<'static> {
    if let Some(n) = &app.notice {
        return Line::from(Span::styled(
            ellipsis(&format!(" {}", n.text), width),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(p) = &app.plan_notice {
        return Line::from(Span::styled(
            ellipsis(&format!(" {p}"), width),
            Style::default(),
        ));
    }
    if let Some(aw) = app.awaiting.iter().find(|aw| aw.response_present) {
        let who = aw.attempt.as_deref().map(short_id).unwrap_or_default();
        return Line::from(Span::styled(
            ellipsis(
                &format!(" The answer for {who} is present — select it and choose Resume (R)"),
                width,
            ),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(aw) = app.awaiting.last() {
        let who = aw.attempt.as_deref().map(short_id).unwrap_or_default();
        return Line::from(Span::styled(
            ellipsis(
                &format!(" {who} is waiting for your answer at {}", aw.path.display()),
                width,
            ),
            dim(),
        ));
    }
    Line::from("")
}

/// The keys that work in the open overlay, when one is open (review USE-3);
/// `None` in the panes.
fn overlay_hints(app: &App) -> Option<Vec<(&'static str, &'static str)>> {
    Some(match &app.mode {
        Mode::Normal => return None,
        Mode::Menu(_) => vec![("↑↓", "move"), ("Enter", "choose"), ("Esc", "close")],
        Mode::Dialog(c) => {
            let mut h = vec![("↑↓", "scroll")];
            if c.dialog.armed {
                h.push(("←→", "button"));
                h.push(("Enter", "press"));
            }
            h.push(("Esc", "cancel"));
            h
        }
        Mode::Note { .. } | Mode::EditNote { .. } => {
            vec![("Enter", "continue"), ("Esc", "cancel")]
        }
        Mode::Details { .. } => {
            let mut h = vec![("↑↓", "scroll"), ("c/Esc", "close")];
            if app.running {
                h.push(("x", "cancel"));
            }
            h
        }
        Mode::Help { .. } => vec![
            ("↑↓", "scroll"),
            (
                "m",
                if app.mouse {
                    "turn mouse off"
                } else {
                    "turn mouse on"
                },
            ),
            ("any other key", "close"),
        ],
        Mode::Verdict { .. } => vec![("↑↓", "check"), ("PgDn", "detail"), ("Esc", "close")],
        Mode::Diff { .. } => vec![("↑↓", "scroll"), ("Esc", "close")],
    })
}

/// The focused pane's keys, in priority order.
fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut h: Vec<(&'static str, &'static str)> = Vec::new();
    match app.focus {
        Focus::Files => {
            h.push(("↑↓", "move"));
            h.push(("←→", "fold/open"));
            h.push(("Enter", "actions"));
        }
        Focus::View => {
            // Enter goes to a link only once one is chosen (review NEW-6).
            let chosen = app.link.and_then(|l| app.links.get(l)).is_some();
            if app.links.is_empty() {
                h.push(("↑↓", "scroll"));
                h.push(("←→", "side/back"));
                h.push(("Enter", "actions"));
            } else if chosen {
                h.push(("↑↓", "choose"));
                h.push(("Enter", "go there"));
            } else {
                h.push(("↑↓", "choose"));
                h.push(("Enter", "actions"));
            }
        }
    }
    if app.running {
        h.insert(0, ("x", "cancel"));
    }
    h.push(("Tab", "pane"));
    h.push(("Esc", "back"));
    if app.focus == Focus::View && !app.pairs.is_empty() {
        h.push(("]f", "next pair"));
    }
    h.push(("c", "details"));
    h.push(("g", "re-read"));
    h
}

fn draw_hints(frame: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;
    // In an overlay, only its keys; in the panes, the focused pane's, with
    // `? help` and `q quit` always last.
    type Keys = Vec<(&'static str, &'static str)>;
    let (list, tail): (Keys, Keys) = match overlay_hints(app) {
        Some(keys) => (keys, Vec::new()),
        None => (hints(app), vec![("?", "help"), ("q", "quit")]),
    };
    let entry = |(k, v): &(&str, &str)| format!(" {k} {v} ");
    let tail_w: usize = tail.iter().map(|e| width_of(&entry(e)) + 1).sum();
    let mut chosen = Vec::new();
    let mut used = tail_w;
    for e in list {
        let w = width_of(&entry(&e)) + 1;
        if used + w > width {
            continue; // drop whole entries, never cut one
        }
        used += w;
        chosen.push(e);
    }
    chosen.extend(tail);
    let mut spans = Vec::new();
    let mut x = area.x;
    for (k, v) in &chosen {
        let text = entry(&(k, v));
        let w = width_of(&text) as u16;
        if x + w > area.x + area.width {
            break;
        }
        app.hits.push((Rect::new(x, area.y, w, 1), Hit::Hint(k)));
        spans.push(Span::styled(format!(" {k}"), bold()));
        spans.push(Span::styled(format!(" {v} "), dim()));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ----- overlays ------------------------------------------------------------------------

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn pct(n: u16, p: u16) -> u16 {
    u16::try_from(u32::from(n) * u32::from(p) / 100).unwrap_or(n)
}

/// An overlay of pre-wrapped rows, scrolled by ROWS: `scroll` is clamped and
/// returned; `prompt` sits on the bottom border.
fn overlay(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: &[Line<'static>],
    scroll: usize,
    prompt: Option<&str>,
    more_key: Option<&str>,
) -> usize {
    frame.render_widget(Clear, area);
    let page = area.height.saturating_sub(2) as usize;
    let scroll = scroll.min(rows.len().saturating_sub(page));
    let more = rows.len() > scroll + page;
    let mut block = Block::default().borders(Borders::ALL).title(Span::styled(
        safe(title, area.width.saturating_sub(4) as usize),
        bold(),
    ));
    let hint = more_key.map(|k| format!(" — {k}")).unwrap_or_default();
    let bottom = match (prompt, more) {
        (Some(p), true) => Some(format!(" ↓ more below{hint} · {p} ")),
        (Some(p), false) => Some(format!(" {p} ")),
        (None, true) => Some(format!(" ↓ more below{hint} ")),
        (None, false) => None,
    };
    if let Some(b) = bottom {
        block = block.title_bottom(Line::from(Span::styled(
            clip(&b, area.width.saturating_sub(4) as usize),
            bold(),
        )));
    }
    let visible: Vec<Line<'static>> = rows.iter().skip(scroll).take(page).cloned().collect();
    frame.render_widget(Paragraph::new(visible).block(block), area);
    scroll
}

/// The scroll of a text-input overlay whose first `input_rows` rows are the
/// input (the cursor on the last).
fn follow_cursor(input_rows: usize, total_rows: usize, rect: Rect) -> usize {
    let page = rect.height.saturating_sub(2) as usize;
    let trailing = total_rows.saturating_sub(input_rows);
    input_rows.saturating_sub(page.saturating_sub(trailing).max(1))
}

const HELP_INTRO: &[&str] = &[
    "Move with the arrow keys, or click.",
    "Enter (or a double click) shows what you can do with the selection.",
    "? shows this screen.",
];

const HELP_MOUSE: &[&str] = &[
    "Click: select a row and its pane; a click on ▸ or ▾ opens or folds it. Double click: what \
     you can do (as Enter). The wheel scrolls the open menu or dialog, else the pane under the \
     pointer. A click on a key in the bottom bar presses it. A dialog's buttons answer a click \
     once it is ready and has been open a second.",
    "To select text, hold Shift (most terminals) or Option (iTerm2); in Terminal, ⌘R turns \
     its mouse reporting off and on — or turn the mouse off here. In tmux, `set -g mouse on`. \
     If clicks print odd characters, start with --no-mouse.",
];

const HELP_KEYS: &[(&str, &str)] = &[
    ("↑ ↓", "move in the focused pane (tree: a row; View and details: scroll)"),
    ("← →", "tree: fold / open, or to the parent / into the View; View: scroll sideways, ← at the edge back to Files"),
    ("Enter", "tree: the action menu · menu: choose · dialog: the focused button"),
    ("Esc", "close a menu, dialog or overlay; View: back to Files; Files: back along your jumps"),
    ("Backspace", "back along your jumps"),
    ("Tab", "the other pane (below 80 columns: Files ⇄ View)"),
    ("PgUp PgDn Home End", "page, or go to the ends"),
    ("c", "the activity details: the command, every event, its exit"),
    ("g", "re-read the project"),
    ("t", "try again, after a refusal"),
    ("q", "quit (a dialog while a command runs)"),
    ("m (in Help)", "the mouse on or off"),
    ("a m e E r R x d v", "shortcuts for the selection's menu items (Accept, Modify, Hand edit, kept edit, Retry, Resume, Cancel, Compare, Show the checks)"),
    ("j k  ]f [f  J K", "also: move, next/previous pair, next/previous unit"),
];

const HELP_LEGEND: &[(&str, &str)] = &[
    ("✗", "failing: the current verdict is red"),
    (
        "⚠",
        "needs attention: a cause you can fix, named in the View",
    ),
    (
        "✓",
        "migrated (steered / by hand when a note or an edit made it)",
    ),
    (
        "✓?",
        "verified, origin not recorded: judged, but no attempt is known to have produced it",
    ),
    ("◐", "tried: attempts exist, none accepted"),
    ("◇", "planned: no attempt yet"),
    ("⊘", "blocked"),
    ("!", "changed since the scan"),
    ("+", "not scanned yet"),
    (
        "?",
        "missing: the scan recorded it, the tree no longer has it",
    ),
    ("·", "a header"),
    ("–", "no exported functions (never planned)"),
    ("○", "not in the plan"),
];

const HELP_ROUTES: &[&str] = &[
    "Model work happens in chat, which is not built yet. Today's routes:",
    "  harness migrate <unit>: a fresh translation (the blind hand-off, or a live provider)",
    "  harness-mcp in a separate Claude Code session: steer attempts (README)",
    "  harness override <unit> <dir>: record an outside edit as a hand edit",
    "Every act shows its exact command and waits until it is ready: keys typed or pasted ahead never answer it, and a held Enter never runs anything.",
];

/// Help's rows, and which of them are the mouse on/off line (a click there
/// is `m`).
fn help_rows(width: usize, mouse: bool) -> (Vec<Line<'static>>, std::ops::Range<usize>) {
    let mut rows = Vec::new();
    for l in HELP_INTRO {
        rows.extend(wrapped(l, width, bold()));
    }
    rows.push(Line::from(""));
    let start = rows.len();
    let toggle = if mouse {
        "Mouse: on — click here, or press m, to turn it off"
    } else {
        "Mouse: off — press m to turn it on"
    };
    rows.extend(wrapped(toggle, width, bold()));
    let toggle = start..rows.len();
    for l in HELP_MOUSE {
        rows.extend(wrapped(l, width, Style::default()));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled("Keys", bold())));
    for (k, v) in HELP_KEYS {
        rows.extend(wrapped(&format!("{k:<18} {v}"), width, Style::default()));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled("States", bold())));
    for (g, v) in HELP_LEGEND {
        rows.extend(wrapped(&format!("{g:<3} {v}"), width, Style::default()));
    }
    rows.push(Line::from(""));
    for l in HELP_ROUTES {
        rows.extend(wrapped(l, width, Style::default()));
    }
    (rows, toggle)
}

/// The run panel's lines (the activity details): the argv, every event
/// line, then its exit.
fn details_rows(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match &app.run {
        None => lines.push(Line::from(Span::styled("no command yet", dim()))),
        Some(run) => {
            lines.extend(wrapped(
                &format!("$ {}", shell_line(&run.argv)),
                width,
                dim(),
            ));
            for l in &run.lines {
                lines.extend(wrapped(&l.text, width, tone(l.tone)));
            }
            if let Some(exit) = &run.exit {
                let t = if exit == "exit 0" {
                    Tone::Good
                } else {
                    Tone::Warn
                };
                lines.push(Line::from(Span::styled(safe(exit, width), tone(t))));
            }
        }
    }
    for aw in &app.awaiting {
        let who = aw.attempt.as_deref().map(short_id).unwrap_or_default();
        let text = if aw.response_present {
            format!("response present for {who} — Resume asks first")
        } else {
            format!("{who} awaits a response at {}", aw.path.display())
        };
        lines.extend(wrapped(&text, width, Style::default().fg(Color::Yellow)));
    }
    lines
}

fn draw_menu(frame: &mut Frame, app: &mut App, area: Rect, m: &Menu) {
    let width = 64u16.min(area.width.saturating_sub(2)).max(20);
    let inner_w = width.saturating_sub(2) as usize;
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut item_rows = Vec::new();
    let mut separated = false;
    for (i, it) in m.items.iter().enumerate() {
        if it.model && !separated {
            separated = true;
            rows.push(Line::from(Span::styled(
                clip(&format!("── {MODEL_SEPARATOR} ──"), inner_w),
                dim(),
            )));
        }
        let accel = it.accel.map(|a| format!(" {a}")).unwrap_or_default();
        let aw = width_of(&accel);
        let mut label = display::line(&it.label);
        if let Some(why) = &it.greyed {
            label = format!("{label} — {}", display::line(why));
        }
        let label = ellipsis(&label, inner_w.saturating_sub(aw + 2));
        let pad = inner_w.saturating_sub(2 + width_of(&label) + aw);
        let mut style = if it.greyed.is_some() {
            dim()
        } else {
            Style::default()
        };
        if i == m.focus {
            style = style.add_modifier(Modifier::REVERSED);
        }
        item_rows.push((i, rows.len()));
        rows.push(Line::from(vec![Span::styled(
            format!("  {label}{}{accel}", " ".repeat(pad)),
            style,
        )]));
    }
    let items_h = (rows.len() as u16 + 2).min(area.height);
    if let Some(why) = &m.footer {
        rows.push(Line::from(""));
        rows.extend(wrapped(
            &format!("Why not: {why}"),
            inner_w,
            Style::default().fg(Color::Yellow),
        ));
    }
    // The items stay where they were when a footer appears — it grows the
    // menu downward — so the next click lands on the item it aimed at
    // (review USE-B-7); only a footer with no room below moves it up.
    let height = (rows.len() as u16 + 2).min(area.height);
    let base = centered(area, width, items_h);
    let bottom = area.y + area.height;
    let rect = Rect {
        y: base.y.min(bottom.saturating_sub(height)).max(area.y),
        height,
        ..base
    };
    let title = format!(" {} ", node_name(app, &app.selection));
    overlay(
        frame,
        rect,
        &title,
        &rows,
        0,
        Some("Enter choose · Esc close"),
        None,
    );
    app.hits.push((rect, Hit::Dialog));
    for (i, row) in item_rows {
        if (row as u16) < rect.height.saturating_sub(2) {
            app.hits.push((
                Rect::new(
                    rect.x + 1,
                    rect.y + 1 + row as u16,
                    rect.width.saturating_sub(2),
                    1,
                ),
                Hit::MenuItem(i),
            ));
        }
    }
}

fn draw_dialog(frame: &mut Frame, app: &mut App, area: Rect) {
    let Mode::Dialog(c) = &mut app.mode else {
        return;
    };
    let c: &mut Confirm = c;
    let width = DIALOG_COLUMNS.min(area.width.saturating_sub(2));
    let inner_w = width.saturating_sub(2) as usize;
    let mut rows: Vec<Line<'static>> = Vec::new();
    // A title too long for the border is repeated whole, first (review NEW-9).
    let title_w = width_of(&display::line(&c.title));
    if title_w + 2 > inner_w {
        rows.extend(wrapped(&c.title, inner_w, bold()));
    }
    for b in &c.body {
        rows.extend(wrapped(b, inner_w, Style::default()));
    }
    if let crate::app::Purpose::Act(p) = &c.purpose {
        rows.push(Line::from(""));
        // Filtered but NEVER cut: the whole command must be seen (it scrolls).
        let argv = Sanitizer::default().push(&format!("Command: {}", shell_line(&p.argv)));
        for row in hard_wrap(&argv, inner_w) {
            rows.push(Line::from(Span::styled(row, bold())));
        }
    }
    // Two rows pinned at the bottom: the dialog's state, its buttons.
    let height = (rows.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let rect = centered(area, width, height);
    let page = rect.height.saturating_sub(4) as usize;
    let total = rows.len();
    c.dialog.scroll = c.dialog.scroll.min(total.saturating_sub(page));
    let scroll = c.dialog.scroll;
    // The argv must be seen to its end before the dialog can arm — and only
    // on a terminal large enough to show it (review SAFE-5).
    let usable = inner_w >= 20 && page >= 2;
    c.dialog.usable = usable;
    // The user sees "ready" from this frame on: a click may answer it
    // (review SAFE-B-7).
    c.dialog.shown_armed = c.dialog.shown_armed || (usable && c.dialog.armed);
    c.dialog.seen = c.dialog.seen || (usable && scroll + page >= total);
    // Too small: this frame shows a note in place of the words — the
    // dialog's own words are never changed (review NEW-1).
    let rows: Vec<Line<'static>> = if usable {
        rows
    } else {
        wrapped(
            "The terminal is too small for this dialog; enlarge it, or press Esc.",
            inner_w.max(1),
            Style::default().fg(Color::Yellow),
        )
    };
    let (total, scroll) = if usable {
        (total, scroll)
    } else {
        (rows.len(), 0)
    };
    // The buttons and the dialog's state, pinned at the bottom.
    let mut spans = Vec::new();
    let mut spots = Vec::new();
    let mut x = 1u16;
    for (i, b) in c.dialog.buttons.iter().enumerate() {
        let text = format!("[ {}  {} ]", b.label, b.key);
        let mut style = if i == 0 || c.dialog.armed {
            bold()
        } else {
            dim()
        };
        if i == c.dialog.focus {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let w = width_of(&text) as u16;
        spots.push((i, x, w));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(text, style));
        x += w + 1;
    }
    let state = if usable {
        c.dialog.state_text()
    } else {
        "too small to show".into()
    };
    let state_style = if c.dialog.armed {
        Style::default().fg(Color::Green)
    } else if c.dialog.too_soon {
        Style::default().fg(Color::Yellow)
    } else {
        dim()
    };
    let state_line = Line::from(Span::styled(
        clip(&format!(" {state}"), inner_w),
        state_style,
    ));
    let buttons = Line::from(clipped(spans, inner_w));
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(bold())
        .title(Span::styled(
            ellipsis(&format!(" {} ", c.title), inner_w),
            bold(),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut visible: Vec<Line<'static>> = rows.into_iter().skip(scroll).take(page).collect();
    if scroll + page < total {
        if let Some(last) = visible.last_mut() {
            *last = Line::from(Span::styled(
                clip("↓ more below — scroll to the end", inner_w),
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    frame.render_widget(Paragraph::new(visible), body);
    let state_row = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        1,
    );
    frame.render_widget(Paragraph::new(state_line), state_row);
    let bar = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    frame.render_widget(Paragraph::new(buttons), bar);
    app.hits.push((rect, Hit::Dialog));
    // Only a button drawn whole is clickable, on a frame that can show the
    // dialog: never one the bar cut, nor one past the dialog's edge.
    if usable {
        for (i, bx, w) in spots {
            if bx + w <= inner.width {
                app.hits
                    .push((Rect::new(inner.x + bx, bar.y, w, 1), Hit::Button(i)));
            }
        }
    }
}

fn draw_overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.mode.clone() {
        Mode::Normal => {}
        Mode::Menu(m) => draw_menu(frame, app, area, &m),
        Mode::Dialog(_) => draw_dialog(frame, app, area),
        Mode::Details { scroll } => {
            // Below 80 columns over the panes, the activity rows and the
            // hint bar still showing — so it can be closed with a click
            // (review USE-B-4; §1 "Activity and the hint bar stay").
            let rect = if area.width < SINGLE_PANE_BELOW {
                Rect::new(area.x, area.y, area.width, area.height.saturating_sub(3))
            } else {
                let h = area.height / 2;
                Rect::new(
                    area.x,
                    area.y + area.height.saturating_sub(h + 3),
                    area.width,
                    h,
                )
            };
            let rows = details_rows(app, rect.width.saturating_sub(2) as usize);
            let title = match &app.run {
                Some(r) => format!(" Details: {} ", r.narrator.label),
                None => " Details ".into(),
            };
            let shown = overlay(
                frame,
                rect,
                &title,
                &rows,
                scroll,
                Some("c or Esc close"),
                Some("↑↓"),
            );
            app.mode = Mode::Details { scroll: shown };
            app.hits.push((rect, Hit::Overlay));
        }
        Mode::Help { scroll } => {
            let rect = centered(
                area,
                pct(area.width, 86).max(40),
                pct(area.height, 86).max(10),
            );
            let (rows, toggle) = help_rows(rect.width.saturating_sub(2) as usize, app.mouse);
            let shown = overlay(
                frame,
                rect,
                " Help ",
                &rows,
                scroll,
                Some("any other key closes"),
                Some("↓"),
            );
            app.mode = Mode::Help { scroll: shown };
            app.hits.push((rect, Hit::Overlay));
            // The mouse on/off line answers a click as `m` does.
            let page = rect.height.saturating_sub(2) as usize;
            for row in toggle.filter(|r| *r >= shown && *r < shown + page) {
                let y = rect.y + 1 + (row - shown) as u16;
                app.hits.push((
                    Rect::new(rect.x + 1, y, rect.width.saturating_sub(2), 1),
                    Hit::Hint("m"),
                ));
            }
        }
        Mode::Verdict { selected, scroll } => {
            let Some(v) = app.shown_verdict() else {
                return;
            };
            let rect = centered(
                area,
                pct(area.width, 85).max(40),
                pct(area.height, 80).max(8),
            );
            let inner = rect.width.saturating_sub(2) as usize;
            let mut rows: Vec<Line<'static>> = Vec::new();
            for (i, c) in v.checks.iter().enumerate() {
                let mut style = if c.passed {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                };
                if i == selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let mark = if c.passed { "✓" } else { "✗" };
                rows.extend(wrapped(
                    &format!("{mark} {} ({})", check_words(&c.name), c.name),
                    inner,
                    style,
                ));
            }
            if let Some(check) = v.checks.get(selected) {
                rows.push(Line::from(""));
                for raw in check.detail.lines() {
                    rows.extend(wrapped(raw, inner, Style::default()));
                }
            }
            let shown = overlay(
                frame,
                rect,
                " The checks ",
                &rows,
                scroll,
                Some("↑↓ check · Esc"),
                Some("PgDn"),
            );
            app.mode = Mode::Verdict {
                selected,
                scroll: shown,
            };
            app.hits.push((rect, Hit::Overlay));
        }
        Mode::Diff { scroll, title, .. } => {
            let rect = centered(
                area,
                pct(area.width, 90).max(40),
                pct(area.height, 85).max(8),
            );
            let inner = rect.width.saturating_sub(2) as usize;
            if app.diff_rows.as_ref().map(|(w, _)| *w) != Some(inner) {
                let Mode::Diff { lines, .. } = &app.mode else {
                    return;
                };
                let rows = lines
                    .iter()
                    .flat_map(|l| {
                        let style = if l.starts_with("+++") || l.starts_with("---") {
                            bold()
                        } else if l.starts_with('+') {
                            Style::default().fg(Color::Green)
                        } else if l.starts_with('-') {
                            Style::default().fg(Color::Red)
                        } else if l.starts_with("@@") {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        };
                        hard_wrap(&display::line(l), inner)
                            .into_iter()
                            .map(move |row| Line::from(Span::styled(row, style)))
                    })
                    .collect();
                app.diff_rows = Some((inner, rows));
            }
            let rows = app
                .diff_rows
                .as_ref()
                .map_or(&[][..], |(_, r)| r.as_slice());
            let shown = overlay(
                frame,
                rect,
                &format!(" Compare: {title} "),
                rows,
                scroll,
                Some("Esc"),
                Some("↓"),
            );
            if let Mode::Diff { scroll, .. } = &mut app.mode {
                *scroll = shown;
            }
            app.hits.push((rect, Hit::Overlay));
        }
        Mode::Note { input, attempt, .. } => {
            let rect = centered(
                area,
                pct(area.width, 80).max(40),
                pct(area.height, 40).max(8),
            );
            let inner = rect.width.saturating_sub(2) as usize;
            let mut rows: Vec<Line<'static>> =
                hard_wrap(&display::line(&format!("{input}▏")), inner)
                    .into_iter()
                    .map(Line::from)
                    .collect();
            let input_rows = rows.len();
            rows.push(Line::from(""));
            rows.extend(wrapped(
                &format!(
                    "{}/{} bytes · the command is shown before it runs",
                    input.len(),
                    crate::app::MAX_NOTE_BYTES
                ),
                inner,
                dim(),
            ));
            let at = follow_cursor(input_rows, rows.len(), rect);
            overlay(
                frame,
                rect,
                &format!(" Modify {}: your note for the model ", short_id(&attempt)),
                &rows,
                at,
                Some("Enter continue · Esc cancel (the note is kept)"),
                None,
            );
            app.hits.push((rect, Hit::Overlay));
        }
        Mode::EditNote { input, stage, .. } => {
            let rect = centered(
                area,
                pct(area.width, 80).max(40),
                pct(area.height, 40).max(8),
            );
            let inner = rect.width.saturating_sub(2) as usize;
            let mut rows: Vec<Line<'static>> =
                hard_wrap(&display::line(&format!("{input}▏")), inner)
                    .into_iter()
                    .map(Line::from)
                    .collect();
            let input_rows = rows.len();
            rows.push(Line::from(""));
            rows.extend(wrapped(
                &format!(
                    "an optional note (empty = none) · {}/{} bytes · staged as exactly \
                     src/logic.rs + src/ffi.rs in {}",
                    input.len(),
                    crate::app::MAX_EDIT_NOTE_BYTES,
                    stage.display()
                ),
                inner,
                dim(),
            ));
            let at = follow_cursor(input_rows, rows.len(), rect);
            overlay(
                frame,
                rect,
                " Hand edit: a note for the human attempt ",
                &rows,
                at,
                Some("Enter continue · Esc keep for later"),
                None,
            );
            app.hits.push((rect, Hit::Overlay));
        }
    }
}

/// Draw the whole cockpit, recording the layout and the clickable regions.
pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = frame.area();
    app.layout.frame = area;
    let [main, act1, act2, hint] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
    let single = area.width < SINGLE_PANE_BELOW;
    app.layout.single_pane = single;
    if single {
        match app.focus {
            Focus::Files => draw_files(frame, app, main),
            Focus::View => draw_view(frame, app, main),
        }
    } else {
        let files_w = if area.width >= WIDE_FROM {
            FILES_WIDE
        } else {
            FILES_NARROW
        };
        // The chat's side (§10, from 150 columns) is taken only once the chat
        // exists: an empty strip read as broken (review USE-15; DECISIONS).
        let [files, view] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(files_w), Constraint::Min(20)])
            .areas(main);
        draw_files(frame, app, files);
        draw_view(frame, app, view);
    }
    let row = activity_row(app, act1);
    frame.render_widget(Paragraph::new(row), act1);
    frame.render_widget(Paragraph::new(notice_row(app, act2.width as usize)), act2);
    draw_hints(frame, app, hint);
    draw_overlay(frame, app, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app, app_of, arm, attempt, key, LIB_C, PROVENANCE};
    use crate::app::{Act, Command, Purpose, RunLine, RunPanel};
    use crate::files::{Cause, FileInfo, FunctionInfo, Origin};
    use crate::narrate::Narrator;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Instant;

    fn text(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    fn render(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The scratch copy's path appears in some texts: replaced so goldens
    /// are machine-independent (the project row shows the copy's name).
    fn golden(name: &str, app: &App, buffer: &Buffer) {
        // The scratch copy's name holds the pid: its digits are masked by
        // position (a name cut with `…` keeps its length).
        let mut got = String::new();
        let text = text(buffer).replace(&app.config.target.display().to_string(), "<target>");
        let mut rest = text.as_str();
        while let Some(i) = rest.find("harness-tui-") {
            got.push_str(&rest[..i + "harness-tui-".len()]);
            rest = &rest[i + "harness-tui-".len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(rest.len());
            got.extend(
                rest[..end]
                    .chars()
                    .map(|c| if c.is_ascii_digit() { '0' } else { c }),
            );
            rest = &rest[end..];
        }
        got.push_str(rest);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name);
        if std::env::var("RUHARNESS_UPDATE_TUI_GOLDENS").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &got).unwrap();
            return;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing golden {} — render it with RUHARNESS_UPDATE_TUI_GOLDENS=1 and review it",
                path.display()
            )
        });
        if want != got {
            let (i, (w, g)) = want
                .lines()
                .chain(std::iter::repeat("<end>"))
                .zip(got.lines().chain(std::iter::repeat("<end>")))
                .enumerate()
                .find(|(_, (w, g))| w != g)
                .unwrap_or((0, ("", "")));
            panic!(
                "the view differs from golden {name} at row {}:\n  golden: {w}\n  now:    {g}\n\
                 A view change updates its golden in the same commit \
                 (RUHARNESS_UPDATE_TUI_GOLDENS=1 cargo test -p harness-tui) so the diff is reviewed.\n\
                 Full render:\n{got}",
                i + 1
            );
        }
    }

    fn row_with<'a>(screen: &'a str, needle: &str) -> &'a str {
        screen
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no row with {needle:?} in\n{screen}"))
    }

    /// Golden 1: 120 columns, an owned file selected: the tree with its
    /// states in words, the View's pairs side by side with cut marks, the
    /// checks in words, the two activity rows, the hint bar.
    #[test]
    fn golden_120_columns() {
        let mut app = app("g120");
        app.select(Selection::File(LIB_C.into()));
        let buffer = render(&mut app, 120, 40);
        golden("wide.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(row_with(&screen, "✓ lib.c").contains("migrated"));
        assert!(screen.contains("⇄"));
        assert!(screen.contains("›"), "a cut code line ends with a dim ›");
        assert!(screen.contains("same outputs as C"));
        assert!(row_with(&screen, "Ready.").starts_with(" Ready."));
        assert!(screen.lines().last().is_some_and(|l| l.ends_with("q quit")));
    }

    /// Golden 2: 80 columns — one layout, the Files pane 24 wide, the pairs
    /// stacked (the View is under 78 columns); below 80, one pane at a time.
    #[test]
    fn golden_80_columns_and_below() {
        let mut app = app("g80");
        app.select(Selection::Unit("u-lib".into()));
        let buffer = render(&mut app, 80, 30);
        golden("narrow.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(
            screen.lines().any(|l| l.contains("│C    ")),
            "stacked:\n{screen}"
        );
        assert!(!screen.contains(" ⇄ "));
        // --layout split forbids stacking; below 80 columns, one pane.
        app.config.layout = LayoutMode::Split;
        assert!(text(&render(&mut app, 80, 30)).contains(" ⇄ "));
        let narrow = text(&render(&mut app, 70, 30));
        assert!(
            narrow.contains(" Files ") && !narrow.contains("⇄"),
            "{narrow}"
        );
        crate::app::tests::code(&mut app, KeyCode::Tab);
        let narrow = text(&render(&mut app, 70, 30));
        assert!(
            !narrow.contains(" Files ") && narrow.contains("u-lib"),
            "{narrow}"
        );
        // Activity stays at every width.
        assert!(narrow.contains("Ready."));
    }

    /// Golden 3: a tree showing every glyph, each with its word.
    #[test]
    fn golden_every_glyph() {
        let mut app = app_of("targets/zopfli", "gglyph");
        let states = [
            UnitState::Blocked,
            UnitState::Attention(Cause::ChangedOutside),
            UnitState::Failing,
            UnitState::Migrated(Origin::Pipeline),
            UnitState::Migrated(Origin::Steered),
            UnitState::Migrated(Origin::Human),
            UnitState::OriginUnknown,
            UnitState::Tried,
            UnitState::Planned,
            UnitState::Other("in-progress".into()),
        ];
        for (i, s) in states.into_iter().enumerate() {
            app.files.units[i].state = s;
        }
        // A blocked unit owns no file (§2.3: ⊘ on unit rows only): its file
        // is nobody's, as `files::build` leaves it (review USE-7).
        for f in app.files.files.iter_mut().filter(|f| f.owner == Some(0)) {
            f.owner = None;
            f.state = FileState::NotInPlan;
        }
        let extra = [
            ("src/zopfli/gone.c", FileState::Missing),
            ("src/zopfli/edited.c", FileState::Changed),
            ("src/zopfli/fresh.c", FileState::New),
            ("src/zopfli/statics.c", FileState::NoExports),
            ("src/zopfli/loose.c", FileState::NotInPlan),
        ];
        for (path, state) in extra {
            app.files.files.push(FileInfo {
                path: path.into(),
                state,
                owner: None,
                functions: vec![FunctionInfo {
                    name: "f".into(),
                    line: 1,
                    in_unit: false,
                }],
            });
        }
        app.files.files.sort_by(|a, b| a.path.cmp(&b.path));
        app.rows = crate::tree::rows(&app.snapshot, &app.files, &app.walk, &app.expansion);
        // The directory's View lists every file with its glyph and word; the
        // units group's View every unit.
        app.select(Selection::Dir("src/zopfli".into()));
        let buffer = render(&mut app, 120, 50);
        golden("glyphs.txt", &app, &buffer);
        app.select(Selection::Units);
        let screen = format!("{}{}", text(&buffer), text(&render(&mut app, 120, 50)));
        for (glyph, word) in [
            ("⊘", "blocked"),
            ("⚠", "needs attention"),
            ("✗", "failing"),
            ("✓", "migrated"),
            ("✓?", "origin not recorded"),
            ("◐", "tried"),
            ("◇", "planned"),
            ("?", "missing"),
            ("!", "changed since scan"),
            ("+", "not scanned yet"),
            ("·", "header"),
            ("–", "no exported functions"),
            ("○", "not in the plan"),
        ] {
            assert!(
                screen
                    .lines()
                    .any(|l| l.contains(&format!("{glyph} ")) && l.contains(word)),
                "{glyph} {word} missing:\n{screen}"
            );
        }
    }

    /// Golden 4: a menu with a greyed item, and its reason in the footer
    /// once Enter is pressed on it.
    #[test]
    fn golden_menu_with_a_greyed_item() {
        let mut app = app("gmenu");
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        app.open_menu();
        let Mode::Menu(m) = &mut app.mode else {
            panic!()
        };
        m.focus = m.items.iter().position(|i| i.greyed.is_some()).unwrap();
        app.on_key(KeyEvent::from(KeyCode::Enter), Instant::now());
        let buffer = render(&mut app, 120, 30);
        golden("menu.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(
            screen.contains("Why not: 1 file changed or new since the scan — scan first"),
            "{screen}"
        );
        assert!(screen.contains("Next step: 1 file changed since the scan"));
    }

    fn running(app: &mut App) {
        let argv: Vec<OsString> = [
            "/opt/ruharness/bin/harness",
            "--json",
            "verify",
            "u-lib",
            "--target=/work/case",
        ]
        .map(OsString::from)
        .to_vec();
        let mut narrator = Narrator::new("Re-check u-lib", &argv);
        narrator.on_event(&crate::events::Event::Check {
            unit: "u-lib".into(),
            name: "differential-driver".into(),
            passed: true,
            detail: "183832 bytes identical".into(),
        });
        let pending = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        app.running = true;
        app.run = Some(RunPanel {
            argv,
            lines: vec![
                RunLine {
                    tone: Tone::Good,
                    text: "[check] symbol-set ✓ 1 exported symbol(s) match".into(),
                },
                RunLine {
                    tone: Tone::Good,
                    text: "[check] differential-driver ✓ 183832 bytes identical".into(),
                },
            ],
            exit: None,
            saw_result: false,
            expect_attempt: None,
            act: Act::Verify,
            recorded: false,
            cleanup: None,
            narrator,
            // The elapsed time shows as whole seconds: a start in the past
            // that renders the same whatever the test's speed.
            started: Instant::now() - std::time::Duration::from_millis(41_050),
            plan_changes: 0,
            pending,
        });
    }

    /// Golden 5: the activity panel while a command runs — the step in
    /// words, the elapsed time, Cancel and Details; `x cancel` leads the
    /// hint bar.
    #[test]
    fn golden_activity_while_running() {
        let mut app = app("grun");
        running(&mut app);
        let buffer = render(&mut app, 120, 20);
        let screen = text(&buffer);
        let row = row_with(&screen, "Re-check u-lib");
        assert!(
            row.contains("Checked: same outputs as C — passed · 41 s"),
            "{row}"
        );
        assert!(row.ends_with("[Cancel x] [Details c]"), "{row}");
        // The spinner frame depends on the clock: masked for the golden.
        let masked = Buffer::with_lines(
            screen
                .lines()
                .map(|l| SPINNER.iter().fold(l.to_string(), |l, f| l.replace(f, "*")))
                .collect::<Vec<_>>(),
        );
        golden("running.txt", &app, &masked);
        assert!(screen.lines().last().unwrap().starts_with(" x cancel"));
    }

    /// Golden 6: the details (`c`): the argv, every event line, the exit.
    #[test]
    fn golden_details() {
        let mut app = app("gdetails");
        running(&mut app);
        app.running = false;
        if let Some(run) = app.run.as_mut() {
            run.exit = Some("exit 0".into());
        }
        app.last = Some("Re-check u-lib — GREEN — all 8 checks passed (41 s)".into());
        key(&mut app, 'c');
        let buffer = render(&mut app, 120, 30);
        golden("details.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(screen.contains("$ /opt/ruharness/bin/harness --json verify u-lib"));
        assert!(screen.contains("exit 0"));
        assert!(screen.contains("Ready. Last: Re-check u-lib — GREEN"));
    }

    /// §5, §13: a dialog before and after arming — Run dim with "reading…",
    /// then "ready: → then Enter, or y"; the argv whole.
    #[test]
    fn golden_dialog_before_and_after_arming() {
        let mut app = app("gdialog");
        app.select(Selection::Unit("u-lib".into()));
        let mut p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        // A fixed target path: the golden must not depend on the machine.
        p.argv[4] = OsString::from("--target=/work/read_scalefactors_lib");
        app.ask(p);
        let buffer = render(&mut app, 120, 30);
        golden("dialog-unarmed.txt", &app, &buffer);
        assert!(text(&buffer).contains("reading…"));
        assert!(text(&buffer).contains("Re-check u-lib with the oracle?"));
        arm(&mut app);
        let buffer = render(&mut app, 120, 30);
        golden("dialog-armed.txt", &app, &buffer);
        assert!(text(&buffer).contains("ready: → then Enter, or y"));
    }

    /// §5.1: a long argv is shown whole — wrapped, never cut — and the
    /// dialog arms only once it was scrolled to its end.
    #[test]
    fn a_long_argv_must_be_seen_to_its_end() {
        let mut app = app("glong");
        attempt(&mut app, PROVENANCE);
        let note: String = (0..60).map(|i| format!("word{i} ")).collect();
        key(&mut app, 'm');
        app.on_paste(note.trim_end());
        app.on_key(KeyEvent::from(KeyCode::Enter), Instant::now());
        let screen = text(&render(&mut app, 80, 16));
        assert!(screen.contains("↓ more below"), "{screen}");
        let Mode::Dialog(c) = &app.mode else { panic!() };
        assert!(!c.dialog.seen);
        for _ in 0..40 {
            key(&mut app, 'j');
        }
        let screen = text(&render(&mut app, 80, 16));
        assert!(screen.contains("word59"), "{screen}");
        let Mode::Dialog(c) = &app.mode else { panic!() };
        assert!(c.dialog.seen);
    }

    /// Every untrusted string passes the display filter: a control
    /// character in a file name never reaches the terminal.
    #[test]
    fn a_control_character_in_a_file_name_is_filtered() {
        let mut app = app("gctrl");
        let bad = app.config.target.join("test_case/src/bad\u{1b}[31mname.c");
        std::fs::write(&bad, "int bad(void) { return 0; }\n").unwrap();
        assert!(app.reload(true));
        let screen = text(&render(&mut app, 120, 30));
        assert!(!screen.contains('\u{1b}'));
        assert!(screen.contains("bad?[31mname.c"), "{screen}");
    }

    /// §8: the hint bar drops whole entries, never cuts one; `? help` and
    /// `q quit` are always last and never dropped.
    #[test]
    fn the_hint_bar_drops_whole_entries() {
        let mut app = app("ghints");
        for width in [40u16, 60, 80, 100, 120] {
            let screen = text(&render(&mut app, width, 24));
            let bar = screen.lines().last().unwrap();
            assert!(bar.ends_with("? help   q quit"), "{width}: {bar}");
            for (k, v) in hints(&app) {
                let entry = format!("{k} {v}");
                assert!(
                    bar.contains(&entry) || !bar.contains(&format!("{k} ")) || k == "Enter",
                    "{width}: {bar}"
                );
            }
        }
    }

    /// Review USE-3: the hint bar lists the keys that work in the open
    /// overlay; `? help` and `q quit` only in the panes.
    #[test]
    fn the_hint_bar_follows_the_mode() {
        let mut app = app("ghintmode");
        let bar = |app: &mut App| {
            text(&render(app, 120, 24))
                .lines()
                .last()
                .unwrap()
                .to_string()
        };
        assert!(bar(&mut app).ends_with("? help   q quit"));
        app.open_menu();
        assert_eq!(bar(&mut app).trim(), "↑↓ move   Enter choose   Esc close");
        crate::app::tests::code(&mut app, KeyCode::Esc);
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'm');
        assert_eq!(bar(&mut app).trim(), "Enter continue   Esc cancel");
        crate::app::tests::code(&mut app, KeyCode::Esc);
        key(&mut app, 'a');
        assert_eq!(bar(&mut app).trim(), "↑↓ scroll   Esc cancel");
        arm(&mut app);
        assert_eq!(
            bar(&mut app).trim(),
            "↑↓ scroll   ←→ button   Enter press   Esc cancel"
        );
    }

    /// Second fix pass, NEW-6 (hints): with links in the View but none
    /// chosen, Enter opens the actions — the bar says so.
    #[test]
    fn the_view_hints_follow_the_link() {
        let mut app = app("glinkhint");
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        crate::app::tests::code(&mut app, KeyCode::Tab);
        let bar = |app: &mut App| {
            text(&render(app, 160, 24))
                .lines()
                .last()
                .unwrap()
                .to_string()
        };
        assert!(bar(&mut app).contains("Enter actions"), "{}", bar(&mut app));
        crate::app::tests::code(&mut app, KeyCode::Down);
        assert!(
            bar(&mut app).contains("Enter go there"),
            "{}",
            bar(&mut app)
        );
    }

    /// Review SAFE-5: a terminal too small to show a dialog whole never
    /// lets it arm.
    #[test]
    fn a_dialog_on_a_tiny_terminal_never_arms() {
        let mut app = app("gtiny");
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        // Even scrolled to its end, a dialog too small to show its command
        // never counts as seen.
        for (w, h) in [(18u16, 30u16), (120, 5), (120, 4)] {
            if let Mode::Dialog(c) = &mut app.mode {
                c.dialog.scroll = usize::MAX / 2;
            }
            render(&mut app, w, h);
            let Mode::Dialog(c) = &app.mode else { panic!() };
            assert!(!c.dialog.seen, "{w}x{h}");
        }
        render(&mut app, 120, 30);
        let Mode::Dialog(c) = &app.mode else { panic!() };
        assert!(c.dialog.seen);
        // Its words survive the small frames (review NEW-1).
        assert!(
            c.body
                .iter()
                .any(|b| b.contains("marks the attempt promoted")),
            "{:?}",
            c.body
        );
    }

    /// Second fix pass, NEW-9: a title too long for the border is shown
    /// whole in the dialog.
    #[test]
    fn a_long_title_is_shown_whole() {
        let mut app = app("glongtitle");
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        if let Mode::Dialog(c) = &mut app.mode {
            c.title = format!(
                "Replace {}'s verified crate with a-13c941dfff95?",
                "u".repeat(60)
            );
        }
        let screen = text(&render(&mut app, 120, 30));
        assert!(screen.contains("with a-13c941dfff95?"), "{screen}");
    }

    /// Review USE-15: no blank strip at 150 columns and wider — the View
    /// takes the width until the chat exists.
    #[test]
    fn no_blank_strip_at_wide_widths() {
        let mut app = app("gwide160");
        render(&mut app, 160, 20);
        let (view, _) = app
            .hits
            .iter()
            .find(|(_, h)| *h == Hit::Pane(Focus::View))
            .unwrap();
        assert_eq!(view.x + view.width, 160);
    }

    /// The hit-record API (Build B's mouse): each drawn tree row, the panes,
    /// the dialog and its buttons are recorded at the cells they occupy.
    #[test]
    fn clickable_regions_are_recorded() {
        let mut app = app("ghits");
        render(&mut app, 120, 30);
        let row = app
            .hits
            .iter()
            .find(|(_, h)| *h == Hit::Row(Selection::Unit("u-lib".into())))
            .map(|(r, _)| *r)
            .expect("the unit row");
        let buffer = render(&mut app, 120, 30);
        let line: String = (row.x..row.x + row.width)
            .map(|x| buffer[(x, row.y)].symbol())
            .collect();
        assert!(line.contains("u-lib"), "{line}");
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::Pane(Focus::View)));
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::Hint("q")));
        app.running = true;
        key(&mut app, 'q');
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Quit));
        let buffer = render(&mut app, 120, 30);
        let (b, _) = app
            .hits
            .iter()
            .find(|(_, h)| *h == Hit::Button(2))
            .expect("the third button");
        let label: String = (b.x..b.x + b.width)
            .map(|x| buffer[(x, b.y)].symbol())
            .collect();
        assert_eq!(label, "[ Stop it and quit  x ]");
        arm(&mut app);
        assert_eq!(key(&mut app, 'x'), Command::CancelAndQuit);
    }

    // ----- the mouse (§7) ------------------------------------------------------

    use crate::app::{QueueClock, DOUBLE_CLICK, DRAG_HINT};
    use crate::dialog::{Kind, CLICK_SETTLE};
    use ratatui::crossterm::event::{
        Event as TermEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use std::time::Duration;

    const LEFT: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const UP: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// One mouse event read at `now` on time, through the loop's own step
    /// (`App::on_event`: the quiet time, then the gesture).
    fn event(app: &mut App, kind: MouseEventKind, at: (u16, u16), now: Instant) -> Command {
        app.on_event(TermEvent::Mouse(mouse(kind, at)), now, Duration::ZERO)
    }

    /// A left press and its release at `at`, read at `now`: what the
    /// release (or the press) did.
    fn click(app: &mut App, at: (u16, u16), now: Instant) -> Command {
        let pressed = event(app, LEFT, at, now);
        let released = event(app, UP, at, now + ms(20));
        if pressed == Command::None {
            released
        } else {
            pressed
        }
    }

    /// A spot inside the region the last frame recorded for `hit` (the one
    /// on top).
    fn spot(app: &App, hit: &Hit) -> (u16, u16) {
        let (r, _) = app
            .hits
            .iter()
            .rev()
            .find(|(_, h)| h == hit)
            .unwrap_or_else(|| panic!("{hit:?} is not drawn: {:?}", app.hits));
        (r.x + r.width / 2, r.y)
    }

    /// [`click`] on the region recorded for `hit`.
    fn click_on(app: &mut App, hit: &Hit, now: Instant) -> Command {
        let at = spot(app, hit);
        click(app, at, now)
    }

    fn drawn(app: &App, hit: &Hit) -> bool {
        app.hits.iter().any(|(_, h)| h == hit)
    }

    /// The text the buffer shows inside `r`.
    fn under(buffer: &Buffer, r: Rect) -> String {
        (r.y..r.y + r.height)
            .flat_map(|y| (r.x..r.x + r.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    /// A dialog that has been open long enough, armed and drawn armed: what
    /// a click may answer.
    fn ready(app: &mut App, width: u16, height: u16, opened: Instant) -> Instant {
        render(app, width, height);
        app.arm(opened + ms(400), false);
        app.arm(opened + ms(800), false);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.armed));
        render(app, width, height);
        opened + CLICK_SETTLE + ms(1)
    }

    /// Review TEST-B-1: every clickable region is where its thing is drawn —
    /// on a scrolled tree, a scrolled list, the activity row while a command
    /// runs, Help, a menu and a dialog — never a region the frame moved.
    #[test]
    fn every_hit_is_where_it_is_drawn() {
        let check = |app: &App, buffer: &Buffer| {
            let mut seen = 0;
            for (i, (r, h)) in app.hits.iter().enumerate() {
                // Only where it is on top: an overlay may cover it.
                let mid = ratatui::layout::Position::new(r.x + r.width / 2, r.y);
                if app.hits[i + 1..]
                    .iter()
                    .any(|(o, oh)| o.contains(mid) && !matches!(oh, Hit::Fold(_)))
                {
                    continue;
                }
                let text = under(buffer, *r);
                let want: Option<String> = match h {
                    Hit::Row(sel) => Some(node_name(app, sel).chars().take(4).collect()),
                    Hit::Fold(_) => Some(
                        if text.starts_with('▾') {
                            "▾"
                        } else {
                            "▸"
                        }
                        .into(),
                    ),
                    Hit::Link(i) => app.links.get(*i).map(|sel| {
                        let name = node_name(app, sel);
                        name.rsplit('/')
                            .next()
                            .unwrap_or(&name)
                            .chars()
                            .take(4)
                            .collect()
                    }),
                    Hit::Activity(k) | Hit::Hint(k) if *k != "m" || text.contains('m') => {
                        Some((*k).into())
                    }
                    Hit::Hint("m") => Some("Mouse".into()),
                    Hit::MenuItem(i) => match &app.mode {
                        Mode::Menu(m) => Some(m.items[*i].label.chars().take(8).collect()),
                        _ => None,
                    },
                    Hit::Button(i) => match &app.mode {
                        Mode::Dialog(c) => Some(format!(
                            "[ {}  {} ]",
                            c.dialog.buttons[*i].label, c.dialog.buttons[*i].key
                        )),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(want) = want {
                    assert!(
                        text.contains(&want),
                        "{h:?} at {r:?} shows {text:?}, not {want:?}"
                    );
                    seen += 1;
                }
            }
            seen
        };
        // A scrolled tree, and a scrolled list.
        let mut app = app_of("targets/zopfli", "mwhere");
        let t = Instant::now();
        render(&mut app, 120, 16);
        let files = spot(&app, &Hit::Pane(Focus::Files));
        event(&mut app, MouseEventKind::ScrollDown, files, t);
        event(&mut app, MouseEventKind::ScrollDown, files, t);
        let buffer = render(&mut app, 120, 16);
        assert!(app.tree_offset > 0);
        assert!(check(&app, &buffer) > 5);
        app.select(Selection::Dir("src/zopfli".into()));
        render(&mut app, 120, 16);
        let view = spot(&app, &Hit::Pane(Focus::View));
        event(&mut app, MouseEventKind::ScrollDown, view, t);
        let buffer = render(&mut app, 120, 16);
        assert!(app.scroll > 0);
        assert!(check(&app, &buffer) > 5);
        // The activity row while running, Help, a menu, a dialog.
        let mut app = crate::app::tests::app("mwhere2");
        running(&mut app);
        let buffer = render(&mut app, 120, 30);
        assert!(drawn(&app, &Hit::Activity("x")) && drawn(&app, &Hit::Activity("c")));
        check(&app, &buffer);
        key(&mut app, '?');
        let buffer = render(&mut app, 120, 30);
        let (r, _) = app
            .hits
            .iter()
            .rev()
            .find(|(_, h)| *h == Hit::Hint("m"))
            .unwrap();
        assert!(under(&buffer, *r).contains("Mouse: on"), "Help's line");
        check(&app, &buffer);
        app.mode = Mode::Normal;
        app.select(Selection::Unit("u-lib".into()));
        app.open_menu();
        let buffer = render(&mut app, 120, 30);
        assert!(check(&app, &buffer) >= 3);
        app.mode = Mode::Normal;
        key(&mut app, 'q');
        let buffer = render(&mut app, 120, 30);
        assert_eq!(
            app.hits
                .iter()
                .filter(|(_, h)| matches!(h, Hit::Button(_)))
                .count(),
            3
        );
        check(&app, &buffer);
    }

    /// §7: a left click focuses the pane and selects the row; a click on a
    /// View link chooses it; a single click opens nothing.
    #[test]
    fn a_click_selects_the_row_and_focuses_its_pane() {
        let mut app = app("mclick");
        let t = Instant::now();
        app.focus = Focus::View;
        render(&mut app, 120, 30);
        let unit = Hit::Row(Selection::Unit("u-lib".into()));
        assert_eq!(click_on(&mut app, &unit, t), Command::None);
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        assert_eq!(app.focus, Focus::Files);
        assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Pane(Focus::View), t + ms(1000));
        assert_eq!(app.focus, Focus::View);
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        // A directory's View lists its files: a click chooses one.
        app.select(Selection::Dir("test_case/src".into()));
        app.focus = Focus::Files;
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Link(0), t + ms(2000));
        assert_eq!(app.focus, Focus::View);
        assert_eq!(app.link, Some(0));
        assert_eq!(
            app.selection,
            Selection::Dir("test_case/src".into()),
            "a link is chosen, not followed"
        );
    }

    /// Review USE-B-1: a click on a row's ▸/▾ opens or folds it (and
    /// selects it) — the mouse reaches every node.
    #[test]
    fn a_click_on_the_marker_opens_and_folds() {
        let mut app = app("mfold");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = Selection::Unit("u-lib".into());
        let fold = Hit::Fold(unit.clone());
        let rows = app.rows.len();
        assert!(!app.expansion.is_open(&unit));
        click_on(&mut app, &fold, t);
        assert!(app.expansion.is_open(&unit));
        assert_eq!(app.selection, unit);
        assert!(app.rows.len() > rows, "its crate and attempts show");
        render(&mut app, 120, 30);
        click_on(&mut app, &fold, t + ms(1000));
        assert!(!app.expansion.is_open(&unit));
        assert_eq!(app.rows.len(), rows);
        // The hint bar's ←→ does the same to the selection.
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("←→"), t + ms(2000));
        assert!(app.expansion.is_open(&unit));
    }

    /// §7: a double click is `Enter` — two presses on the same row within
    /// 400 ms, the first known to have been read on time; never slower,
    /// never on two rows, never with a key between them.
    #[test]
    fn a_double_click_is_enter_on_the_row() {
        let mut app = app("mdouble");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        let units = spot(&app, &Hit::Row(Selection::Units));
        let normal = |app: &App| matches!(app.mode, Mode::Normal);
        // Too slow.
        click(&mut app, unit, t);
        render(&mut app, 120, 30);
        click(&mut app, unit, t + DOUBLE_CLICK + ms(1));
        assert!(normal(&app), "slower than the window");
        // Two rows.
        render(&mut app, 120, 30);
        click(&mut app, units, t + ms(2000));
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(2100));
        assert!(normal(&app), "two rows");
        // The first press may have waited in the queue (a stall): the two
        // may have been far apart.
        render(&mut app, 120, 30);
        app.on_event(
            TermEvent::Mouse(mouse(LEFT, unit)),
            t + ms(3000),
            DOUBLE_CLICK,
        );
        event(&mut app, UP, unit, t + ms(3010));
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(3100));
        assert!(normal(&app), "across a stall");
        // A key between the two presses.
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(4000));
        crate::app::tests::code(&mut app, KeyCode::Right);
        crate::app::tests::code(&mut app, KeyCode::Left);
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(4100));
        assert!(normal(&app), "a key between");
        // A double click on the row: its menu, as Enter.
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(5000));
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(5000) + DOUBLE_CLICK);
        assert!(matches!(app.mode, Mode::Menu(_)), "{:?}", app.mode);
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        // A double click on a View link follows it.
        app.mode = Mode::Normal;
        app.select(Selection::Dir("test_case/src".into()));
        render(&mut app, 120, 30);
        let target = app.links[0].clone();
        let link = spot(&app, &Hit::Link(0));
        click(&mut app, link, t + ms(7000));
        render(&mut app, 120, 30);
        click(&mut app, link, t + ms(7100));
        assert_eq!(app.selection, target);
        // Two presses on an empty part of a pane: nothing.
        app.focus = Focus::Files;
        render(&mut app, 120, 30);
        let empty = (5, 20);
        click(&mut app, empty, t + ms(9000));
        render(&mut app, 120, 30);
        click(&mut app, empty, t + ms(9100));
        assert!(normal(&app));
    }

    /// The real drive (E2E-1): a click, then a quick double click on the
    /// same row — a triple click — opened the menu and closed it again. A
    /// quick press inside what a press just did is swallowed: a triple
    /// click is a double click, and a burst stays one gesture.
    #[test]
    fn a_triple_click_is_a_double_click() {
        let mut app = app("mtriple");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        for i in 0..5 {
            click(&mut app, unit, t + ms(150 * i));
            render(&mut app, 120, 30);
            if i >= 1 {
                assert!(
                    matches!(app.mode, Mode::Menu(_)),
                    "press {i}: {:?}",
                    app.mode
                );
            }
        }
        // Once the burst is over, a click outside the menu closes it.
        click(&mut app, unit, t + ms(150 * 4) + DOUBLE_CLICK + ms(1));
        assert!(matches!(app.mode, Mode::Normal));
    }

    /// Review USE-B-3: a double click that dismisses a menu or Help only
    /// dismisses it — the second press never opens the menu under it.
    #[test]
    fn a_double_click_to_dismiss_only_dismisses() {
        let mut app = app("mdismiss");
        let t = Instant::now();
        render(&mut app, 120, 30);
        // The unit's row, left of the menu and of Help.
        let unit = (3, spot(&app, &Hit::Row(Selection::Unit("u-lib".into()))).1);
        for open in [
            |a: &mut App| a.open_menu(),
            |a: &mut App| {
                key(a, '?');
            },
        ] {
            open(&mut app);
            render(&mut app, 120, 30);
            click(&mut app, unit, t);
            render(&mut app, 120, 30);
            click(&mut app, unit, t + ms(150));
            assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
        }
    }

    /// Mutation-checked rule (§7, §12.1): a click never runs an unarmed
    /// dialog. A press on a button before arming, before a frame showed it
    /// armed, or within a second of it opening is dropped ("Too soon") and
    /// restarts the quiet time; a press dragged off the button does
    /// nothing; a ready button answers its release.
    #[test]
    fn a_click_never_runs_an_unarmed_dialog() {
        let mut app = app("mbutton");
        let t0 = Instant::now();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.now = t0;
        app.ask(scan.clone());
        render(&mut app, 120, 30);
        app.arm(t0, false);
        let (cancel, run) = (spot(&app, &Hit::Button(0)), spot(&app, &Hit::Button(1)));
        for (i, at) in [run, cancel].into_iter().enumerate() {
            let now = t0 + ms(100 * (i as u64 + 1));
            assert_eq!(click(&mut app, at, now), Command::None);
            let Mode::Dialog(c) = &app.mode else {
                panic!("an unarmed click closed the dialog")
            };
            assert!(c.dialog.too_soon && !c.dialog.armed);
            assert_eq!(
                c.dialog.quiet_since,
                now + ms(20),
                "the release restarted it"
            );
        }
        // Armed, but no frame has shown it armed yet.
        app.arm(t0 + ms(600), false);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.armed));
        assert_eq!(click(&mut app, run, t0 + CLICK_SETTLE), Command::None);
        // Shown armed, but within a second of opening.
        render(&mut app, 120, 30);
        assert_eq!(
            click(&mut app, run, t0 + CLICK_SETTLE - ms(30)),
            Command::None
        );
        // A press on Run dragged off: nothing.
        let later = t0 + CLICK_SETTLE + ms(100);
        event(&mut app, LEFT, run, later);
        assert_eq!(
            event(&mut app, UP, (run.0, run.1 - 3), later),
            Command::None
        );
        assert!(matches!(app.mode, Mode::Dialog(_)));
        // Ready: its release runs it.
        assert_eq!(event(&mut app, LEFT, run, later + ms(500)), Command::None);
        assert_eq!(
            event(&mut app, UP, run, later + ms(550)),
            Command::Spawn(scan)
        );
        // The quit dialog: its third button, once ready.
        let mut app = crate::app::tests::app("mquitclick");
        app.running = true;
        let t = Instant::now();
        app.now = t;
        key(&mut app, 'q');
        render(&mut app, 120, 30);
        let stop = spot(&app, &Hit::Button(2));
        assert_eq!(click(&mut app, stop, t), Command::None);
        assert!(matches!(app.mode, Mode::Dialog(_)));
        let when = ready(&mut app, 120, 30, t);
        assert_eq!(click(&mut app, stop, when), Command::CancelAndQuit);
    }

    /// Review SAFE-B-2: the second press of a slow double click on a menu
    /// item never presses the button of the dialog that item opened, even
    /// with the dialog armed and drawn under the pointer — until a second
    /// after it opened.
    #[test]
    fn a_slow_double_click_never_presses_the_dialog_it_opened() {
        let mut app = app("mslow");
        let t = Instant::now();
        app.select(Selection::Project);
        app.open_menu();
        render(&mut app, 120, 30);
        let Mode::Menu(m) = &app.mode else { panic!() };
        let detect = m
            .items
            .iter()
            .position(|i| i.label.starts_with("Find hazards"))
            .unwrap();
        click_on(&mut app, &Hit::MenuItem(detect), t);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.title.starts_with("Find hazards")));
        render(&mut app, 120, 30);
        app.arm(t + ms(30), false);
        app.arm(t + ms(350), false);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.armed));
        render(&mut app, 120, 30);
        let run = spot(&app, &Hit::Button(1));
        for late in [450, 500, 700, 950] {
            assert_eq!(
                click(&mut app, run, t + ms(late)),
                Command::None,
                "at {late} ms"
            );
            assert!(matches!(app.mode, Mode::Dialog(_)));
        }
        render(&mut app, 120, 30);
        assert!(matches!(
            click(&mut app, run, t + CLICK_SETTLE + ms(100)),
            Command::Spawn(_)
        ));
    }

    /// Verification NEW-1: the same mechanism stopped a running command — a
    /// slow double click on "Cancel the running command" landed on the
    /// Cancel dialog's `[Stop it x]`. Every size the verifier swept, and a
    /// second press at any time within the settle time.
    #[test]
    fn a_slow_double_click_on_cancel_never_stops_the_command() {
        let mut app = app("mcancel");
        running(&mut app);
        attempt(&mut app, PROVENANCE);
        let t0 = Instant::now();
        for (i, (w, h)) in [(80, 24), (100, 25), (120, 30), (160, 41)]
            .into_iter()
            .enumerate()
        {
            let t = t0 + Duration::from_secs(10 * i as u64);
            app.mode = Mode::Normal;
            app.open_menu();
            render(&mut app, w, h);
            let Mode::Menu(m) = &app.mode else { panic!() };
            let cancel = m
                .items
                .iter()
                .position(|i| i.label.starts_with("Cancel the running"))
                .unwrap();
            let at = spot(&app, &Hit::MenuItem(cancel));
            click(&mut app, at, t);
            assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Cancel));
            render(&mut app, w, h);
            app.arm(t + ms(30), false);
            app.arm(t + ms(340), false);
            render(&mut app, w, h);
            let stop = spot(&app, &Hit::Button(1));
            // Within the window of the click that opened it: swallowed; after
            // a pause, refused while it settles.
            for late in [380, 450, 950] {
                for spot in [at, stop] {
                    assert_eq!(
                        click(&mut app, spot, t + ms(late)),
                        Command::None,
                        "{w}x{h}"
                    );
                }
            }
            assert!(matches!(app.mode, Mode::Dialog(_)), "{w}x{h}");
        }
    }

    /// Reviews SAFE-B-1, SAFE-B-8: the activity row's buttons answer only in
    /// the panes. Under a dialog they do nothing (never its letter: `x` on
    /// an armed quit dialog is "Stop it and quit"); under a note they never
    /// type; under a menu they are outside it.
    #[test]
    fn activity_buttons_answer_only_in_the_panes() {
        let mut app = app("mactivity");
        let t = Instant::now();
        running(&mut app);
        app.now = t;
        key(&mut app, 'q');
        let when = ready(&mut app, 120, 30, t);
        assert_eq!(click_on(&mut app, &Hit::Activity("x"), when), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Quit));
        app.mode = Mode::Normal;
        key(&mut app, 'x');
        let when = ready(&mut app, 120, 30, when);
        assert_eq!(click_on(&mut app, &Hit::Activity("x"), when), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Cancel));
        // A note being typed.
        app.mode = Mode::Note {
            input: "tighten".into(),
            unit: "u-lib".into(),
            attempt: PROVENANCE.into(),
        };
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Activity("c"), when + ms(1000));
        assert!(matches!(&app.mode, Mode::Note { input, .. } if input == "tighten"));
        // A menu: outside it.
        app.mode = Mode::Normal;
        app.open_menu();
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Activity("c"), when + ms(2000));
        assert!(matches!(app.mode, Mode::Normal), "closed, no details");
        // The panes: the button's key.
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Activity("c"), when + ms(3000));
        assert!(matches!(app.mode, Mode::Details { .. }));
    }

    /// Review §R5, ENG-B-5, TEST-B-5: a button is clickable only where it is
    /// drawn whole — the dialog's and the activity row's — and a dialog's
    /// only on a frame big enough to show it.
    #[test]
    fn only_whole_buttons_on_a_usable_dialog_are_clickable() {
        let mut app = app("mclip");
        app.running = true;
        key(&mut app, 'q');
        // The narrowest frame that shows the third button whole.
        let whole = (40..=90)
            .find(|w| {
                render(&mut app, *w, 30);
                drawn(&app, &Hit::Button(2))
            })
            .unwrap();
        let buffer = render(&mut app, whole, 30);
        let (r, _) = app.hits.iter().find(|(_, h)| *h == Hit::Button(2)).unwrap();
        assert_eq!(under(&buffer, *r), "[ Stop it and quit  x ]");
        render(&mut app, whole - 1, 30);
        assert!(!drawn(&app, &Hit::Button(2)), "one column less cuts it");
        assert!(drawn(&app, &Hit::Button(1)));
        // Too small to show it: no button at all, armed or not.
        arm(&mut app);
        render(&mut app, 22, 7);
        assert!(!app.hits.iter().any(|(_, h)| matches!(h, Hit::Button(_))));
        let Mode::Dialog(c) = &mut app.mode else {
            panic!()
        };
        assert!(!c.dialog.usable);
        c.dialog.shown_armed = true;
        assert!(!c.dialog.press_button(1, Instant::now() + CLICK_SETTLE));
        // The activity row's buttons, cut by a narrow frame.
        let mut app = crate::app::tests::app("mclip2");
        running(&mut app);
        for w in 16..40 {
            let buffer = render(&mut app, w, 12);
            for (r, h) in &app.hits {
                if let Hit::Activity(_) = h {
                    assert!(r.x + r.width <= w, "{h:?} past the frame at {w}");
                    assert!(under(&buffer, *r).ends_with(']'), "{h:?} cut at {w}");
                }
            }
        }
    }

    /// §7: a click outside a menu closes it (and does nothing else); inside
    /// it, on no item, nothing; on an item, what `Enter` on it does. A click
    /// outside a dialog, or on its words, does nothing; inside Help, the
    /// checks or the diff, nothing — outside, they close; a note is never
    /// closed by a click.
    #[test]
    fn outside_a_menu_closes_it_outside_a_dialog_nothing() {
        let mut app = app("moutside");
        let t = Instant::now();
        app.select(Selection::Unit("u-lib".into()));
        app.open_menu();
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Dialog, t);
        assert!(matches!(app.mode, Mode::Menu(_)), "inside the menu");
        click(&mut app, (0, 29), t + ms(500));
        assert!(matches!(app.mode, Mode::Menu(_)), "the hint bar's ↑↓");
        click(&mut app, (0, 0), t + ms(1000));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        // An item: as Enter on it (a greyed one says why, and stays).
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        app.select(Selection::Project);
        app.open_menu();
        render(&mut app, 120, 30);
        let Mode::Menu(m) = &app.mode else { panic!() };
        let greyed = m.items.iter().position(|i| i.greyed.is_some()).unwrap();
        let scan = m
            .items
            .iter()
            .position(|i| i.label == "Scan the project")
            .unwrap();
        click_on(&mut app, &Hit::MenuItem(greyed), t + ms(2000));
        assert!(matches!(&app.mode, Mode::Menu(m) if m.focus == greyed && m.footer.is_some()));
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::MenuItem(scan), t + ms(3000));
        assert!(
            matches!(&app.mode, Mode::Dialog(c) if c.title == "Scan the project?"),
            "{:?}",
            app.mode
        );
        render(&mut app, 120, 30);
        for at in [(0, 0), (119, 28), spot(&app, &Hit::Dialog)] {
            assert_eq!(click(&mut app, at, t + ms(4000)), Command::None);
            assert!(
                matches!(app.mode, Mode::Dialog(_)),
                "a click at {at:?} closed it"
            );
        }
        // Help: inside nothing, outside closes.
        app.mode = Mode::Help { scroll: 0 };
        render(&mut app, 120, 30);
        click(&mut app, (60, 15), t + ms(5000));
        assert!(matches!(app.mode, Mode::Help { .. }), "inside Help");
        click(&mut app, (0, 0), t + ms(6000));
        assert!(matches!(app.mode, Mode::Normal));
        // A note: never closed by a click.
        app.mode = Mode::Note {
            input: "keep".into(),
            unit: "u-lib".into(),
            attempt: PROVENANCE.into(),
        };
        render(&mut app, 120, 30);
        for at in [(0, 0), (60, 15)] {
            click(&mut app, at, t + ms(7000));
            assert!(matches!(&app.mode, Mode::Note { input, .. } if input == "keep"));
        }
    }

    /// Review USE-B-7: a greyed item's "Why not" grows the menu downward —
    /// the items stay under the pointer.
    #[test]
    fn a_menu_footer_never_moves_its_items() {
        let mut app = app("mfooter");
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        app.open_menu();
        render(&mut app, 100, 24);
        let items = |app: &App| -> Vec<(Rect, Hit)> {
            app.hits
                .iter()
                .filter(|(_, h)| matches!(h, Hit::MenuItem(_)))
                .cloned()
                .collect()
        };
        let before = items(&app);
        let Mode::Menu(m) = &app.mode else { panic!() };
        let greyed = m.items.iter().position(|i| i.greyed.is_some()).unwrap();
        click_on(&mut app, &Hit::MenuItem(greyed), Instant::now());
        render(&mut app, 100, 24);
        assert!(matches!(&app.mode, Mode::Menu(m) if m.footer.is_some()));
        assert_eq!(items(&app), before);
    }

    /// §7: the wheel scrolls an open menu (clamped: it never wraps) or
    /// dialog (restarting its quiet time), else the pane under the pointer
    /// — the tree and a list without moving what is chosen, until the next
    /// key there; sideways over the View's code; one check a notch.
    #[test]
    fn the_wheel_scrolls_menus_dialogs_and_the_pane_under_it() {
        let mut app = app_of("targets/zopfli", "mwheel");
        let t = Instant::now();
        render(&mut app, 120, 16);
        let files = spot(&app, &Hit::Pane(Focus::Files));
        let before = app.selection.clone();
        assert!(
            app.rows.len() > app.layout.tree_page,
            "the tree must scroll"
        );
        event(&mut app, MouseEventKind::ScrollDown, files, t);
        assert_eq!(app.tree_offset, 3);
        assert_eq!(app.selection, before, "the wheel never selects");
        render(&mut app, 120, 16);
        assert_eq!(app.tree_offset, 3, "not snapped back to the selection");
        event(&mut app, MouseEventKind::ScrollUp, files, t);
        event(&mut app, MouseEventKind::ScrollUp, files, t);
        assert_eq!(app.tree_offset, 0, "clamped");
        event(&mut app, MouseEventKind::ScrollDown, files, t);
        // A key in the View, or the wheel in an overlay — with the focus in
        // the tree too — leaves it (review USE-B-13, ENG-B-4); a key in the
        // tree follows the selection.
        app.focus = Focus::View;
        crate::app::tests::code(&mut app, KeyCode::Down);
        key(&mut app, 'c');
        app.focus = Focus::Files;
        render(&mut app, 120, 16);
        let details = spot(&app, &Hit::Overlay);
        event(&mut app, MouseEventKind::ScrollDown, details, t);
        assert!(matches!(app.mode, Mode::Details { .. }));
        app.mode = Mode::Normal;
        render(&mut app, 120, 16);
        assert_eq!(app.tree_offset, 3, "still where the wheel left it");
        app.focus = Focus::Files;
        crate::app::tests::code(&mut app, KeyCode::Home);
        render(&mut app, 120, 16);
        assert_eq!(
            app.tree_offset, 0,
            "a key in the tree follows the selection again"
        );
        // A list: the chosen link stays chosen, and stays where wheeled.
        app.select(Selection::Dir("src/zopfli".into()));
        render(&mut app, 120, 16);
        click_on(&mut app, &Hit::Link(0), t + ms(1000));
        render(&mut app, 120, 16);
        let view = spot(&app, &Hit::Pane(Focus::View));
        event(&mut app, MouseEventKind::ScrollDown, view, t + ms(2000));
        event(&mut app, MouseEventKind::ScrollDown, view, t + ms(2000));
        render(&mut app, 120, 16);
        assert_eq!(app.link, Some(0), "still chosen");
        assert_eq!(app.scroll, 6, "not snapped back to the link");
        // A unit's View: its rows, and sideways, the focus kept.
        app.select(Selection::Unit(app.snapshot.units[0].unit.id.clone()));
        app.focus = Focus::Files;
        render(&mut app, 120, 16);
        let view = spot(&app, &Hit::Pane(Focus::View));
        event(&mut app, MouseEventKind::ScrollDown, view, t);
        assert_eq!(app.scroll, 3.min(app.layout.total_rows.saturating_sub(1)));
        assert!(app.code_cols > 16, "code wider than a notch");
        event(&mut app, MouseEventKind::ScrollRight, view, t);
        assert_eq!(app.hscroll, 8);
        event(&mut app, MouseEventKind::ScrollLeft, files, t);
        assert_eq!(app.hscroll, 8, "not over the View");
        assert_eq!(app.focus, Focus::Files);
        // A menu: one item a notch, never wrapping.
        app.open_menu();
        render(&mut app, 120, 16);
        let n = match &app.mode {
            Mode::Menu(m) => m.items.len(),
            _ => panic!(),
        };
        for _ in 0..n + 2 {
            event(&mut app, MouseEventKind::ScrollDown, files, t);
        }
        assert!(matches!(&app.mode, Mode::Menu(m) if m.focus == n - 1));
        event(&mut app, MouseEventKind::ScrollUp, (0, 0), t);
        assert!(matches!(&app.mode, Mode::Menu(m) if m.focus == n - 2));
        // The checks: one a notch.
        app.mode = Mode::Verdict {
            selected: 0,
            scroll: 0,
        };
        event(&mut app, MouseEventKind::ScrollDown, (0, 0), t);
        assert!(matches!(app.mode, Mode::Verdict { selected: 1, .. }));
        // A dialog: it scrolls wherever the pointer is, and the wheel is
        // input — the quiet time restarts.
        app.mode = Mode::Normal;
        let p = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.ask(p);
        render(&mut app, 120, 16);
        event(&mut app, MouseEventKind::ScrollDown, (0, 0), t + ms(500));
        let Mode::Dialog(c) = &app.mode else { panic!() };
        assert_eq!(c.dialog.scroll, 3);
        assert_eq!(c.dialog.quiet_since, t + ms(500));
        assert!(!c.dialog.armed);
        // A note: the wheel does nothing.
        app.mode = Mode::Note {
            input: "n".into(),
            unit: "u".into(),
            attempt: "a".into(),
        };
        event(&mut app, MouseEventKind::ScrollDown, (0, 0), t);
        assert!(matches!(&app.mode, Mode::Note { input, .. } if input == "n"));
    }

    /// Review TEST-B-3: every mouse event restarts an open dialog's quiet
    /// time through the loop's step — a release, a click outside, the
    /// sideways wheel, a right click.
    #[test]
    fn every_mouse_event_restarts_the_quiet_time() {
        let mut app = app("mquiet");
        let t = Instant::now();
        let p = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.ask(p);
        render(&mut app, 120, 30);
        for (i, kind) in [
            UP,
            LEFT,
            MouseEventKind::ScrollLeft,
            MouseEventKind::Down(MouseButton::Right),
        ]
        .into_iter()
        .enumerate()
        {
            let now = t + ms(100 * (i as u64 + 1));
            app.on_event(TermEvent::Mouse(mouse(kind, (0, 0))), now, Duration::ZERO);
            let Mode::Dialog(c) = &app.mode else { panic!() };
            assert_eq!(c.dialog.quiet_since, now, "{kind:?}");
        }
    }

    /// §7: hint-bar entries and the activity buttons do what their key does
    /// — Quit is asked first, running or not (review SAFE-B-4) — and a
    /// double click on one presses it once.
    #[test]
    fn hints_and_buttons_press_their_keys() {
        let mut app = app("mhints");
        let t = Instant::now();
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("?"), t);
        assert!(matches!(app.mode, Mode::Help { .. }));
        // A quick press anywhere right after the click that opened it is
        // part of that gesture — not a click outside Help.
        render(&mut app, 120, 30);
        click(&mut app, (0, 0), t + ms(150));
        assert!(matches!(app.mode, Mode::Help { .. }), "swallowed");
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("any other key"), t + ms(1000));
        assert!(matches!(app.mode, Mode::Normal), "Help closed");
        // Nothing running: Quit asks, and its Quit quits once ready.
        render(&mut app, 120, 30);
        app.now = t + ms(2000);
        assert_eq!(
            click_on(&mut app, &Hit::Hint("q"), t + ms(2000)),
            Command::None
        );
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.kind == Kind::QuitIdle));
        let when = ready(&mut app, 120, 30, t + ms(2020));
        assert_eq!(click_on(&mut app, &Hit::Button(1), when), Command::Quit);
        // Running: the quit dialog.
        app.mode = Mode::Normal;
        running(&mut app);
        render(&mut app, 120, 30);
        assert_eq!(
            click_on(&mut app, &Hit::Hint("q"), when + ms(1000)),
            Command::None
        );
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.kind == Kind::Quit));
        app.mode = Mode::Normal;
        render(&mut app, 120, 30);
        // `[Details c]`, double-clicked: open (a second `c` would close it).
        let details = spot(&app, &Hit::Activity("c"));
        // A triple click too (verification: a third press pressed it again).
        for i in 0..3 {
            click(&mut app, details, when + ms(3000 + 100 * i));
            render(&mut app, 120, 30);
            assert!(
                matches!(app.mode, Mode::Details { .. }),
                "{i}: {:?}",
                app.mode
            );
        }
        // `c/Esc close` closes them.
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("c/Esc"), when + ms(4000));
        assert!(matches!(app.mode, Mode::Normal));
        // `Enter actions`, double-clicked: the menu opens, and stays (the
        // second press never lands on its `Esc close`; review TEST-B-2).
        app.running = false;
        render(&mut app, 120, 30);
        let enter = spot(&app, &Hit::Hint("Enter"));
        click(&mut app, enter, when + ms(5000));
        render(&mut app, 120, 30);
        click(&mut app, enter, when + ms(5100));
        assert!(matches!(app.mode, Mode::Menu(_)), "{:?}", app.mode);
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("Esc"), when + ms(6000));
        assert!(matches!(app.mode, Mode::Normal));
        // Tab; ]f; ↑↓ says what moves.
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("Tab"), when + ms(7000));
        assert_eq!(app.focus, Focus::View);
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("↑↓"), when + ms(8000));
        assert!(app
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("wheel")));
        app.select(Selection::Unit("u-lib".into()));
        app.focus = Focus::View;
        render(&mut app, 120, 12);
        let rows = app.layout.pair_rows.clone();
        click_on(&mut app, &Hit::Hint("]f"), when + ms(9000));
        assert_eq!(
            Some(app.scroll),
            rows.iter().copied().find(|r| *r > 0).or(Some(0))
        );
        // The checks' PgDn.
        app.mode = Mode::Verdict {
            selected: 0,
            scroll: 0,
        };
        render(&mut app, 120, 12);
        click_on(&mut app, &Hit::Hint("PgDn"), when + ms(10000));
        assert!(matches!(app.mode, Mode::Verdict { scroll, .. } if scroll > 0));
    }

    /// §7, reviews USE-B-10, SAFE-B-5, SAFE-B-6: a drag — a press and a
    /// release outside what it pressed — says how to select text, unless
    /// the press itself said something; a key pressed and dragged off does
    /// nothing; Ctrl or Alt clicks are never an answer; mouse events outside
    /// the frame are ignored.
    #[test]
    fn a_drag_says_how_to_select_text_and_presses_nothing() {
        let mut app = app("mdrag");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        let said = |app: &App| app.notice.as_ref().map(|n| n.text.clone());
        click(&mut app, unit, t);
        assert_ne!(said(&app).as_deref(), Some(DRAG_HINT));
        // A slip of one cell: a click.
        event(&mut app, LEFT, unit, t + ms(1000));
        event(&mut app, UP, (unit.0 + 1, unit.1 + 1), t + ms(1100));
        assert_ne!(said(&app).as_deref(), Some(DRAG_HINT));
        // Two cells along the row, or across the View's code: a drag.
        event(&mut app, LEFT, unit, t + ms(2000));
        event(&mut app, UP, (unit.0 + 2, unit.1), t + ms(2100));
        assert_eq!(said(&app).as_deref(), Some(DRAG_HINT));
        app.notice = None;
        event(&mut app, LEFT, (60, 7), t + ms(2500));
        event(&mut app, UP, (80, 9), t + ms(2600));
        assert_eq!(said(&app).as_deref(), Some(DRAG_HINT));
        // A key's words, from its release on it, are never covered.
        render(&mut app, 120, 30);
        let arrows = spot(&app, &Hit::Hint("↑↓"));
        event(&mut app, LEFT, arrows, t + ms(2800));
        event(&mut app, UP, (arrows.0 + 1, arrows.1), t + ms(2810));
        assert!(said(&app).is_some_and(|w| w.contains("wheel")));
        // A key pressed and dragged off: not pressed.
        render(&mut app, 120, 30);
        let help = spot(&app, &Hit::Hint("?"));
        event(&mut app, LEFT, help, t + ms(3000));
        event(&mut app, UP, (help.0, help.1 - 5), t + ms(3100));
        assert!(matches!(app.mode, Mode::Normal));
        // Ctrl or Alt: ignored.
        for modifiers in [KeyModifiers::ALT, KeyModifiers::CONTROL] {
            let ev = MouseEvent {
                kind: LEFT,
                column: help.0,
                row: help.1,
                modifiers,
            };
            app.on_event(TermEvent::Mouse(ev), t + ms(4000), Duration::ZERO);
            event(&mut app, UP, help, t + ms(4050));
            assert!(matches!(app.mode, Mode::Normal), "{modifiers:?}");
        }
        // Outside the frame: ignored — a press outside never pairs with a
        // release inside.
        app.open_menu();
        render(&mut app, 120, 30);
        click(&mut app, (120, 5), t + ms(5000));
        click(&mut app, (5, 30), t + ms(6000));
        assert!(
            matches!(app.mode, Mode::Menu(_)),
            "outside the frame: ignored"
        );
        event(&mut app, LEFT, (40, 5), t + ms(7000));
        event(&mut app, UP, (130, 5), t + ms(7100));
        event(&mut app, UP, (40, 9), t + ms(7200));
        assert_ne!(said(&app).as_deref(), Some(DRAG_HINT));
    }

    /// §7: Help's "Mouse on/off" (a key, `m`, and a click on its line);
    /// with the mouse off every mouse event is ignored.
    #[test]
    fn help_turns_the_mouse_off_and_on() {
        let mut app = app("mtoggle");
        let t = Instant::now();
        key(&mut app, '?');
        let screen = text(&render(&mut app, 120, 30));
        assert!(
            screen.contains("Mouse: on — click here, or press m, to turn it off"),
            "{screen}"
        );
        let (r, _) = app
            .hits
            .iter()
            .rev()
            .find(|(_, h)| *h == Hit::Hint("m"))
            .unwrap();
        let toggle = (r.x + 2, r.y);
        click(&mut app, toggle, t);
        assert!(!app.mouse);
        assert!(matches!(app.mode, Mode::Help { .. }), "Help stays open");
        render(&mut app, 120, 30);
        click(&mut app, toggle, t + ms(1000));
        assert!(!app.mouse, "a click with the mouse off is ignored");
        assert!(text(&render(&mut app, 120, 30)).contains("Mouse: off — press m to turn it on"));
        key(&mut app, 'm');
        assert!(app.mouse);
        key(&mut app, 'm');
        key(&mut app, 'z');
        assert!(matches!(app.mode, Mode::Normal));
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        click(&mut app, unit, t + ms(2000));
        assert_eq!(app.selection, Selection::Project, "the mouse is off");
    }

    /// The details sit over the lower half: the tree above answers clicks
    /// and the wheel as without them; the wheel over them scrolls them.
    /// Below 80 columns they leave the activity rows and the hint bar, so a
    /// click closes them (review USE-B-4).
    #[test]
    fn the_details_leave_the_panes_above_them_alive() {
        let mut app = app("mdetails");
        let t = Instant::now();
        running(&mut app);
        key(&mut app, 'c');
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        let over = spot(&app, &Hit::Overlay);
        click(&mut app, over, t);
        assert_eq!(app.selection, Selection::Project, "a click on the details");
        click(&mut app, unit, t + ms(1000));
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        assert!(matches!(app.mode, Mode::Details { .. }));
        app.mode = Mode::Details { scroll: 0 };
        event(&mut app, MouseEventKind::ScrollDown, over, t + ms(2000));
        assert!(
            matches!(app.mode, Mode::Details { scroll: 3 }),
            "{:?}",
            app.mode
        );
        // Below 80 columns.
        let buffer = render(&mut app, 70, 24);
        assert!(text(&buffer)
            .lines()
            .last()
            .unwrap()
            .contains("c/Esc close"));
        click_on(&mut app, &Hit::Hint("c/Esc"), t + ms(3000));
        assert!(matches!(app.mode, Mode::Normal));
    }

    /// Review PROC-B-5, TEST-B-4: an event may have waited in the queue
    /// since the last poll that saw it empty — a poll that found nothing,
    /// or had to wait for its event; never since one that returned at once.
    #[test]
    fn the_queue_clock_bounds_how_long_input_waited() {
        let t = Instant::now();
        let mut q = QueueClock::new(t);
        q.polled(t, false, t + ms(60));
        assert_eq!(q.late(t + ms(70)), ms(10));
        // Returned at once: the event may have waited since the last empty
        // poll — a stall included.
        q.polled(t + ms(500), true, t + ms(500));
        assert_eq!(q.late(t + ms(500)), ms(440));
        // Had to wait: it came after the poll began.
        q.polled(t + ms(600), true, t + ms(630));
        assert_eq!(q.late(t + ms(631)), ms(31));
    }
}
