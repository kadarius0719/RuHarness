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
