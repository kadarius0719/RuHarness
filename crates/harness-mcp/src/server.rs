//! The server loop (docs/MCP-DESIGN.md §2). A reader thread feeds stdin
//! lines into a channel; this loop answers them, runs AT MOST ONE act (one
//! [`ChildSlot`], as in the cockpit) and, while it runs, answers `ping`,
//! `tools/list` and the read tools at once, refuses a second act with
//! `busy` (nothing is queued out of sight), honours
//! `notifications/cancelled`, and sends progress for a call that asked for
//! it. stdout carries protocol messages only; diagnostics go to stderr
//! through [`log`], which never panics.

use crate::acts::{self, Collected, Posed, Refusal, SteerArgs};
use crate::fence::{self, short};
use crate::policy::{self, Config};
use crate::reads;
use crate::rpc::{self, Frame, Incoming};
use crate::tools::{self, Tool, ANSWERING_RULE, UNTRUSTED_RULE};
use harness_core::attempts;
use harness_core::ledger::Ledger;
use harness_core::TargetContext;
use harness_tui::events::Event;
use harness_tui::model::Snapshot;
use harness_tui::spawn::{self, ChildMsg, ChildSlot, Running};
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The protocol version this server speaks — always (a client that cannot
/// speak it disconnects).
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// How often the loop looks at a running act.
const POLL: Duration = Duration::from_millis(50);
/// Longest silence towards a caller that asked for progress while its act
/// runs (the client aborts a call after its idle window, 30 min by default).
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// How long a shutdown waits for an interrupted child.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(1);
/// Most hand-offs remembered for `harness_answer`.
const MAX_HAND_OFFS: usize = 16;

/// A diagnostic line on stderr. Never panics (`eprintln!` does when stderr
/// is closed — a panic on the loop would leave the child running on the
/// writer lock, §R2 PROTO-2).
pub fn log(line: &str) {
    let _ = writeln!(std::io::stderr(), "harness-mcp: {line}");
}

/// What the reader thread sends.
#[derive(Debug)]
pub enum Input {
    /// One line.
    Frame(Frame),
    /// stdin closed.
    Eof,
    /// stdin failed.
    ReadError(String),
}

/// Set once the server shuts down, and never cleared: an act is spawned
/// only under this lock while it is `false`, so none can start during or
/// after a shutdown.
pub type Gate = Arc<Mutex<bool>>;

/// Shut down: close the gate for good and interrupt the running child's
/// process group, waiting at most [`SHUTDOWN_BUDGET`] (the CLI kills its
/// sandboxed groups and dies by the signal, releasing the writer lock).
pub fn shut_down(gate: &Gate, slot: &ChildSlot) {
    *gate.lock().unwrap_or_else(PoisonError::into_inner) = true;
    let _ = spawn::interrupt_and_wait(slot, SHUTDOWN_BUDGET);
}

/// The panic path: [`shut_down`] without ever blocking — the panicking
/// thread may hold the gate or the slot (§R2 VA-2) — and without waiting.
pub fn shut_down_now(gate: &Gate, slot: &ChildSlot) {
    if let Ok(mut closed) = gate.try_lock() {
        *closed = true;
    }
    let _ = spawn::try_interrupt(slot);
}

struct InFlight {
    id: Value,
    posed: Posed,
    running: Running,
    collected: Collected,
    token: Option<Value>,
    progress: u64,
    last_progress: Instant,
    started: Instant,
    cancelled: bool,
}

/// A hand-off an act of this server posed, which `harness_answer` may
/// answer: the attempt, its response file, and the act that resumes it.
#[derive(Debug, Clone)]
struct HandOff {
    attempt: String,
    response: PathBuf,
    argv: Vec<OsString>,
    posed: Posed,
}

/// The server.
pub struct Server<W: Write> {
    cfg: Config,
    tools: Vec<Tool>,
    out: W,
    slot: ChildSlot,
    gate: Gate,
    in_flight: Option<InFlight>,
    hand_offs: Vec<HandOff>,
    heartbeat: Duration,
}

enum ActError {
    /// Arguments the schema cannot express (`-32602`).
    Params(String),
    /// A refusal (`isError`).
    Refused(Refusal),
}

impl From<Refusal> for ActError {
    fn from(r: Refusal) -> ActError {
        ActError::Refused(r)
    }
}

fn tool_result(structured: Value, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": fence::ordered_text(&structured)}],
        "structuredContent": structured,
        "isError": is_error,
    })
}

fn arg<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// A progress message from closed, harness-owned values only — never model
/// or target text (a unit id is plan text: it is not said; the caller knows
/// which unit it asked about). Anything outside its closed set is `?`.
fn progress_message(ev: &Event) -> Option<String> {
    let set = |s: &str, set: &[&str]| {
        if set.contains(&s) {
            s.to_string()
        } else {
            "?".to_string()
        }
    };
    let id = |s: &str| {
        if fence::is_attempt_id(s) {
            s.to_string()
        } else {
            "?".to_string()
        }
    };
    Some(match ev {
        Event::TurnStart {
            attempt,
            index,
            kind,
            ..
        } => format!(
            "{} turn {index} ({}) started",
            id(attempt),
            set(kind, fence::TURN_KINDS)
        ),
        Event::TurnEnd {
            attempt,
            index,
            kind,
            result,
            ..
        } => format!(
            "{} turn {index} ({}) -> {}",
            id(attempt),
            set(kind, fence::TURN_KINDS),
            set(result, fence::TURN_RESULTS)
        ),
        Event::Check { passed, .. } => {
            format!("check {}", if *passed { "passed" } else { "FAILED" })
        }
        Event::Verdict { green, .. } => {
            format!("verdict {}", if *green { "green" } else { "red" })
        }
        Event::Attempt { id: a, outcome, .. } => {
            format!("attempt {} -> {}", id(a), set(outcome, fence::OUTCOMES))
        }
        Event::Promote {
            attempt, result, ..
        } => format!(
            "promotion of {} -> {}",
            id(attempt),
            set(result, fence::PROMOTION_RESULTS)
        ),
        _ => return None,
    })
}

impl<W: Write> Server<W> {
    /// A server writing protocol messages to `out`.
    pub fn new(cfg: Config, out: W, slot: ChildSlot, gate: Gate) -> Server<W> {
        let tools = tools::tools(&cfg.providers);
        Server {
            cfg,
            tools,
            out,
            slot,
            gate,
            in_flight: None,
            hand_offs: Vec::new(),
            heartbeat: HEARTBEAT,
        }
    }

    /// Serve until stdin ends (or stdout fails); returns the exit code
    /// after the shutdown path ran.
    pub fn run(mut self, rx: Receiver<Input>) -> i32 {
        loop {
            let input = if self.in_flight.is_some() {
                match rx.recv_timeout(POLL) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => Some(Input::Eof),
                }
            } else {
                Some(rx.recv().unwrap_or(Input::Eof))
            };
            let alive = match input {
                Some(Input::Frame(frame)) => self.frame(frame).is_ok(),
                Some(Input::Eof) => false,
                Some(Input::ReadError(e)) => {
                    log(&format!("stdin: {e}"));
                    false
                }
                None => true,
            };
            if !alive || self.pump().is_err() {
                shut_down(&self.gate, &self.slot);
                return 0;
            }
        }
    }

    fn send(&mut self, message: &Value) -> std::io::Result<()> {
        rpc::send(&mut self.out, message)
    }

    /// Handle one line. `Err` only when stdout failed.
    pub fn frame(&mut self, frame: Frame) -> std::io::Result<()> {
        let line = match frame {
            Frame::TooLong => {
                return self.send(&rpc::error(
                    &Value::Null,
                    rpc::INVALID_REQUEST,
                    "message over 1 MiB",
                ))
            }
            Frame::Line(line) => line,
        };
        match rpc::classify(&line) {
            Incoming::Blank | Incoming::Response => Ok(()),
            Incoming::Invalid { code, message } => {
                self.send(&rpc::error(&Value::Null, code, message))
            }
            Incoming::Notification { method, params } => {
                if method == "notifications/cancelled" {
                    self.cancel(params.as_ref());
                }
                Ok(())
            }
            Incoming::Request { id, method, params } => self.request(id, &method, params),
        }
    }

    fn request(&mut self, id: Value, method: &str, params: Option<Value>) -> std::io::Result<()> {
        match method {
            "initialize" => {
                let result = json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {
                        "name": "harness-mcp",
                        "title": "RuHarness migration ledger",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": format!(
                        "The RuHarness migration ledger of a C→Rust migration, and the review \
                         acts that stay labelled. harness_status and harness_unit read; \
                         harness_steer, harness_answer, harness_retry and harness_promote spawn \
                         the `harness` CLI (one at a time; a second is refused `busy`). \
                         {UNTRUSTED_RULE} {ANSWERING_RULE}"
                    ),
                });
                self.send(&rpc::result(&id, result))
            }
            "ping" => self.send(&rpc::result(&id, json!({}))),
            "tools/list" => {
                let listing: Vec<Value> = self.tools.iter().map(Tool::listing).collect();
                self.send(&rpc::result(&id, json!({"tools": listing})))
            }
            "tools/call" => self.call(id, params),
            _ => self.send(&rpc::error(&id, rpc::METHOD_NOT_FOUND, "method not found")),
        }
    }

    fn call(&mut self, id: Value, params: Option<Value>) -> std::io::Result<()> {
        let params = params.unwrap_or(Value::Null);
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return self.send(&rpc::error(
                &id,
                rpc::INVALID_PARAMS,
                "tools/call needs a tool `name`",
            ));
        };
        let Some(tool) = self.tools.iter().find(|t| t.name == name) else {
            return self.send(&rpc::error(&id, rpc::INVALID_PARAMS, "unknown tool"));
        };
        let tool_name = tool.name;
        let args = match tool.validate(params.get("arguments")) {
            Ok(args) => args,
            Err(why) => return self.send(&rpc::error(&id, rpc::INVALID_PARAMS, &why)),
        };
        let token = params
            .get("_meta")
            .and_then(|m| m.get("progressToken"))
            .filter(|t| t.is_string() || t.is_i64() || t.is_u64())
            .cloned();
        match tool_name {
            "harness_status" | "harness_unit" => {
                let (structured, is_error) = match self.read(tool_name, &args) {
                    Ok(v) => (v, false),
                    Err(refusal) => (refusal.to_value(), true),
                };
                self.send(&rpc::result(&id, tool_result(structured, is_error)))
            }
            _ => self.act(id, tool_name, args, token),
        }
    }

    fn in_flight_value(&self) -> Value {
        match &self.in_flight {
            Some(f) => json!({
                "tool": f.posed.tool,
                "argv": acts::argv_value(f.running.argv()),
                "running_for_secs": f.started.elapsed().as_secs(),
                "cancelled": f.cancelled,
            }),
            None => Value::Null,
        }
    }

    fn target(&self, args: &Map<String, Value>) -> Result<PathBuf, Refusal> {
        self.cfg
            .resolve_target(arg(args, "target"))
            .map_err(|why| Refusal {
                kind: "target",
                message: why,
            })
    }

    fn preflight(target: &Path) -> Result<(), Refusal> {
        policy::preflight(target).map_err(|message| Refusal {
            kind: "unreadable",
            message,
        })
    }

    fn read(&self, tool: &str, args: &Map<String, Value>) -> Result<Value, Refusal> {
        let target = self.target(args)?;
        Self::preflight(&target)?;
        let snapshot = Snapshot::load(&target).map_err(|e| Refusal {
            kind: "unreadable",
            message: e.to_string(),
        })?;
        if tool == "harness_status" {
            let routing = match TargetContext::load(&target) {
                Ok(ctx) => reads::routing(&ctx.config, &self.cfg.providers),
                Err(e) => json!({"error": short("config", &e.to_string())}),
            };
            reads::status(
                &snapshot,
                routing,
                self.in_flight_value(),
                arg(args, "after"),
            )
            .map_err(Refusal::refused)
        } else {
            let unit = arg(args, "unit").unwrap_or_default();
            reads::unit(&snapshot, unit, arg(args, "attempt"), arg(args, "symbol"))
                .map_err(Refusal::refused)
        }
    }

    /// The argv and the act of a call — or why not. `harness_answer` writes
    /// the response file here, before its resume is spawned.
    fn prepare(
        &mut self,
        tool: &'static str,
        args: &Map<String, Value>,
    ) -> Result<(Vec<OsString>, Posed), ActError> {
        let unit = arg(args, "unit").unwrap_or_default();
        let posed = |target: PathBuf, model: Option<&str>| Posed {
            tool,
            arguments: Value::Object(args.clone()),
            unit: unit.to_string(),
            target,
            answering_model: model.map(str::to_string),
            finished_before: None,
        };
        match tool {
            "harness_steer" => {
                let provider = match arg(args, "provider") {
                    Some(p) => p,
                    None if self.cfg.providers.iter().any(|p| p == "external") => "external",
                    None => {
                        return Err(ActError::Params(
                            "`provider` is required: this server does not allow `external`, \
                             and a live provider is never chosen for you"
                                .into(),
                        ))
                    }
                };
                let model = arg(args, "model");
                match (provider == "external", model) {
                    (true, None) => {
                        return Err(ActError::Params(
                            "`model` is required with the `external` provider: the model \
                             that answers the hand-off (your own model id)"
                                .into(),
                        ))
                    }
                    (false, Some(_)) => {
                        return Err(ActError::Params(
                            "`model` is only for the `external` provider; a live provider \
                             uses the target's configured model"
                                .into(),
                        ))
                    }
                    _ => {}
                }
                if model.is_some_and(|m| !acts::valid_model(m)) {
                    return Err(ActError::Params(
                        "`model` must be a model name: [A-Za-z0-9] then [A-Za-z0-9._:/@+[]-], \
                         at most 128 bytes"
                            .into(),
                    ));
                }
                let target = self.target(args)?;
                Self::preflight(&target)?;
                let steer = SteerArgs {
                    unit,
                    from: arg(args, "from").unwrap_or_default(),
                    steer: arg(args, "steer").unwrap_or_default(),
                    provider,
                    model,
                };
                let argv = acts::steer_argv(&self.cfg, &target, &steer)?;
                Ok((argv, posed(target, model)))
            }
            "harness_retry" => {
                let target = self.target(args)?;
                let attempt = arg(args, "attempt").unwrap_or_default();
                if !harness_core::plan::is_clean_segment(unit)
                    || !harness_core::plan::is_clean_segment(attempt)
                {
                    return Err(Refusal::refused("unit and attempt must be clean ids").into());
                }
                Self::preflight(&target)?;
                let records =
                    attempts::load_unit_attempts(&Ledger::new(&target), unit).map_err(|e| {
                        Refusal {
                            kind: "unreadable",
                            message: e.to_string(),
                        }
                    })?;
                let record = records
                    .iter()
                    .find(|r| r.id == attempt)
                    .ok_or_else(|| Refusal::refused("no such attempt of this unit"))?;
                let argv = acts::retry_argv(&self.cfg, &target, unit, record)?;
                // Who continues it: an `external` attempt only by the model
                // that answered it, which must say so (§R2 TRUST-6).
                let external = record.provider == "external" || record.provider_kind == "external";
                match (external, arg(args, "model")) {
                    (true, None) => {
                        return Err(ActError::Params(
                            "`model` is required to retry an `external` attempt: the model \
                             that answered it — only that model may continue it"
                                .into(),
                        ))
                    }
                    (true, Some(m)) if m != record.model => {
                        return Err(Refusal::refused(format!(
                            "this attempt was answered by `{}`; only that model may continue it",
                            record.model
                        ))
                        .into())
                    }
                    (false, Some(_)) => {
                        return Err(ActError::Params(
                            "`model` is only for an `external` attempt; a live one uses its \
                             record's model"
                                .into(),
                        ))
                    }
                    _ => {}
                }
                // The answering model is the caller's own `model` (checked
                // equal to the record's), not ledger text (§R3 VD-7).
                let mut p = posed(target, arg(args, "model"));
                p.finished_before = Some(
                    records
                        .iter()
                        .filter(|r| r.outcome != "in-progress")
                        .map(|r| r.id.clone())
                        .collect(),
                );
                Ok((argv, p))
            }
            "harness_answer" => {
                let attempt = arg(args, "attempt").unwrap_or_default();
                let target = self.target(args)?;
                let Some(ix) = self
                    .hand_offs
                    .iter()
                    .position(|h| h.attempt == attempt && h.posed.target == target)
                else {
                    return Err(Refusal::refused(
                        "no hand-off this server posed awaits for this attempt: repeat the call \
                         that posed it (the same arguments resume the same attempt) and answer \
                         the hand-off it returns. A hand-off this server did not pose — above \
                         all a pending unseeded one, which belongs to the blind protocol — is \
                         never answered here",
                    )
                    .into());
                };
                let h = self.hand_offs[ix].clone();
                let expected = h.posed.answering_model.as_deref().unwrap_or_default();
                if arg(args, "model") != Some(expected) {
                    return Err(Refusal::refused(format!(
                        "this hand-off is answered by the model its attempt names (`{expected}`), \
                         not by another: if you are not that model, do not answer it"
                    ))
                    .into());
                }
                Self::preflight(&h.posed.target)?;
                acts::write_response(
                    &h.posed.target,
                    &h.posed.unit,
                    &h.response,
                    arg(args, "text").unwrap_or_default(),
                )?;
                self.hand_offs.remove(ix);
                Ok((h.argv, h.posed))
            }
            _ => {
                let target = self.target(args)?;
                Self::preflight(&target)?;
                let replace = args.get("replace").and_then(Value::as_bool) == Some(true);
                let attempt = arg(args, "attempt").unwrap_or_default();
                let argv = acts::promote_argv(&self.cfg, &target, unit, attempt, replace)?;
                Ok((argv, posed(target, None)))
            }
        }
    }

    fn act(
        &mut self,
        id: Value,
        tool: &'static str,
        args: Map<String, Value>,
        token: Option<Value>,
    ) -> std::io::Result<()> {
        if let Some(f) = &self.in_flight {
            let busy = json!({
                "error": {
                    "kind": "busy",
                    "message": short("refusal", "another act is running; nothing is queued — \
                                                call again when it has ended"),
                },
                "running": {"tool": f.posed.tool, "argv": acts::argv_value(f.running.argv())},
            });
            return self.send(&rpc::result(&id, tool_result(busy, true)));
        }
        let (argv, posed) = match self.prepare(tool, &args) {
            Ok(v) => v,
            Err(ActError::Params(why)) => {
                return self.send(&rpc::error(&id, rpc::INVALID_PARAMS, &why))
            }
            Err(ActError::Refused(r)) => {
                return self.send(&rpc::result(&id, tool_result(r.to_value(), true)))
            }
        };
        let spawned = {
            let guard = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
            if *guard {
                Err("the server is shutting down".to_string())
            } else {
                Running::spawn(argv.clone(), self.slot.clone()).map_err(|e| e.to_string())
            }
        };
        let running = match spawned {
            Ok(r) => r,
            Err(why) => {
                let r = Refusal {
                    kind: "spawn-failed",
                    message: format!("{}: {why}", acts::shell_line(&argv)),
                };
                return self.send(&rpc::result(&id, tool_result(r.to_value(), true)));
            }
        };
        self.in_flight = Some(InFlight {
            id,
            posed,
            running,
            collected: Collected::default(),
            token,
            progress: 0,
            last_progress: Instant::now(),
            started: Instant::now(),
            cancelled: false,
        });
        if let Some(f) = &self.in_flight {
            log(&format!(
                "{tool} started (pid {}): {}",
                f.running.pid(),
                acts::shell_line(&argv)
            ));
        }
        Ok(())
    }

    fn cancel(&mut self, params: Option<&Value>) {
        let Some(request) = params.and_then(|p| p.get("requestId")) else {
            return;
        };
        if let Some(f) = self.in_flight.as_mut() {
            if &f.id == request && !f.cancelled {
                f.cancelled = true;
                let signalled = f.running.interrupt();
                log(&format!(
                    "{} cancelled by the client; interrupting pid {} ({signalled:?})",
                    f.posed.tool,
                    f.running.pid()
                ));
            }
        }
    }

    /// Look at the running act: its events (progress), a heartbeat, and its
    /// end (the response — none for a cancelled call). `Err` only when
    /// stdout failed.
    pub fn pump(&mut self) -> std::io::Result<()> {
        let heartbeat = self.heartbeat;
        let Server {
            out,
            in_flight,
            hand_offs,
            ..
        } = self;
        let Some(f) = in_flight.as_mut() else {
            return Ok(());
        };
        for msg in f.running.drain() {
            match msg {
                ChildMsg::Event(ev) => {
                    if let (Some(token), Some(message)) = (&f.token, progress_message(&ev)) {
                        if !f.cancelled {
                            f.progress += 1;
                            f.last_progress = Instant::now();
                            rpc::send(
                                out,
                                &rpc::notification(
                                    "notifications/progress",
                                    json!({"progressToken": token, "progress": f.progress, "message": message}),
                                ),
                            )?;
                        }
                    }
                    f.collected.event(&ev);
                }
                ChildMsg::Stderr(line) => f.collected.stderr(&line),
                ChildMsg::Eof(_) => {}
            }
        }
        if let Err(e) = f.running.poll_exit() {
            log(&format!("waiting for pid {}: {e}", f.running.pid()));
        }
        let Some(status) = f.running.finished() else {
            if let Some(token) = &f.token {
                if !f.cancelled && f.last_progress.elapsed() >= heartbeat {
                    f.progress += 1;
                    f.last_progress = Instant::now();
                    let message = format!("running for {} s", f.started.elapsed().as_secs());
                    rpc::send(
                        out,
                        &rpc::notification(
                            "notifications/progress",
                            json!({"progressToken": token, "progress": f.progress, "message": message}),
                        ),
                    )?;
                }
            }
            return Ok(());
        };
        let Some(f) = in_flight.take() else {
            return Ok(());
        };
        log(&format!(
            "{} (pid {}) ended: {status}{}",
            f.posed.tool,
            f.running.pid(),
            if f.cancelled {
                " (cancelled: no response)"
            } else {
                ""
            }
        ));
        // The hand-off it now awaits, if any, is this server's to answer.
        if let Some((attempt, response)) = f.collected.awaited() {
            hand_offs.retain(|h| !(h.attempt == attempt && h.posed.target == f.posed.target));
            if hand_offs.len() == MAX_HAND_OFFS {
                hand_offs.remove(0);
            }
            hand_offs.push(HandOff {
                attempt: attempt.to_string(),
                response: PathBuf::from(response),
                argv: f.running.argv().to_vec(),
                posed: f.posed.clone(),
            });
        }
        if f.cancelled {
            return Ok(());
        }
        let (structured, is_error) =
            acts::act_result(&f.posed, f.running.argv(), &f.collected, status);
        rpc::send(out, &rpc::result(&f.id, tool_result(structured, is_error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn config(harness: PathBuf, providers: &[&str]) -> Config {
        Config {
            target: repo().join("targets/zopfli"),
            target_roots: vec![repo().join("targets/tractor/cases")],
            harness: Some(harness),
            providers: providers.iter().map(|p| p.to_string()).collect(),
            allow_unsandboxed: false,
            home: None,
        }
    }

    fn server(providers: &[&str]) -> Server<Vec<u8>> {
        Server::new(
            config(PathBuf::from("/bin/sh"), providers),
            Vec::new(),
            ChildSlot::default(),
            Gate::default(),
        )
    }

    fn messages(s: &mut Server<Vec<u8>>) -> Vec<Value> {
        let out: Vec<Value> = String::from_utf8(s.out.clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        s.out.clear();
        out
    }

    /// Send one line; the messages written in answer.
    fn say(s: &mut Server<Vec<u8>>, line: &str) -> Vec<Value> {
        s.out.clear();
        s.frame(Frame::Line(line.as_bytes().to_vec())).unwrap();
        messages(s)
    }

    fn call_line(id: u64, tool: &str, args: Value, token: Option<&str>) -> String {
        let mut params = json!({"name": tool, "arguments": args});
        if let Some(t) = token {
            params["_meta"] = json!({"progressToken": t});
        }
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params}).to_string()
    }

    fn call(s: &mut Server<Vec<u8>>, id: u64, tool: &str, args: Value) -> Value {
        let out = say(s, &call_line(id, tool, args, None));
        assert_eq!(out.len(), 1, "{out:?}");
        out[0].clone()
    }

    fn structured(r: &Value) -> &Value {
        &r["result"]["structuredContent"]
    }

    #[test]
    fn a_session_is_answered_line_by_line() {
        let mut s = server(&["external"]);
        let init = say(
            &mut s,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        );
        assert_eq!(init[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        let instructions = init[0]["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains(UNTRUSTED_RULE));
        assert!(instructions.contains(ANSWERING_RULE));
        assert!(say(
            &mut s,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .is_empty());
        let list = say(
            &mut s,
            r#"{"jsonrpc":"2.0","id":"l","method":"tools/list"}"#,
        );
        assert_eq!(list[0]["id"], "l");
        assert_eq!(list[0]["result"]["tools"].as_array().unwrap().len(), 6);
        assert_eq!(
            say(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#)[0]["result"],
            json!({})
        );
        let code = |v: &Value| v["error"]["code"].as_i64().unwrap();
        assert_eq!(
            code(&say(&mut s, r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#)[0]),
            -32601
        );
        let unknown = call(&mut s, 4, "harness_verify", json!({}));
        assert_eq!(code(&unknown), -32602);
        let bad = call(&mut s, 5, "harness_unit", json!({"unit": 1}));
        assert_eq!(code(&bad), -32602);
        let parse = &say(&mut s, "{oops")[0];
        assert_eq!((code(parse), &parse["id"]), (-32700, &Value::Null));
        let batch = &say(&mut s, r#"[{"jsonrpc":"2.0","id":6,"method":"ping"}]"#)[0];
        assert_eq!((code(batch), &batch["id"]), (-32600, &Value::Null));
        s.out.clear();
        s.frame(Frame::TooLong).unwrap();
        assert!(String::from_utf8_lossy(&s.out).contains("-32600"));
        assert!(say(
            &mut s,
            r#"{"jsonrpc":"2.0","method":"notifications/whatever"}"#
        )
        .is_empty());
        assert!(say(&mut s, r#"{"jsonrpc":"2.0","id":9,"result":{}}"#).is_empty());
    }

    #[test]
    fn the_read_tools_answer_with_both_channels_outcome_first() {
        let mut s = server(&["external"]);
        let r = call(&mut s, 1, "harness_status", json!({}));
        let result = &r["result"];
        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed, result["structuredContent"], "text = the structure");
        assert!(
            text.find("\"target\"").unwrap() < text.find("\"units\"").unwrap(),
            "the head before the lists"
        );
        let st = structured(&r);
        assert_eq!(st["units"][0]["id"]["text"], "u001-katajainen");
        assert_eq!(st["routing"]["migrate"]["class"], "external");
        assert_eq!(st["act_in_flight"], Value::Null);
        let all = s.tools.clone();
        let schema = |name: &str| all.iter().find(|t| t.name == name).unwrap().output_schema();
        tools::conforms(&schema("harness_status"), st).unwrap();

        let case =
            repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib");
        let r = call(
            &mut s,
            2,
            "harness_unit",
            json!({"unit": "u-lib", "target": case.to_str().unwrap()}),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert!(!structured(&r)["pairs"].as_array().unwrap().is_empty());
        tools::conforms(&schema("harness_unit"), structured(&r)).unwrap();
        // Refusals: a target outside every root, a unit that is not there.
        let r = call(&mut s, 3, "harness_status", json!({"target": "/"}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(structured(&r)["error"]["kind"], "target");
        let r = call(
            &mut s,
            4,
            "harness_status",
            json!({"target": repo().join("targets").to_str().unwrap()}),
        );
        assert_eq!(structured(&r)["error"]["kind"], "target");
        let r = call(&mut s, 5, "harness_unit", json!({"unit": "u-nope"}));
        assert_eq!(r["result"]["isError"], true);
        tools::conforms(&schema("harness_unit"), structured(&r)).unwrap();
    }

    /// §R2 TESTS-1: every act resolves its target through the server's
    /// policy — a target outside the roots is refused before anything is
    /// spawned, and a target inside one is spawned by its canonical path.
    #[test]
    fn every_act_applies_the_target_policy() {
        let mut s = server(&["external"]);
        let outside = repo().join("targets").to_string_lossy().into_owned();
        for (tool, args) in [
            (
                "harness_steer",
                json!({"unit": "u", "from": "a-1", "steer": "x", "model": "m", "target": outside}),
            ),
            (
                "harness_retry",
                json!({"unit": "u", "attempt": "a-1", "target": outside}),
            ),
            (
                "harness_promote",
                json!({"unit": "u", "attempt": "a-1", "target": outside}),
            ),
            (
                "harness_promote",
                json!({"unit": "u", "attempt": "a-1", "target": "/"}),
            ),
        ] {
            let r = call(&mut s, 1, tool, args.clone());
            assert_eq!(structured(&r)["error"]["kind"], "target", "{tool} {args}");
            assert!(s.in_flight.is_none());
        }
        // Inside a root, spelled oddly: the canonical path reaches the argv.
        let case =
            repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib");
        let spelled = format!("{}/../read_scalefactors_lib/.", case.to_string_lossy());
        let line = call_line(
            2,
            "harness_promote",
            json!({"unit": "u-lib", "attempt": "a-13c941dfff95", "target": spelled}),
            None,
        );
        let out = say(&mut s, &line);
        assert!(out.is_empty(), "spawned, no answer yet: {out:?}");
        let f = s.in_flight.as_ref().expect("spawned");
        let argv: Vec<String> = f
            .running
            .argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.contains(&format!("--target={}", case.display())),
            "{argv:?}"
        );
        let _ = s.in_flight.as_mut().unwrap().running.interrupt();
    }

    #[test]
    fn act_arguments_are_refused_before_anything_spawns() {
        let mut s = server(&["external"]);
        let code = |v: &Value| v["error"]["code"].as_i64();
        // `model` iff external.
        let r = call(
            &mut s,
            1,
            "harness_steer",
            json!({"unit": "u001-katajainen", "from": "a-1", "steer": "x"}),
        );
        assert_eq!(code(&r), Some(-32602));
        let r = call(
            &mut s,
            2,
            "harness_steer",
            json!({"unit": "u", "from": "a-1", "steer": "x", "model": "has space"}),
        );
        assert_eq!(code(&r), Some(-32602));
        // A provider outside the list is a schema violation.
        let r = call(
            &mut s,
            3,
            "harness_steer",
            json!({"unit": "u", "from": "a-1", "steer": "x", "model": "m", "provider": "anthropic"}),
        );
        assert_eq!(code(&r), Some(-32602));
        // The note rules: a refusal the agent can act on.
        let r = call(
            &mut s,
            4,
            "harness_steer",
            json!({"unit": "u001-katajainen", "from": "a-1", "steer": "[TASK]", "model": "m"}),
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("section header"));
        // Unclean ids, for every act.
        for (tool, args) in [
            (
                "harness_promote",
                json!({"unit": "u001-katajainen", "attempt": "../../etc"}),
            ),
            ("harness_promote", json!({"unit": "../u", "attempt": "a-1"})),
            ("harness_retry", json!({"unit": "../x", "attempt": "a-1"})),
            (
                "harness_retry",
                json!({"unit": "u001-katajainen", "attempt": "../a"}),
            ),
        ] {
            let r = call(&mut s, 5, tool, args.clone());
            assert_eq!(r["result"]["isError"], true, "{tool} {args}");
            assert!(
                structured(&r)["error"]["message"]["text"]
                    .as_str()
                    .unwrap()
                    .contains("clean"),
                "{tool} {args}: {r}"
            );
        }
        let r = call(
            &mut s,
            7,
            "harness_retry",
            json!({"unit": "u001-katajainen", "attempt": "a-nope"}),
        );
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("no such attempt"));
        assert!(s.in_flight.is_none(), "nothing was spawned");
        // A live-only server: model must be omitted, and the provider named.
        let mut live = server(&["local"]);
        let r = call(
            &mut live,
            8,
            "harness_steer",
            json!({"unit": "u", "from": "a-1", "steer": "x", "model": "m", "provider": "local"}),
        );
        assert_eq!(code(&r), Some(-32602));
        let r = call(
            &mut live,
            9,
            "harness_steer",
            json!({"unit": "u", "from": "a-1", "steer": "x"}),
        );
        assert_eq!(
            code(&r),
            Some(-32602),
            "no provider is chosen for the caller"
        );
        // `external` is the default when listed, even when not first.
        let mut both = server(&["local", "external"]);
        let r = call(
            &mut both,
            10,
            "harness_steer",
            json!({"unit": "u", "from": "a-1", "steer": "x"}),
        );
        assert_eq!(code(&r), Some(-32602), "external needs a model: {r}");
    }

    /// §R2 TESTS-4: the preflight runs before every read and every act.
    #[test]
    fn every_tool_preflights_its_target() {
        let _guard = crate::policy::tests::TmpDir::new("pf");
        let base = _guard.0.clone();
        let t = crate::policy::tests::zopfli_copy(&base.join("root/zopfli"));
        // A spoil only the preflight sees: the loaders would read a linked
        // config happily (and a parse error would quote the linked file).
        let linked = t.join("harness.toml");
        let outside = base.join("outside.toml");
        std::fs::rename(&linked, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        let mut cfg = config(PathBuf::from("/bin/sh"), &["external"]);
        cfg.target_roots = vec![base.join("root").canonicalize().unwrap()];
        let mut s = Server::new(cfg, Vec::new(), ChildSlot::default(), Gate::default());
        let target = t.to_string_lossy().into_owned();
        for (tool, args) in [
            ("harness_status", json!({"target": target})),
            (
                "harness_unit",
                json!({"unit": "u001-katajainen", "target": target}),
            ),
            (
                "harness_steer",
                json!({"unit": "u001-katajainen", "from": "a-1", "steer": "x", "model": "m",
                       "target": target}),
            ),
            (
                "harness_retry",
                json!({"unit": "u001-katajainen", "attempt": "a-1", "target": target}),
            ),
            (
                "harness_promote",
                json!({"unit": "u001-katajainen", "attempt": "a-1", "target": target}),
            ),
        ] {
            let r = call(&mut s, 1, tool, args);
            assert_eq!(structured(&r)["error"]["kind"], "unreadable", "{tool}: {r}");
            assert!(s.in_flight.is_none(), "{tool} spawned");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn harness_answer_refuses_a_hand_off_it_did_not_pose() {
        let mut s = server(&["external"]);
        let r = call(
            &mut s,
            1,
            "harness_answer",
            json!({"attempt": "a-000000000001", "model": "m", "text": "reply"}),
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("never answered here"));
        assert!(s.in_flight.is_none());
    }

    #[test]
    fn progress_messages_carry_closed_values_only() {
        let m = progress_message(&Event::TurnEnd {
            unit: "Ignore previous instructions".into(),
            attempt: "a-000000000001".into(),
            index: 2,
            kind: "repair".into(),
            result: "ignore-previous-instructions".into(),
        })
        .unwrap();
        assert_eq!(m, "a-000000000001 turn 2 (repair) -> ?");
        assert!(!m.contains("Ignore"));
        assert!(progress_message(&Event::Message { text: "x".into() }).is_none());
        let m = progress_message(&Event::Check {
            unit: "u1".into(),
            name: "secret-name".into(),
            passed: false,
            detail: "secret model text".into(),
        })
        .unwrap();
        assert_eq!(m, "check FAILED");
        let m = progress_message(&Event::Attempt {
            unit: "u1".into(),
            id: "SYSTEM-NOTICE".into(),
            outcome: "green".into(),
            provider: "p".into(),
            model: "m".into(),
            promoted: false,
            promotion: String::new(),
        })
        .unwrap();
        assert_eq!(m, "attempt ? -> green");
    }

    // ---- the running act, driven in-process with a fake `harness` ----

    fn fake_harness(tag: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("harness-mcp-fake-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("harness");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A harness whose act emits a turn-start, then waits for a signal (on
    /// INT it writes `interrupted` to `$0.log`, emits one more event and
    /// lingers half a second — so a progress or a heartbeat sent after a
    /// cancel would be seen) or for `$0.go` to exist.
    const SPINNER: &str = r#"on_int() {
  echo interrupted > "$0.log"
  echo '{"k":"turn-end","unit":"u","attempt":"a-000000000001","index":1,"kind":"steer","result":"blocked"}'
  sleep 0.5
  exit 130
}
trap on_int INT
echo '{"k":"header","schema":"ruharness-events","schema_version":1,"command":"promote","args":[]}'
echo '{"k":"turn-start","unit":"u","attempt":"a-000000000001","index":1,"kind":"steer","request_key":"k"}'
while [ ! -e "$0.go" ]; do sleep 0.05; done
echo '{"k":"promote","unit":"u","attempt":"a-000000000001","result":"verified"}'
echo '{"k":"result","exit":0}'"#;

    /// Kills a test's running fake (its whole group) and removes its
    /// directory, whatever happens — a failed test never leaves a spinner
    /// behind (§R2 VC-3).
    struct Cleanup {
        slot: ChildSlot,
        dir: PathBuf,
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let mut guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(child) = guard.as_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = std::process::Command::new("/bin/kill")
                        .args(["-KILL", "--", &format!("-{}", child.id())])
                        .status();
                    let _ = child.wait();
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn spinning_server(tag: &str) -> (Server<Vec<u8>>, PathBuf, Cleanup) {
        let fake = fake_harness(tag, SPINNER);
        let s = Server::new(
            config(fake.clone(), &["external"]),
            Vec::new(),
            ChildSlot::default(),
            Gate::default(),
        );
        let cleanup = Cleanup {
            slot: s.slot.clone(),
            dir: fake.parent().unwrap().to_path_buf(),
        };
        (s, fake, cleanup)
    }

    fn promote(id: u64, token: Option<&str>) -> String {
        call_line(
            id,
            "harness_promote",
            json!({"unit": "u001-katajainen", "attempt": "a-000000000001"}),
            token,
        )
    }

    /// Pump until `done` (or 10 s); every message written meanwhile.
    fn pump_until(s: &mut Server<Vec<u8>>, mut done: impl FnMut(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        loop {
            s.pump().unwrap();
            seen.extend(messages(s));
            if done(&seen) {
                return seen;
            }
            assert!(Instant::now() < deadline, "timed out; saw {seen:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn response_to(msgs: &[Value], id: u64) -> Option<&Value> {
        msgs.iter()
            .find(|m| m.get("id") == Some(&json!(id)) && m.get("method").is_none())
    }

    #[test]
    fn one_act_at_a_time_and_nothing_queued() {
        let (mut s, fake, _cleanup) = spinning_server("busy");
        assert!(say(&mut s, &promote(1, None)).is_empty(), "the act runs");
        // A second act: refused at once, naming the running one.
        let busy = say(&mut s, &promote(2, None));
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0]["result"]["isError"], true);
        assert_eq!(structured(&busy[0])["error"]["kind"], "busy");
        assert_eq!(structured(&busy[0])["running"]["tool"], "harness_promote");
        let schema = s
            .tools
            .iter()
            .find(|t| t.name == "harness_promote")
            .unwrap()
            .output_schema();
        tools::conforms(&schema, structured(&busy[0])).unwrap();
        // A read meanwhile: answered at once, the act in flight shown.
        let r = call(&mut s, 3, "harness_status", json!({}));
        assert_eq!(structured(&r)["act_in_flight"]["tool"], "harness_promote");
        assert!(structured(&r)["act_in_flight"]["argv"]["untrusted"].is_string());
        assert!(structured(&busy[0])["running"]["argv"]["untrusted"].is_string());
        // The act ends: exactly one response, to 1 — nothing ran for 2.
        std::fs::write(format!("{}.go", fake.display()), "").unwrap();
        let seen = pump_until(&mut s, |m| response_to(m, 1).is_some());
        let r = response_to(&seen, 1).unwrap();
        assert_eq!(structured(r)["promote"]["result"], "verified");
        tools::conforms(&schema, structured(r)).unwrap();
        assert!(response_to(&seen, 2).is_none());
        assert!(s.in_flight.is_none());
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }

    #[test]
    fn a_cancel_interrupts_its_own_act_only_and_gets_no_response() {
        let (mut s, fake, _cleanup) = spinning_server("cancel");
        // A heartbeat due while the cancelled act winds down must not go.
        s.heartbeat = Duration::from_millis(100);
        assert!(say(&mut s, &promote(1, Some("t1"))).is_empty());
        // The fake's own turn-start (printed after its trap) — not a
        // heartbeat, which can come first.
        let seen = pump_until(&mut s, |m| {
            m.iter().any(|m| {
                m["params"]["message"]
                    .as_str()
                    .is_some_and(|t| t.contains("turn 1"))
            })
        });
        assert_eq!(seen[0]["params"]["progressToken"], "t1");
        // Cancelling another id (a finished read) is ignored.
        assert!(say(
            &mut s,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":99}}"#
        )
        .is_empty());
        std::thread::sleep(Duration::from_millis(300));
        s.pump().unwrap();
        assert!(s.in_flight.is_some(), "still running");
        assert!(!std::path::Path::new(&format!("{}.log", fake.display())).exists());
        // Cancelling it: interrupted, and no response — nor any progress.
        assert!(say(
            &mut s,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#
        )
        .is_empty());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut after = Vec::new();
        while s.in_flight.is_some() {
            s.pump().unwrap();
            after.extend(messages(&mut s));
            assert!(Instant::now() < deadline, "the act never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            after.is_empty(),
            "nothing after a cancel, not even a heartbeat: {after:?}"
        );
        let log = std::fs::read_to_string(format!("{}.log", fake.display())).unwrap();
        assert_eq!(log.trim(), "interrupted");
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }

    #[test]
    fn progress_needs_a_token_heartbeats_and_stops_at_the_response() {
        let (mut s, fake, _cleanup) = spinning_server("progress");
        s.heartbeat = Duration::from_millis(200);
        // No token: no progress, not even a heartbeat.
        assert!(say(&mut s, &promote(1, None)).is_empty());
        std::thread::sleep(Duration::from_millis(600));
        s.pump().unwrap();
        assert!(messages(&mut s).is_empty(), "no progress without a token");
        std::fs::write(format!("{}.go", fake.display()), "").unwrap();
        let seen = pump_until(&mut s, |m| response_to(m, 1).is_some());
        assert!(seen.iter().all(|m| m.get("method").is_none()), "{seen:?}");
        std::fs::remove_file(format!("{}.go", fake.display())).unwrap();
        // A token: event progress, then heartbeats while it spins,
        // strictly increasing; nothing after the response.
        assert!(say(&mut s, &promote(2, Some("t2"))).is_empty());
        let seen = pump_until(&mut s, |m| {
            m.iter()
                .filter(|m| {
                    m["params"]["message"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("running for"))
                })
                .count()
                >= 2
        });
        std::fs::write(format!("{}.go", fake.display()), "").unwrap();
        let mut seen = seen;
        seen.extend(pump_until(&mut s, |m| response_to(m, 2).is_some()));
        let at = seen
            .iter()
            .position(|m| m.get("id") == Some(&json!(2)))
            .unwrap();
        let progress: Vec<u64> = seen[..at]
            .iter()
            .map(|m| {
                assert_eq!(m["method"], "notifications/progress");
                assert_eq!(m["params"]["progressToken"], "t2");
                m["params"]["progress"].as_u64().unwrap()
            })
            .collect();
        assert!(progress.len() >= 3, "{progress:?}");
        assert!(progress.windows(2).all(|w| w[0] < w[1]), "{progress:?}");
        std::thread::sleep(Duration::from_millis(400));
        s.pump().unwrap();
        assert!(messages(&mut s).is_empty(), "nothing after the response");
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }

    /// `harness_answer` end to end with a fake harness: the act awaits, the
    /// server remembers the hand-off it posed; another model is refused and
    /// nothing is written; the named model's answer is written (counts 0)
    /// and the SAME argv resumes it; the hand-off is then forgotten.
    #[test]
    fn a_posed_hand_off_is_answered_by_its_model_and_resumed() {
        let _guard = crate::policy::tests::TmpDir::new("ans");
        let base = _guard.0.clone();
        let t = crate::policy::tests::zopfli_copy(&base.join("root/zopfli"));
        let traces = t.join("migration/units/u001-katajainen/traces");
        std::fs::create_dir_all(&traces).unwrap();
        let response = traces.join("0123abcd.response.json");
        let fake = fake_harness(
            "answer",
            &format!(
                r#"echo "$@" >> "$0.argv"
if [ -e '{r}' ]; then
  echo '{{"k":"attempt","unit":"u","id":"a-00000000000a","outcome":"green","provider":"external","model":"m","promoted":false,"promotion":""}}'
  exit 0
fi
echo '{{"k":"error","kind":"awaiting","message":"awaiting response: {r}"}}'
echo '{{"k":"awaiting","attempt":"a-00000000000a","path":"{r}","resume":""}}'
exit 1"#,
                r = response.display()
            ),
        );
        let mut cfg = config(fake.clone(), &["external"]);
        cfg.target_roots = vec![base.join("root").canonicalize().unwrap()];
        let mut s = Server::new(cfg, Vec::new(), ChildSlot::default(), Gate::default());
        let steer = json!({"unit": "u001-katajainen", "from": "a-000000000001",
                           "steer": "keep it", "model": "claude-opus-5-5",
                           "target": t.to_string_lossy()});
        assert!(say(&mut s, &call_line(1, "harness_steer", steer.clone(), None)).is_empty());
        let seen = pump_until(&mut s, |m| response_to(m, 1).is_some());
        let r = response_to(&seen, 1).unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(structured(r)["awaiting"]["attempt"], "a-00000000000a");
        assert_eq!(s.hand_offs.len(), 1);
        // Another model: refused, nothing written, still remembered.
        // `answer_with` carries the arguments to pass back, the target
        // included (a hand-off is the posing target's).
        let with = &structured(r)["awaiting"]["answer_with"];
        assert_eq!(with["tool"], "harness_answer");
        assert_eq!(
            with["arguments"]["model"], "claude-opus-5-5",
            "plain: passed back as is"
        );
        assert_eq!(with["arguments"]["target"], steer["target"]);
        let posed_args = with["arguments"].clone();
        let answer = |model: &str| {
            let mut a = posed_args.clone();
            a["model"] = json!(model);
            a["text"] = json!("src/logic.rs ...");
            a
        };
        // The same attempt on the default target is not this hand-off.
        let r = call(
            &mut s,
            9,
            "harness_answer",
            json!({"attempt": "a-00000000000a", "model": "claude-opus-5-5", "text": "x"}),
        );
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("never answered here"));
        let r = call(&mut s, 2, "harness_answer", answer("claude-sonnet-5"));
        assert_eq!(r["result"]["isError"], true);
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("claude-opus-5-5"));
        assert!(!response.exists());
        assert_eq!(s.hand_offs.len(), 1);
        // A target spoiled meanwhile: refused before anything is written.
        let linked = t.join("harness.toml");
        let outside = base.join("outside.toml");
        std::fs::rename(&linked, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        let r = call(&mut s, 8, "harness_answer", answer("claude-opus-5-5"));
        assert_eq!(structured(&r)["error"]["kind"], "unreadable", "{r}");
        assert!(!response.exists());
        std::fs::remove_file(&linked).unwrap();
        std::fs::rename(&outside, &linked).unwrap();
        // Its model: written, resumed with the same argv, green.
        assert!(say(
            &mut s,
            &call_line(3, "harness_answer", answer("claude-opus-5-5"), None)
        )
        .is_empty());
        let seen = pump_until(&mut s, |m| response_to(m, 3).is_some());
        let r = response_to(&seen, 3).unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(structured(r)["attempt"]["outcome"], "green");
        assert_eq!(
            structured(r)["act"],
            "harness_steer",
            "the posing act's result"
        );
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({"text": "src/logic.rs ...", "input_tokens": 0, "output_tokens": 0,
                   "stop_reason": "end_turn"})
        );
        let argvs = std::fs::read_to_string(format!("{}.argv", fake.display())).unwrap();
        let lines: Vec<&str> = argvs.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], lines[1], "the same argv resumes it");
        assert!(s.hand_offs.is_empty(), "answered: forgotten");
        let r = call(&mut s, 4, "harness_answer", answer("claude-opus-5-5"));
        assert_eq!(r["result"]["isError"], true);
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }

    /// §R2 TRUST-6, TESTS-1: an `external` steer attempt is retried only by
    /// the model that answered it; steer and retry spawn the canonical
    /// target, never the caller's spelling.
    #[test]
    fn retry_names_its_model_and_acts_spawn_the_canonical_target() {
        let _guard = crate::policy::tests::TmpDir::new("rt");
        let base = _guard.0.clone();
        let t = crate::policy::tests::zopfli_copy(&base.join("root/zopfli"));
        let dir = t.join("migration/units/u001-katajainen/attempts/a-0000000000aa");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("attempt.json"),
            json!({
                "schema": "ruharness-attempt", "schema_version": 1, "id": "a-0000000000aa",
                "unit": "u001-katajainen", "provider": "external", "provider_kind": "external",
                "model": "claude-opus-5-5", "prompt_digest": "", "unit_source": "s",
                "driver": "d", "toolchain": [], "outcome": "green", "turns": [],
                "candidate_digest": "c", "promoted": false,
                "seeded_from": "a-000000000001", "steer_note": "keep it",
            })
            .to_string(),
        )
        .unwrap();
        let fake = fake_harness("rt", "exit 0");
        let mut cfg = config(fake.clone(), &["external"]);
        cfg.target_roots = vec![base.join("root").canonicalize().unwrap()];
        let mut s = Server::new(cfg, Vec::new(), ChildSlot::default(), Gate::default());
        let _cleanup = Cleanup {
            slot: s.slot.clone(),
            dir: fake.parent().unwrap().to_path_buf(),
        };
        let spelled = format!("{}/../zopfli/.", t.display());
        let retry = |model: Option<&str>| {
            let mut a = json!({"unit": "u001-katajainen", "attempt": "a-0000000000aa",
                               "target": spelled});
            if let Some(m) = model {
                a["model"] = json!(m);
            }
            a
        };
        let r = call(&mut s, 1, "harness_retry", retry(None));
        assert_eq!(r["error"]["code"], -32602, "{r}");
        let r = call(&mut s, 2, "harness_retry", retry(Some("claude-sonnet-5")));
        assert_eq!(r["result"]["isError"], true);
        assert!(structured(&r)["error"]["message"]["text"]
            .as_str()
            .unwrap()
            .contains("claude-opus-5-5"));
        let canonical = format!("--target={}", t.display());
        let spawned_argv = |s: &Server<Vec<u8>>| -> Vec<String> {
            s.in_flight
                .as_ref()
                .expect("spawned")
                .running
                .argv()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert!(say(
            &mut s,
            &call_line(3, "harness_retry", retry(Some("claude-opus-5-5")), None)
        )
        .is_empty());
        let argv = spawned_argv(&s);
        assert!(argv.contains(&canonical), "{argv:?}");
        assert!(
            argv.contains(&"--model=claude-opus-5-5".to_string()),
            "{argv:?}"
        );
        pump_until(&mut s, |m| response_to(m, 3).is_some());
        let steer = json!({"unit": "u001-katajainen", "from": "a-0000000000aa",
                           "steer": "x", "model": "m", "target": spelled});
        assert!(say(&mut s, &call_line(4, "harness_steer", steer, None)).is_empty());
        let argv = spawned_argv(&s);
        assert!(argv.contains(&canonical), "{argv:?}");
        pump_until(&mut s, |m| response_to(m, 4).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_closed_gate_spawns_nothing() {
        let (mut s, fake, _cleanup) = spinning_server("gate");
        *s.gate.lock().unwrap() = true;
        let r = say(&mut s, &promote(1, None));
        assert_eq!(structured(&r[0])["error"]["kind"], "spawn-failed");
        assert!(s.in_flight.is_none());
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }

    #[test]
    fn the_shutdown_interrupts_the_running_act() {
        let (mut s, fake, _cleanup) = spinning_server("shutdown");
        assert!(say(&mut s, &promote(1, Some("t"))).is_empty());
        // Its first event is printed after its `trap`.
        pump_until(&mut s, |m| {
            m.iter().any(|m| m["method"] == "notifications/progress")
        });
        shut_down(&s.gate, &s.slot);
        assert!(*s.gate.lock().unwrap(), "the gate stays closed");
        let log = std::fs::read_to_string(format!("{}.log", fake.display())).unwrap();
        assert_eq!(log.trim(), "interrupted");
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }
}
