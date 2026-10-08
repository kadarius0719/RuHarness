//! `harness features init | save | map` (docs/FEATURES-DESIGN.md §5, §7) on a
//! temp copy of the vendored zopfli target: the refusals, the starter, the
//! save's `--expect`, and a map whose functions separate gzip from zlib.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn copy_dir(src: &Path, dst: &Path) {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "build" || name_str == "target" || name_str == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
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

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness(args: &[&str], stdin: Option<&[u8]>) -> Run {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn harness");
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn zopfli(tag: &str) -> PathBuf {
    let dst = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("features-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli"),
        &dst,
    );
    // These tests start from a target without features (the committed
    // zopfli has its own; the oracle's tests run them).
    std::fs::remove_dir_all(dst.join("migration/features")).unwrap();
    dst
}

const FEATURES: &str = "schema_version = 1\n\
[[feature]]\nid = \"gzip\"\nname = \"Compress to gzip\"\n\
[[feature]]\nid = \"zlib\"\nname = \"Compress to zlib\"\n\
[[scenario]]\nfeature = \"gzip\"\nid = \"text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"sample:text\"\n\
[[scenario]]\nfeature = \"zlib\"\nid = \"text\"\nargs = [\"--zlib\", \"-c\", \"{input}\"]\ninput = \"sample:text\"\n";

fn blake3_of(path: &Path) -> String {
    harness_core::hash::file_hash(path).unwrap()
}

#[test]
fn init_writes_a_starter_once_and_save_guards_the_file() {
    let root = zopfli("init");
    let target = format!("--target={}", root.display());
    let file = root.join("migration/features/features.toml");

    let r = harness(&["features", "init", &target], None);
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let starter = std::fs::read_to_string(&file).unwrap();
    assert!(starter.contains("schema_version = 1"));
    assert!(
        starter.contains("already runs the program with -c"),
        "{starter}"
    );
    let again = harness(&["features", "init", &target], None);
    assert_eq!(again.code, 1);
    assert!(
        again.stderr.contains("never overwrites"),
        "{}",
        again.stderr
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), starter);

    // A save: the new text on stdin, --expect the current bytes.
    let bytes = FEATURES.len().to_string();
    let expect = format!("--expect={}", blake3_of(&file));
    let r = harness(
        &["features", "save", &expect, "--bytes", &bytes, &target],
        Some(FEATURES.as_bytes()),
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FEATURES);

    // The same --expect again: the file changed since — refused, untouched.
    let r = harness(
        &["features", "save", &expect, "--bytes", &bytes, &target],
        Some(FEATURES.as_bytes()),
    );
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("changed since the edit started"),
        "{}",
        r.stderr
    );

    // A text that does not validate: refused with the loader's words.
    let bad = "schema_version = 1\nnope = 1\n";
    let expect = format!("--expect={}", blake3_of(&file));
    let r = harness(
        &[
            "features",
            "save",
            &expect,
            "--bytes",
            &bad.len().to_string(),
            &target,
        ],
        Some(bad.as_bytes()),
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("unknown key \"nope\""), "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FEATURES);

    // A read cut short: refused.
    let r = harness(
        &["features", "save", &expect, "--bytes", "9999", &target],
        Some(FEATURES.as_bytes()),
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("cut short"), "{}", r.stderr);

    // --expect none only when there is no file.
    let r = harness(
        &[
            "features",
            "save",
            "--expect=none",
            "--bytes",
            &bytes,
            &target,
        ],
        Some(FEATURES.as_bytes()),
    );
    assert_eq!(r.code, 1);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_and_init_never_write_through_a_symlinked_directory() {
    let root = zopfli("symlink");
    let target = format!("--target={}", root.display());
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("migration/features")).unwrap();
    let r = harness(&["features", "init", &target], None);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("not a real directory"), "{}", r.stderr);
    let r = harness(
        &[
            "features",
            "save",
            "--expect=none",
            "--bytes",
            &FEATURES.len().to_string(),
            &target,
        ],
        Some(FEATURES.as_bytes()),
    );
    assert_eq!(r.code, 1);
    assert!(
        std::fs::read_dir(&elsewhere).unwrap().next().is_none(),
        "nothing written"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn map_refuses_what_it_cannot_map_and_maps_the_rest() {
    let root = zopfli("map");
    let target = format!("--target={}", root.display());
    let r = harness(&["features", "map", "--allow-unsandboxed", &target], None);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr
            .contains("there is no migration/features/features.toml"),
        "{}",
        r.stderr
    );

    let dir = root.join("migration/features");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("features.toml"), "schema_version = 1\n").unwrap();
    let r = harness(&["features", "map", "--allow-unsandboxed", &target], None);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no scenario"), "{}", r.stderr);

    std::fs::write(dir.join("features.toml"), FEATURES).unwrap();
    // A scanned file changed: the facts' ids would not match.
    let main = root.join("src/zopfli/zopfli_bin.c");
    let original = std::fs::read(&main).unwrap();
    std::fs::write(&main, [original.as_slice(), b"\n/* edited */\n"].concat()).unwrap();
    let r = harness(&["features", "map", "--allow-unsandboxed", &target], None);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("scan the project first"), "{}", r.stderr);
    std::fs::write(&main, &original).unwrap();

    let r = harness(
        &["--json", "features", "map", "--allow-unsandboxed", &target],
        None,
    );
    assert_eq!(r.code, 0, "{}{}", r.stdout, r.stderr);
    let scenario_events: Vec<serde_json::Value> = r
        .stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["k"] == "scenario")
        .collect();
    assert_eq!(scenario_events.len(), 2);
    assert_eq!(scenario_events[1]["n"], 2);
    assert_eq!(scenario_events[1]["of"], 2);
    let map: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("map.json")).unwrap()).unwrap();
    let names = |i: usize| -> Vec<String> {
        map["scenarios"][i]["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[1].as_str().unwrap().to_string())
            .collect()
    };
    assert!(names(0).contains(&"ZopfliGzipCompress".to_string()));
    assert!(!names(0).contains(&"ZopfliZlibCompress".to_string()));
    assert!(names(1).contains(&"ZopfliZlibCompress".to_string()));
    assert!(!names(1).contains(&"ZopfliGzipCompress".to_string()));
    // The scratch copy is gitignored build space; the sources are untouched.
    assert_eq!(std::fs::read(&main).unwrap(), original);
    std::fs::remove_dir_all(&root).ok();
}

/// A launchd-started process may have a soft descriptor limit of 256: the
/// probe's high descriptor falls back below it, and the notes still arrive.
#[test]
fn the_map_works_under_a_low_descriptor_limit() {
    let root = zopfli("ulimit");
    let dir = root.join("migration/features");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("features.toml"), FEATURES).unwrap();
    let script = format!(
        "ulimit -n 256 && exec \"$0\" features map --target={}",
        root.display()
    );
    let out = Command::new("/bin/sh")
        .args(["-c", &script, env!("CARGO_BIN_EXE_harness")])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let map: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("map.json")).unwrap()).unwrap();
    for i in 0..2 {
        assert_eq!(map["scenarios"][i]["noted"], "complete");
        assert!(map["scenarios"][i]["functions"].as_array().unwrap().len() > 50);
    }
    std::fs::remove_dir_all(&root).ok();
}
