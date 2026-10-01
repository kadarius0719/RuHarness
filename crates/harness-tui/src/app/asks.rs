//! The chat's side of the cockpit (docs/CHAT-PANE-DESIGN.md §2–§3, §5.3,
//! §5.4): every act the chat asks for is the cockpit's OWN act — its argv
//! builder, its gates (again at confirm, on a fresh read), its armed dialog,
//! its spawned `harness --json …` — confirmed by the person. The chat's
//! permission request is held meanwhile and answered `deny` with the
//! outcome once the first read after the reap finished. A request never
//! opens a dialog by itself: it waits on a line above the input, inert for
//! a second, and is reviewed, declined, or (a Continue under the
//! continuation permission) held.

use super::*;
use crate::chat::transcript::Tone as T;
use crate::chat::{short_attempt, Event as ChatEvent};
use crate::fence;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

/// A request line is inert this long after it appears (§3.2).
pub const REQUEST_SETTLE: Duration = crate::dialog::CLICK_SETTLE;
/// Keys read within this long of each other are one burst (§3.2).
pub const BURST: Duration = Duration::from_millis(5);
/// The key read this soon after one read with input still pending closes
/// that burst (a frame drawn between two reads can take longer than
/// [`BURST`]).
pub const BURST_TAIL: Duration = Duration::from_millis(100);
/// Typing this recent guards the panes when the focus leaves the chat.
pub const TYPED_RECENTLY: Duration = Duration::from_secs(2);
/// A waiting Continue waits this long after the person's last key or press.
pub const CONTINUE_QUIET: Duration = Duration::from_secs(1);
/// Longest answer the chat may give (the CLI's own bound).
pub const MAX_ANSWER_BYTES: usize = 512 * 1024;
/// Largest outcome message sent back to the chat.
pub const MAX_OUTCOME_BYTES: usize = 8 * 1024;
/// "Declined": the model reads it as the call's result.
pub const DECLINED: &str = "declined by the person";

/// A chat act, on its [`Pending`]: which request it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTag {
    /// The chat's generation.
    pub gen: u64,
    /// The permission request it answers.
    pub request_id: String,
    /// The tool (`harness_migrate`, …).
    pub tool: String,
    /// The model the dialog names as answering the hand-offs (the chat's).
    pub model: String,
    /// Confirming it grants the continuation permission (§3.4): Migrate,
    /// Modify or Retry on `external`, sandboxed.
    pub grant: bool,
    /// Continue: the answer, for the command's stdin (never a file).
    pub answer: Option<String>,
    /// Continue: the request key it answers.
    pub key: Option<String>,
    /// Continue under the continuation permission: no dialog.
    pub permitted: bool,
    /// Continue: the attempt it answers.
    pub attempt: Option<String>,
    /// The permission epoch when the person confirmed it: an act whose
    /// run saw any cause end a permission grants none (§3.4).
    pub epoch: u64,
}

/// What a run reported, for the chat's outcome (§3.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    /// The attempt it ran (the last named).
    pub attempt: Option<String>,
    /// Its `attempt` event's outcome.
    pub outcome: Option<String>,
    /// Its `awaiting` event: the attempt and the request key.
    pub awaiting: Option<(Option<String>, Option<String>)>,
    /// Its `error` event: kind, message.
    pub error: Option<(String, String)>,
    /// The failed checks, by name.
    pub failed: Vec<String>,
    /// Every check.
    pub checks: usize,
    /// Its messages (the last few kept).
    pub messages: Vec<String>,
    /// The last turn it started.
    pub turn: Option<u64>,
}

impl Collected {
    /// One event of the run.
    pub fn on_event(&mut self, ev: &Event) {
        match ev {
            Event::TurnStart { attempt, index, .. } => {
                self.attempt = Some(attempt.clone());
                self.turn = Some(*index);
            }
            Event::Attempt { id, outcome, .. } => {
                self.attempt = Some(id.clone());
                self.outcome = Some(outcome.clone());
            }
            Event::Check { name, passed, .. } => {
                self.checks += 1;
                if !passed {
                    self.failed.push(name.clone());
                }
            }
            Event::Awaiting {
                attempt,
                request_key,
                ..
            } => {
                if let Some(a) = attempt {
                    self.attempt = Some(a.clone());
                }
                self.awaiting = Some((attempt.clone(), request_key.clone()));
            }
            Event::Error { kind, message, .. } => {
                self.error = Some((kind.clone(), message.clone()));
            }
            Event::Message { text } => {
                self.messages.push(text.clone());
                if self.messages.len() > 6 {
                    self.messages.remove(0);
                }
            }
            _ => {}
        }
    }
}

/// A request of the chat's, waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Its act.
    pub pending: Pending,
    /// When it became the line shown (the settle runs from here).
    pub shown_at: Option<Instant>,
    /// The line's words ("Migrate u001 — a model call, answered here in
    /// chat").
    pub words: String,
    /// A Continue that asks: its dialog shows the answer whole.
    pub continue_asks: bool,
}

impl Request {
    fn tag(&self) -> &ChatTag {
        self.pending
            .chat
            .as_ref()
            .expect("a chat request carries its tag")
    }
}

/// An attempt waiting for the chat's answer: the chat hand-off table's row
/// (§3.4) — "the key the cockpit holds".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOff {
    /// The request key it waits on.
    pub key: String,
    /// The chat generation whose act posed it.
    pub gen: u64,
    /// The act that posed it, in words.
    pub act: String,
    /// The turn it waits on.
    pub turn: Option<u64>,
    /// Its unit.
    pub unit: String,
}

/// A continuation permission (§3.4): Continues of this attempt, by this
/// chat and this model, run without a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permit {
    /// The chat generation.
    pub gen: u64,
    /// The model the dialog named.
    pub model: String,
    /// The request of the act that granted it.
    pub from: String,
}

/// A chat act's outcome, waiting for the first read after its reap.
#[derive(Debug, Clone, PartialEq)]
pub struct Owed {
    /// Its tag.
    pub tag: ChatTag,
    /// The act in words.
    pub label: String,
    /// What the run reported.
    pub collect: Collected,
    /// Its exit code.
    pub exit: Option<i32>,
    /// The signal it died by.
    pub signal: Option<String>,
    /// Continue: where the CLI files the answer.
    pub response: Option<PathBuf>,
}

/// The chat's requests, hand-offs and permissions.
#[derive(Debug, Clone, Default)]
pub struct Asks {
    /// Requests waiting for the person, the first shown.
    pub requests: VecDeque<Request>,
    /// A Continue under the permission, waiting to run.
    pub waiting: Option<Request>,
    /// The chat hand-off table, by attempt.
    pub table: BTreeMap<String, HandOff>,
    /// Continuation permissions, by attempt.
    pub permits: BTreeMap<String, Permit>,
    /// Outcomes the chat was not told, by generation (sent with the next
    /// message to the same chat).
    pub unsent: Vec<(u64, String)>,
    /// Chat acts' outcomes waiting for the read after their reap, in order.
    pub outcome: Vec<Owed>,
    /// Bumped by every cause that ends a generation's continuation
    /// permissions (§3.4) and by the person's Cancel of a grant act: a
    /// grant act confirmed before a bump grants nothing.
    pub epoch: u64,
    /// What ended each attempt's permission, in words (review USE-15;
    /// fix check 2, finding 3) — until it is granted again.
    pub ended: BTreeMap<String, &'static str>,
    /// Reads in a row, up to the last one, with more input pending.
    pending_run: u32,
    /// The typing guard is up.
    pub guard: bool,
    /// When the person last typed into the chat's input.
    pub typed_at: Option<Instant>,
    /// Bumped whenever what the chat's `Enter`/`Esc` mean changes: a held
    /// press is dropped across it.
    pub meaning: u64,
    /// The last key was part of a burst.
    pub burst: bool,
    /// When the person last pressed a key or a button.
    pub last_press: Option<Instant>,
    last_key: Option<Instant>,
    /// The chat's transcript area at the last draw (columns, rows).
    pub transcript_size: (usize, usize),
}

impl Asks {
    /// A key was read at `now` (`pending`: more input was waiting): whether
    /// it is part of a burst — keys within [`BURST`] of each other, the
    /// last one included.
    pub fn key_read(&mut self, now: Instant, pending: bool) {
        // The key after reads with input pending is the same burst's last,
        // even when a frame drawn between the reads took longer than 5 ms
        // (review PRO-13) — after two such reads in a row: two keys typed
        // while the loop was busy ("ok" then Enter) are no paste (fix check
        // N3).
        let since = self.last_key.map(|t| now.saturating_duration_since(t));
        self.burst = pending
            || since.is_some_and(|d| d < BURST)
            || (self.pending_run >= 2 && since.is_some_and(|d| d < BURST_TAIL));
        self.last_key = Some(now);
        self.pending_run = if pending { self.pending_run + 1 } else { 0 };
        self.last_press = Some(now);
    }

    /// The focus leaves the chat at `now` with a `draft` or not: the typing
    /// guard goes up when there is one or typing was recent.
    pub fn leaving_chat(&mut self, now: Instant, draft: bool) {
        self.guard = draft
            || self
                .typed_at
                .is_some_and(|t| now.saturating_duration_since(t) < TYPED_RECENTLY);
    }

    /// The request shown on the line, if any.
    pub fn shown(&self) -> Option<&Request> {
        self.requests.front()
    }

    /// The shown request's line is past its settle at `now`.
    pub fn settled(&self, now: Instant) -> bool {
        self.shown()
            .and_then(|r| r.shown_at)
            .is_some_and(|t| now.saturating_duration_since(t) >= REQUEST_SETTLE)
    }

    /// Make the queue's first request the line shown, from `now`.
    fn show_next(&mut self, now: Instant) {
        if let Some(r) = self.requests.front_mut() {
            if r.shown_at.is_none() {
                r.shown_at = Some(now);
            }
        }
        self.meaning += 1;
    }

    /// Every permission of generation `gen` ends for `why` — and one a
    /// grant act running now would give is never given (§3.4; review
    /// SAF-1).
    pub fn end_permits(&mut self, gen: u64, why: &'static str) {
        let ended = &mut self.ended;
        self.permits.retain(|a, p| {
            let keep = p.gen != gen;
            if !keep {
                ended.insert(a.clone(), why);
            }
            keep
        });
        self.epoch += 1;
    }

    /// The permission for `attempt` ends (a hold, a declined or cancelled
    /// Continue). A grant act running now is another attempt's (one
    /// command runs at a time; its attempt is a new one): its grant stands
    /// (fix check N6).
    pub fn end_permit(&mut self, attempt: &str, why: &'static str) {
        if self.permits.remove(attempt).is_some() {
            self.ended.insert(attempt.to_string(), why);
        }
    }
}

/// The dialog's chat lines (§3.2): who asked, who answers the turns. A
/// Continue's answer is drawn whole after them by the view, behind a
/// gutter only the cockpit writes (`view::answer_rows`).
pub fn chat_words(tag: &ChatTag, body: &mut Vec<String>, turns: u32) {
    body.insert(
        0,
        format!(
            "Asked in chat by {}. Read what it does before you run it.",
            tag.model
        ),
    );
    if tag.grant {
        body.insert(
            1,
            format!(
                "The chat answers its model turns (up to {turns}) here; each answer continues the \
                 run without asking again. Nothing is accepted without you."
            ),
        );
    }
}

/// A refusal as the chat reads it: fenced (§3.2 item 6).
fn refused(why: &str) -> String {
    json!({"refused": fence::untrusted("cockpit-reason", why, fence::MESSAGE_CAP)}).to_string()
}

fn arg<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

impl App {
    /// A line of the cockpit's in the transcript — new output: "Chat ●"
    /// outside the chat (review USE-12).
    fn chat_line(&mut self, tone: T, text: impl Into<String>) {
        self.chat.unseen = true;
        self.chat.transcript.line(tone, text);
    }

    // ----- the chat's events -------------------------------------------------

    /// One pass of the loop for the chat: what its runtime printed, and the
    /// requests that come of it.
    pub fn chat_pump(&mut self, now: Instant) {
        if !self.chat_on {
            return;
        }
        for event in self.chat.pump(now) {
            self.on_chat_event(event, now);
        }
    }

    fn on_chat_event(&mut self, event: ChatEvent, now: Instant) {
        match event {
            ChatEvent::Ask {
                gen,
                request_id,
                tool,
                input,
                model,
                ..
            } => self.chat_ask(gen, request_id, &tool, &input, model, now),
            ChatEvent::Withdrawn { gen, request_id } => {
                // A request of it withdrawn: its permissions end (§3.4).
                self.asks.end_permits(gen, "the chat withdrew a request");
                self.withdraw(|t| t.gen == gen && t.request_id == request_id, now);
            }
            ChatEvent::Read { .. } => {}
            ChatEvent::Foreign { gen } => self
                .asks
                .end_permits(gen, "a message the cockpit did not send reached the chat"),
            ChatEvent::Ended { gen } => self.chat_gen_ended(gen, now),
        }
    }

    /// A generation ended: its requests withdrawn, its permissions and
    /// hand-offs gone, its unsent outcomes shown here only (§1.2, §3.3).
    fn chat_gen_ended(&mut self, gen: u64, now: Instant) {
        self.asks.end_permits(gen, "the chat ended");
        self.asks.table.retain(|_, h| h.gen != gen);
        self.withdraw(|t| t.gen == gen, now);
        let (theirs, rest): (Vec<_>, Vec<_>) =
            self.asks.unsent.drain(..).partition(|(g, _)| *g == gen);
        self.asks.unsent = rest;
        for (_, text) in theirs {
            self.chat_line(
                T::Dim,
                format!("the chat that asked for it ended before it heard: {text}"),
            );
        }
    }

    /// Withdraw every request (queued, shown, waiting, in its dialog) whose
    /// tag matches: a dialog open for one closes with a notice — a frame
    /// change for the swallow (§1.2, §5.5).
    fn withdraw(&mut self, matches: impl Fn(&ChatTag) -> bool, now: Instant) {
        let before = self.asks.requests.len() + usize::from(self.asks.waiting.is_some());
        self.asks.requests.retain(|r| !matches(r.tag()));
        if self.asks.waiting.as_ref().is_some_and(|w| matches(w.tag())) {
            self.asks.waiting = None;
        }
        let dialog = matches!(&self.mode, Mode::Dialog(c)
            if matches!(&c.purpose, Purpose::Act(p) if p.chat.as_deref().is_some_and(&matches)));
        if dialog {
            let kind = self.mode_kind();
            self.mode = Mode::Normal;
            self.note_mode_change(kind, now);
            self.say("the chat withdrew its request; the dialog closed");
        }
        let after = self.asks.requests.len() + usize::from(self.asks.waiting.is_some());
        if after < before || dialog {
            self.chat_line(T::Dim, "the request was withdrawn");
            self.asks.show_next(now);
        }
    }

    // ----- a request arrives ---------------------------------------------------

    fn chat_ask(
        &mut self,
        gen: u64,
        request_id: String,
        tool: &str,
        input: &Value,
        model: Option<String>,
        now: Instant,
    ) {
        match self.chat_request(gen, &request_id, tool, input, model.as_deref()) {
            Err(why) => {
                self.chat.deny(&request_id, &refused(&why));
                self.chat_line(T::Bad, format!("refused: {why}"));
            }
            // One waiting Continue at a time: another permitted one asks
            // (review PRO-3) — never dropped unanswered.
            Ok(mut r) if r.tag().permitted && self.asks.waiting.is_some() => {
                if let Some(t) = r.pending.chat.as_deref_mut() {
                    t.permitted = false;
                }
                r.continue_asks = true;
                r.words = format!("{} — review the answer", r.words);
                self.chat_line(T::Warn, format!("asks: {}", r.words));
                self.asks.requests.push_back(r);
                if self.asks.requests.len() == 1 {
                    self.asks.show_next(now);
                }
            }
            Ok(r) if r.tag().permitted => {
                self.chat_line(
                    T::Warn,
                    format!("{} — waits for a quiet moment; Esc holds it", r.words),
                );
                self.asks.waiting = Some(r);
                self.asks.meaning += 1;
            }
            Ok(r) => {
                self.chat_line(T::Warn, format!("asks: {}", r.words));
                self.asks.requests.push_back(r);
                if self.asks.requests.len() == 1 {
                    self.asks.show_next(now);
                }
                if self.focus != Focus::Chat {
                    let key = if self.focus == Focus::Files {
                        "Shift-Tab"
                    } else {
                        "Tab"
                    };
                    self.say(format!(
                        "The chat asks: {} — {key} to the chat",
                        self.asks.shown().map_or("", |r| r.words.as_str())
                    ));
                }
            }
        }
    }

    /// The answering model: the one of the message that made the call, which
    /// must be the one the chat's latest `init` names, a valid name (§3.1).
    fn answering_model(&self, model: Option<&str>) -> Result<String, String> {
        let init = self.chat.model.as_deref().unwrap_or_default();
        match model {
            Some(m) if m == init && fence::valid_model(m) => Ok(m.to_string()),
            Some(m) => Err(format!(
                "the call came from model {m}, not the chat's ({init})"
            )),
            None => Err("the call's model is not known".into()),
        }
    }

    /// The act a tool call maps onto, or why not (§3.1).
    pub(crate) fn chat_request(
        &self,
        gen: u64,
        request_id: &str,
        tool: &str,
        input: &Value,
        model: Option<&str>,
    ) -> Result<Request, String> {
        let model = self.answering_model(model)?;
        // Until a read lands after a failed one, the chat's acts wait (§3.3).
        if let Some(e) = &self.last_load_error {
            return Err(format!(
                "the ledger could not be re-read ({e}) — the person presses g to read it again"
            ));
        }
        let tag = |grant: bool| ChatTag {
            gen,
            request_id: request_id.to_string(),
            tool: tool.to_string(),
            model: model.clone(),
            grant,
            answer: None,
            key: None,
            permitted: false,
            attempt: None,
            epoch: 0,
        };
        let need = |key: &str| {
            arg(input, key)
                .map(str::to_string)
                .ok_or_else(|| format!("`{key}` is missing"))
        };
        let external = self
            .config
            .providers
            .first()
            .is_some_and(|p| p == EXTERNAL_PROVIDER);
        let grant = external && !self.config.allow_unsandboxed;
        let unit = need("unit")?;
        match tool {
            "harness_migrate" => {
                let mut p = self.chat_migrate_argv(&unit, &model)?;
                p.chat = Some(Box::new(tag(grant)));
                Ok(Request {
                    words: format!(
                        "Migrate {unit} — a model call{}",
                        if external {
                            ", answered here in chat"
                        } else {
                            ""
                        }
                    ),
                    pending: p,
                    shown_at: None,
                    continue_asks: false,
                })
            }
            "harness_steer" => {
                let from = need("from")?;
                let note = need("steer")?;
                if let Some(why) = note_problem(&note, MAX_NOTE_BYTES) {
                    return Err(format!("the note: {why}"));
                }
                // A request waits for a running command (§3.2): the
                // running-command gate is Review's and confirm's.
                let mut p =
                    self.act_argv_unchecked(Act::Modify, Some(&unit), Some(&from), Some(&note))?;
                chat_shape(&mut p.argv, &model, external);
                p.label = format!("Modify {} (asked in chat)", short_id(&from));
                p.chat = Some(Box::new(tag(grant)));
                Ok(Request {
                    words: format!(
                        "Modify {} with a note — a model call{}",
                        short_id(&from),
                        if external {
                            ", answered here in chat"
                        } else {
                            ""
                        }
                    ),
                    pending: p,
                    shown_at: None,
                    continue_asks: false,
                })
            }
            "harness_retry" => {
                let attempt = need("attempt")?;
                let mut p = self.chat_retry_argv(&unit, &attempt, &model)?;
                let external = p.argv.iter().any(|a| a == "--provider=external");
                p.chat = Some(Box::new(tag(external && !self.config.allow_unsandboxed)));
                Ok(Request {
                    words: format!(
                        "Retry {} — a model call{}",
                        short_id(&attempt),
                        if external {
                            ", answered here in chat"
                        } else {
                            ""
                        }
                    ),
                    pending: p,
                    shown_at: None,
                    continue_asks: false,
                })
            }
            "harness_answer" => {
                let attempt = need("attempt")?;
                let key = need("request_key")?;
                let text = need("text")?;
                let mut p = self.chat_continue_argv(gen, &unit, &attempt, &key, &text, &model)?;
                let permitted = !self.config.allow_unsandboxed
                    && self
                        .asks
                        .permits
                        .get(&attempt)
                        .is_some_and(|pm| pm.gen == gen && pm.model == model);
                let mut t = tag(false);
                t.answer = Some(text);
                t.key = Some(key);
                t.permitted = permitted;
                t.attempt = Some(attempt.clone());
                p.chat = Some(Box::new(t));
                let turn = self.asks.table.get(&attempt).and_then(|h| h.turn);
                let turn = turn.map_or_else(String::new, |n| format!(" turn {n}"));
                Ok(Request {
                    words: if permitted {
                        format!("Continues {}{turn}", short_attempt(&attempt))
                    } else {
                        format!(
                            "Continue {}{turn} with the chat's answer — read it first",
                            short_attempt(&attempt)
                        )
                    },
                    pending: p,
                    shown_at: None,
                    continue_asks: !permitted,
                })
            }
            _ => Err("not available in the cockpit".into()),
        }
    }

    /// The chat's Migrate (§3.1): `migrate <unit> --no-promote
    /// --provider=<p> --model=<m> --requester=chat` — `<p>` the first
    /// `--provider`; `<m>` the chat's model when `<p>` is `external` (the
    /// chat answers), else the target's migrate model. Only a unit that is
    /// planned, tried or failing.
    fn chat_migrate_argv(&self, unit: &str, model: &str) -> Result<Pending, String> {
        if !harness_core::plan::is_clean_segment(unit) {
            return Err(format!("{unit:?} is not a plain unit id"));
        }
        let (i, u) = self
            .snapshot
            .units
            .iter()
            .enumerate()
            .find(|(_, u)| u.unit.id == unit)
            .ok_or_else(|| format!("unit {unit} is not in the plan"))?;
        let state = self
            .files
            .units
            .get(i)
            .map(|x| x.state.clone())
            .unwrap_or_else(|| crate::files::unit_state(u));
        if !matches!(
            state,
            UnitState::Planned | UnitState::Tried | UnitState::Failing
        ) {
            return Err(format!(
                "unit {unit} is {} — Migrate is for a unit that is planned, tried or failing",
                state.word()
            ));
        }
        let provider = self
            .config
            .providers
            .first()
            .ok_or("no provider is allowed (start with --provider <name>)")?;
        let m = if provider == EXTERNAL_PROVIDER {
            model.to_string()
        } else {
            self.migrate_model.clone()
        };
        let rest = vec![
            os("migrate"),
            os(unit),
            self.target_arg(),
            os("--no-promote"),
            os(format!("--provider={provider}")),
            os(format!("--model={m}")),
            os("--requester=chat"),
        ];
        Ok(Pending {
            stdin: None,
            act: Act::Migrate,
            argv: self.with_sandbox_flag(self.harness_argv(&rest)?),
            label: format!("Migrate {unit} (asked in chat)"),
            unit: Some(unit.to_string()),
            attempt: None,
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
            chat: None,
        })
    }

    /// Why the chat's Retry of `r` is refused (§3.1): only a record labelled
    /// `chat` (a retry of any other would record unlabelled output at the
    /// chat's request); an `external` one only when its model is the chat's
    /// (the chat answers its turns); Retry's other refusals.
    fn chat_retry_refusal(&self, r: &AttemptRecord, model: &str) -> Option<String> {
        if r.requester.as_deref() != Some(harness_core::attempts::REQUESTER_CHAT) {
            return Some(format!(
                "attempt {} was not asked for in chat — the chat retries only its own attempts",
                r.id
            ));
        }
        let external = r.provider == EXTERNAL_PROVIDER || r.provider_kind == EXTERNAL_PROVIDER;
        if external && r.model != model {
            return Some(format!(
                "attempt {} was answered by {}, not by this chat's model {model}",
                r.id, r.model
            ));
        }
        if r.outcome == "in-progress" {
            return Some(format!("attempt {} is not finished", r.id));
        }
        if r.seeded_from.is_some() != r.steer_note.is_some() {
            return Some(format!(
                "attempt {} records only half of a steer: inconsistent, not retried",
                r.id
            ));
        }
        // The cockpit's own provider list binds the chat too, `external`
        // included (review SAF-8).
        if !self.config.providers.contains(&r.provider) {
            return Some(format!(
                "provider `{}` is not allowed — start the cockpit with `--provider {}`",
                r.provider, r.provider
            ));
        }
        None
    }

    fn chat_retry_argv(&self, unit: &str, attempt: &str, model: &str) -> Result<Pending, String> {
        for id in [unit, attempt] {
            if !harness_core::plan::is_clean_segment(id) {
                return Err(format!("{id:?} is not a plain id"));
            }
        }
        let u = self
            .snapshot
            .unit(unit)
            .ok_or_else(|| format!("unit {unit} is gone"))?;
        let a = u
            .attempt(attempt)
            .ok_or_else(|| format!("attempt {attempt} is not in unit {unit}"))?;
        if let Some(why) = self.chat_retry_refusal(&a.record, model) {
            return Err(why);
        }
        let rest = self.retry_rest(unit, &a.record);
        Ok(Pending {
            stdin: None,
            act: Act::Retry,
            argv: self.with_sandbox_flag(self.harness_argv(&rest)?),
            label: format!("Retry {} (asked in chat)", short_id(attempt)),
            unit: Some(unit.to_string()),
            attempt: Some(attempt.to_string()),
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
            chat: None,
        })
    }

    /// The response file of `key` in `unit`'s chat traces.
    fn chat_response(&self, unit: &str, key: &str) -> PathBuf {
        Ledger::new(&self.config.target)
            .unit_dir(unit)
            .join("traces")
            .join(harness_core::attempts::CHAT_TRACES)
            .join(format!("{key}.response.json"))
    }

    /// The chat's Continue (§3.4): only for a chat-labelled attempt in
    /// progress, whose request key the cockpit holds for THIS chat, which the
    /// chat read whole, answered by the attempt's model, with an answer of
    /// at most 512 KiB, whose key has no response yet. Its argv re-runs the
    /// record's own shape with the answer on stdin.
    fn chat_continue_argv(
        &self,
        gen: u64,
        unit: &str,
        attempt: &str,
        key: &str,
        text: &str,
        model: &str,
    ) -> Result<Pending, String> {
        for id in [unit, attempt] {
            if !harness_core::plan::is_clean_segment(id) {
                return Err(format!("{id:?} is not a plain id"));
            }
        }
        if key.len() != 8
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("the request key is not 8 lowercase hex".into());
        }
        let u = self
            .snapshot
            .unit(unit)
            .ok_or_else(|| format!("unit {unit} is gone"))?;
        let a = u
            .attempt(attempt)
            .ok_or_else(|| format!("attempt {attempt} is not in unit {unit}"))?;
        let r = &a.record;
        let held = self
            .asks
            .table
            .get(attempt)
            .filter(|h| h.gen == gen && h.unit == unit);
        let Some(held) = held else {
            return Err(no_key_held(r));
        };
        if held.key != key {
            return Err(format!(
                "attempt {attempt} waits on request {}, not {key}",
                held.key
            ));
        }
        if !self
            .chat
            .keys_read
            .contains(&(gen, attempt.to_string(), key.to_string()))
        {
            return Err(format!(
                "read the request whole first: harness_request {attempt} {key}, every page"
            ));
        }
        if r.requester.as_deref() != Some(harness_core::attempts::REQUESTER_CHAT) {
            return Err(format!(
                "attempt {attempt} was not asked for in chat: its hand-off is answered by hand \
                 (see Help), or ask for a steer attempt from it"
            ));
        }
        if r.outcome != "in-progress" {
            return Err(format!("attempt {attempt} is no longer in progress"));
        }
        if r.model != model {
            return Err(format!(
                "attempt {attempt} is answered by {}, not by {model}",
                r.model
            ));
        }
        if text.is_empty() {
            return Err("the answer is empty".into());
        }
        if text.len() > MAX_ANSWER_BYTES {
            return Err(format!(
                "the answer is {} bytes; at most {MAX_ANSWER_BYTES}",
                text.len()
            ));
        }
        if response_present(&self.chat_response(unit, key)) {
            return Err(format!(
                "request {key} already has a response — the harness files it on the next resume"
            ));
        }
        if !self.config.providers.contains(&r.provider) {
            return Err(format!(
                "provider `{}` is not allowed — the person starts the cockpit with `--provider {}`",
                r.provider, r.provider
            ));
        }
        let mut rest = vec![
            os("migrate"),
            os(unit),
            self.target_arg(),
            os("--no-promote"),
            os(format!("--provider={}", r.provider)),
            os(format!("--model={}", r.model)),
        ];
        if let (Some(seed), Some(note)) = (&r.seeded_from, &r.steer_note) {
            rest.push(os(format!("--from={seed}")));
            rest.push(os(format!("--steer={note}")));
        }
        if harness_core::attempts::sample_base(attempt) != attempt {
            rest.push(os("--retry"));
        }
        rest.push(os("--requester=chat"));
        rest.push(os("--answer=-"));
        rest.push(os(format!("--answer-bytes={}", text.len())));
        rest.push(os(format!("--answer-key={key}")));
        Ok(Pending {
            stdin: None,
            act: Act::Continue,
            argv: self.with_sandbox_flag(self.harness_argv(&rest)?),
            label: format!("Continue {} (asked in chat)", short_id(attempt)),
            unit: Some(unit.to_string()),
            attempt: Some(attempt.to_string()),
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
            chat: None,
        })
    }

    /// The chat acts' gates again at confirm, on a fresh read of what they
    /// act on (the lock holder, the running command and the preflight were
    /// checked by the caller).
    pub(super) fn chat_gate(&mut self, p: &Pending, tag: &ChatTag) -> Result<(), String> {
        if tag.gen != self.chat.gen || !self.chat.held.contains(&tag.request_id) {
            return Err("the chat withdrew this request".into());
        }
        let ledger = Ledger::new(&self.config.target);
        let unit = p.unit.as_deref().ok_or("no unit")?;
        let load = |a: &str| {
            AttemptRecord::load(&harness_core::attempts::attempt_dir(&ledger, unit, a))
                .map_err(|e| format!("attempt {a} could not be read: {e}"))
        };
        match p.act {
            Act::Migrate => {
                let plan = harness_core::plan::Plan::load(&ledger.plan_path())
                    .map_err(|e| format!("the plan could not be read: {e}"))?;
                let u = plan
                    .units
                    .iter()
                    .find(|u| u.id == unit)
                    .ok_or_else(|| format!("unit {unit} is no longer in the plan"))?;
                if matches!(u.status.as_str(), "verified" | "merged" | "blocked") {
                    return Err(format!("unit {unit} is {} now", u.status.as_str()));
                }
                Ok(())
            }
            Act::Retry => {
                let a = p.attempt.as_deref().ok_or("no attempt")?;
                match self.chat_retry_refusal(&load(a)?, &tag.model) {
                    Some(why) => Err(why),
                    None => Ok(()),
                }
            }
            Act::Continue => {
                let a = p.attempt.as_deref().ok_or("no attempt")?;
                let key = tag.key.as_deref().ok_or("no request key")?;
                let r = load(a)?;
                if r.outcome != "in-progress" {
                    return Err(format!("attempt {a} is no longer in progress"));
                }
                if r.requester.as_deref() != Some(harness_core::attempts::REQUESTER_CHAT) {
                    return Err(format!("attempt {a} was not asked for in chat"));
                }
                if r.model != tag.model {
                    return Err(format!("attempt {a} is answered by {}", r.model));
                }
                if self.asks.table.get(a).map(|h| h.key.as_str()) != Some(key) {
                    return Err(format!("attempt {a} no longer waits on request {key}"));
                }
                if response_present(&self.chat_response(unit, key)) {
                    return Err(format!("request {key} already has a response"));
                }
                Ok(())
            }
            // Modify's own gates were checked when its argv was built; the
            // note travels as the argv shows it.
            _ => Ok(()),
        }
    }

    // ----- the person's answer -------------------------------------------------

    /// Review (`Enter` on an empty input, or its button): the shown
    /// request's armed dialog — never before the line settled, never while
    /// a command runs (§3.2).
    pub(crate) fn chat_review(&mut self, now: Instant) {
        let Some(r) = self.asks.shown().cloned() else {
            return;
        };
        if !self.asks.settled(now) {
            self.say("a request just arrived — look first");
            return;
        }
        if self.running {
            self.say("Review is after the running command");
            return;
        }
        let tag = r.tag().clone();
        // Built again now: what the dialog shows is what the gates see.
        let fresh = if r.pending.act == Act::Continue {
            Ok(r.pending.clone())
        } else {
            let input = request_input(&r.pending);
            self.chat_request(
                tag.gen,
                &tag.request_id,
                &tag.tool,
                &input,
                Some(&tag.model),
            )
            .map(|f| f.pending)
        };
        match fresh {
            Ok(p) => self.open_dialog(Purpose::Act(p)),
            Err(why) => {
                self.asks.requests.pop_front();
                self.chat.deny(&tag.request_id, &refused(&why));
                self.chat_line(T::Bad, format!("refused: {why}"));
                self.say(format!("the chat's request is refused: {why}"));
                self.asks.show_next(now);
            }
        }
    }

    /// Decline the shown request (`Esc` on its line, or its button), with
    /// the draft as the reason when `with_draft` (§3.2).
    pub(crate) fn chat_decline(&mut self, now: Instant, with_draft: bool) {
        if !self.asks.settled(now) {
            self.say("a request just arrived — look first");
            return;
        }
        let Some(r) = self.asks.requests.pop_front() else {
            return;
        };
        let tag = r.tag().clone();
        let message = if with_draft && !self.chat.input.is_empty() {
            // The person's own words: never fenced as untrusted data (the
            // brief tells the model to follow nothing so fenced — review
            // USE-6).
            // Cut at the cap of the cockpit's messages to the chat — and
            // said (fix check N11; its check, finding 6).
            let mut reason = self.chat.input.take();
            if reason.len() > fence::MESSAGE_CAP {
                let mut end = fence::MESSAGE_CAP;
                while !reason.is_char_boundary(end) {
                    end -= 1;
                }
                reason.truncate(end);
                reason.push_str(" … (cut)");
                self.chat_line(
                    T::Warn,
                    format!(
                        "your reason was cut to its first {} KiB for the chat",
                        fence::MESSAGE_CAP / 1024
                    ),
                );
            }
            format!("{DECLINED}, who says: {}", json!(reason))
        } else {
            DECLINED.to_string()
        };
        self.chat_declined(&tag, &message, now);
    }

    fn chat_declined(&mut self, tag: &ChatTag, message: &str, now: Instant) {
        // Declining a Continue ends the permission (§3.4) — its attempt
        // rides on the tag (review SAF-5).
        if let Some(a) = &tag.attempt {
            self.asks
                .end_permit(a, "you declined another answer of the chat's for it");
        }
        self.chat.deny(&tag.request_id, message);
        self.chat_line(T::Bad, "✗ declined");
        self.asks.show_next(now);
    }

    /// Hold the waiting Continue (`Esc` on its line, or `[Hold]`): the
    /// permission ends and it becomes a Continue that asks — its request
    /// line, settling anew (§3.4).
    pub(crate) fn chat_hold(&mut self, now: Instant) {
        self.chat_asks_instead(now, "held");
    }

    /// The waiting Continue becomes a Continue that asks — `why` in words
    /// ("held" when the person held it, else what ended the permission,
    /// review USE-15).
    fn chat_asks_instead(&mut self, now: Instant, why: &str) {
        let Some(mut r) = self.asks.waiting.take() else {
            return;
        };
        if let Some(a) = r.pending.attempt.clone() {
            self.asks.end_permit(&a, "you held a Continue of it");
        }
        if let Some(t) = r.pending.chat.as_mut() {
            t.permitted = false;
        }
        r.continue_asks = true;
        r.words = format!("{} — {why}: review the answer", r.words);
        r.shown_at = None;
        self.chat_line(T::Warn, format!("{why}: {}", r.words));
        self.asks.requests.push_front(r);
        self.asks.show_next(now);
    }

    /// A dialog of a chat act closed (§3.2 item 4).
    pub(super) fn close_chat_dialog(&mut self, confirm: Confirm, choice: Choice) -> Command {
        let Purpose::Act(p) = confirm.purpose else {
            return Command::None;
        };
        let Some(tag) = p.chat.as_deref().cloned() else {
            return Command::None;
        };
        let now = self.now;
        let ours = |r: &Request| r.tag().request_id == tag.request_id;
        match choice {
            Choice::Run => match self.confirm_gate(&p) {
                Ok(()) => {
                    self.asks.requests.retain(|r| !ours(r));
                    self.asks.show_next(now);
                    let mut p = p;
                    if let Some(t) = p.chat.as_deref_mut() {
                        t.epoch = self.asks.epoch;
                    }
                    Command::Spawn(p)
                }
                Err(why) => {
                    self.asks.requests.retain(|r| !ours(r));
                    self.chat.deny(&tag.request_id, &refused(&why));
                    self.chat_line(T::Bad, format!("refused: {why}"));
                    self.say(format!("{}: {why}", p.label));
                    self.asks.show_next(now);
                    Command::None
                }
            },
            _ => {
                self.asks.requests.retain(|r| !ours(r));
                self.chat_declined(&tag, DECLINED, now);
                Command::None
            }
        }
    }

    // ----- running, reaping, answering -----------------------------------------

    /// A command was spawned: a chat act says so in the transcript.
    pub(super) fn chat_spawned(&mut self, p: &Pending) {
        let Some(tag) = &p.chat else {
            return;
        };
        if tag.permitted {
            self.asks.waiting = None;
            self.chat_line(
                T::Good,
                format!(
                    "continued, as you agreed when you ran the migration: {}",
                    p.label
                ),
            );
        } else {
            self.chat_line(T::Good, format!("you ran it: {}", p.label));
        }
    }

    /// A chat act's command could not be started: the chat hears why (a
    /// waiting Continue is gone with it).
    pub(super) fn chat_start_failed(&mut self, tag: &ChatTag, why: &str) {
        let why = format!("the command could not be started: {why}");
        if tag.permitted {
            self.asks.waiting = None;
        }
        self.chat.deny(&tag.request_id, &refused(&why));
        self.chat_line(T::Bad, format!("refused: {why}"));
        self.say(format!("the chat's act: {why}"));
    }

    /// An `awaiting` event of a run for the chat: the hand-off table holds
    /// its key; a confirmed act that grants the continuation permission
    /// grants it for this attempt (§3.4).
    pub(super) fn chat_hand_off(
        &mut self,
        attempt: Option<String>,
        key: Option<String>,
        tag: Option<ChatTag>,
        turn: Option<u64>,
        unit: String,
        label: String,
    ) {
        let (Some(attempt), Some(key)) = (attempt, key) else {
            return;
        };
        let gen = tag.as_ref().map_or(self.chat.gen, |t| t.gen);
        self.asks.table.insert(
            attempt.clone(),
            HandOff {
                key,
                gen,
                act: label,
                turn,
                unit,
            },
        );
        // Granted only if nothing ended a permission since the person
        // confirmed the act, its request is still held and its chat still
        // runs (§3.4; review SAF-1).
        let live = |t: &ChatTag| {
            t.epoch == self.asks.epoch
                && t.gen == self.chat.gen
                && self.chat.alive()
                && self.chat.held.contains(&t.request_id)
        };
        if let Some(t) = tag.filter(|t| t.grant && !self.config.allow_unsandboxed && live(t)) {
            self.asks.ended.remove(&attempt);
            self.asks.permits.insert(
                attempt,
                Permit {
                    gen: t.gen,
                    model: t.model,
                    from: t.request_id,
                },
            );
        }
    }

    /// The command was reaped: a chat run that ended without an `awaiting`
    /// clears its attempt's row; a chat act's outcome waits for the first
    /// read at or after this reap (§3.3).
    pub(super) fn chat_reaped(&mut self, status: ExitStatus) {
        use std::os::unix::process::ExitStatusExt;
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let chat_run = run.argv.iter().any(|a| a == "--requester=chat");
        if chat_run && run.collect.awaiting.is_none() {
            for a in [run.collect.attempt.as_ref(), run.pending.attempt.as_ref()]
                .into_iter()
                .flatten()
            {
                self.asks.table.remove(a);
            }
        }
        if let Some(tag) = run.pending.chat.as_deref().cloned() {
            let response = tag
                .key
                .as_ref()
                .zip(run.pending.unit.as_ref())
                .map(|(k, u)| self.chat_response(u, k));
            self.asks.outcome.push(Owed {
                tag,
                label: run.narrator.label.clone(),
                collect: run.collect.clone(),
                exit: status.code(),
                signal: status.signal().map(signal_name),
                response,
            });
        }
    }

    /// A read at or after the reap finished (`failed` says why it did not
    /// land): the owed outcome goes to the chat — or, its request gone, to
    /// the chat's next message, or to the transcript only (§3.3).
    pub(super) fn chat_after_read(&mut self, failed: Option<&str>) {
        for owed in std::mem::take(&mut self.asks.outcome) {
            self.chat_owed(owed, failed);
        }
    }

    fn chat_owed(&mut self, owed: Owed, failed: Option<&str>) {
        let (text, line, tone) = outcome_message(&owed, failed);
        self.chat_line(tone, line);
        // While a Stop is on its way the request is being cancelled: an
        // answer now would be lost with it (review PRO-6).
        if !self.chat.stopping() && self.chat.deny(&owed.tag.request_id, &text) {
            return;
        }
        if owed.tag.gen == self.chat.gen && self.chat.alive() {
            self.asks.unsent.push((owed.tag.gen, text));
            self.chat_line(
                T::Dim,
                "the chat will hear the outcome with your next message",
            );
        } else {
            self.chat_line(
                T::Dim,
                "the chat that asked for it has ended; it is not told",
            );
        }
    }

    /// The person cancelled the running command: a chat Continue running
    /// under the permission ends it (§3.4).
    pub(super) fn chat_cancelled(&mut self) {
        let Some(run) = self.run.as_ref().filter(|_| self.running) else {
            return;
        };
        // The grant act stopped by the person gives no permission — an
        // `awaiting` still in its pipe included (fix check N1), and one it
        // gave already is taken back (fix check 2, finding 2).
        if let Some(t) = run.pending.chat.as_ref().filter(|t| t.grant) {
            self.asks.epoch += 1;
            let (gen, from) = (t.gen, t.request_id.clone());
            let taken: Vec<String> = self
                .asks
                .permits
                .iter()
                .filter(|(_, p)| p.gen == gen && p.from == from)
                .map(|(a, _)| a.clone())
                .collect();
            for a in taken {
                self.asks.end_permit(&a, "you stopped the act that gave it");
            }
        }
        if let Some(a) = run
            .pending
            .attempt
            .clone()
            .filter(|_| run.pending.act == Act::Continue)
        {
            self.asks.end_permit(&a, "you stopped a Continue of it");
        }
    }

    /// The loop's step for a waiting Continue (§3.4): it runs once nothing
    /// modal is open, no command runs, and the person has not pressed a key
    /// or a button for a second — its confirm checks first.
    pub fn chat_step(&mut self, now: Instant) -> Command {
        if self.asks.waiting.is_none() || self.chat_waits_why(now).is_some() {
            return Command::None;
        }
        let Some(r) = self.asks.waiting.take() else {
            return Command::None;
        };
        let tag = r.tag().clone();
        let permitted = r
            .pending
            .attempt
            .as_ref()
            .and_then(|a| self.asks.permits.get(a))
            .is_some_and(|pm| pm.gen == tag.gen && pm.model == tag.model)
            && !self.config.allow_unsandboxed;
        if !permitted {
            // The permission ended meanwhile: it asks instead, saying why
            // (review USE-15).
            let ended = r
                .pending
                .attempt
                .as_ref()
                .and_then(|a| self.asks.ended.get(a));
            let why = match ended {
                Some(w) => format!("the permission ended ({w})"),
                None => "the permission ended".to_string(),
            };
            self.asks.waiting = Some(r);
            self.chat_asks_instead(now, &why);
            return Command::None;
        }
        self.refresh_holder();
        let gate = match self.busy() {
            Some(why) => Err(why),
            None => crate::preflight::preflight(&self.config.target)
                .map_err(|why| format!("the project cannot be read safely: {why}"))
                .and_then(|()| self.chat_gate(&r.pending, &tag)),
        };
        match gate {
            Ok(()) => Command::Spawn(r.pending),
            Err(why) => {
                self.chat.deny(&tag.request_id, &refused(&why));
                self.chat_line(T::Bad, format!("refused: {why}"));
                self.asks.meaning += 1;
                Command::None
            }
        }
    }

    /// Why the waiting Continue waits now, in words (the activity row says
    /// it): a menu, a dialog or a note open, a command running, the
    /// person's last key or press within a second. Read-only overlays do
    /// not hold it.
    pub fn chat_waits_why(&self, now: Instant) -> Option<&'static str> {
        self.asks.waiting.as_ref()?;
        // A request line shown owns Esc: the Continue waits behind it, so
        // it is never run unseen (review SAF-6).
        if self.asks.shown().is_some() {
            return Some("the chat's migration continues when you answer its request");
        }
        // Earlier outcomes reach the chat first (review PRO-4).
        if !self.asks.outcome.is_empty() {
            return Some("the chat's migration continues in a moment");
        }
        match &self.mode {
            Mode::Menu(_) => return Some("the chat's migration continues when you close the menu"),
            Mode::Dialog(_) => {
                return Some("the chat's migration continues when you close the dialog")
            }
            Mode::Note { .. } | Mode::EditNote { .. } => {
                return Some("the chat's migration continues when you finish the note")
            }
            _ => {}
        }
        if self.running {
            return Some("the chat's migration continues after the running command");
        }
        if self
            .asks
            .last_press
            .is_some_and(|t| now.saturating_duration_since(t) < CONTINUE_QUIET)
        {
            return Some("the chat's migration continues in a moment");
        }
        None
    }

    // ----- the chat's keys ------------------------------------------------------------

    /// Scroll the transcript by `by` rows.
    pub(super) fn scroll_chat(&mut self, by: isize) {
        let (w, h) = self.asks.transcript_size;
        self.chat.transcript.scroll_by(by, w.max(1), h.max(1));
    }

    /// The next (or previous) pane: Files → View → Chat → Files (the chat
    /// only when it exists).
    pub(super) fn next_focus(&self, forward: bool) -> Focus {
        let order: &[Focus] = if self.chat_on {
            &[Focus::Files, Focus::View, Focus::Chat]
        } else {
            &[Focus::Files, Focus::View]
        };
        let at = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        order[if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }]
    }

    /// Move the focus: leaving the chat raises the typing guard when a
    /// draft exists or typing was recent (§5.4); entering it drops it.
    pub(super) fn move_focus(&mut self, to: Focus) {
        if self.focus == Focus::Chat && to != Focus::Chat {
            self.asks
                .leaving_chat(self.now, !self.chat.input.is_empty());
        }
        if to == Focus::Chat {
            self.asks.guard = false;
            self.chat.unseen = false;
            // Opened once, the chat keeps its column from 156 columns.
            self.chat_column = true;
        }
        self.focus = to;
    }

    /// The key that brings the focus back to the chat from here.
    fn key_back(&self) -> &'static str {
        match self.focus {
            Focus::Files => "Shift-Tab",
            _ => "Tab",
        }
    }

    /// The typing guard (§5.4): while it is up and the panes have the focus,
    /// letters and `Enter` — and whole bursts — are dropped, until a
    /// navigation key (which ends it and acts) or a click.
    pub(super) fn guarded(&mut self, key: KeyEvent) -> bool {
        if !self.asks.guard
            || self.focus == Focus::Chat
            || !matches!(self.mode, Mode::Normal | Mode::Details { .. })
        {
            return false;
        }
        let navigation = matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Esc
        );
        if navigation && !self.asks.burst {
            self.asks.guard = false;
            return false;
        }
        let text = matches!(key.code, KeyCode::Char(_) | KeyCode::Enter);
        if text || self.asks.burst {
            let back = self.key_back();
            self.say(format!(
                "you left the chat — letters here are commands; {back} or click Chat to type"
            ));
            return true;
        }
        false
    }

    /// A key in the chat (§5.4): letters are text.
    pub(super) fn chat_key(&mut self, key: KeyEvent, now: Instant) -> Command {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let burst = self.asks.burst;
        let (w, h) = self.asks.transcript_size;
        let page = h.max(2) as isize - 1;
        let typed = |app: &mut App| {
            app.asks.typed_at = Some(now);
        };
        match key.code {
            KeyCode::Char('c') if ctrl => {
                // Once a Stop is on its way, Ctrl-C moves on: a runtime
                // that never answers it never keeps the person in.
                if self.chat.turn && !self.chat.stopping() {
                    self.chat_stop();
                } else if !self.chat.input.is_empty() {
                    self.chat.input.take();
                    self.say("the draft is cleared");
                } else {
                    // In the chat Ctrl-C's quit is always asked (§5.4): a
                    // second press after a clear never quits at once
                    // (review USE-16).
                    self.open_dialog(Purpose::Quit);
                }
            }
            KeyCode::Char('x') if ctrl => {
                if self.running {
                    self.open_dialog(Purpose::Cancel);
                } else {
                    self.say("nothing is running");
                }
            }
            KeyCode::Char('n') if ctrl => self.chat_ask_new(),
            KeyCode::Char('j') if ctrl => {
                self.chat.input.insert("\n");
                typed(self);
            }
            KeyCode::Enter if alt || burst => {
                // A line break: Alt-Enter, or an Enter inside a burst (a
                // paste without bracketed paste) — never a send.
                self.chat.input.insert("\n");
                typed(self);
            }
            KeyCode::Enter => {
                let input = &mut self.chat.input;
                if input.cursor == input.text.len() && input.text.ends_with('\\') {
                    input.backspace();
                    input.insert("\n");
                    typed(self);
                } else if !self.chat.input.is_empty() {
                    self.chat_send(now);
                } else if self.asks.shown().is_some() {
                    self.chat_review(now);
                }
            }
            KeyCode::Esc => {
                if self.asks.shown().is_some() {
                    self.chat_decline(now, false);
                } else if self.asks.waiting.is_some() {
                    self.chat_hold(now);
                } else if self.chat.turn && self.chat.stopping() {
                    self.say("stopping… — the reply ends in a moment");
                } else if self.chat.turn && !self.chat_act_running() {
                    self.chat_stop();
                } else {
                    self.say("Esc stays in the chat — Tab leaves it");
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let next = self.next_focus(key.code == KeyCode::Tab);
                self.move_focus(next);
            }
            KeyCode::F(1) => self.mode = Mode::Help { scroll: 0 },
            KeyCode::Up => {
                if !self.chat.input.up() {
                    self.chat.transcript.scroll_by(-1, w.max(1), h.max(1));
                }
            }
            KeyCode::Down => {
                if !self.chat.input.down() {
                    self.chat.transcript.scroll_by(1, w.max(1), h.max(1));
                }
            }
            KeyCode::PageUp => self.chat.transcript.scroll_by(-page, w.max(1), h.max(1)),
            KeyCode::PageDown => self.chat.transcript.scroll_by(page, w.max(1), h.max(1)),
            KeyCode::Home if self.chat.input.is_empty() => self.chat.transcript.to_top(w.max(1)),
            KeyCode::End if self.chat.input.is_empty() => self.chat.transcript.follow(),
            KeyCode::Home => self.chat.input.home(),
            KeyCode::End => self.chat.input.end(),
            KeyCode::Left => self.chat.input.left(),
            KeyCode::Right => self.chat.input.right(),
            KeyCode::Backspace => {
                self.chat.input.backspace();
                typed(self);
            }
            KeyCode::Delete => {
                self.chat.input.delete();
                typed(self);
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                let dropped = self.chat.input.insert(&c.to_string());
                if dropped > 0 {
                    self.say("the draft is full");
                }
                typed(self);
            }
            _ => {}
        }
        Command::None
    }

    /// A paste into the chat: its text, line breaks kept, bounded.
    pub(crate) fn chat_paste(&mut self, text: &str, now: Instant) {
        let dropped = self.chat.input.paste(text);
        self.asks.typed_at = Some(now);
        self.asks.last_press = Some(now);
        if dropped > 0 {
            self.say(format!("the paste did not fit: {dropped} bytes dropped"));
        }
    }

    fn chat_act_running(&self) -> bool {
        self.running && self.run.as_ref().is_some_and(|r| r.pending.chat.is_some())
    }

    /// Stop the chat's turn: every permission of it ends (§3.5).
    pub(crate) fn chat_stop(&mut self) {
        if self.chat.stop() {
            self.asks.end_permits(self.chat.gen, "you stopped the chat");
        }
    }

    /// `Ctrl-N` or `[New]`: New chat, asked when a conversation exists.
    pub(crate) fn chat_ask_new(&mut self) {
        if self.chat.has_conversation() {
            self.open_dialog(Purpose::NewChat);
        } else {
            self.say("no conversation yet — type a message to start one");
        }
    }

    /// New chat (confirmed): the old one ends; its requests and
    /// permissions with it.
    pub(super) fn chat_new_chat(&mut self) {
        let now = self.now;
        if let Some(ChatEvent::Ended { gen }) = self.chat.new_chat(now) {
            self.chat_gen_ended(gen, now);
        }
    }

    /// Send the draft, with the context block first (and any outcome this
    /// chat was not told).
    pub(crate) fn chat_send(&mut self, now: Instant) {
        if let Err(why) = &self.chat.bins {
            let why = why.clone();
            self.say(why);
            return;
        }
        // The end-to-end tests' panic (debug builds only, armed by the
        // environment): a message sent while a chat runs panics the loop, so
        // the panic hook must end the chat's group.
        #[cfg(debug_assertions)]
        if self.chat.alive()
            && std::env::var_os("HARNESS_TUI_TEST_PANIC").is_some_and(|v| v == "chat-send")
        {
            panic!("the test trigger: a panic while sending a chat message");
        }
        let text = self.chat.input.take();
        let context = self.context_block();
        match self.chat.send(&text, Some(&context), now) {
            Ok(()) => {
                let gen = self.chat.gen;
                self.asks.unsent.retain(|(g, _)| *g != gen);
                self.asks.typed_at = None;
            }
            Err(why) => {
                self.chat.input.insert(&text);
                self.say(why);
            }
        }
    }

    /// The context block (§5.3): what is selected, harness-shaped only, and
    /// the outcomes this chat was not told.
    pub fn context_block(&self) -> String {
        let mut out = format!("[cockpit context] About: {}.", self.about());
        for (gen, text) in &self.asks.unsent {
            if *gen == self.chat.gen && self.chat.alive() {
                out.push_str(&format!(
                    "\n[cockpit context] While you were stopped, an act you asked for ran: {text}"
                ));
            }
        }
        out
    }

    /// The selection in words, harness-shaped only: its kind, a unit or
    /// attempt id of their grammar, a repo path of `[A-Za-z0-9._/-]` (1–200
    /// characters) or none, and the state word.
    pub fn about(&self) -> String {
        let path = |p: &str| {
            let ok = (1..=200).contains(&p.len())
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
            if ok {
                p.to_string()
            } else {
                "a file whose name is not shown".to_string()
            }
        };
        let unit_word = |id: &str| -> String {
            match self.snapshot.units.iter().position(|u| u.unit.id == id) {
                Some(i) if harness_core::plan::is_clean_segment(id) => {
                    let state = self.files.units.get(i).map(|x| x.state.word());
                    match state {
                        Some(s) => format!("the unit {id} ({s})"),
                        None => format!("the unit {id}"),
                    }
                }
                _ => "a unit whose id is not shown".into(),
            }
        };
        match &self.selection {
            Selection::Project => "the project".into(),
            Selection::Dir(p) => format!("the directory {}", path(p)),
            Selection::File(p) => format!("the C file {}", path(p)),
            Selection::Function(p, _) => format!("a function in {}", path(p)),
            Selection::Units => "the list of units".into(),
            Selection::Features => "the list of the person's features".into(),
            // Ids only: a feature's name is the person's text, never the
            // chat's (docs/FEATURES-DESIGN.md §2.3).
            Selection::Feature(id) if harness_core::features::is_id(id) => {
                format!("the feature {id}")
            }
            Selection::Feature(_) => "a feature".into(),
            Selection::Speed => "the speed of the C against the Rust".into(),
            Selection::Unit(id) => unit_word(id),
            Selection::Crate(id) => format!("the crate of {}", unit_word(id)),
            Selection::Attempt(u, a) => {
                let outcome = self
                    .snapshot
                    .unit(u)
                    .and_then(|x| x.attempt(a))
                    .map(|x| x.record.outcome.clone())
                    .filter(|o| fence::OUTCOMES.contains(&o.as_str()));
                let id = if fence::is_attempt_id(a) {
                    a.clone()
                } else {
                    "an attempt".into()
                };
                match outcome {
                    Some(o) => format!("the attempt {id} ({o}) of {}", unit_word(u)),
                    None => format!("the attempt {id} of {}", unit_word(u)),
                }
            }
        }
    }

    /// The menu's "Migrate — ask in chat": the request typed into the input
    /// (never over a draft), the chat focused.
    pub(super) fn chat_migrate_item(&mut self) {
        if !self.chat.input.is_empty() {
            self.say("the chat has a draft — send or clear it first");
            self.move_focus(Focus::Chat);
            return;
        }
        let text = match &self.selection {
            Selection::Unit(id) => format!("Migrate {id}"),
            _ => "Migrate the next planned unit".to_string(),
        };
        self.chat.input.insert(&text);
        self.move_focus(Focus::Chat);
    }

    /// A click on one of the chat's keys (§5.5).
    pub(super) fn press_chat(&mut self, k: &str, now: Instant) -> Command {
        match k {
            "review" => {
                self.move_focus(Focus::Chat);
                self.chat_review(now);
            }
            "decline" => self.chat_decline(now, false),
            "decline-draft" => self.chat_decline(now, true),
            "hold" => self.chat_hold(now),
            "stop" => self.chat_stop(),
            "new" => {
                self.move_focus(Focus::Chat);
                self.chat_ask_new();
            }
            "close" => {
                self.chat_column = false;
                if self.focus == Focus::Chat {
                    self.move_focus(Focus::View);
                }
            }
            "help" => self.mode = Mode::Help { scroll: 0 },
            "tab-view" => self.move_focus(Focus::View),
            "tab-chat" => {
                self.chat_column = true;
                self.move_focus(Focus::Chat);
            }
            _ => {}
        }
        Command::None
    }

    /// A click on a hint or activity entry while the chat has the focus:
    /// what its key does in the chat — never a letter typed into the input
    /// (§5.4: `[Details]` opens the details, `[Try again]` its dialog,
    /// `Ctrl-C` asks first).
    pub(super) fn press_chat_hint(&mut self, k: &str, now: Instant) -> Command {
        match k {
            "c" => {
                self.mode = Mode::Details {
                    scroll: usize::MAX / 2,
                };
                Command::None
            }
            "t" => match self.try_again.clone() {
                Some(p) if !self.running => {
                    self.ask(p);
                    Command::None
                }
                _ => {
                    self.say("nothing to try again");
                    Command::None
                }
            },
            "x" | "Ctrl-X" => {
                if self.running {
                    self.open_dialog(Purpose::Cancel);
                }
                Command::None
            }
            // What the key does now, a quit asked (review USE-7).
            "Ctrl-C" => {
                if self.chat.turn && !self.chat.stopping() {
                    self.chat_stop();
                } else if !self.chat.input.is_empty() {
                    self.chat.input.take();
                    self.say("the draft is cleared");
                } else {
                    self.open_dialog(Purpose::Quit);
                }
                Command::None
            }
            "q" => {
                self.open_dialog(Purpose::Quit);
                Command::None
            }
            "F1" | "?" => {
                self.mode = Mode::Help { scroll: 0 };
                Command::None
            }
            "Ctrl-N" => {
                self.chat_ask_new();
                Command::None
            }
            "Enter" => self.chat_key(KeyEvent::from(KeyCode::Enter), now),
            "Esc" => self.chat_key(KeyEvent::from(KeyCode::Esc), now),
            "Tab" => self.chat_key(KeyEvent::from(KeyCode::Tab), now),
            "Ctrl-J" => {
                self.chat.input.insert("\n");
                self.asks.typed_at = Some(now);
                Command::None
            }
            _ => Command::None,
        }
    }
}

/// The input a request's act was built from (Review rebuilds it fresh).
fn request_input(p: &Pending) -> Value {
    let mut v = json!({"unit": p.unit.clone().unwrap_or_default()});
    match p.act {
        Act::Modify => {
            v["from"] = json!(p.attempt.clone().unwrap_or_default());
            v["steer"] = json!(p.note.clone().unwrap_or_default());
        }
        Act::Retry => v["attempt"] = json!(p.attempt.clone().unwrap_or_default()),
        _ => {}
    }
    v
}

/// The chat's shape of a Modify argv: `--model=` the chat's when the
/// provider is `external` (it answers), and the label.
fn chat_shape(argv: &mut Vec<OsString>, model: &str, external: bool) {
    if external {
        for a in argv.iter_mut() {
            if a.to_string_lossy().starts_with("--model=") {
                *a = os(format!("--model={model}"));
            }
        }
    }
    let at = argv
        .iter()
        .position(|a| a == "--allow-unsandboxed")
        .unwrap_or(argv.len());
    argv.insert(at, os("--requester=chat"));
}

/// "No key held" (§3.4): the act that resumes the attempt and re-poses its
/// request, from its record.
fn no_key_held(r: &AttemptRecord) -> String {
    let sample = harness_core::attempts::sample_base(&r.id) != r.id;
    let act = match (&r.seeded_from, &r.steer_note) {
        _ if sample => format!("ask for Retry of {}", r.id),
        (Some(seed), Some(note)) => format!(
            "ask for Modify from {seed} with its recorded note {}",
            json!(fence::untrusted("note", note, fence::STEER_NOTE_CAP))
        ),
        _ => format!("ask for Migrate of {} again", r.unit),
    };
    format!(
        "no answer is expected here for attempt {}: the cockpit holds no request key for it in \
         this chat — {act}; it resumes the attempt and poses the request again",
        r.id
    )
}

/// The closed outcome word of a run (§3.3).
fn outcome_word(o: &Owed) -> &'static str {
    let c = &o.collect;
    if o.signal.is_some() || c.error.as_ref().is_some_and(|(k, _)| k == "interrupted") {
        return "interrupted";
    }
    if c.awaiting.is_some() {
        return "awaiting";
    }
    match c.outcome.as_deref() {
        Some("green") => return "green",
        Some("red") => return "red",
        _ => {}
    }
    match (o.exit, &c.error) {
        (Some(0), _) => "done",
        (Some(1), Some(_)) => "refused",
        _ => "failed",
    }
}

/// The message the chat reads as the act's result (§3.3): who ran it, then
/// the outcome, every ledger- or CLI-derived string fenced, filled item by
/// item to at most [`MAX_OUTCOME_BYTES`] (whole items dropped, `omitted`
/// naming them) — and its transcript line.
fn outcome_message(o: &Owed, failed_read: Option<&str>) -> (String, String, T) {
    let who = if o.tag.permitted {
        "Continued by the cockpit under the permission the person gave when they ran the \
         migration"
    } else {
        "Ran by the cockpit after the person confirmed it"
    };
    let word = outcome_word(o);
    let c = &o.collect;
    let act = match o.tag.tool.as_str() {
        "harness_migrate" => "migrate",
        "harness_steer" => "steer",
        "harness_retry" => "retry",
        _ => "continue",
    };
    let mut body = json!({
        "act": act,
        "outcome": word,
        "exit": o.exit,
        "attempt": c.attempt.as_deref().map_or(Value::Null, fence::attempt),
        "awaiting": c.awaiting.as_ref().map_or(Value::Null, |(a, k)| json!({
            "attempt": a.as_deref().map_or(Value::Null, fence::attempt),
            "request_key": k,
        })),
        "answer_filed": o.response.as_ref().map(|r| response_present(r)),
        "failed_checks": [],
        "messages": [],
        "omitted": Value::Null,
    });
    if let Some(sig) = &o.signal {
        body["signal"] = json!(sig);
    }
    if let Some((kind, message)) = &c.error {
        body["error"] = json!({
            "kind": fence::closed("error kind", kind, fence::ERROR_KINDS),
            "message": fence::untrusted("message", message, fence::MESSAGE_CAP),
        });
    }
    if let Some(e) = failed_read {
        body["read"] = fence::untrusted(
            "cockpit-reason",
            &format!("the ledger could not be re-read: {e}"),
            fence::MESSAGE_CAP,
        );
    }
    if word == "awaiting" {
        body["next"] = json!(
            "read the request whole with harness_request, then answer it with harness_answer"
        );
    }
    let mut omitted = BTreeMap::new();
    let size = |v: &Value| v.to_string().len() + who.len() + 2;
    for (key, items) in [
        ("failed_checks", c.failed.clone()),
        ("messages", c.messages.clone()),
    ] {
        for (i, text) in items.iter().enumerate() {
            let item = fence::untrusted(
                if key == "messages" {
                    "message"
                } else {
                    "check"
                },
                text,
                fence::MESSAGE_CAP,
            );
            let mut next = body.clone();
            if let Some(list) = next[key].as_array_mut() {
                list.push(item);
            }
            if size(&next) + 64 > MAX_OUTCOME_BYTES {
                omitted.insert(key, items.len() - i);
                break;
            }
            body = next;
        }
    }
    if !omitted.is_empty() {
        body["omitted"] = json!(omitted);
    }
    let text = format!("{who}: {}", fence::ordered_text(&body));
    let checks = if c.checks > 0 {
        format!(
            ", {} of {} checks passed",
            c.checks - c.failed.len(),
            c.checks
        )
    } else {
        String::new()
    };
    let (line, tone) = match word {
        "green" => (format!("✓ {} — GREEN{checks}", o.label), T::Good),
        "red" => (format!("✗ {} — RED{checks}", o.label), T::Bad),
        "awaiting" => (
            format!(
                "{} — awaiting the chat's answer to turn {}",
                o.label,
                c.turn.map_or("?".into(), |t| t.to_string())
            ),
            T::Warn,
        ),
        "refused" => (
            format!(
                "refused: {}",
                c.error.as_ref().map_or("", |(_, m)| m.as_str())
            ),
            T::Bad,
        ),
        "interrupted" => (format!("{} — stopped", o.label), T::Bad),
        "done" => (format!("✓ {} — done", o.label), T::Good),
        _ => (
            format!(
                "✗ {} — failed (exit {})",
                o.label,
                o.exit.map_or("?".into(), |e| e.to_string())
            ),
            T::Bad,
        ),
    };
    (text, line, tone)
}

#[cfg(test)]
mod tests;
