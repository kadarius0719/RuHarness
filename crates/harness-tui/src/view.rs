//! Drawing (docs/TUI-DESIGN.md §3): the rail, the function pairs (two
//! columns padded to equal height with filler lines, or stacked below
//! [`WIDE_MIN_COLUMNS`]), the verdict strip, the run panel, the status line
//! and the overlays. Every piece of target-, model- or harness-derived text
//! goes through the display filter ([`crate::display`]) before it reaches
//! the terminal; widths are measured per grapheme exactly as ratatui draws
//! them; only the visible rows of the pairs panel are built; the view never
//! reads the ledger itself.

use crate::app::{
    attempt_tags, shell_line, short_id, Act, App, CodeLine, Focus, LayoutMode, Mode, PairView,
    Shown, Tone,
};
use crate::display::{self, Sanitizer};
use crate::highlight::{Class, Pieces};
use crate::model::{ProvenanceView, UnitView};
use harness_core::status::VerdictState;
use ratatui::buffer::CellWidth;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Below this many columns (measured from the terminal) the pairs stack and
/// the rail becomes a top line, unless `--layout split` forbids it.
pub const WIDE_MIN_COLUMNS: u16 = 110;
/// Rail width in the wide layout.
const RAIL_COLUMNS: u16 = 32;
/// Run panel height, borders included.
const RUN_ROWS: u16 = 9;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
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

/// Every grapheme of already-filtered `text` with the cells ratatui draws
/// it in (`Span::styled_graphemes` + `CellWidth`, as `Paragraph` renders),
/// so a padded column is exactly as wide on screen as measured here; `f`
/// returns `false` to stop.
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

fn width_of(text: &str) -> usize {
    let mut n = 0;
    graphemes(text, |_, w| {
        n += w;
        true
    });
    n
}

/// Already-filtered `text` cut into rows of at most `width` cells (never
/// splitting a grapheme; a row holds at least one).
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

/// Highlighted raw pieces of one line → spans at most `width` columns wide
/// (tabs expanded from the code's first column, controls as `?`, the raw
/// line cut at [`display::MAX_LINE_BYTES`] on a char boundary). Returns the
/// spans and the columns used.
fn fit(pieces: &Pieces, width: usize) -> (Vec<Span<'static>>, usize) {
    let mut san = Sanitizer::default();
    let mut budget = display::MAX_LINE_BYTES;
    let mut col = 0;
    let mut spans = Vec::new();
    for (class, raw) in pieces {
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
        let mut full = false;
        graphemes(&text, |g, w| {
            if col + w > width {
                full = true;
                return false;
            }
            piece.push_str(g);
            col += w;
            true
        });
        if !piece.is_empty() {
            spans.push(Span::styled(piece, class_style(*class)));
        }
        if full {
            break;
        }
    }
    (spans, col)
}

/// The spans of one cell of a side: gutter + code, a link, a note, or the
/// filler of the shorter side. Always exactly `width` columns.
fn cell(line: Option<&CodeLine>, gutter: usize, width: usize) -> Vec<Span<'static>> {
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
            let (code, cw) = fit(pieces, width.saturating_sub(gw));
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

/// Row `r` (0-based, `< pair_height`) of one pair; `(gc, gr)` are its two
/// sides' gutters (computed once per pair, not per row).
fn pair_row(
    p: &PairView,
    (gc, gr): (usize, usize),
    r: usize,
    width: usize,
    wide: bool,
) -> Line<'static> {
    let title = Style::default().add_modifier(Modifier::BOLD);
    let last = pair_height(p, wide) - 1;
    if r == last {
        return Line::from(Span::styled("─".repeat(width), dim()));
    }
    if wide {
        let left = (width.saturating_sub(3)) / 2;
        let right = width.saturating_sub(3 + left);
        if r == 0 {
            let lt = safe(&p.c_title, left);
            let lw = width_of(&lt);
            return Line::from(vec![
                Span::styled(lt, title),
                Span::raw(" ".repeat(left.saturating_sub(lw))),
                Span::styled(" ⇄ ", dim()),
                Span::styled(safe(&p.rust_title, right), title),
            ]);
        }
        let i = r - 1;
        let mut spans = cell(p.c.get(i), gc, left);
        spans.push(Span::styled(" │ ", dim()));
        spans.extend(cell(p.rust.get(i), gr, right));
        return Line::from(spans);
    }
    let heading = |lead: &'static str, text: &str| {
        Line::from(vec![
            Span::styled(lead, dim()),
            Span::styled(safe(text, width.saturating_sub(5)), title),
        ])
    };
    let c_end = 1 + p.c.len();
    if r == 0 {
        heading("C    ", &p.c_title)
    } else if r < c_end {
        Line::from(cell(p.c.get(r - 1), gc, width))
    } else if r == c_end {
        heading("Rust ", &p.rust_title)
    } else {
        Line::from(cell(p.rust.get(r - c_end - 1), gr, width))
    }
}

/// The pairs panel's rows `[from, from + height)`, built only for those
/// rows (a frame is drawn every 60 ms), plus the first row of each pair and
/// the total row count.
pub fn pair_window(
    pairs: &[PairView],
    width: usize,
    wide: bool,
    from: usize,
    height: usize,
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
            rows.push(pair_row(&pairs[i], g, local, width, wide));
            row += 1;
        } else {
            i += 1;
            gutters = None;
        }
    }
    (rows, starts, total)
}

fn is_wide(app: &App, columns: u16) -> bool {
    match app.config.layout {
        LayoutMode::Split => true,
        LayoutMode::Stacked => false,
        LayoutMode::Auto => columns >= WIDE_MIN_COLUMNS,
    }
}

/// The unit's glyph and short state in the rail.
fn unit_glyph(unit: &UnitView) -> (&'static str, Style, String) {
    let r = &unit.report;
    if let Some(h) = &r.write_in_flight {
        return (
            "⟳",
            Style::default().fg(Color::Yellow),
            format!("writing: {}", h.command),
        );
    }
    if let Some(id) = &r.promotion_interrupted {
        return (
            "↺",
            Style::default().fg(Color::Yellow),
            format!("promotion of {} interrupted", short_id(id)),
        );
    }
    if r.contradiction {
        return (
            "!",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            "CONTRADICTION".into(),
        );
    }
    match (&r.verdict.state, r.verdict.green) {
        (VerdictState::Unreadable, _) => (
            "?",
            Style::default().fg(Color::Red),
            "verdict unreadable".into(),
        ),
        (VerdictState::Missing, _) => ("○", dim(), r.status.clone()),
        (VerdictState::Present, Some(false)) => {
            ("✗", Style::default().fg(Color::Red), "RED".into())
        }
        _ if !r.verdict.stale.is_empty() || !r.source_fresh => {
            ("◐", Style::default().fg(Color::Yellow), "stale".into())
        }
        _ => ("●", Style::default().fg(Color::Green), r.status.clone()),
    }
}

fn provenance_text(unit: &UnitView) -> String {
    match &unit.provenance {
        ProvenanceView::None if matches!(unit.report.status.as_str(), "verified" | "merged") => {
            "provenance unknown".into()
        }
        ProvenanceView::None => "no provenance".into(),
        ProvenanceView::Pipeline(id) => format!("* pipeline {}", short_id(id)),
        ProvenanceView::Ambiguous(ids) => format!("ambiguous provenance ({})", ids.len()),
        ProvenanceView::Steered(id) => format!("*s promoted from steer attempt {}", short_id(id)),
        ProvenanceView::Human { attempt, origin } if attempt == origin => {
            format!("*h promoted from a hand edit {}", short_id(origin))
        }
        ProvenanceView::Human { attempt, origin } => format!(
            "*h promoted from {} (a steer of hand edit {})",
            short_id(attempt),
            short_id(origin)
        ),
    }
}

fn outcome_style(outcome: &str) -> Style {
    match outcome {
        "green" => Style::default().fg(Color::Green),
        "in-progress" => Style::default().fg(Color::Yellow),
        _ => Style::default().fg(Color::Red),
    }
}

/// The first index of a `rows`-long window over `len` items that keeps
/// `sel` in view.
fn window(len: usize, rows: usize, sel: usize) -> usize {
    if len <= rows || rows == 0 {
        return 0;
    }
    sel.saturating_sub(rows / 2).min(len - rows)
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

fn rail_lines(app: &App, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(match &app.snapshot.facts_state {
        Some(f) if f.stale == 0 => Line::from(Span::styled(
            clip(&format!("facts fresh ({} files)", f.files), width),
            Style::default().fg(Color::Green),
        )),
        Some(f) => Line::from(Span::styled(
            clip(&format!("facts STALE ({} files)", f.stale), width),
            Style::default().fg(Color::Yellow),
        )),
        None => Line::from(Span::styled(
            safe(app.snapshot.note.as_deref().unwrap_or("no facts"), width),
            Style::default().fg(Color::Yellow),
        )),
    });
    // Both lists are windowed to the rail, each keeping its cursor in view.
    let n_units = app.snapshot.units.len();
    let n_items = app.unit_view().map_or(0, |u| u.attempts.len() + 1);
    let fixed = 2 + if n_items > 0 { 2 } else { 0 };
    let avail = height.saturating_sub(fixed);
    let (unit_rows, item_rows) = if n_units + n_items <= avail {
        (n_units, n_items)
    } else {
        // The attempts take what they need of their half; the units get the
        // rest (never an empty row while units are hidden).
        let items = n_items.min(avail.saturating_sub(n_units.min((avail / 2).max(1))));
        (avail.saturating_sub(items).min(n_units), items)
    };
    let unit_start = window(n_units, unit_rows, app.unit);
    let more = |hidden: usize| format!(" (+{hidden})");
    lines.push(Line::from(Span::styled(
        clip(
            &format!(
                "units {}/{}  (J/K){}",
                if n_units == 0 { 0 } else { app.unit + 1 },
                n_units,
                if unit_rows < n_units {
                    more(n_units - unit_rows)
                } else {
                    String::new()
                }
            ),
            width,
        ),
        dim(),
    )));
    for (i, unit) in app
        .snapshot
        .units
        .iter()
        .enumerate()
        .skip(unit_start)
        .take(unit_rows)
    {
        let (glyph, style, state) = unit_glyph(unit);
        let mark = if i == app.unit { "▸" } else { " " };
        let mut name = Style::default();
        if i == app.unit {
            name = name.add_modifier(Modifier::BOLD);
        }
        lines.push(Line::from(clipped(
            vec![
                Span::raw(mark.to_string()),
                Span::styled(glyph.to_string(), style),
                Span::styled(format!(" {} {}", unit.unit.id, state), name),
            ],
            width,
        )));
    }
    if let Some(unit) = app.unit_view() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            safe(&format!("{} (Tab, Enter)", unit.unit.id), width),
            dim(),
        )));
        let item_start = window(n_items, item_rows, app.rail);
        let mut items: Vec<(Option<&str>, Vec<Span<'static>>)> = vec![(
            None,
            vec![
                Span::raw("crate  "),
                Span::styled(provenance_text(unit), dim()),
            ],
        )];
        for a in &unit.attempts {
            let mut spans = vec![
                Span::raw(format!("{} ", short_id(&a.record.id))),
                Span::styled(a.record.outcome.clone(), outcome_style(&a.record.outcome)),
            ];
            for tag in attempt_tags(unit, a) {
                spans.push(Span::styled(format!(" {tag}"), dim()));
            }
            items.push((Some(a.record.id.as_str()), spans));
        }
        for (i, (id, spans)) in items
            .into_iter()
            .enumerate()
            .skip(item_start)
            .take(item_rows)
        {
            let shown = match (&app.shown, id) {
                (Shown::Crate, None) => true,
                (Shown::Attempt(s), Some(id)) => s == id,
                _ => false,
            };
            let mut row = vec![Span::raw(if shown { "▸ " } else { "  " })];
            row.extend(spans);
            let mut line = Line::from(clipped(row, width));
            if i == app.rail && app.focus == Focus::Rail {
                line = line.style(Style::default().add_modifier(Modifier::REVERSED));
            } else if i == app.rail {
                line = line.style(Style::default().add_modifier(Modifier::UNDERLINED));
            }
            lines.push(line);
        }
    }
    lines
}

fn header_line(app: &App, width: usize) -> Line<'static> {
    let Some(unit) = app.unit_view() else {
        return Line::from(Span::styled(
            safe(app.snapshot.note.as_deref().unwrap_or("no units"), width),
            Style::default().fg(Color::Yellow),
        ));
    };
    let mut spans = vec![Span::styled(
        unit.unit.id.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    match app.shown_attempt() {
        Some(a) => {
            let r = &a.record;
            spans.push(Span::raw(format!(" · attempt {} ", r.id)));
            spans.push(Span::styled(r.outcome.clone(), outcome_style(&r.outcome)));
            spans.push(Span::styled(
                format!(" · {} {}", r.provider, r.model),
                dim(),
            ));
            let tags = attempt_tags(unit, a);
            if !tags.is_empty() {
                spans.push(Span::styled(format!(" · {}", tags.join(" ")), dim()));
            }
            if let Some(note) = &r.steer_note {
                spans.push(Span::styled(format!(" · note: {note}"), dim()));
            }
            if let Some(note) = &r.note {
                spans.push(Span::styled(format!(" · note: {note}"), dim()));
            }
        }
        None => {
            spans.push(Span::raw(" · unit crate · "));
            spans.push(Span::raw(unit.report.status.clone()));
            spans.push(Span::styled(format!(" · {}", provenance_text(unit)), dim()));
        }
    }
    Line::from(clipped(spans, width))
}

/// The verdict strip in `rows` rows: FAILED checks first, so a cut strip
/// never hides a failure, and a `+N` marker for whatever did not fit.
fn verdict_lines(app: &App, width: usize, rows: usize) -> Vec<Line<'static>> {
    let Some(v) = app.shown_verdict() else {
        return vec![Line::from(Span::styled(
            clip("[verdict] none for what is shown", width),
            dim(),
        ))];
    };
    let rows = rows.max(1);
    let lead = if v.green {
        "[verdict] GREEN "
    } else {
        "[verdict] RED "
    };
    let mut chips: Vec<(String, Style)> = vec![(
        lead.to_string(),
        if v.green {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        },
    )];
    let order = v
        .checks
        .iter()
        .filter(|c| !c.passed)
        .chain(v.checks.iter().filter(|c| c.passed));
    for check in order {
        chips.push((
            display::line(&format!(
                "{} {} ",
                if check.passed { "✓" } else { "✗" },
                check.name
            )),
            if check.passed {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ));
    }
    // Lay the chips out; when they overflow, keep room for the marker.
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
            lines.push(Vec::new());
            used = 0;
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

fn run_lines(app: &App, width: usize, height: usize) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let title = match (&app.run, app.running) {
        (_, true) => " run: running — x cancels ".to_string(),
        (Some(run), false) => format!(" run: {} ", run.exit.as_deref().unwrap_or("ended")),
        (None, false) => " run ".to_string(),
    };
    let mut footer: Vec<Line<'static>> = app
        .awaiting
        .iter()
        .map(|aw| {
            let who = aw.attempt.as_deref().map(short_id).unwrap_or_default();
            let text = if aw.response_present {
                format!("response present for {who} — R resumes (asks first)")
            } else {
                format!("{who} awaits a response at {}", aw.path.display())
            };
            Line::from(Span::styled(
                safe(&text, width),
                Style::default().fg(Color::Yellow),
            ))
        })
        .collect();
    match &app.run {
        None => lines.push(Line::from(Span::styled(
            clip(
                "no command yet — a accept · m modify · e hand edit · r retry · R resume",
                width,
            ),
            dim(),
        ))),
        Some(run) => {
            lines.push(Line::from(Span::styled(
                safe(&format!("$ {}", shell_line(&run.argv)), width),
                dim(),
            )));
            // Only the tail is shown: build only the tail.
            let room = height.saturating_sub(1 + footer.len() + usize::from(run.exit.is_some()));
            let skip = run.lines.len().saturating_sub(room);
            lines.extend(
                run.lines
                    .iter()
                    .skip(skip)
                    .map(|l| Line::from(Span::styled(safe(&l.text, width), tone(l.tone)))),
            );
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
    lines.append(&mut footer);
    // Keep the argv, then the tail.
    if lines.len() > height && height > 1 {
        let tail = lines.split_off(lines.len() - (height - 1));
        lines.truncate(1);
        lines.extend(tail);
    }
    (title, lines)
}

const HELP: &[(&str, &str)] = &[
    ("j / k", "scroll the pairs (rail focus: move the cursor)"),
    ("] f / [ f", "next / previous function pair"),
    ("J / K", "next / previous unit"),
    ("Tab", "rail focus on / off"),
    ("Enter", "show the attempt under the rail cursor"),
    ("PgDn / PgUp", "page the pairs (and overlays)"),
    (
        "d",
        "diff the shown attempt's Rust with the provenance attempt's",
    ),
    ("v", "verdict detail"),
    ("a", "Accept: promote the shown green attempt"),
    (
        "m",
        "Modify: a steer note → a new attempt seeded from the shown one",
    ),
    (
        "e",
        "hand edit in $VISUAL / $EDITOR → a labelled human attempt",
    ),
    (
        "E",
        "the latest kept hand edit again (never lost: n keeps it, D at its prompt discards it)",
    ),
    ("r", "retry the shown attempt's run"),
    ("R", "resume a hand-off that awaits a response"),
    ("x", "cancel the running command (SIGINT)"),
    ("g", "re-read the ledger"),
    ("q / Q", "quit (while a command runs: q lets it finish, x stops it)"),
    (
        "",
        "Every act shows its exact command and asks y/n first; a y typed ahead of the prompt, or pasted, never answers it.",
    ),
];

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let pct = |n: u16, p: u16| u16::try_from(u32::from(n) * u32::from(p) / 100).unwrap_or(n);
    let w = pct(area.width, pct_w).max(20).min(area.width);
    let h = pct(area.height, pct_h).max(6).min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// An overlay of pre-wrapped rows (each at most the inner width), scrolled
/// by ROWS: `scroll` is clamped to what the rows need and returned, only
/// the visible page is copied, and `prompt` (if any) sits on the bottom
/// border, always visible, after "more below — `more_key`" when rows are
/// hidden below.
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
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(safe(title, area.width.saturating_sub(4) as usize));
    let hint = more_key.map(|k| format!(" — {k}")).unwrap_or_default();
    let bottom = match (prompt, more) {
        (Some(p), true) => Some(format!(" more below{hint} · {p} ")),
        (Some(p), false) => Some(format!(" {p} ")),
        (None, true) => Some(format!(" more below{hint} ")),
        (None, false) => None,
    };
    if let Some(b) = bottom {
        block = block.title_bottom(Line::from(Span::styled(
            clip(&b, area.width.saturating_sub(4) as usize),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    let visible: Vec<Line<'static>> = rows.iter().skip(scroll).take(page).cloned().collect();
    frame.render_widget(Paragraph::new(visible).block(block), area);
    scroll
}

/// The scroll of a text-input overlay whose first `input_rows` rows are the
/// input (the cursor on the last): the cursor row stays visible, with as
/// much of what follows it as fits.
fn follow_cursor(input_rows: usize, total_rows: usize, rect: Rect) -> usize {
    let page = rect.height.saturating_sub(2) as usize;
    let trailing = total_rows.saturating_sub(input_rows);
    input_rows.saturating_sub(page.saturating_sub(trailing).max(1))
}

/// Raw text → filtered, wrapped rows of `width`, all in `style`.
fn wrapped(raw: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    hard_wrap(&display::line(raw), width)
        .into_iter()
        .map(|row| Line::from(Span::styled(row, style)))
        .collect()
}

fn draw_overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.mode.clone() {
        Mode::Normal => {}
        Mode::Help { scroll } => {
            let rect = centered(area, 80, 80);
            let inner = rect.width.saturating_sub(2) as usize;
            let rows: Vec<Line<'static>> = HELP
                .iter()
                .flat_map(|(k, v)| wrapped(&format!("{k:<12} {v}"), inner, Style::default()))
                .collect();
            let shown = overlay(
                frame,
                rect,
                " keys ",
                &rows,
                scroll,
                Some("any other key closes"),
                Some("j"),
            );
            app.mode = Mode::Help { scroll: shown };
        }
        Mode::Verdict { selected, scroll } => {
            let Some(v) = app.shown_verdict() else {
                return;
            };
            let rect = centered(area, 85, 80);
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
                rows.extend(wrapped(&format!("{mark} {}", c.name), inner, style));
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
                " verdict ",
                &rows,
                scroll,
                Some("j/k check · Esc"),
                Some("PgDn"),
            );
            app.mode = Mode::Verdict {
                selected,
                scroll: shown,
            };
        }
        Mode::Diff { scroll, title, .. } => {
            let rect = centered(area, 90, 85);
            let inner = rect.width.saturating_sub(2) as usize;
            if app.diff_rows.as_ref().map(|(w, _)| *w) != Some(inner) {
                let Mode::Diff { lines, .. } = &app.mode else {
                    return;
                };
                let rows = lines
                    .iter()
                    .flat_map(|l| {
                        let style = if l.starts_with("+++") || l.starts_with("---") {
                            Style::default().add_modifier(Modifier::BOLD)
                        } else if l.starts_with('+') {
                            Style::default().fg(Color::Green)
                        } else if l.starts_with('-') {
                            Style::default().fg(Color::Red)
                        } else if l.starts_with("@@") {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        };
                        wrapped(l, inner, style)
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
                &format!(" diff: {title} "),
                rows,
                scroll,
                Some("Esc"),
                Some("j/PgDn"),
            );
            if let Mode::Diff { scroll, .. } = &mut app.mode {
                *scroll = shown;
            }
        }
        Mode::Note { input } => {
            let who = app
                .shown_attempt()
                .map(|a| a.record.id.clone())
                .unwrap_or_default();
            let rect = centered(area, 80, 40);
            let inner = rect.width.saturating_sub(2) as usize;
            let mut rows = wrapped(&format!("{input}▏"), inner, Style::default());
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
                &format!(" Modify: steer note for {who} "),
                &rows,
                at,
                Some("Enter continue · Esc cancel"),
                None,
            );
        }
        Mode::EditNote { input, stage, .. } => {
            let rect = centered(area, 80, 40);
            let inner = rect.width.saturating_sub(2) as usize;
            let mut rows = wrapped(&format!("{input}▏"), inner, Style::default());
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
        Mode::Confirm(pending) => {
            let w = centered(area, 85, 40).width;
            let inner = w.saturating_sub(2) as usize;
            // Filtered but NOT cut at the display filter's 4 KiB: `y` needs
            // every byte of the command on screen (it scrolls).
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let mut rows: Vec<Line<'static>> = hard_wrap(
                &Sanitizer::default().push(&shell_line(&pending.argv)),
                inner,
            )
            .into_iter()
            .map(|row| Line::from(Span::styled(row, bold)))
            .collect();
            let hand_edit = pending.act == Act::HandEdit;
            if hand_edit {
                rows.push(Line::from(""));
                rows.extend(wrapped(
                    "the edit is staged as exactly src/logic.rs + src/ffi.rs; n keeps it for \
                     later (E), D discards it",
                    inner,
                    dim(),
                ));
            }
            // As tall as the command needs, up to the screen minus the
            // status line (a refused `y` explains itself there).
            let h = u16::try_from(rows.len() + 2)
                .unwrap_or(u16::MAX)
                .max(4)
                .min(area.height.saturating_sub(1));
            let rect = Rect {
                x: area.x + (area.width - w) / 2,
                y: area.y
                    + (area.height.saturating_sub(1) - h.min(area.height.saturating_sub(1))) / 2,
                width: w,
                height: h,
            };
            let page = h.saturating_sub(2) as usize;
            let total = rows.len();
            let prompt = if hand_edit {
                "y run it · n keep · D discard"
            } else {
                "y run it · n cancel"
            };
            let shown = overlay(
                frame,
                rect,
                &format!(" {} — run this? ", pending.act.label()),
                &rows,
                app.confirm_scroll,
                Some(prompt),
                Some("j"),
            );
            app.confirm_scroll = shown;
            // `y` needs the whole command to have been on screen.
            app.confirm_seen = app.confirm_seen || shown + page >= total;
        }
        Mode::QuitConfirm => {
            let rows = vec![
                Line::from("A command is running."),
                Line::from("q  quit and let it finish (it runs on to its end)"),
                Line::from("x  stop it (SIGINT) and quit"),
                Line::from("Esc  stay"),
            ];
            let prompt = if app.confirm_armed {
                "ready: q or x · Esc stays"
            } else {
                "reading… · Esc stays"
            };
            overlay(
                frame,
                centered(area, 70, 30),
                " quit? ",
                &rows,
                0,
                Some(prompt),
                None,
            );
            // Drawn whole: the event loop may arm it.
            app.confirm_seen = true;
        }
    }
}

fn status_line(app: &App, width: usize) -> Line<'static> {
    match &app.notice {
        Some(n) => Line::from(Span::styled(
            safe(n, width),
            Style::default().fg(Color::Yellow),
        )),
        None => Line::from(Span::styled(
            clip(
                "? keys · Tab rail · ]f/[f pairs · a accept · m modify · e edit · r retry · R \
                 resume · x cancel · d diff · v verdict · g reload · q quit",
                width,
            ),
            dim(),
        )),
    }
}

/// Draw the whole cockpit, and record the pairs layout for the keys.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let wide = is_wide(app, area.width);
    let [main, run, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(RUN_ROWS.min(area.height.saturating_sub(5)).max(3)),
            Constraint::Length(1),
        ])
        .areas(area);
    let (rail_area, right) = if wide {
        let [rail, right] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(RAIL_COLUMNS), Constraint::Min(20)])
            .areas(main);
        (Some(rail), right)
    } else {
        (None, main)
    };
    let verdict_rows: u16 = if wide { 2 } else { 1 };
    let top_rows = if wide { 1 } else { 2 };
    let [top, pairs_area, verdict_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_rows),
            Constraint::Min(1),
            Constraint::Length(verdict_rows),
        ])
        .areas(right);

    if let Some(rail) = rail_area {
        let block = Block::default().borders(Borders::RIGHT).border_style(dim());
        let inner = block.inner(rail);
        frame.render_widget(block, rail);
        frame.render_widget(
            Paragraph::new(rail_lines(app, inner.width as usize, inner.height as usize)),
            inner,
        );
        frame.render_widget(Paragraph::new(header_line(app, top.width as usize)), top);
    } else {
        // The rail collapses to a top line (plus the header).
        let w = top.width as usize;
        let unit_part = app
            .unit_view()
            .map(|u| {
                let (glyph, _, state) = unit_glyph(u);
                format!(
                    "unit {}/{} {glyph} {} {state} · rail {}/{} (Tab) · ",
                    app.unit + 1,
                    app.snapshot.units.len(),
                    u.unit.id,
                    app.rail,
                    u.attempts.len()
                )
            })
            .unwrap_or_default();
        let facts = match &app.snapshot.facts_state {
            Some(f) if f.stale == 0 => "facts fresh".to_string(),
            Some(f) => format!("facts STALE ({})", f.stale),
            None => "no facts".to_string(),
        };
        let mut rail_row = Line::from(Span::raw(safe(&format!("{unit_part}{facts}"), w)));
        if app.focus == Focus::Rail {
            let item = app
                .unit_view()
                .and_then(|u| {
                    let a = app.rail.checked_sub(1).and_then(|i| u.attempts.get(i))?;
                    let mut s =
                        format!(" ▸ cursor: {} {}", short_id(&a.record.id), a.record.outcome);
                    for tag in attempt_tags(u, a) {
                        s.push(' ');
                        s.push_str(&tag);
                    }
                    Some(s)
                })
                .unwrap_or_else(|| " ▸ cursor: crate".into());
            rail_row = Line::from(Span::styled(
                safe(&format!("{unit_part}{item}"), w),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }
        frame.render_widget(Paragraph::new(vec![rail_row, header_line(app, w)]), top);
    }

    let width = pairs_area.width as usize;
    let height = pairs_area.height as usize;
    let (_, _, total) = pair_window(&app.pairs, width, wide, 0, 0);
    app.scroll = app.scroll.min(total.saturating_sub(1));
    let (visible, starts, total) = pair_window(&app.pairs, width, wide, app.scroll, height);
    app.layout = crate::app::Layout {
        pair_rows: starts,
        total_rows: total,
        page: height,
    };
    frame.render_widget(Paragraph::new(visible), pairs_area);
    frame.render_widget(
        Paragraph::new(verdict_lines(
            app,
            verdict_area.width as usize,
            verdict_rows as usize,
        )),
        verdict_area,
    );

    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(run);
    let (title, lines) = run_lines(app, inner.width as usize, inner.height as usize);
    frame.render_widget(block.title(title), run);
    frame.render_widget(Paragraph::new(lines), inner);
    frame.render_widget(
        Paragraph::new(status_line(app, status.width as usize)),
        status,
    );

    draw_overlay(frame, app, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Config, Pending, RunLine};
    use crate::highlight::Class;
    use crate::model::Snapshot;
    use crate::spawn::ChildMsg;
    use crate::testutil::{scratch_target, READ_SCALEFACTORS};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn app(tag: &str, layout: LayoutMode) -> App {
        let target = scratch_target(READ_SCALEFACTORS, tag);
        let snapshot = Snapshot::load(&target).unwrap();
        App::new(
            Config {
                target,
                harness: Some(PathBuf::from("/opt/ruharness/bin/harness")),
                allow_unsandboxed: false,
                layout,
                providers: vec!["external".into()],
            },
            snapshot,
        )
    }

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

    /// The scratch copy's path appears in some texts (the run panel's
    /// argv): replaced so goldens are machine-independent.
    fn golden(name: &str, app: &App, buffer: &Buffer) {
        let got = text(buffer).replace(&app.config.target.display().to_string(), "<target>");
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

    #[test]
    fn wide_layout_golden() {
        let mut app = app("view-wide", LayoutMode::Auto);
        let buffer = render(&mut app, 140, 42);
        golden("wide.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(screen.contains("facts fresh"));
        assert!(screen.contains("⇄"), "a pair heading pairs C with Rust");
        assert!(screen.contains("[verdict] GREEN"));
        // The rail marks the provenance attempt.
        assert!(row_with(&screen, "a-13c9 green").contains('*'));
    }

    #[test]
    fn narrow_layout_golden_stacks_the_pairs() {
        let mut app = app("view-narrow", LayoutMode::Auto);
        let buffer = render(&mut app, 100, 42);
        golden("narrow.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(screen.lines().any(|l| l.starts_with("C    ")));
        assert!(!screen.contains(" ⇄ "));
        // Stacked: each pair is its C side, then its Rust side.
        let (rows, starts, _) = pair_window(&app.pairs, 100, false, 0, usize::MAX);
        let rows: Vec<String> = rows.iter().map(|l| l.to_string()).collect();
        for (i, start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(rows.len());
            let c = rows[*start..end]
                .iter()
                .position(|r| r.starts_with("C    "));
            let rust = rows[*start..end]
                .iter()
                .position(|r| r.starts_with("Rust "));
            assert!(matches!((c, rust), (Some(0), Some(r)) if r > 0), "pair {i}");
        }
        // --layout split forbids the fallback; stacked forces it.
        app.config.layout = LayoutMode::Split;
        assert!(text(&render(&mut app, 100, 42)).contains(" ⇄ "));
        app.config.layout = LayoutMode::Stacked;
        assert!(!text(&render(&mut app, 160, 42)).contains(" ⇄ "));
    }

    #[test]
    fn help_overlay_golden() {
        let mut app = app("view-help", LayoutMode::Auto);
        app.mode = Mode::Help { scroll: 0 };
        let buffer = render(&mut app, 120, 36);
        golden("help.txt", &app, &buffer);
        assert!(text(&buffer).contains("asks y/n first"));
    }

    /// Filler lines pad the shorter side; a failed check is a red chip.
    #[test]
    fn filler_lines_and_a_failed_check() {
        let mut app = app("view-fail", LayoutMode::Auto);
        app.pairs = vec![PairView {
            symbol: "f".into(),
            c_title: "f (f.c:1)".into(),
            rust_title: "f (src/ffi.rs:1)".into(),
            c: (1..=4)
                .map(|n| CodeLine::Code {
                    number: n,
                    pieces: vec![(Class::Plain, format!("c line {n}"))],
                })
                .collect(),
            rust: vec![CodeLine::Code {
                number: 1,
                pieces: vec![(Class::Plain, "rust line".into())],
            }],
        }];
        let unit = &mut app.snapshot.units[0];
        let verdict = unit.verdict.as_mut().unwrap();
        verdict.green = false;
        verdict.checks[0].passed = false;
        let failed = verdict.checks[0].name.clone();
        let buffer = render(&mut app, 140, 30);
        let screen = text(&buffer);
        for n in 2..=4 {
            let row = row_with(&screen, &format!("c line {n}"));
            assert!(row.contains("│ ~"), "filler on the Rust side: {row}");
        }
        assert!(screen.contains("[verdict] RED"));
        // The failed chip is red.
        let y = screen
            .lines()
            .position(|l| l.contains(&format!("✗ {failed}")))
            .unwrap() as u16;
        let x = screen
            .lines()
            .nth(y as usize)
            .unwrap()
            .chars()
            .take_while(|c| *c != '✗')
            .count() as u16;
        assert_eq!(buffer[(x, y)].fg, Color::Red);
    }

    /// The run panel mid-run: the argv, then the events as they arrive.
    #[test]
    fn run_panel_mid_run_golden() {
        let mut app = app("view-run", LayoutMode::Auto);
        let argv: Vec<OsString> = [
            "/opt/ruharness/bin/harness",
            "--json",
            "migrate",
            "u-lib",
            "--no-promote",
        ]
        .map(OsString::from)
        .to_vec();
        app.on_spawned(&Pending {
            act: crate::app::Act::Retry,
            argv,
            cleanup: None,
            expect_attempt: None,
        });
        let stream = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/events/migrate-green.ndjson"),
        )
        .unwrap();
        for line in stream.lines().take(8) {
            app.on_child_msg(ChildMsg::Event(crate::events::parse_line(line)));
        }
        app.on_child_msg(ChildMsg::Stderr("warning: something on stderr".into()));
        let buffer = render(&mut app, 140, 40);
        golden("run-mid.txt", &app, &buffer);
        let screen = text(&buffer);
        assert!(screen.contains("run: running"));
        // The argv stays; the tail of the events follows it.
        assert!(screen.contains("$ /opt/ruharness/bin/harness --json migrate u-lib --no-promote"));
        assert!(screen.contains("stderr"));
        assert!(screen.contains("[check] driver-shape ✓"));
        let run = app.run.as_ref().unwrap();
        assert!(run
            .lines
            .iter()
            .any(|l| l.text == "turn 1 translate → green"));
    }

    /// §2 display filter end to end: tabs land on 8-column stops of the
    /// code (after the gutter), escapes never reach the terminal.
    #[test]
    fn tabs_and_escapes_render_safely() {
        let mut app = app("view-tabs", LayoutMode::Auto);
        app.pairs = vec![PairView {
            symbol: "t".into(),
            c_title: "t (t.c:1)".into(),
            rust_title: "t \u{1b}[31m(src/ffi.rs:1)".into(),
            c: vec![
                CodeLine::Code {
                    number: 1,
                    pieces: vec![(Class::Plain, "\tif (x) {".into())],
                },
                CodeLine::Code {
                    number: 2,
                    pieces: vec![
                        (Class::Keyword, "\t\t".into()),
                        (Class::Plain, "return;".into()),
                    ],
                },
            ],
            rust: vec![CodeLine::Note("evil \u{1b}]0;title\u{7}".into())],
        }];
        let buffer = render(&mut app, 140, 30);
        let screen = text(&buffer);
        assert!(!screen.contains('\u{1b}') && !screen.contains('\u{7}'));
        let pairs_x = RAIL_COLUMNS as usize;
        let gutter = 2; // "1 "
        let col_of = |needle: &str| {
            let row = row_with(&screen, needle);
            row[..row.find(needle).unwrap()].chars().count()
        };
        assert_eq!(col_of("if (x)"), pairs_x + gutter + 8);
        assert_eq!(col_of("return;"), pairs_x + gutter + 16);
        assert!(screen.contains("evil ?]0;title?"));
    }

    #[test]
    fn a_long_line_is_cut_on_a_char_boundary() {
        let raw = format!("{}é tail", "x".repeat(display::MAX_LINE_BYTES - 1));
        let (spans, used) = fit(&vec![(Class::Plain, raw)], usize::MAX);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(used, display::MAX_LINE_BYTES - 1);
        assert!(joined.chars().all(|c| c == 'x'), "the straddling é is cut");
        // A cell is always exactly its width, filler included.
        let spans = cell(None, 3, 10);
        let w: usize = spans.iter().map(|s| width_of(&s.content)).sum();
        assert_eq!(w, 10);
    }

    #[test]
    fn overlays_render_their_argv_and_prompt() {
        let mut app = app("view-confirm", LayoutMode::Auto);
        app.mode = Mode::Confirm(Pending {
            act: crate::app::Act::Modify,
            argv: ["harness", "--json", "migrate", "u-lib", "--steer=it's done"]
                .map(OsString::from)
                .to_vec(),
            cleanup: None,
            expect_attempt: None,
        });
        let screen = text(&render(&mut app, 140, 40));
        assert!(screen.contains("Modify (steer) — run this?"));
        assert!(screen.contains("harness --json migrate u-lib '--steer=it'\\''s done'"));
        assert!(screen.contains("y run it · n cancel"));
        app.mode = Mode::Note {
            input: "prefer iter()".into(),
        };
        let screen = text(&render(&mut app, 140, 40));
        assert!(screen.contains("prefer iter()"));
        app.mode = Mode::QuitConfirm;
        assert!(text(&render(&mut app, 140, 40)).contains("x  stop it (SIGINT) and quit"));
        app.run = None;
        app.mode = Mode::Normal;
        app.run = Some(crate::app::RunPanel {
            argv: vec![OsString::from("harness")],
            lines: vec![RunLine {
                tone: crate::app::Tone::Bad,
                text: "error (locked): ledger is locked".into(),
            }],
            exit: Some("exit 1".into()),
            saw_result: true,
            expect_attempt: None,
            act: crate::app::Act::Accept,
            recorded: false,
            cleanup: None,
        });
        let screen = text(&render(&mut app, 140, 40));
        assert!(screen.contains("run: exit 1"));
        assert!(screen.contains("error (locked)"));
    }

    /// Review VIEW-1: a cut strip never hides a failure, and says how many
    /// chips it cut.
    #[test]
    fn the_verdict_strip_shows_failures_first_and_counts_the_cut() {
        let mut app = app("view-chips", LayoutMode::Auto);
        let verdict = app.snapshot.units[0].verdict.as_mut().unwrap();
        verdict.green = false;
        let last = verdict.checks.len() - 1;
        verdict.checks[last].passed = false;
        let failed = verdict.checks[last].name.clone();
        let screen = text(&render(&mut app, 100, 40));
        let strip = row_with(&screen, "[verdict] RED");
        assert!(strip.contains(&format!("✗ {failed}")), "{strip}");
        assert!(strip.contains('+'), "the cut is counted: {strip}");
    }

    /// Review VIEW-2/3 (STATE-5): overlays scroll by wrapped rows, so the
    /// end of a long detail or diff is reachable, and the scroll is clamped.
    #[test]
    fn overlays_scroll_to_the_end_of_wrapped_content() {
        let mut app = app("view-scroll", LayoutMode::Auto);
        let long: Vec<String> = (0..40)
            .map(|i| format!("+line {i} {}", "w".repeat(200)))
            .chain(["+THE END".to_string()])
            .collect();
        app.mode = Mode::Diff {
            scroll: usize::MAX,
            lines: long,
            title: "t".into(),
        };
        let screen = text(&render(&mut app, 120, 40));
        assert!(screen.contains("+THE END"), "{screen}");
        let Mode::Diff { scroll, .. } = app.mode else {
            unreachable!()
        };
        assert!(scroll < usize::MAX / 2, "clamped: {scroll}");
        // A long check detail too.
        let verdict = app.snapshot.units[0].verdict.as_mut().unwrap();
        verdict.checks[0].detail =
            (0..80).map(|i| format!("detail {i}\n")).collect::<String>() + "LAST LINE";
        app.mode = Mode::Verdict {
            selected: 0,
            scroll: usize::MAX,
        };
        assert!(text(&render(&mut app, 120, 40)).contains("LAST LINE"));
    }

    /// Review VIEW-4: a long command is shown whole before `y` counts.
    #[test]
    fn a_long_command_must_be_seen_whole_before_y() {
        let mut app = app("view-longargv", LayoutMode::Auto);
        let note = "n".repeat(1900);
        app.mode = Mode::Confirm(Pending {
            act: crate::app::Act::Modify,
            argv: ["harness", "--json", "migrate", "u-lib"]
                .map(OsString::from)
                .into_iter()
                .chain([OsString::from(format!("--steer={note}"))])
                .collect(),
            cleanup: None,
            expect_attempt: None,
        });
        let screen = text(&render(&mut app, 100, 20));
        assert!(screen.contains("more below"), "{screen}");
        assert!(!app.confirm_seen);
        for _ in 0..40 {
            app.on_key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char('j'),
            ));
            render(&mut app, 100, 20);
        }
        assert!(app.confirm_seen, "scrolled to the end");
        assert!(app.confirm_waiting());
    }

    /// Review VIEW-5: the rail is windowed; the selected unit and the rail
    /// cursor stay on screen.
    #[test]
    fn the_rail_keeps_its_cursors_in_view() {
        let target = scratch_target("targets/zopfli", "view-rail");
        let snapshot = Snapshot::load(&target).unwrap();
        let n = snapshot.units.len();
        assert!(n > 8, "zopfli has many units");
        let mut app = App::new(
            Config {
                target,
                harness: None,
                allow_unsandboxed: false,
                layout: LayoutMode::Auto,
                providers: vec!["external".into()],
            },
            snapshot,
        );
        for _ in 0..n {
            app.on_key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char('J'),
            ));
        }
        let last = app.snapshot.units[n - 1].unit.id.clone();
        let screen = text(&render(&mut app, 140, 20));
        let rail: String = screen
            .lines()
            .map(|l| l.chars().take(RAIL_COLUMNS as usize).collect::<String>() + "\n")
            .collect();
        assert!(
            // The selected unit's own row (the attempt list's heading repeats
            // the id, so that is not enough).
            rail.lines()
                .any(|l| l.starts_with('▸') && l.contains(&format!(" {last} "))),
            "{rail}"
        );
        assert!(
            rail.contains("crate"),
            "the attempt list is still shown: {rail}"
        );
    }

    /// Review VIEW-6: widths are measured per grapheme, as ratatui draws.
    #[test]
    fn widths_follow_graphemes_as_drawn() {
        for s in ["👩‍💻", "e\u{301}", "日本", "abc"] {
            let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
            buf.set_line(0, 0, &Line::from(format!("{s}|")), 10);
            let drawn = (0..10).find(|x| buf[(*x, 0)].symbol() == "|").unwrap() as usize;
            assert_eq!(width_of(s), drawn, "{s:?}");
        }
        assert_eq!(clip("👩‍💻x", 2), "👩‍💻");
    }

    /// Review VIEW-7/8: huge terminals do not overflow, and only the
    /// visible rows are built (the window equals the full layout's slice).
    #[test]
    fn huge_terminals_and_windows() {
        let mut app = app("view-huge", LayoutMode::Auto);
        app.mode = Mode::Help { scroll: 0 };
        render(&mut app, 1000, 900);
        let (all, starts, total) = pair_window(&app.pairs, 120, true, 0, usize::MAX);
        assert_eq!(all.len(), total);
        let (part, starts2, total2) = pair_window(&app.pairs, 120, true, 5, 7);
        assert_eq!((starts2, total2), (starts, total));
        assert_eq!(part.len(), 7);
        for (i, line) in part.iter().enumerate() {
            assert_eq!(line.to_string(), all[5 + i].to_string());
        }
    }

    /// Review VIEW-9: the narrow rail line keeps the attempt's tags.
    #[test]
    fn the_narrow_cursor_line_keeps_the_tags() {
        let mut app = app("view-narrow-tags", LayoutMode::Auto);
        app.focus = Focus::Rail;
        let i = app
            .unit_view()
            .unwrap()
            .attempts
            .iter()
            .position(|a| a.record.id == "a-13c941dfff95")
            .unwrap();
        app.rail = i + 1;
        let screen = text(&render(&mut app, 100, 30));
        let line = row_with(&screen, "cursor: a-13c9");
        assert!(line.contains('*'), "{line}");
    }

    fn key(app: &mut App, code: ratatui::crossterm::event::KeyCode) {
        app.on_key(ratatui::crossterm::event::KeyEvent::from(code));
    }

    /// Verification round (VIEW-NEW-1/7): a command longer than the display
    /// filter's 4 KiB cut is still shown to its last byte before it counts
    /// as seen, and the prompt never covers the status line.
    #[test]
    fn a_command_over_4_kib_is_shown_to_its_last_byte() {
        use ratatui::crossterm::event::KeyCode;
        let mut app = app("view-4k", LayoutMode::Auto);
        app.config.allow_unsandboxed = true;
        let note = "'".repeat(1990);
        app.mode = Mode::Confirm(Pending {
            act: crate::app::Act::Modify,
            argv: ["harness", "--json", "migrate", "u-lib"]
                .map(OsString::from)
                .into_iter()
                .chain([
                    OsString::from(format!("--steer={note}")),
                    OsString::from("--allow-unsandboxed"),
                ])
                .collect(),
            cleanup: None,
            expect_attempt: None,
        });
        assert!(
            shell_line(match &app.mode {
                Mode::Confirm(p) => &p.argv,
                _ => unreachable!(),
            })
            .len()
                > display::MAX_LINE_BYTES
        );
        let mut screen = String::new();
        for _ in 0..200 {
            screen = text(&render(&mut app, 120, 30));
            if app.confirm_seen {
                break;
            }
            key(&mut app, KeyCode::Char('j'));
        }
        assert!(app.confirm_seen);
        assert!(
            screen.contains("--allow-unsandboxed"),
            "the tail is on screen when seen"
        );
        // A refused `y` explains itself on the status line, uncovered.
        app.confirm_seen = false;
        app.confirm_scroll = 0;
        render(&mut app, 120, 30);
        key(&mut app, KeyCode::Char('y'));
        let screen = text(&render(&mut app, 120, 30));
        let status = screen.lines().last().unwrap();
        assert!(status.contains("j scrolls to its end"), "{status}");
    }

    /// Verification round (VIEW-NEW-2/3): a note's cursor stays in view; the
    /// help scrolls to its last row.
    #[test]
    fn inputs_follow_the_cursor_and_the_help_scrolls() {
        use ratatui::crossterm::event::KeyCode;
        let mut app = app("view-inputs", LayoutMode::Auto);
        app.mode = Mode::EditNote {
            input: format!("{}END", "x".repeat(397)),
            unit: "u-lib".into(),
            stage: PathBuf::from("/tmp/s/stage"),
            tmp: PathBuf::from("/tmp/s"),
        };
        assert!(text(&render(&mut app, 80, 20)).contains("END▏"));
        // Scrolled past its end, the help clamps to its last page: the
        // closing sentence's last words are on screen.
        app.mode = Mode::Help { scroll: 1000 };
        let screen = text(&render(&mut app, 80, 20));
        assert!(matches!(app.mode, Mode::Help { scroll } if scroll > 0 && scroll < 1000));
        assert!(screen.contains("q / Q"), "{screen}");
        let squeezed: String = screen.chars().filter(|c| c.is_alphabetic()).collect();
        assert!(squeezed.contains("neveranswersit"), "{screen}");
        key(&mut app, KeyCode::Char('k'));
        assert!(
            matches!(app.mode, Mode::Help { .. }),
            "k scrolls, it does not close"
        );
    }

    /// Verification round (VIEW-NEW-6): no units reads 0/0.
    #[test]
    fn an_empty_plan_reads_zero_units() {
        let mut app = app("view-empty", LayoutMode::Auto);
        app.snapshot.units.clear();
        app.pairs.clear();
        let screen = text(&render(&mut app, 140, 30));
        assert!(screen.contains("units 0/0"), "{screen}");
    }
}
