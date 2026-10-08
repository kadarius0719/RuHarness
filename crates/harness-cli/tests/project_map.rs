//! `harness project map` through the binary (docs/PROJECT-MAP-DESIGN.md
//! §3.8, §5 step a): zopfli's folder, the refusals and their exit codes, and
//! a file name holding a newline in both output modes. Every child runs with
//! the test process's own adoption file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, canonical temporary folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-project-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir.canonicalize().unwrap())
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness_env(args: &[&str], env: &[(&str, &Path)]) -> Run {
    let file = harness_core::adopt::testing::adoption_file();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harness"));
    cmd.args(args).env(harness_core::adopt::ADOPTED_ENV, file);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn harness(args: &[&str]) -> Run {
    harness_env(args, &[])
}

#[test]
fn zopfli_s_source_folder_is_mapped_from_its_harness_toml() {
    let root = repo().join("targets/zopfli");
    harness_core::adopt::testing::adopt(&root);
    let run = harness(&["project", "map", "--target", root.to_str().unwrap()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("26 files (13 .c, 13 .h): 13 compiled, 0 did not; 0 ambiguous"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("src/zopfli/tree.c — compiled; include folders: none; defines"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("nothing written yet"));
}

#[test]
fn a_newline_in_a_file_name_never_reaches_the_output_raw() {
    let tmp = Tmp::new("newline");
    tmp.write("a\nb.c", "int ab(void) { return 1; }\n");
    tmp.write("ok.c", "int ok(void) { return 2; }\n");

    let run = harness(&["project", "map", "--target", tmp.arg(), "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let events: Vec<serde_json::Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l:?}: {e}")))
        .collect();
    let files: Vec<&serde_json::Value> =
        events.iter().filter(|e| e["k"] == "project-file").collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "a\nb.c");
    assert_eq!(files[0]["compiled"], true);
    assert_eq!(files[0]["defined"], 1);
    assert!(run.stdout.contains(r#""path":"a\nb.c""#), "{}", run.stdout);

    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("  a?b.c — compiled"), "{}", run.stdout);
    assert!(
        run.stdout.lines().all(|l| !l.starts_with("b.c")),
        "{}",
        run.stdout
    );
}

#[test]
fn a_home_folder_root_is_refused() {
    let home = Tmp::new("home");
    home.write("a.c", "int a;\n");
    let run = harness_env(
        &["project", "map", "--target", home.arg()],
        &[("HOME", home.0.as_path())],
    );
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("is your home folder, which the map never reads"),
        "{}",
        run.stderr
    );
    assert!(!run.stdout.contains("compiled"), "{}", run.stdout);
}

#[test]
fn a_folder_without_c_is_refused() {
    let tmp = Tmp::new("empty");
    tmp.write("README", "no C here\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains("no C files (.c or .h) were found"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_ledger_made_elsewhere_is_refused_before_anything_compiles() {
    let tmp = Tmp::new("ledger");
    tmp.write("a.c", "int a;\n");
    tmp.write("migration/facts.jsonl", "");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("this folder already holds migration results made elsewhere"),
        "{}",
        run.stderr
    );
    assert!(!run.stdout.contains("compiled"), "{}", run.stdout);
}

#[test]
fn a_usage_error_exits_2() {
    let run = harness(&["project", "map", "--no-such-flag"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
}
