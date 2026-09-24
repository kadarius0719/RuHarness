//! The server end to end on a zopfli copy (docs/MCP-DESIGN.md §5 "End to
//! end"), through the `external` hand-off:
//! - a pending BLIND hand-off (an unseeded attempt the CLI posed) is flagged
//!   by `harness_status` and refused by `harness_retry` and
//!   `harness_answer` — the chat never answers it;
//! - a steer attempt awaits, is answered with `harness_answer` (the server
//!   writes the response and resumes the SAME attempt) to green, recorded
//!   `steered`; progress only for a call that asked for it;
//! - a retry of it reproduces and records nothing;
//! - a promotion whose oracle spins (the whole-program check runs zopfli's
//!   `main`, made to spin): a second act is `busy`, a read is answered at
//!   once, `notifications/cancelled` interrupts it (no response, the harness
//!   dies by SIGINT, the spinning program with it);
//! - stdin EOF during the same interrupts the child and exits 0.
//!
//! The `harness` binary is the workspace build next to `harness-mcp`.

mod common;

use common::*;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

fn harness(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(harness_bin()).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The fixture reply: the M0 katajainen translation, in the emission layout.
fn reply_text() -> String {
    let logic = include_str!("../../harness-cli/tests/fixtures/katajainen_logic.rs");
    let ffi = include_str!("../../harness-cli/tests/fixtures/katajainen_ffi.rs");
    format!(
        "src/logic.rs\n```rust\n{logic}```\nsrc/ffi.rs\n```rust\n{ffi}```\nRUHARNESS_END_OF_OUTPUT\n"
    )
}

/// The harness the server spawned for call `id`, and the spinning whole
/// program under it (a response to `id` meanwhile fails the test with it).
fn spinning(c: &mut Client, id: u64, reaper: &mut Reaper) -> (u32, Vec<u32>) {
    let server = c.pid();
    let early = |c: &mut Client| {
        for m in c.drain(0) {
            assert!(m.get("id") != Some(&json!(id)), "the act ended: {m}");
        }
    };
    let harness_pid = wait_for("the spawned harness", 60, || {
        early(c);
        children_of(server)
            .into_iter()
            .find(|p| comm_of(*p).ends_with("harness"))
    });
    reaper.add(harness_pid);
    let programs = wait_for("the spinning whole program", 300, || {
        early(c);
        let d: Vec<u32> = children_of(harness_pid)
            .into_iter()
            .filter(|p| {
                let comm = comm_of(*p);
                comm.ends_with("whole_c") || comm.ends_with("whole_mixed")
            })
            .collect();
        (!d.is_empty()).then_some(d)
    });
    for p in &programs {
        reaper.add(*p);
    }
    // Still there a moment later: it spins, it did not just run.
    std::thread::sleep(std::time::Duration::from_millis(500));
    for p in &programs {
        assert!(alive(*p), "the whole program spins");
    }
    (harness_pid, programs)
}

fn attempt_of<'a>(status: &'a Value, id: &str) -> &'a Value {
    status["units"][0]["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap_or_else(|| panic!("no attempt {id} in {status}"))
}

fn error_text(r: &Value) -> String {
    structured(r)["error"]["message"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn hand_offs_busy_cancel_and_eof_end_to_end() {
    let harness_path = harness_bin();
    let tmp = TempDir::new("e2e");
    let target_dir = tmp.0.join("zopfli");
    copy_dir(&repo().join("targets/zopfli"), &target_dir);
    let target = target_dir.canonicalize().unwrap();
    let t = target.to_str().unwrap();
    let unit = "u001-katajainen";
    let unit_dir = target.join("migration/units").join(unit);
    let _ = std::fs::remove_dir_all(unit_dir.join("traces"));
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));

    // The whole program (zopfli's own `main`, which the oracle's
    // whole-program check runs as `whole_c`/`whole_mixed`) spins while a
    // flag file exists. The unit's differential driver may not touch files
    // (its shape gate), and zopfli_bin.c lies outside u001's include
    // closure: the edit, made before the scan, leaves every attempt bound.
    let flag = tmp.0.join("spin");
    let bin = target.join("src/zopfli/zopfli_bin.c");
    let text = std::fs::read_to_string(&bin).unwrap();
    let main = "int main(int argc, char* argv[]) {";
    assert_eq!(text.matches(main).count(), 1);
    let text = format!(
        "#include <unistd.h>\n{}",
        text.replace(
            main,
            &format!(
                "{main}\n  if (access(\"{}\", F_OK) == 0) {{ volatile unsigned long x = 0; \
                 for (;;) {{ x++; }} }}",
                flag.display()
            )
        )
    );
    std::fs::write(&bin, text).unwrap();
    for cmd in ["scan", "plan"] {
        let (code, out, err) = harness(&[cmd, "--target", t]);
        assert_eq!(code, 0, "{out}\n{err}");
    }

    // The base attempt: the CLI's own blind hand-off, left PENDING.
    let migrate = [
        "migrate",
        "--allow-unsandboxed",
        unit,
        "--target",
        t,
        "--no-promote",
    ];
    let (code, out, err) = harness(&migrate);
    assert_eq!(code, 1, "{out}\n{err}");
    let base: String = std::fs::read_dir(unit_dir.join("attempts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .next()
        .expect("the base attempt");

    // Declared first, dropped last: on a failure the client's SIGTERM path
    // runs before the reaper's SIGKILLs.
    let mut reaper = Reaper::default();
    let mut c = Client::start(&[
        "--target",
        t,
        "--harness",
        harness_path.to_str().unwrap(),
        "--allow-unsandboxed",
    ]);
    c.initialize();

    // The pending blind hand-off: flagged, and never the chat's to answer.
    c.call(1, "harness_status", json!({}), None);
    let (r, _) = c.response(&json!(1), 60);
    let status = structured(&r);
    assert_eq!(status["units"][0]["blind_hand_off_pending"], true);
    assert_eq!(attempt_of(&status, &base)["blind_hand_off_pending"], true);
    c.call(
        2,
        "harness_retry",
        json!({"unit": unit, "attempt": base}),
        None,
    );
    let (r, _) = c.response(&json!(2), 30);
    assert!(is_error(&r));
    assert!(error_text(&r).contains("never answer it"), "{r}");
    c.call(
        3,
        "harness_answer",
        json!({"attempt": base, "model": "claude-sonnet-5", "text": reply_text()}),
        None,
    );
    let (r, _) = c.response(&json!(3), 30);
    assert!(is_error(&r));
    assert!(error_text(&r).contains("never answered here"), "{r}");
    assert!(
        !std::fs::read_dir(unit_dir.join("traces"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .path()
                .to_string_lossy()
                .ends_with(".response.json")),
        "the server wrote no response for the blind hand-off"
    );
    // The blind protocol answers it (here: the test, as the audited batch
    // would), and the CLI finishes it.
    let request = std::fs::read_dir(unit_dir.join("traces"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".request.json"))
        .expect("the pending request");
    std::fs::write(
        request
            .to_string_lossy()
            .replace(".request.json", ".response.json"),
        json!({"text": reply_text(), "input_tokens": 0, "output_tokens": 0,
               "stop_reason": "end_turn"})
        .to_string(),
    )
    .unwrap();
    let (code, out, err) = harness(&migrate);
    assert_eq!(code, 0, "{out}\n{err}");

    // A steer attempt, no progress token: it awaits the hand-off.
    let args = json!({
        "unit": unit,
        "from": base,
        "steer": "- keep the loop bounds explicit",
        "model": "claude-opus-5-5",
    });
    c.call(4, "harness_steer", args.clone(), None);
    let (r, before) = c.response(&json!(4), 300);
    assert!(before.is_empty(), "no progress without a token: {before:?}");
    assert!(!is_error(&r), "{r}");
    let s = structured(&r);
    let aw = &s["awaiting"];
    let steered = aw["attempt"].as_str().expect("the attempt id").to_string();
    assert_ne!(steered, base);
    assert_eq!(
        aw["posed_by"],
        json!({"tool": "harness_steer", "arguments": args})
    );
    assert_eq!(aw["answer_with"]["tool"], "harness_answer");
    assert_eq!(aw["answering_model"]["text"], "claude-opus-5-5");
    let request = aw["request_path"]["text"].as_str().unwrap();
    assert!(Path::new(request).is_file(), "{request}");
    assert!(s["argv"]["text"].as_str().unwrap().contains("--no-promote"));
    // Answered by another model: refused; nothing written.
    c.call(
        5,
        "harness_answer",
        json!({"attempt": steered, "model": "someone-else", "text": reply_text()}),
        None,
    );
    let (r, _) = c.response(&json!(5), 30);
    assert!(is_error(&r));
    assert!(error_text(&r).contains("claude-opus-5-5"), "{r}");
    let response = aw["response_path"]["text"].as_str().unwrap().to_string();
    assert!(!Path::new(&response).exists());

    // Answered, with a token: the server writes the response and resumes
    // the SAME attempt to green; progress seen.
    c.call(
        6,
        "harness_answer",
        json!({"attempt": steered, "model": "claude-opus-5-5", "text": reply_text()}),
        Some("tok-6"),
    );
    let (r, before) = c.response(&json!(6), 300);
    assert!(!is_error(&r), "{r}");
    let s = structured(&r);
    assert_eq!(s["attempt"]["id"], steered.as_str());
    assert_eq!(s["attempt"]["outcome"], "green");
    assert_eq!(s["attempt"]["promoted"], false);
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
    assert_eq!(written["input_tokens"], 0);
    assert_eq!(written["stop_reason"], "end_turn");
    let progress: Vec<u64> = before
        .iter()
        .filter(|m| m["method"] == "notifications/progress")
        .map(|m| {
            assert_eq!(m["params"]["progressToken"], "tok-6");
            m["params"]["progress"].as_u64().unwrap()
        })
        .collect();
    assert!(!progress.is_empty(), "progress for a call with a token");
    assert!(progress.windows(2).all(|w| w[0] < w[1]), "{progress:?}");
    // Nothing more for it: the hand-off is answered.
    c.call(
        7,
        "harness_answer",
        json!({"attempt": steered, "model": "claude-opus-5-5", "text": reply_text()}),
        None,
    );
    let (r, before) = c.response(&json!(7), 30);
    assert!(is_error(&r));
    assert!(
        before.iter().all(|m| m.get("id").is_none()),
        "nothing after 6's response: {before:?}"
    );

    // Recorded as steered, seeded from the base.
    c.call(8, "harness_status", json!({}), None);
    let (r, _) = c.response(&json!(8), 60);
    let status = structured(&r);
    let a = attempt_of(&status, &steered);
    assert_eq!(a["authorship"], json!({"kind": "steered"}));
    assert_eq!(a["seeded_from"], base.as_str());
    assert_eq!(a["bound"], true);
    assert_eq!(a["has_candidate"], true);
    assert_eq!(status["units"][0]["blind_hand_off_pending"], false);

    // A retry in the record's run shape reproduces: nothing recorded. It
    // names the model that answered it (only that model may continue it).
    c.call(
        9,
        "harness_retry",
        json!({"unit": unit, "attempt": steered, "model": "claude-opus-5-5"}),
        None,
    );
    let (r, _) = c.response(&json!(9), 300);
    assert!(!is_error(&r), "{r}");
    let s = structured(&r);
    assert_eq!(s["recorded"], false, "{s}");
    assert!(s["argv"]["text"].as_str().unwrap().contains("--retry"));

    // A promotion whose oracle spins.
    std::fs::write(&flag, "").unwrap();
    let promote = json!({"unit": unit, "attempt": steered, "replace": true});
    c.call(10, "harness_promote", promote.clone(), Some("tok-10"));
    let (harness_pid, programs) = spinning(&mut c, 10, &mut reaper);
    // A second act is refused at once, naming the running one.
    c.call(11, "harness_steer", args.clone(), None);
    let (r, _) = c.response(&json!(11), 10);
    assert!(is_error(&r));
    let s = structured(&r);
    assert_eq!(s["error"]["kind"], "busy");
    assert_eq!(s["running"]["tool"], "harness_promote");
    // A read is answered meanwhile, showing the act in flight.
    c.call(12, "harness_status", json!({}), None);
    let (r, _) = c.response(&json!(12), 60);
    assert_eq!(structured(&r)["act_in_flight"]["tool"], "harness_promote");
    // Cancel: no response for 10; the harness and the program die.
    c.send(
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                  "params": {"requestId": 10, "reason": "test"}}),
    );
    wait_for("the harness to die", 20, || {
        (!alive(harness_pid)).then_some(())
    });
    for p in &programs {
        wait_for("the whole program to die", 20, || {
            (!alive(*p)).then_some(())
        });
    }
    c.request(json!(13), "ping", json!({}));
    let (_, before) = c.response(&json!(13), 10);
    assert!(
        before.iter().all(|m| m.get("id") != Some(&json!(10))),
        "a response for the cancelled call: {before:?}"
    );
    let log = c.stderr.lock().unwrap().clone();
    assert!(
        log.contains("harness_promote (pid")
            && log.contains("SIGINT")
            && log.contains("(cancelled: no response)"),
        "{log}"
    );
    // Died by the signal, not a clean exit: its lock holder line stays.
    let holder = std::fs::read_to_string(target.join("migration/.lock")).unwrap();
    assert!(
        holder.contains(&format!("\"pid\":{harness_pid}")),
        "the holder line: {holder:?}"
    );
    c.call(14, "harness_status", json!({}), None);
    let (r, _) = c.response(&json!(14), 60);
    assert_eq!(structured(&r)["act_in_flight"], Value::Null);

    // EOF during the same: the child is interrupted, the server exits 0.
    c.call(15, "harness_promote", promote, None);
    let (harness_pid, programs) = spinning(&mut c, 15, &mut reaper);
    c.close_stdin();
    let status = c.wait_exit(20);
    assert_eq!(status.code(), Some(0));
    wait_for("the harness to die", 20, || {
        (!alive(harness_pid)).then_some(())
    });
    for p in &programs {
        wait_for("the whole program to die", 20, || {
            (!alive(*p)).then_some(())
        });
    }
}
