//! The run log while the cockpit waits (docs/RUN-LOG-DESIGN.md §B,
//! 2026-11-27; the person, watching the Log view go silent while the chat
//! wrote a long answer: "it would be great to give status in terms of
//! waiting for things to happen in the logs too"): the Log view's live row,
//! the sparse "still waiting" rows, the chat's answer going on, a normal
//! pause for the chat said in one plain row — and the status row's
//! heartbeat coming back after a person's own command ends.

use super::*;
use crate::app::asks::ChatTag;
use crate::app::{ChatPause, Pending, LAST_SHOWN_FOR, STILL_WAITING_EVERY};
use crate::runlog::Event as LogEvent;
use crate::spawn::ChildMsg;
use crate::testutil::TmpDir;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::{Duration, UNIX_EPOCH};

const MODEL: &str = "claude-opus-5-5";
const FILE: &str = "/Users/b/lz4-map/migration/units/u-lib/traces/chat/2c9ea42d.response.json";

/// A cockpit with a live chat (a sink), the log's clocks fixed.
fn waiting_app(tag: &str, tmp: &TmpDir) -> App {
    let mut a = app(tag);
    a.chat_on = true;
    a.chat.test_live(&tmp.0, MODEL);
    a.chat.turn = false;
    a.runlog.set_offset(Some(0));
    a.runlog
        .set_wall_clock(Some(UNIX_EPOCH + Duration::from_secs(1_762_535_553)));
    a
}

/// The chat's Migrate of `unit`, as the cockpit spawns it.
fn chat_migrate(a: &App, unit: &str, answer: Option<&str>) -> Pending {
    // The argv's shape is a Re-check's: the act and the tag make it the
    // chat's Migrate (only the chat asks for one on external).
    let mut p = a.act_argv(Act::Verify, Some(unit), None, None).unwrap();
    p.act = Act::Migrate;
    p.argv.push(OsString::from("--requester=chat"));
    p.label = format!("Migrate {unit} (asked in chat)");
    p.chat = Some(Box::new(ChatTag {
        gen: a.chat.gen,
        request_id: "r1".into(),
        tool: "harness_migrate".into(),
        model: MODEL.into(),
        grant: false,
        answer: answer.map(str::to_string),
        key: answer.map(|_| "2c9ea42d".to_string()),
        permitted: false,
        attempt: answer.map(|_| "a-da34f87c2c91".to_string()),
        epoch: 0,
    }));
    p
}

fn out(a: &mut App, line: &str) {
    a.on_child_msg(ChildMsg::Event {
        event: crate::events::parse_line(line),
        line: line.to_string(),
    });
}

fn err(a: &mut App, line: &str) {
    a.on_child_msg(ChildMsg::Stderr(line.to_string()));
}

/// The pause sequence of the person's screenshot (2026-11-27), as the
/// harness prints it: stdout's events and stderr's lines interleaved.
fn the_pause(a: &mut App) {
    let p = chat_migrate(a, "u-lib", None);
    a.on_spawned(&p);
    out(
        a,
        r#"{"k":"turn-start","unit":"u-lib","attempt":"a-da34f87c2c91","index":1,"kind":"translate","request_key":"2c9ea42d"}"#,
    );
    err(a, &format!("awaiting response: {FILE}"));
    out(
        a,
        &format!(
            r#"{{"k":"awaiting","attempt":"a-da34f87c2c91","path":"{FILE}","resume":"harness migrate u-lib --target=/Users/b/lz4-map --provider=external --model=claude-opus-5-5 --no-promote --requester=chat","request_key":"2c9ea42d"}}"#
        ),
    );
    err(
        a,
        "migrate: external provider mode — write the reply beside its request under \
         /Users/b/lz4-map/migration/units/u-lib/traces/chat as the envelope {\"text\": <the \
         reply>, \"input_tokens\": 0} (the model's reply as its \"text\"), then re-run: \
         harness migrate u-lib --target=/Users/b/lz4-map",
    );
    err(a, &format!("error: awaiting response: {FILE}"));
    out(
        a,
        &format!(r#"{{"k":"error","kind":"awaiting","message":"awaiting response: {FILE}"}}"#),
    );
    out(a, r#"{"k":"result","exit":1}"#);
    let _ = a.on_child_exit(ExitStatus::from_raw(1 << 8));
}

fn plain(rows: &[&LogEvent]) -> Vec<String> {
    rows.iter()
        .filter(|e| !e.raw_only)
        .map(|e| e.words.clone())
        .collect()
}

fn log_screen(a: &mut App, raw: bool) -> String {
    a.open_log();
    if raw {
        key(a, 'r');
    }
    text(&render(a, 160, 40))
}

/// A normal pause for the chat is one plain row in the notice tone; the
/// harness's own lines of it are under `r`, and the file keeps them all.
#[test]
fn a_pause_for_the_chat_is_one_plain_row() {
    let tmp = TmpDir::new("lw-pause");
    let mut a = waiting_app("lw-pause", &tmp);
    let before = a.runlog.serial();
    the_pause(&mut a);
    let rows: Vec<&LogEvent> = a.runlog.events.iter().skip(before).collect();
    let shown = plain(&rows);
    assert_eq!(
        shown,
        [
            "Migrate u-lib (asked in chat) — started (allowed by you)",
            "turn 1 translate → …  (a-da34f87c2c91)",
            "waiting for the chat's answer to turn 1",
        ],
        "{rows:#?}"
    );
    let exit = rows.iter().rfind(|e| !e.raw_only).unwrap();
    assert_eq!(exit.words, "waiting for the chat's answer to turn 1");
    assert_eq!(exit.tone, crate::runlog::Tone::Notice);
    assert!(exit.raw.as_deref().unwrap().starts_with("exit 1 · paused:"));
    // Every line the harness printed is still an event (and so in the
    // file): seven of them folded, none dropped.
    let folded = rows.iter().filter(|e| e.raw_only).count();
    assert_eq!(folded, 7, "{rows:#?}");
    // The file holds every line, the folded ones as they were written.
    let kept = a.runlog.events.len();
    a.runlog.close();
    let file = a
        .config
        .target
        .join("migration/build")
        .join(crate::runlog::FILE);
    let written = std::fs::read_to_string(&file).unwrap();
    let lines: Vec<&str> = written
        .lines()
        .filter(|l| !l.contains(r#""k":"header""#))
        .collect();
    assert_eq!(lines.len(), kept, "{written}");
    assert!(written.contains("result: exit 1") && written.contains("external provider mode"));
    a.runlog.reopen();
    // The plain view.
    a.log_pass();
    let screen = log_screen(&mut a, false);
    assert!(
        screen.contains("waiting for the chat's answer to turn 1"),
        "{screen}"
    );
    for gone in [
        ".response.json",
        "hint:",
        "external provider mode",
        "error: awaiting",
        "result: exit 1",
        "{\"",
    ] {
        assert!(
            !screen.contains(gone),
            "{gone:?} in the plain view:\n{screen}"
        );
    }
    // The raw view of the same events shows all of it.
    a.mode = Mode::Normal;
    let screen = log_screen(&mut a, true);
    for kept in [
        ".response.json",
        "hint: harness migrate",
        "external provider mode",
        "error: awaiting",
        "awaiting: awaiting response",
        "result: exit 1",
        "{\"k\":\"awaiting\"",
    ] {
        assert!(
            screen.contains(kept),
            "{kept:?} not in the raw view:\n{screen}"
        );
    }
}

/// Only a recognised pause folds: a red verdict's exit and an error that is
/// no pause stay in the plain view, in the warn tone.
#[test]
fn a_real_failure_still_shows_plainly() {
    let tmp = TmpDir::new("lw-fail");
    let mut a = waiting_app("lw-fail", &tmp);
    let before = a.runlog.serial();
    let p = chat_migrate(&a, "u-lib", None);
    a.on_spawned(&p);
    out(
        &mut a,
        r#"{"k":"error","kind":"locked","message":"the ledger is held by another command"}"#,
    );
    err(&mut a, "error: the ledger is held by another command");
    out(&mut a, r#"{"k":"result","exit":1}"#);
    let _ = a.on_child_exit(ExitStatus::from_raw(1 << 8));
    // A red verdict: exit 10.
    let p = chat_migrate(&a, "u-lib", None);
    a.on_spawned(&p);
    out(
        &mut a,
        r#"{"k":"verdict","unit":"u-lib","green":false,"path":"migration/units/u-lib/verdict.json"}"#,
    );
    out(&mut a, r#"{"k":"result","exit":10}"#);
    let _ = a.on_child_exit(ExitStatus::from_raw(10 << 8));
    let rows: Vec<&LogEvent> = a.runlog.events.iter().skip(before).collect();
    assert!(rows.iter().all(|e| !e.raw_only), "{rows:#?}");
    let shown = plain(&rows);
    for w in [
        "error (locked): the ledger is held by another command",
        "error: the ledger is held by another command",
        "result: exit 1",
        "result: exit 10",
    ] {
        assert!(shown.iter().any(|s| s == w), "{w:?} not shown: {shown:#?}");
    }
    let exits: Vec<&&LogEvent> = rows
        .iter()
        .filter(|e| e.raw.as_deref().is_some_and(|r| r.starts_with("exit ")))
        .collect();
    assert_eq!(exits.len(), 2);
    assert!(exits
        .iter()
        .all(|e| e.tone == crate::runlog::Tone::Warn && e.words.starts_with("failed (exit")));
    // A person's own hand-off keeps the row that names the file.
    let mut p = a.act_argv(Act::Verify, Some("u-lib"), None, None).unwrap();
    p.chat = None;
    a.on_spawned(&p);
    err(&mut a, &format!("Error: awaiting response: {FILE}"));
    let _ = a.on_child_exit(ExitStatus::from_raw(1 << 8));
    let last = a.runlog.events.back().unwrap();
    assert!(!last.raw_only && last.words.contains(FILE), "{last:#?}");
    assert!(a
        .runlog
        .events
        .iter()
        .any(|e| e.words == format!("Error: awaiting response: {FILE}") && !e.raw_only));
}

fn pause(a: &mut App, since: Instant, piece: &str) {
    a.chat_pause = Some(ChatPause {
        since,
        turn: Some(1),
        piece: Some(piece.into()),
        logged: 0,
    });
}

/// The Log view's row holding `words`, its frame taken off.
fn row_with(a: &mut App, words: &str) -> Option<String> {
    let screen = text(&render(a, 120, 30));
    screen
        .lines()
        .find(|l| l.starts_with('│') && l.contains(words))
        .map(|l| l.trim_matches('│').trim().to_string())
}

/// The live row (docs/RUN-LOG-DESIGN.md §B 2026-11-27): each state on a
/// fixed clock, the time now in the time column, nothing when idle.
#[test]
fn the_live_row_says_what_the_cockpit_waits_for() {
    let tmp = TmpDir::new("lw-now");
    let mut a = waiting_app("lw-now", &tmp);
    let t = Instant::now();
    a.now = t;
    a.open_log();
    // Idle: no row.
    assert_eq!(log_now_row(&a), None);
    // A command running: its label and its time, as the status row has them.
    let p = a.act_argv(Act::Verify, Some("u-lib"), None, None).unwrap();
    a.on_spawned(&p);
    if let Some(run) = a.run.as_mut() {
        run.started = Instant::now() - Duration::from_millis(12_400);
    }
    let (piece, words) = log_now_row(&a).unwrap();
    assert_eq!(piece.as_deref(), Some("u-lib"));
    assert_eq!(words, "running — Re-check u-lib · 12 s");
    // Drawn with the time now in the time column, after the newest event.
    let row = row_with(&mut a, "running — Re-check u-lib").unwrap();
    assert_eq!(row, "17:12:33  u-lib  running — Re-check u-lib · 12 s");
    a.running = false;
    // Paused on the chat, a reply running.
    pause(&mut a, t - Duration::from_secs(200), "u-lib");
    a.chat.turn = true;
    a.chat.reply_started = Some(t - Duration::from_secs(183));
    a.chat.heard_at(t);
    assert_eq!(
        log_now_row(&a).unwrap().1,
        "waiting for the chat to answer turn 1 — thinking 3 min 3 s · heard 0 s ago"
    );
    // The status row says the same facts.
    let status = text(&render(&mut a, 120, 30));
    assert!(
        status.contains(" Chat thinking 3 min 3 s · heard 0 s ago — answering turn 1 for u-lib"),
        "{status}"
    );
    // No reply running: the stall, with its time.
    a.chat.turn = false;
    a.chat.reply_started = None;
    a.chat.heard_at(t - Duration::from_secs(40));
    assert_eq!(
        log_now_row(&a).unwrap().1,
        "waiting — the chat's reply ended and the command has not been asked again (40 s)"
    );
    // The person: a request of the chat's under the log.
    chat_pane_request(&mut a);
    assert_eq!(log_now_row(&a).unwrap().1, "waiting for you — Esc to see");
    a.asks.requests.clear();
    // The chat ended with the turn open: the way out.
    a.chat
        .end(Instant::now(), Duration::ZERO, Duration::from_millis(100));
    assert_eq!(
        log_now_row(&a).unwrap().1,
        "waiting — the chat ended and turn 1 is still open; Ctrl-N starts a new chat"
    );
    // A hand-off that is the person's own.
    a.chat_pause = None;
    a.last_waiting = true;
    assert_eq!(log_now_row(&a).unwrap().1, "waiting for you — Esc to see");
    a.last_waiting = false;
    assert_eq!(log_now_row(&a), None);
}

/// A chat request read behind the overlay (the chat pane's tests' shape).
fn chat_pane_request(a: &mut App) {
    let pending = chat_migrate(a, "u-lib", None);
    a.asks.requests.push_back(crate::app::asks::Request {
        pending,
        shown_at: Some(Instant::now()),
        words: "Migrate u-lib — a model call, answered here in chat".into(),
        continue_asks: false,
    });
}

/// Following, the live row is the last row under the newest event;
/// scrolled up, it stays pinned at the bottom and the hint to the newest
/// is kept.
#[test]
fn the_live_row_stays_at_the_bottom_when_scrolled_up() {
    let tmp = TmpDir::new("lw-pin");
    let mut a = waiting_app("lw-pin", &tmp);
    let t = Instant::now();
    a.now = t;
    let t0 = UNIX_EPOCH + Duration::from_secs(1_762_535_000);
    for i in 0..60u64 {
        a.runlog.push_at(
            t0 + Duration::from_secs(i),
            crate::runlog::Kind::Chat,
            Some("u-lib"),
            format!("· read the request of a-da34f87c… (page {i})"),
            None,
            crate::runlog::Tone::Dim,
        );
    }
    pause(&mut a, t - Duration::from_secs(200), "u-lib");
    a.chat.turn = true;
    a.chat.reply_started = Some(t - Duration::from_secs(183));
    a.chat.heard_at(t);
    a.open_log();
    let buffer = render(&mut a, 120, 30);
    golden("log-waiting.txt", &a, &buffer);
    let screen = text(&buffer);
    let lines: Vec<&str> = screen.lines().collect();
    let live = lines
        .iter()
        .position(|l| l.contains("waiting for the chat to answer turn 1"))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(lines[live - 1].contains("(page 59)"), "{screen}");
    assert!(!screen.contains("End: newest"));
    // Scrolled up: the newest is out of sight, the row stays at the bottom.
    code(&mut a, KeyCode::PageUp);
    let screen = text(&render(&mut a, 120, 30));
    let lines: Vec<&str> = screen.lines().collect();
    let live = lines
        .iter()
        .position(|l| l.contains("waiting for the chat to answer turn 1"))
        .unwrap_or_else(|| panic!("{screen}"));
    assert_eq!(
        live,
        lines.len() - 3 - 2,
        "pinned above the footer:\n{screen}"
    );
    assert!(!screen.contains("(page 59)"), "{screen}");
    assert!(screen.contains("End: newest"), "{screen}");
    // End follows again.
    code(&mut a, KeyCode::End);
    let screen = text(&render(&mut a, 120, 30));
    assert!(screen.contains("(page 59)"), "{screen}");
}

/// One continuous wait on the chat is written into the log every five
/// minutes, never more often; the wait's end, when the chat's answer goes
/// on, is a row of its own.
#[test]
fn the_wait_is_written_every_five_minutes_and_its_end_once() {
    let tmp = TmpDir::new("lw-five");
    let mut a = waiting_app("lw-five", &tmp);
    let t0 = Instant::now();
    pause(&mut a, t0, "u-lib");
    let still = |a: &App| -> Vec<String> {
        a.runlog
            .events
            .iter()
            .filter(|e| e.words.starts_with("still waiting"))
            .map(|e| e.words.clone())
            .collect()
    };
    a.chat.heard_at(t0 + Duration::from_secs(298));
    a.log_waits(t0 + STILL_WAITING_EVERY - Duration::from_secs(1));
    assert!(still(&a).is_empty(), "none at 4 min 59 s");
    a.log_waits(t0 + STILL_WAITING_EVERY);
    assert_eq!(
        still(&a),
        ["still waiting for the chat's answer to turn 1 — 5 min, heard 2 s ago"]
    );
    a.log_waits(t0 + STILL_WAITING_EVERY + Duration::from_secs(1));
    a.log_waits(t0 + STILL_WAITING_EVERY * 2 - Duration::from_secs(1));
    assert_eq!(still(&a).len(), 1, "never more often");
    a.chat.heard_at(t0 + Duration::from_secs(600));
    a.log_waits(t0 + STILL_WAITING_EVERY * 2);
    assert_eq!(
        still(&a)[1],
        "still waiting for the chat's answer to turn 1 — 10 min, heard 0 s ago"
    );
    let row = a.runlog.events.back().unwrap();
    assert_eq!(row.piece.as_deref(), Some("u-lib"));
    assert_eq!(row.tone, crate::runlog::Tone::Notice);
    // The chat filed its answer and the command goes on.
    a.now = t0 + Duration::from_secs(14 * 60 + 42);
    let p = chat_migrate(&a, "u-lib", Some("fn f() {}"));
    a.on_spawned(&p);
    assert!(a.chat_pause.is_none());
    assert!(
        a.runlog
            .events
            .iter()
            .any(|e| e.words == "the chat answered turn 1 after 14 min 42 s"),
        "{:#?}",
        a.runlog.events
    );
    a.log_waits(t0 + STILL_WAITING_EVERY * 3);
    assert_eq!(still(&a).len(), 2, "none after the wait ends");
}

/// Seen on the person's screen (2026-11-27): the chat answered turn 1 of
/// one piece, the person ran a Re-check on another, and when it ended the
/// status row said `Ready. Last: …` for the rest of the wait. Now the
/// pause stays open, `Last:` is read for a few seconds, and the heartbeat
/// comes back; the Log view's live row says the wait meanwhile.
#[test]
fn the_heartbeat_comes_back_after_a_persons_command() {
    let tmp = TmpDir::new("lw-back");
    let mut a = waiting_app("lw-back", &tmp);
    let t0 = Instant::now();
    pause(&mut a, t0 - Duration::from_secs(60), "u-lz4frame");
    a.chat.turn = true;
    a.chat.reply_started = Some(t0 - Duration::from_secs(60));
    a.now = t0;
    a.chat.heard_at(t0);
    let status = |a: &mut App| {
        text(&render(a, 160, 30))
            .lines()
            .rev()
            .nth(2)
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        status(&mut a).starts_with(" Chat thinking 1 min 0 s"),
        "{}",
        status(&mut a)
    );
    // The person's Re-check of another piece.
    let p = a.act_argv(Act::Verify, Some("u-lib"), None, None).unwrap();
    a.on_spawned(&p);
    assert!(
        a.chat_pause.is_some(),
        "another piece leaves the pause open"
    );
    let _ = a.on_child_exit(ExitStatus::from_raw(0));
    let ended = a.last_ended.unwrap();
    a.now = ended + Duration::from_secs(1);
    a.chat.heard_at(a.now);
    let row = status(&mut a);
    assert!(row.starts_with(" Ready. Last: Re-check u-lib"), "{row}");
    // Meanwhile the live row says the wait.
    a.open_log();
    assert!(log_now_row(&a)
        .unwrap()
        .1
        .starts_with("waiting for the chat to answer turn 1 — thinking"));
    a.mode = Mode::Normal;
    // Read, then the heartbeat again.
    a.now = ended + LAST_SHOWN_FOR;
    a.chat.heard_at(a.now);
    let row = status(&mut a);
    assert!(row.starts_with(" Chat thinking "), "{row}");
    assert!(row.contains("answering turn 1 for u-lz4frame"), "{row}");
    // The person's own command on the paused piece takes it over.
    let p = a.act_argv(Act::Verify, Some("u-lib"), None, None).unwrap();
    let now = a.now;
    pause(&mut a, now, "u-lib");
    a.on_spawned(&p);
    assert!(a.chat_pause.is_none());
}
