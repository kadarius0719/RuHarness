//! The Project view's panes (docs/COCKPIT-WRAPPER-DESIGN.md §9.3, "one
//! list, one card, one line"): the rows' names, and the right pane — the
//! Next step in one line, then a card for the selected row: what it is, its
//! decision with its files numbered and the model's advice and why, and
//! what Enter does. Never the list again, never a paragraph about the
//! harness. Every string from the map goes through the display filter
//! (`wrapped`, `ellipsis`) before it is drawn.

use super::*;
use crate::menu::Action;
use crate::project::{HeldSet, Offered, State};

/// The project's folder name (never the full path).
pub fn folder(app: &App) -> String {
    app.config
        .target
        .file_name()
        .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned())
}

/// A Project view row's name in the tree.
pub fn node_name(app: &App, sel: &Selection) -> String {
    let map = app.project_map();
    match sel {
        Selection::MapProject if map.is_some() => "Map the project again".into(),
        Selection::MapProject => "Map the project".into(),
        // The id alone: the card says what it is.
        Selection::Program(id) | Selection::Tool(id) => id.clone(),
        Selection::ProjectHome => "◂ Back to the project view".into(),
        _ => String::new(),
    }
}

/// The right pane's title in the Project view: the folder and where the
/// map stands.
pub fn title(app: &App) -> String {
    let state = match app.project.as_ref().map(|p| &p.map) {
        Some(Err(_)) => "its map cannot be read".to_string(),
        Some(Ok(None)) | None => "not mapped yet".to_string(),
        Some(Ok(Some(m))) => {
            let programs =
                m.offered.iter().filter(|o| o.kind != "library").count() + m.not_compiled.len();
            format!(
                "{programs} program{} found",
                if programs == 1 { "" } else { "s" }
            )
        }
    };
    format!(" {} ", display::line(&format!("{} · {state}", folder(app))))
}

/// What each state word means, one plain line each: the help's (`?`, on
/// the Project view), not the pane's — except a failed program's, the last
/// line of its card.
pub const STATE_GLOSSES: &[(&str, &str)] = &[
    (
        "✓ links",
        "its files compile and link; Accept makes it a tool.",
    ),
    (
        "⚠ needs a decision",
        "two files define the same functions; pick one in Accept.",
    ),
    (
        "✗ did not compile or link",
        "the build likely needs more flags.",
    ),
    (
        "✓ compiles",
        "a library's files compile; a library is never linked.",
    ),
    (
        "✓ accepted",
        "it is a tool: open it to scan, plan and migrate it.",
    ),
    (
        "· fuzzer, not a tool",
        "linked only with the project's fuzzing driver.",
    ),
    (
        "· incomplete",
        "the map could not tell every file it needs.",
    ),
];

/// The gloss of a program that did not compile or link.
const FAILED_GLOSS: &str =
    "The build likely needs flags its build configuration does not give yet.";

/// The card of the Map row: what is there, and what mapping does — the
/// only prose left on the pane (and the help's, too).
pub const NOTHING_MAPPED: &str = "Nothing is mapped yet.";
/// What mapping does, in two sentences.
pub const MAPPING_WORDS: &str = "Mapping finds its programs and libraries, the files each one \
     needs, what they share, and whether each program links. It compiles in the sandbox, runs \
     nothing, and writes only under migration/.";

use crate::project::ADVISED;

/// A number in words, for the few files a decision has.
fn number(n: usize) -> String {
    match n {
        2 => "two".into(),
        3 => "three".into(),
        4 => "four".into(),
        5 => "five".into(),
        n => n.to_string(),
    }
}

/// The card's width at most: on a wide screen the state and the advised
/// file stay near what they describe.
pub const CARD_MAX: usize = 100;

/// The Project view's right pane: the Next step in one line, then the
/// card of the selection. The card holds no link: Enter in this pane opens
/// the selection's menu, as on its row. The last command is the status
/// line's, under the panes, not repeated here.
pub fn project_view(
    app: &App,
    width: usize,
    _links: &mut Vec<(usize, Selection)>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if app.project.is_none() {
        return lines;
    }
    let width = width.min(CARD_MAX);
    if let Some(next) = app.project_next() {
        lines.extend(next_line(&next, width));
        lines.push(Line::from(""));
    }
    card(app, width, &mut lines);
    lines
}

/// The Next step, one line: `Next:` in the accent, the phrase bold.
pub fn next_line(next: &crate::next::Advice, width: usize) -> Vec<Line<'static>> {
    let line = next.line();
    let phrase = line.strip_prefix("Next: ").unwrap_or(&line);
    next_step(&format!("Next: {phrase}"), width)
}

/// The card of the selected row.
fn card(app: &App, width: usize, lines: &mut Vec<Line<'static>>) {
    let Some(p) = &app.project else {
        return;
    };
    let sel = app.selection.clone();
    match (&sel, &p.map) {
        (Selection::Program(id), Ok(Some(m))) => match m.get(id) {
            Some(o) => program_card(app, o, width, lines),
            None => return,
        },
        (Selection::Tool(id), _) => {
            head(id, "✓", "accepted", width, lines);
            lines.extend(wrapped(&build_file(id), width, Style::default()));
            progress_line(app, id, width, lines);
        }
        (_, Err(why)) => {
            lines.extend(wrapped(
                &format!("The map cannot be read: {why}."),
                width,
                attention(),
            ));
        }
        (_, Ok(None)) => {
            lines.extend(wrapped(NOTHING_MAPPED, width, Style::default()));
            lines.extend(wrapped(MAPPING_WORDS, width, notice()));
        }
        (_, Ok(Some(_))) => {
            lines.extend(wrapped(
                "The map found no program and no library.",
                width,
                Style::default(),
            ));
        }
    }
    // What Enter does, unless the Next step above already says it.
    let next = app.project_next().map(|n| crate::menu::reason_of(&n.text));
    if let Some(enter) = enter_line(app, &sel)
        .filter(|e| next.as_deref() != Some(crate::menu::reason_of(e).as_str()))
    {
        lines.push(Line::from(""));
        lines.extend(wrapped(&format!("Enter: {enter}"), width, Style::default()));
    }
    // A program that did not compile or link: what the state means, last.
    if let Selection::Program(id) = &sel {
        let failed = app.project_offered(id).is_some_and(|o| {
            matches!(o.state, State::DidNotCompile | State::DidNotLink)
                && !app.project_tools().contains(id)
        });
        if failed {
            lines.extend(wrapped(FAILED_GLOSS, width, label_role()));
        }
    }
}

/// An accepted tool's progress, from its ledger: "3 pieces · 1 waits for
/// your Accept" (fix pass, item 4).
fn progress_line(app: &App, id: &str, width: usize, lines: &mut Vec<Line<'static>>) {
    if let Some(pr) = app.project.as_ref().and_then(|p| p.progress.get(id)) {
        let style = if pr.waiting() {
            attention()
        } else {
            Style::default()
        };
        let words = pr.words();
        lines.extend(wrapped(
            &if pr.waiting() {
                format!("⚑ {words}")
            } else {
                words
            },
            width,
            style,
        ));
    }
}

/// An accepted tool's build configuration, in words with its path.
fn build_file(id: &str) -> String {
    format!(
        "Its build configuration: migration/tools/{id}/{}",
        harness_core::config::CONFIG_FILE
    )
}

/// The card's first line: the id, and its mark and state at the right.
fn head(id: &str, glyph: &str, word: &str, width: usize, lines: &mut Vec<Line<'static>>) {
    let id = display::line(id);
    let state = format!("{glyph} {word}");
    let style = glyph_style(glyph).add_modifier(Modifier::BOLD);
    let gap = width.saturating_sub(width_of(&id) + width_of(&state));
    if gap >= 2 {
        lines.push(Line::from(vec![
            Span::styled(id, bold()),
            Span::raw(" ".repeat(gap)),
            Span::styled(state, style),
        ]));
    } else {
        lines.extend(wrapped(&id, width, bold()));
        lines.push(Line::from(Span::styled(state, style)));
    }
}

/// A program's or a library's card.
fn program_card(app: &App, o: &Offered, width: usize, lines: &mut Vec<Line<'static>>) {
    let accepted = app.project_tools().contains(&o.id);
    if accepted {
        head(&o.id, "✓", "accepted", width, lines);
    } else {
        head(&o.id, o.state.glyph(), o.state.words(), width, lines);
    }
    // Once accepted, what the map guessed no longer matters: its path.
    let file = if accepted {
        o.path.clone()
    } else {
        format!("{} · {}", o.path, o.what())
    };
    lines.extend(wrapped(&file, width, Style::default()));
    if let Some(l) = app.project.as_ref().and_then(|p| p.labels.get(&o.id)) {
        lines.extend(wrapped(&l.words(), width, label_role()));
    }
    if accepted {
        lines.extend(wrapped(&build_file(&o.id), width, Style::default()));
        progress_line(app, &o.id, width, lines);
        return;
    }
    if !o.problem.is_empty() {
        lines.extend(wrapped(&format!("✗ {}", o.problem), width, bad()));
    }
    if matches!(o.state, State::Fuzzer | State::Driver) {
        lines.extend(wrapped(
            "A fuzzer is not accepted as a tool: it is linked only with the project's driver.",
            width,
            label_role(),
        ));
    }
    for h in &o.held {
        decision_lines(app, o, h, width, lines);
    }
}

/// One decision on a card: the question, the files numbered, the model's
/// advice and why.
fn decision_lines(
    app: &App,
    o: &Offered,
    h: &HeldSet,
    width: usize,
    lines: &mut Vec<Line<'static>>,
) {
    lines.push(Line::from(""));
    lines.extend(wrapped(
        &format!(
            "Decision: {} files define the same functions. Keep which?",
            number(h.definers.len())
        ),
        width,
        Style::default(),
    ));
    if !h.under.is_empty() {
        let paths: Vec<String> = h.under.iter().map(|u| definer_path(o, u)).collect();
        lines.extend(wrapped(
            &format!("Asked only if you keep {}.", paths.join(" or ")),
            width,
            Style::default(),
        ));
    }
    let advice = app.project.as_ref().and_then(|p| p.advice.get(&h.set));
    for (n, (index, path)) in h.definers.iter().enumerate() {
        let file = format!("  {}  {}", n + 1, display::line(path));
        // Indented, never wrapped (the wrap would drop the indent): a path
        // too long for the pane is cut with "…".
        if advice.is_none_or(|a| &a.keep != index) {
            lines.push(Line::from(ellipsis(&file, width)));
            continue;
        }
        // The mark right after the file, as in Accept's picker.
        if width_of(&file) + 2 + width_of(ADVISED) <= width {
            lines.push(Line::from(vec![
                Span::raw(file),
                Span::raw("  "),
                Span::styled(ADVISED, label_role()),
            ]));
        } else {
            lines.push(Line::from(ellipsis(&file, width)));
            lines.push(Line::from(Span::styled(
                ellipsis(&format!("     {ADVISED}"), width),
                label_role(),
            )));
        }
    }
    match advice {
        None => {
            // What gets advice, from where the person is.
            let how = if app.project_map().is_some_and(|m| m.unstated()) {
                "Advice comes once the build configuration is written and the project mapped again"
                    .to_string()
            } else if app.chat_on {
                "No advice yet — Enter, then Ask for advice (the chat answers)".to_string()
            } else {
                // The command spelled for this project (fix pass, item 1).
                format!(
                    "No advice yet — run {} (the models topic), or start the cockpit without \
                     --no-chat",
                    harness_core::runtime_view::command_line("project ask", None)
                )
            };
            lines.extend(wrapped(&how, width, label_role()));
        }
        Some(a) => {
            lines.extend(wrapped(&a.why_line(), width, Style::default()));
            lines.extend(wrapped(
                "This is advice; you pick the file when you accept.",
                width,
                label_role(),
            ));
        }
    }
}

/// A definer's file, by its index (the index itself when the map does not
/// name it).
fn definer_path(o: &Offered, index: &str) -> String {
    o.held
        .iter()
        .flat_map(|x| x.definers.iter())
        .find(|(i, _)| i == index)
        .map_or_else(|| index.to_string(), |(_, path)| path.clone())
}

/// What Enter does on the selected row, as the card's last line says it:
/// the row's Next step as a verb phrase ("Accept t-lzg"), with what
/// follows when something does.
pub fn enter_line(app: &App, sel: &Selection) -> Option<String> {
    let advice = app.project_row_advice(sel)?;
    let action = advice.action.as_ref()?;
    let phrase = crate::menu::reason_of(&advice.text);
    Some(match action {
        Action::AcceptProgram { id, .. }
            if app.project_offered(id).is_some_and(|o| !o.held.is_empty()) =>
        {
            format!("{phrase} — it asks for the file")
        }
        Action::WriteBuildConfig
            if matches!(sel, Selection::Program(_))
                && app.project_map().is_some_and(|m| !m.unstated()) =>
        {
            format!("{phrase}, then Map the project again")
        }
        Action::AcceptProgram { .. }
        | Action::OpenTool(_)
        | Action::WriteBuildConfig
        | Action::Project(crate::project::Act::Map) => phrase,
        _ => return None,
    })
}

/// A tool's row back to the Project view: what it leads to.
pub fn home_view(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(next) = app.project_row_advice(&Selection::ProjectHome) {
        lines.extend(next_step(&next.line(), width));
        lines.push(Line::from(""));
    }
    let tool = app.config.tool.as_deref().unwrap_or("this tool");
    lines.extend(wrapped(
        &format!(
            "{tool} is one tool of the project {}. The project view lists every program and \
             library the map found; from there you map the project again, ask a model, accept \
             another program, or open another tool.",
            folder(app)
        ),
        width,
        Style::default(),
    ));
    if let Some(why) = app.switch_refusal() {
        lines.push(Line::from(""));
        lines.extend(wrapped(&format!("Not now: {why}."), width, bold()));
    }
    lines
}
