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
        .env("PATH", &path)
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

    // ---- §R2 5: a note that starts with `-` travels attached
    //      (`--steer=<note>`, what the cockpit spawns), and the resume hint
    //      keeps it attached so the shell hands clap one word.
    let dash_note = "-keep the wrapping add";
    let steer_arg = format!("--steer={dash_note}");
    let r = migrate(&["--from", &seed, &steer_arg]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let awaiting = find(&evs, "awaiting").expect("awaiting event");
    let dash_id = awaiting["attempt"].as_str().unwrap().to_string();
    let resume = awaiting["resume"].as_str().unwrap().to_string();
    assert!(
        resume.contains(&format!("--steer='{dash_note}'")),
        "{resume}"
    );
    let request_path = pending_request(&traces).expect("the dash steer request");
    write_response(
        &request_path,
        &emission(&logic.replacen("\n", "\n\n\n", 1), ffi),
    );
    let out = Command::new("/bin/sh")
        .args(["-c", &resume])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let dash = record(&unit_dir, &dash_id);
    assert_eq!(dash["steer_note"], dash_note);
    assert_eq!(dash["outcome"], "green");
    assert_eq!(
        attempt_dirs(&unit_dir).len(),
        3,
        "the resume finished the SAME attempt"
    );

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
    // §R2 4: an override killed mid-judge leaves its record in-progress;
    // the same edit then reclaims it instead of being refused for good.
    let attempt_json = unit_dir
        .join("attempts")
        .join(&human_id)
        .join("attempt.json");
    let finished = std::fs::read_to_string(&attempt_json).unwrap();
    let mut interrupted: serde_json::Value = serde_json::from_str(&finished).unwrap();
    interrupted["outcome"] = "in-progress".into();
    interrupted["turns"] = serde_json::json!([]);
    interrupted["candidate_digest"] = "".into();
    std::fs::write(
        &attempt_json,
        serde_json::to_string_pretty(&interrupted).unwrap(),
    )
    .unwrap();
    let r = override_(&["--note", "a comment for the reviewer"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(record(&unit_dir, &human_id)["outcome"], "green");
    assert_eq!(std::fs::read_to_string(&attempt_json).unwrap(), finished);
    // §R2 6: a deny-scan red says why, and keeps what was submitted.
    let good_logic = hand.clone();
    std::fs::write(
        edit.join("src/logic.rs"),
        format!("{hand}\npub fn sneaky() {{ unsafe {{}} }}\n"),
    )
    .unwrap();
    let r = override_(&[]);
    assert_eq!(r.code, 10, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    assert!(
        evs.iter().any(|e| e["k"] == "message"
            && e["text"]
                .as_str()
                .unwrap_or("")
                .starts_with("override: deny scan: ")
            && e["text"].as_str().unwrap_or("").contains("unsafe")),
        "{}",
        r.stdout
    );
    let denied_id = find(&evs, "attempt").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(unit_dir
        .join("attempts")
        .join(&denied_id)
        .join("edit/src/logic.rs")
        .is_file());
    std::fs::write(edit.join("src/logic.rs"), &good_logic).unwrap();
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

/// docs/CHAT-PANE-DESIGN.md §4: a chat-requested steer through the CLI —
/// labelled, its hand-off in `traces/chat/` with the event's `request_key`,
/// its resume hint and args carrying the label; answered with `--answer`
/// (filed by the CLI, only for exactly that request), refused up front when
/// the attempt does not wait on it, and `answer-unused` when it waits on
/// another request.
#[test]
fn a_chat_steer_is_labelled_and_answered_through_the_cli() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("ruharness-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&repo_root.join("targets/zopfli"), &tmp);
    // The CLI names canonical paths (/var → /private/var on macOS).
    let tmp = tmp.canonicalize().unwrap();
    let target = tmp.to_str().unwrap();
    let unit = "u001-katajainen";
    let unit_dir = tmp.join("migration/units").join(unit);
    let traces = unit_dir.join("traces");
    let _ = std::fs::remove_dir_all(&traces);
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));
    let logic = include_str!("fixtures/katajainen_logic.rs");
    let ffi = include_str!("fixtures/katajainen_ffi.rs");
    for cmd in ["scan", "plan"] {
        let r = harness(&[cmd, "--target", target]);
        assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    }
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
    // A blind seed, through the flat traces.
    assert_eq!(migrate(&[]).code, 1);
    write_response(&pending_request(&traces).unwrap(), &emission(logic, ffi));
    let r = migrate(&[]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let seed = find(&events(&r), "attempt").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Two chat steers of the seed: each waits on its own request in
    // traces/chat, the event naming its key; the hint keeps the label.
    let chat_traces = traces.join("chat");
    let pose = |note: &str| {
        let r = migrate(&["--from", &seed, "--steer", note, "--requester=chat"]);
        assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
        let evs = events(&r);
        let aw = find(&evs, "awaiting").expect("awaiting event").clone();
        let key = aw["request_key"].as_str().expect("request_key").to_string();
        assert!(Path::new(aw["path"].as_str().unwrap()).starts_with(&chat_traces));
        assert!(chat_traces.join(format!("{key}.request.json")).is_file());
        assert!(aw["resume"].as_str().unwrap().contains("--requester=chat"));
        assert!(aw["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "--requester=chat"));
        (aw["attempt"].as_str().unwrap().to_string(), key)
    };
    let (first, first_key) = pose("keep the wrapping add explicit");
    let (second, second_key) = pose("name the loop bounds");
    assert_ne!(first_key, second_key);
    let rec = record(&unit_dir, &first);
    assert_eq!(rec["requester"], "chat");
    assert_eq!(rec["schema_version"], 2);
    assert!(
        pending_request(&traces).is_none(),
        "nothing chat-requested in the flat traces"
    );

    let answer_file = tmp.join("answer.txt");
    let revised = logic.replacen("\n", "\n\n", 1);
    std::fs::write(&answer_file, emission(&revised, ffi)).unwrap();
    let answer_from = |file: &Path, key: &str, extra: &[&str]| {
        let mut args = vec!["--from", &seed];
        args.extend_from_slice(extra);
        args.push("--requester=chat");
        let a = format!("--answer={}", file.display());
        let k = format!("--answer-key={key}");
        let mut all: Vec<&str> = args;
        all.push(&a);
        all.push(&k);
        migrate(&all)
    };
    let answer = |key: &str, extra: &[&str]| answer_from(&answer_file, key, extra);
    // The second steer's answer offered to the FIRST attempt: a pending
    // chat request of the same unit and model, but not the one the first
    // attempt's id was derived from — refused up front (§R CE-6), the second
    // hand-off untouched.
    let r = answer(&second_key, &["--steer", "keep the wrapping add explicit"]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(
        find(&events(&r), "error").unwrap()["kind"],
        "answer-refused"
    );
    assert!(
        r.stderr.contains("is not the request attempt"),
        "{}",
        r.stderr
    );
    assert!(!chat_traces
        .join(format!("{second_key}.response.json"))
        .exists());
    // Its own key: filed by the CLI, the SAME attempt resumes to green.
    let r = answer(&first_key, &["--steer", "keep the wrapping add explicit"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let done = find(&events(&r), "attempt").unwrap().clone();
    assert_eq!(done["id"], first.as_str());
    assert_eq!(done["outcome"], "green");
    let filed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(chat_traces.join(format!("{first_key}.response.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(filed["input_tokens"], 0);
    assert_eq!(filed["stop_reason"], "end_turn");
    // Answered once: again, refused up front (it is finished).
    let r = answer(&first_key, &["--steer", "keep the wrapping add explicit"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("--answer refused"), "{}", r.stderr);
    assert_eq!(
        find(&events(&r), "error").unwrap()["kind"],
        "answer-refused",
        "an up-front refusal is not answer-unused (§R CE-1)"
    );
    // The answer file's and key's own checks, typed the same.
    let link = tmp.join("answer-link.txt");
    std::os::unix::fs::symlink(&answer_file, &link).unwrap();
    let empty = tmp.join("empty.txt");
    std::fs::write(&empty, " \n").unwrap();
    let steer2 = ["--steer", "name the loop bounds"];
    for (file, key, why) in [
        (&link, second_key.as_str(), "symlinks are refused"),
        (&empty, second_key.as_str(), "the answer is empty"),
        (&answer_file, "ABCDEF12", "not a trace key"),
    ] {
        let r = answer_from(file, key, &steer2);
        assert_eq!(r.code, 1, "{why}");
        assert_eq!(
            find(&events(&r), "error").unwrap()["kind"],
            "answer-refused"
        );
        assert!(r.stderr.contains(why), "{why}: {}", r.stderr);
    }
    // Only an `external` run files an answer.
    let r = answer(
        &second_key,
        &["--steer", "name the loop bounds", "--provider=replay"],
    );
    assert_eq!(r.code, 1);
    assert_eq!(
        find(&events(&r), "error").unwrap()["kind"],
        "answer-refused"
    );
    assert!(r.stderr.contains("only an `external` run"), "{}", r.stderr);
    // A red answer to the second steer: judged, it poses a repair.
    let broken = tmp.join("broken.txt");
    std::fs::write(&broken, emission("fn broken( {\n", ffi)).unwrap();
    let r = answer_from(&broken, &second_key, &steer2);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    assert_eq!(find(&evs, "error").unwrap()["kind"], "awaiting");
    let repair_key = find(&evs, "awaiting").unwrap()["request_key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(repair_key, second_key);
    // No repair allowed any more: the resume finishes red without asking
    // for the repair's answer — answer-unused, AFTER the run's own events
    // (§R CE-4), nothing filed.
    let toml = tmp.join("harness.toml");
    let mut cfg = std::fs::read_to_string(&toml).unwrap();
    cfg.push_str("\n[llm.migrate]\nmax_repairs = 0\n");
    std::fs::write(&toml, cfg).unwrap();
    let r = answer(&repair_key, &steer2);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let done = find(&evs, "attempt").expect("the finished attempt's own event");
    assert_eq!(done["id"], second.as_str());
    assert_eq!(done["outcome"], "red");
    assert_eq!(find(&evs, "error").unwrap()["kind"], "answer-unused");
    assert!(!chat_traces
        .join(format!("{repair_key}.response.json"))
        .exists());
    // Without the label, clap refuses --answer before anything runs.
    let a = format!("--answer={}", answer_file.display());
    let k = format!("--answer-key={second_key}");
    let r = migrate(&["--from", &seed, "--steer", "name the loop bounds", &a, &k]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("--requester"), "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&tmp);
}
