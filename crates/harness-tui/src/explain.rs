//! Explain anything (docs/COCKPIT-WRAPPER-DESIGN.md §9.5): for every row
//! the person can select, one sentence for the hint line under the panes
//! and a short card (`i`, or **What is this?**, the last item of every
//! Enter menu) — both written here, from the same facts, so the line and
//! the card never disagree.
//!
//! The words say what a thing is and what to do with it, in the person's
//! terms: never the harness's own names for its files and states (the
//! build configuration's file name only on a tool's card, its path line).

use crate::app::App;
use crate::featmap::Group as FeatureGroup;
use crate::files::{Cause, FileState, UnitState};
use crate::model::{short_id, ProvenanceView, UnitView};
use crate::project::State;
use crate::speed::Group as SpeedGroup;
use crate::tree::Selection;
use std::path::Path;

/// A hint sentence is at most this many characters (a test holds every row
/// to it): it must fit one status line.
pub const HINT_MAX: usize = 159;

/// What a row is: its hint sentence and its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    /// The card's title: the row's name.
    pub title: String,
    /// One sentence: what the row is.
    pub hint: String,
    /// Shorter forms of the same sentence, longest first, for a narrow
    /// status line.
    pub shorter: Vec<String>,
    /// Three to eight short lines: what it is, where it came from, why it
    /// is there, what to do next, where its files live.
    pub card: Vec<String>,
}

/// The selected row's hint sentence (drawn every frame: no menu is built
/// for it).
pub fn hint(app: &App, sel: &Selection) -> String {
    explained(app, sel).hint
}

/// The selected row's hint sentence in `width` columns: the longest form
/// that fits whole (the shortest when none does; the status line cuts it).
pub fn hint_in(app: &App, sel: &Selection, width: usize) -> String {
    let e = explained(app, sel);
    let fits = |h: &String| h.chars().count() <= width;
    std::iter::once(&e.hint)
        .chain(e.shorter.iter())
        .find(|h| fits(h))
        .or(e.shorter.last())
        .unwrap_or(&e.hint)
        .clone()
}

/// What `sel` is, for its card: its own lines, then what Enter recommends
/// there — the menu's first item, for the selected row.
pub fn explain(app: &App, sel: &Selection) -> Explained {
    let mut e = explained(app, sel);
    if let Some(next) = next_words(app, sel) {
        e.card.truncate(7);
        e.card.push(format!("Next: {next}"));
    }
    e
}

/// What `sel` is: its hint and its card's own lines.
fn explained(app: &App, sel: &Selection) -> Explained {
    let mut e = match sel {
        Selection::Project => project(app),
        Selection::Dir(d) => dir(app, d),
        Selection::File(p) => file(app, p),
        Selection::Function(p, name) => function(app, p, name),
        Selection::Units => units(app),
        Selection::Unit(id) => unit(app, id),
        Selection::Crate(id) => crate_row(app, id),
        Selection::Attempt(u, a) => attempt(app, u, a),
        Selection::Features => features(app),
        Selection::Feature(id) => feature(app, id),
        Selection::Speed => speed(app),
        Selection::MapProject => map_row(app),
        Selection::Program(id) => program(app, id),
        Selection::Tool(id) => tool(app, id),
        Selection::ProjectHome => home(app),
    };
    // `fit` keeps its shorter forms behind the sentence.
    let all = std::mem::take(&mut e.hint);
    let mut forms = all.split(FORMS).map(str::to_string);
    e.hint = forms.next().unwrap_or_default();
    e.shorter = forms.collect();
    e.card.truncate(8);
    e
}

/// Between a sentence and its shorter forms, inside `fit`'s result.
const FORMS: char = '\u{1f}';

/// The candidates that fit [`HINT_MAX`], longest first, joined by
/// [`FORMS`] (the last one, cut, when none does): the sentence and its
/// shorter forms.
fn fit(candidates: &[String]) -> String {
    let fitting: Vec<&str> = candidates
        .iter()
        .filter(|c| c.chars().count() <= HINT_MAX)
        .map(String::as_str)
        .collect();
    if !fitting.is_empty() {
        return fitting.join(&FORMS.to_string());
    }
    let last = candidates.last().cloned().unwrap_or_default();
    let mut cut: String = last.chars().take(HINT_MAX - 1).collect();
    cut.push('…');
    cut
}

/// What Enter recommends on the row: for the selected row, the menu item
/// that does its Next step ("Enter, then Accept t-lzg"); else the Next
/// step's own words.
fn next_words(app: &App, sel: &Selection) -> Option<String> {
    let project = app.in_project_view() || *sel == Selection::ProjectHome;
    let advice = if project {
        app.project_row_advice(sel)
    } else {
        app.advice(sel)
    }?;
    if let (Some(want), true) = (&advice.action, *sel == app.selection) {
        let items = if project {
            app.project_menu_items()
        } else {
            app.menu_items()
        };
        if let Some(it) = items
            .iter()
            .find(|it| &it.action == want && it.greyed.is_none())
        {
            return Some(format!("Enter, then {}", it.label));
        }
    }
    let line = advice.line();
    let text = line
        .strip_prefix("Next: ")
        .unwrap_or(&line)
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// A path under the target, relative to its root, `/`-joined.
fn rel(app: &App, path: &Path) -> String {
    path.strip_prefix(&app.snapshot.root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

/// The last part of a path.
fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The tool's name, or the project's folder.
fn program_name(app: &App) -> String {
    match &app.config.tool {
        Some(id) => id.clone(),
        None => folder(app),
    }
}

fn folder(app: &App) -> String {
    app.config
        .target
        .file_name()
        .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned())
}

/// The unit's state, as the Files pane has it.
fn state_of<'a>(app: &'a App, id: &str) -> Option<&'a UnitState> {
    let i = app.snapshot.units.iter().position(|u| u.unit.id == id)?;
    app.files.units.get(i).map(|i| &i.state)
}

/// The counts the tool's summary uses: pieces that can move (blocked and
/// stays-C left out), of them done; pieces that stay C.
fn piece_counts(app: &App) -> (usize, usize, usize) {
    let (mut pieces, mut done, mut stay) = (0, 0, 0);
    for info in &app.files.units {
        match &info.state {
            UnitState::Blocked => {}
            UnitState::StaysC(_) => stay += 1,
            UnitState::Migrated(_) | UnitState::OriginUnknown => {
                pieces += 1;
                done += 1;
            }
            _ => pieces += 1,
        }
    }
    (pieces, done, stay)
}

// ----- the tool's screen ------------------------------------------------------

fn project(app: &App) -> Explained {
    let (pieces, done, stay) = piece_counts(app);
    let name = program_name(app);
    let title = match &app.config.tool {
        Some(id) => format!("{} · tool {id}", folder(app)),
        None => folder(app),
    };
    let rust = if app.snapshot.units.is_empty() {
        "it is not cut into pieces yet (scan and plan it first)".to_string()
    } else {
        match (pieces, done) {
            (1, 1) => "its one piece is in Rust".to_string(),
            (1, _) => "its one piece is not in Rust yet".to_string(),
            _ => format!("{done} of its {pieces} pieces are in Rust"),
        }
    };
    let hint = match &app.config.tool {
        Some(id) => fit(&[
            format!(
                "{id} is one program of {}, built from the files below; {rust}.",
                folder(app)
            ),
            format!("{id} is one program of the project; {rust}."),
            format!("{id}: {rust}."),
        ]),
        None => fit(&[
            format!("{name}: the C project moving to Rust, its files below; {rust}."),
            format!("The C project moving to Rust; {rust}."),
            format!("The project: {rust}."),
        ]),
    };
    let ledger = app.snapshot.ledger_rel();
    let mut card = vec![match &app.config.tool {
        Some(id) => format!(
            "{id} is one program of the project {}: the C files it is built from, below.",
            folder(app)
        ),
        None => format!(
            "The C project in {}: its files are listed below.",
            folder(app)
        ),
    }];
    if app.snapshot.units.is_empty() {
        card.push("It is not cut into pieces yet: scan it, then refresh the plan.".into());
    } else {
        let mut parts = vec![
            plural(pieces, "piece that can move", "pieces that can move"),
            format!("{done} in Rust"),
        ];
        if stay > 0 {
            parts.push(format!("{stay} stay C"));
        }
        card.push(format!("Its pieces: {}.", parts.join(" · ")));
    }
    card.push("Your C files are never changed: each piece's Rust is written beside them.".into());
    if app.config.tool.is_some() {
        card.push(format!(
            "Its build configuration: {ledger}/{}",
            harness_core::config::CONFIG_FILE
        ));
    }
    card.push(format!("Its records: {ledger}/"));
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn dir(app: &App, d: &str) -> Explained {
    let prefix = format!("{d}/");
    let under: Vec<_> = app
        .files
        .files
        .iter()
        .filter(|f| f.path.starts_with(&prefix))
        .collect();
    let n = under.len();
    let outside = under
        .iter()
        .filter(|f| f.state == FileState::NotInTool)
        .count();
    let owned = under
        .iter()
        .filter(|f| matches!(f.state, FileState::Owned(_)))
        .count();
    let tool = program_name(app);
    let hint = if outside == n && n > 0 {
        fit(&[format!(
            "{prefix}: none of its {} is part of {tool}.",
            plural(n, "file", "files")
        )])
    } else if outside > 0 {
        fit(&[
            format!(
                "{prefix}: {} of its {n} files {} part of {tool}; the rest are not.",
                n - outside,
                if n - outside == 1 { "is" } else { "are" }
            ),
            format!(
                "A folder: {} of its {n} files are part of {tool}.",
                n - outside
            ),
        ])
    } else {
        fit(&[
            format!(
                "{prefix}: a folder of {}; {owned} of them hold the C of a piece.",
                plural(n, "file", "files")
            ),
            format!("A folder of {}.", plural(n, "file", "files")),
        ])
    };
    let rollup = app.files.rollup(d).words();
    let mut card = vec![format!(
        "A folder of the project: {}.",
        plural(n, "file", "files")
    )];
    if outside > 0 {
        card.push(format!(
            "{} not part of {tool}: the program never reaches them.",
            plural(outside, "file is", "files are")
        ));
    }
    if !rollup.is_empty() {
        card.push(format!("Where it stands: {rollup}."));
    }
    if outside < n {
        card.push(format!(
            "{} the C of a piece; the rest have nothing to move on their own.",
            plural(owned, "file holds", "files hold")
        ));
    }
    card.push("Enter or → opens it; ← folds it.".into());
    Explained {
        title: prefix.clone(),
        hint,
        shorter: Vec::new(),
        card,
    }
}

/// The file holding the program's `main`, from the scan's facts.
fn main_file(app: &App) -> Option<String> {
    let facts = app.snapshot.facts.as_ref()?;
    facts
        .symbols
        .iter()
        .find(|s| s.name == "main")
        .map(|s| s.file.clone())
}

fn file(app: &App, p: &str) -> Explained {
    let name = base(p).to_string();
    let title = name.clone();
    let Some(f) = app.files.file(p) else {
        return Explained {
            title,
            hint: format!("{name}: a file of the project."),
            shorter: Vec::new(),
            card: vec![
                format!("{p}: a file of the project."),
                "It is not in the last read of the folder.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let tool = program_name(app);
    let (hint, mut card) = match &f.state {
        FileState::Owned(u) => {
            let unit = app.snapshot.units.get(*u);
            let id = unit.map_or("its piece", |u| u.unit.id.as_str());
            let state = app.files.units.get(*u).map(|i| &i.state);
            let words = state.map(unit_state_clause).unwrap_or_default();
            let mut card = vec![
                format!("{p}: C code of {tool}."),
                format!(
                    "Its functions belong to the piece {id}: they move to Rust together, as one."
                ),
            ];
            if let Some(u) = unit.filter(|u| u.crate_dir.is_some()) {
                card.push(format!(
                    "This C stays as it is: the Rust is a separate copy, in {}/.",
                    rel(app, u.crate_dir.as_deref().expect("filtered"))
                ));
                card.push("The judge builds both and compares their outputs.".into());
            } else {
                card.push("This file is never changed: the Rust is written beside it.".into());
            }
            (
                fit(&[
                    format!("{name} is the C of the piece {id}; {words}."),
                    format!("The C of the piece {id}; {words}."),
                    format!("The C of the piece {id}."),
                ]),
                card,
            )
        }
        FileState::Header => (
            "A header: declarations only; nothing to move.".to_string(),
            vec![
                format!("{p}: a header."),
                "It declares what other files define: types, constants, function names.".into(),
                "It holds no code of its own to move, so it is not a piece.".into(),
            ],
        ),
        FileState::NotInTool => {
            let from = main_file(app)
                .map(|m| base(&m).to_string())
                .unwrap_or_else(|| "the program's own files".into());
            (
                fit(&[
                    format!(
                        "{name} is in the project, but {tool} never calls into it: the map \
                         followed what {from} links and this file is not reached."
                    ),
                    format!(
                        "{name} is in the project, but {tool} never reaches it, so it is not \
                         part of this tool."
                    ),
                ]),
                vec![
                    format!("{p}: a file of the project, not of {tool}."),
                    format!(
                        "The map followed what {from} links; nothing it links reaches this file."
                    ),
                    "Nothing here acts on it. Another program of the project may use it: see \
                     the project view."
                        .into(),
                ],
            )
        }
        FileState::NoExports => (
            fit(&[format!(
                "{name} has no functions other files call, so it is not a piece of its own."
            )]),
            vec![
                format!("{p}: C code with no exported functions."),
                "A piece is a group of functions other code calls; this file has none.".into(),
                "Its code moves with whatever uses it, if anything does.".into(),
            ],
        ),
        FileState::NotInPlan => (
            fit(&[format!(
                "{name} has functions, but no piece holds it yet: Refresh the plan to cut it \
                 into pieces."
            )]),
            vec![
                format!("{p}: C code not in the plan yet."),
                "The plan cuts the C into pieces; this file came after the last cut.".into(),
                "Refresh the plan (Enter on the project row) to give it a piece.".into(),
            ],
        ),
        FileState::Changed => (
            fit(&[format!(
                "{name} changed since the last scan: Scan the project to note what changed."
            )]),
            vec![
                format!("{p}: changed since the last scan."),
                "What is known about its functions may be out of date.".into(),
                "Scan the project, then Refresh the plan.".into(),
            ],
        ),
        FileState::New => (
            fit(&[format!(
                "{name} is new since the last scan: Scan the project to read it."
            )]),
            vec![
                format!("{p}: not scanned yet."),
                "It was added after the last scan, so nothing is known about it.".into(),
                "Scan the project to read it.".into(),
            ],
        ),
        FileState::Missing => (
            fit(&[format!(
                "{name} was scanned before but is gone from the folder now."
            )]),
            vec![
                format!("{p}: missing."),
                "The last scan read it; it is not in the folder any more.".into(),
                "Scan the project and Refresh the plan to catch up.".into(),
            ],
        ),
    };
    if card.len() < 3 {
        card.push("Enter shows what you can do with it.".into());
    }
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn function(app: &App, p: &str, name: &str) -> Explained {
    let short = crate::view::short_symbol(name).to_string();
    let title = format!("{short}()");
    let f = app.files.file(p);
    let info = f.and_then(|f| f.functions.iter().find(|x| x.name == name));
    let line = info.map_or(1, |x| x.line);
    let owner = f
        .and_then(|f| f.owner)
        .and_then(|u| app.snapshot.units.get(u));
    let in_unit = info.is_some_and(|x| x.in_unit);
    let (hint, mut card) = match (in_unit, owner) {
        (true, Some(u)) => {
            let n = u.unit.symbols.len();
            (
                fit(&[
                    format!(
                        "{short}() is one of the {} of {}; it moves to Rust with them.",
                        plural(n, "function", "functions"),
                        u.unit.id
                    ),
                    format!("{short}(): a function of the piece {}.", u.unit.id),
                ]),
                vec![
                    format!("{short}(), at line {line} of {}.", base(p)),
                    format!(
                        "Other code calls it, so it belongs to the piece {}: it moves to Rust \
                         with the piece's other functions.",
                        u.unit.id
                    ),
                ],
            )
        }
        (_, Some(u)) => (
            fit(&[
                format!(
                    "{short}() is used only inside {}: it goes with its file's piece, {}, \
                     never on its own.",
                    base(p),
                    u.unit.id
                ),
                format!("{short}(): used only inside its file."),
            ]),
            vec![
                format!("{short}(), at line {line} of {}.", base(p)),
                "Only its own file calls it, so it is not one of the piece's functions.".into(),
                format!("It goes wherever its file's piece, {}, goes.", u.unit.id),
            ],
        ),
        (_, None) => (
            fit(&[
                format!(
                    "{short}() is used only inside {}; no piece holds it.",
                    base(p)
                ),
                format!("{short}(): a function of {}.", base(p)),
            ]),
            vec![
                format!("{short}(), at line {line} of {}.", base(p)),
                "No piece of the plan holds its file.".into(),
            ],
        ),
    };
    card.push("Enter on it shows its code.".into());
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn units(app: &App) -> Explained {
    let (pieces, done, stay) = piece_counts(app);
    let n = app.snapshot.units.len();
    let all = format!(
        "The program's pieces: {}, each a group of C functions that move to Rust together.",
        plural(n, "piece", "pieces")
    );
    // Why some do not move is said here, not on the Next line.
    let hint = if stay > 0 {
        fit(&[
            format!(
                "The program's pieces: {pieces} can move to Rust, {done} of them done; {stay} \
                 call other pieces, so in this version they stay C."
            ),
            all,
        ])
    } else {
        fit(&[all])
    };
    let mut card = vec![
        "The plan cuts the C into pieces: functions that call each other move together.".into(),
        format!("{pieces} can move now; {done} of them are in Rust."),
    ];
    if stay > 0 {
        card.push(format!(
            "{} other pieces, so in this version {} C.",
            plural(stay, "piece calls", "pieces call"),
            if stay == 1 { "it stays" } else { "they stay" }
        ));
    }
    card.push("Each piece goes: a test program, then its Rust, then your Accept.".into());
    Explained {
        title: format!("Units ({n})"),
        hint,
        shorter: Vec::new(),
        card,
    }
}

/// A unit's state as a clause after "; ".
fn unit_state_clause(s: &UnitState) -> String {
    match s {
        UnitState::Blocked => "it is set aside: its files are gone from the C".into(),
        UnitState::Attention(cause) => format!("it needs attention: {}", cause_words(cause)),
        UnitState::Failing => "its Rust failed the judge's last check".into(),
        UnitState::Migrated(_) | UnitState::OriginUnknown => {
            "its Rust is done and passed the judge's check".into()
        }
        UnitState::Waiting => "a try of its Rust stopped to wait for an answer".into(),
        UnitState::StaysC(deps) => {
            format!("it calls {}, so in this version it stays C", and_list(deps))
        }
        UnitState::RustGreen => "a try passed every check and waits for your Accept".into(),
        UnitState::RustTried => "a model tried its Rust; no try has passed every check".into(),
        UnitState::TestProgramReady => "its test program is ready, so its Rust can be made".into(),
        UnitState::TestProgramStale => {
            "its test program is out of date and must be made again".into()
        }
        UnitState::NoTestProgram => "it has no test program yet".into(),
        UnitState::Other(w) => format!("its state is {}", w.replace('-', " ")),
    }
}

/// A cause in plain words (never the harness's own name for the judge's
/// record).
fn cause_words(c: &Cause) -> &'static str {
    match c {
        Cause::Contradiction => "its status and the judge's check disagree",
        Cause::PromotionInterrupted { .. } => "an Accept was interrupted",
        Cause::SourceChanged => "its C changed since it was planned",
        Cause::StaleVerdict => "the judge's check is out of date",
        Cause::ChangedOutside => "its Rust was changed outside the cockpit",
        Cause::NoEvidence => "the judge's check of its Rust is missing",
        Cause::MadeElsewhere { .. } => "the judge's check was made on another computer",
    }
}

/// Where a unit's functions are: "src/a.c", "src/a.c and src/b.c", "3
/// files".
fn files_words(u: &UnitView) -> String {
    match u.unit.files.as_slice() {
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        many => plural(many.len(), "file", "files"),
    }
}

fn unit(app: &App, id: &str) -> Explained {
    let title = id.to_string();
    let Some(u) = app.snapshot.unit(id) else {
        return Explained {
            title,
            hint: format!("{id}: a piece of the program."),
            shorter: Vec::new(),
            card: vec![
                format!("{id}: a piece of the program."),
                "It is not in the last read of the plan.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let state = state_of(app, id)
        .cloned()
        .unwrap_or(UnitState::Other(String::new()));
    let n = u.unit.symbols.len();
    let funcs = plural(n, "function", "functions");
    let moves = if n == 1 {
        "that moves to Rust"
    } else {
        "that move to Rust together"
    };
    let hint = match &state {
        UnitState::StaysC(deps) => fit(&[
            format!(
                "{id} calls {}; in this version the judge tests each piece alone, so a piece \
                 that calls other pieces stays C.",
                and_list(deps)
            ),
            format!("{id} calls other pieces; the judge tests each piece alone, so it stays C."),
        ]),
        s => {
            let clause = unit_state_clause(s);
            fit(&[
                format!(
                    "{id} is one piece of the program: {funcs} in {} {moves}; {clause}.",
                    files_words(u)
                ),
                format!(
                    "{id} is one piece of the program: {funcs} {moves}; \
                     {clause}."
                ),
                format!("{id}: one piece of the program; {clause}."),
                format!("{id}: one piece of the program; {}.", s.word()),
            ])
        }
    };
    let mut card = vec![
        format!(
            "One piece of the program: {funcs} in {} {moves}.",
            files_words(u)
        ),
        format!("Now: {}.", unit_state_clause(&state)),
    ];
    if let UnitState::StaysC(_) = state {
        card.push(
            "The judge tests each piece alone against its C; a piece that calls other pieces \
             cannot be tested that way yet."
                .into(),
        );
    }
    match &u.crate_dir {
        Some(dir) => {
            card.push(match origin_words(u) {
                Some(o) => format!("Its Rust: {o}, kept in {}/.", rel(app, dir)),
                None => format!(
                    "Its Rust: kept in {}/ (where it came from is not recorded).",
                    rel(app, dir)
                ),
            });
            card.push(
                "The C stays as it is: the judge builds both and compares their outputs.".into(),
            );
        }
        None if !u.attempts.is_empty() => card.push(format!(
            "Its tries: {}; none is its Rust yet.",
            u.attempts.len()
        )),
        None => {}
    }
    if let Some(driver) = u
        .unit
        .oracle_param_str("driver")
        .filter(|_| u.driver != crate::model::DriverState::None)
    {
        card.push(format!(
            "Its test program: {driver}, a small C program the judge runs on the C and the Rust."
        ));
    }
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

/// Where the unit's Rust came from, in words: "written by try a-fbbe
/// (asked in chat)".
/// `None` when no record says.
fn origin_words(u: &UnitView) -> Option<String> {
    Some(match &u.provenance {
        ProvenanceView::None => return None,
        ProvenanceView::Pipeline(a) => format!("written by try {}", short_id(a)),
        ProvenanceView::Chat(a) => format!("written by try {} (asked in chat)", short_id(a)),
        ProvenanceView::Steered(a) => {
            format!("written by try {} (steered by a note)", short_id(a))
        }
        ProvenanceView::Ambiguous(ids) => {
            format!("written by one of {} tries alike", ids.len())
        }
        ProvenanceView::Human { attempt, origin } if attempt == origin => {
            format!("your hand edit {}", short_id(origin))
        }
        ProvenanceView::Human { attempt, origin } => format!(
            "try {}, a steer of your hand edit {}",
            short_id(attempt),
            short_id(origin)
        ),
    })
}

fn crate_row(app: &App, id: &str) -> Explained {
    let title = format!("{id}'s Rust");
    let Some(u) = app.snapshot.unit(id) else {
        return Explained {
            title,
            hint: format!("The Rust of {id}."),
            shorter: Vec::new(),
            card: vec![
                format!("The Rust of {id}."),
                "It is not in the last read of the plan.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let Some(dir) = &u.crate_dir else {
        return Explained {
            title,
            hint: fit(&[format!(
                "The Rust of {id}: none yet; it appears once a try of it is accepted."
            )]),
            shorter: Vec::new(),
            card: vec![
                format!("The Rust of {id}: none yet."),
                "A try writes Rust in its own folder; Accept makes it this piece's Rust.".into(),
                "Your C files are never changed.".into(),
            ],
        };
    };
    let origin = origin_words(u);
    let plain =
        format!("The Rust for {id}; the C stays beside it so the judge can compare the two.");
    let hint = match &origin {
        Some(o) => fit(&[
            format!(
                "The Rust for {id}, {o}; the C stays beside it so the judge can compare the two."
            ),
            plain,
        ]),
        None => plain,
    };
    let promoted = matches!(
        u.provenance,
        ProvenanceView::Pipeline(_)
            | ProvenanceView::Chat(_)
            | ProvenanceView::Steered(_)
            | ProvenanceView::Human { .. }
    );
    let mut card = vec![
        format!("The Rust of {id}: a Rust crate in {}/.", rel(app, dir)),
        match (&origin, promoted) {
            (Some(o), true) => {
                format!("Where it came from: {o}; Accept copied it here from the try's folder.")
            }
            (Some(o), false) => format!("Where it came from: {o}."),
            (None, _) => "Where it came from is not recorded; no recorded try made it.".into(),
        },
        "Why the C stays: the judge builds the C and this Rust and compares their outputs; \
         your C files are never changed."
            .into(),
        "The program itself keeps its C: the Rust is linked in its place only in the judge's, \
         the features' and Speed's own builds."
            .into(),
    ];
    if let Some(s) = state_of(app, id) {
        card.push(format!("Now: {}.", unit_state_clause(s)));
    }
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

/// What a try's outcome means, as a clause.
fn outcome_clause(outcome: &str) -> &'static str {
    match outcome {
        "green" => "green means every check passed",
        "red" => "red means a check failed",
        "in-progress" => "it stopped to wait for an answer",
        "truncated" => "the model's answer was cut short",
        "format" => "the model's answer was not in the form asked for",
        "blocked" => "it was stopped before it finished",
        _ => "it did not finish",
    }
}

fn attempt(app: &App, unit_id: &str, a: &str) -> Explained {
    let title = format!("Try {}", short_id(a));
    let Some((u, at)) = app
        .snapshot
        .unit(unit_id)
        .and_then(|u| u.attempt(a).map(|at| (u, at)))
    else {
        return Explained {
            title,
            hint: format!("A try at {unit_id}'s Rust."),
            shorter: Vec::new(),
            card: vec![
                format!("A try at {unit_id}'s Rust."),
                "It is not in the last read of the records.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let tags = crate::app::attempt_tags(u, at);
    let human = at.is_human();
    let chat = at.record.requester.is_some();
    let who = if human {
        format!("A hand edit: your own Rust for {unit_id}")
    } else if chat {
        format!("A try: the Rust a model wrote for {unit_id} (asked in chat)")
    } else {
        format!("A try: the Rust a model wrote for {unit_id}")
    };
    let outcome = outcome_clause(&at.record.outcome);
    let tag_clause = if tags.iter().any(|t| t.starts_with("in use")) {
        Some("in use means it is the piece's Rust now")
    } else if tags.iter().any(|t| t == "superseded") {
        Some("superseded means a later try replaced it")
    } else if tags.iter().any(|t| t == "of older C") {
        Some("of older C means the C changed since it ran")
    } else {
        None
    };
    let hint = fit(&[
        match tag_clause {
            Some(t) => format!("{who}; {outcome}; {t}."),
            None => format!("{who}; {outcome}."),
        },
        format!("{who}; {outcome}."),
    ]);
    let turns = at.record.turns.len();
    let dir = harness_core::attempts::attempt_dir(&app.snapshot.ledger(), unit_id, a);
    let mut card = vec![if human {
        format!("Your hand edit of {unit_id}'s Rust, checked by the judge.")
    } else {
        format!(
            "A try at {unit_id}'s Rust by the model {}, in {}.",
            at.record.model,
            plural(turns, "turn", "turns")
        )
    }];
    card.push(if chat {
        "Asked in chat.".into()
    } else if human {
        "Recorded from the cockpit's Hand edit or the command line.".into()
    } else {
        "Asked from the cockpit or the command line.".into()
    });
    let outcome_line = outcome.to_string();
    card.push(format!(
        "{}{}.",
        outcome_line[..1].to_uppercase(),
        &outcome_line[1..]
    ));
    if let Some(t) = tag_clause {
        card.push(format!("{}{}.", t[..1].to_uppercase(), &t[1..]));
    }
    card.push(format!(
        "Why its own folder: each try keeps its Rust apart, in {}/, so tries can be compared; \
         Accept copies the one you choose into the piece's Rust.",
        rel(app, &dir)
    ));
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn features(app: &App) -> Explained {
    let path = rel(
        app,
        &harness_core::features::features_path(&app.snapshot.ledger()),
    );
    let (title, hint) = match &app.features.group {
        FeatureGroup::NoFile => (
            "Features (none yet)".to_string(),
            "Your own checks of what the program must keep doing; none written yet.".to_string(),
        ),
        FeatureGroup::Invalid(_) => (
            "Features (error)".to_string(),
            "Your own checks of what the program must keep doing; the file has an error to fix."
                .to_string(),
        ),
        FeatureGroup::Valid => {
            let n = app.features.features.len();
            (
                format!("Features ({n})"),
                fit(&[
                    format!(
                        "Your own checks of what the program must keep doing: {}, run on every \
                     check of a piece.",
                        plural(n, "feature", "features")
                    ),
                    format!(
                        "Your own checks of the program: {}.",
                        plural(n, "feature", "features")
                    ),
                ]),
            )
        }
    };
    let card = vec![
        "Your features: what a person notices the program do, in your own words.".into(),
        "Each is one or more runs of the whole program (its flags, an input).".into(),
        "Every check of a piece then runs them on the C and on the program with that piece's \
         Rust, and compares what a person would see."
            .into(),
        format!("Their file: {path}"),
    ];
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn feature(app: &App, id: &str) -> Explained {
    let Some(f) = app.features.feature(id) else {
        return Explained {
            title: id.to_string(),
            hint: format!("{id}: one of your features."),
            shorter: Vec::new(),
            card: vec![
                format!("{id}: one of your features."),
                "It is not in the last read of the features file.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let name = crate::display::line(&f.name);
    let runs = plural(f.scenarios.len(), "run", "runs");
    let pieces = plural(f.units.len(), "piece", "pieces");
    let hint = fit(&[
        format!(
            "{name}: one of your features, {runs} of the whole program; it reaches {pieces}; \
             {}.",
            f.state.word()
        ),
        format!("{name}: one of your features; {}.", f.state.word()),
        format!("One of your features; {}.", f.state.word()),
    ]);
    let card = vec![
        format!("One of your features: {name} (its id: {id})."),
        format!("It is {runs} of the whole program; the map found it runs {pieces}."),
        format!("Now: {}.", f.state.word()),
        "Every check of a piece it runs compares it on the C and on that piece's Rust.".into(),
    ];
    Explained {
        title: name,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn speed(app: &App) -> Explained {
    let path = rel(
        app,
        &harness_core::perf::workloads::workloads_path(&app.snapshot.ledger()),
    );
    let state = match &app.speed.group {
        SpeedGroup::NoFile => "none named yet".to_string(),
        SpeedGroup::NoWorkload => "the file names no workload yet".into(),
        SpeedGroup::FileError(_) => "the workloads file has an error to fix".into(),
        SpeedGroup::NotYetRun => "nothing measured yet".into(),
        SpeedGroup::COnly => "only the C measured so far".into(),
        SpeedGroup::Units { measured, of } => {
            format!("{measured} of the {of} pieces in Rust measured")
        }
    };
    let hint = fit(&[
        format!(
            "How fast the program runs with its Rust against the C, on workloads you name; \
         {state}."
        ),
        format!("Speed of the Rust against the C: {state}."),
    ]);
    let card = vec![
        "Speed: the program's run time with its Rust, against the C alone.".into(),
        "You name the workloads: runs of the program with its flags and an input.".into(),
        "Each is run on the C, with each piece's Rust in its place, and with all of them; \
         your C is not changed."
            .into(),
        format!("Now: {state}."),
        format!("The workloads file: {path}"),
    ];
    Explained {
        title: app.speed.label(),
        hint,
        shorter: Vec::new(),
        card,
    }
}

// ----- the Project view ---------------------------------------------------------

fn map_row(app: &App) -> Explained {
    let again = app.project_map().is_some();
    let title = if again {
        "Map the project again".to_string()
    } else {
        "Map the project".to_string()
    };
    let hint = if again {
        "Map the project again: read its programs and files once more, with its build \
         configuration as it is now."
            .to_string()
    } else {
        "Map the project: find its programs and libraries, the files each needs, and whether \
         each program links."
            .to_string()
    };
    let card = vec![
        "Mapping reads the project's C files and finds its programs (each file with a main) \
         and its libraries."
            .into(),
        "It compiles them in the sandbox to see what each program needs and whether it links; \
         it runs nothing."
            .into(),
        "Its map: migration/map/project-map.json; your files are not changed.".into(),
    ];
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn program(app: &App, id: &str) -> Explained {
    let title = id.to_string();
    let Some(o) = app.project_offered(id) else {
        return Explained {
            title,
            hint: format!("{id}: a program of the project."),
            shorter: Vec::new(),
            card: vec![
                format!("{id}: a program of the project."),
                "It is not in the last read of the map.".into(),
                "g reads the project again.".into(),
            ],
        };
    };
    let accepted = app.project_tools().iter().any(|t| t == id);
    let path = o.path.clone();
    let found = if o.kind == "library" {
        format!("a library the map found (from {path})")
    } else {
        format!("a program the map found ({path} has a main)")
    };
    let state = if accepted {
        "it is accepted: a tool you can open".to_string()
    } else {
        match o.state {
            State::Linked => "it links, so it can be accepted".into(),
            State::Held => "it needs a decision before it can be accepted".into(),
            State::DidNotCompile => "it did not compile; its build likely needs more flags".into(),
            State::DidNotLink => "it did not link; its build likely needs more flags".into(),
            State::Incomplete => "the map could not tell every file it needs".into(),
            State::Fuzzer | State::Driver => "a fuzzer, not a tool".into(),
            State::Compiles => "its files compile, so it can be accepted".into(),
        }
    };
    let hint = fit(&[
        format!("{id}: {found}; {state}."),
        format!("{id}: a {} of the project; {state}.", o.noun()),
    ]);
    let mut card = vec![format!("{path} · {}.", o.what()), format!("Now: {state}.")];
    if !accepted {
        if !o.held.is_empty() {
            card.push("Two files define the same functions: Accept asks which one to keep.".into());
        }
        if !o.problem.is_empty() {
            card.push(format!("What failed: {}", o.problem));
        }
        card.push(
            "Accepting it makes it a tool: you then scan it, plan it and move its pieces.".into(),
        );
    } else if let Some(p) = app.project.as_ref().and_then(|p| p.progress.get(id)) {
        card.push(format!("Its progress: {}.", p.words()));
    }
    card.push("The project's map: migration/map/project-map.json".into());
    Explained {
        title,
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn tool(app: &App, id: &str) -> Explained {
    let progress = app
        .project
        .as_ref()
        .and_then(|p| p.progress.get(id))
        .map(|p| p.words());
    let hint = fit(&[
        match &progress {
            Some(p) => format!("{id}: an accepted program of the project, a tool; {p}."),
            None => format!("{id}: an accepted program of the project, a tool."),
        },
        format!("{id}: an accepted tool; Enter opens it."),
    ]);
    let mut card = vec![format!(
        "{id}: a program of the project, accepted as a tool."
    )];
    if let Some(p) = progress {
        card.push(format!("Its progress: {p}."));
    }
    card.push("Open it to scan it, plan it and move its pieces to Rust.".into());
    card.push(format!(
        "Its build configuration: migration/tools/{id}/{}",
        harness_core::config::CONFIG_FILE
    ));
    Explained {
        title: id.to_string(),
        hint,
        shorter: Vec::new(),
        card,
    }
}

fn home(app: &App) -> Explained {
    let tool = app
        .config
        .tool
        .clone()
        .unwrap_or_else(|| "this tool".into());
    Explained {
        title: "Back to the project view".into(),
        hint: "The project's other programs and their decisions.".into(),
        shorter: Vec::new(),
        card: vec![
            format!("{tool} is one program of the project {}.", folder(app)),
            "The project view lists every program and library the map found.".into(),
            "From there you map the project again, accept another program, or open another \
             tool."
                .into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app, app_of, HARNESS};
    use crate::app::{Config, LayoutMode};
    use crate::testutil::{file_list_tool, TmpDir, LZG_TOOL};
    use std::path::PathBuf;

    /// The words a person must never meet (§9.3, §9.5): the harness's own
    /// names. The build configuration's file name only on a card's path
    /// line.
    pub(crate) fn banned(text: &str, path_line_ok: bool) -> Option<&'static str> {
        let lower = text.to_lowercase();
        if lower.contains("harness.toml")
            && !(path_line_ok && text.starts_with("Its build configuration: "))
        {
            return Some("harness.toml");
        }
        [
            "stated",
            "held",
            "choice d1",
            "d1.1",
            "d1.2",
            "ledger",
            "oracle",
            "verdict",
            "alternative-implementation",
            "cannot-tell",
        ]
        .into_iter()
        .find(|w| lower.contains(w))
    }

    /// Every hint under the limit, every card three to eight lines, none
    /// holding a banned name.
    fn check(app: &App, sel: &Selection) {
        let e = explain(app, sel);
        eprintln!("DBG {sel:?}\n  H: {}\n  {}", e.hint, e.card.join("\n  "));
        assert!(!e.hint.is_empty(), "{sel:?}: no hint");
        assert!(
            e.hint.chars().count() < 160,
            "{sel:?}: {} characters: {}",
            e.hint.chars().count(),
            e.hint
        );
        assert!(!e.hint.contains('…'), "{sel:?}: cut: {}", e.hint);
        assert_eq!(banned(&e.hint, false), None, "{sel:?}: {}", e.hint);
        assert!(
            (3..=8).contains(&e.card.len()),
            "{sel:?}: {} card lines: {:#?}",
            e.card.len(),
            e.card
        );
        for line in &e.card {
            assert_eq!(banned(line, true), None, "{sel:?}: {line}");
        }
        assert!(!e.title.is_empty(), "{sel:?}: no title");
    }

    /// Every node a tool's screen can show: the project, the groups, each
    /// unit with its crate and tries, each feature, Speed, every folder,
    /// file and function.
    fn every_row(app: &App) -> Vec<Selection> {
        let mut all = vec![
            Selection::Project,
            Selection::Units,
            Selection::Features,
            Selection::Speed,
        ];
        for u in &app.snapshot.units {
            let id = u.unit.id.clone();
            all.push(Selection::Unit(id.clone()));
            // A crate row is shown only when its Rust exists.
            if u.crate_dir.is_some() {
                all.push(Selection::Crate(id.clone()));
            }
            for a in &u.attempts {
                all.push(Selection::Attempt(id.clone(), a.record.id.clone()));
            }
        }
        for f in &app.features.features {
            all.push(Selection::Feature(f.id.clone()));
        }
        let mut dirs = std::collections::BTreeSet::new();
        for f in &app.files.files {
            let mut at = 0;
            while let Some(i) = f.path[at..].find('/') {
                dirs.insert(f.path[..at + i].to_string());
                at += i + 1;
            }
            all.push(Selection::File(f.path.clone()));
            for func in &f.functions {
                all.push(Selection::Function(f.path.clone(), func.name.clone()));
            }
        }
        all.extend(dirs.into_iter().map(Selection::Dir));
        all
    }

    #[test]
    fn every_row_of_zopfli_and_the_tractor_case_is_explained() {
        for app in [
            app_of("targets/zopfli", "explain-zopfli"),
            app("explain-tractor"),
        ] {
            let rows = every_row(&app);
            assert!(rows.len() > 10, "{rows:?}");
            for sel in &rows {
                check(&app, sel);
            }
        }
    }

    /// The crate row is shown only for a unit whose Rust exists — never
    /// "no Rust yet", never under a piece that stays C.
    #[test]
    fn a_crate_row_only_when_its_rust_exists() {
        let mut app = app_of("targets/zopfli", "explain-crate-row");
        for id in ["u001-katajainen", "u-lz77", "u-cache"] {
            app.expansion.set(&Selection::Unit(id.into()), true);
        }
        app.rebuild_rows();
        let crates: Vec<Selection> = app
            .rows
            .iter()
            .filter_map(|r| r.selection().cloned())
            .filter(|s| matches!(s, Selection::Crate(_)))
            .collect();
        assert_eq!(crates, [Selection::Crate("u001-katajainen".into())]);
        assert!(!crate::tree::exists(
            &app.snapshot,
            &app.files,
            &app.features,
            &Selection::Crate("u-lz77".into())
        ));
    }

    #[test]
    fn a_file_list_tools_rows_and_the_way_back_are_explained() {
        let dir = file_list_tool("explain-file-list");
        let read = crate::load::read_tool(&dir.0, Some(LZG_TOOL)).unwrap();
        let mut app = App::new(
            Config {
                target: dir.0.clone(),
                tool: Some(LZG_TOOL.into()),
                harness: Some(PathBuf::from(HARNESS)),
                allow_unsandboxed: false,
                layout: LayoutMode::Auto,
                providers: vec!["external".into()],
            },
            read,
        );
        app.join_project();
        let mut rows = every_row(&app);
        rows.push(Selection::ProjectHome);
        for sel in &rows {
            check(&app, sel);
        }
        // A file the program never reaches says why, with the file the map
        // followed.
        assert_eq!(
            hint(&app, &Selection::File("src/other/decode.c".into())),
            "decode.c is in the project, but t-lzg never calls into it: the map followed what \
             lzg.c links and this file is not reached."
        );
        assert_eq!(
            hint(&app, &Selection::File("src/include/lzg.h".into())),
            "A header: declarations only; nothing to move."
        );
        // The tool's card names its build configuration on its path line.
        let card = explain(&app, &Selection::Project).card;
        assert!(
            card.contains(&"Its build configuration: migration/tools/t-lzg/harness.toml".into()),
            "{card:#?}"
        );
    }

    #[test]
    fn the_sentences_say_what_a_row_is() {
        let app = app_of("targets/zopfli", "explain-words");
        assert_eq!(
            hint(&app, &Selection::Unit("u-lz77".into())),
            "u-lz77 calls u-cache and u-hash; in this version the judge tests each piece alone, \
             so a piece that calls other pieces stays C."
        );
        // No record says which try made it: the sentence leaves it out.
        assert_eq!(
            hint(&app, &Selection::Crate("u001-katajainen".into())),
            "The Rust for u001-katajainen; the C stays beside it so the judge can compare the \
             two."
        );
        let card = explain(&app, &Selection::Crate("u001-katajainen".into())).card;
        assert!(
            card[0].starts_with("The Rust of u001-katajainen: a Rust crate in migration/units/"),
            "{card:#?}"
        );
        assert!(
            card.iter().any(|l| l.starts_with("Why the C stays: ")),
            "{card:#?}"
        );
        let speed = hint(&app, &Selection::Speed);
        assert!(
            speed.starts_with(
                "How fast the program runs with its Rust against the C, on workloads you name"
            ),
            "{speed}"
        );
        // A try says what its words mean.
        let tractor = crate::app::tests::app("explain-try");
        assert_eq!(
            hint(
                &tractor,
                &Selection::Attempt("u-lib".into(), crate::app::tests::PROVENANCE.into()),
            ),
            "A try: the Rust a model wrote for u-lib; green means every check passed; in use \
             means it is the piece's Rust now."
        );
        assert_eq!(
            hint(&tractor, &Selection::Crate("u-lib".into())),
            "The Rust for u-lib, written by try a-13c9; the C stays beside it so the judge can \
             compare the two."
        );
    }

    #[test]
    fn every_row_of_the_project_view_is_explained() {
        use crate::project::tests::{map_text, write_map};
        let tmp = TmpDir::new("explain-project");
        let root = tmp.0.join("liblzg");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.c"), "int main(void) { return 0; }\n").unwrap();
        let config = Config {
            target: root.clone(),
            tool: None,
            harness: Some(PathBuf::from(HARNESS)),
            allow_unsandboxed: false,
            layout: LayoutMode::Auto,
            providers: vec!["external".into()],
        };
        // Before a map: the one row.
        let app = App::new_project(config.clone());
        check(&app, &Selection::MapProject);
        // After a map with a decision, and one program accepted.
        write_map(&root, &map_text("stated", false));
        let tool = harness_core::config::tool_dir(&root, "t-lzg");
        std::fs::create_dir_all(&tool).unwrap();
        std::fs::write(tool.join("harness.toml"), "schema_version = 2\n").unwrap();
        let app = App::new_project(config);
        let rows: Vec<Selection> = app
            .project_rows()
            .iter()
            .filter_map(|r| r.selection().cloned())
            .collect();
        assert!(rows.len() >= 5, "{rows:?}");
        for sel in rows.iter().chain([&Selection::Tool("t-lzg".into())]) {
            check(&app, sel);
        }
        assert_eq!(
            hint(&app, &Selection::Program("t-unlzg".into())),
            "t-unlzg: a program the map found (tools/unlzg.c has a main); it needs a decision \
             before it can be accepted."
        );
    }
}
