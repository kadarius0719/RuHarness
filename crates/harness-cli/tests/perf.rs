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

/// The one row that starts with `start`.
fn row<'a>(rows: &'a [String], start: &str) -> &'a str {
    let found: Vec<&String> = rows.iter().filter(|r| r.starts_with(start)).collect();
    assert_eq!(found.len(), 1, "{start}: {rows:#?}");
    found[0]
}

/// The `perf-row` events of a `--json` run.
fn perf_rows(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|v: &serde_json::Value| v["k"] == "perf-row")
        .collect()
}

/// A workloads file with one workload, `w` (`zopfli -h`).
const ONE_WORKLOAD: &str = "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = [\"-h\"]\n";

/// A row as `perf run` stores it, without a run: `outcome` on workload `w`
/// over five runs (`other` runs too unless it is a baseline), with `inputs`
/// added to the target's digests today — so it reads current until
/// something changes.
fn stored_row(t: &Path, outcome: &str, inputs: serde_json::Value) -> serde_json::Value {
    stored_row_on(t, "w", outcome, inputs)
}

/// [`stored_row`] on `workload` (one without an input file).
fn stored_row_on(
    t: &Path,
    workload: &str,
    outcome: &str,
    inputs: serde_json::Value,
) -> serde_json::Value {
    use harness_core::perf::workloads::{self as wl, WorkloadsState};
    let ctx = harness_core::TargetContext::load(t).unwrap();
    let facts = harness_core::Facts::load(&t.join("migration/facts.jsonl")).unwrap();
    let WorkloadsState::Ready(workloads) = wl::load(t).unwrap() else {
        panic!("{}: no workloads file", t.display());
    };
    let run = serde_json::json!({"cpu_us": 1_300_000, "wall_us": 1_300_000, "end": "exit 0"});
    let mut row = serde_json::json!({
        "workload": workload,
        "outcome": outcome,
        "short": false,
        "runs": 5,
        "platform_metrics": "cpu-time",
        "inputs": {
            "workload": wl::digest(workloads.get(workload).unwrap(), None),
            "program": harness_core::features::program_digest_now(&ctx, &facts),
            "program_name": harness_core::features::program_name(&ctx.config),
            "recipe": harness_core::perf::PERF_RECIPE,
            "launcher": harness_core::perf::PERF_LAUNCHER,
            "computer": {"os": "15.6", "build": "24G84", "arch": "arm64", "cpu": "Apple M3",
                         "two_kinds": true, "fast_cores": 4},
            "compilers": {"cc": "cc 1.0 (stand-in)"},
        },
        "c": vec![run.clone(); 5],
    });
    if outcome != "baseline" {
        row["other"] = serde_json::json!(vec![run; 5]);
    }
    for (k, v) in inputs.as_object().unwrap() {
        row["inputs"][k] = v.clone();
    }
    row
}

/// The workloads file [`ONE_WORKLOAD`] and the C alone's baseline on `w`,
/// stored as `perf run` would store it (see [`stored_row`]).
fn store_a_baseline(t: &Path) {
    use harness_core::perf::results as res;
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(t.join("migration/perf/workloads.toml"), ONE_WORKLOAD).unwrap();
    let mut file = res::ProgramResults::default();
    let row = stored_row(t, "baseline", serde_json::json!({}));
    file.c_alone.push(serde_json::from_value(row).unwrap());
    res::write_program(&t.join("migration/perf/program.json"), &file).unwrap();
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

/// `perf run` refuses stale facts before anything is built ("scan first",
/// §3.10), whichever way they went stale: a file the scan recorded changed
/// though the program does not build it (a C file in a subfolder that
/// nothing includes), or a C file the whole-program build compiles that the
/// scan never saw. Each case is checked to be one the other cannot see.
#[test]
fn run_refuses_stale_facts_before_building() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    use harness_core::features::{program_digest_now, program_paths, STALE_PROGRAM};
    let t = zopfli("stale-facts");
    let target = t.to_str().unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(t.join("migration/perf/workloads.toml"), ONE_WORKLOAD).unwrap();
    let scan = || {
        let r = harness(&["scan", "--target", target], None);
        assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
        (
            harness_core::TargetContext::load(&t).unwrap(),
            harness_core::Facts::load(&t.join("migration/facts.jsonl")).unwrap(),
        )
    };
    let refused = |case: &str| {
        let r = harness(&["perf", "run", "--target", target], None);
        assert_eq!(r.code, 1, "{case}: {}\n{}", r.stdout, r.stderr);
        assert!(
            r.stderr.contains(
                "the program's C changed since the scan: scan the project first, then measure"
            ),
            "{case}: {}",
            r.stderr
        );
        assert!(!r.stdout.contains("building"), "{case}: {}", r.stdout);
        assert!(
            !t.join("migration/build/.perf").exists(),
            "{case}: perf built before refusing"
        );
    };

    // A recorded C file outside the program changes: the program's digest
    // does not see it.
    let unused = "src/zopfli/extra/unused.c";
    std::fs::create_dir_all(t.join("src/zopfli/extra")).unwrap();
    std::fs::write(t.join(unused), "int unused(void) { return 0; }\n").unwrap();
    let (ctx, facts) = scan();
    assert!(facts.files.iter().any(|f| f.path == unused));
    assert!(!program_paths(&ctx, &facts).iter().any(|p| p == unused));
    std::fs::write(t.join(unused), "int unused(void) { return 1; }\n").unwrap();
    assert_ne!(program_digest_now(&ctx, &facts), STALE_PROGRAM);
    refused("a recorded file outside the program changed");

    // A top-level C file the scan never saw: every recorded file is
    // unchanged, only the program's digest sees it.
    let (ctx, facts) = scan();
    std::fs::write(
        t.join("src/zopfli/added.c"),
        "int added(void) { return 0; }\n",
    )
    .unwrap();
    for f in &facts.files {
        assert_eq!(blake3_of(&t.join(&f.path)), f.hash, "{}", f.path);
    }
    assert_eq!(program_digest_now(&ctx, &facts), STALE_PROGRAM);
    refused("a C file the scan never saw");
}

/// `perf run --as-it-stands-only` with one measurable unit (zopfli's u001),
/// or none, is refused before anything is built (§3.10), naming the units
/// left out and why as the run does ("says why"): no build step said, no
/// build folder, no results.
#[test]
fn as_it_stands_only_refuses_before_building() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("stands-only");
    let target = t.to_str().unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = [\"-h\"]\n",
    )
    .unwrap();
    let refused = |words: &str| {
        let r = harness(
            &["perf", "run", "--target", target, "--as-it-stands-only"],
            None,
        );
        assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
        assert!(r.stderr.contains(words), "{words}: {}", r.stderr);
        assert!(!r.stdout.contains("building"), "{words}: {}", r.stdout);
        assert!(
            !t.join("migration/build/.perf").exists(),
            "{words}: perf built before refusing"
        );
        assert!(!t.join("migration/perf/program.json").exists(), "{words}");
    };
    refused("one measurable unit (u001-katajainen) — the program as it stands needs two");
    // A second verified unit that cannot be measured (no verdict) is named
    // with why, as the run names it — never "measured" before a build.
    edit(
        &t.join("migration/plan.toml"),
        "id = \"u-tree\"\nstatus = \"pending\"",
        "id = \"u-tree\"\nstatus = \"verified\"",
    );
    refused(
        "one measurable unit (u001-katajainen) — u-tree left out: verify it first — the \
         program as it stands needs two",
    );
    // No measurable unit, one left out: u001 is no longer verified.
    edit(
        &t.join("migration/plan.toml"),
        "id = \"u001-katajainen\"\nstatus = \"verified\"",
        "id = \"u001-katajainen\"\nstatus = \"pending\"",
    );
    refused(
        "no measurable unit — u-tree left out: verify it first — the program as it stands \
         needs two",
    );
    // No accepted unit at all.
    edit(
        &t.join("migration/plan.toml"),
        "id = \"u-tree\"\nstatus = \"verified\"",
        "id = \"u-tree\"\nstatus = \"pending\"",
    );
    refused("no accepted unit to compare yet");
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
/// computer is not checked, in those words; `--no-check` runs neither. A
/// stored row gives them something to judge.
#[test]
fn show_checks_the_compilers_in_the_sandbox() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("compilers");
    let target = t.to_str().unwrap();
    store_a_baseline(&t);
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

/// `perf show` stops at the first compiler that fails (§3.9): when `cc
/// --version` fails the compilers are already "not checked", so `rustc -V`
/// is never started — a hung one would cost a whole `[oracle]
/// timeout_secs` more. The stand-in rustc leaves a mark in a temp folder
/// (which every tool profile may write) before it hangs.
#[test]
fn show_stops_at_the_first_compiler_that_fails() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("first-compiler");
    let target = t.to_str().unwrap();
    store_a_baseline(&t);
    let bin = t.join("stand-in/bin");
    let marks = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("perf-cli-rustc-started-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&marks);
    std::fs::create_dir_all(&marks).unwrap();
    let mark = marks.join("rustc-started");
    script(&bin.join("cc"), "exit 1\n");
    script(
        &bin.join("rustc"),
        &format!("echo started > '{}'\nexec sleep 60\n", mark.display()),
    );
    edit(
        &t.join("harness.toml"),
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\ntimeout_secs = 30\n",
    );
    let path = path_with(&bin);
    let started = Instant::now();
    let r = harness_env(
        &["perf", "show", "--target", target],
        None,
        &[("PATH", &path)],
    );
    let took = started.elapsed();
    let rustc_ran = mark.exists();
    let _ = std::fs::remove_dir_all(&marks);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("perf: compilers not checked"),
        "{}",
        r.stdout
    );
    assert!(!rustc_ran, "rustc was started after cc failed");
    // Generous: a started rustc would be stopped only at 30 s.
    assert!(took < Duration::from_secs(25), "{took:?}");
}

/// With no row stored there is nothing to judge (§3.9): `perf show` runs no
/// compiler, checks no computer and does not say the C is not checked, and
/// says only that nothing is measured yet — on a target whose allowlist
/// lacks `cc`, a compiler check would read "compilers not checked"; in a
/// home folder without a launcher cache, a computer check would read
/// "computer not checked"; without facts, judging would read "the C not
/// checked". With a row stored, it checks again.
#[test]
fn show_checks_nothing_when_nothing_is_stored() {
    let t = zopfli("nothing-stored");
    let target = t.to_str().unwrap();
    edit(
        &t.join("harness.toml"),
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        "allowlist = [\"cargo\", \"rustc\", \"nm\"]\n",
    );
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(t.join("migration/perf/workloads.toml"), ONE_WORKLOAD).unwrap();
    // A home folder without perf's launcher cache.
    let home = t.with_extension("home");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let show = || {
        harness_env(
            &["perf", "show", "--target", target],
            None,
            &[("HOME", home.as_os_str())],
        )
    };
    let nothing = "perf: nothing measured yet — run harness perf run\n";
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, nothing, "{}", r.stdout);
    // And without facts.
    let facts = t.join("migration/facts.jsonl");
    let kept = std::fs::read(&facts).unwrap();
    std::fs::remove_file(&facts).unwrap();
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, nothing, "no facts: {}", r.stdout);
    std::fs::write(&facts, kept).unwrap();

    // A row stored: judged, and the compilers it could not run and the
    // computer it could not check are said.
    store_a_baseline(&t);
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("perf: compilers not checked"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains("perf: computer not checked — run harness perf run once"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("perf: the C on w — "), "{}", r.stdout);
}

/// Without facts `perf show` cannot hash the C (§3.9): it says so once —
/// once for the show, not once a row: two rows are stored — and judges the
/// rest — no row reads "the C changed", as in the cockpit — whether the
/// facts file is gone or cannot be read. With the facts the same rows are
/// current, and an edit to the C is said again.
#[test]
fn show_without_facts_does_not_judge_the_c() {
    use harness_core::perf::results as res;
    let t = zopfli("no-facts");
    let target = t.to_str().unwrap();
    store_a_baseline(&t);
    // A second row: the C alone on a second workload, `w2`.
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        format!("{ONE_WORKLOAD}[[workload]]\nid = \"w2\"\nargs = [\"-c\"]\n"),
    )
    .unwrap();
    let program = t.join("migration/perf/program.json");
    let mut file = res::read_program(&program).unwrap().unwrap();
    let w2 = stored_row_on(&t, "w2", "baseline", serde_json::json!({}));
    file.c_alone.push(serde_json::from_value(w2).unwrap());
    res::write_program(&program, &file).unwrap();
    let show = || harness(&["perf", "show", "--target", target, "--no-check"], None);
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("perf: the C on w — "), "{}", r.stdout);
    assert!(!r.stdout.contains("out of date"), "{}", r.stdout);
    assert!(!r.stdout.contains("not checked"), "{}", r.stdout);

    let facts = t.join("migration/facts.jsonl");
    let kept = std::fs::read(&facts).unwrap();
    for (case, bytes) in [("gone", None), ("unreadable", Some("not facts\n"))] {
        match bytes {
            None => std::fs::remove_file(&facts).unwrap(),
            Some(b) => std::fs::write(&facts, b).unwrap(),
        }
        let r = show();
        assert_eq!(r.code, 0, "{case}: {}", r.stderr);
        let shown = rows_of(&r.stdout)
            .iter()
            .filter(|r| r.starts_with("perf: the C on w"))
            .count();
        assert_eq!(shown, 2, "{case}: both rows shown: {}", r.stdout);
        assert_eq!(
            r.stdout
                .matches("perf: the C not checked: no facts — run harness scan")
                .count(),
            1,
            "{case}: said once, not once a row: {}",
            r.stdout
        );
        assert!(!r.stdout.contains("the C changed"), "{case}: {}", r.stdout);
        assert!(!r.stdout.contains("out of date"), "{case}: {}", r.stdout);
    }
    // The rest is still judged.
    edit(
        &t.join("migration/perf/workloads.toml"),
        "args = [\"-h\"]",
        "args = [\"-c\"]",
    );
    let r = show();
    let rows = rows_of(&r.stdout);
    let w = row(&rows, "perf: the C on w — ");
    assert!(w.contains("out of date: your workload changed"), "{w}");
    assert!(!w.contains("the C changed"), "{w}");

    // The facts back: an edit to the C is said again.
    std::fs::write(&facts, kept).unwrap();
    let main = t.join("src/zopfli/zopfli_bin.c");
    let mut text = std::fs::read_to_string(&main).unwrap();
    text.push_str("\n/* an edit */\n");
    std::fs::write(&main, text).unwrap();
    let r = show();
    assert!(r.stdout.contains("the C changed"), "{}", r.stdout);
    assert!(!r.stdout.contains("not checked"), "{}", r.stdout);
}

/// Without facts `perf show` cannot tell which units the program as it
/// stands holds today either ("left out now", "accepted since", the plan's
/// order): with such a row stored, the one not-checked line says so — once
/// — instead of letting the row read current unjudged. With the facts the
/// row is judged ("u001-katajainen is left out now"); and when the units
/// cannot be read although the facts can, the line says that too.
#[test]
fn show_without_facts_says_the_held_units_are_not_checked() {
    use harness_core::perf::results as res;
    let t = zopfli("no-facts-held");
    let target = t.to_str().unwrap();
    store_a_baseline(&t);
    let crate_dir = t.join("migration/units/u001-katajainen/katajainen_rs");
    let digest = harness_core::hash::unit_crate_file_set_hash(&t, &crate_dir).unwrap();
    let held = serde_json::json!({"units": [{"id": "u001-katajainen", "crate": digest}]});
    let program = t.join("migration/perf/program.json");
    let mut file = res::read_program(&program).unwrap().unwrap();
    file.as_it_stands
        .push(serde_json::from_value(stored_row(&t, "measured", held)).unwrap());
    res::write_program(&program, &file).unwrap();
    let show = || harness(&["perf", "show", "--target", target, "--no-check"], None);
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    let rows = rows_of(&r.stdout);
    let stands = row(&rows, "perf: the program as it stands on w — ");
    assert!(
        !stands.contains("out of date"),
        "current at first: {stands}"
    );

    let plan = t.join("migration/plan.toml");
    edit(
        &plan,
        "id = \"u001-katajainen\"\nstatus = \"verified\"",
        "id = \"u001-katajainen\"\nstatus = \"pending\"",
    );
    let r = show();
    let rows = rows_of(&r.stdout);
    let stands = row(&rows, "perf: the program as it stands on w — ");
    assert!(
        stands.contains("out of date: u001-katajainen is left out now"),
        "{stands}"
    );
    assert!(!r.stdout.contains("not checked"), "{}", r.stdout);

    let wider = "perf: the C and the units the program as it stands holds not checked: no facts \
                 — run harness scan";
    let facts = t.join("migration/facts.jsonl");
    let kept = std::fs::read(&facts).unwrap();
    std::fs::remove_file(&facts).unwrap();
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.matches(wider).count(), 1, "{}", r.stdout);
    assert!(
        !r.stdout.contains("perf: the C not checked"),
        "one line, not two: {}",
        r.stdout
    );
    let rows = rows_of(&r.stdout);
    let stands = row(&rows, "perf: the program as it stands on w — ");
    assert!(!stands.contains("out of date"), "{stands}");
    std::fs::write(&facts, kept).unwrap();

    // The facts read but the units do not (u001's folder is a file): the
    // held units are not checked, and the line says why.
    edit(
        &plan,
        "id = \"u001-katajainen\"\nstatus = \"pending\"",
        "id = \"u001-katajainen\"\nstatus = \"verified\"",
    );
    let unit_dir = t.join("migration/units/u001-katajainen");
    std::fs::rename(&unit_dir, t.join("u001-moved")).unwrap();
    std::fs::write(&unit_dir, "not a folder").unwrap();
    let r = show();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout
            .matches("perf: the units the program as it stands holds not checked: ")
            .count(),
        1,
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("left out now"), "{}", r.stdout);
}

/// One unit results file that cannot be read hides no other row in `perf
/// show` (as in the cockpit): the C alone's row and the units that read are
/// printed first, then the error names the bad file, and the show exits 1.
#[test]
fn show_prints_the_rows_that_read_before_a_bad_unit_file() {
    use harness_core::perf::results as res;
    let t = zopfli("bad-unit-file");
    let target = t.to_str().unwrap();
    store_a_baseline(&t);
    let units = t.join("migration/perf/units");
    std::fs::create_dir_all(&units).unwrap();
    std::fs::write(units.join("u001-katajainen.json"), "{\"junk\": 1}").unwrap();
    let show = || harness(&["perf", "show", "--target", target, "--no-check"], None);
    let r = show();
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(r.stdout.contains("perf: the C on w — "), "{}", r.stdout);
    assert!(
        r.stderr.contains("units/u001-katajainen.json"),
        "{}",
        r.stderr
    );
    assert!(!r.stdout.contains("nothing measured yet"), "{}", r.stdout);

    // A bad file beside a good one: the good unit's row is shown too, and
    // only the bad file is named.
    std::fs::write(units.join("u000-junk.json"), "{\"junk\": 1}").unwrap();
    let mut u001 = res::UnitResults::new("u001-katajainen");
    let digest = format!("blake3:{}", "a".repeat(64));
    let crates = serde_json::json!({"crates": [{"id": "u001-katajainen", "digest": digest}]});
    u001.rows
        .push(serde_json::from_value(stored_row(&t, "measured", crates)).unwrap());
    res::write_unit(&units.join("u001-katajainen.json"), &u001).unwrap();
    let r = show();
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(r.stdout.contains("perf: the C on w — "), "{}", r.stdout);
    assert!(
        r.stdout.contains("perf: u001-katajainen on w — "),
        "{}",
        r.stdout
    );
    assert!(r.stderr.contains("units/u000-junk.json"), "{}", r.stderr);
    assert!(r.stderr.contains("results file: "), "{}", r.stderr);
    assert!(!r.stderr.contains("invalid plan"), "{}", r.stderr);
    assert!(!r.stderr.contains("u001-katajainen.json"), "{}", r.stderr);
}

/// `perf show` only reads: it creates no folder, and it refuses a linked
/// `migration/perf` or `migration/perf/units` instead of reading another
/// folder's files as this target's rows (§3.9: links are refused on read) —
/// even a valid results file there, which a real units folder would show.
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

    // A units folder linked to one outside the project, which holds a
    // valid results file for u001.
    use harness_core::perf::results as res;
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(t.join("migration/perf/workloads.toml"), ONE_WORKLOAD).unwrap();
    let outside = t.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    let mut u001 = res::UnitResults::new("u001-katajainen");
    let digest = format!("blake3:{}", "a".repeat(64));
    let crates = serde_json::json!({"crates": [{"id": "u001-katajainen", "digest": digest}]});
    u001.rows
        .push(serde_json::from_value(stored_row(&t, "measured", crates)).unwrap());
    let u001_file = outside.join("u001-katajainen.json");
    res::write_unit(&u001_file, &u001).unwrap();
    assert!(res::read_unit(&u001_file, "u001-katajainen")
        .unwrap()
        .is_some());
    std::fs::write(
        outside.join("private-notes.json"),
        "{\"api_key\": \"SECRET-OUTSIDE\"}",
    )
    .unwrap();
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
        assert!(!text.contains("u001-katajainen on w"), "{text}");
        assert!(!text.contains("SECRET-OUTSIDE"), "{text}");
        assert!(!text.contains("private-notes"), "{text}");
    }

    // The same file in a real units folder is shown: the link alone was
    // refused.
    std::fs::remove_file(t.join("migration/perf/units")).unwrap();
    std::fs::create_dir(t.join("migration/perf/units")).unwrap();
    std::fs::copy(
        &u001_file,
        t.join("migration/perf/units/u001-katajainen.json"),
    )
    .unwrap();
    let r = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("perf: u001-katajainen on w — "),
        "{}",
        r.stdout
    );

    // And a linked migration/perf.
    std::fs::remove_dir_all(t.join("migration/perf")).unwrap();
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

/// u-util (zopfli's util.c, `ZopfliInitOptions`) in Rust: the same as the
/// C, except that options whose `verbose` is 0x5BE5 make it print 65 MiB
/// first — something the judge never tries (its driver zeroes the options).
const UTIL_RS: &str = r#"//! zopfli's util.c in Rust (a perf test's second unit).

use std::io::Write;

/// zopfli's `ZopfliOptions`, field for field.
#[repr(C)]
pub struct ZopfliOptions {
    verbose: i32,
    verbose_more: i32,
    numiterations: i32,
    blocksplitting: i32,
    blocksplittinglast: i32,
    blocksplittingmax: i32,
}

/// # Safety
/// `options` points at a writable, initialised `ZopfliOptions`.
#[no_mangle]
pub unsafe extern "C" fn ZopfliInitOptions(options: *mut ZopfliOptions) {
    let o = &mut *options;
    if o.verbose == 0x5BE5 {
        let chunk = vec![b'x'; 1 << 20];
        let mut out = std::io::stdout().lock();
        for _ in 0..65 {
            let _ = out.write_all(&chunk);
        }
        let _ = out.flush();
    }
    o.verbose = 0;
    o.verbose_more = 0;
    o.numiterations = 15;
    o.blocksplitting = 1;
    o.blocksplittinglast = 0;
    o.blocksplittingmax = 15;
}
"#;

/// u-util's differential driver.
const UTIL_DRIVER: &str = "#include <stdio.h>\n#include \"zopfli.h\"\n\n\
int main(void) {\n  ZopfliOptions o = {0};\n  ZopfliInitOptions(&o);\n  \
printf(\"%d %d %d %d %d %d\\n\", o.verbose, o.verbose_more, o.numiterations,\n         \
o.blocksplitting, o.blocksplittinglast, o.blocksplittingmax);\n  return 0;\n}\n";

/// The zopfli copy with a second verified unit (u-util, through `harness
/// verify`) and a C that, given `--crash`, aborts; given `--time`, prints
/// the time; given `--spew`, hands the options' defaulting a `verbose` of
/// 0x5BE5.
fn two_units(tag: &str) -> PathBuf {
    let t = zopfli(tag);
    let target = t.to_str().unwrap();
    let unit = t.join("migration/units/u-util");
    std::fs::create_dir_all(unit.join("util_rs/src")).unwrap();
    std::fs::write(
        unit.join("util_rs/Cargo.toml"),
        "[package]\nname = \"util_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(unit.join("util_rs/src/lib.rs"), UTIL_RS).unwrap();
    std::fs::write(unit.join("driver.c"), UTIL_DRIVER).unwrap();
    let ends = "symbols = [\"ZopfliInitOptions\"]\n\
                interface = [\"void ZopfliInitOptions(ZopfliOptions* options)\"]\n\
                depends_on = []\ntest_strategy = \"\"\ndone_criteria = \"\"\n";
    edit(
        &t.join("migration/plan.toml"),
        ends,
        &format!(
            "{ends}\n[unit.oracle]\nkind = \"c-abi-differential\"\n\
             driver = \"migration/units/u-util/driver.c\"\nrust_crate = \"util_rs\"\n\
             replaces = [\"src/zopfli/util.c\"]\n"
        ),
    );
    let main = t.join("src/zopfli/zopfli_bin.c");
    edit(
        &main,
        "#include <string.h>\n",
        "#include <string.h>\n#include <sys/time.h>\n",
    );
    edit(
        &main,
        "  ZopfliInitOptions(&options);\n",
        "  memset(&options, 0, sizeof(options));\n\
         \x20 if (argc > 1 && StringsEqual(argv[1], \"--spew\")) options.verbose = 0x5BE5;\n\
         \x20 ZopfliInitOptions(&options);\n\
         \x20 if (argc > 1 && StringsEqual(argv[1], \"--crash\")) abort();\n\
         \x20 if (argc > 1 && StringsEqual(argv[1], \"--time\")) {\n\
         \x20   struct timeval tv;\n\
         \x20   gettimeofday(&tv, 0);\n\
         \x20   printf(\"%ld.%06ld\\n\", (long)tv.tv_sec, (long)tv.tv_usec);\n\
         \x20   return 0;\n\
         \x20 }\n",
    );
    let r = harness(&["scan", "--target", target], None);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let r = harness(&["verify", "u-util", "--target", target], None);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    t
}

/// §4's end to end on two verified units (zopfli's u001 and u-util): the
/// program as it stands holds both in plan order; a crashing C is shown
/// only under the C; a C that prints the time is never "behaves
/// differently" on any side; output over the cap on the Rust side only is
/// "prints too much"; and after one unit's Rust changes, `perf show` says
/// so on that unit's rows and the program as it stands's.
#[test]
fn two_units_end_to_end() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = two_units("two");
    let target = t.to_str().unwrap();
    std::fs::create_dir_all(t.join("bench")).unwrap();
    std::fs::write(t.join("bench/tiny.txt"), "hi\n").unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(
        t.join("migration/perf/workloads.toml"),
        "schema_version = 1\n\
         [[workload]]\nid = \"tiny\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/tiny.txt\"\nruns = 5\n\
         [[workload]]\nid = \"crash\"\nargs = [\"--crash\"]\nruns = 5\n\
         [[workload]]\nid = \"time\"\nargs = [\"--time\"]\nruns = 5\n\
         [[workload]]\nid = \"spew\"\nargs = [\"--spew\"]\nruns = 5\n",
    )
    .unwrap();
    let r = harness(&["perf", "run", "--target", target, "--json"], None);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("the program as it stands — u001-katajainen, u-util"),
        "{}",
        r.stdout
    );
    let events = perf_rows(&r.stdout);
    let on = |workload: &str| -> Vec<&serde_json::Value> {
        events
            .iter()
            .filter(|e| e["workload"] == workload)
            .collect()
    };
    let program = harness_core::perf::results::read_program(&t.join("migration/perf/program.json"))
        .unwrap()
        .unwrap();
    let unit_rows = |id: &str| {
        harness_core::perf::results::read_unit(
            &t.join(format!("migration/perf/units/{id}.json")),
            id,
        )
        .unwrap()
        .unwrap()
        .rows
    };
    let (u001, util) = (unit_rows("u001-katajainen"), unit_rows("u-util"));
    let stored = |rows: &[harness_core::perf::results::Row], workload: &str| {
        rows.iter().find(|r| r.workload == workload).cloned()
    };

    // The program as it stands: both units, in plan order, none left out.
    assert!(!program.as_it_stands.is_empty());
    for row in &program.as_it_stands {
        let ids: Vec<&str> = row
            .inputs
            .units
            .iter()
            .flatten()
            .map(|u| u.id.as_str())
            .collect();
        assert_eq!(ids, ["u001-katajainen", "u-util"], "{}", row.workload);
        assert!(row.inputs.left_out.iter().flatten().next().is_none());
    }

    // A crashing C: its row says so, and only ever under the C — once: the
    // C failed in step 1, so the workload's other rows are not run (§3.5).
    let crash = on("crash");
    assert_eq!(crash.len(), 1, "{crash:?}");
    for e in &crash {
        assert_eq!(e["side"], "c", "{e}");
        assert_eq!(e["outcome"], "c-crashed", "{e}");
        assert!(
            e["words"]
                .as_str()
                .unwrap()
                .contains("the C crashes on crash"),
            "{e}"
        );
    }
    assert_eq!(
        stored(&program.c_alone, "crash").unwrap().outcome,
        "c-crashed"
    );
    for rows in [&u001, &util, &program.as_it_stands] {
        assert!(stored(rows, "crash").is_none());
    }

    // A C that prints the time cannot be compared against, and no side
    // reads "behaves differently" for it: its one row is the C's.
    assert_eq!(
        stored(&program.c_alone, "time").unwrap().outcome,
        "c-unstable"
    );
    let time = on("time");
    assert_eq!(time.len(), 1, "{time:?}");
    assert_eq!(time[0]["side"], "c", "{time:?}");
    for e in time {
        assert_ne!(e["outcome"], "behaves-differently", "{e}");
    }
    for rows in [&u001, &util, &program.as_it_stands] {
        assert!(stored(rows, "time").is_none_or(|r| r.outcome != "behaves-differently"));
    }

    // Output over the cap on the Rust side only: u-util and the program as
    // it stands print too much; the C alone and u001 do not.
    for (side, rows) in [("unit", &util), ("program", &program.as_it_stands)] {
        let e = on("spew")
            .into_iter()
            .find(|e| e["side"] == side && (side == "program" || e["unit"] == "u-util"))
            .unwrap_or_else(|| panic!("{side}: {events:?}"))
            .clone();
        assert_eq!(e["outcome"], "behaves-differently", "{e}");
        assert!(
            e["words"].as_str().unwrap().contains("more than 64 MiB"),
            "{e}"
        );
        let row = stored(rows, "spew").unwrap();
        assert_eq!(row.outcome, "behaves-differently");
        assert!(row.first_difference.as_ref().is_some_and(|d| d.over_cap));
    }
    assert_ne!(
        stored(&u001, "spew").unwrap().outcome,
        "behaves-differently"
    );
    assert_ne!(
        stored(&program.c_alone, "spew").unwrap().outcome,
        "behaves-differently"
    );

    // Just measured: every row current, and the words rebuilt.
    let show = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(show.code, 0, "{}", show.stderr);
    assert!(!show.stdout.contains("out of date"), "{}", show.stdout);
    let rows = rows_of(&show.stdout);
    assert!(row(&rows, "perf: u-util on spew — ").contains("more than 64 MiB"));

    // u-util's Rust changes: its rows and the program as it stands's say
    // so; u001's stay current; the plan's order did not change.
    let lib = t.join("migration/units/u-util/util_rs/src/lib.rs");
    let mut text = std::fs::read_to_string(&lib).unwrap();
    text.push_str("\n// an edit\n");
    std::fs::write(&lib, text).unwrap();
    let show = harness(&["perf", "show", "--target", target, "--no-check"], None);
    assert_eq!(show.code, 0, "{}", show.stderr);
    let rows = rows_of(&show.stdout);
    for start in [
        "perf: u-util on tiny — ",
        "perf: the program as it stands on tiny — ",
    ] {
        assert!(
            row(&rows, start).contains("u-util's Rust changed since"),
            "{}",
            show.stdout
        );
    }
    assert!(
        !row(&rows, "perf: u001-katajainen on tiny — ").contains("out of date"),
        "{}",
        show.stdout
    );
    assert!(
        !show.stdout.contains("the plan's order changed"),
        "{}",
        show.stdout
    );
}

/// `perf run` reads the compilers with `perf show`'s own code (§3.9): the
/// same runs, output cap and first line, so what it stores is what a check
/// reads. Here the target's `rustc` is a stand-in that answers `rustc -V`
/// alone: first with two lines (the first is stored), then with its line
/// and more than the cap — a version `perf show` cannot read, so `perf
/// run` cannot either: stored as the bare name, never a line a check would
/// not see. A unit row names its rustc; u001 is not measured (its Rust
/// changed since verify), so no build needs that rustc.
#[test]
fn run_reads_the_compilers_as_show_does() {
    if !cfg!(target_os = "macos") {
        eprintln!("perf runs on macOS only: skipped");
        return;
    }
    let t = zopfli("run-compilers");
    let target = t.to_str().unwrap();
    std::fs::create_dir_all(t.join("migration/perf")).unwrap();
    std::fs::write(t.join("migration/perf/workloads.toml"), ONE_WORKLOAD).unwrap();
    let lib = t.join("migration/units/u001-katajainen/katajainen_rs/src/lib.rs");
    let mut text = std::fs::read_to_string(&lib).unwrap();
    text.push_str("\n// an edit since verify\n");
    std::fs::write(&lib, text).unwrap();
    let bin = t.join("stand-in/bin");
    let path = path_with(&bin);
    let stored_rustc = || {
        let r = harness_env(
            &["perf", "run", "--target", target],
            None,
            &[("PATH", &path)],
        );
        assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
        let unit = harness_core::perf::results::read_unit(
            &t.join("migration/perf/units/u001-katajainen.json"),
            "u001-katajainen",
        )
        .unwrap()
        .unwrap();
        let w = unit.rows.iter().find(|r| r.workload == "w").unwrap();
        assert_eq!(w.outcome, "not-verified");
        w.inputs.compilers.rustc.clone()
    };
    let only_v = "[ \"$*\" = \"-V\" ] || exit 1\necho 'rustc 1.0.0 (stand-in)'\n";
    script(
        &bin.join("rustc"),
        &format!("{only_v}echo 'a second line'\n"),
    );
    assert_eq!(stored_rustc().as_deref(), Some("rustc 1.0.0 (stand-in)"));
    let long = "x".repeat(80);
    script(
        &bin.join("rustc"),
        &format!("{only_v}i=0\nwhile [ $i -lt 1000 ]; do echo {long}; i=$((i+1)); done\n"),
    );
    assert_eq!(stored_rustc().as_deref(), Some("rustc"));
    // perf show cannot read it either.
    let show = harness_env(
        &["perf", "show", "--target", target],
        None,
        &[("PATH", &path)],
    );
    assert_eq!(show.code, 0, "{}", show.stderr);
    assert!(
        show.stdout.contains("perf: compilers not checked"),
        "{}",
        show.stdout
    );
}
