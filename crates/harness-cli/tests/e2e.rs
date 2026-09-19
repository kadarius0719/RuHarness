//! End-to-end pipeline test against the vendored zopfli target, in a temp
//! copy so the repo's own ledger is never mutated. Pins the CLI contract
//! (docs/SCHEMAS.md): subcommands, exit codes 0/1/2/10, the staleness
//! refusal, red-verdict demotion, and green-evidence preservation.

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

/// The one `*.request.json` in `dir` that has no matching response yet.
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

/// Render the two files in the executor's emission contract.
fn emission(logic: &str, ffi: &str) -> String {
    format!(
        "src/logic.rs\n```rust\n{logic}```\nsrc/ffi.rs\n```rust\n{ffi}```\nRUHARNESS_END_OF_OUTPUT\n"
    )
}

/// Write the external provider's response file next to a request.
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

#[test]
fn full_pipeline_on_zopfli() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src_target = repo_root.join("targets/zopfli");
    let tmp = std::env::temp_dir().join(format!("ruharness-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&src_target, &tmp);
    let target = tmp.to_str().unwrap();

    // 1. scan
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "scan failed: {}\n{}", r.stdout, r.stderr);
    assert!(tmp.join("migration/facts.jsonl").exists());

    // 2. plan — reconciles against the committed plan; u001 must survive.
    let r = harness(&["plan", "--target", target]);
    assert_eq!(r.code, 0, "plan failed: {}\n{}", r.stdout, r.stderr);
    let plan_text = std::fs::read_to_string(tmp.join("migration/plan.toml")).unwrap();
    assert!(
        plan_text.contains("u001-katajainen"),
        "u001 lost:\n{plan_text}"
    );
    assert!(r.stdout.contains("execution order"), "{}", r.stdout);

    // 3. verify u001 — the full oracle, green.
    let r = harness(&[
        "verify",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 0, "verify failed: {}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("GREEN"), "{}", r.stdout);
    let verdict =
        std::fs::read_to_string(tmp.join("migration/units/u001-katajainen/oracle-latest.json"))
            .unwrap();
    assert!(verdict.contains("\"green\": true"), "{verdict}");
    assert!(tmp
        .join("migration/units/u001-katajainen/oracle-last-green.json")
        .exists());
    let plan_text = std::fs::read_to_string(tmp.join("migration/plan.toml")).unwrap();
    assert!(
        plan_text.contains("\"verified\""),
        "status not verified:\n{plan_text}"
    );

    // 4. status — everything fresh, no contradictions.
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("verdict=green (fresh)"), "{}", r.stdout);
    assert!(!r.stdout.contains("CONTRADICTION"), "{}", r.stdout);

    // 5. staleness gate: touch a unit source file -> verify refuses (exit 1)
    //    without running the oracle.
    // 4b. observer pipeline: detect fires on the real target.
    let r = harness(&["detect", "--target", target]);
    assert_eq!(r.code, 0, "detect failed: {}\n{}", r.stdout, r.stderr);
    let findings_text =
        std::fs::read_to_string(tmp.join("migration/observer/findings.jsonl")).unwrap();
    assert!(
        findings_text.contains("macro-statement-body"),
        "ZOPFLI_APPEND_DATA not flagged:\n{findings_text}"
    );
    assert!(
        findings_text.contains("function-pointer-arg"),
        "{findings_text}"
    );

    // 4c. observe with no traces (fresh-clone conditions): external mode
    //     writes requests and exits 1 awaiting responses.
    let _ = std::fs::remove_dir_all(tmp.join("migration/observer/traces"));
    let r = harness(&["observe", "--target", target]);
    assert_eq!(
        r.code, 1,
        "expected awaiting-responses: {}\n{}",
        r.stdout, r.stderr
    );
    assert!(r.stderr.contains("awaiting"), "{}", r.stderr);
    let traces: Vec<_> = std::fs::read_dir(tmp.join("migration/observer/traces"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".request.json"))
        .collect();
    assert!(!traces.is_empty(), "no request files written");
    // Request files must not leak finding messages into the trusted region
    // (spot-check: the detector message text appears in findings.jsonl only).
    let one_request = std::fs::read_to_string(traces[0].path()).unwrap();
    assert!(
        !one_request.contains("heap ownership crosses the function boundary"),
        "detector message leaked into prompt"
    );

    // 4d. review CLI: unknown finding refused; real finding recorded.
    let r = harness(&[
        "review",
        "f-0000000000000000",
        "--reinstate",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let first_id = findings_text
        .lines()
        .find_map(|l| l.split("\"id\":\"").nth(1).map(|s| s[..18].to_string()))
        .expect("a finding id");
    let r = harness(&[
        "review",
        &first_id,
        "--reinstate",
        "--note",
        "e2e",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(tmp.join("migration/observer/reviews.jsonl").exists());

    // 4e. sync-runtime writes the managed block and --check is then clean.
    let r = harness(&["sync-runtime", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let agents = std::fs::read_to_string(tmp.join("AGENTS.md")).unwrap();
    assert!(agents.contains("BEGIN RUHARNESS GENERATED"), "{agents}");
    let r = harness(&["sync-runtime", "--target", target, "--check"]);
    assert_eq!(r.code, 0, "check not clean: {}\n{}", r.stdout, r.stderr);

    // 4f. M2 refusal exit codes (docs/SCHEMAS.md CLI additions), all exit 1:
    //     --check on an out-of-date block; detect on stale facts; observe on
    //     stale findings.
    let stale_agents = agents.replace("risk", "RISK");
    std::fs::write(tmp.join("AGENTS.md"), &stale_agents).unwrap();
    let r = harness(&["sync-runtime", "--target", target, "--check"]);
    assert_eq!(
        r.code, 1,
        "check must flag drift: {}\n{}",
        r.stdout, r.stderr
    );
    std::fs::write(tmp.join("AGENTS.md"), &agents).unwrap();

    let util_h = tmp.join("src/zopfli/util.h");
    let util_src = std::fs::read_to_string(&util_h).unwrap();
    std::fs::write(&util_h, format!("{util_src}\n/* e2e touch */\n")).unwrap();
    let r = harness(&["detect", "--target", target]);
    assert_eq!(r.code, 1, "detect must refuse stale facts: {}", r.stdout);
    assert!(r.stderr.contains("harness scan"), "{}", r.stderr);
    // Rescan makes facts fresh, but findings are now bound to the old file
    // hash -> observe must refuse and point at detect.
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0);
    let r = harness(&["observe", "--target", target]);
    assert_eq!(
        r.code, 1,
        "observe must refuse stale findings: {}",
        r.stdout
    );
    assert!(r.stderr.contains("harness detect"), "{}", r.stderr);
    // Restore and re-sync so the later steps see a consistent tree.
    std::fs::write(&util_h, &util_src).unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0);
    let r = harness(&["detect", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);

    // ---- M3: executor (`harness migrate`) via the external hand-off ----
    let unit_dir = tmp.join("migration/units/u001-katajainen");
    let traces_dir = unit_dir.join("traces");
    let _ = std::fs::remove_dir_all(&traces_dir);
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));
    let logic = include_str!("fixtures/katajainen_logic.rs");
    let ffi = include_str!("fixtures/katajainen_ffi.rs");

    // M-a. first run writes a translate request and exits 1 awaiting a reply.
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1, "expected awaiting: {}\n{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("awaiting response"), "{}", r.stderr);
    let request = pending_request(&traces_dir).expect("translate request written");
    let request_text = std::fs::read_to_string(&request).unwrap();
    assert!(
        request_text.contains("c_source_"),
        "C source must be nonce-delimited"
    );
    assert!(
        !request_text.contains("not a total order"),
        "hazard message text leaked into the prompt"
    );

    // M-b. supply the known-good translation -> GREEN, but NOT promoted
    //      (the unit is already verified).
    write_response(&request, &emission(logic, ffi));
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(
        r.code, 0,
        "migrate should be green: {}\n{}",
        r.stdout, r.stderr
    );
    assert!(r.stdout.contains("GREEN"), "{}", r.stdout);
    assert!(r.stdout.contains("not promoted"), "{}", r.stdout);
    let attempts: Vec<_> = std::fs::read_dir(unit_dir.join("attempts"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(attempts.len(), 1, "exactly one attempt dir");
    let attempt_json = std::fs::read_to_string(attempts[0].path().join("attempt.json")).unwrap();
    assert!(
        attempt_json.contains("\"outcome\": \"green\""),
        "{attempt_json}"
    );
    assert!(
        attempt_json.contains("\"input_tokens\": null"),
        "external usage must be null, not 0: {attempt_json}"
    );
    assert!(attempts[0].path().join("candidate/src/logic.rs").exists());
    assert!(
        !unit_dir.join("katajainen_rs/src/logic.rs").exists(),
        "must not be promoted"
    );

    // M-c. replay verifies the trajectory and writes NOTHING to the ledger.
    let before = std::fs::read_to_string(attempts[0].path().join("attempt.json")).unwrap();
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
        "--provider",
        "replay",
    ]);
    assert_eq!(r.code, 0, "replay: {}\n{}", r.stdout, r.stderr);
    let after = std::fs::read_to_string(attempts[0].path().join("attempt.json")).unwrap();
    assert_eq!(before, after, "replay must not touch the attempts ledger");
    assert_eq!(
        std::fs::read_dir(unit_dir.join("attempts"))
            .unwrap()
            .count(),
        1
    );

    // M-d. status lists the attempt.
    let r = harness(&["state", "status", "--target", target]);
    assert!(r.stdout.contains("attempts: 1"), "{}", r.stdout);

    // M-e. promotion: with the unit no longer verified, the same (resumed)
    //      attempt promotes through the two-rename protocol and re-verifies.
    let plan_path = tmp.join("migration/plan.toml");
    let plan_text = std::fs::read_to_string(&plan_path).unwrap();
    std::fs::write(
        &plan_path,
        plan_text.replacen("status = \"verified\"", "status = \"in-progress\"", 1),
    )
    .unwrap();
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 0, "promotion run: {}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("promoted and verified"), "{}", r.stdout);
    assert!(
        unit_dir.join("katajainen_rs/src/logic.rs").exists(),
        "candidate swapped in"
    );
    assert!(
        !unit_dir.join(".katajainen_rs.prev").exists(),
        "backup must be cleaned up"
    );
    let plan_text = std::fs::read_to_string(&plan_path).unwrap();
    assert!(plan_text.contains("status = \"verified\""), "{plan_text}");
    let attempt_json = std::fs::read_to_string(attempts[0].path().join("attempt.json")).unwrap();
    assert!(
        attempt_json.contains("\"promoted\": true"),
        "{attempt_json}"
    );
    let r = harness(&["state", "status", "--target", target]);
    assert!(r.stdout.contains("verdict=green (fresh)"), "{}", r.stdout);
    assert!(!r.stdout.contains("CONTRADICTION"), "{}", r.stdout);

    // M-f. a WRONG translation (different model string => new attempt) goes
    //      red through the real oracle and produces a repair request carrying
    //      the failure class and the current candidate.
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
        "--model",
        "e2e-wrong-model",
    ]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let request = pending_request(&traces_dir).expect("second translate request");
    let wrong = logic.replace(
        "bitlengths[leaves[0].count as usize] = 1;",
        "bitlengths[leaves[0].count as usize] = 2;",
    );
    assert_ne!(wrong, logic, "mutation site not found in fixture");
    write_response(&request, &emission(&wrong, ffi));
    let r = harness(&[
        "migrate",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
        "--model",
        "e2e-wrong-model",
    ]);
    assert_eq!(
        r.code, 1,
        "repair turn must await: {}\n{}",
        r.stdout, r.stderr
    );
    let repair = pending_request(&traces_dir).expect("repair request written");
    let repair_text = std::fs::read_to_string(&repair).unwrap();
    assert!(
        repair_text.contains("[FAILURE CLASS]"),
        "no failure class in repair prompt"
    );
    assert!(
        repair_text.contains("[CURRENT RUST]"),
        "no current candidate in repair prompt"
    );
    assert_eq!(
        std::fs::read_dir(unit_dir.join("attempts"))
            .unwrap()
            .count(),
        2,
        "the wrong-model run is a distinct attempt"
    );

    // 5'. red path first: introduce a behavioral change in the Rust crate
    //     (the C side is untouched, so the stale gate must NOT fire) ->
    //     exit 10, status demoted, last-green preserved.
    // After the M3 promotion above, the unit crate uses the executor layout
    // (harness-owned lib.rs scaffold + logic.rs); mutate whichever holds the logic.
    let crate_src = tmp.join("migration/units/u001-katajainen/katajainen_rs/src");
    let lib_rs = if crate_src.join("logic.rs").exists() {
        crate_src.join("logic.rs")
    } else {
        crate_src.join("lib.rs")
    };
    let rust_src = std::fs::read_to_string(&lib_rs).unwrap();
    let broken = rust_src.replace(
        "bitlengths[leaves[0].count as usize] = 1;",
        "bitlengths[leaves[0].count as usize] = 2;",
    );
    assert_ne!(rust_src, broken, "mutation site not found");
    std::fs::write(&lib_rs, &broken).unwrap();
    let r = harness(&[
        "verify",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(
        r.code, 10,
        "expected oracle red: {}\n{}",
        r.stdout, r.stderr
    );
    assert!(r.stdout.contains("RED"), "{}", r.stdout);
    let verdict =
        std::fs::read_to_string(tmp.join("migration/units/u001-katajainen/oracle-latest.json"))
            .unwrap();
    assert!(verdict.contains("\"green\": false"), "{verdict}");
    let last_green =
        std::fs::read_to_string(tmp.join("migration/units/u001-katajainen/oracle-last-green.json"))
            .unwrap();
    assert!(
        last_green.contains("\"green\": true"),
        "green evidence lost"
    );
    let plan_text = std::fs::read_to_string(tmp.join("migration/plan.toml")).unwrap();
    assert!(
        plan_text.contains("\"in-progress\""),
        "not demoted:\n{plan_text}"
    );
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("verdict=red (fresh)"), "{}", r.stdout);

    // 6. restore the crate; staleness gate: touch a unit C source file ->
    //    verify refuses (exit 1) without running the oracle.
    std::fs::write(&lib_rs, &rust_src).unwrap();
    let kata = tmp.join("src/zopfli/katajainen.c");
    let mut c_src = std::fs::read_to_string(&kata).unwrap();
    c_src.push_str("\n/* touched by e2e */\n");
    std::fs::write(&kata, c_src).unwrap();
    let r = harness(&[
        "verify",
        "--allow-unsandboxed",
        "u001-katajainen",
        "--target",
        target,
    ]);
    assert_eq!(
        r.code, 1,
        "expected stale refusal: {}\n{}",
        r.stdout, r.stderr
    );
    assert!(r.stderr.contains("stale"), "{}", r.stderr);
    assert!(
        r.stderr.contains("harness scan"),
        "recovery advice: {}",
        r.stderr
    );

    // 7. status reports the divergence; plan (without a rescan) refuses too.
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("SOURCE-STALE"), "{}", r.stdout);
    let r = harness(&["plan", "--target", target]);
    assert_eq!(r.code, 1, "plan must refuse stale facts: {}", r.stdout);
    assert!(r.stderr.contains("harness scan"), "{}", r.stderr);

    // 8. usage error -> clap's exit 2.
    let r = harness(&["no-such-subcommand"]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);

    let _ = std::fs::remove_dir_all(&tmp);
}
