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
    let r = harness(&["verify", "u001-katajainen", "--target", target]);
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

    // 5'. red path first: introduce a behavioral change in the Rust crate
    //     (the C side is untouched, so the stale gate must NOT fire) ->
    //     exit 10, status demoted, last-green preserved.
    let lib_rs = tmp.join("migration/units/u001-katajainen/katajainen_rs/src/lib.rs");
    let rust_src = std::fs::read_to_string(&lib_rs).unwrap();
    let broken = rust_src.replace(
        "bitlengths[leaves[0].count as usize] = 1;",
        "bitlengths[leaves[0].count as usize] = 2;",
    );
    assert_ne!(rust_src, broken, "mutation site not found");
    std::fs::write(&lib_rs, &broken).unwrap();
    let r = harness(&["verify", "u001-katajainen", "--target", target]);
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
    let r = harness(&["verify", "u001-katajainen", "--target", target]);
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
