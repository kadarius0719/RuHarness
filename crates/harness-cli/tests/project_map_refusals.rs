//! Two refusals of `harness project map` §4 lists that no other test runs
//! through the binary (docs/PROJECT-MAP-DESIGN.md §3.8): no sandbox and no
//! `--allow-unsandboxed`, and a walk cut short by a cap — exit 1 by name,
//! with the file facts it gathered still printed. Every child runs with the
//! test process's own adoption file.

use std::path::PathBuf;
use std::process::Command;

/// A fresh, canonical temporary folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-map-refusal-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir.canonicalize().unwrap())
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn harness(args: &[&str]) -> (i32, String, String) {
    let file = harness_core::adopt::testing::adoption_file();
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .env(harness_core::adopt::ADOPTED_ENV, file)
        .output()
        .expect("spawn harness");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Where no sandbox is available, `project map` compiles nothing unless
/// the person passes `--allow-unsandboxed`. On a host with a sandbox there
/// is nothing to refuse: the test says so and stops.
#[test]
fn the_map_is_refused_without_a_sandbox() {
    if harness_oracle::sandbox_mode() != "none" {
        eprintln!("skipped: this host has a sandbox, so `project map` is never refused for one");
        return;
    }
    let dir = Tmp::new("no-sandbox");
    std::fs::write(dir.0.join("a.c"), "int a(void) { return 1; }\n").unwrap();
    let (code, stdout, stderr) = harness(&["project", "map", "--target", dir.0.to_str().unwrap()]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains("no sandbox is available on this platform; `project map`")
            && stderr.contains("pass --allow-unsandboxed to accept that"),
        "{stderr}"
    );
    assert!(!dir.0.join("migration").exists(), "nothing written");
}

/// A walk that reaches the depth cap stops there: exit 1, the cap named,
/// and the files it did visit still printed with their facts.
#[test]
fn a_walk_cut_short_exits_1_by_name_with_its_facts() {
    let dir = Tmp::new("deep");
    std::fs::write(dir.0.join("top.c"), "int top(void) { return 1; }\n").unwrap();
    let mut deep = dir.0.clone();
    for _ in 0..40 {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("deep.c"), "int deep(void) { return 2; }\n").unwrap();
    let (code, stdout, stderr) = harness(&[
        "project",
        "map",
        "--allow-unsandboxed",
        "--target",
        dir.0.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains("error: the map stopped at its depth limit of 32 folders deep"),
        "{stderr}"
    );
    // The facts gathered before the cap are printed.
    assert!(stdout.contains("top.c"), "{stdout}");
    assert!(!stdout.contains("deep.c"), "{stdout}");
}
