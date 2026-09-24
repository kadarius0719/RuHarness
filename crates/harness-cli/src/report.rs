//! Where the CLI's stdout goes (docs/CLI-HARDENING.md §4): human lines as
//! always, or — under the global `--json` flag — a newline-delimited stream
//! of `ruharness-events` objects, one compact JSON object per line and
//! nothing else on stdout. Human logs and errors stay on stderr in both
//! modes. Events are a courtesy for a live consumer: every value they carry
//! is the ledger's own, and the ledger stays the truth.

use serde::Serialize;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// `schema` of the events stream.
pub const EVENTS_SCHEMA_NAME: &str = "ruharness-events";
/// Its version: additive changes never bump it.
pub const EVENTS_SCHEMA_VERSION: u64 = 1;

/// Whether stdout carries events (`--json`) or human lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Human lines, as before.
    Human,
    /// NDJSON events only.
    Json,
}

static MODE: OnceLock<Mode> = OnceLock::new();
/// Serialises whole-line writes across threads (`bench --jobs`).
static STDOUT: Mutex<()> = Mutex::new(());

/// Choose the mode once, at startup. A second call is ignored.
pub fn init(mode: Mode) {
    let _ = MODE.set(mode);
}

/// The mode in force (human until [`init`]).
pub fn mode() -> Mode {
    MODE.get().copied().unwrap_or(Mode::Human)
}

/// Write one line to stdout, ignoring EPIPE (`harness … | head` must exit
/// with a documented code, not a broken-pipe panic; a consumer that went
/// away does not stop a run the ledger will record anyway).
fn write_line(line: &str) {
    let _guard = STDOUT.lock().unwrap_or_else(|e| e.into_inner());
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(line.as_bytes());
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
}

/// A human line: printed as is, or wrapped in a `message` event.
pub fn line(text: String) {
    match mode() {
        Mode::Human => write_line(&text),
        Mode::Json => event(&Message {
            k: "message",
            text: &text,
        }),
    }
}

/// An event: one compact JSON object on stdout under `--json`; nothing in
/// human mode (the command prints its own lines).
pub fn event(ev: &impl Serialize) {
    if mode() != Mode::Json {
        return;
    }
    match serde_json::to_string(ev) {
        Ok(json) => write_line(&json),
        Err(e) => eprintln!("events: cannot serialize an event: {e}"),
    }
}

#[derive(Serialize)]
struct Message<'a> {
    k: &'static str,
    text: &'a str,
}

/// The stream's first line.
#[derive(Serialize)]
pub struct Header<'a> {
    /// `header`.
    pub k: &'static str,
    /// [`EVENTS_SCHEMA_NAME`].
    pub schema: &'static str,
    /// [`EVENTS_SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The subcommand.
    pub command: &'a str,
    /// Its positional/flag arguments as given.
    pub args: &'a [String],
    /// This process.
    pub pid: u32,
    /// The harness build.
    pub harness: &'static str,
}

/// Emit the header for `command` with `args`.
pub fn header(command: &str, args: &[String]) {
    event(&Header {
        k: "header",
        schema: EVENTS_SCHEMA_NAME,
        schema_version: EVENTS_SCHEMA_VERSION,
        command,
        args,
        pid: std::process::id(),
        harness: env!("CARGO_PKG_VERSION"),
    });
}

/// The stream's last line.
#[derive(Serialize)]
pub struct ResultEvent<'a> {
    /// `result`.
    pub k: &'static str,
    /// The shell-visible exit code (128+N on signal death).
    pub exit: i32,
    /// The signal the harness dies by, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<&'a str>,
}

/// Emit the `result` line.
pub fn result(exit: i32, signal: Option<&str>) {
    event(&ResultEvent {
        k: "result",
        exit,
        signal,
    });
}

/// The machine kind of a failed command's error: from the typed
/// `harness_core::Error` variants, never from prose.
pub fn error_kind(e: &anyhow::Error) -> &'static str {
    match e.downcast_ref::<harness_core::Error>() {
        Some(harness_core::Error::Locked { .. }) => "locked",
        Some(harness_core::Error::Stale { .. }) => "stale",
        Some(harness_core::Error::Awaiting { .. }) => "awaiting",
        Some(harness_core::Error::Interrupted) => "interrupted",
        _ => "harness",
    }
}

#[derive(Serialize)]
struct ErrorEvent<'a> {
    k: &'static str,
    kind: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    holder: Option<&'a harness_core::ledger::Holder>,
}

/// Emit the `error` event for a failed command (the human text goes to
/// stderr by the caller).
pub fn error(e: &anyhow::Error) {
    let holder = match e.downcast_ref::<harness_core::Error>() {
        Some(harness_core::Error::Locked { holder }) => holder.as_ref(),
        _ => None,
    };
    event(&ErrorEvent {
        k: "error",
        kind: error_kind(e),
        message: format!("{e:#}"),
        holder,
    });
}

/// `awaiting`: the `external` hand-off wants a response file.
#[derive(Serialize)]
pub struct Awaiting<'a> {
    /// `awaiting`.
    pub k: &'static str,
    /// The attempt being resumed (`None` for triage).
    pub attempt: Option<&'a str>,
    /// The response file the next run picks up.
    pub path: String,
    /// The exact command that resumes it.
    pub resume: String,
}

/// `check`: one oracle check of a verdict.
#[derive(Serialize)]
pub struct CheckEvent<'a> {
    /// `check`.
    pub k: &'static str,
    /// The unit.
    pub unit: &'a str,
    /// Check name.
    pub name: &'a str,
    /// Passed.
    pub passed: bool,
    /// Its detail, verbatim.
    pub detail: &'a str,
}

/// `verdict`: a verdict was stored.
#[derive(Serialize)]
pub struct VerdictEvent<'a> {
    /// `verdict`.
    pub k: &'static str,
    /// The unit.
    pub unit: &'a str,
    /// Green.
    pub green: bool,
    /// Where it was stored.
    pub path: String,
}

/// Emit one `check` per oracle check of `verdict` (no `verdict` line: for
/// a verdict that is NOT stored, e.g. a rolled-back promotion's).
pub fn checks(unit: &str, verdict: &harness_core::Verdict) {
    if mode() != Mode::Json {
        return;
    }
    for c in &verdict.checks {
        event(&CheckEvent {
            k: "check",
            unit,
            name: &c.name,
            passed: c.passed,
            detail: &c.detail,
        });
    }
}

/// Emit one `check` per oracle check and the `verdict` line — only after
/// the verdict was stored at `path`.
pub fn verdict(unit: &str, verdict: &harness_core::Verdict, path: &std::path::Path) {
    if mode() != Mode::Json {
        return;
    }
    checks(unit, verdict);
    event(&VerdictEvent {
        k: "verdict",
        unit,
        green: verdict.green,
        path: path.display().to_string(),
    });
}

/// `attempt`: a run ended (or a recorded attempt was verified).
#[derive(Serialize)]
pub struct AttemptEvent<'a> {
    /// `attempt`.
    pub k: &'static str,
    /// The unit.
    pub unit: &'a str,
    /// The attempt id.
    pub id: &'a str,
    /// Its outcome.
    pub outcome: &'a str,
    /// Provider profile.
    pub provider: &'a str,
    /// Model string.
    pub model: &'a str,
    /// Whether it is promoted (after this command).
    pub promoted: bool,
    /// Why it was or was not promoted (courtesy; the ledger has the flag).
    pub promotion: &'a str,
}

/// `promote`: a promotion ended.
#[derive(Serialize)]
pub struct PromoteEvent<'a> {
    /// `promote`.
    pub k: &'static str,
    /// The unit.
    pub unit: &'a str,
    /// The attempt.
    pub attempt: &'a str,
    /// `verified` or `rolled-back`.
    pub result: &'static str,
}

/// `turn-start` / `turn-end`, from the trajectory's progress hook.
#[derive(Serialize)]
struct TurnStart<'a> {
    k: &'static str,
    unit: &'a str,
    attempt: &'a str,
    index: usize,
    kind: &'a str,
    request_key: &'a str,
}

#[derive(Serialize)]
struct TurnEnd<'a> {
    k: &'static str,
    unit: &'a str,
    attempt: &'a str,
    index: usize,
    #[serde(flatten)]
    turn: &'a harness_core::attempts::Turn,
}

/// The progress sink installed under `--json`: turn events, verbatim.
pub struct Progress;

impl harness_llm::Progress for Progress {
    fn turn_start(&self, unit: &str, attempt: &str, index: usize, kind: &str, request_key: &str) {
        event(&TurnStart {
            k: "turn-start",
            unit,
            attempt,
            index,
            kind,
            request_key,
        });
    }

    fn turn_end(
        &self,
        unit: &str,
        attempt: &str,
        index: usize,
        turn: &harness_core::attempts::Turn,
    ) {
        event(&TurnEnd {
            k: "turn-end",
            unit,
            attempt,
            index,
            turn,
        });
    }
}
