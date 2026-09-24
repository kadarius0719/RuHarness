//! CLI hardening (docs/CLI-HARDENING.md) end to end on a temp copy of the
//! vendored zopfli target: the `--json` events stream, `migrate
//! --no-promote` + `harness promote` with its refusals, promotion crash
//! recovery by evidence, the writer lock, and cancellation that kills the
//! sandboxed process group and dies BY the signal.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Every stdout line parsed as JSON (a stray human line fails the test).
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

/// The `unit` event of `harness --json state status` for `unit`.
fn unit_event(target: &str, unit: &str) -> serde_json::Value {
    let r = harness(&["--json", "state", "status", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    events(&r)
        .into_iter()
        .find(|e| e["k"] == "unit" && e["id"] == unit)
        .unwrap_or_else(|| panic!("no unit event for {unit}:\n{}", r.stdout))
}

fn comm_of(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn children_of(pid: u32) -> Vec<u32> {
    let out = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

#[test]
fn hardening_on_zopfli() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("ruharness-hardening-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&repo_root.join("targets/zopfli"), &tmp);
    let target = tmp.to_str().unwrap();
    let unit = "u001-katajainen";
    let unit_dir = tmp.join("migration/units").join(unit);
    let crate_dir = unit_dir.join("katajainen_rs");

    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let r = harness(&["plan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    // A clean release leaves an empty holder line.
    assert_eq!(
        std::fs::read_to_string(tmp.join("migration/.lock")).unwrap(),
        ""
    );

    // ---- 1. `--json state status` is NDJSON only, with the pinned shapes.
    let r = harness(&["--json", "state", "status", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let header = &evs[0];
    assert_eq!(header["k"], "header");
    assert_eq!(header["schema"], "ruharness-events");
    assert_eq!(header["schema_version"], 1);
    assert_eq!(header["command"], "state");
    assert_eq!(
        header["args"],
        serde_json::json!(["status", "--target", target])
    );
    assert!(find(&evs, "facts").is_some(), "{}", r.stdout);
    let u = evs
        .iter()
        .find(|e| e["k"] == "unit" && e["id"] == unit)
        .unwrap_or_else(|| panic!("no unit event for {unit}:\n{}", r.stdout));
    for key in [
        "status",
        "source_fresh",
        "verdict",
        "contradiction",
        "attempts",
    ] {
        assert!(u.get(key).is_some(), "unit event lacks {key}: {u}");
    }
    assert_eq!(u["verdict"]["state"], "present", "{u}");
    let last = evs.last().unwrap();
    assert_eq!(last["k"], "result");
    assert_eq!(last["exit"], 0);

    // ---- 2. migrate --no-promote through the external hand-off, in json mode.
    let traces_dir = unit_dir.join("traces");
    let _ = std::fs::remove_dir_all(&traces_dir);
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));
    let logic = include_str!("fixtures/katajainen_logic.rs");
    let ffi = include_str!("fixtures/katajainen_ffi.rs");
    let migrate = [
        "--json",
        "migrate",
        "--allow-unsandboxed",
        unit,
        "--target",
        target,
        "--no-promote",
    ];
    let r = harness(&migrate);
    assert_eq!(r.code, 1, "expected awaiting: {}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let awaiting = find(&evs, "awaiting").expect("awaiting event");
    assert!(
        awaiting["resume"]
            .as_str()
            .unwrap()
            .contains("--no-promote"),
        "{awaiting}"
    );
    assert!(
        awaiting["attempt"].as_str().unwrap().starts_with("a-"),
        "{awaiting}"
    );
    assert_eq!(find(&evs, "error").unwrap()["kind"], "awaiting");
    assert_eq!(evs.last().unwrap()["exit"], 1);
    assert!(r.stderr.contains("awaiting response"), "{}", r.stderr);
    assert!(r.stderr.contains("--no-promote"), "{}", r.stderr);

    let request = pending_request(&traces_dir).expect("translate request written");
    write_response(&request, &emission(logic, ffi));
    let r = harness(&migrate);
    assert_eq!(
        r.code, 0,
        "migrate should be green: {}\n{}",
        r.stdout, r.stderr
    );
    let evs = events(&r);
    let start = find(&evs, "turn-start").expect("turn-start");
    assert_eq!(start["index"], 1);
    assert_eq!(start["kind"], "translate");
    assert_eq!(start["unit"], unit);
    let end = find(&evs, "turn-end").expect("turn-end");
    assert_eq!(end["result"], "green");
    assert!(
        end["request_key"].is_string() && end["response_hash"].is_string(),
        "{end}"
    );
    let attempt = find(&evs, "attempt").expect("attempt event");
    assert_eq!(attempt["outcome"], "green");
    assert_eq!(attempt["promoted"], false);
    assert_eq!(attempt["promotion"], "not promoted: --no-promote");
    let id = attempt["id"].as_str().unwrap().to_string();
    // The final turn's stored verdict: one `check` per check, then the
    // `verdict` line naming attempt-verdict.json, before `attempt`.
    let pos = |k: &str| evs.iter().position(|e| e["k"] == k).unwrap();
    assert!(
        evs.iter().any(|e| e["k"] == "check" && e["unit"] == unit),
        "{}",
        r.stdout
    );
    let verdict = find(&evs, "verdict").expect("verdict event");
    assert!(
        verdict["path"]
            .as_str()
            .unwrap()
            .ends_with("attempt-verdict.json"),
        "{verdict}"
    );
    assert_eq!(verdict["green"], true);
    assert!(pos("turn-end") < pos("check") && pos("verdict") < pos("attempt"));
    assert_eq!(end["request_key"], evs_turn_key(&unit_dir, &id));
    let attempt_dir = unit_dir.join("attempts").join(&id);
    assert!(std::fs::read_to_string(attempt_dir.join("attempt.json"))
        .unwrap()
        .contains("\"promoted\": false"));
    assert!(!unit_dir.join(format!(".promote-{id}")).exists());

    // ---- 3. promote's refusals, before any write.
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        "../x",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("is not an attempt id"), "{}", r.stderr);
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        "a-000000000000",
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("has no attempt"), "{}", r.stderr);
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stderr.contains("--replace"), "{}", r.stderr);
    // Binding: a driver that changed since the record → stale, typed.
    let driver = unit_dir.join("driver.c");
    let driver_src = std::fs::read(&driver).unwrap();
    let mut touched = driver_src.clone();
    touched.extend_from_slice(b"\n/* touched */\n");
    std::fs::write(&driver, &touched).unwrap();
    let r = harness(&[
        "--json",
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
        "--replace",
    ]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("bound to superseded inputs"),
        "{}",
        r.stderr
    );
    assert!(
        r.stderr.contains("driver is not the current one"),
        "{}",
        r.stderr
    );
    assert_eq!(find(&events(&r), "error").unwrap()["kind"], "stale");
    std::fs::write(&driver, &driver_src).unwrap();
    assert!(!unit_dir.join(format!(".promote-{id}")).exists());
    assert!(!unit_dir.join(".katajainen_rs.prev").exists());

    // ---- 4. the explicit act: promote --replace verifies in place.
    let r = harness(&[
        "--json",
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
        "--replace",
    ]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let p = find(&evs, "promote").expect("promote event");
    assert_eq!(p["result"], "verified");
    assert!(evs.iter().any(|e| e["k"] == "check"), "{}", r.stdout);
    assert_eq!(find(&evs, "verdict").unwrap()["green"], true);
    assert!(std::fs::read_to_string(attempt_dir.join("attempt.json"))
        .unwrap()
        .contains("\"promoted\": true"));
    assert!(!unit_dir.join(format!(".promote-{id}")).exists());
    assert!(!unit_dir.join(".katajainen_rs.prev").exists());
    assert!(crate_dir.join("src/logic.rs").exists());
    let r = harness(&["state", "status", "--target", target]);
    assert!(r.stdout.contains("verdict=green (fresh)"), "{}", r.stdout);
    assert!(!r.stdout.contains("CONTRADICTION"), "{}", r.stdout);
    // ---- 4b. the status report's shapes: contradiction, write in flight,
    //      a stale input list — every one pinned in JSON.
    let plan_path = tmp.join("migration/plan.toml");
    let plan_src = std::fs::read_to_string(&plan_path).unwrap();
    std::fs::write(
        &plan_path,
        plan_src.replacen("status = \"verified\"", "status = \"in-progress\"", 1),
    )
    .unwrap();
    let u = unit_event(target, unit);
    assert_eq!(u["status"], "in-progress");
    assert_eq!(u["contradiction"], true, "{u}");
    assert!(u.get("write_in_flight").is_none(), "{u}");
    assert_eq!(u["verdict"]["state"], "present");
    assert_eq!(u["verdict"]["green"], true);
    assert_eq!(u["verdict"]["stale"], serde_json::json!([]));
    let r = harness(&["state", "status", "--target", target]);
    assert!(r.stdout.contains("CONTRADICTION"), "{}", r.stdout);
    // A LIVE writer holding the ledger turns the same picture into
    // "write in flight" (CONC-H1); releasing it restores the contradiction.
    {
        let held = harness_core::ledger::WriterLock::acquire(
            &harness_core::ledger::Ledger::new(&tmp),
            &format!("verify {unit}"),
        )
        .unwrap();
        let u = unit_event(target, unit);
        assert_eq!(u["write_in_flight"]["pid"], std::process::id(), "{u}");
        assert_eq!(u["write_in_flight"]["command"], format!("verify {unit}"));
        assert_eq!(u["contradiction"], false);
        let r = harness(&["state", "status", "--target", target]);
        assert!(r.stdout.contains("write in flight (pid"), "{}", r.stdout);
        assert!(!r.stdout.contains("CONTRADICTION"), "{}", r.stdout);
        drop(held);
    }
    let u = unit_event(target, unit);
    assert_eq!(u["contradiction"], true);
    assert!(u.get("write_in_flight").is_none());
    std::fs::write(&plan_path, &plan_src).unwrap();
    // A stale driver under a verified status: the stale list names it.
    std::fs::write(&driver, &touched).unwrap();
    let u = unit_event(target, unit);
    assert_eq!(u["verdict"]["stale"], serde_json::json!(["driver"]), "{u}");
    assert_eq!(u["contradiction"], true);
    assert_eq!(u["source_fresh"], true);
    std::fs::write(&driver, &driver_src).unwrap();

    // ---- 4c. a red in-place verdict rolls back: `check` events and
    //      `promote{rolled-back}`, but no `verdict` line (nothing stored);
    //      the committed verdict stays green.
    let candidate = attempt_dir.join("candidate");
    let cand_logic = candidate.join("src/logic.rs");
    let cand_src = std::fs::read(&cand_logic).unwrap();
    let mut broken = cand_src.clone();
    broken.extend_from_slice(b"\nfn __broken( {\n");
    std::fs::write(&cand_logic, &broken).unwrap();
    let record_path = attempt_dir.join("attempt.json");
    let record_src = std::fs::read_to_string(&record_path).unwrap();
    let mut record: serde_json::Value = serde_json::from_str(&record_src).unwrap();
    record["candidate_digest"] =
        serde_json::Value::String(harness_core::hash::crate_content_hash(&candidate).unwrap());
    std::fs::write(&record_path, serde_json::to_string_pretty(&record).unwrap()).unwrap();
    let r = harness(&[
        "--json",
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
        "--replace",
    ]);
    assert_eq!(r.code, 10, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    assert_eq!(find(&evs, "promote").unwrap()["result"], "rolled-back");
    assert!(
        evs.iter()
            .any(|e| e["k"] == "check" && e["passed"] == false),
        "{}",
        r.stdout
    );
    assert!(
        find(&evs, "verdict").is_none(),
        "no verdict was stored:\n{}",
        r.stdout
    );
    assert!(
        harness_core::Verdict::load(&unit_dir.join("oracle-latest.json"))
            .unwrap()
            .green
    );
    assert!(crate_dir.join("src/logic.rs").exists());
    assert!(!unit_dir.join(format!(".promote-{id}")).exists());
    assert!(!unit_dir.join(".katajainen_rs.prev").exists());
    std::fs::write(&cand_logic, &cand_src).unwrap();
    std::fs::write(&record_path, &record_src).unwrap();

    // 5. already promoted.
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
    ]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("already promoted"), "{}", r.stderr);

    // ---- 6. crash recovery by evidence.
    // (a) A marker left after the tail completed: finished, marker removed.
    let marker = unit_dir.join(format!(".promote-{id}"));
    std::fs::create_dir(&marker).unwrap();
    let r = harness(&["verify", "--allow-unsandboxed", unit, "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("finished its bookkeeping"),
        "{}",
        r.stdout
    );
    assert!(!marker.exists());
    // (b) A marker with the candidate swapped in but no verdict bound to it:
    //     the in-place verify never completed → rolled back (a FIRST
    //     promotion has no `.prev`, so the crate is gone) — and the next
    //     promote re-establishes everything.
    std::fs::create_dir(&marker).unwrap();
    std::fs::remove_file(unit_dir.join("oracle-latest.json")).unwrap();
    let r = harness(&[
        "promote",
        "--allow-unsandboxed",
        unit,
        &id,
        "--target",
        target,
        "--replace",
    ]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("recover: rolled back"), "{}", r.stdout);
    assert!(r.stdout.contains("promoted and verified"), "{}", r.stdout);
    assert!(!marker.exists());
    assert!(crate_dir.join("src/logic.rs").exists());
    let r = harness(&["state", "status", "--target", target]);
    assert!(!r.stdout.contains("CONTRADICTION"), "{}", r.stdout);

    // ---- 7. the writer lock and cancellation. `sleep` is not on the
    //      driver libc allowlist, so the design's "sleeping driver" is a
    //      spin: it lives until the 120 s oracle timeout, and the only way
    //      its group ends within the test is the harness's kill.
    std::fs::write(
        unit_dir.join("driver.c"),
        "int main(void) { volatile unsigned long x = 0; for (;;) { x++; } }\n",
    )
    .unwrap();
    let long = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["verify", "--allow-unsandboxed", unit, "--target", target])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn harness verify");
    let pid = long.id();
    // Wait until it holds the lock and the spinning C driver is its live
    // child (the build and the toolchain probes come first).
    let mut kids = Vec::new();
    for _ in 0..900 {
        let holder = std::fs::read_to_string(tmp.join("migration/.lock")).unwrap_or_default();
        kids = children_of(pid)
            .into_iter()
            .filter(|k| comm_of(*k).ends_with("drv_c"))
            .collect();
        if holder.contains(&format!("\"pid\":{pid}")) && !kids.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!kids.is_empty(), "verify never ran the spinning driver");
    let r = harness(&["--json", "scan", "--target", target]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(&format!(
            "ledger is locked by another harness command (pid {pid}, `verify {unit}`"
        )),
        "{}",
        r.stderr
    );
    let evs = events(&r);
    let err = find(&evs, "error").unwrap();
    assert_eq!(err["kind"], "locked");
    assert_eq!(err["holder"]["pid"], pid);
    // Ctrl-C: the group dies, the harness dies BY SIGINT.
    assert!(
        kids.iter().all(|k| alive(*k)),
        "driver not alive at cancellation time"
    );
    assert!(Command::new("/bin/kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    let out = long.wait_with_output().unwrap();
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(2), "{out:?}");
        assert!(out.status.code().is_none());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("SIGINT"), "{stderr}");
    for kid in kids {
        let mut gone = false;
        for _ in 0..50 {
            if !alive(kid) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(gone, "child {kid} survived the cancellation");
    }
    // The kernel released the lock with the holder's death; the stale line
    // names the dead pid until the next holder truncates it.
    assert!(std::fs::read_to_string(tmp.join("migration/.lock"))
        .unwrap()
        .contains(&format!("\"pid\":{pid}")));
    // The dead holder's line is diagnostics, not a writer at work: a real
    // contradiction is still reported as one (LOCK-READ-1). The verdict is
    // fresh again once the original driver is back.
    std::fs::write(&driver, &driver_src).unwrap();
    std::fs::write(
        &plan_path,
        plan_src.replacen("status = \"verified\"", "status = \"in-progress\"", 1),
    )
    .unwrap();
    let u = unit_event(target, unit);
    assert_eq!(u["contradiction"], true, "{u}");
    assert!(u.get("write_in_flight").is_none(), "{u}");
    std::fs::write(&plan_path, &plan_src).unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(
        std::fs::read_to_string(tmp.join("migration/.lock")).unwrap(),
        ""
    );

    // ---- 8. observe's hand-off is typed too: `awaiting` with no attempt.
    let _ = std::fs::remove_dir_all(tmp.join("migration/observer/traces"));
    let r = harness(&["detect", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let r = harness(&["--json", "observe", "--target", target]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    let evs = events(&r);
    let awaiting = find(&evs, "awaiting").expect("awaiting event");
    assert!(awaiting["attempt"].is_null(), "{awaiting}");
    assert!(awaiting["path"]
        .as_str()
        .unwrap()
        .ends_with(".response.json"));
    assert_eq!(find(&evs, "error").unwrap()["kind"], "awaiting");
    assert!(
        r.stderr.contains("supply the response file(s)"),
        "{}",
        r.stderr
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A consumer that stopped reading must not keep the harness alive: with
/// stdout a full pipe nobody drains, the very first `--json` write parks the
/// main thread holding the stdout mutex; SIGINT must still end the process
/// BY the signal (the courtesy output is bounded).
#[cfg(unix)]
#[test]
fn a_signal_ends_the_harness_even_when_stdout_is_stalled() {
    use std::io::Write;
    use std::os::unix::process::ExitStatusExt;
    let (reader, writer) = std::io::pipe().unwrap();
    let mut filler = writer.try_clone().unwrap();
    // Fill the pipe (the filler parks once it is full; the test exits later).
    std::thread::spawn(move || {
        let chunk = vec![b'x'; 1 << 16];
        for _ in 0..64 {
            if filler.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut child = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(["--json", "state", "status", "--target", "."])
        .stdout(Stdio::from(writer))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn harness");
    let pid = child.id();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the harness should be parked on its stalled stdout"
    );
    assert!(Command::new("/bin/kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    let mut status = None;
    for _ in 0..100 {
        if let Some(s) = child.try_wait().unwrap() {
            status = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let status = status.expect("the harness did not die within 10 s");
    assert_eq!(status.signal(), Some(2), "{status:?}");
    drop(reader);
}

/// `nohup`-style: an inherited SIG_IGN on SIGHUP is honoured when our output
/// is not a terminal (the handler is not installed for it), so a detached
/// run survives a hangup; SIGTERM still ends it BY the signal.
#[cfg(unix)]
#[test]
fn an_ignored_sighup_stays_ignored_when_detached() {
    use std::os::unix::process::ExitStatusExt;
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("ruharness-sighup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&repo_root.join("targets/zopfli"), &tmp);
    let target = tmp.to_str().unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "trap '' HUP; exec \"$0\" \"$@\"",
            env!("CARGO_BIN_EXE_harness"),
            "verify",
            "--allow-unsandboxed",
            "u001-katajainen",
            "--target",
            target,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn via sh");
    let pid = child.id();
    for _ in 0..600 {
        let holder = std::fs::read_to_string(tmp.join("migration/.lock")).unwrap_or_default();
        if holder.contains(&format!("\"pid\":{pid}")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(Command::new("/bin/kill")
        .args(["-HUP", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(std::time::Duration::from_millis(700));
    assert!(
        child.try_wait().unwrap().is_none(),
        "a detached run died on an ignored SIGHUP"
    );
    assert!(Command::new("/bin/kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.signal(), Some(15), "{out:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The first turn's `request_key` as the ledger journaled it.
fn evs_turn_key(unit_dir: &Path, id: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(unit_dir.join("attempts").join(id).join("attempt.json")).unwrap();
    let record: serde_json::Value = serde_json::from_str(&text).unwrap();
    record["turns"][0]["request_key"].clone()
}
