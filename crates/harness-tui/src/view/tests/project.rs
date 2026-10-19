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
pub(super) fn project(tag: &str) -> (TmpDir, PathBuf) {
    let tmp = TmpDir::new(&format!("pv-{tag}"));
    let root = tmp.0.join("liblzg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.c"), "int main(void) { return 0; }\n").unwrap();
    (tmp, root)
}

pub(super) fn cockpit(root: &Path) -> App {
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

/// The build configuration's menu item, with this machine's editor.
fn write_item(app: &App) -> String {
    format!(
        "{} (in {})",
        app.build_config_label(),
        crate::app::features_edit::editor_name(&app.features_editor())
    )
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
        screen.contains("Next step: No map yet — press Enter to map the project"),
        "{screen}"
    );
    assert!(
        !screen.contains(&root.parent().unwrap().display().to_string()),
        "the full path is never the heading: {screen}"
    );
    assert!(!screen.contains("Type a number"), "{screen}");
    // The menu would hold only the Next step's own item: one Enter opens
    // its dialog (the newcomer's ten minutes, finding 9). On a fixed path
    // so the golden is the machine's own.
    app.config.target = PathBuf::from("/work/liblzg");
    assert_eq!(app.project_menu_items().len(), 1);
    code(&mut app, KeyCode::Enter);
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

/// A run is never "done" when it failed: a map that printed an error says
/// "done, with a problem" and the error; one that exited 1 says "failed
/// (exit 1)" and its first error line — on the status line and in the
/// details. When the folder is then refused as made elsewhere, the cockpit
/// asks its adoption question again (one plain sentence first), the Next
/// step and the menu offer it until it is answered, and Adopt adopts.
#[test]
fn a_failed_run_says_so_and_offers_the_adoption_question() {
    harness_core::adopt::testing::adoption_file();
    let (_tmp, root) = project("fail");
    let mut app = cockpit(&root);
    let run = |app: &mut App, stderr: &str, status: i32| {
        arm(app);
        let Command::Spawn(p) = key(app, 'y') else {
            panic!("y did not run it: {:?}", app.mode)
        };
        app.on_spawned(&p);
        app.on_child_msg(crate::spawn::ChildMsg::Event(crate::events::parse_line(
            "project map: wrote migration/map/project-map.json",
        )));
        app.on_child_msg(crate::spawn::ChildMsg::Stderr(stderr.into()));
        app.on_child_exit(std::process::ExitStatus::from_raw(status));
    };
    code(&mut app, KeyCode::Enter);
    // The map's own files: from now on the folder holds results this
    // computer did not record.
    guessed_map(&root);
    run(
        &mut app,
        "harness: could not record /x as made on this computer (parse error); the next command \
         will ask for `--adopt`",
        0,
    );
    let last = app.last.clone().unwrap();
    assert!(
        last.starts_with(
            "Map the project — done, with a problem: could not record /x as made on this \
             computer (parse error)"
        ) && last.ends_with("c for its words"),
        "{last}"
    );
    assert!(app
        .run
        .as_ref()
        .unwrap()
        .exit
        .as_deref()
        .unwrap()
        .starts_with("done, with a problem: could not record /x"));
    // The adoption question, in the cockpit's own dialog.
    let Mode::Dialog(c) = &app.mode else {
        panic!("no adoption dialog: {:?}", app.mode)
    };
    assert_eq!(c.title, "Adopt this folder?");
    assert_eq!(c.body[0], crate::adoption::LEAD);
    assert!(c
        .body
        .iter()
        .any(|b| b.contains("this folder already holds migration results made elsewhere")));
    let screen = text(&render(&mut app, 120, 34));
    assert!(screen.contains("[ Adopt  y ]"), "{screen}");
    // Esc: said, and offered again by the Next step and the menu.
    code(&mut app, KeyCode::Esc);
    assert!(matches!(app.mode, Mode::Normal));
    let next = app.project_next().unwrap();
    assert!(
        next.text
            .starts_with("this folder's results were made elsewhere and are not trusted here yet"),
        "{}",
        next.text
    );
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app)[0], "Adopt this folder…");
    // A map run now is refused, exit 1: failed, with the refusal's words.
    choose(&mut app, "Map the project again");
    run(
        &mut app,
        "Error: /x: this folder already holds migration results made elsewhere (0 units, 0 \
         verified): to trust them here, add `--adopt` once",
        1 << 8,
    );
    let last = app.last.clone().unwrap();
    assert!(
        last.starts_with(
            "Map the project — failed (exit 1): /x: this folder already holds migration results \
             made elsewhere"
        ),
        "{last}"
    );
    assert!(app
        .run
        .as_ref()
        .unwrap()
        .exit
        .as_deref()
        .unwrap()
        .starts_with("failed (exit 1): /x: this folder already holds"));
    // Asked again, not a dead end.
    assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == crate::app::Purpose::Adopt));
    code(&mut app, KeyCode::Esc);
    key(&mut app, 'c');
    let screen = text(&render(&mut app, 120, 34));
    assert!(screen.contains("failed (exit 1)"), "{screen}");
    app.mode = Mode::Normal;
    // Adopt: the folder is trusted, nothing is offered any more.
    app.ask_adopt();
    arm(&mut app);
    assert_eq!(key(&mut app, 'y'), Command::None);
    harness_core::adopt::check(&root).unwrap();
    assert!(app.not_adopted.is_none());
    assert!(
        app.notice.as_ref().unwrap().text.starts_with("adopt: "),
        "{:?}",
        app.notice
    );
}

/// A guessed map whose build files hold a Makefile (its example says
/// `make`).
fn guessed_map(root: &Path) {
    write_map(
        root,
        &map_text("guessed", false).replacen(
            "\"root_hash\"",
            "\"build_evidence\": {\"build_files\": [\"src/Makefile\"]}, \"root_hash\"",
            1,
        ),
    );
}

/// While the configuration is a guess: the Next step names Write the build
/// configuration (or Ask for a proposal); Write leads every menu, with its
/// reason; Ask still sits under the model separator; Accept waits, greyed
/// with why (the newcomer's ten minutes, finding 1, and the one menu rule
/// for model items, finding 6).
#[test]
fn a_guessed_configuration_asks_for_the_build() {
    let (_tmp, root) = project("guess");
    guessed_map(&root);
    let mut app = cockpit(&root);
    let screen = text(&render(&mut app, 220, 40));
    assert!(
        screen.contains(
            "Next step: The configuration is a guess — press Enter and choose Write the build \
             configuration (or Ask for a proposal), then Map the project again"
        ),
        "{screen}"
    );
    assert!(
        screen.contains(
            "Configuration: a guess — no migration/map/config.toml states the build yet (the \
             map compiled with no flags of yours)"
        ),
        "{screen}"
    );
    app.select(Selection::Program("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            write_item(&app),
            "Accept this program".into(),
            "Accept it under a run name of your own…".into(),
            "Map the project again".into(),
            "Ask for a proposal".into(),
        ]
    );
    assert!(write_item(&app).starts_with("Write the build configuration (in "));
    assert_eq!(
        menu(&app).reason.as_deref(),
        Some("the configuration is a guess")
    );
    let accept = &menu(&app).items[1];
    assert_eq!(
        accept.greyed.as_deref(),
        Some(crate::project::ACCEPT_NEEDS_STATED)
    );
    // The recommended item first, "· uses a model" only when it does; the
    // separator heads the other model items.
    let screen = text(&render(&mut app, 120, 34));
    let at = |words: &str| {
        screen
            .lines()
            .position(|l| l.contains(words))
            .unwrap_or_else(|| panic!("{words}:\n{screen}"))
    };
    assert_eq!(
        at("↳ recommended: the configuration is a guess"),
        at("Write the build configuration") + 1
    );
    assert!(!row_with(&screen, "↳ recommended").contains("uses a model"));
    assert!(at("Map the project again") < at("── Uses a model — can take minutes ──"));
    let lines: Vec<&str> = screen.lines().collect();
    let ask = lines
        .iter()
        .rposition(|l| l.contains("  Ask for a proposal"))
        .unwrap();
    assert!(at("── Uses a model — can take minutes ──") < ask);
    choose(&mut app, "Ask for a proposal");
    assert!(dialog_argv(&app).ends_with(&["--build".to_string()]));
    let Mode::Dialog(c) = &app.mode else { panic!() };
    assert_eq!(c.title, "Ask a model for a proposal?");
}

/// Write the build configuration: the dialog says what opens and what
/// follows; the editor gets the map's example; on return the file is
/// checked and written to migration/map/config.toml, the map read again,
/// and the Next step is Map the project again — Enter's menu leads with it.
/// A draft that does not parse is offered again at its line; nothing is
/// written.
#[test]
fn the_build_configuration_is_written_from_the_cockpit() {
    let (_tmp, root) = project("write-config");
    std::fs::create_dir_all(root.join("src/include")).unwrap();
    guessed_map(&root);
    let mut app = cockpit(&root);
    let name = crate::app::features_edit::editor_name(&app.features_editor());
    code(&mut app, KeyCode::Enter);
    let write = write_item(&app);
    choose(&mut app, &write);
    let Mode::Dialog(c) = &app.mode else {
        panic!("no dialog: {:?}", app.mode)
    };
    assert_eq!(c.title, format!("Write the build configuration in {name}?"));
    assert_eq!(
        c.body,
        crate::app::features_edit::build_config_words(&name, true)
    );
    assert!(c.body[0]
        .contains("opens migration/map/config.toml, filled in with the example the map suggests"));
    assert!(c.body[3].ends_with("then choose Map the project again to map with it."));
    let screen = text(&render(&mut app, 120, 34));
    assert!(screen.contains(&format!("[ Open {name}  y ]")), "{screen}");
    arm(&mut app);
    let Command::EditFeatures { file, .. } = key(&mut app, 'y') else {
        panic!("y did not open the editor")
    };
    let example = app.project_map().unwrap().example.clone();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), example);
    assert!(example.starts_with("[[configuration]]\nname = \"make\"\nfrom = \"make\"\n"));
    assert!(app
        .kept_paths()
        .contains(&file.parent().unwrap().to_path_buf()));
    // A draft that does not parse: offered again at its line, nothing
    // written.
    std::fs::write(
        &file,
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\n",
    )
    .unwrap();
    let ok = || Ok(std::process::ExitStatus::from_raw(0));
    assert_eq!(app.features_edited(ok()), Command::None);
    let Mode::Dialog(c) = &app.mode else { panic!() };
    assert_eq!(c.title, "Your build configuration file has an error");
    assert!(c.body[0].starts_with("line "), "{:?}", c.body);
    assert!(!root.join("migration/map/config.toml").exists());
    // The person's flags: checked, written, the map read again.
    let mine = "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\"-Isrc/include\"]\n";
    std::fs::write(&file, mine).unwrap();
    app.mode = Mode::Normal;
    assert!(matches!(app.features_edited(ok()), Command::Cleanup(_)));
    assert_eq!(
        std::fs::read_to_string(root.join("migration/map/config.toml")).unwrap(),
        mine
    );
    assert!(app.config_draft.is_none());
    assert_eq!(
        app.notice.as_ref().map(|n| n.text.as_str()),
        Some("migration/map/config.toml is written — choose Map the project again to map with it")
    );
    let screen = text(&render(&mut app, 220, 40));
    for words in [
        "Next step: migration/map/config.toml is written but this map was made before it — \
         press Enter and choose Map the project again to use it",
        "Configuration: migration/map/config.toml is written since this map — Map the project \
         again to use it",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app)[0], "Map the project again");
    assert!(menu_labels(&app).contains(&format!("Edit the build configuration (in {name})")));
    choose(&mut app, "Map the project again");
    let Mode::Dialog(c) = &app.mode else { panic!() };
    assert_eq!(c.title, "Map the project again?");
    // Mapped again under it: the map is newer, the Next step moves on.
    app.mode = Mode::Normal;
    std::thread::sleep(std::time::Duration::from_millis(5));
    write_map(&root, &map_text("stated", false));
    app.reload_project();
    let next = app.project_next().unwrap();
    assert!(
        next.text.starts_with("t-unlzg holds a choice"),
        "{}",
        next.text
    );
}

/// The example the map suggests, left as it is where there was no file,
/// is written all the same: it states "no flags" (the dialog said what it
/// holds).
#[test]
fn the_example_left_as_it_is_is_a_configuration() {
    let (_tmp, root) = project("example-kept");
    guessed_map(&root);
    let mut app = cockpit(&root);
    assert_eq!(
        app.start_edit(crate::app::features_edit::DraftKind::BuildConfig),
        Command::None
    );
    arm(&mut app);
    let Command::EditFeatures { file, .. } = key(&mut app, 'y') else {
        panic!("y did not open the editor")
    };
    // As an editor that took its time: not "returned at once".
    if let Some(d) = app.config_draft.as_mut() {
        d.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
    }
    let example = std::fs::read_to_string(&file).unwrap();
    assert!(matches!(
        app.features_edited(Ok(std::process::ExitStatus::from_raw(0))),
        Command::Cleanup(_)
    ));
    assert_eq!(
        std::fs::read_to_string(root.join("migration/map/config.toml")).unwrap(),
        example
    );
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
        "✓ t-lzg",
        "⚠ t-unlzg",
        "held on a choice",
        "tools/unlzg.c (a tool, by its folder)",
        "t-lzg — tools/lzg.c (a tool, by its folder): linked",
        "held on a choice: two files define the same function; Accept asks which to keep.",
        "tools/broken.c — did not compile",
        "Libraries (2)",
        "src/bad.c (a library)",
        "Configuration: stated (make)",
        "Next step: t-unlzg holds a choice — press Enter on it and choose Accept this program",
        "Choice d1: two files define the same function, so linking cannot tell them apart (d1.1 \
         src/decode.c, d1.2 src/mini.c)",
        "The model's advice (claude-x): keep d1.2",
        "Accepted tools: none yet",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            "Accept this program".to_string(),
            "Accept it under a run name of your own…".into(),
            write_item(&app),
            "Map the project again".into(),
            "Ask a model for advice".into(),
        ]
    );
    assert_eq!(menu(&app).reason.as_deref(), Some("t-unlzg holds a choice"));
    assert!(menu(&app).items[4].model);
    // The picker: d1, its definers none suggested, the advice beside them.
    choose(&mut app, "Accept this program");
    let m = menu(&app);
    assert_eq!(m.title.as_deref(), Some("Accept t-unlzg"));
    assert_eq!(
        m.note,
        [
            "t-unlzg holds the choice d1: two files define the same function, so keep which \
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
    assert!(c.body[2].starts_with("It writes: migration/tools/<id>/harness.toml only"));
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

/// Right after an Accept, the Next step names the accepted tool first —
/// before another program's held choice — and its row is selected; once
/// opened, the Next step moves on (the newcomer's ten minutes, finding 5).
#[test]
fn after_an_accept_the_next_step_is_the_new_tool() {
    let (_tmp, root) = project("accept-next");
    write_map(&root, &map_text("stated", false));
    let mut app = cockpit(&root);
    app.select(Selection::Program("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Accept this program");
    arm(&mut app);
    let Command::Spawn(p) = key(&mut app, 'y') else {
        panic!("y did not run it: {:?}", app.mode)
    };
    app.on_spawned(&p);
    let dir = harness_core::config::tool_dir(&root, "t-lzg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("harness.toml"), "schema_version = 2\n").unwrap();
    app.on_child_exit(std::process::ExitStatus::from_raw(0));
    assert_eq!(app.selection, Selection::Tool("t-lzg".into()));
    let next = app.project_next().unwrap();
    assert_eq!(
        next.text,
        "t-lzg is accepted — open it and scan it: press Enter on its row and choose Open tool \
         t-lzg"
    );
    code(&mut app, KeyCode::Enter);
    assert_eq!(menu_labels(&app)[0], "Open tool t-lzg");
    assert_eq!(menu(&app).reason.as_deref(), Some("t-lzg is accepted"));
    // Opened: the held choice is the project's Next step again.
    app.mode = Mode::Normal;
    let read = crate::load::read_tool(&root, Some("t-lzg"));
    if let Ok(read) = read {
        app.open_tool("t-lzg".into(), read);
        app.show_project();
        assert!(app
            .project_next()
            .unwrap()
            .text
            .starts_with("t-unlzg holds a choice"));
    }
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
        "t-lzg — tools/lzg.c — press Enter on its row and choose Open tool",
        "t-lzg — tools/lzg.c (a tool, by its folder): accepted",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    app.select(Selection::Tool("t-lzg".into()));
    code(&mut app, KeyCode::Enter);
    assert_eq!(
        menu_labels(&app),
        [
            "Open tool t-lzg".to_string(),
            write_item(&app),
            "Map the project again".into(),
            "Ask a model for advice".into()
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
    // Its menu would hold only the Next step's item: one Enter goes back.
    assert_eq!(app.project_menu_items().len(), 1);
    code(&mut app, KeyCode::Enter);
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
