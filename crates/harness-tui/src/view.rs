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
use crate::featmap;
use crate::files::{self, FileState, UnitState};
use crate::highlight::{Class, Pieces};
use crate::menu::MODEL_SEPARATOR;
use crate::model::UnitView;
use crate::narrate::{check_words, elapsed_words};
use crate::speed;
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

mod chat_pane;
pub use chat_pane::{state_word, CHAT_COLUMN_FROM, CHAT_COLUMN_MAX, VIEW_KEEPS};

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
        "⚠" | "!" | "◐" | "⚑" | "↻" => Style::default().fg(Color::Yellow),
        "✓" | "◉" => Style::default().fg(Color::Green),
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

/// A dialog's height for `rows` rows of words on a screen `screen` rows
/// high: its rows and the two pinned at the bottom, within the screen —
/// rows beyond a u16 (a long answer) never wrap the count (fix check 2,
/// finding 7).
fn dialog_height(rows: usize, screen: u16) -> u16 {
    u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_add(4)
        .min(screen.saturating_sub(2))
        .max(6)
}

/// A chat answer's rows in a dialog `width` columns wide (§3.2): every
/// line whole — filtered, hard-wrapped, its indentation kept — behind a
/// gutter only the cockpit writes: a line's first row carries its number,
/// the rows that go on with it none. No answer can pass a line of its own
/// off as the rest of the line above (review SAF-2; fix check N1).
pub(crate) fn answer_rows(text: &str, width: usize) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let digits = lines.len().to_string().len();
    let body = width.saturating_sub(digits + 2).max(1);
    let mut rows = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // A CRLF line's `\r` shown as ␍, not as the filter's `?` (fix
        // check 2, finding 7): the harness files it as it is.
        let (line, cr) = match line.strip_suffix('\r') {
            Some(l) => (l, "␍"),
            None => (*line, ""),
        };
        let line = format!("{}{cr}", Sanitizer::default().push(line));
        for (j, row) in hard_wrap(&line, body).into_iter().enumerate() {
            rows.push(if j == 0 {
                format!("{:>digits$}│ {row}", i + 1)
            } else {
                format!("{:>digits$}┆ {row}", "")
            });
        }
    }
    rows
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
        Selection::Features => match &app.features.group {
            featmap::Group::NoFile => "Features (none yet)".into(),
            featmap::Group::Invalid(_) => "Features (error)".into(),
            featmap::Group::Valid => format!("Features ({})", app.features.features.len()),
        },
        Selection::Feature(id) => feature_row_name(app, id),
        Selection::Speed => app.speed.label(),
    }
}

/// A feature's name in the tree: the person's words, filtered; when two
/// cut names would read the same, the id replaces the tail (§8.3).
fn feature_row_name(app: &App, id: &str) -> String {
    let name_of = |f: &featmap::FeatureView| display::line(&f.name);
    let Some(f) = app.features.feature(id) else {
        return id.to_string();
    };
    let name = name_of(f);
    let cut: String = name.chars().take(FEATURE_NAME_ROOM).collect();
    let collides = app.features.features.iter().any(|g| {
        g.id != f.id
            && name_of(g)
                .chars()
                .take(FEATURE_NAME_ROOM)
                .collect::<String>()
                == cut
    });
    if collides && name.chars().count() > FEATURE_NAME_ROOM.saturating_sub(id.len() + 2) {
        let keep = FEATURE_NAME_ROOM.saturating_sub(id.len() + 2);
        format!("{} ·{id}", name.chars().take(keep).collect::<String>())
    } else {
        name
    }
}

/// The room a feature's name has in the tree at 80 columns (§8.3).
const FEATURE_NAME_ROOM: usize = 14;

/// A row's glyph and state word (a rollup for the project and directories).
fn row_label(app: &App, sel: &Selection) -> (&'static str, String) {
    match sel {
        Selection::Project => ("", app.files.rollup("").text()),
        Selection::Dir(d) => ("", app.files.rollup(d).text()),
        Selection::Units | Selection::Features | Selection::Speed => app.node_label(sel),
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
    if matches!(
        sel,
        Selection::Project | Selection::Units | Selection::Features | Selection::Speed
    ) {
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
    // The wheel lets the selected row scroll away until the next key in the
    // panes with the focus in the tree, or a click on a row (§R6); otherwise
    // the tree keeps it in view.
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
            let mut origin = if unit.crate_dir.is_some() {
                format!("Crate {} · {verdict}", provenance_words(unit))
            } else {
                "No crate yet".to_string()
            };
            if let Some(marker) = coverage_marker(app, unit) {
                origin.push_str(&format!(" · {marker}"));
            }
            lines.extend(wrapped(&origin, width, dim()));
        }
    }
    if let Some(line) = unit_features_line(app, &unit.unit.id) {
        lines.extend(wrapped(&line, width, Style::default().fg(Color::Cyan)));
    }
    if let Some((first, second)) = app.speed.unit_header(&unit.unit.id) {
        lines.push(Line::from(clipped(vec![Span::raw(first)], width)));
        lines.push(Line::from(clipped(
            vec![Span::styled(second, dim())],
            width,
        )));
    }
    let advice = app.speed.advice(unit, !app.config.providers.is_empty());
    for d in &advice.differences {
        lines.extend(wrapped(d, width, Style::default().fg(Color::Red)));
    }
    if let Some(next) = &advice.next {
        lines.extend(wrapped(&format!("Next: {next}"), width, bold()));
    }
    if let Some(change) = &advice.change {
        lines.extend(wrapped(change, width, Style::default()));
    }
    lines
}

/// Which of the person's features ran a function (§8.4), from a current map.
fn function_features_line(app: &App, file: &str, name: &str) -> Option<String> {
    let model = &app.features;
    // Only from a current, complete map: "none" is a negative claim (the
    // unit line says "not known" otherwise; review C5).
    if model.group != featmap::Group::Valid || model.features.is_empty() || !model.complete {
        return None;
    }
    let pair = (file.to_string(), name.to_string());
    if model.unwatched.contains(&pair) {
        return Some(match model.unwatched_why.get(&pair) {
            Some(why) => format!("Not watched by the map: {why}."),
            None => "Not watched by the map (the probe could not put a note in it).".into(),
        });
    }
    Some(match model.by_function.get(&pair) {
        Some(ids) => format!(
            "Run by: {}",
            ids.iter()
                .filter_map(|id| model.feature(id).map(|f| display::line(&f.name)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => "Run by none of your features.".into(),
    })
}

/// How the unit's verdict covers today's features, in words (§3): a marker
/// beside the verdict, never its state.
fn coverage_marker(app: &App, unit: &UnitView) -> Option<String> {
    use harness_core::features::Coverage;
    let Some(Coverage::Behind(reasons)) = &unit.report.features else {
        return None;
    };
    let red = unit.verdict.as_ref().is_some_and(|v| !v.green);
    let invalid_now = matches!(app.features.group, featmap::Group::Invalid(_));
    let mut words: Vec<String> = Vec::new();
    for reason in reasons {
        words.push(match reason.as_str() {
            "not-yet" if red => "not checked — the verdict stopped before them".into(),
            "not-yet" => "not checked on this unit yet — Re-check it".into(),
            "changed" => "not checked since you changed them — Re-check it".into(),
            "invalid" if invalid_now => {
                "not checked — features.toml has an error (fix it first)".into()
            }
            "invalid" => "not checked — features.toml had an error then (Re-check it)".into(),
            // The facts do not describe the C as it is (review O2): a scan
            // comes first, or the Re-check records the same.
            "program"
                if app
                    .snapshot
                    .features_now
                    .as_ref()
                    .is_some_and(|n| n.program == harness_core::features::STALE_PROGRAM) =>
            {
                "not known — the C changed since the scan: Scan the project, then Re-check it"
                    .into()
            }
            "program" => "checked before other C changed — Re-check it".into(),
            "skipped" => {
                // Not-in-program skips are the unit's place, not a scenario
                // to fix: said apart (review C14).
                let reasons: Vec<harness_core::features::SkipReason> = unit
                    .verdict
                    .as_ref()
                    .map(|v| {
                        v.inputs
                            .features_skipped
                            .iter()
                            .filter_map(|e| harness_core::features::parse_skip(e))
                            .map(|(_, _, r)| r)
                            .collect()
                    })
                    .unwrap_or_default();
                let outside = reasons
                    .iter()
                    .filter(|r| **r == harness_core::features::SkipReason::NotInProgram)
                    .count();
                let n = reasons.len() - outside;
                match (n, outside) {
                    (0, 0) => continue,
                    (0, _) => "not part of the program — nothing to do".into(),
                    (n, _) => format!("{n} scenario{} could not run — see Features", plural_s(n)),
                }
            }
            _ => continue,
        });
    }
    (!words.is_empty()).then(|| format!("your features: {}", words.join("; ")))
}

/// Which features run a unit's functions (§8.4) — only from a current,
/// complete map; failures first.
pub(crate) fn unit_features_line(app: &App, unit_id: &str) -> Option<String> {
    let model = &app.features;
    if model.group != featmap::Group::Valid || model.features.is_empty() {
        return None;
    }
    let uf = model.by_unit.get(unit_id)?;
    if uf.outside {
        return Some("Not part of the program your features run — its verdicts skip them.".into());
    }
    if model.no_single_main {
        return Some(
            "Features need a program with one main() — this target's facts show none.".into(),
        );
    }
    if !model.complete {
        return Some("Which features run it: not known — map the features.".into());
    }
    if uf.running.is_empty() {
        return Some(if uf.unwatched == 0 {
            "None of your features runs this unit's functions, so their checks pass whatever \
             its Rust does."
                .into()
        } else {
            format!(
                "None of your features ran its watched functions; {} of its functions could not \
                 be watched, so this is not proof.",
                uf.unwatched
            )
        });
    }
    let result_of = |fid: &str| {
        model
            .feature(fid)
            .and_then(|f| f.units.iter().find(|r| r.unit == unit_id))
            .map(|r| r.result.clone())
    };
    let names = |pick: &dyn Fn(&featmap::UnitResult) -> bool| -> Vec<String> {
        uf.running
            .iter()
            .filter(|(fid, _)| result_of(fid).as_ref().is_some_and(pick))
            .filter_map(|(fid, _)| model.feature(fid).map(|f| display::line(&f.name)))
            .collect()
    };
    let failed = names(&|r| matches!(r, featmap::UnitResult::Failed(_)));
    // A unit that is still C is not "not re-checked" (review C8).
    let has_rust = app.snapshot.units.iter().any(|u| {
        u.unit.id == unit_id
            && matches!(
                u.unit.status,
                harness_core::plan::UnitStatus::Verified | harness_core::plan::UnitStatus::Merged
            )
    });
    let pending = if has_rust {
        names(&|r| {
            matches!(
                r,
                featmap::UnitResult::NotChecked | featmap::UnitResult::Absent
            )
        })
    } else {
        Vec::new()
    };
    let n = uf.running.len();
    let mut text = format!(
        "{n} of your features run{} it",
        if n == 1 { "s" } else { "" }
    );
    if !failed.is_empty() {
        text.push_str(&format!(
            " · {} failed: {}",
            failed.len(),
            failed.join(", ")
        ));
    } else if !pending.is_empty() {
        text.push_str(&format!(" · {} not re-checked", pending.len()));
    } else if names(&|r| *r == featmap::UnitResult::Passed).len() == n {
        text.push_str(" · all passed");
    } else if !has_rust {
        text.push_str(" · still C");
    }
    text.push_str(" — see Features");
    Some(text)
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
    let feature_prefix = harness_core::features::CHECK_PREFIX;
    let feature_passes = v
        .checks
        .iter()
        .filter(|c| c.passed && c.name.starts_with(feature_prefix))
        .count();
    for check in v.checks.iter().filter(|c| !c.passed).chain(
        v.checks
            .iter()
            .filter(|c| c.passed && !c.name.starts_with(feature_prefix)),
    ) {
        let words = check_words(&check.name);
        match grouped
            .iter_mut()
            .find(|(p, w, _)| *p == check.passed && *w == words)
        {
            Some((_, _, n)) => *n += 1,
            None => grouped.push((check.passed, words, 1)),
        }
    }
    if feature_passes > 0 {
        let reach = features_reach_words(app, v);
        grouped.push((true, format!("scenarios ×{feature_passes}{reach}"), 1));
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

/// The overlay's note under a feature check: the feature's name and
/// arguments, and whether its feature runs the unit (§8.4).
fn feature_check_note(app: &App, unit: &str, name: &str, passed: bool) -> Option<String> {
    let rest = name.strip_prefix(harness_core::features::CHECK_PREFIX)?;
    let (fid, sid) = rest.split_once('/')?;
    let f = app.features.feature(fid)?;
    let s = f.scenarios.iter().find(|s| s.id == sid)?;
    let mut text = format!(
        "{}: {} {}",
        display::line(&f.name),
        app.snapshot.program_name,
        s.argv.join(" ")
    );
    // An older verdict ran an earlier features file: today's arguments and
    // map may not be what it ran (review C14).
    if app.shown_verdict().is_some_and(|v| {
        app.snapshot
            .features_now
            .as_ref()
            .is_some_and(|now| v.inputs.features != now.features)
    }) {
        text.push_str(" (from an earlier features file)");
        return Some(text);
    }
    if app.features.complete {
        let runs = app
            .features
            .by_unit
            .get(unit)
            .is_some_and(|uf| uf.running.iter().any(|(id, _)| id == fid));
        if !runs {
            text.push_str(if passed {
                " (its feature does not run this unit's functions)"
            } else {
                " (its feature does not run this unit's functions — yet it failed: the map may \
                 be incomplete)"
            });
        }
    }
    Some(text)
}

/// "(4 run this unit)" for a unit's passing feature checks, from a current
/// map; what the map cannot say, said (§8.4).
fn features_reach_words(app: &App, v: &harness_core::Verdict) -> String {
    let model = &app.features;
    if app
        .snapshot
        .features_now
        .as_ref()
        .is_some_and(|now| v.inputs.features != now.features)
    {
        return " (from an earlier features file)".into();
    }
    match &model.map {
        featmap::MapStatus::None | featmap::MapStatus::Unreadable(_) => " (not mapped yet)".into(),
        featmap::MapStatus::OutOfDate(_) => " (map out of date)".into(),
        featmap::MapStatus::Current if !model.complete => " (map incomplete)".into(),
        featmap::MapStatus::Current => {
            let Some(uf) = model.by_unit.get(&v.unit) else {
                return String::new();
            };
            let running: std::collections::BTreeSet<&str> =
                uf.running.iter().map(|(f, _)| f.as_str()).collect();
            let n = v
                .checks
                .iter()
                .filter(|c| c.passed)
                .filter_map(|c| c.name.strip_prefix(harness_core::features::CHECK_PREFIX))
                .filter(|rest| rest.split('/').next().is_some_and(|f| running.contains(f)))
                .count();
            format!(" ({n} run this unit)")
        }
    }
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
    let start = lines.len();
    lines.extend(summary_features(app, width, links, start));
    let start = lines.len();
    lines.extend(summary_speed(app, width, links, start));
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

/// The project summary's features line (§8.4), a link to the group.
fn summary_features(
    app: &App,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
    start: usize,
) -> Vec<Line<'static>> {
    let model = &app.features;
    let text = match &model.group {
        // A library (no single main()): features do not apply; say nothing.
        featmap::Group::NoFile if model.no_single_main => return Vec::new(),
        featmap::Group::NoFile => "Features: none yet — see Features".to_string(),
        featmap::Group::Invalid(_) => "Features: features.toml has an error — see Features".into(),
        featmap::Group::Valid => {
            let mut counts: Vec<(&'static str, usize)> = Vec::new();
            let mut caveat = false;
            for f in &model.features {
                if matches!(
                    f.state,
                    featmap::FeatureState::HoldsSoFar { .. } | featmap::FeatureState::AllMigrated
                ) {
                    caveat = true;
                }
                let key = summary_key(&f.state);
                match counts.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((key, 1)),
                }
            }
            let mut text = format!(
                "Features: {} — {}",
                model.features.len(),
                counts
                    .iter()
                    .map(|(k, n)| summary_count(k, *n))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if model.map.current() && model.watched > 0 {
                text.push_str(&format!(
                    " · they ran {} of {} watched functions",
                    model.ran, model.watched
                ));
            }
            let behind = model.recheck.len();
            if behind > 0 {
                text.push_str(&format!(
                    " · {behind} unit{} not re-checked",
                    plural_s(behind)
                ));
            }
            if caveat {
                text.push_str(" (each unit checked with only its own Rust swapped in)");
            }
            text
        }
    };
    let mut lines = vec![Line::from("")];
    links.push((start + lines.len(), Selection::Features));
    lines.extend(wrapped(&text, width, Style::default()));
    lines
}

/// The project summary's Speed line (docs/PERF-DESIGN.md §3.11), a link to
/// the group; nothing without a workloads file.
fn summary_speed(
    app: &App,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
    start: usize,
) -> Vec<Line<'static>> {
    let Some(text) = app.speed.summary_line() else {
        return Vec::new();
    };
    links.push((start, Selection::Speed));
    let mut lines = wrapped(&text, width, Style::default());
    for (fact, _) in app.speed.program_differences() {
        links.push((start + lines.len(), Selection::Speed));
        lines.extend(wrapped(
            &format!("Speed: {fact} — see Speed"),
            width,
            Style::default().fg(Color::Red),
        ));
    }
    lines
}

/// How a row's sentence names its side.
fn speed_side_label(side: &speed::SideKey) -> String {
    match side {
        speed::SideKey::C => "the C".into(),
        speed::SideKey::AsItStands => "the program as it stands".into(),
        speed::SideKey::Unit(id) => id.clone(),
    }
}

/// One row of the Speed View: the workload, its short form (dimmed with
/// "out of date: …" when stale), and its last-try or found-before line.
fn speed_row_lines(row: &speed::SpeedRow, column: usize, width: usize) -> Vec<Line<'static>> {
    let name = ellipsis(
        &display::line(&row.workload),
        column.saturating_sub(2).max(4),
    );
    let pad = column.saturating_sub(width_of(&name));
    let mut spans = vec![Span::raw(format!("  {name}{}", " ".repeat(pad)))];
    if row.out_of_date.is_empty() {
        spans.push(Span::raw(row.words.short.clone()));
    } else {
        spans.push(Span::styled(row.words.short.clone(), dim()));
        spans.push(Span::styled(
            format!(" · out of date: {}", row.out_of_date.join(", ")),
            dim(),
        ));
    }
    let mut lines = vec![Line::from(clipped(spans, width))];
    // Under its row, wrapped whole at a shallow indent: never cut mid-word
    // beside a wide workload column.
    for d in &row.words.details {
        if d.starts_with("last try:") || d.starts_with("a difference found before") {
            for part in word_wrap(d, width.saturating_sub(4)) {
                lines.push(Line::from(Span::styled(format!("    {part}"), dim())));
            }
        }
    }
    lines
}

/// The Speed View's sections as they are laid out: each row a link, its
/// sentence kept for the focused row.
struct SpeedSection<'l, 'r> {
    lines: &'l mut Vec<Line<'static>>,
    links: &'l mut Vec<(usize, Selection)>,
    sentences: Vec<Option<(speed::SideKey, &'r speed::SpeedRow)>>,
    column: usize,
    width: usize,
}

impl<'r> SpeedSection<'_, 'r> {
    fn push(
        &mut self,
        heading: Line<'static>,
        heading_link: Option<Selection>,
        side: speed::SideKey,
        rows: &'r [speed::SpeedRow],
    ) {
        if let Some(sel) = heading_link {
            self.links.push((self.lines.len(), sel));
            self.sentences.push(None);
        }
        self.lines.push(heading);
        for row in rows {
            let link = match &side {
                speed::SideKey::Unit(id) => Selection::Unit(id.clone()),
                _ => Selection::Speed,
            };
            self.links.push((self.lines.len(), link));
            self.sentences.push(Some((side.clone(), row)));
            self.lines
                .extend(speed_row_lines(row, self.column, self.width));
        }
    }
}

/// The Speed View (docs/PERF-DESIGN.md §3.11): the header the rows record,
/// the C alone, the program as it stands, each unit worst first; the
/// focused row's full sentence below.
fn speed_view(app: &App, width: usize, links: &mut Vec<(usize, Selection)>) -> Vec<Line<'static>> {
    let model = &app.speed;
    let mut lines = Vec::new();
    match &model.group {
        speed::Group::NoFile => {
            lines.extend(wrapped(
                "Speed compares the C against the Rust in use: the program runs your \
                 workloads — inputs you choose — first as the C, then with each unit's \
                 Rust, many times each, and perf says which is faster and by how much.",
                width,
                Style::default(),
            ));
            lines.push(Line::from(""));
            lines.extend(wrapped(
                "On this row, press Enter and choose Write your workloads file.",
                width,
                bold(),
            ));
            return lines;
        }
        speed::Group::NoWorkload => {
            lines.extend(wrapped(
                "Your workloads file has no workload yet.",
                width,
                Style::default(),
            ));
            lines.push(Line::from(""));
            lines.extend(wrapped(
                "On this row, press Enter and choose Edit the workloads file.",
                width,
                bold(),
            ));
            return lines;
        }
        speed::Group::FileError(why) => {
            lines.extend(wrapped(
                // The error names the file, its line and column.
                &display::line(why),
                width,
                Style::default().fg(Color::Yellow),
            ));
            lines.push(Line::from(""));
            lines.extend(wrapped(
                "On this row, press Enter and choose Edit the workloads file.",
                width,
                bold(),
            ));
            lines.extend(wrapped(
                "Until it is fixed, perf cannot measure — verify and migrations still work.",
                width,
                dim(),
            ));
            return lines;
        }
        _ => {}
    }
    lines.extend(wrapped(
        "Speed — your workloads, the C against the Rust in use",
        width,
        bold(),
    ));
    for h in &model.header {
        lines.extend(wrapped(h, width, dim()));
    }
    if model.measuring {
        lines.extend(wrapped(
            "A perf run is measuring now — rows appear as each finishes.",
            width,
            Style::default().fg(Color::Yellow),
        ));
    }
    for e in &model.errors {
        lines.extend(wrapped(e, width, Style::default().fg(Color::Yellow)));
    }
    lines.push(Line::from(""));
    // The workload column: the longest workload id + 2.
    let all_rows = model
        .c_rows
        .iter()
        .chain(&model.program_rows)
        .chain(model.units.iter().flat_map(|u| &u.rows));
    let column = all_rows
        .map(|r| width_of(&display::line(&r.workload)))
        .max()
        .unwrap_or(0)
        + 2;
    let column = column.min(width.saturating_sub(2 + 26).max(6));
    // Each row is a link (its unit, or Speed itself) so the focused one's
    // full sentence shows below.
    let mut section = SpeedSection {
        lines: &mut lines,
        links,
        sentences: Vec::new(),
        column,
        width,
    };
    if model.group == speed::Group::NotYetRun {
        section.lines.extend(wrapped(
            "Nothing measured yet — on this row, press Enter and choose Measure speed.",
            width,
            bold(),
        ));
    } else if model.c_rows.is_empty() {
        // Units measured alone (`--unit`): the C alone is not, yet.
        section
            .lines
            .push(Line::from(Span::styled("The original C", bold())));
        section.lines.extend(wrapped(
            "not measured yet — Measure speed on this row measures the C alone; measuring one \
             unit does not",
            width,
            dim(),
        ));
    } else {
        section.push(
            Line::from(Span::styled("The original C", bold())),
            None,
            speed::SideKey::C,
            &model.c_rows,
        );
    }
    if !model.program_rows.is_empty() {
        let held = model.held.len();
        let left = model.left_out.len();
        let heading = if left == 0 {
            format!("As it stands ({held} unit{})", plural_s(held))
        } else {
            format!(
                "As it stands ({held} of {} units — {left} left out)",
                held + left
            )
        };
        section.push(
            Line::from(clipped(vec![Span::styled(heading, bold())], width)),
            None,
            speed::SideKey::AsItStands,
            &model.program_rows,
        );
        for (fact, which) in model.program_differences() {
            section
                .lines
                .extend(wrapped(&fact, width, Style::default().fg(Color::Red)));
            section.lines.extend(wrapped(&which, width, dim()));
        }
    }
    for u in &model.units {
        section.push(
            Line::from(clipped(vec![Span::styled(u.id.clone(), bold())], width)),
            Some(Selection::Unit(u.id.clone())),
            speed::SideKey::Unit(u.id.clone()),
            &u.rows,
        );
    }
    let sentences = section.sentences;
    if !model.orphans.is_empty() {
        let more = match model.orphans_more {
            0 => String::new(),
            n => format!(" and {n} more"),
        };
        lines.push(Line::from(""));
        lines.extend(wrapped(
            &format!(
                "Results of units no longer in the plan (in migration/perf/units/): {}{more}",
                model.orphans.join(", ")
            ),
            width,
            dim(),
        ));
    }
    // The focused row's full sentence and details.
    let focused = (app.focus == Focus::View)
        .then_some(app.link)
        .flatten()
        .and_then(|l| sentences.get(l))
        .and_then(|s| s.as_ref());
    if let Some((side, row)) = focused {
        lines.push(Line::from(""));
        lines.extend(wrapped(
            &format!(
                "{} on {} — {}",
                speed_side_label(side),
                display::line(&row.workload),
                row.words.headline
            ),
            width,
            Style::default(),
        ));
        for d in &row.words.details {
            lines.extend(wrapped(d, width, dim()));
        }
        lines.extend(wrapped(&row.measured_on, width, dim()));
        if !row.out_of_date.is_empty() {
            lines.extend(wrapped(
                &format!("out of date: {}", row.out_of_date.join(", ")),
                width,
                Style::default().fg(Color::Yellow),
            ));
        }
    } else if !sentences.is_empty() {
        lines.push(Line::from(""));
        lines.extend(wrapped(
            "Move to a row (Tab, then ↓) to read its full words.",
            width,
            dim(),
        ));
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

/// "From the last map" preface when the map is out of date or unreadable
/// (§8.4): every map-derived line below it is the last map's, and no
/// negative claim is made from it.
fn map_preface(app: &App, width: usize) -> Vec<Line<'static>> {
    match &app.features.map {
        featmap::MapStatus::OutOfDate(why) => wrapped(
            &format!("From the last map — out of date ({})", why.join(", ")),
            width,
            Style::default().fg(Color::Yellow),
        ),
        featmap::MapStatus::Unreadable(why) => wrapped(
            &format!("The map could not be read: {why}"),
            width,
            Style::default().fg(Color::Yellow),
        ),
        _ => Vec::new(),
    }
}

/// The Features group's View (§8.4).
fn features_view(
    app: &App,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let model = &app.features;
    match &model.group {
        featmap::Group::NoFile => {
            lines.extend(wrapped(
                "A feature is something a person does with the program and can see the \
                 result of. Each is one or more runs of the whole program; every Re-check \
                 then runs them on the C and on the program with the unit's Rust.",
                width,
                Style::default(),
            ));
            lines.push(Line::from(""));
            if app.snapshot.facts.is_none() {
                lines.extend(wrapped("Scan the project first.", width, bold()));
            } else if model.no_single_main {
                lines.extend(wrapped(
                    "Features need a program with one main() — this target has none (a \
                     library?); not supported yet.",
                    width,
                    dim(),
                ));
            } else {
                lines.extend(wrapped(
                    "On this row, press Enter and choose Write your features file.",
                    width,
                    bold(),
                ));
            }
            return lines;
        }
        featmap::Group::Invalid(why) => {
            lines.extend(wrapped(
                &format!("features.toml has an error: {why}"),
                width,
                Style::default().fg(Color::Yellow),
            ));
            lines.push(Line::from(""));
            let next = if why.contains("symlink") {
                "Replace the symlink with a regular file outside the cockpit."
            } else if why.contains("newer harness") {
                "Update the harness."
            } else {
                "On this row, press Enter and choose Edit the features file."
            };
            lines.extend(wrapped(next, width, bold()));
            lines.extend(wrapped(
                "Until it is fixed, verdicts do not run your features — Re-checks and \
                 migrations still work, and say so.",
                width,
                dim(),
            ));
            return lines;
        }
        featmap::Group::Valid => {}
    }
    if model.features.is_empty() {
        lines.extend(wrapped(
            "No features yet — choose Edit the features file to add yours.",
            width,
            bold(),
        ));
        return lines;
    }
    if model.no_single_main {
        lines.extend(wrapped(
            "Features need a program with one main() — this target's facts show none (a \
             library?); not supported yet.",
            width,
            dim(),
        ));
    }
    lines.extend(map_preface(app, width));
    for f in &model.features {
        links.push((lines.len(), Selection::Feature(f.id.clone())));
        let glyph = f.state.glyph();
        let mut word = f.state.word();
        if let Some(also) = &f.also {
            word.push_str(&format!(" · {also}"));
        }
        lines.push(Line::from(clipped(
            vec![
                Span::styled(format!("{glyph} "), glyph_style(glyph)),
                Span::raw(display::line(&f.name)),
                Span::styled(format!("  {word}"), dim()),
            ],
            width,
        )));
        let count =
            |pick: fn(&featmap::UnitRow) -> bool| f.units.iter().filter(|r| pick(r)).count();
        // The units in the program (what "k of n" counts), and apart the
        // ones outside it; "not mapped" when the map has none of its
        // scenarios (review C6/C13).
        let outside = count(|r| r.result == featmap::UnitResult::Outside);
        let inside = f.units.len() - outside;
        let mapped = f.scenarios.iter().any(|s| s.record.is_some());
        let noted = f.scenarios.iter().all(|s| {
            s.record
                .as_ref()
                .is_some_and(|r| r.noted == "complete" && r.probe_agrees)
        });
        let mut parts = vec![if !mapped {
            "not mapped".to_string()
        } else if !noted && inside == 0 {
            "its units not known (map incomplete)".to_string()
        } else {
            format!("{inside} unit{}", plural_s(inside))
        }];
        if outside > 0 {
            parts.push(format!("{outside} outside the program"));
        }
        for (n, what) in [
            (count(|r| r.result == featmap::UnitResult::Passed), "pass"),
            (
                count(|r| matches!(r.result, featmap::UnitResult::Failed(_))),
                "fail",
            ),
            (
                count(|r| {
                    r.has_rust
                        && matches!(
                            r.result,
                            featmap::UnitResult::NotChecked | featmap::UnitResult::Absent
                        )
                }),
                "not re-checked",
            ),
            (
                count(|r| {
                    !r.has_rust
                        && matches!(
                            r.result,
                            featmap::UnitResult::NotChecked | featmap::UnitResult::Absent
                        )
                }),
                "still C",
            ),
        ] {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        lines.push(Line::from(Span::styled(
            clip(&format!("    {}", parts.join(" · ")), width),
            dim(),
        )));
    }
    lines.push(Line::from(""));
    if model.watched > 0 || !model.unwatched.is_empty() {
        let mut text = format!(
            "{}our features ran {} of the {} functions the map watches",
            if model.map.current() {
                "Y"
            } else {
                "In the last map, y"
            },
            model.ran,
            model.watched
        );
        if !model.unwatched.is_empty() {
            text.push_str(&format!(
                " ({} more could not be watched)",
                model.unwatched.len()
            ));
        }
        text.push('.');
        lines.extend(wrapped(&text, width, Style::default()));
    }
    if model.complete {
        let facts = app.snapshot.facts.as_ref();
        let never: Vec<(String, String)> = facts
            .map(|f| {
                let mut seen = std::collections::BTreeSet::new();
                f.symbols
                    .iter()
                    .map(|s| (s.file.clone(), s.name.clone()))
                    .filter(|p| seen.insert(p.clone()))
                    .filter(|p| !model.by_function.contains_key(p) && !model.unwatched.contains(p))
                    .collect()
            })
            .unwrap_or_default();
        if !never.is_empty() {
            lines.extend(wrapped(
                &format!(
                    "{} function{} no feature ran:",
                    never.len(),
                    plural_s(never.len())
                ),
                width,
                bold(),
            ));
            for (file, name) in never.iter().take(20) {
                links.push((lines.len(), Selection::Function(file.clone(), name.clone())));
                lines.push(Line::from(clipped(
                    vec![
                        Span::raw(format!("  {}()", short_symbol(name))),
                        Span::styled(format!("  {file}"), dim()),
                    ],
                    width,
                )));
            }
            if never.len() > 20 {
                lines.push(Line::from(Span::styled(
                    format!("  … {} more", never.len() - 20),
                    dim(),
                )));
            }
        }
    }
    lines.push(Line::from(""));
    lines.extend(wrapped(
        "Edited features.toml outside the cockpit? Press g to re-read.",
        width,
        dim(),
    ));
    lines
}

/// A feature state's key in the summary's counts.
fn summary_key(state: &featmap::FeatureState) -> &'static str {
    use featmap::FeatureState as S;
    match state {
        S::Failing => "failing",
        S::ScenarioCannotRun => "cannot-run",
        S::NeedsRecheck => "recheck",
        S::NotMapped => "not-mapped",
        S::MapOutOfDate => "out-of-date",
        S::MapIncomplete => "incomplete",
        S::ReachesNoUnit => "no-unit",
        S::AllMigrated => "migrated",
        S::HoldsSoFar { .. } => "holds",
        S::AllC => "all-c",
        S::SeeUnits => "see",
    }
}

/// "3 need a re-check", "1 holds so far": a count in words (review C13).
fn summary_count(key: &str, n: usize) -> String {
    let one = n == 1;
    let words = match key {
        "failing" => "failing",
        "cannot-run" if one => "has a scenario that cannot run",
        "cannot-run" => "have a scenario that cannot run",
        "recheck" if one => "needs a re-check",
        "recheck" => "need a re-check",
        "not-mapped" => "not mapped yet",
        "out-of-date" => "with the map out of date",
        "incomplete" => "with the map incomplete",
        "no-unit" if one => "reaches no unit",
        "no-unit" => "reach no unit",
        "migrated" => "with all units migrated",
        "holds" if one => "holds so far",
        "holds" => "hold so far",
        "all-c" => "all C",
        _ => "to see",
    };
    format!("{n} {words}")
}

fn plural_s(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// A unit result in words (§8.4's "Where its code lives").
fn result_words(row: &featmap::UnitRow) -> (String, Style) {
    if !row.has_rust
        && matches!(
            row.result,
            featmap::UnitResult::NotChecked | featmap::UnitResult::Absent
        )
    {
        return ("– still C".into(), dim());
    }
    match &row.result {
        featmap::UnitResult::Passed => ("✓ passed".into(), Style::default().fg(Color::Green)),
        featmap::UnitResult::Failed(ids) => (
            format!("✗ failed: {}", ids.join(", ")),
            Style::default().fg(Color::Red),
        ),
        featmap::UnitResult::CouldNotRun(reason) => {
            (format!("– could not run: {}", reason.words()), dim())
        }
        featmap::UnitResult::Absent => ("– not re-checked".into(), dim()),
        featmap::UnitResult::NotChecked => ("– not re-checked".into(), dim()),
        featmap::UnitResult::Outside => ("– outside the program".into(), dim()),
    }
}

/// The next step of a feature's state, in words (§8.2).
fn feature_next(f: &featmap::FeatureView) -> Option<String> {
    use featmap::FeatureState as S;
    Some(match &f.state {
        S::Failing => "Open the failing unit below and its verdict.".into(),
        S::ScenarioCannotRun => {
            let why = f
                .scenarios
                .iter()
                .find_map(|s| s.skipped.map(|r| (s.id.clone(), r)));
            match why {
                Some((id, reason)) => format!(
                    "Scenario {id}: {} — {}. Choose Edit the features file.",
                    reason.words(),
                    reason.what_to_do()
                ),
                None => "A scenario's C output differs between runs, or it did not exit — \
                         change or remove it (Edit the features file)."
                    .into(),
            }
        }
        S::NeedsRecheck => format!(
            "Your features are not checked on {} yet — Re-check {}:",
            if f.recheck.len() == 1 {
                "this unit"
            } else {
                "these units"
            },
            if f.recheck.len() == 1 { "it" } else { "each" }
        ),
        S::NotMapped | S::MapOutOfDate => "Map the features (press Enter).".into(),
        S::MapIncomplete => "A scenario's run left no usable notes, or behaved differently \
                             with them — see its line below."
            .into(),
        S::ReachesNoUnit => format!(
            "Its code is outside every unit ({} function{}), or the map could not watch it. \
             Nothing to do — or add a scenario that reaches a unit.",
            f.outside_units,
            plural_s(f.outside_units)
        ),
        S::AllMigrated | S::HoldsSoFar { .. } => {
            let mut text = "Each unit was checked with only its own Rust swapped in — no build \
                            has them all in Rust together yet."
                .to_string();
            if matches!(f.state, S::AllMigrated) && f.outside_units > 0 {
                text.push_str(&format!(
                    " {} function{} it runs {} outside every unit and stay C.",
                    f.outside_units,
                    plural_s(f.outside_units),
                    if f.outside_units == 1 { "is" } else { "are" }
                ));
            }
            text
        }
        S::AllC => return None,
        S::SeeUnits => "See its units below.".into(),
    })
}

/// A feature's View (§8.4).
fn feature_view(
    app: &App,
    id: &str,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(f) = app.features.feature(id) else {
        return lines;
    };
    let glyph = f.state.glyph();
    let mut word = f.state.word();
    if let Some(also) = &f.also {
        word.push_str(&format!(" · {also}"));
    }
    lines.push(Line::from(clipped(
        vec![
            Span::styled(format!("{glyph} "), glyph_style(glyph)),
            Span::styled(word, bold()),
            Span::styled(format!("  · id {}", f.id), dim()),
        ],
        width,
    )));
    if let Some(next) = feature_next(f) {
        lines.extend(wrapped(&next, width, Style::default()));
    }
    // F3's next step names its units, each a link (§8.2; review C3).
    if matches!(f.state, featmap::FeatureState::NeedsRecheck) {
        for u in &f.recheck {
            links.push((lines.len(), Selection::Unit(u.clone())));
            lines.push(Line::from(clipped(
                vec![
                    Span::raw(format!("  {u}")),
                    Span::styled("  open it, then Re-check", dim()),
                ],
                width,
            )));
        }
    }
    if app.features.no_single_main {
        lines.extend(wrapped(
            "Features need a program with one main() — this target's facts show none (a \
             library?); not supported yet.",
            width,
            dim(),
        ));
    }
    lines.push(Line::from(""));
    lines.extend(map_preface(app, width));
    lines.push(Line::from(Span::styled("Scenarios", bold())));
    let program = app.snapshot.program_name.clone();
    for s in &f.scenarios {
        let mut argv = vec![program.clone()];
        argv.extend(s.argv.iter().cloned());
        let mut text = format!("  {} · {}", s.id, argv.join(" "));
        if let Some(input) = s.input {
            text.push_str(&format!(" (the sample: {input})"));
        }
        lines.extend(wrapped(&text, width, Style::default()));
        if let Some(r) = &s.record {
            let stderr = if r.stderr_bytes == 0 {
                "stderr empty".to_string()
            } else if r.stderr_head.is_empty() {
                format!("stderr {} bytes", r.stderr_bytes)
            } else {
                format!("stderr: {}", r.stderr_head)
            };
            lines.extend(wrapped(
                &format!("    {} · stdout {} bytes · {stderr}", r.end, r.stdout_bytes),
                width,
                dim(),
            ));
            let flag = if !r.stable {
                Some("its output differs between runs — it cannot be a check".to_string())
            } else if !r.end.starts_with("exit ") {
                // Before "compares little": a run that crashed or timed out
                // is no check at all (review C10).
                Some(format!(
                    "it did not exit ({}) — it cannot be a check",
                    r.end
                ))
            } else if !r.probe_agrees {
                Some("the run with notes behaved differently — its map may be wrong".into())
            } else if r.noted != "complete" {
                Some(format!(
                    "no notes were recorded ({})",
                    r.reason.as_deref().unwrap_or("unreadable")
                ))
            } else if !r.end.starts_with("exit 0") || r.stdout_bytes == 0 {
                Some(
                    "it compares little: only the exit status and stderr (it exited non-zero \
                     or printed nothing to stdout)"
                        .into(),
                )
            } else {
                None
            };
            if let Some(flag) = flag {
                lines.extend(wrapped(
                    &format!("    ⚑ {flag}"),
                    width,
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        if let Some(reason) = s.skipped {
            lines.extend(wrapped(
                &format!(
                    "    ⚑ could not run: {} — {}",
                    reason.words(),
                    reason.what_to_do()
                ),
                width,
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Where its code lives", bold())));
    if f.scenarios.iter().all(|s| s.record.is_none()) {
        lines.extend(wrapped("  (not known — map the features)", width, dim()));
    } else if f.units.is_empty() && f.outside_units == 0 && !app.features.complete {
        // Records without notes (fix check N5): not "nowhere".
        lines.extend(wrapped(
            "  (not known — the map has no usable notes for it)",
            width,
            dim(),
        ));
    }
    for row in &f.units {
        let (glyph, uword) = app
            .snapshot
            .units
            .iter()
            .position(|u| u.unit.id == row.unit)
            .and_then(|i| app.files.units.get(i))
            .map(|i| (i.state.glyph(), i.state.word()))
            .unwrap_or(("", String::new()));
        let (result, style) = result_words(row);
        links.push((lines.len(), Selection::Unit(row.unit.clone())));
        // The feature's result right after the id: a narrow View cuts the
        // unit's own word first, never the result (review C7).
        lines.push(Line::from(clipped(
            vec![
                Span::styled(format!("  {glyph:<2} "), glyph_style(glyph)),
                Span::raw(row.unit.clone()),
                Span::styled(format!("  {result}"), style),
                Span::styled(
                    format!("  · runs {} of its {} · {uword}", row.ran, row.of),
                    dim(),
                ),
            ],
            width,
        )));
    }
    if f.outside_units > 0 {
        lines.extend(wrapped(
            &format!(
                "  Outside every unit: {} function{} (headers, files with no exported functions)",
                f.outside_units,
                plural_s(f.outside_units)
            ),
            width,
            dim(),
        ));
    }
    if !f.also_fails.is_empty() {
        // "not in them" is a negative claim: only from a complete map.
        lines.push(Line::from(Span::styled(
            if app.features.complete {
                "Also fails on (its functions are not in them)"
            } else {
                "Fails on"
            },
            Style::default().fg(Color::Red),
        )));
        for u in &f.also_fails {
            links.push((lines.len(), Selection::Unit(u.clone())));
            lines.push(Line::from(Span::raw(format!("  {u}"))));
        }
    }
    if app.features.features.len() >= 2 && app.features.complete {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Only this feature runs", bold())));
        if f.specific.is_empty() {
            lines.extend(wrapped(
                "  everything it runs is also run by another feature",
                width,
                dim(),
            ));
        }
        for (file, name) in &f.specific {
            links.push((lines.len(), Selection::Function(file.clone(), name.clone())));
            lines.push(Line::from(clipped(
                vec![
                    Span::raw(format!("  {}()", short_symbol(name))),
                    Span::styled(format!("  {file}"), dim()),
                ],
                width,
            )));
        }
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
    // The chat's strip on this border: the title is cut before it (review
    // USE-10).
    let room = if app.chat_on && area.width >= 20 {
        (area.width as usize).saturating_sub(20)
    } else {
        area.width as usize
    };
    let block = pane_block(ellipsis(&view_title(app), room.max(4)), focused);
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
        Selection::Features => Some(features_view(app, width, &mut links)),
        Selection::Feature(id) => Some(feature_view(app, id, width, &mut links)),
        Selection::Speed => Some(speed_view(app, width, &mut links)),
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
        if let Selection::Function(file, name) = &sel {
            if let Some(line) = function_features_line(app, file, name) {
                head.extend(wrapped(&line, width, Style::default().fg(Color::Cyan)));
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
    if let Selection::Function(file, name) = &sel {
        if let Some(line) = function_features_line(app, file, name) {
            head.extend(wrapped(&line, width, Style::default().fg(Color::Cyan)));
        }
    }
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
    let in_chat = app.focus == Focus::Chat;
    let (lead, lead_style) = match (&app.run, app.running) {
        (Some(run), true) => {
            let elapsed = run.started.elapsed();
            let frame = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
            // In the chat letters are text: the keys that work there.
            buttons.push((
                if in_chat {
                    "[Cancel Ctrl-X]"
                } else {
                    "[Cancel x]"
                },
                "x",
            ));
            buttons.push((if in_chat { "[Details]" } else { "[Details c]" }, "c"));
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
                buttons.push((
                    if in_chat {
                        "[Try again]"
                    } else {
                        "[Try again t]"
                    },
                    "t",
                ));
            }
            if app.run.is_some() {
                buttons.push((if in_chat { "[Details]" } else { "[Details c]" }, "c"));
            }
            let text = match (app.chat_waits_why(app.now), &app.last) {
                (Some(why), _) => why.to_string(),
                (None, Some(last)) => format!("Ready. Last: {last}"),
                (None, None) => "Ready.".to_string(),
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
    // A request of the chat's stays announced outside the chat until it is
    // answered, naming the key back from the focused pane (§3.2).
    if app.focus != Focus::Chat {
        if let Some(r) = app.asks.shown() {
            let key = if app.focus == Focus::Files {
                "Shift-Tab"
            } else {
                "Tab"
            };
            return Line::from(Span::styled(
                ellipsis(
                    &format!(" The chat asks: {} — {key} to the chat", r.words),
                    width,
                ),
                Style::default().fg(Color::Yellow),
            ));
        }
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
        // Esc never moves when arming adds entries after it (review
        // N-C1-2) — and never sits under the panes' first entry, `x cancel`
        // while a command runs, which opens this very kind of dialog (N2-6).
        Mode::Dialog(c) => {
            let mut h = vec![("↑↓", "scroll"), ("Esc", "cancel")];
            if c.dialog.armed {
                h.push(("←→", "button"));
                h.push(("Enter", "press"));
            }
            h
        }
        Mode::Note { .. } | Mode::EditNote { .. } => {
            vec![("Enter", "continue"), ("Esc", "cancel")]
        }
        // Opened from the chat, the details keep the chat's keys: no
        // letters (review USE-1; fix check N2).
        Mode::Details { .. } if app.focus == Focus::Chat => {
            let mut h = vec![("↑↓", "scroll"), ("Esc", "close")];
            if app.running {
                h.push(("Ctrl-X", "cancel"));
            }
            h
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
        Focus::Chat => return chat_pane::chat_hints(app),
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
        // In the chat `?` and `q` are text: its own hints end in F1 and
        // Ctrl-C (docs/CHAT-PANE-DESIGN.md §5.4).
        None if app.focus == Focus::Chat => (hints(app), chat_pane::chat_hint_tail(app)),
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
     a second after it opened — Run and the others that act only once it is ready.",
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

const HELP_FEATURES: &[&str] = &[
    "A feature is something a person does with the program and sees the result of — \
     \"compress a file to zlib\", \"show the help\". Each is one or more scenarios: one run of \
     the whole program with fixed arguments and, at most, one of three samples as its input \
     (about 30 000 bytes of English text, 16 KiB of pseudo-random bytes, or an empty file). An \
     argument is a flag or a word — never a path: the program runs in an empty folder of its \
     own. Programs that fork are checked only up to the fork.",
    "Every Re-check (and every judged turn of a migration) runs each scenario on the C program \
     and on the program with that unit's Rust swapped in, and compares the exit status, stdout \
     and stderr. Each unit is checked with only its own Rust swapped in. A scenario the C \
     cannot run the same way twice is skipped and named — it never blocks a Re-check, and \
     neither does a features file with an error.",
    "Map the features runs each scenario on a scratch copy of the C in which every function \
     notes that it ran: the map says which units each feature runs (\"runs\"), from the \
     functions it could put a note in (\"watches\"). Changing a scenario makes verdicts made \
     before say \"not checked since you changed them\" until re-checked; renaming a feature does \
     not.",
    "The file, migration/features/features.toml: schema_version = 1, then a [[feature]] table \
     per feature (id = \"zlib\", name = \"Compress to zlib\") and a [[scenario]] table per \
     run (feature = \"zlib\", id = \"text\", args = [\"--zlib\", \"-c\", \"{input}\"], \
     input = \"sample:text\"). Ids are lowercase letters, digits and dashes; \"{input}\" \
     stands for the sample's file name.",
    "Enter on Features: Write / Edit the features file (in your editor — the cockpit says how \
     to save and leave: in nano, Ctrl-O then Enter saves and Ctrl-X leaves; in vi, press i to \
     type, then Esc and :wq and Enter to save and leave), Map the features. Edited it outside \
     the cockpit? Press g. Commit migration/features/ with your work.",
];

const HELP_FEATURE_LEGEND: &[(&str, &str)] = &[
    ("✗", "failing: a unit's check of it failed"),
    (
        "⚑",
        "a scenario cannot run as written — change or remove it",
    ),
    (
        "↻",
        "needs a re-check: a unit with Rust was not checked on it",
    ),
    ("⋯", "not mapped yet"),
    ("≃", "the map is out of date"),
    ("◔", "the map is incomplete for it"),
    ("∅", "reaches no unit"),
    ("✓", "all its units migrated, each checked alone"),
    ("◉", "holds so far: its migrated units pass"),
    ("◌", "all its code is still C"),
    ("·", "see its units: they differ"),
];

const HELP_SPEED: &[&str] = &[
    "Speed compares the C against the Rust in use. A workload is one run of the whole program \
     with arguments you choose and, at most, one input file of yours (inside the project). perf \
     runs it as the C alone, then each verified unit's Rust swapped in alone, then the program \
     as it stands (every verified unit together), the C and the Rust taking turns, many times \
     each, and says which is faster and by how much — or that it cannot tell. It never changes \
     a verdict.",
    "perf compares what the program prints and how it ends: when the Rust prints or ends \
     differently on a workload, that row says \"behaves differently\" and keeps both outputs; \
     Compare the outputs shows them around their first difference. Verify does not run your \
     workloads, so only perf finds this. perf stops even a fork; verify allows a fork but not \
     starting another program.",
    "The file, migration/perf/workloads.toml: schema_version = 1, then a [[workload]] table per \
     run (id = \"big-text\", args = [\"-c\", \"{input}\"], input = \"bench/big.txt\", \
     runs = 15). Enter on Speed: Write / Edit the workloads file, Measure speed; on a verified \
     unit, Measure this unit's speed. Keep the computer quiet while it measures. Commit \
     migration/perf/ to keep a history: measuring again replaces a row.",
    "When a unit's Rust is slower and speed matters, change the Rust the way it was made: \
     made by a model — Modify its attempt with a note about speed, Replace the verified crate \
     with the new attempt, measure again, and Replace it back if it is not faster; a hand \
     edit — Hand edit its crate, Replace the verified crate with the new attempt, measure \
     again, and Replace it back if it is not faster (a hand edit alone is recorded, never \
     accepted: perf would time the same crate); written outside the cockpit — commit the crate \
     first (git), edit it in your editor, run harness verify <unit> in a terminal, then \
     measure again.",
];

const HELP_SPEED_WORDS: &[(&str, &str)] = &[
    ("about as fast", "within 2 % of the C, either way"),
    (
        "slower 6.2 % (4.1–8.3 %)",
        "the best guess, and the range it lies in (perf is at least 95 % sure of it)",
    ),
    (
        "probably slower",
        "slower, but not clearly past the 2 % line",
    ),
    ("close call", "too close to the 2 % line to call"),
    (
        "can't tell: ±3.4 %",
        "the runs varied too much — measure again with 31 runs, on a quiet computer",
    ),
    (
        "can't tell: slow cores",
        "many runs ran mostly on the slower cores — the computer may have been busy (close \
         other work and measure again), or the program runs there by design",
    ),
    (
        "can't tell: too few",
        "too few runs gave a value — measure again",
    ),
    (
        "short run: can't tell",
        "the run is too short to tell a difference this small — use a bigger input; more runs \
         will not settle it",
    ),
    (
        "no clear diff ±1.2 %",
        "31 runs found no difference bigger than that",
    ),
    (
        "too short to time",
        "the run is too quick to time — use a bigger input",
    ),
    (
        "behaves differently",
        "the Rust prints or ends differently from the C on this workload",
    ),
    (
        "out of date",
        "the C, the Rust or the workload changed since — measure again",
    ),
    (
        "parallel",
        "it uses several cores: the words compare total CPU work",
    ),
];

const HELP_CHAT: &[&str] = &[
    "The chat (Tab, or click Chat): ask for model work in plain words — \"migrate this\". It \
     reads the project and ASKS: a request waits on a line above the input, still for a \
     second; Enter (on an empty input) reviews it in the same armed dialog as every act, Esc \
     declines it. Scan, the plan, Re-check and Accept stay yours: the chat says which to use.",
    "Letters in the chat are text. Enter sends; Ctrl-J, \\ then Enter, or Alt-Enter is a new \
     line. Esc: decline a request, hold a waiting Continue, or stop the reply. Ctrl-C: stop, \
     clear the draft, or quit (asked). Ctrl-X: cancel the running command. Ctrl-N: a new \
     chat (asked). ↑↓ PgUp PgDn scroll; Home/End on an empty input: the top / the newest \
     line. F1: this screen.",
    "When you run a migration the chat asked for, it answers the model's turns here and each \
     answer continues the run without asking again — until you hold one (Esc), decline or \
     cancel one, stop the chat, or start a new one. Nothing is accepted without you.",
    "After you leave the chat with a draft, letters in the panes are dropped until you press \
     an arrow, Tab or Esc, or click — they would be commands there.",
];

const HELP_ROUTES: &[&str] = &[
    "Other routes for model work:",
    "  harness migrate <unit>: a fresh translation (the blind hand-off, or a live provider)",
    "  harness-mcp in a separate Claude Code session: steer attempts (README)",
    "  harness override <unit> <dir>: record an outside edit as a hand edit",
    "Every act shows its exact command and waits until it is ready: keys typed or pasted ahead never answer it, and a held Enter never runs anything.",
];

/// Help's rows, and which of them are the mouse on/off line (a click there
/// is `m`).
/// `chat`: the chat pane exists — `Some(why)` when it cannot be used.
fn help_rows(
    width: usize,
    mouse: bool,
    chat: Option<Option<&str>>,
) -> (Vec<Line<'static>>, std::ops::Range<usize>) {
    let mut rows = Vec::new();
    for l in HELP_INTRO {
        rows.extend(wrapped(l, width, bold()));
    }
    if chat.is_some() {
        rows.extend(wrapped(
            "Tab moves Files → View → Chat; in the chat, ask for model work in plain words.",
            width,
            bold(),
        ));
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
    rows.push(Line::from(Span::styled("Features", bold())));
    for l in HELP_FEATURES {
        rows.extend(wrapped(l, width, Style::default()));
    }
    for (g, v) in HELP_FEATURE_LEGEND {
        rows.extend(wrapped(&format!("{g:<3} {v}"), width, Style::default()));
    }
    // Why a scenario cannot run, and what to do (§6.1's table).
    rows.extend(wrapped(
        "When a scenario cannot run, its line says why:",
        width,
        Style::default(),
    ));
    for reason in harness_core::features::SkipReason::ALL {
        rows.extend(wrapped(
            &format!("  {} — {}", reason.words(), reason.what_to_do()),
            width,
            dim(),
        ));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled("Speed", bold())));
    for l in HELP_SPEED {
        rows.extend(wrapped(l, width, Style::default()));
    }
    for (w, v) in HELP_SPEED_WORDS {
        rows.extend(wrapped(&format!("  {w} — {v}"), width, dim()));
    }
    rows.push(Line::from(""));
    if let Some(why) = chat {
        rows.push(Line::from(Span::styled("Chat", bold())));
        // Unavailable: why, in words (§1.1).
        if let Some(why) = why {
            rows.extend(wrapped(
                &format!("The chat is unavailable here: {why}."),
                width,
                Style::default().fg(Color::Yellow),
            ));
        }
        for l in HELP_CHAT {
            rows.extend(wrapped(l, width, Style::default()));
        }
        rows.push(Line::from(""));
    }
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
    // (review USE-B-7).
    // Never moves them: a footer with no room below is cut (review N-C2-5).
    let base = centered(area, width, items_h);
    let bottom = area.y + area.height;
    let rect = Rect {
        height: (rows.len() as u16 + 2).min(bottom.saturating_sub(base.y)),
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
    let mouse = app.mouse;
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
        if let Some(text) = p.chat.as_ref().and_then(|t| t.answer.as_deref()) {
            rows.extend(wrapped(
                &format!(
                    "The chat's answer ({} bytes), as the harness will file it — its lines \
                     numbered; a row without a number goes on with the line above:",
                    text.len()
                ),
                inner_w,
                Style::default(),
            ));
            rows.extend(answer_rows(text, inner_w).into_iter().map(Line::from));
            rows.push(Line::from("(end of the answer)"));
        }
        rows.push(Line::from(""));
        // Filtered but NEVER cut: the whole command must be seen (it scrolls).
        let argv = Sanitizer::default().push(&format!("Command: {}", shell_line(&p.argv)));
        for row in hard_wrap(&argv, inner_w) {
            rows.push(Line::from(Span::styled(row, bold())));
        }
    }
    // Two rows pinned at the bottom: the dialog's state, its buttons.
    let height = dialog_height(rows.len(), area.height);
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
        // Under the chat-dialog rules no letter presses a button: none is
        // shown on one (docs/CHAT-PANE-DESIGN.md §3.2).
        let text = if c.dialog.chat_rules && i > 0 {
            format!("[ {} ]", b.label)
        } else {
            format!("[ {}  {} ]", b.label, b.key)
        };
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
    let state = if !usable {
        "too small to show".into()
    } else if mouse
        && c.dialog.armed
        && !c.dialog.click_refused
        && c.dialog.opened.elapsed() >= crate::dialog::CLICK_SETTLE
    {
        // Invited only once a click would answer (review N2-8).
        format!("{} · or click", c.dialog.state_text())
    } else {
        c.dialog.state_text()
    };
    let state_style = if c.dialog.armed && !c.dialog.click_refused {
        Style::default().fg(Color::Green)
    } else if c.dialog.too_soon || c.dialog.click_refused {
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
            let (rows, toggle) = help_rows(
                rect.width.saturating_sub(2) as usize,
                app.mouse,
                app.chat_on
                    .then(|| app.chat.bins.as_ref().err().map(String::as_str)),
            );
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
                if let Some(note) = feature_check_note(app, &v.unit, &c.name, c.passed) {
                    rows.extend(wrapped(&format!("    {note}"), inner, dim()));
                }
            }
            if !v.inputs.features_skipped.is_empty() {
                rows.push(Line::from(""));
                rows.extend(wrapped("Your scenarios that could not run:", inner, bold()));
                for entry in &v.inputs.features_skipped {
                    if let Some((f, sc, reason)) = harness_core::features::parse_skip(entry) {
                        rows.extend(wrapped(
                            &format!("  {f}/{sc}: {} — {}", reason.words(), reason.what_to_do()),
                            inner,
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                }
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
    // The chat (docs/CHAT-PANE-DESIGN.md §5.1): a column of its own from
    // 156 columns once opened; below that the right column shows it only
    // while it has the focus, and a tab strip keeps it one click away.
    let chat = app.chat_on;
    if single {
        match app.focus {
            // The strip on whichever pane shows: the chat is one click away
            // at every width (review USE-11).
            Focus::Files => {
                draw_files(frame, app, main);
                if chat {
                    chat_pane::tab_strip(frame, app, main, Focus::Files);
                }
            }
            Focus::View => {
                draw_view(frame, app, main);
                if chat {
                    chat_pane::tab_strip(frame, app, main, Focus::View);
                }
            }
            Focus::Chat => chat_pane::draw_chat(frame, app, main, false, true),
        }
    } else {
        let files_w = if area.width >= WIDE_FROM {
            FILES_WIDE
        } else {
            FILES_NARROW
        };
        let column = chat && app.chat_column && area.width >= CHAT_COLUMN_FROM;
        if column {
            let chat_w = CHAT_COLUMN_MAX.min(area.width - files_w - VIEW_KEEPS);
            let [files, view, chat_area] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(files_w),
                    Constraint::Min(VIEW_KEEPS),
                    Constraint::Length(chat_w),
                ])
                .areas(main);
            draw_files(frame, app, files);
            draw_view(frame, app, view);
            chat_pane::draw_chat(frame, app, chat_area, true, false);
        } else {
            let [files, right] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(files_w), Constraint::Min(20)])
                .areas(main);
            draw_files(frame, app, files);
            if chat && app.focus == Focus::Chat {
                chat_pane::draw_chat(frame, app, right, false, true);
            } else {
                draw_view(frame, app, right);
                if chat {
                    chat_pane::tab_strip(frame, app, right, Focus::View);
                }
            }
        }
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

    /// Review SAF-2, fix check N1: an answer's line is shown whole — never
    /// cut, its indentation kept — and a row that goes on with a line is
    /// the cockpit's to mark: an answer's own `↩`, `│` or `┆` never passes a
    /// line of its own off as the rest of the line above.
    #[test]
    fn an_answer_is_drawn_whole_behind_the_cockpits_gutter() {
        let long = format!("pub fn f() {{}}{}HIDDEN();", " ".repeat(5000));
        let forged = "    // see below ↩\n    std::process::abort();\n  ┆ x();\n\tdone│";
        let rows = answer_rows(&format!("{long}\n{forged}"), 60);
        assert!(rows.iter().all(|r| width_of(r) <= 60), "{rows:?}");
        // The long line: one numbered row, then rows without a number that
        // hold every byte of it.
        assert!(rows[0].starts_with("1│ pub fn f()"), "{}", rows[0]);
        let first_of = |n: &str| rows.iter().position(|r| r.starts_with(n)).unwrap();
        let two = first_of("2│");
        let joined: String = rows[..two]
            .iter()
            .map(|r| {
                r.split_once(['│', '┆'])
                    .unwrap()
                    .1
                    .strip_prefix(' ')
                    .unwrap()
            })
            .collect();
        assert_eq!(joined, long);
        assert!(rows[1..two].iter().all(|r| r.starts_with(" ┆ ")));
        // Each line of the forged part starts a numbered row, indentation
        // kept — its own marks are only its text.
        assert_eq!(rows[two], "2│     // see below ↩");
        assert_eq!(rows[two + 1], "3│     std::process::abort();");
        assert_eq!(rows[two + 2], "4│   ┆ x();");
        assert_eq!(rows[two + 3], "5│         done│");
        assert_eq!(rows.len(), two + 4);
        // CRLF: the `\r` shown as ␍ (fix check 2, finding 7).
        assert_eq!(answer_rows("a\r\nb", 60), ["1│ a␍", "2│ b"]);
    }

    /// Fix check 2, finding 7: a dialog of more rows than a u16 counts is
    /// as high as the screen allows — never a wrapped count.
    #[test]
    fn a_dialog_of_many_rows_fills_the_screen() {
        for rows in [65_532, 65_535, 70_000, 600_000] {
            assert_eq!(dialog_height(rows, 30), 28, "{rows}");
        }
        assert_eq!(dialog_height(3, 30), 7);
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
        app.rows = crate::tree::rows(
            &app.snapshot,
            &app.files,
            &app.features,
            &app.walk,
            &app.expansion,
        );
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
            collect: Default::default(),
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
        // Esc where arming never moves it (review N-C1-2), not first — under
        // the panes' `x cancel` (N2-6).
        assert_eq!(bar(&mut app).trim(), "↑↓ scroll   Esc cancel");
        arm(&mut app);
        assert_eq!(
            bar(&mut app).trim(),
            "↑↓ scroll   Esc cancel   ←→ button   Enter press"
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
        // A paste between the two presses.
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(11000));
        app.on_event(TermEvent::Paste("p".into()), t + ms(11050), Duration::ZERO);
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(11100));
        assert!(normal(&app), "a paste between");
    }

    /// The real drive (E2E-1): a click, then a quick double click on the
    /// same row — a triple click — opened the menu and closed it again. A
    /// quick press right after the double click (which opened the menu) is
    /// swallowed: a triple click, even a quadruple one, is a double click.
    /// The window runs from the double click: a swallowed press never
    /// extends it (review N-C1-3).
    #[test]
    fn a_triple_click_is_a_double_click() {
        let mut app = app("mtriple");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        for i in 0..4 {
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
        // The window of the double click (at 150 ms) is over: a click outside
        // the menu closes it.
        click(&mut app, unit, t + ms(150) + DOUBLE_CLICK + ms(1));
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
        // Two refused clicks close together are no double click that could
        // swallow the next, allowed one (review PROC-B-2).
        for late in [450, 500, 700, 950, 1000] {
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
        assert!(!c
            .dialog
            .press_button(1, Instant::now() + CLICK_SETTLE, Duration::ZERO));
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
        q.polled(t, true, false, t + ms(60));
        assert_eq!(q.late(t + ms(70)), ms(10));
        // Input already queued (the zero poll found it): it may have waited
        // since the last empty poll — a stall included — however long the
        // poll that returned it took (review N2-3: a slow poll proves
        // nothing).
        q.polled(t + ms(500), false, true, t + ms(502));
        assert_eq!(q.late(t + ms(502)), ms(442));
        // Proved empty, then returned by the wait: it came during the wait,
        // and is charged all of it.
        q.polled(t + ms(600), true, true, t + ms(630));
        assert_eq!(q.late(t + ms(631)), ms(31));
    }

    // ----- the check of the fix pass (§R7) -----------------------------------

    /// Mutation-checked rule, checks N-C1-1 / N-C2-1 / N-C3-1 (found by all
    /// three): a press held while a key opens something is dropped — its
    /// release never answers a dialog it was not pressed in, nor a screen
    /// it was not pressed on.
    #[test]
    fn a_held_press_never_answers_what_a_key_opened() {
        // (a) [Cancel x] held in the panes; `q` opens the quit dialog; it
        // arms; the release.
        let mut app = app("mheld");
        running(&mut app);
        let t = Instant::now();
        render(&mut app, 120, 30);
        let cancel = spot(&app, &Hit::Activity("x"));
        event(&mut app, LEFT, cancel, t);
        app.now = t + ms(10);
        app.on_event(
            TermEvent::Key(KeyEvent::from(KeyCode::Char('q'))),
            t + ms(10),
            Duration::ZERO,
        );
        let when = ready(&mut app, 120, 30, t + ms(10));
        assert_eq!(event(&mut app, UP, cancel, when), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Quit));
        // (b) An accepted press on "Quit, let it finish"; Esc, x: the cancel
        // dialog, unarmed, where the button was; the release.
        let quit = spot(&app, &Hit::Button(1));
        assert_eq!(event(&mut app, LEFT, quit, when + ms(100)), Command::None);
        crate::app::tests::code(&mut app, KeyCode::Esc);
        key(&mut app, 'x');
        render(&mut app, 120, 30);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Cancel));
        assert_eq!(event(&mut app, UP, quit, when + ms(200)), Command::None);
        assert!(matches!(app.mode, Mode::Dialog(_)));
        // (c) A ready Scan dialog: Run pressed; Esc; the same dialog again,
        // at the same place; the release.
        let mut app = crate::app::tests::app("mheld2");
        let t = Instant::now();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.now = t;
        app.ask(scan.clone());
        let when = ready(&mut app, 120, 30, t);
        let run = spot(&app, &Hit::Button(1));
        event(&mut app, LEFT, run, when);
        crate::app::tests::code(&mut app, KeyCode::Esc);
        app.now = when + ms(50);
        app.ask(scan);
        render(&mut app, 120, 30);
        assert_eq!(event(&mut app, UP, run, when + ms(100)), Command::None);
        // A paste drops it too.
        let when = ready(&mut app, 120, 30, when + ms(50));
        event(&mut app, LEFT, run, when);
        app.on_event(TermEvent::Paste("x".into()), when, Duration::ZERO);
        assert_eq!(event(&mut app, UP, run, when + ms(50)), Command::None);
    }

    /// Check N-C2-2: every window allows for how long a press may have
    /// waited in the queue — a press read late may have come within the
    /// settle time (refused) or right after the gesture (swallowed).
    #[test]
    fn a_press_read_late_is_judged_by_when_it_may_have_come() {
        let mut app = app("mlate");
        let t = Instant::now();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.now = t;
        app.ask(scan);
        let _ = ready(&mut app, 120, 30, t);
        let run = spot(&app, &Hit::Button(1));
        let read = t + CLICK_SETTLE + ms(100);
        app.on_event(TermEvent::Mouse(mouse(LEFT, run)), read, ms(610));
        assert_eq!(
            app.on_event(TermEvent::Mouse(mouse(UP, run)), read + ms(5), ms(610)),
            Command::None
        );
        // A triple click whose third press was read late: still swallowed.
        let mut app = crate::app::tests::app("mlate2");
        render(&mut app, 120, 30);
        let unit = spot(&app, &Hit::Row(Selection::Unit("u-lib".into())));
        let t = Instant::now();
        click(&mut app, unit, t);
        render(&mut app, 120, 30);
        click(&mut app, unit, t + ms(150));
        assert!(matches!(app.mode, Mode::Menu(_)));
        render(&mut app, 120, 30);
        app.on_event(TermEvent::Mouse(mouse(LEFT, unit)), t + ms(700), ms(420));
        app.on_event(TermEvent::Mouse(mouse(UP, unit)), t + ms(705), ms(420));
        assert!(
            matches!(app.mode, Mode::Menu(_)),
            "swallowed, the menu open"
        );
    }

    /// Checks N-C1-3 / N-C3-2: the swallow window runs from the gesture —
    /// steady clicks on the dialog it opened are not all eaten; and keys
    /// that open nothing count every click.
    #[test]
    fn a_swallow_never_eats_steady_clicks() {
        let mut app = app("mchain");
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
        render(&mut app, 120, 30);
        app.arm(t + ms(30), false);
        app.arm(t + ms(340), false);
        render(&mut app, 120, 30);
        let cancel = spot(&app, &Hit::Button(0));
        // 370: swallowed; 720: refused, it says so; 1070: settled — closed.
        click(&mut app, cancel, t + ms(370));
        assert!(matches!(&app.mode, Mode::Dialog(c) if !c.dialog.click_refused));
        click(&mut app, cancel, t + ms(720));
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.click_refused));
        render(&mut app, 120, 30);
        assert!(text(&render(&mut app, 120, 30)).contains("Too soon — click again"));
        click(&mut app, cancel, t + ms(1070));
        assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
        // Tab, clicked twice quickly: two Tabs.
        render(&mut app, 120, 30);
        let tab = spot(&app, &Hit::Hint("Tab"));
        let focus = app.focus;
        click(&mut app, tab, t + ms(3000));
        render(&mut app, 120, 30);
        click(&mut app, tab, t + ms(3150));
        assert_eq!(app.focus, focus, "two Tabs");
    }

    /// Check N-C3-3: the tree follows its selection again after a key in
    /// the panes that leaves the focus in it (Tab into it), never after a
    /// key in or out of an overlay; the same for a list's chosen link.
    #[test]
    fn follow_comes_back_only_with_a_key_in_the_panes() {
        let mut app = app_of("targets/zopfli", "mfollow");
        let t = Instant::now();
        render(&mut app, 120, 16);
        let files = spot(&app, &Hit::Pane(Focus::Files));
        for _ in 0..3 {
            event(&mut app, MouseEventKind::ScrollDown, files, t);
        }
        render(&mut app, 120, 16);
        let wheeled = app.tree_offset;
        assert!(wheeled > 0);
        // Help, opened and closed with the focus in the tree: still wheeled.
        key(&mut app, '?');
        crate::app::tests::code(&mut app, KeyCode::Esc);
        render(&mut app, 120, 16);
        assert_eq!(app.tree_offset, wheeled);
        // Tab to the View and back into the tree: it shows the selection.
        crate::app::tests::code(&mut app, KeyCode::Tab);
        render(&mut app, 120, 16);
        assert_eq!(app.tree_offset, wheeled, "a key in the View");
        crate::app::tests::code(&mut app, KeyCode::Tab);
        render(&mut app, 120, 16);
        assert_eq!(app.tree_offset, 0, "Tab into the tree follows");
        // A list: wheeled; Help in and out; still where wheeled; a key in the
        // View follows its link again.
        app.select(Selection::Dir("src/zopfli".into()));
        render(&mut app, 120, 16);
        click_on(&mut app, &Hit::Link(0), t + ms(1000));
        render(&mut app, 120, 16);
        let view = spot(&app, &Hit::Pane(Focus::View));
        event(&mut app, MouseEventKind::ScrollDown, view, t + ms(2000));
        event(&mut app, MouseEventKind::ScrollDown, view, t + ms(2000));
        key(&mut app, '?');
        crate::app::tests::code(&mut app, KeyCode::Esc);
        render(&mut app, 120, 16);
        assert_eq!(app.scroll, 6);
        crate::app::tests::code(&mut app, KeyCode::Home);
        render(&mut app, 120, 16);
        // The first link is the list's second row (under the roll-up).
        assert_eq!(app.scroll, 1, "a key in the View follows the link");
    }

    /// Checks N-C1-5 / N-C2-4 / N-C3-4, N-C2-3 / N-C3-6: a double click on
    /// ▸ opens it once; a click that slipped two cells along a key presses
    /// it and says nothing about selecting text.
    #[test]
    fn a_double_click_on_a_marker_and_a_slipped_click() {
        let mut app = app("mmarker");
        let t = Instant::now();
        render(&mut app, 120, 30);
        let unit = Selection::Unit("u-lib".into());
        let fold = spot(&app, &Hit::Fold(unit.clone()));
        click(&mut app, fold, t);
        render(&mut app, 120, 30);
        click(&mut app, fold, t + ms(150));
        assert!(app.expansion.is_open(&unit), "opened once");
        // A slipped click on Tab, read now (a key clears older notices).
        let mut app = crate::app::tests::app("mslip");
        render(&mut app, 120, 30);
        let (r, _) = app
            .hits
            .iter()
            .find(|(_, h)| *h == Hit::Hint("Tab"))
            .cloned()
            .unwrap();
        let focus = app.focus;
        let now = Instant::now();
        event(&mut app, LEFT, (r.x + 1, r.y), now);
        event(&mut app, UP, (r.x + r.width - 1, r.y), now);
        assert_ne!(app.focus, focus, "pressed");
        assert!(app.notice.as_ref().is_none_or(|n| n.text != DRAG_HINT));
    }

    /// Check N-C3-5: the mouse is in use while a press is down and for a
    /// moment after any mouse event — the loop reads its last reports away
    /// then, and only then (a key-driven quit or edit drains nothing).
    #[test]
    fn the_mouse_is_busy_only_around_its_events() {
        let mut app = app("mbusy");
        let t = Instant::now();
        render(&mut app, 120, 30);
        assert!(!app.mouse_busy(t));
        event(&mut app, LEFT, (40, 5), t);
        assert!(app.mouse_busy(t + ms(5000)), "a press is down");
        event(&mut app, UP, (40, 5), t + ms(10));
        assert!(app.mouse_busy(t + ms(400)));
        assert!(!app.mouse_busy(t + ms(600)));
        // After the loop stopped reading with the mouse on (a stopped
        // command's wait), reports may still come (review N2-1).
        app.mouse_may_report(t + ms(5000));
        assert!(app.mouse_busy(t + ms(5400)));
        app.mouse = false;
        assert!(!app.mouse_busy(t + ms(20)));
    }

    /// Check N-C2-5: a menu's footer never moves its items, even on a frame
    /// with no room for it below (it is cut).
    #[test]
    fn a_menu_footer_is_cut_before_its_items_move() {
        let mut app = app("mfooter2");
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        for h in [10, 11, 14] {
            app.mode = Mode::Normal;
            app.open_menu();
            render(&mut app, 50, h);
            let items = |app: &App| -> Vec<(Rect, Hit)> {
                app.hits
                    .iter()
                    .filter(|(_, h)| matches!(h, Hit::MenuItem(_)))
                    .cloned()
                    .collect()
            };
            let before = items(&app);
            let Mode::Menu(m) = &mut app.mode else {
                panic!()
            };
            m.focus = m.items.iter().position(|i| i.greyed.is_some()).unwrap();
            crate::app::tests::code(&mut app, KeyCode::Enter);
            render(&mut app, 50, h);
            assert_eq!(items(&app), before, "at 50x{h}");
        }
    }

    /// Checks' test gaps: a menu item acts on its release (a press alone, or
    /// dragged off, chooses nothing); the activity row is outside the checks
    /// and the diff; `]f`, `g`, `t` and the wheel over the panes above the
    /// details do what they say; a click in a dialog on `↑↓` says what
    /// moves there.
    #[test]
    fn the_rest_of_the_gestures() {
        let mut app = app("mrest");
        let t = Instant::now();
        app.select(Selection::Unit("u-lib".into()));
        app.open_menu();
        render(&mut app, 120, 30);
        let item = spot(&app, &Hit::MenuItem(0));
        event(&mut app, LEFT, item, t);
        assert!(matches!(app.mode, Mode::Menu(_)), "a press alone");
        event(&mut app, UP, (item.0, item.1 + 3), t + ms(50));
        assert!(matches!(app.mode, Mode::Menu(_)), "dragged off");
        // The activity row under the checks: outside them.
        app.mode = Mode::Verdict {
            selected: 0,
            scroll: 0,
        };
        running(&mut app);
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Activity("c"), t + ms(1000));
        assert!(matches!(app.mode, Mode::Normal), "closed, no details");
        // `]f` goes to the next pair; `g` re-reads; `[Try again t]` offers
        // the command again.
        app.running = false;
        app.focus = Focus::View;
        render(&mut app, 120, 30);
        app.layout.pair_rows = vec![0, 7];
        click_on(&mut app, &Hit::Hint("]f"), t + ms(2000));
        assert_eq!(app.scroll, 7);
        render(&mut app, 160, 30);
        assert_eq!(
            click_on(&mut app, &Hit::Hint("g"), t + ms(3000)),
            Command::Reload
        );
        app.try_again = app.run.as_ref().map(|r| r.pending.clone());
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Activity("t"), t + ms(4000));
        assert!(matches!(&app.mode, Mode::Dialog(c) if matches!(c.purpose, Purpose::Act(_))));
        // In the dialog, `↑↓`: the wheel or the keys, no rows.
        render(&mut app, 120, 30);
        click_on(&mut app, &Hit::Hint("↑↓"), t + ms(5000));
        assert!(app
            .notice
            .as_ref()
            .is_some_and(|n| n.text == "turn the wheel to scroll; ↑↓ are keys"));
        // The wheel over the panes above the details scrolls the pane.
        let mut app = app_of("targets/zopfli", "mrest2");
        key(&mut app, 'c');
        render(&mut app, 120, 30);
        let details = match app.mode {
            Mode::Details { scroll } => scroll,
            _ => panic!(),
        };
        // A tree row above the details.
        let files = (5, 2);
        event(&mut app, MouseEventKind::ScrollDown, files, t);
        assert_eq!(app.tree_offset, 3, "the tree scrolled");
        assert!(matches!(app.mode, Mode::Details { scroll } if scroll == details));
    }

    // ----- the check of the second fix pass (§R8) ----------------------------

    /// Check N2-9 / p8: the key alone drops a held press — a click on the
    /// dialog's `Enter press` held while `→` moves the focus to Run never
    /// runs it (the same screen and dialog: only the key guards this).
    /// And N2-4: that hint presses the focused button as a click on it
    /// would — never within the settle time.
    #[test]
    fn the_dialogs_enter_hint_is_a_click_on_the_focused_button() {
        let mut app = app("menterhint");
        let t = Instant::now();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.now = t;
        app.ask(scan.clone());
        let when = ready(&mut app, 120, 30, t);
        let enter = spot(&app, &Hit::Hint("Enter"));
        event(&mut app, LEFT, enter, when);
        crate::app::tests::code(&mut app, KeyCode::Right);
        render(&mut app, 120, 30);
        assert_eq!(event(&mut app, UP, enter, when + ms(50)), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.focus == 1));
        // Focus on Run by a key, then a click on `Enter press` within the
        // settle time of a fresh dialog: refused.
        let mut app = crate::app::tests::app("menterhint2");
        let t = Instant::now();
        app.now = t;
        app.ask(scan.clone());
        render(&mut app, 120, 30);
        app.arm(t + ms(10), false);
        app.arm(t + ms(320), false);
        render(&mut app, 120, 30);
        app.now = t + ms(350);
        app.on_key(KeyEvent::from(KeyCode::Right), t + ms(350));
        render(&mut app, 120, 30);
        let enter = spot(&app, &Hit::Hint("Enter"));
        assert_eq!(click(&mut app, enter, t + ms(420)), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.dialog.click_refused));
        assert_eq!(
            click(&mut app, enter, t + CLICK_SETTLE + ms(100)),
            Command::Spawn(scan)
        );
    }

    /// Check N2-7: keys the details pass to the panes — Tab back into the
    /// tree, which moves no selection — bring a wheeled tree back to it.
    #[test]
    fn a_key_through_the_details_follows_the_tree() {
        let mut app = app_of("targets/zopfli", "mfollow2");
        let t = Instant::now();
        app.focus = Focus::View;
        key(&mut app, 'c');
        render(&mut app, 120, 30);
        for _ in 0..4 {
            event(&mut app, MouseEventKind::ScrollDown, (5, 2), t);
        }
        render(&mut app, 120, 30);
        assert!(app.tree_offset > 0);
        crate::app::tests::code(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Files);
        render(&mut app, 120, 30);
        let cursor = app.cursor().unwrap();
        assert!(
            cursor >= app.tree_offset && cursor < app.tree_offset + app.layout.tree_page,
            "the selection is in view"
        );
        assert!(matches!(app.mode, Mode::Details { .. }));
    }

    /// Checks N2-8b, N2-9: "· or click" only with the mouse on and once a
    /// click would answer; "Too soon — click again" in the waiting colour.
    #[test]
    fn the_ready_line_invites_a_click_only_when_one_answers() {
        let mut app = app("mready");
        let t = Instant::now();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.ask(scan);
        // The dialog's state line, and the colour of its first letter.
        let state = |app: &mut App| -> (String, Color) {
            let buffer = render(app, 120, 30);
            for y in 0..30u16 {
                let row: Vec<String> = (0..120u16)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                let line = row.concat();
                for word in ["ready:", "Too soon —"] {
                    if let Some(byte) = line.find(word) {
                        let x = line[..byte].chars().count() as u16;
                        return (line.trim().to_string(), buffer[(x, y)].fg);
                    }
                }
            }
            panic!("no state line")
        };
        render(&mut app, 120, 30);
        app.arm(t, false);
        app.arm(t + ms(400), false);
        let (line, fg) = state(&mut app);
        assert!(
            line.contains("ready") && !line.contains("or click"),
            "{line}"
        );
        assert_eq!(fg, Color::Green);
        if let Mode::Dialog(c) = &mut app.mode {
            c.dialog.opened = Instant::now().checked_sub(CLICK_SETTLE * 2).unwrap();
        }
        let (line, _) = state(&mut app);
        assert!(line.contains("· or click"), "{line}");
        app.mouse = false;
        let (line, _) = state(&mut app);
        assert!(!line.contains("or click"), "the mouse off: {line}");
        app.mouse = true;
        if let Mode::Dialog(c) = &mut app.mode {
            c.dialog.click_refused = true;
        }
        let (line, fg) = state(&mut app);
        assert!(line.contains("Too soon — click again"), "{line}");
        assert_eq!(fg, Color::Yellow);
        // Under the chat's dialog rules a click is invited too (review
        // USE-8).
        if let Mode::Dialog(c) = &mut app.mode {
            c.dialog.click_refused = false;
            c.dialog.chat_rules = true;
        }
        let (line, _) = state(&mut app);
        assert!(line.contains("· or click"), "chat rules: {line}");
    }

    // ----- the person's features (docs/FEATURES-DESIGN.md §8) -----------------

    const FEATURES_TOML: &str = "schema_version = 1\n\
[[feature]]\nid = \"gzip\"\nname = \"Compress to gzip\"\n\
[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\
[[scenario]]\nfeature = \"gzip\"\nid = \"text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"sample:text\"\n\
[[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n";

    /// zopfli with `text` as its features, and u001's verdicts from before
    /// it had any (their features inputs dropped).
    fn zopfli_with_features(tag: &str, text: &str) -> App {
        let app = crate::app::tests::app_of_without_features("targets/zopfli", tag);
        let unit = app.config.target.join("migration/units/u001-katajainen");
        for name in ["oracle-latest.json", "oracle-last-green.json"] {
            let path = unit.join(name);
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let inputs = v["inputs"].as_object_mut().unwrap();
            for key in ["features", "program", "features_skipped"] {
                inputs.remove(key);
            }
            std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        }
        let dir = app.config.target.join("migration/features");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("features.toml"), text).unwrap();
        crate::app::tests::app_of_path(&app.config.target)
    }

    /// A current map in which gzip runs katajainen and main, help runs main.
    fn write_current_map(app: &App) {
        let read = crate::load::read(&app.config.target).unwrap();
        let now = read.map_now.expect("today's inputs");
        let record =
            |f: &str, s: &str, funcs: Vec<(&str, &str)>| harness_core::features::ScenarioRecord {
                feature: f.into(),
                scenario: s.into(),
                end: "exit 0".into(),
                stdout_bytes: 1,
                stderr_bytes: 0,
                stderr_head: String::new(),
                stable: true,
                probe_agrees: true,
                noted: "complete".into(),
                reason: None,
                functions: funcs
                    .into_iter()
                    .map(|(a, b)| (a.into(), b.into()))
                    .collect(),
            };
        let map = harness_core::features::FeatureMap {
            schema: harness_core::features::MAP_SCHEMA_NAME.into(),
            schema_version: 1,
            inputs: now,
            unwatched: vec![],
            unwatched_reasons: Vec::new(),
            scenarios: vec![
                record(
                    "gzip",
                    "text",
                    vec![
                        ("src/zopfli/katajainen.c", "ZopfliLengthLimitedCodeLengths"),
                        ("src/zopfli/zopfli_bin.c", "main"),
                    ],
                ),
                record("help", "flag", vec![("src/zopfli/zopfli_bin.c", "main")]),
            ],
        };
        std::fs::write(
            harness_core::features::map_path(&app.config.target),
            map.to_bytes().unwrap(),
        )
        .unwrap();
    }

    /// Speed results on a copy of zopfli: the C alone, the program as it
    /// stands and u001, each on two workloads, with today's digests (or
    /// `stale` ones).
    fn write_speed_results(app: &App, stale: bool) -> App {
        use harness_core::perf::results::{
            self as res, Compilers, Computer, CrateDigest, LeftOut, ProgramResults, Row, RowInputs,
            Run, Step1, Step1Run, UnitRef, UnitResults,
        };
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        std::fs::create_dir_all(root.join("bench")).unwrap();
        std::fs::write(root.join("bench/big.txt"), "big ".repeat(1000)).unwrap();
        std::fs::write(root.join("bench/small.txt"), "small\n").unwrap();
        std::fs::create_dir_all(&perf).unwrap();
        std::fs::write(
            perf.join("workloads.toml"),
            "schema_version = 1\n\
             [[workload]]\nid = \"big-text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/big.txt\"\n\
             [[workload]]\nid = \"many-small\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/small.txt\"\n",
        )
        .unwrap();
        // A first read gives today's digests.
        let id = "u001-katajainen";
        let fake = format!("blake3:{}", "f".repeat(64));
        std::fs::create_dir_all(perf.join(res::UNITS_DIR)).unwrap();
        let mut file = UnitResults::new(id);
        file.rows.push(Row {
            workload: "big-text".into(),
            outcome: "not-verified".into(),
            short: None,
            runs: None,
            platform_metrics: None,
            inputs: RowInputs {
                workload: fake.clone(),
                program: fake.clone(),
                crates: None,
                replaces: None,
                program_name: "zopfli".into(),
                units: None,
                left_out: None,
                recipe: harness_core::perf::PERF_RECIPE.into(),
                launcher: harness_core::perf::PERF_LAUNCHER.into(),
                computer: Computer {
                    os: "15.6".into(),
                    build: "24G84".into(),
                    arch: "arm64".into(),
                    cpu: "Apple M3".into(),
                    two_kinds: true,
                    fast_cores: 4,
                },
                compilers: Compilers {
                    cc: "Apple clang version 17.0.0".into(),
                    rustc: Some("rustc 1.94.1 (e408947bf 2026-03-25)".into()),
                },
            },
            c: None,
            other: None,
            std: None,
            fat_lto: None,
            profile: None,
            step1: None,
            failed_run: None,
            setup: Some(res::SetupFacts {
                reason: Some("not-fresh".into()),
                ..res::SetupFacts::default()
            }),
            first_difference: None,
            found_before: None,
            last_try: None,
        });
        res::write_unit(&res::unit_path(&perf, id), &file).unwrap();
        let read = crate::load::read(&root).unwrap();
        let p = &read.snapshot.perf;
        let digest = |w: &str| match p.inputs.get(w) {
            Some(crate::perfread::InputNow::Digest(d)) if !stale => d.clone(),
            _ => fake.clone(),
        };
        let program = if stale {
            fake.clone()
        } else {
            p.program_now.clone().expect("the program's digest")
        };
        let krate = p.crates.get(id).expect("u001's crate digest").clone();
        let program_name = read.snapshot.program_name.clone();
        // Runs around `base` cycles, spread ±1 % evenly.
        let side = |base: f64| -> Vec<Run> {
            (0..15)
                .map(|i| {
                    let c = base * (1.0 + 0.01 * (i as f64 / 7.0 - 1.0));
                    Run {
                        instructions: Some(33_000_000_000),
                        cycles: Some(c as u64),
                        cpu_us: Some((c / 3_200.0) as u64),
                        wall_us: Some((c / 3_200.0) as u64 + 5_000),
                        memory: Some(12_400_000),
                        p_instructions: Some(33_000_000_000),
                        p_cycles: Some(c as u64),
                        load: Some(150),
                        end: "exit 0".into(),
                        ..Run::default()
                    }
                })
                .collect()
        };
        let short = |cpu: u64| Step1 {
            c_first: Step1Run {
                instructions: Some(2_000_000),
                cpu_us: Some(cpu),
                end: "exit 0".into(),
                stdout_bytes: 30,
                stderr_bytes: 0,
            },
            other: None,
            c_second: Step1Run {
                instructions: Some(2_000_000),
                cpu_us: Some(cpu),
                end: "exit 0".into(),
                stdout_bytes: 30,
                stderr_bytes: 0,
            },
        };
        let base = file.rows[0].clone();
        let inputs = |w: &str, rust: bool| RowInputs {
            workload: digest(w),
            program: program.clone(),
            program_name: program_name.clone(),
            compilers: Compilers {
                cc: base.inputs.compilers.cc.clone(),
                rustc: rust.then(|| base.inputs.compilers.rustc.clone().unwrap()),
            },
            ..base.inputs.clone()
        };
        let measured = |w: &str, rust: bool, slower: Option<f64>| Row {
            workload: w.into(),
            outcome: if slower.is_some() {
                "measured"
            } else {
                "baseline"
            }
            .into(),
            short: Some(false),
            runs: Some(15),
            platform_metrics: Some("macos-v6-cycles".into()),
            inputs: inputs(w, rust),
            c: Some(side(3.9e9)),
            other: slower.map(|s| side(3.9e9 * (1.0 + s))),
            std: rust.then_some(true),
            setup: None,
            ..base.clone()
        };
        let too_short = |w: &str, rust: bool| Row {
            workload: w.into(),
            outcome: "too-short".into(),
            inputs: inputs(w, rust),
            step1: Some(Step1 {
                other: rust.then(|| short(1_100).c_first),
                ..short(1_000)
            }),
            setup: None,
            ..base.clone()
        };
        let mut unit = UnitResults::new(id);
        let mut u_big = measured("big-text", true, Some(0.062));
        u_big.inputs.crates = Some(vec![CrateDigest {
            id: id.into(),
            digest: krate.clone(),
        }]);
        u_big.inputs.replaces = Some(
            read.snapshot
                .unit(id)
                .unwrap()
                .unit
                .oracle_param_list("replaces"),
        );
        let mut u_small = too_short("many-small", true);
        u_small.inputs.crates = u_big.inputs.crates.clone();
        u_small.inputs.replaces = u_big.inputs.replaces.clone();
        unit.rows = vec![u_big, u_small];
        res::write_unit(&res::unit_path(&perf, id), &unit).unwrap();
        let held = |mut r: Row| {
            r.inputs.units = Some(vec![UnitRef {
                id: id.into(),
                crate_digest: krate.clone(),
            }]);
            r.inputs.left_out = Some(vec![LeftOut {
                id: "u-zopfli_bin".into(),
                crate_digest: String::new(),
                reason: "does-not-link".into(),
            }]);
            r
        };
        let program_file = ProgramResults {
            c_alone: vec![
                measured("big-text", false, None),
                too_short("many-small", false),
            ],
            as_it_stands: vec![
                held(measured("big-text", true, Some(0.062))),
                held(too_short("many-small", true)),
            ],
            ..ProgramResults::default()
        };
        res::write_program(&res::program_path(&perf), &program_file).unwrap();
        crate::app::tests::app_of_path(&root)
    }

    /// The workload whose id is the longest allowed (24 characters).
    const LONG_WORKLOAD: &str = "compress-a-big-text-file";

    /// [`write_speed_results`] and what the Speed View's golden must show
    /// (§4 *The cockpit*, build notes 16, 23 and 28): a workload whose id is
    /// 24 characters, its C-alone line at 12.41 s and 124.3 MB with a C-side
    /// last try beside it, a unit row on several cores, and the program as
    /// it stands built with a second rustc.
    fn write_speed_golden(app: &App) -> App {
        use harness_core::perf::results::{self as res, LastTry};
        let app = write_speed_results(app, false);
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        assert_eq!(LONG_WORKLOAD.len(), 24);
        let workloads = perf.join("workloads.toml");
        let mut text = std::fs::read_to_string(&workloads).unwrap();
        text.push_str(&format!(
            "[[workload]]\nid = \"{LONG_WORKLOAD}\"\nargs = [\"--i10\", \"{{input}}\"]\n\
             input = \"bench/big.txt\"\n"
        ));
        std::fs::write(&workloads, text).unwrap();
        let read = crate::load::read(&root).unwrap();
        let Some(crate::perfread::InputNow::Digest(digest)) =
            read.snapshot.perf.inputs.get(LONG_WORKLOAD).cloned()
        else {
            panic!("{:?}", read.snapshot.perf.inputs)
        };
        let path = res::program_path(&perf);
        let mut program = res::read_program(&path).unwrap().unwrap();
        let mut c = program.c_alone[0].clone();
        c.workload = LONG_WORKLOAD.into();
        c.inputs.workload = digest.clone();
        for r in c.c.iter_mut().flatten() {
            r.cpu_us = Some(12_410_000);
            r.wall_us = Some(12_415_000);
            r.memory = Some(124_300_000);
        }
        c.last_try = Some(LastTry {
            outcome: "c-unstable".into(),
            setup: None,
            units: None,
        });
        program.c_alone.push(c);
        for r in &mut program.as_it_stands {
            r.inputs.compilers.rustc = Some("rustc 1.95.0 (0a1b2c3d4 2026-05-14)".into());
        }
        res::write_program(&path, &program).unwrap();
        let id = "u001-katajainen";
        let path = res::unit_path(&perf, id);
        let mut unit = res::read_unit(&path, id).unwrap().unwrap();
        let mut u = unit.rows[0].clone();
        u.workload = LONG_WORKLOAD.into();
        u.inputs.workload = digest;
        for r in u.other.iter_mut().flatten() {
            r.wall_us = r.cpu_us.map(|c| c / 4);
        }
        unit.rows.push(u);
        res::write_unit(&path, &unit).unwrap();
        crate::app::tests::app_of_path(&root)
    }

    #[test]
    fn the_speed_view_at_54_columns() {
        let app = app_of("targets/zopfli", "speed-view");
        let mut app = write_speed_golden(&app);
        assert!(
            app.snapshot.perf.units.values().all(Result::is_ok),
            "{:?}",
            app.snapshot.perf.units
        );
        assert!(
            matches!(app.snapshot.perf.program, Ok(Some(_))),
            "{:?}",
            app.snapshot.perf.program
        );
        app.select(Selection::Speed);
        let buffer = render(&mut app, 80, 40);
        golden("speed-54.txt", &app, &buffer);
        let screen = text(&buffer);
        // The 24-character id whole, its short forms whole beside it at
        // column 2 + 26, the last try under its row, the header's compilers.
        let view: Vec<&str> = screen.lines().filter_map(|l| l.split('│').nth(3)).collect();
        for (short, at) in [("CPU 12.4 s · 124 MB", 0), ("slower 6.2 % · parallel", 1)] {
            let line = view
                .iter()
                .filter(|l| l.starts_with(&format!("  {LONG_WORKLOAD}  ")))
                .nth(at)
                .unwrap_or_else(|| panic!("{screen}"));
            assert_eq!(line.trim_end(), format!("  {LONG_WORKLOAD}  {short}"));
        }
        let c = view
            .iter()
            .position(|l| l.starts_with(&format!("  {LONG_WORKLOAD}  CPU")))
            .unwrap();
        assert!(
            view[c + 1].starts_with("    last try: the C ends or prints"),
            "{screen}"
        );
        assert!(
            view.iter()
                .any(|l| l.contains("with 2 compilers — see each row")),
            "{screen}"
        );
        // The focused row's full sentence below the list, and the computer
        // and compilers it records.
        app.focus = Focus::View;
        app.link = Some(6);
        let screen = text(&render(&mut app, 80, 60));
        assert!(
            screen.contains("u001-katajainen on big-text — slower"),
            "{screen}"
        );
        assert!(
            screen.contains("measured on Apple M3, 15.6 24G84, with rustc 1.94.1"),
            "{screen}"
        );
        // The unit's header and the project summary.
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Speed: slower 6.2 % on big-text"),
            "{screen}"
        );
        assert!(
            screen.contains("(5.6–6.8 %) · 2 of 3 workloads"),
            "{screen}"
        );
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Speed: 1 of 1 unit measured — 1 slower, 1 parallel — see Speed"),
            "{screen}"
        );
        let mut app = write_speed_results(&app, false);
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("(5.6–6.8 %) · 1 of 2 workloads"),
            "{screen}"
        );
        // Stale digests: every row says why, dimmed.
        let mut app = write_speed_results(&app, true);
        app.select(Selection::Speed);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains(
                "slower 6.2 % (5.6–6.8 %) · out of date: your workload changed, the C changed"
            ),
            "{screen}"
        );
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(screen.contains("· 1 out of date — see Speed"), "{screen}");
    }

    #[test]
    fn speed_menus_build_the_measure_commands() {
        use crate::speed::SideKey;
        let find = |items: &[crate::menu::Item], label: &str| {
            items.iter().find(|i| i.label.starts_with(label)).cloned()
        };
        let mut app = app_of("targets/zopfli", "speed-menu");
        app.select(Selection::Speed);
        let items = app.menu_items();
        assert!(
            find(&items, "Write your workloads file (in ").is_some(),
            "{items:?}"
        );
        let measure = find(&items, "Measure speed").expect("Measure speed");
        assert!(measure.greyed.is_some(), "no workloads file: {measure:?}");
        let mut app = write_speed_results(&app, false);
        app.select(Selection::Speed);
        let items = app.menu_items();
        assert!(
            find(&items, "Edit the workloads file (in ").is_some(),
            "{items:?}"
        );
        let measure = find(&items, "Measure speed").unwrap();
        if !cfg!(target_os = "macos") {
            assert!(
                measure
                    .greyed
                    .as_deref()
                    .is_some_and(|g| g.contains("macOS only")),
                "{measure:?}"
            );
            return;
        }
        assert_eq!(measure.greyed, None, "{measure:?}");
        let p = measure.pending.unwrap();
        let target = format!("--target={}", app.config.target.display());
        assert_eq!(
            crate::app::tests::strs(&p.argv),
            [crate::app::tests::HARNESS, "--json", "perf", "run", &target]
        );
        assert!(
            find(&items, "Measure the program as it stands").is_none(),
            "one measurable unit: {items:?}"
        );
        let (title, body) = app.dialog_words(&p);
        assert_eq!(title, "Measure speed?");
        assert_eq!(
            body[0],
            "Runs your program on 2 workloads (big-text, many-small): the C alone, then the \
             program with each verified unit's Rust alone (u001-katajainen) — 15 runs a side."
        );
        assert!(
            body[1].starts_with("Takes about ") && body[1].contains("builds included"),
            "{body:?}"
        );
        assert!(
            body.iter().any(|l| l.starts_with("No verdict changes")),
            "{body:?}"
        );
        assert!(
            body.iter()
                .any(|l| l.starts_with("Keep the computer quiet")),
            "{body:?}"
        );
        // A unit: its own command; nothing asks for more runs yet.
        app.select(Selection::Unit("u001-katajainen".into()));
        let items = app.menu_items();
        let this = find(&items, "Measure this unit's speed").expect("on a verified unit");
        let argv = crate::app::tests::strs(&this.pending.unwrap().argv);
        assert_eq!(
            argv.last().map(String::as_str),
            Some("--unit=u001-katajainen")
        );
        assert!(find(&items, "Measure again with 31 runs").is_none());
        app.select(Selection::Unit("u-cache".into()));
        assert!(
            find(&app.menu_items(), "Measure this unit's speed").is_none(),
            "a unit still in C has no speed of its own"
        );
        // A row that cannot tell asks for 31 runs on its workload only.
        let perf = harness_core::perf::perf_dir(&app.config.target);
        let path = harness_core::perf::results::unit_path(&perf, "u001-katajainen");
        let mut file = harness_core::perf::results::read_unit(&path, "u001-katajainen")
            .unwrap()
            .unwrap();
        for (i, r) in file.rows[0].other.as_mut().unwrap().iter_mut().enumerate() {
            let f = if i % 2 == 0 { 0.88 } else { 1.14 };
            r.cycles = Some((3.9e9 * f) as u64);
            r.p_cycles = r.cycles;
        }
        harness_core::perf::results::write_unit(&path, &file).unwrap();
        let mut app = crate::app::tests::app_of_path(&app.config.target);
        let side = SideKey::Unit("u001-katajainen".into());
        assert_eq!(
            app.speed.more_runs(&side),
            ["big-text"],
            "{:?}",
            app.speed.unit("u001-katajainen")
        );
        app.select(Selection::Unit("u001-katajainen".into()));
        let items = app.menu_items();
        let more = find(&items, "Measure again with 31 runs (big-text)").expect("offered");
        let p = more.pending.unwrap();
        let argv = crate::app::tests::strs(&p.argv);
        assert_eq!(
            argv[5..],
            ["--unit=u001-katajainen", "--workload=big-text", "--runs=31"]
        );
        let (title, body) = app.dialog_words(&p);
        assert_eq!(title, "Measure u001-katajainen again with 31 runs?");
        assert!(body[0].contains("on 1 workload (big-text)"), "{body:?}");
    }

    /// A unit row that behaves differently: its fact, its next step, the
    /// change by provenance; the outputs compared from the kept files —
    /// never from files that do not match what the row recorded.
    #[test]
    fn a_difference_is_a_fact_with_its_outputs_to_compare() {
        use harness_core::perf::results::{self as res, Difference, KeptFile};
        let app = app_of("targets/zopfli", "speed-differs");
        let mut app = write_speed_results(&app, false);
        // The slower row first: its next step, by provenance (zopfli's u001
        // records no attempt: commit, edit, verify).
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 200, 50));
        assert!(
            screen.contains("Next: perf times the Rust in use."),
            "{screen}"
        );
        assert!(
            screen.contains("Commit the unit's crate first (git) — replacing it deletes it"),
            "{screen}"
        );
        // Now its row prints differently, both outputs kept.
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        let id = "u001-katajainen";
        let path = res::unit_path(&perf, id);
        let mut file = res::read_unit(&path, id).unwrap().unwrap();
        let kept_dir = harness_core::perf::kept_outputs_dir(&root, Some(id));
        std::fs::create_dir_all(&kept_dir).unwrap();
        let outputs = [
            ("big-text.c.stdout", &b"line one\nline two\n"[..]),
            ("big-text.c.stderr", &b""[..]),
            ("big-text.other.stdout", &b"line one\nline 2\n"[..]),
            ("big-text.other.stderr", &b""[..]),
        ];
        let mut kept = Vec::new();
        for (name, bytes) in outputs {
            std::fs::write(kept_dir.join(name), bytes).unwrap();
            kept.push(KeptFile {
                name: name.into(),
                size: bytes.len() as u64,
                blake3: harness_core::hash::bytes_hash(bytes),
            });
        }
        let r = &mut file.rows[0];
        r.outcome = "behaves-differently".into();
        r.c = None;
        r.other = None;
        r.runs = None;
        r.short = None;
        r.platform_metrics = None;
        r.std = None;
        r.first_difference = Some(Difference {
            stream: "stdout".into(),
            c_len: 18,
            other_len: 16,
            offset: 14,
            c_end: "exit 0".into(),
            other_end: "exit 0".into(),
            over_cap: false,
            kept,
        });
        res::write_unit(&path, &file).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        app.select(Selection::Unit(id.into()));
        let screen = text(&render(&mut app, 200, 50));
        assert!(
            screen.contains(
                "With u001-katajainen's Rust the program prints differently (stdout, byte 15) on \
                 big-text — verify does not run this workload"
            ),
            "{screen}"
        );
        assert!(
            screen.contains("Next: Compare the outputs; then change the unit's Rust (below)"),
            "{screen}"
        );
        assert!(!screen.contains("perf times the Rust in use"), "{screen}");
        let items = app.menu_items();
        let compare = items
            .iter()
            .find(|i| i.label == "Compare the outputs (big-text)")
            .expect("offered")
            .clone();
        let crate::menu::Action::CompareOutputs(side, workload) = compare.action else {
            panic!()
        };
        let Ok(Mode::Diff { lines, title, .. }) = app.compare_outputs(&side, &workload) else {
            panic!("the comparison")
        };
        assert_eq!(title, "the C and u001-katajainen's Rust on big-text");
        assert!(lines.iter().any(|l| l == "-line two"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "+line 2"), "{lines:?}");
        assert!(
            lines[2].contains("first difference at byte 15"),
            "{lines:?}"
        );
        // A kept file that changed since: not compared.
        std::fs::write(
            kept_dir.join("big-text.other.stdout"),
            b"line one\nline 3\n",
        )
        .unwrap();
        assert_eq!(
            app.compare_outputs(&side, &workload).err().as_deref(),
            Some("the two outputs are not on this computer — measure again")
        );
    }

    /// A behaves-differently fact is kept through an Accept (§3.11 [m25]):
    /// it clears only when a re-measure ends measured or too-short; after
    /// the unit's crate changed it stays, "found before the unit's Rust
    /// changed", with its next step and the outputs still to compare — and
    /// a later measure that ended another way keeps it as found before.
    #[test]
    fn a_difference_is_kept_through_an_accept() {
        use harness_core::perf::results::{self as res, Difference, KeptFile};
        let app = app_of("targets/zopfli", "speed-differs-accept");
        let app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        let id = "u001-katajainen";
        let path = res::unit_path(&perf, id);
        let mut file = res::read_unit(&path, id).unwrap().unwrap();
        let kept_dir = harness_core::perf::kept_outputs_dir(&root, Some(id));
        std::fs::create_dir_all(&kept_dir).unwrap();
        let mut kept = Vec::new();
        for (name, bytes) in [
            ("big-text.c.stdout", &b"line one\nline two\n"[..]),
            ("big-text.other.stdout", &b"line one\nline 2\n"[..]),
        ] {
            std::fs::write(kept_dir.join(name), bytes).unwrap();
            kept.push(KeptFile {
                name: name.into(),
                size: bytes.len() as u64,
                blake3: harness_core::hash::bytes_hash(bytes),
            });
        }
        let difference = Difference {
            stream: "stdout".into(),
            c_len: 18,
            other_len: 16,
            offset: 14,
            c_end: "exit 0".into(),
            other_end: "exit 0".into(),
            over_cap: false,
            kept,
        };
        let r = &mut file.rows[0];
        r.outcome = "behaves-differently".into();
        r.c = None;
        r.other = None;
        r.runs = None;
        r.short = None;
        r.platform_metrics = None;
        r.std = None;
        r.first_difference = Some(difference.clone());
        // The state an Accept leaves: today's crate is not the one the row
        // measured (the verdict stays green and fresh).
        for c in r.inputs.crates.iter_mut().flatten() {
            c.digest = format!("blake3:{}", "e".repeat(64));
        }
        res::write_unit(&path, &file).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        let unit = app.snapshot.unit(id).unwrap().clone();
        let fact =
            "With u001-katajainen's Rust the program prints differently (stdout, byte 15) on \
                    big-text — verify does not run this workload";
        let next = "Compare the outputs; then change the unit's Rust (below) and measure this \
                    unit again";
        let advice = app.speed.advice(&unit, true);
        assert_eq!(
            advice.differences,
            [format!(
                "{fact} — found before the unit's Rust changed — measure this unit again to \
                 check"
            )]
        );
        assert_eq!(advice.next.as_deref(), Some(next));
        app.select(Selection::Unit(id.into()));
        let screen = text(&render(&mut app, 300, 50));
        assert!(
            screen.contains("found before the unit's Rust changed"),
            "{screen}"
        );
        assert!(
            app.menu_items()
                .iter()
                .any(|i| i.label == "Compare the outputs (big-text)"),
            "the outputs are still there to compare"
        );
        // A later measure that ended another way (a set-up row): kept as
        // found before.
        let mut file = res::read_unit(&path, id).unwrap().unwrap();
        let today = file.rows[1].inputs.crates.clone();
        let r = &mut file.rows[0];
        r.outcome = "not-verified".into();
        r.first_difference = None;
        r.found_before = Some(difference);
        r.setup = Some(res::SetupFacts {
            reason: Some("not-fresh".into()),
            ..res::SetupFacts::default()
        });
        r.inputs.crates = today;
        res::write_unit(&path, &file).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert_eq!(
            app.speed.advice(&unit, true).differences,
            [format!(
                "{fact} — found before; the last measure ended another way — measure again to \
                 check"
            )]
        );
    }

    /// The program as it stands printing differently: on its heading and
    /// in the summary, with which unit to measure alone.
    #[test]
    fn a_difference_as_it_stands_names_its_units() {
        use harness_core::perf::results::{self as res, Difference};
        let app = app_of("targets/zopfli", "speed-differs-ais");
        let mut app = write_speed_results(&app, false);
        let perf = harness_core::perf::perf_dir(&app.config.target);
        let path = res::program_path(&perf);
        let mut program = res::read_program(&path).unwrap().unwrap();
        let r = &mut program.as_it_stands[0];
        r.outcome = "behaves-differently".into();
        r.c = None;
        r.other = None;
        r.runs = None;
        r.short = None;
        r.platform_metrics = None;
        r.std = None;
        r.first_difference = Some(Difference {
            stream: "exit".into(),
            c_len: 0,
            other_len: 0,
            offset: 0,
            c_end: "exit 0".into(),
            other_end: "exit 1".into(),
            over_cap: false,
            kept: Vec::new(),
        });
        res::write_program(&path, &program).unwrap();
        app = crate::app::tests::app_of_path(&app.config.target);
        let diffs = app.speed.program_differences();
        assert_eq!(
            diffs,
            [(
                "With the program as it stands (u001-katajainen) the program exits differently \
                 (exit 1 where the C has exit 0) on big-text"
                    .to_string(),
                "no unit's Rust differs alone — it is how they work together".to_string()
            )]
        );
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 200, 50));
        assert!(
            screen.contains(
                "Speed: With the program as it stands (u001-katajainen) the program exits"
            ),
            "{screen}"
        );
        // The help says what perf compares.
        let (rows, _) = help_rows(1000, true, None);
        let help: String = rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(
            help.contains("perf stops even a fork; verify allows a fork but not"),
            "{help}"
        );
    }

    /// How to change a unit's Rust follows where it came from (§3.11).
    #[test]
    fn the_change_follows_the_provenance() {
        use crate::model::ProvenanceView as P;
        use crate::speed::change_words;
        let app = crate::app::tests::app("speed-change");
        let mut unit = app.snapshot.unit("u-lib").unwrap().clone();
        unit.provenance = P::Pipeline("a-13c941dfff95".into());
        assert_eq!(
            change_words(&unit, true),
            "Modify a-13c9 with a note about speed (give these numbers), then Replace u-lib's \
             verified crate with the new attempt, measure this unit again — and if it is not \
             faster, Replace it back with a-13c9"
        );
        // No provider: Modify is greyed, said in the cockpit's own words
        // (build note 30) — the very reason the greyed item gives.
        let mut no_provider = App::new(
            crate::app::Config {
                providers: Vec::new(),
                ..app.config.clone()
            },
            crate::load::read(&app.config.target).unwrap(),
        );
        no_provider.select(Selection::Attempt(
            "u-lib".into(),
            crate::app::tests::PROVENANCE.into(),
        ));
        let modify = no_provider
            .menu_items()
            .into_iter()
            .find(|i| i.label == "Modify with a note")
            .expect("Modify offered, greyed");
        assert_eq!(modify.greyed.as_deref(), Some(crate::model::NO_PROVIDER));
        assert_eq!(
            change_words(&unit, false),
            format!(
                "Modify is greyed: {} — with one, Modify a-13c9 with a note about speed (give \
                 these numbers), then Replace u-lib's verified crate with the new attempt, \
                 measure this unit again — and if it is not faster, Replace it back with a-13c9",
                crate::model::NO_PROVIDER
            )
        );
        // A steer's attempt and a chat's are a model's too: the same words.
        let modify = change_words(&unit, true);
        for made in [
            P::Steered("a-13c941dfff95".into()),
            P::Chat("a-13c941dfff95".into()),
        ] {
            unit.provenance = made.clone();
            assert_eq!(change_words(&unit, true), modify, "{made:?}");
        }
        unit.provenance = P::Ambiguous(vec!["a-28d8aaaa".into(), "a-13c9bbbb".into()]);
        assert!(
            change_words(&unit, true).starts_with("Modify a-13c9 "),
            "the lowest id"
        );
        let dir = std::env::temp_dir().join(format!("speed-change-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/logic.rs"), "").unwrap();
        std::fs::write(dir.join("src/ffi.rs"), "").unwrap();
        unit.crate_dir = Some(dir.clone());
        // A recorded hand edit: the edit alone is never accepted, so the
        // words name Replace (and the way back to the attempt in use now) —
        // build note 22; tests/speed_change.rs runs each act.
        unit.provenance = P::Human {
            attempt: "a-77b2aaaa".into(),
            origin: "a-66c1bbbb".into(),
        };
        assert_eq!(
            change_words(&unit, true),
            "Hand edit u-lib's crate, then Replace u-lib's verified crate with the new attempt \
             and measure this unit again — and if it is not faster, Replace it back with a-77b2"
        );
        // A crate without either file Hand edit opens: commit, edit, verify.
        for missing in ["src/ffi.rs", "src/logic.rs"] {
            std::fs::remove_file(dir.join(missing)).unwrap();
            assert!(
                change_words(&unit, true).starts_with("Commit the unit's crate first (git)"),
                "without {missing}"
            );
            std::fs::write(dir.join(missing), "").unwrap();
        }
        // Code the cockpit did not record, even with both files: there is
        // no attempt to Replace it back with.
        unit.provenance = P::None;
        assert!(change_words(&unit, true).starts_with("Commit the unit's crate first (git)"));
        assert!(change_words(&unit, true).contains("run harness verify u-lib in a terminal"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_speed_group_says_its_state() {
        let mut app = app_of("targets/zopfli", "speed-states");
        assert_eq!(app.speed.label(), "Speed (no file)");
        app.select(Selection::Speed);
        let screen = text(&render(&mut app, 80, 40));
        assert!(
            screen.contains("press Enter and choose Write your"),
            "{screen}"
        );
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            !screen.contains("Speed:"),
            "no workloads file, no line: {screen}"
        );
        let dir = harness_core::perf::perf_dir(&app.config.target);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("workloads.toml"),
            harness_core::perf::workloads::STARTER,
        )
        .unwrap();
        let app = crate::app::tests::app_of_path(&app.config.target);
        assert_eq!(app.speed.label(), "Speed (no workload)");
        std::fs::write(
            dir.join("workloads.toml"),
            "schema_version = 1\n[[workload]]\nid = \"W\"\n",
        )
        .unwrap();
        let mut app = crate::app::tests::app_of_path(&app.config.target);
        assert_eq!(app.speed.label(), "Speed (file error)");
        assert_eq!(
            app.node_label(&Selection::Speed),
            ("⚠", "error".to_string())
        );
        app.select(Selection::Speed);
        let screen = text(&render(&mut app, 80, 40));
        assert!(
            screen.contains("workloads.toml line 3, column 6:"),
            "{screen}"
        );
        std::fs::write(
            dir.join("workloads.toml"),
            "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = [\"-h\"]\n",
        )
        .unwrap();
        let mut app = crate::app::tests::app_of_path(&app.config.target);
        assert_eq!(app.speed.label(), "Speed (not yet run)");
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Speed: not measured yet — see Speed"),
            "{screen}"
        );
    }

    /// "Measurable today" is perf's own selection (§3.2, build note 24): a
    /// unit whose plan `replaces` changed since its verdict — which stays
    /// green and fresh — is left out, its Measure greyed in perf's words,
    /// and the program as it stands is judged against that.
    #[test]
    fn measurable_today_follows_every_condition_of_perfs_selection() {
        use harness_core::perf::results::{self as res, LeftOut};
        let app = app_of("targets/zopfli", "speed-measurable");
        let app = write_speed_results(&app, false);
        let id = "u001-katajainen";
        let root = app.config.target.clone();
        assert_eq!(app.speed.measurable, [id]);
        assert!(
            app.speed.program_rows[0].out_of_date.is_empty(),
            "{:?}",
            app.speed.program_rows[0].out_of_date
        );
        // The plan's replaces edited after verify.
        let plan = root.join("migration/plan.toml");
        let text = std::fs::read_to_string(&plan).unwrap();
        let edited = text.replace(
            r#"replaces = ["src/zopfli/katajainen.c"]"#,
            r#"replaces = ["src/zopfli/katajainen.c", "src/zopfli/util.c"]"#,
        );
        assert_ne!(edited, text);
        std::fs::write(&plan, edited).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert!(app.snapshot.unit(id).unwrap().report.fresh_green());
        assert!(
            app.speed.measurable.is_empty(),
            "{:?}",
            app.speed.measurable
        );
        // Held by the row, left out now.
        assert_eq!(
            app.speed.program_rows[0].out_of_date,
            ["u001-katajainen is left out now"]
        );
        app.select(Selection::Unit(id.into()));
        let this = app
            .menu_items()
            .into_iter()
            .find(|i| i.label == "Measure this unit's speed")
            .expect("on a verified unit");
        if cfg!(target_os = "macos") {
            assert_eq!(
                this.greyed.as_deref(),
                Some("u001-katajainen's replaced files changed since verify — Re-check it")
            );
        }
        // A row that left it out for that reason is current: nothing was
        // verified since.
        let perf = harness_core::perf::perf_dir(&root);
        let path = res::program_path(&perf);
        let mut program = res::read_program(&path).unwrap().unwrap();
        for r in &mut program.as_it_stands {
            let held = r.inputs.units.take().unwrap();
            r.inputs.units = Some(Vec::new());
            r.inputs.left_out.as_mut().unwrap().push(LeftOut {
                id: id.into(),
                crate_digest: held[0].crate_digest.clone(),
                reason: "replaces-changed".into(),
            });
        }
        res::write_program(&path, &program).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        for r in &app.speed.program_rows {
            assert!(r.out_of_date.is_empty(), "{:?}", r.out_of_date);
        }
    }

    /// A unit's header (§3.11, build note 16): line 1 the short form without
    /// its interval or "· parallel"; line 2 the interval the headline gives
    /// (the × one at 2× and more), "parallel" whenever the row uses several
    /// cores — even where the short form had no room for it —, the count.
    #[test]
    fn the_unit_header_carries_the_interval_and_parallel() {
        use harness_core::perf::results as res;
        let app = app_of("targets/zopfli", "speed-header");
        let app = write_speed_golden(&app);
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        let id = "u001-katajainen";
        let path = res::unit_path(&perf, id);
        // The other side `f` times the C's cycles, on four cores.
        let header = |f: f64| {
            let mut file = res::read_unit(&path, id).unwrap().unwrap();
            file.rows.retain(|r| r.workload == LONG_WORKLOAD);
            let c = file.rows[0].c.clone().unwrap();
            for (o, c) in file.rows[0].other.iter_mut().flatten().zip(&c) {
                let cycles = (c.cycles.unwrap() as f64 * f) as u64;
                o.cycles = Some(cycles);
                o.p_cycles = Some(cycles);
                o.cpu_us = Some(cycles / 3_200);
                o.wall_us = Some(cycles / 3_200 / 4);
            }
            res::write_unit(&path, &file).unwrap();
            let app = crate::app::tests::app_of_path(&root);
            let (a, b) = app.speed.unit_header(id).unwrap();
            assert!(a.chars().count() <= 54 && b.chars().count() <= 54);
            (
                app.speed.unit(id).unwrap().rows[0].words.short.clone(),
                a,
                b,
            )
        };
        let (short, a, b) = header(1.062);
        assert_eq!(short, "slower 6.2 % · parallel");
        assert_eq!(a, format!("Speed: slower 6.2 % on {LONG_WORKLOAD}"));
        assert_eq!(b, "(5.6–6.8 %) · parallel · 1 of 1 workload");
        let (short, a, b) = header(3.7);
        assert_eq!(short, "3.7× as slow · parallel");
        assert_eq!(a, format!("Speed: 3.7× as slow on {LONG_WORKLOAD}"));
        assert!(
            b.starts_with('(') && b.ends_with("×) · parallel · 1 of 1 workload"),
            "{b}"
        );
        // No room for "· parallel" in a close call's short form: the
        // header's second line still says it.
        let (short, a, b) = header(1.021);
        assert!(short.starts_with("close call: ≈"), "{short}");
        assert!(!short.contains("parallel"), "{short}");
        assert!(a.starts_with("Speed: close call: ≈"), "{a}");
        assert!(
            b.starts_with('(') && b.ends_with("%) · parallel · 1 of 1 workload"),
            "{b}"
        );
    }

    /// A unit whose only rows are set-up rows ("not measured") is counted
    /// among those that could be, never as measured (§3.11 summary).
    #[test]
    fn a_unit_with_only_set_up_rows_is_not_counted_as_measured() {
        use harness_core::perf::results as res;
        let app = app_of("targets/zopfli", "speed-set-up-only");
        let mut app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        let perf = harness_core::perf::perf_dir(&root);
        let id = "u001-katajainen";
        let path = res::unit_path(&perf, id);
        let mut file = res::read_unit(&path, id).unwrap().unwrap();
        let mut set_up = file.rows[0].clone();
        set_up.outcome = "not-verified".into();
        set_up.short = None;
        set_up.runs = None;
        set_up.platform_metrics = None;
        set_up.c = None;
        set_up.other = None;
        set_up.std = None;
        set_up.setup = Some(res::SetupFacts {
            reason: Some("not-fresh".into()),
            ..res::SetupFacts::default()
        });
        file.rows = vec![set_up];
        res::write_unit(&path, &file).unwrap();
        app = crate::app::tests::app_of_path(&root);
        assert_eq!(app.speed.label(), "Speed (0 of 1)");
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Speed: 0 of 1 unit measured — 1 not measured — see Speed"),
            "{screen}"
        );
    }

    /// Unit rows with no C-alone rows (a first `--unit` measure): the view
    /// says the C alone is not measured yet, never "Nothing measured yet"
    /// above the rows it lists.
    #[test]
    fn units_measured_alone_say_the_c_is_not_measured_yet() {
        let app = app_of("targets/zopfli", "speed-units-only");
        let app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        std::fs::remove_file(harness_core::perf::results::program_path(
            &harness_core::perf::perf_dir(&root),
        ))
        .unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        app.select(Selection::Speed);
        let screen = text(&render(&mut app, 80, 40));
        assert!(!screen.contains("Nothing measured yet"), "{screen}");
        assert!(screen.contains("The original C"), "{screen}");
        assert!(
            screen.contains("not measured yet — Measure speed on this row measures"),
            "{screen}"
        );
        assert!(
            screen.contains("big-text    slower 6.2 % (5.6–6.8 %)"),
            "{screen}"
        );
    }

    /// Rows from different computers and compilers: the header points to
    /// the rows once, and each row names its own (§3.11, build note 28).
    #[test]
    fn rows_from_other_computers_name_their_own() {
        use harness_core::perf::results as res;
        let app = app_of("targets/zopfli", "speed-computers");
        let app = write_speed_golden(&app);
        let root = app.config.target.clone();
        let path = res::program_path(&harness_core::perf::perf_dir(&root));
        let mut program = res::read_program(&path).unwrap().unwrap();
        for r in &mut program.c_alone {
            r.inputs.computer.cpu = "Apple M1".into();
        }
        res::write_program(&path, &program).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert_eq!(app.speed.header.len(), 1);
        let header = &app.speed.header[0];
        assert!(
            header.starts_with("measured on 2 kinds of computer with 2 compilers — see each row"),
            "{header}"
        );
        assert_eq!(header.matches("see each row").count(), 1, "{header}");
        assert!(app
            .speed
            .row(&crate::speed::SideKey::C, "big-text")
            .unwrap()
            .measured_on
            .starts_with("measured on Apple M1, 15.6 24G84, with Apple clang"));
        app.select(Selection::Speed);
        app.focus = Focus::View;
        app.link = Some(0);
        let screen = text(&render(&mut app, 120, 60));
        assert!(
            screen.contains("measured on Apple M1, 15.6 24G84, with Apple clang version 17.0.0"),
            "{screen}"
        );
    }

    /// Every short form, at its longest, whole beside a 24-character
    /// workload id within 54 columns; a last-try line wrapped whole under
    /// its row, never cut mid-word.
    #[test]
    fn short_forms_fit_beside_the_longest_workload_id() {
        let app = app_of("targets/zopfli", "speed-widths");
        let app = write_speed_golden(&app);
        let mut row = app
            .speed
            .row(&crate::speed::SideKey::C, LONG_WORKLOAD)
            .unwrap()
            .clone();
        let column = LONG_WORKLOAD.len() + 2;
        for short in [
            "slower 12 % (10.1–14.6 %)",
            "faster 12 % (10.1–14.6 %)",
            "close call: ≈9.9 % slower",
            "probably slower ≈9.9 %",
            "about as fast · parallel",
            "slower 9.9 % · parallel",
            "can't tell: slow cores",
            "short run: can't tell",
            "too short · Rust 99× CPU",
            "no clear diff ±99 %",
            "CPU 12.4 s · 124 MB",
        ] {
            row.words.short = short.into();
            let lines = speed_row_lines(&row, column, 54);
            let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(first, format!("  {LONG_WORKLOAD}  {short}"));
            assert!(width_of(&first) <= 54, "{first}");
            // The last try: whole, in rows of at most 54 columns.
            let rest: Vec<String> = lines[1..]
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            assert!(rest.iter().all(|l| width_of(l) <= 54), "{rest:?}");
            let joined = rest.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
            assert_eq!(
                joined,
                "last try: the C ends or prints differently from one run to the next"
            );
        }
    }

    /// Help's glossary has an entry for each can't-tell short form, each
    /// with its own next step (more runs only where they can settle it).
    #[test]
    fn the_glossary_names_each_kind_of_cant_tell() {
        let (rows, _) = help_rows(1000, true, None);
        let help: String = rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        for (short, next) in [
            ("can't tell: ±", "measure again with 31 runs"),
            ("can't tell: slow cores", "may have been busy"),
            ("can't tell: too few", "measure again"),
            (
                "short run: can't tell",
                "use a bigger input; more runs will not settle it",
            ),
            ("no clear diff ±", "31 runs found no difference"),
        ] {
            let line = help
                .lines()
                .find(|l| l.trim_start().starts_with(short))
                .unwrap_or_else(|| panic!("{short}: {help}"));
            assert!(line.contains(next), "{line}");
        }
        assert!(help.contains("at least 95 % sure"), "{help}");
        assert!(!help.contains("surely"), "{help}");
    }

    /// A unit the program as it stands left out with its crate's digest
    /// (its own link failed) is judged by that crate — hashed even with no
    /// results file of its own, as after an as-it-stands-only measure.
    #[test]
    fn a_unit_left_out_is_judged_by_its_own_crate() {
        use harness_core::perf::results::{self as res, LeftOut};
        let app = app_of("targets/zopfli", "speed-left-out");
        let app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        let id = "u001-katajainen";
        let perf = harness_core::perf::perf_dir(&root);
        let path = res::program_path(&perf);
        let mut program = res::read_program(&path).unwrap().unwrap();
        for r in &mut program.as_it_stands {
            let held = r.inputs.units.take().unwrap();
            r.inputs.units = Some(Vec::new());
            r.inputs.left_out.as_mut().unwrap().push(LeftOut {
                id: id.into(),
                crate_digest: held[0].crate_digest.clone(),
                reason: "does-not-link".into(),
            });
        }
        res::write_program(&path, &program).unwrap();
        std::fs::remove_file(res::unit_path(&perf, id)).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert!(app.snapshot.perf.crates.contains_key(id));
        for r in &app.speed.program_rows {
            assert!(r.out_of_date.is_empty(), "{:?}", r.out_of_date);
        }
    }

    /// The inputs the cockpit hashes (§3.11): never while a perf run holds
    /// the lock — the holder read inside `Snapshot::load` —, "can't check"
    /// past the load's budget, and perf's own words for an input it refuses
    /// (the target still opens).
    #[test]
    fn the_cockpit_hashes_no_input_while_measuring_and_says_what_it_cannot_check() {
        use crate::perfread::InputNow;
        use crate::speed::SideKey;
        let app = app_of("targets/zopfli", "speed-inputs");
        let app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        let lock = harness_core::ledger::Ledger::new(&root).lock_path();
        let holder = |command: &str| {
            format!(
                "{{\"pid\":{},\"command\":\"{command}\",\"started\":\"2026-09-25T00:00:00Z\"}}\n",
                std::process::id()
            )
        };
        let big = |app: &App| app.speed.row(&SideKey::C, "big-text").unwrap().clone();
        std::fs::write(
            &lock,
            holder(&format!("{} --target .", harness_core::perf::PERF_RUN_LOCK)),
        )
        .unwrap();
        std::fs::write(root.join("bench/big.txt"), "changed while measuring").unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert!(app.snapshot.perf.measuring);
        assert_eq!(
            app.snapshot.perf.inputs["big-text"],
            InputNow::WhileMeasuring
        );
        assert!(
            matches!(app.snapshot.perf.inputs["many-small"], InputNow::Digest(_)),
            "unchanged: its cached digest"
        );
        assert_eq!(big(&app).out_of_date, ["can't check while measuring"]);
        assert_eq!(big(&app).out_of_date_tokens, ["measuring"]);
        app.select(Selection::Speed);
        let screen = text(&render(&mut app, 80, 40));
        assert!(screen.contains("A perf run is measuring now"), "{screen}");
        // Another writer is no perf run: the input is hashed.
        std::fs::write(&lock, holder("verify u001-katajainen")).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert!(!app.snapshot.perf.measuring);
        assert_eq!(big(&app).out_of_date_tokens, ["workload"]);
        std::fs::remove_file(&lock).unwrap();
        // Past the load's budget: "can't check", the row's digest kept.
        let mut snapshot = app.snapshot.clone();
        snapshot
            .perf
            .inputs
            .insert("big-text".into(), InputNow::TooLarge);
        let model = crate::speed::build(&snapshot);
        let row = model.row(&SideKey::C, "big-text").unwrap();
        assert_eq!(
            row.out_of_date,
            ["can't check: inputs too large to hash here"]
        );
        assert_eq!(row.out_of_date_tokens, ["too-large"]);
        // An input perf refuses (over 64 MiB): its words, unread; the
        // target opens.
        std::fs::File::create(root.join("bench/huge.bin"))
            .unwrap()
            .set_len(harness_core::perf::workloads::MAX_INPUT_BYTES + 1)
            .unwrap();
        let workloads = harness_core::perf::workloads::workloads_path(&root);
        let text = std::fs::read_to_string(&workloads).unwrap();
        std::fs::write(&workloads, text.replace("bench/big.txt", "bench/huge.bin")).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert_eq!(
            big(&app).out_of_date,
            ["bench/huge.bin is over 64 MiB — use a smaller input"]
        );
        assert_eq!(big(&app).out_of_date_tokens, ["input-unusable"]);
    }

    /// Day one (§3.6): the C alone measured before a plan is judged as any
    /// row is; without facts only the C goes unchecked, and the header says
    /// so.
    #[test]
    fn the_c_alone_is_judged_before_a_plan() {
        use crate::speed::SideKey;
        let app = app_of("targets/zopfli", "speed-day-one");
        let app = write_speed_results(&app, false);
        let root = app.config.target.clone();
        std::fs::remove_file(root.join("migration/plan.toml")).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert_eq!(app.snapshot.note.as_deref(), Some(crate::model::NO_PLAN));
        let big = |app: &App| app.speed.row(&SideKey::C, "big-text").unwrap().clone();
        assert!(
            big(&app).out_of_date.is_empty(),
            "{:?}",
            big(&app).out_of_date
        );
        std::fs::write(root.join("bench/big.txt"), "another input").unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert_eq!(big(&app).out_of_date, ["your workload changed"]);
        // Without facts: the C cannot be hashed here; the rest is judged.
        std::fs::remove_file(root.join("migration/facts.jsonl")).unwrap();
        let app = crate::app::tests::app_of_path(&root);
        assert_eq!(big(&app).out_of_date_tokens, ["workload"]);
        assert!(
            app.speed
                .header
                .iter()
                .any(|h| h == "the C is not checked here: no facts — run harness scan"),
            "{:?}",
            app.speed.header
        );
    }

    /// Measure is greyed in the words `perf run` refuses with: the
    /// workloads file's state (§3.1), an interrupted Accept (§3.2), and the
    /// program as it stands's 31 runs with one measurable unit (§3.10).
    #[test]
    fn measure_is_greyed_with_perfs_own_words() {
        use harness_core::perf::results as res;
        use harness_core::perf::workloads::{self as wl, WorkloadsState};
        if !cfg!(target_os = "macos") {
            return; // greyed "macOS only" first.
        }
        let greyed = |app: &mut App, sel: Selection, label: &str| -> Option<String> {
            app.select(sel);
            app.menu_items()
                .into_iter()
                .find(|i| i.label == label)
                .unwrap_or_else(|| panic!("{label}"))
                .greyed
        };
        let mut app = app_of("targets/zopfli", "speed-greyed");
        let root = app.config.target.clone();
        assert_eq!(
            greyed(&mut app, Selection::Speed, "Measure speed"),
            WorkloadsState::NoFile.blocker()
        );
        let dir = harness_core::perf::perf_dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("workloads.toml"), wl::STARTER).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert_eq!(
            greyed(&mut app, Selection::Speed, "Measure speed"),
            WorkloadsState::NoWorkload.blocker()
        );
        std::fs::write(
            dir.join("workloads.toml"),
            "schema_version = 1\n[[workload]]\nid = \"w\"\nruns = 99\n",
        )
        .unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        let words = greyed(&mut app, Selection::Speed, "Measure speed").unwrap();
        assert_eq!(Some(words.clone()), wl::load(&root).unwrap().blocker());
        assert!(
            words.contains("workloads.toml line 4, column")
                && words.ends_with("— fix it, or Edit the workloads file"),
            "{words}"
        );
        // An interrupted Accept: perf's own words for it.
        write_speed_results(&app, false);
        let id = "u001-katajainen";
        let marker = root
            .join("migration/units")
            .join(id)
            .join(".promote-a-1234");
        std::fs::create_dir_all(&marker).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert_eq!(
            greyed(
                &mut app,
                Selection::Unit(id.into()),
                "Measure this unit's speed"
            )
            .as_deref(),
            Some(
                "an Accept of a-1234 was interrupted — Re-check u001-katajainen (or run harness \
                 verify u001-katajainen) to finish or undo it; Measure does not"
            )
        );
        std::fs::remove_dir(&marker).unwrap();
        // The program as it stands asks for 31 runs, but only one unit is
        // measurable: perf would build everything, then refuse.
        let path = res::program_path(&dir);
        let mut program = res::read_program(&path).unwrap().unwrap();
        for (i, r) in program.as_it_stands[0]
            .other
            .as_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            let f = if i % 2 == 0 { 0.88 } else { 1.14 };
            r.cycles = Some((3.9e9 * f) as u64);
            r.p_cycles = r.cycles;
        }
        res::write_program(&path, &program).unwrap();
        let mut app = crate::app::tests::app_of_path(&root);
        assert_eq!(app.speed.measurable, [id]);
        assert_eq!(
            app.speed.more_runs(&crate::speed::SideKey::AsItStands),
            ["big-text"]
        );
        assert_eq!(
            greyed(
                &mut app,
                Selection::Speed,
                "Measure the program as it stands again with 31 runs"
            )
            .as_deref(),
            Some(
                "the program as it stands needs two verified units — with one, that unit's own \
                 row measures the same program"
            )
        );
    }

    /// The committed zopfli (the dogfood): its features, its map, u001
    /// verified on them.
    #[test]
    fn the_committed_zopfli_features_are_mapped_and_hold() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "feat-dogfood");
        app.select(Selection::Features);
        let screen = text(&render(&mut app, 160, 40));
        if harness_core::features::platform() != "macos-aarch64" {
            // The committed map was made on macOS (aarch64).
            assert!(screen.contains("made on another platform"), "{screen}");
            return;
        }
        assert!(app.features.map.current());
        for line in [
            "◉ Compress to gzip  holds so far · 1 of 10 units",
            "◉ Compress to raw deflate  holds so far · 1 of 9 units",
            "◌ Show the help  all C",
            "◌ Report a missing file  all C",
            "Your features ran 108 of the 111 functions the map watches.",
        ] {
            assert!(screen.contains(line), "{line}\n{screen}");
        }
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("verdict green, fresh\u{20}"),
            "no marker on a verdict that covers today's features: {screen}"
        );
        assert!(!screen.contains("your features:"), "{screen}");
        assert!(
            screen.contains("5 of your features run it · all passed — see Features"),
            "{screen}"
        );
        // Review C7: a narrow View keeps the result, right after the id.
        app.select(Selection::Feature("gzip".into()));
        let screen = text(&render(&mut app, 80, 40));
        assert!(screen.contains("u001-katajainen  ✓ passed"), "{screen}");
        // Review C8: a unit still in C is not "not re-checked".
        app.select(Selection::Unit("u-zopfli_bin".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("of your features run it · still C"),
            "{screen}"
        );
        assert!(!screen.contains("not re-checked"), "{screen}");
        let path = harness_core::features::features_path(&app.config.target);
        let text_now = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text_now.replace("\"--i1\"", "\"--i2\"")).unwrap();
        let mut app = crate::app::tests::app_of_path(&app.config.target);
        assert!(!app.features.map.current());
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("your features: not checked since you changed them — Re-check it"),
            "{screen}"
        );
        // Review C14: its feature checks ran an earlier features file.
        assert!(
            screen.contains("(from an earlier features file)"),
            "{screen}"
        );
        // Review C6: what the last map says, said as the last map's.
        app.select(Selection::Features);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("In the last map, your features ran 108 of the 111 functions"),
            "{screen}"
        );
        assert!(!screen.contains("functions no feature ran"), "{screen}");
    }

    /// Review C5/C6/C10/C12/C14 and fix check N5: an incomplete map makes no
    /// negative claim, a crashed scenario is no check, Help says the rest.
    #[test]
    fn an_incomplete_map_makes_no_negative_claim() {
        let mut app = zopfli_with_features("feat-incomplete", FEATURES_TOML);
        let read = crate::load::read(&app.config.target).unwrap();
        let now = read.map_now.expect("today's inputs");
        let record = |f: &str, s: &str, end: &str, noted: &str, funcs: Vec<(&str, &str)>| {
            harness_core::features::ScenarioRecord {
                feature: f.into(),
                scenario: s.into(),
                end: end.into(),
                stdout_bytes: 1,
                stderr_bytes: 0,
                stderr_head: String::new(),
                stable: true,
                probe_agrees: true,
                noted: noted.into(),
                reason: (noted != "complete").then(|| "none written".to_string()),
                functions: funcs
                    .into_iter()
                    .map(|(a, b)| (a.into(), b.into()))
                    .collect(),
            }
        };
        let map = harness_core::features::FeatureMap {
            schema: harness_core::features::MAP_SCHEMA_NAME.into(),
            schema_version: 1,
            inputs: now,
            unwatched: vec![],
            unwatched_reasons: Vec::new(),
            scenarios: vec![
                record("gzip", "text", "exit 0", "unavailable", vec![]),
                record(
                    "help",
                    "flag",
                    "signal 11",
                    "complete",
                    vec![("src/zopfli/zopfli_bin.c", "main")],
                ),
            ],
        };
        std::fs::write(
            harness_core::features::map_path(&app.config.target),
            map.to_bytes().unwrap(),
        )
        .unwrap();
        app = crate::app::tests::app_of_path(&app.config.target);
        assert!(app.features.map.current() && !app.features.complete);
        app.select(Selection::Features);
        let screen = text(&render(&mut app, 160, 40));
        assert!(!screen.contains("functions no feature ran"), "{screen}");
        assert!(
            screen.contains("its units not known (map incomplete)"),
            "{screen}"
        );
        app.select(Selection::Feature("gzip".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("(not known — the map has no usable notes for it)"),
            "{screen}"
        );
        assert!(!screen.contains("Only this feature runs"), "{screen}");
        app.select(Selection::Feature("help".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("it did not exit (signal 11) — it cannot be a check"),
            "{screen}"
        );
        assert!(!screen.contains("compares little"), "{screen}");
        app.select(Selection::Function(
            "src/zopfli/katajainen.c".into(),
            "ZopfliLengthLimitedCodeLengths".into(),
        ));
        let screen = text(&render(&mut app, 160, 40));
        assert!(!screen.contains("Run by none"), "{screen}");
        let (help, _) = help_rows(160, false, None);
        // Wrapped lines, read as one text.
        let help: String = help
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for words in [
            "see its units: they differ",
            harness_core::features::SkipReason::CSideUnstable.words(),
            "[[scenario]] table per",
        ] {
            assert!(help.contains(words), "{words}\n{help}");
        }
    }

    #[test]
    fn features_start_with_a_map_then_a_re_check() {
        let mut app = zopfli_with_features("feat-next", FEATURES_TOML);
        let (step, act) = app.next_step().expect("a next step");
        assert!(step.starts_with("Map the features"), "{step}");
        assert_eq!(act, Some(crate::app::Act::MapFeatures));
        let items = app.menu_items();
        let map = items
            .iter()
            .find(|i| i.label == "Map the features")
            .expect("the project menu offers it");
        assert!(map.greyed.is_none(), "{:?}", map.greyed);
        let argv = crate::app::tests::strs(&map.pending.as_ref().unwrap().argv);
        assert_eq!(
            argv,
            [
                crate::app::tests::HARNESS,
                "--json",
                "features",
                "map",
                &format!("--target={}", app.config.target.display())
            ]
        );
        assert_eq!(
            crate::menu::recommended(
                &items,
                &Selection::Project,
                Some(crate::app::Act::MapFeatures),
                None
            ),
            items
                .iter()
                .position(|i| i.label == "Map the features")
                .unwrap()
        );
        // Mapped: u001 has Rust, and its verdict predates the features.
        write_current_map(&app);
        app = crate::app::tests::app_of_path(&app.config.target);
        assert!(app.features.map.current());
        let (step, act) = app.next_step().expect("a next step");
        assert!(step.starts_with("Re-check u001-katajainen"), "{step}");
        assert_eq!(act, None);
        // The unit's screen: the marker on its verdict line, the features line.
        app.select(Selection::Unit("u001-katajainen".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("your features: not checked on this unit yet"),
            "{screen}"
        );
        assert!(
            screen.contains("1 of your features runs it · 1 not re-checked"),
            "{screen}"
        );
        // The Features view: gzip needs a re-check; help is all C.
        app.select(Selection::Features);
        let screen = text(&render(&mut app, 160, 40));
        // Every unit with Rust counts: help needs the re-check too (§8.2 F3).
        assert!(
            screen.contains("↻ Compress to gzip  needs a re-check"),
            "{screen}"
        );
        assert!(
            screen.contains("2 units · 1 not re-checked · 1 still C"),
            "{screen}"
        );
        assert!(
            screen.contains("↻ Show the help  needs a re-check"),
            "{screen}"
        );
        assert!(screen.contains("1 unit · 1 still C"), "{screen}");
        assert!(screen.contains("Your features ran 2 of the"), "{screen}");
        // Review C3: help touches only a C unit, yet needs the re-check of
        // u001 — its View names it, a link; the summary counts it.
        app.select(Selection::Feature("help".into()));
        let mut links = Vec::new();
        let lines = feature_view(&app, "help", 160, &mut links);
        let screen: String = lines.iter().map(|l| format!("{l}\n")).collect();
        assert!(
            screen.contains("Your features are not checked on this unit yet — Re-check it:"),
            "{screen}"
        );
        assert!(
            links
                .iter()
                .any(|(_, sel)| *sel == Selection::Unit("u001-katajainen".into())),
            "{links:?}"
        );
        app.select(Selection::Project);
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Features: 2 — 2 need a re-check"),
            "{screen}"
        );
        assert!(screen.contains("1 unit not re-checked"), "{screen}");

        // §8.4/§8.5: the dialogs say what the features do.
        let words = |app: &App, act, unit, attempt| {
            let p = app.act_argv(act, unit, attempt, None).expect("pending");
            app.dialog_words(&p).1.join("\n")
        };
        let recheck = words(&app, crate::app::Act::Verify, Some("u001-katajainen"), None);
        assert!(
            recheck.contains("It also runs your 2 feature scenarios"),
            "{recheck}"
        );
        assert!(
            recheck.contains("1 of your features runs it · 1 not re-checked"),
            "{recheck}"
        );
        let migrate = {
            let p = app
                .act_argv(
                    crate::app::Act::Modify,
                    Some("u001-katajainen"),
                    Some("a-d6b377fb9257"),
                    Some("keep it"),
                )
                .expect("pending");
            app.dialog_words(&p).1.join("\n")
        };
        assert!(
            migrate.contains("Each judged turn also runs your 2 feature scenarios."),
            "{migrate}"
        );
        let accept = words(
            &app,
            crate::app::Act::Accept,
            Some("u001-katajainen"),
            Some("a-ef81857896e5"),
        );
        assert!(
            accept.contains("Your features that run it: Compress to gzip."),
            "{accept}"
        );
        let scan = words(&app, crate::app::Act::Scan, None, None);
        assert!(
            scan.contains("A change in the C makes the features map out of date."),
            "{scan}"
        );
    }

    /// §7.2 step 3 and §8.5: the editor dialog's button names the editor;
    /// the menu focuses what the Features row's state asks for.
    #[test]
    fn the_features_menu_and_its_dialog_say_what_opens() {
        let mut app =
            crate::app::tests::app_of_without_features("targets/zopfli", "feat-openlabel");
        app.select(Selection::Features);
        app.open_menu();
        let crate::app::Mode::Menu(menu) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert!(
            menu.items[menu.focus]
                .label
                .starts_with("Write your features file"),
            "{:?}",
            menu.items[menu.focus]
        );
        app.mode = crate::app::Mode::Normal;
        app.start_features_edit();
        let crate::app::Mode::Dialog(c) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        let name = crate::app::features_edit::editor_name(&app.features_editor());
        assert!(
            c.dialog
                .buttons
                .iter()
                .any(|b| b.label == format!("Open {name}")),
            "{:?}",
            c.dialog.buttons
        );
        // With an error, Edit is focused.
        let mut app = zopfli_with_features("feat-focus-error", "schema_version = 1\nnope = 1\n");
        app.select(Selection::Features);
        app.open_menu();
        let crate::app::Mode::Menu(menu) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert!(
            menu.items[menu.focus]
                .label
                .starts_with("Edit the features file"),
            "{:?}",
            menu.items[menu.focus]
        );
    }

    #[test]
    fn a_features_file_with_an_error_is_a_row_and_a_sentence_not_a_failure() {
        let mut app = zopfli_with_features("feat-invalid", "schema_version = 1\nnope = 1\n");
        assert!(matches!(app.features.group, featmap::Group::Invalid(_)));
        let (glyph, _) = app.node_label(&Selection::Features);
        assert_eq!(glyph, "⚠");
        app.select(Selection::Features);
        let screen = text(&render(&mut app, 120, 30));
        assert!(screen.contains("Features (error)"), "{screen}");
        assert!(screen.contains("unknown key \"nope\""), "{screen}");
        assert!(screen.contains("Re-checks and"), "{screen}");
        assert!(
            !app.menu_items()
                .iter()
                .any(|i| i.label == "Map the features"),
            "nothing to map"
        );
    }

    #[test]
    fn a_unit_no_feature_reaches_says_so_only_from_a_current_complete_map() {
        let mut app = zopfli_with_features("feat-none-runs", FEATURES_TOML);
        app.select(Selection::Unit("u-cache".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("Which features run it: not known"),
            "{screen}"
        );
        write_current_map(&app);
        let mut app = crate::app::tests::app_of_path(&app.config.target);
        app.select(Selection::Unit("u-cache".into()));
        let screen = text(&render(&mut app, 160, 40));
        assert!(
            screen.contains("None of your features runs this unit's functions"),
            "{screen}"
        );
    }
}
