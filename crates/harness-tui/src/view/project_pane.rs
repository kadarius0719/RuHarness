//! The Project view's panes (docs/PROJECT-MAP-DESIGN.md §3.7; pass 3 of
//! docs/reviews/2026-10-18-cockpit-usability-critique.md): the rows' names
//! and the right pane — the project in words, its Next step first, then
//! what the selected row is. Every string from the map goes through the
//! display filter (`wrapped`, `ellipsis`) before it is drawn.

use super::*;
use crate::project::{MapView, Offered, State};

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
        Some(Err(_)) => "project, its map cannot be read".to_string(),
        Some(Ok(None)) | None => "project, not mapped yet".to_string(),
        Some(Ok(Some(m))) => {
            let programs =
                m.offered.iter().filter(|o| o.kind != "library").count() + m.not_compiled.len();
            format!(
                "project, {programs} program{} found",
                if programs == 1 { "" } else { "s" }
            )
        }
    };
    format!(" {} ", display::line(&format!("{} · {state}", folder(app))))
}

/// An offered program's line in the summary: its mark, id, path, kind
/// guess and state in words.
/// One item of a list: its mark, then its words wrapped under it, indented.
fn item_lines(glyph: &str, text: &str, width: usize) -> Vec<Line<'static>> {
    word_wrap(text, width.saturating_sub(4).max(8))
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            if i == 0 {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{glyph} "), glyph_style(glyph)),
                    Span::raw(row),
                ])
            } else {
                Line::from(format!("    {row}"))
            }
        })
        .collect()
}

fn offered_lines(
    o: &Offered,
    accepted: bool,
    label: Option<&crate::project::Label>,
    width: usize,
) -> Vec<Line<'static>> {
    let state = if accepted {
        "accepted".to_string()
    } else if o.state == State::Held {
        format!(
            "held on a choice ({})",
            o.held
                .iter()
                .map(|h| h.set.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        o.state.words().to_string()
    };
    // What the map knows it is, then the model's label when there is one.
    let mut what = format!(" ({})", o.what());
    if let Some(l) = label {
        what.push_str(&format!(", {}", l.words()));
    }
    let glyph = if accepted { "✓" } else { o.state.glyph() };
    item_lines(
        glyph,
        &format!("{} — {}{what}: {state}", o.id, o.path),
        width,
    )
}

/// What the states shown mean, one plain line each, for the states the
/// list holds (the newcomer's ten minutes, finding 6).
fn state_glosses(offered: &[&Offered], tools: &[String]) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut add = |s: &'static str| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for o in offered.iter().filter(|o| !tools.contains(&o.id)) {
        match o.state {
            State::Linked => add("linked: its files compile and link into one program."),
            State::Held => add(
                "held on a choice: two files define the same function; Accept asks which to \
                 keep.",
            ),
            State::DidNotCompile | State::DidNotLink => add(
                "did not compile / did not link: the build likely needs flags the configuration \
                 does not state yet.",
            ),
            State::Compiles => add("compiles: a library's files compile; it is never linked."),
            State::Incomplete | State::Fuzzer | State::Driver => {}
        }
    }
    out
}

/// The counts of a group of programs by state, in words.
fn counts(offered: &[&Offered], tools: &[String]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for o in offered {
        let key = if tools.contains(&o.id) {
            "accepted".to_string()
        } else {
            o.state.words().to_string()
        };
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The Project view's right pane: the Next step, the project in words,
/// its programs, libraries and tools (each a link to its row), then what
/// the selected row is.
pub fn project_view(
    app: &App,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let Some(p) = &app.project else {
        return lines;
    };
    if let Some(next) = app.project_next() {
        lines.extend(next_step(&next.line(), width));
        lines.push(Line::from(""));
    }
    let name = folder(app);
    let m = match &p.map {
        Err(why) => {
            lines.extend(wrapped(
                &format!(
                    "{name} is a C project with no harness.toml; its map cannot be read: {why}."
                ),
                width,
                attention(),
            ));
            last_line(app, width, &mut lines);
            return lines;
        }
        Ok(None) => {
            lines.extend(wrapped(
                &format!("{name} is a C project with no harness.toml: nothing is mapped yet."),
                width,
                Style::default(),
            ));
            lines.push(Line::from(""));
            lines.extend(wrapped(
                "Mapping finds its programs and libraries, the files each one needs, what they \
                 share, and whether each program links. It compiles in the sandbox, runs \
                 nothing, and writes only under migration/.",
                width,
                notice(),
            ));
            tools_lines(app, None, width, links, &mut lines);
            last_line(app, width, &mut lines);
            return lines;
        }
        Ok(Some(m)) => m,
    };
    lines.extend(wrapped(
        &format!("{name} is a C project with no harness.toml; the cockpit shows its map."),
        width,
        Style::default(),
    ));
    // A configuration written since the map: said as it is, not as the
    // map's (the newcomer's ten minutes, finding 11).
    let configuration = if p.config_newer {
        "migration/map/config.toml is written since this map — Map the project again to use it"
            .to_string()
    } else if m.guessed && !m.proposed {
        format!(
            "{} (the map compiled with no flags of yours)",
            m.configuration_words()
        )
    } else {
        m.configuration_words()
    };
    lines.extend(wrapped(
        &format!("Configuration: {configuration}"),
        width,
        if m.unstated() || p.config_newer {
            bold()
        } else {
            Style::default()
        },
    ));
    let programs: Vec<&Offered> = m.offered.iter().filter(|o| o.kind != "library").collect();
    let libraries: Vec<&Offered> = m.offered.iter().filter(|o| o.kind == "library").collect();
    lines.push(Line::from(""));
    let mut head = format!("Programs ({})", programs.len() + m.not_compiled.len());
    let mut said = counts(&programs, &p.tools);
    if !m.not_compiled.is_empty() {
        if !said.is_empty() {
            said.push_str(" · ");
        }
        said.push_str(&format!("{} did not compile", m.not_compiled.len()));
    }
    if !said.is_empty() {
        head.push_str(&format!(": {said}"));
    }
    lines.extend(wrapped(&head, width, section_role()));
    for o in &programs {
        links.push((lines.len(), Selection::Program(o.id.clone())));
        lines.extend(offered_lines(
            o,
            p.tools.contains(&o.id),
            p.labels.get(&o.id),
            width,
        ));
    }
    for path in &m.not_compiled {
        lines.extend(item_lines(
            "✗",
            &format!("{path}: did not compile (it holds a main)"),
            width,
        ));
    }
    for gloss in state_glosses(&programs, &p.tools) {
        lines.extend(wrapped(gloss, width, notice()));
    }
    if libraries.is_empty() {
        lines.extend(wrapped("Libraries: none", width, section_role()));
    } else {
        lines.extend(wrapped(
            &format!(
                "Libraries ({}): {}",
                libraries.len(),
                counts(&libraries, &p.tools)
            ),
            width,
            section_role(),
        ));
        for o in &libraries {
            links.push((lines.len(), Selection::Program(o.id.clone())));
            lines.extend(offered_lines(o, p.tools.contains(&o.id), None, width));
        }
    }
    tools_lines(app, Some(m), width, links, &mut lines);
    selected_lines(app, m, width, &mut lines);
    last_line(app, width, &mut lines);
    lines
}

/// The accepted tools, each a link to its "Open tool" row.
fn tools_lines(
    app: &App,
    map: Option<&MapView>,
    width: usize,
    links: &mut Vec<(usize, Selection)>,
    lines: &mut Vec<Line<'static>>,
) {
    let tools = app.project_tools();
    if tools.is_empty() {
        if map.is_some() {
            lines.extend(wrapped("Accepted tools: none yet", width, section_role()));
        }
        return;
    }
    lines.extend(wrapped(
        &format!("Accepted tools ({})", tools.len()),
        width,
        section_role(),
    ));
    for id in tools {
        links.push((lines.len(), Selection::Tool(id.clone())));
        lines.extend(item_lines(
            "▸",
            &format!(
                "{} — press Enter on its row and choose Open tool",
                crate::project::tool_label(map, id)
            ),
            width,
        ));
    }
}

/// What the selected program or library is, in words: its file, its kind,
/// its state, each held choice with its definers and the model's advice.
fn selected_lines(app: &App, m: &MapView, width: usize, lines: &mut Vec<Line<'static>>) {
    let Selection::Program(id) = &app.selection else {
        return;
    };
    let Some(o) = m.get(id) else {
        return;
    };
    lines.push(Line::from(""));
    lines.extend(wrapped(&format!("── {id} ──"), width, section_role()));
    let kind = match o.kind.as_str() {
        "library" => "a library (compiled, never linked)".to_string(),
        _ => o.what(),
    };
    let accepted = app.project_tools().contains(id);
    let state = if accepted {
        format!(
            "accepted as a tool; its picks and its files are in migration/tools/{id}/harness.toml"
        )
    } else {
        o.state.words().to_string()
    };
    lines.extend(wrapped(
        &format!("{}: {kind} — {state}.", o.path),
        width,
        Style::default(),
    ));
    if let Some(l) = app.project.as_ref().and_then(|p| p.labels.get(id)) {
        lines.extend(wrapped(
            &format!(
                "The model ({}) calls it “{}” ({}): {}",
                harness_core::text::safe_line(&l.model),
                harness_core::text::safe_line(&l.name),
                l.kind,
                harness_core::text::safe_line(&l.purpose)
            ),
            width,
            label_role(),
        ));
    }
    if accepted {
        return;
    }
    let advice = app.project.as_ref().map(|p| &p.advice);
    for h in &o.held {
        let definers = h
            .definers
            .iter()
            .map(|(i, path)| format!("{i} {path}"))
            .collect::<Vec<_>>()
            .join(", ");
        let under = if h.under.is_empty() {
            String::new()
        } else {
            format!(" (asked only when you keep {})", h.under.join(" or "))
        };
        lines.extend(wrapped(
            &format!(
                "Choice {}{under}: two files define the same function, so linking cannot tell \
                 them apart ({definers}); Accept asks which to keep.",
                h.set
            ),
            width,
            bold(),
        ));
        if let Some(a) = advice.and_then(|a| a.get(&h.set)) {
            lines.extend(wrapped(&a.words(), width, label_role()));
        }
    }
}

/// The last command, as the activity line says it.
fn last_line(app: &App, width: usize, lines: &mut Vec<Line<'static>>) {
    if let Some(last) = &app.last {
        lines.push(Line::from(""));
        lines.extend(wrapped(&format!("Last: {last}"), width, last_style(app)));
    }
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
             library the map found and its accepted tools; from there you map the project \
             again, ask a model, accept another program, or open another tool.",
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
