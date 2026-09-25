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
/// From this many columns the right side is reserved for the chat (§10).
pub const CHAT_FROM: u16 = 150;
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
    // always shows it (the name gives way).
    let show_word = !word.is_empty() && !internal && (name_w + 2 + word_w <= room || selected);
    let (name_text, pad, word_text) = if show_word {
        let word_text = ellipsis(&word, room.saturating_sub(2).max(1).min(word_w));
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
    app.tree_offset = follow(app.rows.len(), height, cursor, app.tree_offset);
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
            Span::styled(format!("{} ", s.glyph()), glyph_style(s.glyph())),
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
                (s.stale, "changed"),
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
            let key = format!("{} {}", info.state.glyph(), info.state.word());
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
    } else if let Some(note) = &app.snapshot.note {
        lines.extend(wrapped(note, width, dim()));
    }
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
        links.get(app.link).map(|(row, _)| *row)
    } else {
        None
    };
    if let Some(row) = link_row {
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
    for (row, sel) in &links {
        if *row >= app.scroll && *row < app.scroll + height {
            let y = area.y + (*row - app.scroll) as u16;
            app.hits
                .push((Rect::new(area.x, y, area.width, 1), Hit::Row(sel.clone())));
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
        app.hits.push((Rect::new(x, area.y, w, 1), Hit::Hint(key)));
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
            if app.links.is_empty() {
                h.push(("↑↓", "scroll"));
                h.push(("←→", "side/back"));
                h.push(("Enter", "actions"));
            } else {
                h.push(("↑↓", "choose"));
                h.push(("Enter", "go there"));
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
    let tail: [(&'static str, &'static str); 2] = [("?", "help"), ("q", "quit")];
    let entry = |(k, v): &(&str, &str)| format!(" {k} {v} ");
    let tail_w: usize = tail.iter().map(|e| width_of(&entry(e)) + 1).sum();
    let mut chosen = Vec::new();
    let mut used = tail_w;
    for e in hints(app) {
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
    "Move with the arrow keys.",
    "Enter shows what you can do with the selection.",
    "? shows this screen.",
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

fn help_rows(width: usize) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for l in HELP_INTRO {
        rows.extend(wrapped(l, width, bold()));
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
    rows
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
    if let Some(why) = &m.footer {
        rows.push(Line::from(""));
        rows.extend(wrapped(
            &format!("Why not: {why}"),
            inner_w,
            Style::default().fg(Color::Yellow),
        ));
    }
    let height = (rows.len() as u16 + 2).min(area.height);
    let rect = centered(area, width, height);
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
    let width = DIALOG_COLUMNS.min(area.width.saturating_sub(2)).max(20);
    let inner_w = width.saturating_sub(2) as usize;
    let mut rows: Vec<Line<'static>> = Vec::new();
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
    // The argv must be seen to its end before the dialog can arm.
    c.dialog.seen = c.dialog.seen || scroll + page >= total;
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
    let state = c.dialog.state_text();
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
            safe(&format!(" {} ", c.title), inner_w),
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
    for (i, bx, w) in spots {
        app.hits
            .push((Rect::new(inner.x + bx, bar.y, w, 1), Hit::Button(i)));
    }
}

fn draw_overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.mode.clone() {
        Mode::Normal => {}
        Mode::Menu(m) => draw_menu(frame, app, area, &m),
        Mode::Dialog(_) => draw_dialog(frame, app, area),
        Mode::Details { scroll } => {
            let rect = if area.width < SINGLE_PANE_BELOW {
                area
            } else {
                let h = area.height / 2;
                Rect::new(area.x, area.y + area.height - h - 3, area.width, h)
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
        }
        Mode::Help { scroll } => {
            let rect = centered(
                area,
                pct(area.width, 86).max(40),
                pct(area.height, 86).max(10),
            );
            let rows = help_rows(rect.width.saturating_sub(2) as usize);
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
        }
    }
}

/// Draw the whole cockpit, recording the layout and the clickable regions.
pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = frame.area();
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
        // At 150 columns or more the right side is reserved for the chat
        // (§10); nothing is drawn there yet.
        let chat_w = if area.width >= CHAT_FROM {
            area.width / 4
        } else {
            0
        };
        let [files, view, _chat] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(files_w),
                Constraint::Min(20),
                Constraint::Length(chat_w),
            ])
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
}
