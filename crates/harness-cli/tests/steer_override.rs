//! The review acts' CLI additions (docs/TUI-DESIGN.md §5) end to end on a
//! temp copy of the vendored zopfli target: a steer attempt through the
//! `external` hand-off, resumed from the human `resume` hint through a
//! shell; and a labelled human attempt through `harness override`, promoted
//! by `harness promote`, with its refusals.

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn harness(args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .output()
        .expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn events(run: &Run) -> Vec<serde_json::Value> {
    run.stdout
        .lines()
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("not an event line: {l:?}: {e}"))
        })
        .collect()
}

fn find<'a>(evs: &'a [serde_json::Value], k: &str) -> Option<&'a serde_json::Value> {
    evs.iter().find(|e| e["k"] == k)
}

fn pending_request(dir: &Path) -> Option<PathBuf> {
    let mut pending: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".request.json"))
        .filter(|p| {
            !PathBuf::from(
                p.to_string_lossy()
                    .replace(".request.json", ".response.json"),
            )
            .exists()
        })
        .collect();
    pending.sort();
    pending.pop()
}

fn emission(logic: &str, ffi: &str) -> String {
    format!(
        "src/logic.rs\n```rust\n{logic}```\nsrc/ffi.rs\n```rust\n{ffi}```\nRUHARNESS_END_OF_OUTPUT\n"
    )
}

fn write_response(request: &Path, text: &str) {
    let response = PathBuf::from(
        request
            .to_string_lossy()
            .replace(".request.json", ".response.json"),
    );
    let body = serde_json::json!({
        "text": text, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"
    });
    std::fs::write(response, serde_json::to_string_pretty(&body).unwrap()).unwrap();
}

fn attempt_dirs(unit_dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(unit_dir.join("attempts"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn record(unit_dir: &Path, id: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(unit_dir.join("attempts").join(id).join("attempt.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn steer_and_override_on_zopfli() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // A path with a space: the resume hint must quote it.
    let tmp = std::env::temp_dir().join(format!("ruharness steer {}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&repo_root.join("targets/zopfli"), &tmp);
    let target = tmp.to_str().unwrap();
    let unit = "u001-katajainen";
    let unit_dir = tmp.join("migration/units").join(unit);
    let traces = unit_dir.join("traces");
    let _ = std::fs::remove_dir_all(&traces);
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));
    let logic = include_str!("fixtures/katajainen_logic.rs");
    let ffi = include_str!("fixtures/katajainen_ffi.rs");

    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let r = harness(&["plan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);

    // ---- a seed: a green translate attempt through the hand-off.
    let migrate = |extra: &[&str]| {
        let mut args = vec![
            "--json",
            "migrate",
            "--allow-unsandboxed",
            unit,
            "--target",
            target,
            "--no-promote",
        ];
        args.extend_from_slice(extra);
        harness(&args)
    };
    let r = migrate(&[]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    write_response(&pending_request(&traces).unwrap(), &emission(logic, ffi));
    let r = migrate(&[]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let seed = find(&events(&r), "attempt").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // ---- --steer without --from: refused, listing the seeds.
    let r = migrate(&["--steer", "make it faster"]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("--steer and --from go together") && r.stderr.contains(&seed),
        "{}",
        r.stderr
    );

    // ---- a steer attempt whose note needs quoting, through the hand-off.
    let note = "Don't index twice; use \"iter()\" & keep $x wrapping.";
    let r = migrate(&["--from", &seed, "--steer", note]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let awaiting = find(&evs, "awaiting").expect("awaiting event");
    let steer_id = awaiting["attempt"].as_str().unwrap().to_string();
    assert_ne!(steer_id, seed);
    let args: Vec<&str> = awaiting["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(
        args.contains(&note),
        "args carry the note verbatim: {args:?}"
    );
    assert!(!args.contains(&"--json"), "{args:?}");
    let resume = awaiting["resume"].as_str().unwrap().to_string();
    assert!(resume.starts_with("harness migrate "), "{resume}");
    let request_path = pending_request(&traces).expect("the steer request");
    let request = std::fs::read_to_string(&request_path).unwrap();
    assert!(request.contains("[GUIDANCE]"), "{request}");
    // The steer turn's reply: the same (green) translation, reformatted, so
    // it is a different candidate.
    let revised = logic.replacen("\n", "\n\n", 1);
    write_response(&request_path, &emission(&revised, ffi));
    // Resume the way a human would: the hint, through a shell, with the
    // `harness` it names resolved to this build.
    let bin_dir = PathBuf::from(env!("CARGO_BIN_EXE_harness"))
        .parent()
        .unwrap()
        .to_path_buf();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new("/bin/sh")
        .args(["-c", &resume])
        .env("PATH", path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains(&format!("attempt {steer_id}")), "{stdout}");
    assert!(stdout.contains("not promoted (--no-promote)"), "{stdout}");
    assert_eq!(
        attempt_dirs(&unit_dir),
        {
            let mut v = vec![seed.clone(), steer_id.clone()];
            v.sort();
            v
        },
        "the resume finished the SAME attempt"
    );
    let steer = record(&unit_dir, &steer_id);
    assert_eq!(steer["seeded_from"], seed.as_str());
    assert_eq!(steer["steer_note"], note);
    assert_eq!(steer["turns"][0]["kind"], "steer");
    assert_eq!(steer["outcome"], "green");
    // Evidence-first replay of the steer attempt from its record alone.
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        unit,
        "--target",
        target,
        "--provider",
        "replay",
        "--attempt",
        &steer_id,
    ]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("prompt: conformant"), "{}", r.stdout);

    // ---- override: a hand edit of the steer candidate.
    let edit = std::env::temp_dir().join(format!("ruharness-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&edit);
    std::fs::create_dir_all(edit.join("src")).unwrap();
    let cand = unit_dir
        .join("attempts")
        .join(&steer_id)
        .join("candidate/src");
    std::fs::copy(cand.join("logic.rs"), edit.join("src/logic.rs")).unwrap();
    std::fs::copy(cand.join("ffi.rs"), edit.join("src/ffi.rs")).unwrap();
    let edit_s = edit.to_str().unwrap();
    let override_ = |extra: &[&str]| {
        let mut args = vec!["--json", "override", "--allow-unsandboxed", unit, edit_s];
        args.extend_from_slice(&["--target", target]);
        args.extend_from_slice(extra);
        harness(&args)
    };
    // Unchanged: refused, nothing recorded.
    let before = attempt_dirs(&unit_dir);
    let r = override_(&[]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr
            .contains(&format!("identical to attempt {steer_id}")),
        "{}",
        r.stderr
    );
    assert_eq!(attempt_dirs(&unit_dir), before);
    // Shape refusals.
    std::fs::write(edit.join("src/extra.rs"), "pub fn x() {}\n").unwrap();
    let r = override_(&[]);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("src/extra.rs is not accepted"),
        "{}",
        r.stderr
    );
    std::fs::remove_file(edit.join("src/extra.rs")).unwrap();
    std::fs::write(
        edit.join("Cargo.toml"),
        "[package]\nname = \"x\"\n[build-dependencies]\n",
    )
    .unwrap();
    let r = override_(&[]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("Cargo.toml differs"), "{}", r.stderr);
    std::fs::remove_file(edit.join("Cargo.toml")).unwrap();
    // A real edit (a comment): judged green, labelled human, not promoted.
    let mut hand = std::fs::read_to_string(edit.join("src/logic.rs")).unwrap();
    hand.push_str("\n// reviewed by hand\n");
    std::fs::write(edit.join("src/logic.rs"), &hand).unwrap();
    let r = override_(&["--note", "a comment for the reviewer"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let att = find(&evs, "attempt").unwrap();
    assert_eq!(att["provider"], "human");
    assert_eq!(att["outcome"], "green");
    assert_eq!(att["promoted"], false);
    assert!(find(&evs, "verdict").unwrap()["path"]
        .as_str()
        .unwrap()
        .ends_with("attempt-verdict.json"));
    let human_id = att["id"].as_str().unwrap().to_string();
    let human = record(&unit_dir, &human_id);
    assert_eq!(human["provider_kind"], "human");
    assert_eq!(human["model"], "-");
    assert_eq!(human["turns"][0]["kind"], "human");
    assert_eq!(human["note"], "a comment for the reviewer");
    // Accept it: a human attempt promotes like any green attempt.
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        &human_id,
        "--target",
        target,
        "--replace",
    ]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("promoted and verified"), "{}", r.stdout);
    // The status report lists it with its label.
    let r = harness(&["state", "status", "--target", target]);
    assert!(
        r.stdout.contains(&format!("{human_id}:human:green")),
        "{}",
        r.stdout
    );

    let _ = std::fs::remove_dir_all(&edit);
    let _ = std::fs::remove_dir_all(&tmp);
}
