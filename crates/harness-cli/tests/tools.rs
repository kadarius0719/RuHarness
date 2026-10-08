//! Mapped tools through the binary (docs/PROJECT-MAP-DESIGN.md §3.7): the
//! lookup order of `--target` and `--tool`, every ledger path of a tool
//! under `migration/tools/<id>/`, one `sync-runtime` block per tool, and a
//! file-list target read by every command (tests/file_list.rs runs them in
//! full).

use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "harness-cli-tools-{tag}-{}-{}",
        std::process::id(),
        harness_core::hash::random_hex(4)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
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

fn folder_toml(name: &str, source_dir: &str) -> String {
    format!("schema_version = 1\n[target]\nname = \"{name}\"\nsource_dir = \"{source_dir}\"\n")
}

/// A project of two programs, `src/a` and `src/b`, each mapped as a tool
/// whose `harness.toml` is the folder form over its own folder.
fn project(tag: &str) -> PathBuf {
    let root = tmp(tag);
    for (dir, body) in [
        ("src/a", "int a_twice(int x) { return 2 * x; }\n"),
        ("src/b", "int b_plus(int x) { return x + 1; }\n"),
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        let name = dir.rsplit('/').next().unwrap();
        std::fs::write(root.join(dir).join(format!("{name}.c")), body).unwrap();
    }
    for (id, dir) in [("t-a", "src/a"), ("t-b", "src/b")] {
        let tool = harness_core::config::tool_dir(&root, id);
        std::fs::create_dir_all(&tool).unwrap();
        std::fs::write(tool.join("harness.toml"), folder_toml(id, dir)).unwrap();
    }
    root
}

#[test]
fn the_lookup_order_and_every_ledger_path_of_a_tool() {
    let root = project("lookup");
    let target = root.to_str().unwrap();
    // Two tools, no --tool: refused, both named (and no adoption asked: a
    // migration/ of hand-written tools holds no results).
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("2 mapped tools") && r.stderr.contains("t-a, t-b"),
        "{}",
        r.stderr
    );
    // A bad id: a usage error, in one sentence.
    let r = harness(&["scan", "--target", target, "--tool", "T-A"]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("is not a tool id"), "{}", r.stderr);
    let r = harness(&["scan", "--target", target, "--tool", "t-c"]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("no mapped tool t-c"), "{}", r.stderr);
    // --tool t-a: its ledger, and only its.
    for cmd in ["scan", "plan"] {
        let r = harness(&[cmd, "--target", target, "--tool", "t-a"]);
        assert_eq!(r.code, 0, "{cmd}: {}\n{}", r.stdout, r.stderr);
    }
    let a = root.join("migration/tools/t-a");
    assert!(a.join("facts.jsonl").is_file());
    assert!(a.join("plan.toml").is_file());
    assert!(!root.join("migration/facts.jsonl").exists());
    assert!(!root.join("migration/plan.toml").exists());
    let facts = std::fs::read_to_string(a.join("facts.jsonl")).unwrap();
    assert!(facts.contains("src/a/a.c") && !facts.contains("src/b/b.c"));
    // The detectors write under the tool's ledger too.
    let r = harness(&["detect", "--target", target, "--tool", "t-a"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(a.join("observer").is_dir());
    assert!(!root.join("migration/observer").exists());
    // `features init` and `perf init` write in the tool's ledger.
    for args in [["features", "init"], ["perf", "init"]] {
        let r = harness(&[args[0], args[1], "--target", target, "--tool", "t-a"]);
        assert_eq!(r.code, 0, "{args:?}: {}\n{}", r.stdout, r.stderr);
    }
    assert!(a.join("features/features.toml").is_file());
    assert!(a.join("perf/workloads.toml").is_file());
    assert!(!root.join("migration/features").exists());
    assert!(!root.join("migration/perf").exists());
    // Everything the tool's commands wrote lies under its folder (the
    // root's migration/ holds the token, the lock-free tools/ and nothing
    // else).
    let mut top: Vec<String> = std::fs::read_dir(root.join("migration"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    assert_eq!(top, [".ruharness-adopted", "tools"], "{top:?}");

    // Only one tool left: opened without --tool.
    std::fs::remove_dir_all(root.join("migration/tools/t-b")).unwrap();
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    // A root harness.toml wins without --tool: its ledger is migration/.
    std::fs::write(root.join("harness.toml"), folder_toml("whole", "src")).unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(root.join("migration/facts.jsonl").is_file());
    // ...and --tool still opens the tool, never the root file.
    let before = std::fs::read(a.join("facts.jsonl")).unwrap();
    std::fs::write(root.join("src/a/a2.c"), "int a_more(void) { return 3; }\n").unwrap();
    let r = harness(&["scan", "--target", target, "--tool", "t-a"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let after = std::fs::read_to_string(a.join("facts.jsonl")).unwrap();
    assert_ne!(before, after.as_bytes());
    assert!(after.contains("src/a/a2.c") && !after.contains("src/b/b.c"));
}

/// `harness` with `stdin` piped in.
fn harness_in(args: &[&str], stdin: &str) -> Run {
    use std::io::Write;
    let file = harness_core::adopt::testing::adoption_file();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .env(harness_core::adopt::ADOPTED_ENV, file)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn harness");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Every command that takes `--tool` opens the tool's ledger with it,
/// through the binary: without it a project of two tools is refused; with
/// it the command reaches its own work (or its own reason) in that tool's
/// ledger, and its next-step hints spell `--tool` so they open it again.
#[test]
fn every_tool_command_opens_the_tool_it_names() {
    let root = project("commands");
    let target = root.to_str().unwrap();
    let a = root.join("migration/tools/t-a");
    let edit = root.join("edit");
    std::fs::create_dir_all(edit.join("src")).unwrap();
    let edit = edit.to_str().unwrap();
    let with_tool = |args: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        v.extend(["--target", target, "--tool", "t-a"].map(String::from));
        v
    };
    let run = |args: &[String]| harness(&args.iter().map(String::as_str).collect::<Vec<_>>());
    // Before a scan: each reaches the tool's ledger and says to scan it.
    let cases: [(&[&str], &str); 4] = [
        (
            &["review", "f-x", "--uphold-dismiss"],
            "run `harness detect --tool t-a` first",
        ),
        (
            &["migrate", "u-x", "--allow-unsandboxed"],
            "migration/tools/t-a",
        ),
        (
            &["override", "u-x", edit, "--allow-unsandboxed"],
            "migration/tools/t-a",
        ),
        (
            &["promote", "u-x", "a-1", "--allow-unsandboxed"],
            "migration/tools/t-a",
        ),
    ];
    for (args, says) in cases {
        let r = harness(&[args, &["--target", target]].concat());
        assert_eq!(r.code, 1, "{args:?} without --tool: {}", r.stderr);
        assert!(
            r.stderr.contains("2 mapped tools"),
            "{args:?}: {}",
            r.stderr
        );
        let r = run(&with_tool(args));
        assert_eq!(r.code, 1, "{args:?}: {}\n{}", r.stdout, r.stderr);
        assert!(
            r.stderr.contains(says) && !r.stderr.contains("mapped tools"),
            "{args:?}: {}",
            r.stderr
        );
    }
    // perf show before anything: the starter's command carries --tool.
    let r = run(&with_tool(&["perf", "show", "--no-check"]));
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("write your workloads file first — harness perf init --tool t-a"),
        "{}",
        r.stdout
    );
    // features save and perf save write the tool's files, named by its path.
    let features = "schema_version = 1\n";
    let r = harness_in(
        &with_tool(&["features", "save", "--expect", "none", "--bytes", "19"])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        features,
    );
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("features: saved migration/tools/t-a/features/features.toml"),
        "{}",
        r.stdout
    );
    assert!(a.join("features/features.toml").is_file());
    let r = harness_in(
        &with_tool(&["perf", "save", "--expect", "none", "--bytes", "19"])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        features,
    );
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(a.join("perf/workloads.toml").is_file());
    assert!(!root.join("migration/features").exists() && !root.join("migration/perf").exists());
    // perf show: the workloads file it names is the tool's.
    let r = run(&with_tool(&["perf", "show", "--no-check"]));
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("add a [[workload]] to migration/tools/t-a/perf/workloads.toml"),
        "{}",
        r.stdout
    );
    // A features file with an error is named by the tool's path.
    let bad = "schema_version = 1\nnope = 1\n";
    let expect = harness_core::hash::bytes_hash(features.as_bytes());
    let bytes = bad.len().to_string();
    let r = harness_in(
        &with_tool(&["features", "save", "--expect", &expect, "--bytes", &bytes])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        bad,
    );
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr
            .contains("error: migration/tools/t-a/features/features.toml:"),
        "{}",
        r.stderr
    );
}

/// No target here, and `--tool` on a project without tools: one sentence
/// each, exit 1.
#[test]
fn a_folder_that_names_no_target_is_told_so_plainly() {
    let root = tmp("no-target");
    let target = root.to_str().unwrap();
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert_eq!(
        r.stderr.trim_end(),
        format!(
            "error: {target} is not a harness target (no harness.toml, and no mapped tool under \
             migration/tools/); point --target at a folder that holds a harness.toml"
        )
    );
    std::fs::write(root.join("harness.toml"), folder_toml("x", ".")).unwrap();
    let r = harness(&["state", "status", "--target", target, "--tool", "t-a"]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert_eq!(
        r.stderr.trim_end(),
        format!(
            "error: {target}: this project has no mapped tools; drop --tool (its harness.toml \
             is the target)"
        )
    );
}

#[test]
fn sync_runtime_keeps_one_block_per_tool() {
    let root = project("sync");
    let target = root.to_str().unwrap();
    for id in ["t-a", "t-b"] {
        for cmd in ["scan", "plan"] {
            let r = harness(&[cmd, "--target", target, "--tool", id]);
            assert_eq!(r.code, 0, "{id} {cmd}: {}\n{}", r.stdout, r.stderr);
        }
    }
    for id in ["t-a", "t-b"] {
        let r = harness(&["sync-runtime", "--target", target, "--tool", id]);
        assert_eq!(r.code, 0, "{id}: {}\n{}", r.stdout, r.stderr);
    }
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents.matches("BEGIN RUHARNESS GENERATED").count(),
        2,
        "{agents}"
    );
    for id in ["t-a", "t-b"] {
        assert!(
            agents.contains(&format!(
                "tool={id} (source: migration/tools/{id}/ — do not edit; run `harness \
                 sync-runtime --tool {id}`)"
            )),
            "{agents}"
        );
        assert!(agents.contains(&format!("`migration/tools/{id}/plan.toml`")));
        let r = harness(&["sync-runtime", "--target", target, "--tool", id, "--check"]);
        assert_eq!(r.code, 0, "{id} check: {}\n{}", r.stdout, r.stderr);
    }
    // Syncing one again leaves the other's block as it was.
    let r = harness(&["sync-runtime", "--target", target, "--tool", "t-a"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        agents
    );
    // A drifted block is named with its own command.
    std::fs::write(
        root.join("AGENTS.md"),
        agents.replace("(tool t-b)", "(tool x)"),
    )
    .unwrap();
    let r = harness(&[
        "sync-runtime",
        "--target",
        target,
        "--tool",
        "t-b",
        "--check",
    ]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stderr.contains("run `harness sync-runtime --tool t-b`"),
        "{}",
        r.stderr
    );
    let r = harness(&[
        "sync-runtime",
        "--target",
        target,
        "--tool",
        "t-a",
        "--check",
    ]);
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn a_file_list_target_is_refused_by_no_command() {
    let root = tmp("file-list");
    for dir in ["src/lib", "src/include"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("src/lib/lzg.c"), "int lzg(void) { return 0; }\n").unwrap();
    let tool = harness_core::config::tool_dir(&root, "t-lzg");
    std::fs::create_dir_all(&tool).unwrap();
    std::fs::write(
        tool.join("harness.toml"),
        "schema_version = 2\n[target]\nname = \"lzg\"\n\
         files = [{ path = \"src/lib/lzg.c\", include_dirs = [\"src/include\"] }]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [\"-std=c99\"] }\n",
    )
    .unwrap();
    let target = root.to_str().unwrap();
    // The scanner and the detectors read the file list.
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let facts = std::fs::read_to_string(tool.join("facts.jsonl")).unwrap();
    assert!(facts.contains("src/lib/lzg.c"), "{facts}");
    let r = harness(&["detect", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    // Every other command opens it: plan writes the tool's plan; verify,
    // gen-driver and perf go as far as their own reasons (no such unit; no
    // workloads file), observe all the way — each by its exit code and its
    // own words, never refusing the form.
    let r = harness(&["plan", "--target", target, "--tool", "t-lzg"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(Path::new(&tool).join("plan.toml").is_file());
    for (args, code, why) in [
        (
            vec!["verify", "u-x", "--allow-unsandboxed"],
            1,
            "error: unknown unit `u-x`",
        ),
        (vec!["observe"], 0, ""),
        (vec!["gen-driver", "u-x"], 1, "error: unknown unit `u-x`"),
        (
            vec!["perf", "run"],
            1,
            "error: write your workloads file first — harness perf init --tool t-lzg gives a \
             starter",
        ),
    ] {
        let mut argv = args.clone();
        argv.extend(["--target", target, "--tool", "t-lzg"]);
        let r = harness(&argv);
        assert_eq!(r.code, code, "{args:?}: {}\n{}", r.stdout, r.stderr);
        assert_eq!(r.stderr.trim_end(), why, "{args:?}: {}", r.stdout);
    }
    // A too-new schema says so, before anything else.
    std::fs::write(tool.join("harness.toml"), "schema_version = 3\n").unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stderr.contains("schema_version 3") && r.stderr.contains("upgrade the harness"),
        "{}",
        r.stderr
    );
}
