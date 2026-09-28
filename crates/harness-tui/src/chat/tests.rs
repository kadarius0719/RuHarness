//! The chat against the replay fake (`tests/fixtures/chat/replay.sh`)
//! playing Claude Code's recordings, and against hand-written synthetic
//! recordings for what no run showed.

use super::*;
use crate::testutil::TmpDir;
use std::path::Path;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat")
}

/// A fake `claude` in `dir` replaying `recording`, logging into `dir/log`.
pub(crate) fn fake(dir: &Path, recording: &Path, extra: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let path = dir.join("claude");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n{extra}\nexec /bin/sh '{}' '{}' '{}' \"$@\"\n",
            fixtures().join("replay.sh").display(),
            recording.display(),
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A chat whose `claude` replays `recording`.
fn chat(tmp: &TmpDir, recording: &Path, extra: &str) -> Chat {
    let claude = fake(&tmp.0, recording, extra);
    let vars = vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (OsString::from("HOME"), tmp.0.clone().into_os_string()),
    ];
    let mut c = Chat::new(
        Ok(Binaries {
            claude,
            mcp: PathBuf::from("/usr/bin/false"),
        }),
        &vars,
        None,
        tmp.0.clone(),
        Procs::default(),
    );
    c.temp = std::env::temp_dir();
    c
}

/// Pump until `done` sees what it waits for (bounded); every event seen.
fn pump_until(c: &mut Chat, mut done: impl FnMut(&Chat, &[Event]) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut all = Vec::new();
    loop {
        all.extend(c.pump(Instant::now()));
        if done(c, &all) {
            return all;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; events {all:?}; transcript {:?}",
            c.transcript.all_rows(80)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn asks(events: &[Event]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Ask {
                tool, request_id, ..
            } => Some((tool.clone(), request_id.clone())),
            _ => None,
        })
        .collect()
}

fn text(c: &mut Chat) -> String {
    c.transcript
        .all_rows(200)
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join("\n")
}

fn stdin_log(tmp: &TmpDir) -> Vec<Value> {
    std::fs::read_to_string(tmp.0.join("log/stdin"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn end(c: &mut Chat) {
    c.end(Instant::now(), Duration::ZERO, Duration::from_millis(200));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !c.ending.is_empty() {
        c.pump(Instant::now());
        assert!(Instant::now() < deadline, "the end took too long");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A full hand-off round, replayed: the reads are allowed at once, each act
/// is held for the app and answered by it; the chat's read of the request's
/// last page is noted; the conversation ends in the model's summary.
#[test]
fn a_recorded_round_is_played_through() {
    let tmp = TmpDir::new("chat-round");
    let mut c = chat(&tmp, &fixtures().join("round.jsonl"), "");
    assert_eq!(c.phase(), Phase::Idle);
    c.send(
        "Please migrate this unit.",
        Some("[cockpit context] About: the project."),
        Instant::now(),
    )
    .unwrap();
    assert!(c.turn);
    let ev = pump_until(&mut c, |_, e| !asks(e).is_empty());
    let (tool, rid) = asks(&ev)[0].clone();
    assert_eq!(tool, "harness_migrate");
    assert!(matches!(
        &ev.iter().find(|e| matches!(e, Event::Ask { .. })).unwrap(),
        Event::Ask { model: Some(m), gen: 1, .. } if m.starts_with("claude-haiku")
    ));
    assert_eq!(c.phase(), Phase::Thinking);
    assert!(!c.owes_output(), "a held act is silent by design");
    assert!(c.deny(
        &rid,
        "Ran by the cockpit after the person confirmed it; …awaiting…"
    ));
    assert!(!c.deny(&rid, "twice"), "answered once");
    let ev = pump_until(&mut c, |_, e| !asks(e).is_empty());
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Read { gen: 1, key, .. } if key == "9d99c98a")));
    assert!(c
        .keys_read
        .iter()
        .any(|(g, _, k)| *g == 1 && k == "9d99c98a"));
    let (tool, rid) = asks(&ev)[0].clone();
    assert_eq!(tool, "harness_answer");
    c.deny(&rid, "Continued by the cockpit …green…");
    pump_until(&mut c, |c, _| !c.turn);
    let t = text(&mut c);
    assert!(t.contains("› Please migrate this unit."), "{t}");
    assert!(t.contains("· read the project's status — done"), "{t}");
    assert!(
        t.contains("· read the request of a-939463cb… (page 1) — done"),
        "{t}"
    );
    assert!(t.contains("Translation passed all checks"), "{t}");
    assert!(t.contains("of plan usage"), "{t}");
    assert!(!t.contains("did not send"), "{t}");
    // What the cockpit wrote: initialize, the message with its context
    // block, the reads' allows (input unchanged), the two denials.
    let written = stdin_log(&tmp);
    assert_eq!(written[0]["request"]["subtype"], "initialize");
    assert_eq!(
        written[1]["message"]["content"][0]["text"],
        "[cockpit context] About: the project."
    );
    let behaviors: Vec<&str> = written
        .iter()
        .filter_map(|w| w["response"]["response"]["behavior"].as_str())
        .collect();
    assert_eq!(behaviors, ["allow", "deny", "allow", "deny"]);
    assert!(written.iter().all(|w| w["response"]["response"]
        .get("updatedPermissions")
        .is_none()));
    // The runtime's environment and working directory were the cockpit's.
    let cwd = std::fs::read_to_string(tmp.0.join("log/cwd")).unwrap();
    assert!(cwd.contains(runtime::DIR_PREFIX), "{cwd}");
    let env = std::fs::read_to_string(tmp.0.join("log/env")).unwrap();
    assert!(env.lines().any(|l| l == "TERM=dumb"), "{env}");
    let dir = c.live.as_ref().unwrap().dir.clone();
    end(&mut c);
    assert!(!dir.exists(), "the chat's directory is removed");
    assert_eq!(c.phase(), Phase::Idle);
}

/// Stop while an act is held: the runtime withdraws it (the app hears of
/// it), its marker reads as "stopped", never as a foreign message; a second
/// message goes to the same process.
#[test]
fn a_stop_withdraws_the_held_request() {
    let tmp = TmpDir::new("chat-stop");
    let mut c = chat(&tmp, &fixtures().join("stop.jsonl"), "");
    c.send("Please migrate this unit.", None, Instant::now())
        .unwrap();
    let ev = pump_until(&mut c, |_, e| !asks(e).is_empty());
    let (_, rid) = asks(&ev)[0].clone();
    assert!(c.stop());
    assert!(!c.stop(), "one interrupt per turn");
    let ev = pump_until(&mut c, |c, _| !c.turn);
    assert!(ev.contains(&Event::Withdrawn {
        gen: 1,
        request_id: rid.clone()
    }));
    assert!(
        !ev.iter().any(|e| matches!(e, Event::Foreign { .. })),
        "{ev:?}"
    );
    assert!(
        !c.deny(&rid, "late"),
        "a withdrawn request is never answered"
    );
    let t = text(&mut c);
    assert!(t.contains("stopped"), "{t}");
    c.send(
        "Never mind. What is the unit's status?",
        None,
        Instant::now(),
    )
    .unwrap();
    pump_until(&mut c, |c, _| !c.turn);
    assert!(text(&mut c).contains("Status of u001-katajainen"));
    assert_eq!(c.gen, 1, "the same chat");
    end(&mut c);
}

/// A decline reaches the model as the call's result.
#[test]
fn a_decline_is_answered() {
    let tmp = TmpDir::new("chat-decline");
    let mut c = chat(&tmp, &fixtures().join("decline.jsonl"), "");
    c.send("Please migrate this unit.", None, Instant::now())
        .unwrap();
    let ev = pump_until(&mut c, |_, e| !asks(e).is_empty());
    let (_, rid) = asks(&ev)[0].clone();
    c.deny(&rid, "declined by the person");
    pump_until(&mut c, |c, _| !c.turn);
    assert!(text(&mut c).contains("The person declined"));
    let written = stdin_log(&tmp);
    assert!(written
        .iter()
        .any(|w| w["response"]["response"]["message"] == "declined by the person"));
    end(&mut c);
}

/// Hand-written recordings (synthetic: shapes no run showed).
fn synthetic(tmp: &TmpDir, lines: &[Value]) -> PathBuf {
    let path = tmp.0.join("synthetic.jsonl");
    let mut text = String::new();
    // The shapes the fake expects: Python's separators.
    for l in lines {
        let dir = l["dir"].as_str().unwrap();
        let msg = serde_json::to_string(&l["msg"])
            .unwrap()
            .replace("\":", "\": ")
            .replace(",\"", ", \"");
        text.push_str(&format!(
            "{{\"dir\": \"{dir}\", \"msg\": {msg}, \"t\": 0.0}}\n"
        ));
    }
    std::fs::write(&path, text).unwrap();
    path
}

fn init_line(tools: &[&str], status: &str) -> Value {
    serde_json::json!({"dir": "out", "msg": {"type": "system", "subtype": "init",
        "model": "claude-haiku-4-5-20251001", "permissionMode": "default",
        "claude_code_version": TESTED_VERSION, "apiKeySource": "none",
        "tools": tools, "mcp_servers": [{"name": "harness", "status": status}]}})
}

fn start_lines() -> Vec<Value> {
    vec![
        serde_json::json!({"dir": "in", "msg": {"type": "control_request", "request_id": "r0",
            "request": {"subtype": "initialize", "hooks": null}}}),
        serde_json::json!({"dir": "in", "msg": {"type": "user", "uuid": "old-uuid",
            "message": {"role": "user", "content": []}}}),
        serde_json::json!({"dir": "out", "msg": {"type": "control_response",
            "response": {"subtype": "success", "request_id": "r0", "response": {}}}}),
    ]
}

/// Mutation-checked rule (§1.2): every `init` is checked — a tool the
/// cockpit does not expect, a failed server or another permission mode
/// ends the chat, naming it.
#[test]
fn an_unexpected_init_ends_the_chat() {
    for (tools, status, mode, words) in [
        (
            [COCKPIT_TOOLS, &["Bash"]].concat(),
            "connected",
            "default",
            "offers a tool the cockpit does not expect: Bash",
        ),
        (
            COCKPIT_TOOLS.to_vec(),
            "failed",
            "default",
            "the harness tools did not start: harness-mcp is failed",
        ),
        (
            COCKPIT_TOOLS.to_vec(),
            "connected",
            "bypassPermissions",
            "permission mode bypassPermissions",
        ),
        (
            COCKPIT_TOOLS[..3].to_vec(),
            "connected",
            "default",
            "the harness tools are incomplete",
        ),
    ] {
        let tmp = TmpDir::new("chat-init");
        let mut lines = start_lines();
        let mut init = init_line(&tools, status);
        init["msg"]["permissionMode"] = mode.into();
        lines.push(init);
        let rec = synthetic(&tmp, &lines);
        let mut c = chat(&tmp, &rec, "");
        c.send("hi", None, Instant::now()).unwrap();
        let ev = pump_until(&mut c, |_, e| {
            e.iter().any(|e| matches!(e, Event::Ended { .. }))
        });
        assert!(ev.contains(&Event::Ended { gen: 1 }));
        assert!(!c.alive());
        let t = text(&mut c);
        assert!(t.contains(words), "{words}: {t}");
        end(&mut c);
    }
    // A pending server passes with a subset of the tools.
    let tmp = TmpDir::new("chat-init-ok");
    let mut lines = start_lines();
    lines.push(init_line(&COCKPIT_TOOLS[..2], "pending"));
    lines.push(
        serde_json::json!({"dir": "out", "msg": {"type": "result", "subtype": "success",
        "is_error": false, "total_cost_usd": 0.01}}),
    );
    let rec = synthetic(&tmp, &lines);
    let mut c = chat(&tmp, &rec, "");
    c.send("hi", None, Instant::now()).unwrap();
    pump_until(&mut c, |c, _| !c.turn);
    assert!(c.alive());
    end(&mut c);
}

/// Mutation-checked rule (§1.1): a user message the cockpit did not send —
/// not an echo of its own, not the runtime's marker after its interrupt —
/// is said and reported; a marker with no Stop is foreign too.
#[test]
fn a_message_the_cockpit_did_not_send_is_reported() {
    let tmp = TmpDir::new("chat-foreign");
    let mut lines = start_lines();
    lines.push(init_line(COCKPIT_TOOLS, "connected"));
    lines.push(serde_json::json!({"dir": "out", "msg": {"type": "user", "isReplay": true,
        "uuid": "old-uuid", "message": {"role": "user", "content": [{"type": "text", "text": "echo"}]}}}));
    lines.push(
        serde_json::json!({"dir": "out", "msg": {"type": "user", "isReplay": true,
        "uuid": "someone-else", "message": {"role": "user", "content": "from the inbox"}}}),
    );
    lines.push(serde_json::json!({"dir": "out", "msg": {"type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": "[Request interrupted by user]"}]}}}));
    lines.push(
        serde_json::json!({"dir": "out", "msg": {"type": "result", "subtype": "success",
        "is_error": false}}),
    );
    let rec = synthetic(&tmp, &lines);
    let mut c = chat(&tmp, &rec, "");
    c.send("hi", None, Instant::now()).unwrap();
    let ev = pump_until(&mut c, |c, _| !c.turn);
    let foreign = ev
        .iter()
        .filter(|e| matches!(e, Event::Foreign { gen: 1 }))
        .count();
    assert_eq!(
        foreign, 2,
        "the inbox message and the marker without a Stop: {ev:?}"
    );
    let t = text(&mut c);
    assert!(t.contains("“from the inbox”"), "{t}");
    assert!(!t.contains("“echo”"), "{t}");
    end(&mut c);
}

/// Reads are allowed at once; anything but the cockpit's tools, from any
/// other server, is denied; another control request is answered with an
/// error — none reaches the app.
#[test]
fn only_the_harness_reads_are_allowed() {
    let tmp = TmpDir::new("chat-deny");
    let mut lines = start_lines();
    lines.push(init_line(COCKPIT_TOOLS, "connected"));
    let ask = |id: &str, tool: &str, server: &str| {
        serde_json::json!({"dir": "out", "msg": {"type": "control_request", "request_id": id,
            "request": {"subtype": "can_use_tool", "tool_name": tool,
                        "mcp_server": {"name": server}, "input": {"unit": "u"}}}})
    };
    let answer = |id: &str, behavior: &str| {
        serde_json::json!({"dir": "in", "msg": {"type": "control_response",
            "response": {"subtype": "success", "request_id": id,
                         "response": {"behavior": behavior}}}})
    };
    lines.push(ask("q1", "mcp__harness__harness_unit", "harness"));
    lines.push(answer("q1", "allow"));
    lines.push(ask("q2", "mcp__harness__harness_unit", "other"));
    lines.push(answer("q2", "deny"));
    lines.push(ask("q3", "Bash", "harness"));
    lines.push(answer("q3", "deny"));
    lines.push(ask("q4", "mcp__harness__harness_promote", "harness"));
    lines.push(answer("q4", "deny"));
    lines.push(
        serde_json::json!({"dir": "out", "msg": {"type": "control_request", "request_id": "q5",
        "request": {"subtype": "model_consent"}}}),
    );
    lines.push(
        serde_json::json!({"dir": "in", "msg": {"type": "control_response",
        "response": {"subtype": "error", "request_id": "q5"}}}),
    );
    lines.push(
        serde_json::json!({"dir": "out", "msg": {"type": "result", "subtype": "success",
        "is_error": false}}),
    );
    let rec = synthetic(&tmp, &lines);
    let mut c = chat(&tmp, &rec, "");
    c.send("hi", None, Instant::now()).unwrap();
    let ev = pump_until(&mut c, |c, _| !c.turn);
    assert!(asks(&ev).is_empty(), "{ev:?}");
    let t = text(&mut c);
    assert!(t.contains("this model needs consent"), "{t}");
    let stderr = std::fs::read_to_string(tmp.0.join("log/stdin")).unwrap();
    assert!(!stderr.is_empty());
    end(&mut c);
}

/// The watchdogs (§1.2): no `init` within 30 s; a turn that owes output
/// and is silent for 60 s — never while a request is held.
#[test]
fn the_watchdogs_speak_once() {
    let tmp = TmpDir::new("chat-quiet");
    let mut lines = start_lines();
    lines.push(init_line(COCKPIT_TOOLS, "connected"));
    let rec = synthetic(&tmp, &lines);
    let mut c = chat(&tmp, &rec, "");
    let t0 = Instant::now();
    c.send("hi", None, t0).unwrap();
    pump_until(&mut c, |c, _| c.saw_init);
    let quiet = |c: &mut Chat| text(c).matches("nothing from the model").count();
    c.pump(Instant::now() + QUIET - Duration::from_secs(2));
    assert_eq!(quiet(&mut c), 0);
    c.pump(Instant::now() + QUIET + Duration::from_secs(1));
    assert_eq!(quiet(&mut c), 1);
    c.pump(Instant::now() + QUIET * 2);
    assert_eq!(quiet(&mut c), 1, "once per quiet spell");
    // A held request: silent by design.
    c.quiet_warned = false;
    c.held.insert("x".into());
    c.pump(Instant::now() + QUIET * 3);
    assert_eq!(quiet(&mut c), 1);
    c.held.clear();
    end(&mut c);
    // No init at all.
    let tmp = TmpDir::new("chat-start");
    let rec = synthetic(&tmp, &start_lines()[..2]);
    let mut c = chat(&tmp, &rec, "");
    c.send("hi", None, Instant::now()).unwrap();
    c.pump(Instant::now() + START_WAIT - Duration::from_secs(1));
    assert!(!text(&mut c).contains("has not started"));
    c.pump(Instant::now() + START_WAIT + Duration::from_secs(1));
    assert!(text(&mut c).contains("the chat has not started"));
    end(&mut c);
}

/// A runtime that dies says why (its last stderr line); the next message
/// starts a new generation after a separator.
#[test]
fn a_chat_that_ends_by_itself_says_why_and_the_next_message_starts_anew() {
    let tmp = TmpDir::new("chat-dies");
    let rec = synthetic(&tmp, &start_lines()[..2]);
    let mut c = chat(&tmp, &rec, "echo 'Error: something broke' >&2; exit 1");
    c.send("hi", None, Instant::now()).unwrap();
    let ev = pump_until(&mut c, |_, e| {
        e.iter().any(|e| matches!(e, Event::Ended { .. }))
    });
    assert!(ev.contains(&Event::Ended { gen: 1 }));
    assert!(text(&mut c).contains("the chat ended: Error: something broke"));
    let rec2 = synthetic(&tmp, &start_lines()[..2]);
    let _ = rec2;
    c.bins = Ok(Binaries {
        claude: fake(&tmp.0, &tmp.0.join("synthetic.jsonl"), ""),
        mcp: PathBuf::from("/usr/bin/false"),
    });
    c.send("again", None, Instant::now()).unwrap();
    assert_eq!(c.gen, 2);
    assert!(text(&mut c).contains("a new chat — it does not see the conversation above"));
    end(&mut c);
}

#[test]
fn ids_models_and_pages() {
    let u = fresh_uuid();
    assert_eq!(u.len(), 36);
    assert_eq!(&u[14..15], "4");
    assert_ne!(fresh_uuid(), u);
    assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku-4-5");
    assert_eq!(short_model("claude-opus-5"), "opus-5");
    assert!(last_page(r#"{"request_key":"k","omitted":null}"#, "k"));
    assert!(!last_page(
        r#"{"request_key":"k","omitted":{"next_page":2}}"#,
        "k"
    ));
    assert!(!last_page(r#"{"request_key":"other","omitted":null}"#, "k"));
    assert!(
        !last_page(r#"{"request_key":"k"}"#, "k"),
        "omitted must be there, null"
    );
    assert!(!last_page("not json", "k"));
    assert_eq!(short_attempt("a-939463cb78bc"), "a-939463cb…");
}
