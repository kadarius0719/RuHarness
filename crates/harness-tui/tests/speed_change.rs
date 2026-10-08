//! docs/PERF-DESIGN.md §10 build note 22: each act a slower row's next step
//! names changes the crate perf measures — the unit's verified crate,
//! `units/<id>/<rust_crate>`, the one `perf run` builds — and a recorded
//! hand edit alone does not (it is never accepted). The cockpit's own argv
//! for Hand edit and Replace run through the real `harness` on a scratch
//! copy of the tractor case, whose `u-lib` came from a model's attempt.
//! Needs the CLI built (`cargo build -p harness-cli`; `cargo test
//! --workspace` does).

use harness_tui::app::{App, Config, LayoutMode};
use harness_tui::tree::Selection;
use std::path::{Path, PathBuf};
use std::process::Command;

const CASE: &str = "targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib";
const UNIT: &str = "u-lib";
const PIPELINE: &str = "a-13c941dfff95";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn harness_bin() -> PathBuf {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
    let path = Path::new(env!("CARGO_BIN_EXE_harness-tui")).with_file_name("harness");
    assert!(
        path.is_file(),
        "{} is missing: build the CLI first (`cargo build -p harness-cli`; \
         `cargo test --workspace` does)",
        path.display()
    );
    path
}

fn copy_dir(src: &Path, dst: &Path) {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "build" || name == "target" || name == ".git" || name == ".lock" {
            continue;
        }
        let (from, to) = (entry.path(), dst.join(&name));
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
    // A copied ledger is adopted for this test process, as the person's
    // `--adopt` would (docs/PROJECT-MAP-DESIGN.md §3.7).
    if dst.join("migration").is_dir() {
        harness_core::adopt::testing::adopt(dst);
    }
}

fn app(root: &Path) -> App {
    App::new(
        Config {
            target: root.to_path_buf(),
            tool: None,
            harness: Some(harness_bin()),
            allow_unsandboxed: true,
            layout: LayoutMode::Auto,
            providers: vec!["external".into()],
        },
        harness_tui::load::read(root).unwrap(),
    )
}

/// The digest of the crate perf measures (what `perf run` records on the
/// unit's row and the cockpit hashes to judge "its Rust changed since").
fn measured(root: &Path) -> String {
    harness_core::hash::unit_crate_file_set_hash(
        root,
        &root.join("migration/units").join(UNIT).join("u_lib_rs"),
    )
    .unwrap()
}

fn run(argv: &[std::ffi::OsString]) {
    let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(
        out.status.success(),
        "{argv:?}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn attempts(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> =
        std::fs::read_dir(root.join("migration/units").join(UNIT).join("attempts"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    ids.sort();
    ids
}

/// The cockpit's Hand edit, as the editor would leave it: a comment added
/// to logic.rs, staged, recorded with `harness override`. The new attempt.
fn hand_edit(root: &Path, note: &str) -> String {
    let before = attempts(root);
    let app = app(root);
    let crate_dir = app
        .snapshot
        .unit(UNIT)
        .and_then(|u| u.crate_dir.clone())
        .unwrap();
    let session = harness_tui::handedit::prepare(&crate_dir, &std::env::temp_dir()).unwrap();
    let mut logic = std::fs::read_to_string(&session.files[0]).unwrap();
    logic.push_str(&format!("\n// {note}\n"));
    std::fs::write(&session.files[0], logic).unwrap();
    let stage = session.stage().unwrap().expect("the edit changed a file");
    let pending = app
        .hand_edit_argv(UNIT, &stage, session.tmp.clone(), None)
        .unwrap();
    run(&pending.argv);
    let _ = std::fs::remove_dir_all(&session.tmp);
    let new: Vec<String> = attempts(root)
        .into_iter()
        .filter(|a| !before.contains(a))
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    new[0].clone()
}

/// The cockpit's Replace on `attempt` (its menu item, its argv).
fn replace(root: &Path, attempt: &str) {
    let mut app = app(root);
    app.select(Selection::Attempt(UNIT.into(), attempt.into()));
    let label = format!(
        "Replace {UNIT}'s verified crate with {}",
        harness_tui::model::short_id(attempt)
    );
    let item = app
        .menu_items()
        .into_iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("{label} offered"));
    assert_eq!(item.greyed, None, "{label}");
    run(&item.pending.expect("an act").argv);
}

fn words(root: &Path) -> String {
    let app = app(root);
    harness_tui::speed::change_words(app.snapshot.unit(UNIT).unwrap(), true)
}

#[test]
fn each_act_the_next_step_names_changes_the_crate_perf_measures() {
    let tmp = std::env::temp_dir().join(format!("harness-tui-speed-change-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&repo().join(CASE), &tmp);
    let root = tmp.canonicalize().unwrap();
    let first = measured(&root);
    // A model's attempt: Modify it (a new attempt, never accepted — like a
    // hand edit), Replace, and back.
    assert!(
        words(&root).ends_with(&format!(
            "Replace it back with {}",
            harness_tui::model::short_id(PIPELINE)
        )),
        "{}",
        words(&root)
    );
    // Make the crate a recorded hand edit's: Hand edit, then Replace.
    let h1 = hand_edit(&root, "first hand edit");
    assert_eq!(
        measured(&root),
        first,
        "a hand edit alone is never accepted"
    );
    replace(&root, &h1);
    let in_use = measured(&root);
    assert_ne!(in_use, first, "Replace changes the crate perf measures");
    // Its next step, with the attempt in use now named for the way back.
    assert_eq!(
        words(&root),
        format!(
            "Hand edit {UNIT}'s crate, then Replace {UNIT}'s verified crate with the new attempt \
             and measure this unit again — and if it is not faster, Replace it back with {}",
            harness_tui::model::short_id(&h1)
        )
    );
    // Each act it names, in turn.
    let h2 = hand_edit(&root, "second hand edit");
    assert_eq!(
        measured(&root),
        in_use,
        "the hand edit alone: perf would time the same crate"
    );
    replace(&root, &h2);
    assert_ne!(measured(&root), in_use, "Replace with the new attempt");
    replace(&root, &h1);
    assert_eq!(
        measured(&root),
        in_use,
        "Replace it back with the attempt in use before"
    );
    // The model-made branch's way back.
    replace(&root, PIPELINE);
    assert_eq!(measured(&root), first);
    let _ = std::fs::remove_dir_all(&tmp);
}
