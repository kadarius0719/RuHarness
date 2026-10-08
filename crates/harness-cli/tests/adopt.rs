//! Adoption of a ledger made elsewhere, through the binary
//! (docs/PROJECT-MAP-DESIGN.md §3.7): the refusal, `--adopt`, the first
//! `scan` recording its own ledger, and a benchmark suite adopted as one root.
//! Every child runs with the test process's own adoption file.

use std::path::{Path, PathBuf};
use std::process::Command;

const REFUSAL_HEAD: &str = "this folder already holds migration results made elsewhere (";
const REFUSAL_TAIL: &str = "): to trust them here, add `--adopt` once";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "harness-cli-adopt-{tag}-{}-{}",
        std::process::id(),
        harness_core::hash::random_hex(4)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn copy_dir(src: &Path, dst: &Path) {
    harness_core::adopt::testing::adoption_file();
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            "build" | "target" | ".git" | ".scorer-vendor" | ".bench"
        ) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&from, &to);
        } else if entry.file_type().unwrap().is_file() {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness(args: &[&str]) -> Run {
    let file = harness_core::adopt::testing::adoption_file();
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .env(harness_core::adopt::ADOPTED_ENV, file)
        .output()
        .expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn adoption_text() -> String {
    std::fs::read_to_string(harness_core::adopt::testing::adoption_file()).unwrap_or_default()
}

#[test]
fn a_ledger_made_elsewhere_is_refused_until_adopted_once() {
    // zopfli's ledger copied elsewhere: its token came along, its root is
    // not listed on this computer.
    let root = tmp("zopfli");
    copy_dir(&repo().join("targets/zopfli"), &root);
    let crate_target = root.join("migration/units/u001-katajainen/katajainen_rs/target");
    std::fs::create_dir_all(crate_target.join("debug")).unwrap();
    std::fs::create_dir_all(root.join("migration/build/old")).unwrap();
    // A link where a promote marker would be: the link goes, its target stays.
    let outside = tmp("outside");
    std::fs::write(outside.join("keep"), "x").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        root.join("migration/units/u001-katajainen/.promote-a-1"),
    )
    .unwrap();
    let t = root.to_str().unwrap();
    // The token a download would ship.
    let shipped = format!("{}\n", "c".repeat(32));
    std::fs::write(root.join("migration/.ruharness-adopted"), &shipped).unwrap();

    let r = harness(&["state", "status", "--target", t]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(REFUSAL_HEAD) && r.stderr.contains(REFUSAL_TAIL),
        "{}",
        r.stderr
    );
    // The refusal names the folder.
    assert!(
        r.stderr
            .contains(&format!("error: {t}: this folder already holds")),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("verified"), "{}", r.stderr);
    assert!(crate_target.exists(), "a refusal deletes nothing");

    let r = harness(&["state", "status", "--target", t, "--adopt"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("are claims made elsewhere until `harness verify` runs them here"),
        "{}",
        r.stdout
    );
    assert!(!crate_target.exists());
    assert!(!root.join("migration/build").exists());
    assert!(
        std::fs::symlink_metadata(root.join("migration/units/u001-katajainen/.promote-a-1"))
            .is_err()
    );
    assert!(outside.join("keep").exists(), "a link's target is kept");
    // A fresh token was written over the one the copy brought along: a
    // later tree shipped with that token is not trusted.
    let token = std::fs::read_to_string(root.join("migration/.ruharness-adopted")).unwrap();
    assert_ne!(token.trim(), shipped.trim());
    assert_eq!(token.trim().len(), 32);
    assert!(adoption_text().contains(token.trim()));

    // Trusted from now on; a second --adopt deletes nothing.
    assert_eq!(harness(&["state", "status", "--target", t]).code, 0);
    std::fs::create_dir_all(root.join("migration/build/new")).unwrap();
    let r = harness(&["state", "status", "--target", t, "--adopt"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("already trusted"), "{}", r.stdout);
    assert!(root.join("migration/build/new").exists());

    // Another tree unpacked at the same path, with a token of its own.
    std::fs::write(
        root.join("migration/.ruharness-adopted"),
        format!("{}\n", "a".repeat(32)),
    )
    .unwrap();
    let r = harness(&["state", "status", "--target", t]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains(REFUSAL_HEAD), "{}", r.stderr);
}

#[test]
fn the_projects_own_migration_folder_is_not_adopted() {
    let root = tmp("own");
    std::fs::copy(
        repo().join("targets/zopfli/harness.toml"),
        root.join("harness.toml"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("migration")).unwrap();
    std::fs::write(root.join("migration/0001_create_users.sql"), "create table").unwrap();
    let t = root.to_str().unwrap();
    // Said first, before any adoption question — and the same with --adopt.
    for args in [
        &["state", "status", "--target", t][..],
        &["state", "status", "--target", t, "--adopt"][..],
    ] {
        let r = harness(args);
        assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
        assert!(
            r.stderr.contains(
                "this project has a migration/ folder of its own; move or rename it, or map a \
                 copy"
            ),
            "{}",
            r.stderr
        );
        assert!(!r.stderr.contains("--adopt"), "{}", r.stderr);
    }
    assert!(!adoption_text().contains(t));
}

/// A tool written by hand holds no results: its first command is not asked
/// to adopt, and records the project as made here (with a fresh token, not
/// one a download shipped beside the file).
#[test]
fn a_hand_written_tool_is_made_here() {
    let root = tmp("hand");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.c"), "int f(void) { return 1; }\n").unwrap();
    let tool = root.join("migration/tools/t-a");
    std::fs::create_dir_all(&tool).unwrap();
    std::fs::write(
        tool.join("harness.toml"),
        "schema_version = 2\n[target]\nname = \"a\"\nfiles = [\n\
         { path = \"src/a.c\", include_dirs = [] },\n]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [] }\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("migration/map")).unwrap();
    std::fs::write(root.join("migration/map/config.toml"), "").unwrap();
    let shipped = format!("{}\n", "d".repeat(32));
    std::fs::write(root.join("migration/.ruharness-adopted"), &shipped).unwrap();
    let t = root.to_str().unwrap();
    let r = harness(&["scan", "--target", t, "--tool", "t-a"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let token = std::fs::read_to_string(root.join("migration/.ruharness-adopted")).unwrap();
    assert_ne!(token, shipped);
    let text = adoption_text();
    assert!(
        text.contains(t) && text.contains(token.trim()) && text.contains("how = \"created\""),
        "{text}"
    );
    // The next command is not asked either; --adopt on it says nothing of
    // claims (nothing was made elsewhere).
    let r = harness(&["plan", "--target", t, "--tool", "t-a", "--adopt"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("claims"), "{}", r.stdout);
}

#[test]
fn the_first_scan_on_a_fresh_folder_records_it() {
    let root = tmp("fresh");
    std::fs::write(
        root.join("harness.toml"),
        "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.c"), "int f(void) { return 1; }\n").unwrap();
    let t = root.to_str().unwrap();
    let r = harness(&["scan", "--target", t]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let token = std::fs::read_to_string(root.join("migration/.ruharness-adopted")).unwrap();
    let text = adoption_text();
    assert!(text.contains(t) && text.contains(token.trim()), "{text}");
    assert!(text.contains("how = \"created\""), "{text}");
    // The next command is not asked.
    let r = harness(&["plan", "--target", t]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
}

#[test]
fn a_benchmark_suite_is_adopted_as_one_root() {
    let suite = tmp("suite");
    copy_dir(&repo().join("targets/tractor"), &suite);
    // The copied suite token came along; the suite is not listed here.
    let s = suite.to_str().unwrap();
    let r = harness(&["bench", "status", "--suite", s]);
    assert_eq!(r.code, 1, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(REFUSAL_HEAD) && r.stderr.contains(REFUSAL_TAIL),
        "{}",
        r.stderr
    );
    let r = harness(&["bench", "status", "--suite", s, "--adopt"]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("bench status: 100 case(s)"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("error:"), "{}", r.stdout);
    // Every case is covered by the one root.
    let case = suite.join("cases/Hidden-Tests/B01_organic/ima_decode_lib");
    let r = harness(&["state", "status", "--target", case.to_str().unwrap()]);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert_eq!(harness(&["bench", "status", "--suite", s]).code, 0);
}
