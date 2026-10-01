//! `harness perf init | save | run | show` (docs/PERF-DESIGN.md §3.10) on a
//! temp copy of the vendored zopfli target, whose u001 is verified: the
//! refusals in their words and exit codes, the starter, the save's
//! `--expect`, the C alone and the unit measured end to end through the
//! launcher, `perf-row` events, and `perf show` with its currency.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn copy_dir(src: &Path, dst: &Path) {
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
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness(args: &[&str], stdin: Option<&[u8]>) -> Run {
    harness_env(args, stdin, &[])
}

/// [`harness`] with these variables set over the test's own.
fn harness_env(args: &[&str], stdin: Option<&[u8]>, env: &[(&str, &OsStr)]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .envs(env.iter().copied())
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
        .join(format!("perf-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli"),
        &dst,
    );
    let _ = std::fs::remove_dir_all(dst.join("migration/perf"));
    dst
}

/// A pseudo-random text input of about `words` words.
fn text_input(path: &Path, words: usize) {
    let vocab = [
        "alpha", "beta", "gamma", "delta", "zopfli", "deflate", "huffman", "tree",
    ];
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut out = String::with_capacity(words * 7);
    for _ in 0..words {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push_str(vocab[(state % vocab.len() as u64) as usize]);
        out.push(' ');
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, out).unwrap();
}

fn blake3_of(path: &Path) -> String {
    harness_core::hash::file_hash(path).unwrap()
}

/// An executable shell script at `path`.
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The test's PATH with `dir` first.
fn path_with(dir: &Path) -> OsString {
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(dirs).unwrap()
}

/// `text` with `from` replaced by `to`, which must be there exactly once.
fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(
        text.matches(from).count(),
        1,
        "{}: {from:?}",
        path.display()
    );
    std::fs::write(path, text.replace(from, to)).unwrap();
}

/// `perf show`'s and `perf run`'s rows: each `perf:` line with the lines
/// that continue it, joined with single spaces.
fn rows_of(stdout: &str) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
        match rows.last_mut() {
            Some(last) if !line.starts_with("perf: ") => {
                last.push(' ');
                last.push_str(&words);
            }
            _ => rows.push(words),
        }
    }
    rows
}

/// The `perf-row` events of a `--json` run.
fn perf_rows(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|v: &serde_json::Value| v["k"] == "perf-row")
        .collect()
}

#[test]
fn init_writes_a_starter_once_and_save_guards_the_file() {
    let t = zopfli("init");
    let target = t.to_str().unwrap();
    let path = t.join("migration/perf/workloads.toml");
    let r = harness(&["perf", "init", "--target", target], None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        harness_core::perf::workloads::STARTER
    );
    let again = harness(&["perf", "init", "--target", target], None);
    assert_eq!(again.code, 1);
    assert!(
        again.stderr.contains("never overwrites it"),
        "{}",
        again.stderr
    );
    // Save: the --expect guard, the rule's line and column, the cap.
    let text = "schema_version = 1\n[[workload]]\nid = \"text\"\nargs = [\"-c\"]\n";
    let bytes = text.len().to_string();
    let stale = harness(
        &[
            "perf", "save", "--target", target, "--expect", "none", "--bytes", &bytes,
        ],
        Some(text.as_bytes()),
    );
    assert_eq!(stale.code, 1, "{}", stale.stderr);
    assert!(
        stale.stderr.contains("changed since the edit started"),
        "{}",
        stale.stderr
    );
    let expect = blake3_of(&path);
    let bad = "schema_version = 1\n[[workload]]\nid = \"Text\"\n";
    let r = harness(
        &[
            "perf",
            "save",
            "--target",
            target,
            "--expect",
            &expect,
            "--bytes",
            &bad.len().to_string(),
        ],
        Some(bad.as_bytes()),
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("line 3, column 6"), "{}", r.stderr);
    let r = harness(
        &[
            "perf", "save", "--target", target, "--expect", &expect, "--bytes", &bytes,
        ],
        Some(text.as_bytes()),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn run_refuses_by_name() {
    let t = zopfli("refuse");
    let target = t.to_str().unwrap();
    // No file, the starter, a broken file: exit 1 with the state's words.
    let r = harness(&["perf", "run", "--target", target], None);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr
            .contains("write your workloads file first — harness perf init gives a starter"),
        "{}",
        r.stderr
    );
    harness(&["perf", "init", "--target", target], None);
    let r = harness(&["perf", "run", "--target", target], None);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr
            .contains("add a [[workload]] to migration/perf/workloads.toml"),
        "{}",
        r.stderr
    );
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        "schema_version = 1\n[[workload]]\nid = \"w\"\nruns = 40\n",
    )
    .unwrap();
    let r = harness(&["perf", "run", "--target", target], None);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("line 4, column 8")
            && r.stderr.contains("— fix it, or Edit the workloads file"),
        "{}",
        r.stderr
    );
    // A usage error is exit 2.
    let r = harness(&["perf", "run", "--target", target, "--runs", "4"], None);
    assert_eq!(r.code, 2, "{}", r.stderr);
    // An unknown unit is named beside the known ones.
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = [\"-h\"]\n",
    )
    .unwrap();
    if cfg!(target_os = "macos") {
        let r = harness(
            &["perf", "run", "--target", target, "--unit", "u-nope"],
            None,
        );
        assert_eq!(r.code, 1, "{}", r.stderr);
        assert!(r.stderr.contains("u001-katajainen"), "{}", r.stderr);
    }
}

#[test]
fn zopfli_measured_end_to_end() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("run");
    let target = t.to_str().unwrap();
    text_input(&t.join("bench/text.txt"), 60_000);
    std::fs::write(t.join("bench/tiny.txt"), "hi\n").unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        "schema_version = 1\n\
         [[workload]]\nid = \"text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/text.txt\"\nruns = 5\n\
         [[workload]]\nid = \"tiny\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/tiny.txt\"\nruns = 5\n",
    )
    .unwrap();
    let r = harness(&["perf", "run", "--target", target, "--json"], None);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let events = perf_rows(&r.stdout);
    let find = |side: &str, workload: &str| {
        events
            .iter()
            .find(|e| e["side"] == side && e["workload"] == workload)
            .unwrap_or_else(|| panic!("{side} {workload}: {events:?}"))
            .clone()
    };
    assert_eq!(find("c", "tiny")["outcome"], "too-short");
    assert!(find("c", "tiny")["words"]
        .as_str()
        .unwrap()
        .starts_with("too short to time: the C ran"));
    let c = find("c", "text");
    assert!(matches!(c["outcome"].as_str(), Some("baseline")), "{c}");
    let unit = find("unit", "text");
    assert_eq!(unit["unit"], "u001-katajainen");
    assert!(
        matches!(
            unit["outcome"].as_str(),
            Some("measured") | Some("too-short")
        ),
        "{unit}"
    );
    // The results files read back strictly.
    let program = harness_core::perf::results::read_program(&t.join("migration/perf/program.json"))
        .unwrap()
        .unwrap();
    assert_eq!(program.c_alone.len(), 2);
    let unit_file = harness_core::perf::results::read_unit(
        &t.join("migration/perf/units/u001-katajainen.json"),
        "u001-katajainen",
    )
    .unwrap()
    .unwrap();
    assert_eq!(unit_file.rows.len(), 2);
    // `perf show` rebuilds the words; the computer is checked through the
    // current launcher cache; --no-check says nothing of either.
    let show = harness(&["perf", "show", "--target", target], None);
    assert_eq!(show.code, 0, "{}", show.stderr);
    assert!(
        show.stdout.contains("perf: the C on text — CPU about "),
        "{}",
        show.stdout
    );
    assert!(
        show.stdout.contains("perf: u001-katajainen on text — "),
        "{}",
        show.stdout
    );
    assert!(
        !show.stdout.contains("computer not checked"),
        "{}",
        show.stdout
    );
    // perf show reads the compilers as perf run did (tool runs, the tool
    // environment): the same lines, so no row is out of date.
    assert!(!show.stdout.contains("compilers not checked"));
    assert!(!show.stdout.contains("out of date"), "{}", show.stdout);
    // Another `cc` first on the PATH is another compiler: every row says so.
    let bin = t.join("stand-in/bin");
    script(&bin.join("cc"), "echo 'cc 1.0 (stand-in)'\n");
    let path = path_with(&bin);
    let show = harness_env(
        &["perf", "show", "--target", target],
        None,
        &[("PATH", &path)],
    );
    assert_eq!(show.code, 0, "{}", show.stderr);
    let text_rows: Vec<String> = rows_of(&show.stdout)
        .into_iter()
        .filter(|r| r.contains(" on text — "))
        .collect();
    assert_eq!(text_rows.len(), 2, "{}", show.stdout);
    for r in &text_rows {
        assert!(
            r.contains("out of date: measured with other compilers"),
            "{r}"
        );
    }
    // An edit to the C makes every row out of date, each with its reason.
    let main = t.join("src/zopfli/zopfli_bin.c");
    let mut text = std::fs::read_to_string(&main).unwrap();
    text.push_str("\n/* an edit */\n");
    std::fs::write(&main, text).unwrap();
    let show = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert!(
        show.stdout.contains("out of date: the C changed"),
        "{}",
        show.stdout
    );
    // And `perf run` refuses stale facts: scan first.
    let r = harness(&["perf", "run", "--target", target], None);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("scan the project first"), "{}", r.stderr);
}

/// True when every tool profile may write under `path` anyway (the temp
/// folders), so a write there proves nothing about the sandbox.
fn in_a_temp_folder(path: &Path) -> bool {
    let path = path.canonicalize().unwrap();
    let tmpdir = std::env::var_os("TMPDIR").and_then(|d| PathBuf::from(d).canonicalize().ok());
    [
        PathBuf::from("/private/tmp"),
        PathBuf::from("/private/var/folders"),
    ]
    .into_iter()
    .chain(tmpdir)
    .any(|d| path.starts_with(d))
}

/// `perf show` checks the compilers as tool runs (§3.9): in the tool
/// sandbox, with the tool environment, stopped at `[oracle] timeout_secs`.
/// The target picks which compilers run (its `rust-toolchain.toml`, or the
/// PATH as here), so they are target code. Without a launcher cache the
/// computer is not checked, in those words; `--no-check` runs neither.
#[test]
fn show_checks_the_compilers_in_the_sandbox() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("compilers");
    let target = t.to_str().unwrap();
    let bin = t.join("stand-in/bin");
    let marker = t.join("written-by-rustc");
    script(&bin.join("cc"), "echo 'cc 1.0 (stand-in)'\n");
    script(
        &bin.join("rustc"),
        &format!(
            "echo outside > '{}'\necho 'rustc 1.0.0 (stand-in)'\n",
            marker.display()
        ),
    );
    let path = path_with(&bin);
    let r = harness_env(
        &["perf", "show", "--target", target],
        None,
        &[("PATH", &path)],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!r.stdout.contains("compilers not checked"), "{}", r.stdout);
    if in_a_temp_folder(&t) {
        eprintln!("the target is in a temp folder: the sandbox's write rule not checked");
    } else {
        assert!(
            !marker.exists(),
            "rustc wrote in the target: it ran unsandboxed"
        );
    }

    // A compiler that never answers is stopped at the target's timeout.
    script(&bin.join("rustc"), "exec sleep 60\n");
    edit(
        &t.join("harness.toml"),
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\ntimeout_secs = 2\n",
    );
    let started = Instant::now();
    let r = harness_env(
        &["perf", "show", "--target", target],
        None,
        &[("PATH", &path)],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert!(
        r.stdout.contains("perf: compilers not checked"),
        "{}",
        r.stdout
    );

    // No launcher cache (a home folder without one): the computer is not
    // checked, and the words say how to check it.
    script(&bin.join("rustc"), "echo 'rustc 1.0.0 (stand-in)'\n");
    let home = t.with_extension("home");
    std::fs::create_dir_all(&home).unwrap();
    let env: [(&str, &OsStr); 2] = [("PATH", &path), ("HOME", home.as_os_str())];
    let r = harness_env(&["perf", "show", "--target", target], None, &env);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout
            .contains("perf: computer not checked — run harness perf run once"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("compilers not checked"), "{}", r.stdout);
    let r = harness_env(
        &["perf", "show", "--target", target, "--no-check"],
        None,
        &env,
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        !r.stdout.contains("not checked"),
        "--no-check checks neither: {}",
        r.stdout
    );
}

/// `perf show` only reads: it creates no folder, and it refuses a linked
/// `migration/perf` or `migration/perf/units` instead of reading another
/// folder's files as this target's rows (§3.9: links are refused on read).
#[test]
fn show_reads_only_and_refuses_linked_folders() {
    let t = zopfli("show-links");
    let target = t.to_str().unwrap();
    let r = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("perf: nothing measured yet"),
        "{}",
        r.stdout
    );
    assert!(
        !t.join("migration/perf").exists(),
        "perf show made migration/perf"
    );

    // A units folder linked to one outside the project.
    let outside = t.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("u001-katajainen.json"),
        "{\"schema\": \"SECRET-OUTSIDE\"}",
    )
    .unwrap();
    std::fs::write(
        outside.join("private-notes.json"),
        "{\"api_key\": \"SECRET-OUTSIDE\"}",
    )
    .unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::os::unix::fs::symlink(&outside, t.join("migration/perf/units")).unwrap();
    let r = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(
        r.stderr
            .contains("migration/perf/units: must be a directory (a link is refused)"),
        "{}",
        r.stderr
    );
    for text in [&r.stdout, &r.stderr] {
        assert!(!text.contains("SECRET-OUTSIDE"), "{text}");
        assert!(!text.contains("private-notes"), "{text}");
    }

    // And a linked migration/perf.
    std::fs::remove_file(t.join("migration/perf/units")).unwrap();
    std::fs::remove_dir(t.join("migration/perf")).unwrap();
    std::os::unix::fs::symlink(&outside, t.join("migration/perf")).unwrap();
    let r = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(
        r.stderr
            .contains("migration/perf: must be a directory (a link is refused)"),
        "{}",
        r.stderr
    );
}
