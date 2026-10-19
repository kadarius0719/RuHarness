//! The Project view (docs/PROJECT-MAP-DESIGN.md §3.7; pass 3 of
//! docs/reviews/2026-10-18-cockpit-usability-critique.md): the cockpit's
//! own screen on an untouched project — before a map, after a map with a
//! held choice, after an accept — its menus, its dialogs, and a tool's row
//! back to it.

use super::*;
use crate::app::tests::{code, dialog_argv, HARNESS};
use crate::app::{Config, LayoutMode};
use crate::project::tests::{map_text, write_map};
use crate::testutil::TmpDir;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;

/// An untouched C project named `liblzg` (one `.c`, no `harness.toml`).
fn project(tag: &str) -> (TmpDir, PathBuf) {
    let tmp = TmpDir::new(&format!("pv-{tag}"));
    let root = tmp.0.join("liblzg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.c"), "int main(void) { return 0; }\n").unwrap();
    (tmp, root)
}

fn cockpit(root: &Path) -> App {
    App::new_project(Config {
        target: root.to_path_buf(),
        tool: None,
        harness: Some(PathBuf::from(HARNESS)),
        allow_unsandboxed: false,
        layout: LayoutMode::Auto,
        providers: vec!["external".into()],
    })
}

fn menu_labels(app: &App) -> Vec<String> {
    match &app.mode {
        Mode::Menu(m) => m.items.iter().map(|i| i.label.clone()).collect(),
        other => panic!("no menu: {other:?}"),
    }
}

fn menu(app: &App) -> &Menu {
    match &app.mode {
        Mode::Menu(m) => m,
        other => panic!("no menu: {other:?}"),
    }
}

/// Choose the open menu's item labelled `label`.
fn choose(app: &mut App, label: &str) -> Command {
    let at = menu_labels(app)
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("no item {label:?} in {:?}", menu_labels(app)));
    for _ in 0..at {
        code(app, KeyCode::Down);
    }
    code(app, KeyCode::Enter)
}

fn selectable(app: &App) -> Vec<Selection> {
    app.rows
        .iter()
        .filter_map(|r| r.selection().cloned())
        .collect()
}

/// Before a map: one row, "Map the project"; the right pane names the
/// folder (never its full path) and says the Next step; Enter's menu holds
/// Map alone, recommended; its dialog says what it runs, how long it takes
/// and what it writes, and `y` runs exactly that command, no `--json`.
#[test]
fn the_project_view_before_a_map() {
    let (_tmp, root) = project("before");
    let mut app = cockpit(&root);
    assert!(app.in_project_view());
    assert_eq!(selectable(&app), [Selection::MapProject]);
    let buffer = render(&mut app, 120, 30);
    golden("project-before-map.txt", &app, &buffer);
    let screen = text(&buffer);
    assert!(screen.contains("┌ Project "), "{screen}");
    assert!(
        screen.contains(" liblzg · project, not mapped yet "),
        "{screen}"
    );
    assert!(
        screen.contains("Next step: No map yet — press Enter and choose Map the project"),
        "{screen}"
    );
    assert!(
        !screen.contains(&root.parent().unwrap().display().to_string()),
        "the full path is never the heading: {screen}"
    );
    assert!(!screen.contains("Type a number"), "{screen}");
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app), ["Map the project"]);
    assert_eq!(menu(&app).reason.as_deref(), Some("No map yet"));
    // The dialog, on a fixed path so the golden is the machine's own.
    app.mode = Mode::Normal;
    app.config.target = PathBuf::from("/work/liblzg");
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Map the project");
    let buffer = render(&mut app, 120, 30);
    golden("project-map-dialog.txt", &app, &buffer);
    let Mode::Dialog(c) = &app.mode else {
        panic!("no dialog: {:?}", app.mode)
    };
    assert_eq!(c.title, "Map the project?");
    assert_eq!(
        c.body,
        [
            "Map the project: find its programs and libraries, the files each one needs, what \
             they share, and whether each program links (its code is compiled and linked in the \
             sandbox, never run).",
            "It runs: harness project map --target /work/liblzg",
            "It takes: a few seconds for a small project, minutes for a large one (at most 30 \
             minutes).",
            "It writes: migration/map/project-map.json (and migration/.gitignore the first \
             time); the project's own files are not changed.",
        ]
    );
    assert_eq!(
        dialog_argv(&app),
        [HARNESS, "project", "map", "--target", "/work/liblzg"]
    );
    arm(&mut app);
    let Command::Spawn(p) = key(&mut app, 'y') else {
        panic!("y did not run it")
    };
    assert_eq!(p.act, Act::ProjectMap);
    assert!(!p.argv.iter().any(|a| a == "--json"));
}

/// A project act's output is the command line's words in the activity
/// details; its end re-reads the map, and the rows follow.
#[test]
fn a_map_run_shows_its_words_and_the_rows_follow() {
    let (_tmp, root) = project("run");
    let mut app = cockpit(&root);
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Map the project");
    arm(&mut app);
    let Command::Spawn(p) = key(&mut app, 'y') else {
        panic!("y did not run it")
    };
    app.on_spawned(&p);
    for line in ["programs: 3", "  p1 t-lzg — tools/lzg.c (main)"] {
        app.on_child_msg(crate::spawn::ChildMsg::Event(crate::events::parse_line(
            line,
        )));
    }
    write_map(&root, &map_text("stated", false));
    app.on_child_exit(std::process::ExitStatus::from_raw(0));
    let last = app.last.clone().unwrap();
    assert!(
        last.starts_with("Map the project — done (") && last.ends_with("c for its words"),
        "{last}"
    );
    let lines: Vec<String> = app
        .run
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|l| l.text.clone())
        .collect();
    assert_eq!(lines, ["programs: 3", "  p1 t-lzg — tools/lzg.c (main)"]);
    assert_eq!(app.run.as_ref().unwrap().exit.as_deref(), Some("exit 0"));
    assert!(selectable(&app).contains(&Selection::Program("t-unlzg".into())));
    key(&mut app, 'c');
    let screen = text(&render(&mut app, 120, 30));
    assert!(
        screen.contains("  p1 t-lzg — tools/lzg.c (main)"),
        "{screen}"
    );
    assert!(!screen.contains("? programs"), "{screen}");
}

/// While the configuration is a guess: the Next step names the build or
/// Ask for a proposal; Ask leads every menu; Accept waits, greyed with why.
#[test]
fn a_guessed_configuration_asks_for_the_build() {
    let (_tmp, root) = project("guess");
    write_map(&root, &map_text("guessed", false));
    let mut app = cockpit(&root);
    let screen = text(&render(&mut app, 220, 40));
    assert!(
        screen.contains(
            "Next step: The configuration is a guess — state the build in \
             migration/map/config.toml"
        ) || screen.contains("Next step: The configuration is a guess"),
        "{screen}"
    );
    assert!(
        screen
            .contains("Configuration: a guess — no migration/map/config.toml states the build yet"),
        "{screen}"
    );
    app.select(Selection::Program("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            "Ask for a proposal",
            "Accept this program",
            "Accept it under a run name of your own…",
            "Map the project again",
        ]
    );
    let accept = &menu(&app).items[1];
    assert_eq!(
        accept.greyed.as_deref(),
        Some(crate::project::ACCEPT_NEEDS_STATED)
    );
    choose(&mut app, "Ask for a proposal");
    assert!(dialog_argv(&app).ends_with(&["--build".to_string()]));
    let Mode::Dialog(c) = &app.mode else { panic!() };
    assert_eq!(c.title, "Ask a model for a proposal?");
}

/// After a map with a held choice: the programs as rows with their kind
/// guess and their state in words, each file under its row, the libraries,
/// the file that did not compile; the right pane in words; Enter's menu on
/// the held program puts Accept first; the picker asks each reached set,
/// the model's advice beside it, then the dialog runs accept with the
/// person's keeps.
#[test]
fn the_project_view_after_a_map_with_a_held_choice() {
    let (_tmp, root) = project("held");
    write_map(&root, &map_text("stated", false));
    std::fs::write(
        root.join(crate::project::REPLY_FILE),
        r#"{"schema": "ruharness-project-map-reply", "root_hash": "blake3:aa",
            "inputs_hash": "blake3:bb", "items": {
            "d1": {"keep": "d1.2", "reason": "alternative-implementation",
                   "model": "claude-x", "provider": "external"}}}"#,
    )
    .unwrap();
    let mut app = cockpit(&root);
    app.select(Selection::Program("t-unlzg".into()));
    let buffer = render(&mut app, 120, 34);
    golden("project-held.txt", &app, &buffer);
    let screen = text(&render(&mut app, 220, 44));
    for words in [
        "Programs (4)",
        "✓ t-lzg (tool)",
        "⚠ t-unlzg (tool)",
        "held on a choice",
        "tools/unlzg.c",
        "tools/broken.c — did not compile",
        "Libraries (2)",
        "l-bad (library)",
        "Configuration: stated (make)",
        "Next step: t-unlzg holds a choice — press Enter on it and choose Accept this program",
        "Choice d1: linking cannot tell its files apart (d1.1 src/decode.c, d1.2 src/mini.c)",
        "The model's advice (claude-x): keep d1.2",
        "Accepted tools: none yet",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            "Accept this program",
            "Accept it under a run name of your own…",
            "Map the project again",
            "Ask a model for advice",
        ]
    );
    assert_eq!(menu(&app).reason.as_deref(), Some("t-unlzg holds a choice"));
    assert!(menu(&app).items[3].model);
    // The picker: d1, its definers none suggested, the advice beside them.
    choose(&mut app, "Accept this program");
    let m = menu(&app);
    assert_eq!(m.title.as_deref(), Some("Accept t-unlzg"));
    assert_eq!(
        m.note,
        [
            "t-unlzg holds the choice d1: linking cannot tell its files apart, so keep which \
             one?",
            "The model's advice (claude-x): keep d1.2, reason alternative-implementation — \
             advice only, the choice is yours.",
        ]
    );
    assert_eq!(menu_labels(&app), ["d1.1 src/decode.c", "d1.2 src/mini.c"]);
    assert!(m.reason.is_none(), "nothing is recommended in the picker");
    let buffer = render(&mut app, 120, 34);
    golden("project-picker.txt", &app, &buffer);
    // d1.1 kept: d2, reached only under it, is asked next.
    choose(&mut app, "d1.1 src/decode.c");
    assert_eq!(
        menu_labels(&app),
        ["d2.1 src/check_a.c", "d2.2 src/check_b.c"]
    );
    choose(&mut app, "d2.2 src/check_b.c");
    let target = root.display().to_string();
    assert_eq!(
        dialog_argv(&app),
        [
            HARNESS, "project", "accept", "t-unlzg", "--target", &target, "--keep", "d1=d1.1",
            "--keep", "d2=d2.2"
        ]
    );
    let Mode::Dialog(c) = &app.mode else { panic!() };
    assert_eq!(c.title, "Accept t-unlzg?");
    assert!(c.body[0].starts_with("Accept a program: check the map still matches the project"));
    assert!(c.body[3].starts_with("It writes: migration/tools/<id>/harness.toml only"));
    // d1.2 kept: d2 is not reached, the dialog follows at once.
    app.mode = Mode::Normal;
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Accept this program");
    choose(&mut app, "d1.2 src/mini.c");
    assert!(dialog_argv(&app).ends_with(&["--keep".to_string(), "d1=d1.2".to_string()]));
}

/// A run name of the person's, typed after the picks; a name the command
/// line would refuse is said so, and typing goes on.
#[test]
fn accept_under_a_run_name() {
    let (_tmp, root) = project("runname");
    write_map(&root, &map_text("stated", false));
    let mut app = cockpit(&root);
    app.select(Selection::Program("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app)[0], "Accept this program");
    assert_eq!(
        menu(&app).reason.as_deref(),
        Some("t-lzg links and is ready")
    );
    choose(&mut app, "Accept it under a run name of your own…");
    assert!(matches!(app.mode, Mode::RunName { .. }), "{:?}", app.mode);
    let screen = text(&render(&mut app, 120, 30));
    assert!(screen.contains("Accept t-lzg: its run name"), "{screen}");
    key(&mut app, '-');
    code(&mut app, KeyCode::Enter);
    assert!(matches!(app.mode, Mode::RunName { .. }));
    code(&mut app, KeyCode::Backspace);
    for c in "lzg2".chars() {
        key(&mut app, c);
    }
    code(&mut app, KeyCode::Enter);
    assert!(dialog_argv(&app).ends_with(&["--run-name".to_string(), "lzg2".to_string()]));
}

/// After an accept: the tool keeps a row "Open tool <id> — <path>" whose
/// Enter opens it; the Next step moves on.
#[test]
fn the_project_view_after_an_accept() {
    let (_tmp, root) = project("accepted");
    write_map(&root, &map_text("stated", false));
    let dir = harness_core::config::tool_dir(&root, "t-lzg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("harness.toml"), "schema_version = 2\n").unwrap();
    let mut app = cockpit(&root);
    let buffer = render(&mut app, 120, 34);
    golden("project-accepted.txt", &app, &buffer);
    let screen = text(&render(&mut app, 220, 44));
    for words in [
        "Accepted tools (1)",
        "Open tool t-lzg — tools/lzg.c",
        "t-lzg — tools/lzg.c — Enter on its row opens it",
        "t-lzg — tools/lzg.c (tool): accepted",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    app.select(Selection::Tool("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            "Open tool t-lzg",
            "Map the project again",
            "Ask a model for advice"
        ]
    );
    assert_eq!(
        choose(&mut app, "Open tool t-lzg"),
        Command::OpenTool("t-lzg".into())
    );
    // Not while a command runs.
    app.running = true;
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu(&app).items[0].greyed.as_deref(),
        Some("a command is running (one at a time)")
    );
}

/// A tool's screen on a project keeps a row back to the Project view —
/// first in its tree — whose Enter goes back; opening the tool from the
/// Project view gives the same row.
#[test]
fn a_tools_screen_comes_back_to_the_project_view() {
    // Under a fixed name, so the golden does not depend on the pid.
    let p = crate::testutil::file_list_tool("pv-home");
    let outer = TmpDir::new("pv-home-outer");
    let root = outer.0.join("liblzg");
    std::fs::rename(&p.0, &root).unwrap();
    harness_core::adopt::testing::adopt(&root);
    let config = Config {
        target: root.clone(),
        tool: Some(crate::testutil::LZG_TOOL.into()),
        harness: Some(PathBuf::from(HARNESS)),
        allow_unsandboxed: false,
        layout: LayoutMode::Auto,
        providers: vec!["external".into()],
    };
    let read = crate::load::read_tool(&root, Some(crate::testutil::LZG_TOOL)).unwrap();
    let mut app = App::new(config.clone(), read);
    app.join_project();
    assert_eq!(app.rows[0].selection(), Some(&Selection::ProjectHome));
    assert_eq!(app.selection, Selection::Project);
    let screen = text(&render(&mut app, 120, 30));
    assert!(
        row_with(&screen, "◂ Back to the project view").starts_with("│"),
        "{screen}"
    );
    app.select(Selection::ProjectHome);
    let buffer = render(&mut app, 120, 30);
    golden("tool-come-back.txt", &app, &buffer);
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app), ["Back to the project view"]);
    choose(&mut app, "Back to the project view");
    assert!(app.in_project_view());
    assert_eq!(app.config.tool, None);
    assert_eq!(app.selection, Selection::Tool("t-lzg".into()));
    // And into the tool again, as the event loop does on Open tool.
    let read = crate::load::read_tool(&root, Some("t-lzg")).unwrap();
    app.open_tool("t-lzg".into(), read);
    assert!(!app.in_project_view());
    assert_eq!(app.config.tool.as_deref(), Some("t-lzg"));
    assert_eq!(app.rows[0].selection(), Some(&Selection::ProjectHome));
    assert!(app.chat.tool.as_deref() == Some("t-lzg"));
    // A running command holds the screen: going back says why.
    app.running = true;
    app.select(Selection::ProjectHome);
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu(&app).items[0].greyed.as_deref(),
        Some("a command is running (one at a time)")
    );
}
