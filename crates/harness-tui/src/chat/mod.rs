//! The cockpit's chat (docs/CHAT-PANE-DESIGN.md): Claude Code, run headless
//! as a child of the cockpit, reads the ledger through harness-mcp in cockpit
//! mode and ASKS for model work; the person confirms each act in the
//! cockpit's own armed dialog and the cockpit runs it (the app's side,
//! `app::chat`). This module is the chat itself:
//!
//! - [`runtime`]: the child process — resolved binaries, the environment,
//!   the private directory and its sweep, the readers and the writer, the end
//!   routine every way out goes through;
//! - [`stream`]: the stream-JSON protocol, both ways;
//! - [`transcript`], [`input`]: what the pane shows and what the person types;
//! - [`Chat`]: one conversation after another (each process a generation):
//!   started lazily on the first message, every `init` checked, reads allowed
//!   at once, every act handed to the app as an [`Event::Ask`] and held —
//!   never allowed — until the app answers it, the runtime's own markers and
//!   echoes told apart from a message the cockpit did not send.

pub mod input;
pub mod runtime;
pub mod stream;
pub mod transcript;

use input::Input;
use runtime::{Binaries, Ending, Out, Procs, Runtime};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use stream::{Block, In};
use transcript::{Tone, Transcript};

/// The brief the chat follows (§6), given with `--append-system-prompt`.
pub const BRIEF: &str = include_str!("../chat_brief.md");
/// No `init` this long after the start: said, and New chat offered.
pub const START_WAIT: Duration = Duration::from_secs(30);
/// A turn that owes output and is silent this long: said, Stop offered.
pub const QUIET: Duration = Duration::from_secs(60);
/// How often a live runtime's leader is looked at (a death EOF may not
/// report).
const LEADER_PROBE: Duration = Duration::from_secs(2);
/// How long the pipes of a leader seen exited are still read.
const LEADER_GRACE: Duration = Duration::from_millis(500);
/// The Claude Code release the protocol was verified with.
pub const TESTED_VERSION: &str = "2.1.274";
/// The tools harness-mcp's cockpit mode offers, by their runtime names.
pub const COCKPIT_TOOLS: &[&str] = &[
    "mcp__harness__harness_status",
    "mcp__harness__harness_unit",
    "mcp__harness__harness_request",
    "mcp__harness__harness_migrate",
    "mcp__harness__harness_gen_driver",
    "mcp__harness__harness_steer",
    "mcp__harness__harness_retry",
    "mcp__harness__harness_answer",
    "mcp__harness__harness_map_questions",
    "mcp__harness__harness_map_answer",
    "mcp__harness__harness_features_propose",
    "mcp__harness__harness_workloads_propose",
    "mcp__harness__harness_features_file",
    "mcp__harness__harness_workloads_file",
];

/// Why the chat cannot be used now (docs/CHAT-PANE-DESIGN.md §1.6): the
/// connection dialog's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// `claude` is not found, or `claude --version` fails.
    NotInstalled,
    /// Claude Code started and said it is not signed in.
    NotSignedIn,
    /// Claude Code ended by itself, or the cockpit refused what it started
    /// with.
    Stopped,
    /// harness-mcp started, but lacks a tool the cockpit expects: it is an
    /// older build than the cockpit (`cargo run -p harness-tui` builds the
    /// cockpit alone). The fix is a rebuild, not a sign-in.
    OldTools,
}

/// The connection's problem and what the cockpit saw of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Its case.
    pub trouble: Trouble,
    /// What was seen: the runtime's or the check's own words, one line.
    pub seen: String,
    /// The one line that fixes it, when it is a command to paste (the
    /// rebuild of an old harness-mcp); the dialog's step 2.
    pub fix: Option<String>,
}

impl Problem {
    fn new(trouble: Trouble, seen: &str) -> Problem {
        let line = harness_core::text::safe_line(seen.trim());
        Problem {
            trouble,
            seen: line.chars().take(300).collect(),
            fix: None,
        }
    }

    /// An old harness-mcp: what it lacks, and the line that rebuilds it.
    fn old_tools(mcp: &Path, missing: &str) -> Problem {
        let short = missing.strip_prefix("mcp__harness__").unwrap_or(missing);
        let mut p = Problem::new(
            Trouble::OldTools,
            &format!("harness-mcp at {} lacks {short}", mcp.display()),
        );
        p.fix = Some(rebuild_line(mcp));
        p
    }

    /// The chat pane's line when the chat is ended for this problem.
    pub fn said(&self) -> String {
        match (&self.trouble, &self.fix) {
            (Trouble::OldTools, Some(fix)) => format!(
                "the chat's harness tools are out of date: {} — rebuild it ({fix}), then \
                 Check again — the chat is off",
                self.seen
            ),
            _ => self.seen.clone(),
        }
    }

    /// The dialog's title, in plain words.
    pub fn title(&self) -> &'static str {
        match self.trouble {
            Trouble::NotInstalled => "Claude Code is not installed",
            Trouble::NotSignedIn => "Claude Code is not signed in",
            Trouble::Stopped => "Claude Code stopped",
            Trouble::OldTools => "The chat's harness tools are out of date",
        }
    }

    /// Why the chat's items are greyed, and the way to the walk-through.
    pub fn reason(&self) -> String {
        format!(
            "{} — choose Check the chat's connection to see how to fix it",
            self.title()
        )
    }

    /// The dialog's body: what the chat needs, the steps, what was seen.
    pub fn body(&self) -> Vec<String> {
        let lead = match self.trouble {
            Trouble::NotInstalled => {
                "The chat (and every \"ask in chat\" item) runs Claude Code, and the cockpit \
                 could not run it. Install Claude Code first, then:"
            }
            Trouble::NotSignedIn => {
                "The chat (and every \"ask in chat\" item) runs Claude Code, and it says it is \
                 not signed in:"
            }
            Trouble::Stopped => {
                "The chat's Claude Code stopped (what it said is below). When it is the \
                 sign-in:"
            }
            Trouble::OldTools => {
                let fix = self.fix.as_deref().unwrap_or("cargo build -p harness-mcp");
                return vec![
                    "The chat reads the project through harness-mcp, and the one the cockpit \
                     found is older than the cockpit: it lacks a tool the chat needs. Rebuild \
                     it:"
                    .into(),
                    "1. Open a new Terminal window.".into(),
                    format!("2. Paste this line and press Return: {fix}"),
                    "3. Wait for it to finish (about a minute).".into(),
                    "4. Come back here and choose Check again.".into(),
                    format!("What the cockpit saw: {}", self.seen),
                ];
            }
        };
        vec![
            lead.to_string(),
            "1. Open a new Terminal window.".into(),
            "2. Type claude and press Return.".into(),
            "3. Follow the sign-in it shows.".into(),
            "4. Come back here and choose Check again.".into(),
            format!("What the cockpit saw: {}", self.seen),
        ]
    }
}

/// The line that rebuilds harness-mcp where it was found: a workspace
/// build (`…/target/debug/harness-mcp`) by `cargo build` in that checkout,
/// anything else by `cargo install` from the person's RuHarness folder.
pub fn rebuild_line(mcp: &Path) -> String {
    let mut parts = mcp.components().rev();
    let name = parts.next();
    let profile = parts.next().and_then(|c| match c.as_os_str().to_str() {
        Some("debug") => Some(""),
        Some("release") => Some(" --release"),
        _ => None,
    });
    let target = parts.next().is_some_and(|c| c.as_os_str() == "target");
    if let (Some(_), Some(profile), true) = (name, profile, target) {
        if let Some(repo) = mcp.ancestors().nth(3) {
            return harness_core::text::safe_line(&format!(
                "cd {} && cargo build{profile} -p harness-mcp",
                repo.display()
            ));
        }
    }
    "cd <your RuHarness folder> && cargo install --locked --path crates/harness-mcp".into()
}

/// How the binaries were found, to look again on Check again.
#[derive(Debug, Clone, Default)]
pub struct Resolver {
    /// `--chat-runtime`.
    pub runtime: Option<PathBuf>,
    /// `--harness-mcp`.
    pub mcp: Option<PathBuf>,
    /// The cockpit's own binary.
    pub exe: Option<PathBuf>,
    /// `PATH`.
    pub path: Option<OsString>,
}

impl Resolver {
    /// The binaries, found again.
    pub fn resolve(&self) -> Result<Binaries, String> {
        runtime::resolve(
            self.runtime.as_deref(),
            self.mcp.as_deref(),
            self.exe.as_deref(),
            self.path.as_deref(),
        )
    }
}

/// `bins`' error says `claude` itself is missing (not harness-mcp).
pub fn claude_missing(why: &str) -> bool {
    why.starts_with("claude not found") || why.starts_with("--chat-runtime")
}
/// The acts the chat may ask for (short names).
pub const ACT_TOOLS: &[&str] = &[
    "harness_migrate",
    "harness_gen_driver",
    "harness_steer",
    "harness_retry",
    "harness_answer",
];

/// What the chat tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The chat asks for an act (`harness_migrate`, …): held until the app
    /// answers it with [`Chat::deny`].
    Ask {
        /// The chat's generation.
        gen: u64,
        /// The permission request's id.
        request_id: String,
        /// The call's id.
        tool_use_id: Option<String>,
        /// The tool's short name.
        tool: String,
        /// Its arguments.
        input: Value,
        /// The model of the message that made the call.
        model: Option<String>,
    },
    /// The runtime withdrew a held request.
    Withdrawn {
        /// Its generation.
        gen: u64,
        /// The request.
        request_id: String,
    },
    /// The chat read the last page of a pending request: it may answer it.
    Read {
        /// Its generation.
        gen: u64,
        /// The attempt.
        attempt: String,
        /// The request key.
        key: String,
    },
    /// A message the cockpit did not send arrived.
    Foreign {
        /// Its generation.
        gen: u64,
    },
    /// A generation ended (however): its requests are withdrawn, its
    /// permissions end.
    Ended {
        /// The generation.
        gen: u64,
    },
    /// Claude Code is not signed in, or stopped by itself: the connection
    /// dialog (docs/CHAT-PANE-DESIGN.md §1.6).
    Unavailable {
        /// What was seen.
        problem: Problem,
    },
    /// `harness_map_answer` recorded the chat's answer: the Project view
    /// reads the map's reply again.
    MapAnswered,
    /// `harness_features_propose` (or, `workloads`,
    /// `harness_workloads_propose`) wrote the chat's proposal
    /// (docs/COCKPIT-WRAPPER-DESIGN.md §9.6).
    ProposalWritten {
        /// The workloads file's proposal, not the features file's.
        workloads: bool,
    },
}

/// Where the chat is, for the pane's title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No message sent yet (or the last chat ended).
    Idle,
    /// Started, no `init` yet.
    Starting,
    /// A turn runs.
    Thinking,
    /// Between turns.
    Ready,
}

/// A read the chat made (its transcript line, and for a request its key).
#[derive(Debug, Clone)]
struct ReadCall {
    line: u64,
    words: String,
    request: Option<(String, String)>,
    /// Its place in the folded reads' line (§9.5), when it is a read.
    part: Option<usize>,
}

/// Consecutive reads folded into one line (docs/COCKPIT-WRAPPER-DESIGN.md
/// §9.5): "· read the project's status and unit u-x — done".
#[derive(Debug, Clone, Default)]
struct ReadGroup {
    /// Its transcript line.
    line: u64,
    /// What each read reads ("the project's status"), and how it ended:
    /// `None` running, `Some(true)` done, `Some(false)` failed.
    parts: Vec<(String, Option<bool>)>,
}

impl ReadGroup {
    /// The line's tone and words.
    fn line(&self) -> (Tone, String) {
        let names: Vec<String> = self.parts.iter().map(|(w, _)| w.clone()).collect();
        let read = match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            Some((last, _)) => last.clone(),
            None => String::new(),
        };
        let failed: Vec<&str> = self
            .parts
            .iter()
            .filter(|(_, s)| *s == Some(false))
            .map(|(w, _)| w.as_str())
            .collect();
        if !failed.is_empty() {
            let what = if failed.len() == self.parts.len() {
                "failed".to_string()
            } else {
                format!("{} failed", failed.join(" and "))
            };
            return (Tone::Bad, format!("· read {read} — {what}"));
        }
        if self.parts.iter().all(|(_, s)| *s == Some(true)) {
            (Tone::Dim, format!("· read {read} — done"))
        } else {
            (Tone::Dim, format!("· read {read}"))
        }
    }
}

/// The chat.
#[derive(Debug)]
pub struct Chat {
    /// The binaries, or why the chat is unavailable.
    pub bins: Result<Binaries, String>,
    /// The runtime's environment (chosen once, at start).
    pub env: Vec<(OsString, OsString)>,
    /// What it signs in with, in words.
    pub sign_in: String,
    /// `--chat-model`.
    pub model_flag: Option<String>,
    /// The target (canonical).
    pub target: PathBuf,
    /// The mapped tool the cockpit opened (`None`: a folder-form target).
    pub tool: Option<String>,
    /// Where chat directories are made.
    pub temp: PathBuf,
    /// Every chat process not yet reaped (shared with the signal path).
    pub procs: Procs,
    /// The current (or last) generation; 0 before the first.
    pub gen: u64,
    /// The live process.
    pub live: Option<Runtime>,
    /// Processes on their way out.
    pub ending: Vec<Ending>,
    /// What the pane shows.
    pub transcript: Transcript,
    /// What the person types.
    pub input: Input,
    /// The model the latest `init` named.
    pub model: Option<String>,
    /// The runtime's version.
    pub version: Option<String>,
    /// `apiKeySource` from the latest `init`.
    pub api_key_source: Option<String>,
    /// A turn is running.
    pub turn: bool,
    /// New output arrived while the pane was not looked at.
    pub unseen: bool,
    /// Requests of the live generation held for the app (by request id).
    pub held: BTreeSet<String>,
    /// Requests the chat read whole, in its generation: (gen, attempt, key).
    pub keys_read: BTreeSet<(u64, String, String)>,
    stopping: bool,
    saw_init: bool,
    started: Option<Instant>,
    start_warned: bool,
    last_out: Instant,
    quiet_warned: bool,
    sent: BTreeSet<String>,
    started_uuids: BTreeSet<String>,
    interrupts: BTreeSet<String>,
    calls: BTreeMap<String, (Option<String>, String)>,
    reads: BTreeMap<String, ReadCall>,
    read_group: Option<ReadGroup>,
    cur_msg: Option<String>,
    cur_block: Option<u64>,
    cost: f64,
    turn_started: Option<Instant>,
    unparsed_said: bool,
    serial: u64,
    last_error: Option<String>,
    probed: Instant,
    /// When the probe first saw the leader exited.
    exited_at: Option<Instant>,
    /// Why the last chat ended, for the title: by itself, or asked.
    pub ended_by_itself: bool,
    /// The last turn ended by a Stop (the title says "stopped").
    pub stopped_last: bool,
    /// Interrupt markers a Stop may still bring: kept until the turn it
    /// stopped ends aborted or the runtime is idle — one sent as a turn
    /// ended stops the turn queued behind it (fix check N4).
    markers: u32,
    /// A marker came in this turn.
    marked: bool,
    /// The cockpit's messages not yet echoed: a turn is queued while one is
    /// (Claude Code 2.1.274 says `queued_turn_count` 0 all the same — fix
    /// check 2, finding 1).
    unechoed: BTreeSet<String>,
    /// The new chat's line was said (New chat, before its first message).
    announced: bool,
    /// A message was sent in this chat (not an earlier one still on the
    /// screen): what `has_conversation` means once the process is gone.
    /// The transcript keeps every chat's cells, so it cannot say it (the
    /// person's loop, 2026-10-19: "start a new chat first" never ended).
    pub(crate) spoke: bool,
    /// Why the chat cannot be used now (the connection dialog's case).
    pub problem: Option<Problem>,
    /// `claude --version` answered since the last problem.
    pub installed: bool,
    /// How the binaries were found (Check again looks again).
    pub resolver: Option<Resolver>,
    /// The person's last message, sent again by Check again after a
    /// failed sign-in.
    pub last_sent: Option<String>,
    /// `harness_map_answer` calls allowed, by call id: their result tells
    /// the app to read the map's reply again.
    map_answers: BTreeSet<String>,
    /// A draft of the person's own is kept, of the features file and of
    /// the workloads file: the app sets it before each pump, and a
    /// proposal of that file is refused while it is (§9.6: the draft slot
    /// is theirs).
    pub drafts_kept: [bool; 2],
    /// Proposal calls allowed, by call id (`true`: the workloads file's):
    /// their result tells the app the proposal is written.
    proposals: BTreeMap<String, bool>,
}

/// The transcript's line between two chats.
const NEW_CHAT: &str = "— a new chat — it does not see the conversation above —";

/// A fresh UUID (version 4) from the system's randomness.
pub fn fresh_uuid() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        b = (t ^ (u128::from(std::process::id()) << 64)).to_le_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The first word of a model id, for the title (`claude-haiku-4-5-…` →
/// `haiku-4-5`).
pub fn short_model(model: &str) -> String {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    // Drop a trailing date stamp.
    match m.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => m.to_string(),
    }
}

/// A tool call's input value as a short plain string (for words).
fn arg(input: &Value, key: &str) -> String {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.chars().take(80).collect())
        .unwrap_or_default()
}

impl Chat {
    /// A chat over `target`, not started. `vars`: the cockpit's own
    /// environment.
    pub fn new(
        bins: Result<Binaries, String>,
        vars: &[(OsString, OsString)],
        model_flag: Option<String>,
        target: PathBuf,
        procs: Procs,
    ) -> Chat {
        let env = runtime::environment(vars);
        let sign_in = runtime::sign_in_words(&env);
        // No `claude` at all: the walk-through from the start.
        let problem = match &bins {
            Err(why) if claude_missing(why) => Some(Problem::new(Trouble::NotInstalled, why)),
            _ => None,
        };
        Chat {
            bins,
            env,
            sign_in,
            model_flag,
            target,
            tool: None,
            temp: std::env::temp_dir(),
            procs,
            gen: 0,
            live: None,
            ending: Vec::new(),
            transcript: Transcript::default(),
            input: Input::default(),
            model: None,
            version: None,
            api_key_source: None,
            turn: false,
            unseen: false,
            held: BTreeSet::new(),
            keys_read: BTreeSet::new(),
            stopping: false,
            saw_init: false,
            started: None,
            start_warned: false,
            last_out: Instant::now(),
            quiet_warned: false,
            sent: BTreeSet::new(),
            started_uuids: BTreeSet::new(),
            interrupts: BTreeSet::new(),
            calls: BTreeMap::new(),
            reads: BTreeMap::new(),
            read_group: None,
            cur_msg: None,
            cur_block: None,
            cost: 0.0,
            turn_started: None,
            unparsed_said: false,
            serial: 0,
            last_error: None,
            probed: Instant::now(),
            exited_at: None,
            ended_by_itself: false,
            stopped_last: false,
            markers: 0,
            marked: false,
            unechoed: BTreeSet::new(),
            announced: false,
            spoke: false,
            problem,
            installed: false,
            resolver: None,
            last_sent: None,
            map_answers: BTreeSet::new(),
            drafts_kept: [false; 2],
            proposals: BTreeMap::new(),
        }
    }

    /// The connection check before the chat starts (docs/CHAT-PANE-DESIGN.md
    /// §1.6): `claude` found and `claude --version` answering. A problem
    /// already seen stands until Check again ([`Chat::check_again`]). `None`
    /// when the chat may start; a harness-mcp that is missing is not Claude
    /// Code's problem — [`Chat::available`] says it.
    pub fn check(&mut self) -> Option<Problem> {
        if self.problem.is_some() || self.installed {
            return self.problem.clone();
        }
        let claude = match &self.bins {
            Ok(b) => b.claude.clone(),
            Err(why) if claude_missing(why) => {
                self.problem = Some(Problem::new(Trouble::NotInstalled, why));
                return self.problem.clone();
            }
            Err(_) => return None,
        };
        match runtime::version(&claude, &self.env, runtime::VERSION_WAIT) {
            Ok(_) => self.installed = true,
            Err(seen) => self.problem = Some(Problem::new(Trouble::NotInstalled, &seen)),
        }
        self.problem.clone()
    }

    /// Check again: the binaries found again, the problem forgotten, the
    /// check run anew. The sign-in shows only when Claude Code starts.
    pub fn check_again(&mut self) -> Option<Problem> {
        if let Some(r) = &self.resolver {
            self.bins = r.resolve();
        }
        self.problem = None;
        self.installed = false;
        self.check()
    }

    /// Note a problem seen in the stream (the first of a generation counts):
    /// the app's [`Event::Unavailable`].
    fn trouble(&mut self, trouble: Trouble, seen: &str, events: &mut Vec<Event>) {
        if self.problem.is_some() {
            return;
        }
        self.trouble_with(Problem::new(trouble, seen), events);
    }

    /// Note a problem already built (an old harness-mcp, with its fix).
    fn trouble_with(&mut self, p: Problem, events: &mut Vec<Event>) {
        if self.problem.is_some() {
            return;
        }
        self.problem = Some(p.clone());
        events.push(Event::Unavailable { problem: p });
    }

    /// The chat can be used (its binaries were found).
    pub fn available(&self) -> bool {
        self.bins.is_ok()
    }

    /// A process is live.
    pub fn alive(&self) -> bool {
        self.live.is_some()
    }

    /// Where it is.
    pub fn phase(&self) -> Phase {
        match (&self.live, self.saw_init, self.turn) {
            (None, _, _) => Phase::Idle,
            (Some(_), false, _) => Phase::Starting,
            (Some(_), true, true) => Phase::Thinking,
            (Some(_), true, false) => Phase::Ready,
        }
    }

    /// A conversation exists in this chat (Quit and New chat ask first; a
    /// move between the Project view and a tool asks too): the process is
    /// live, or a message was sent to it. An earlier chat's cells on the
    /// screen are not one.
    pub fn has_conversation(&self) -> bool {
        self.live.is_some() || self.spoke
    }

    /// A Stop was sent and the turn has not ended yet.
    pub fn stopping(&self) -> bool {
        self.stopping
    }

    /// The model owes output: a turn runs and nothing is held for the
    /// person or the cockpit (a held request or a running act is silent by
    /// design).
    pub fn owes_output(&self) -> bool {
        self.live.is_some() && self.turn && self.held.is_empty()
    }

    fn next_id(&mut self, what: &str) -> String {
        self.serial += 1;
        format!("cockpit-{what}-{}-{}", self.gen, self.serial)
    }

    fn say(&mut self, tone: Tone, text: impl Into<String>) -> u64 {
        self.unseen = true;
        self.transcript.line(tone, text)
    }

    /// Start a new process (the first message, or the first after an end).
    fn start(&mut self, now: Instant) -> Result<(), String> {
        let bins = self.bins.clone()?;
        let dir = runtime::create_dir(&self.temp)
            .map_err(|e| format!("the chat's directory could not be made: {e}"))?;
        // Dead cockpits' directories go, off the loop (§1.1, §R5) — in the
        // temp dir, and in /tmp where a long temp dir sends them.
        {
            use std::os::unix::fs::MetadataExt;
            let (temp, own) = (self.temp.clone(), dir.clone());
            let parent = dir.parent().map(PathBuf::from);
            if let Ok(uid) = std::fs::metadata(&dir).map(|m| m.uid()) {
                let _ = std::thread::Builder::new().spawn(move || {
                    runtime::sweep(&temp, uid, &own);
                    if let Some(p) = parent.filter(|p| *p != temp) {
                        runtime::sweep(&p, uid, &own);
                    }
                });
            }
        }
        self.gen += 1;
        let argv = runtime::argv(
            &bins,
            &dir,
            &self.target,
            self.tool.as_deref(),
            BRIEF,
            self.model_flag.as_deref(),
        );
        let rt = match Runtime::spawn(self.gen, argv, dir.clone(), &self.env, &self.procs) {
            Ok(rt) => rt,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(format!("claude could not be started: {e}"));
            }
        };
        let init = self.next_id("init");
        rt.send(stream::initialize(&init));
        self.live = Some(rt);
        self.saw_init = false;
        self.started = Some(now);
        self.start_warned = false;
        self.turn = false;
        self.stopping = false;
        self.markers = 0;
        self.marked = false;
        self.unechoed.clear();
        self.exited_at = None;
        self.held.clear();
        self.sent.clear();
        self.started_uuids.clear();
        self.calls.clear();
        self.reads.clear();
        self.read_group = None;
        self.cur_msg = None;
        self.cost = 0.0;
        self.model = None;
        self.say(
            Tone::Dim,
            format!("starting Claude Code (signed in with {})", self.sign_in),
        );
        Ok(())
    }

    /// Send the person's `text` (with the `context` block first): the chat
    /// starts if it is not running. `Err` says why nothing was sent.
    pub fn send(&mut self, text: &str, context: Option<&str>, now: Instant) -> Result<(), String> {
        if self.live.is_none() {
            // Said once — kept said if the start fails (fix check 2, 9).
            // Only after a conversation: never after a start that failed
            // before one (fix check 3, F5).
            if self.gen > 0 && !self.announced && self.has_conversation() {
                self.say(Tone::Cockpit, NEW_CHAT);
                self.announced = true;
            }
            self.start(now)?;
            self.announced = false;
        }
        let uuid = fresh_uuid();
        let line = stream::user_message(&uuid, context, text);
        let Some(rt) = self.live.as_ref() else {
            return Err("the chat is not running".into());
        };
        if !rt.send(line) {
            return Err("the chat is not taking messages".into());
        }
        self.sent.insert(uuid.clone());
        self.unechoed.insert(uuid.clone());
        self.last_sent = Some(text.to_string());
        self.spoke = true;
        self.transcript.you(&uuid, text);
        self.transcript.follow();
        if !self.turn {
            self.turn = true;
            self.turn_started = Some(now);
        }
        self.stopped_last = false;
        self.last_out = now;
        self.quiet_warned = false;
        Ok(())
    }

    /// Answer held request `request_id` with a denial carrying `message` —
    /// how every act's outcome, refusal or decline reaches the model.
    /// `false` when it is no longer held (withdrawn, or its process ended).
    pub fn deny(&mut self, request_id: &str, message: &str) -> bool {
        if !self.held.remove(request_id) {
            return false;
        }
        self.released();
        self.live
            .as_ref()
            .is_some_and(|rt| rt.send(stream::deny(request_id, message)))
    }

    /// The last held request was answered or withdrawn: the model owes
    /// output from now — the quiet clock starts here, not at its last line
    /// before the act (review PRO-1).
    fn released(&mut self) {
        if self.held.is_empty() {
            self.last_out = Instant::now();
            self.quiet_warned = false;
        }
    }

    /// Stop the turn (`interrupt`).
    pub fn stop(&mut self) -> bool {
        if !self.turn || self.stopping {
            return false;
        }
        let id = self.next_id("interrupt");
        let sent = self
            .live
            .as_ref()
            .is_some_and(|rt| rt.send(stream::interrupt(&id)));
        if sent {
            self.interrupts.insert(id);
            self.stopping = true;
            self.markers += 1;
            self.say(Tone::Dim, "stopping…");
        }
        sent
    }

    /// End the live process gracefully: `interrupt` if a turn runs, stdin
    /// closed; SIGTERM after `term`, SIGKILL after `kill` (§1.4). An older
    /// chat still ending is killed first. The generation's end is returned
    /// for the app.
    pub fn end(&mut self, now: Instant, term: Duration, kill: Duration) -> Option<Event> {
        let rt = self.live.take()?;
        // An older chat still ending is killed first; one that could not be
        // reaped in time stays to be stepped (review PRO-9).
        self.ending.retain_mut(|e| !e.kill_now());
        self.ended_by_itself = false;
        if self.turn {
            let id = self.next_id("interrupt");
            rt.send(stream::interrupt(&id));
        }
        self.ending.push(Ending::new(rt, now, term, kill));
        self.turn = false;
        self.stopping = false;
        self.held.clear();
        Some(Event::Ended { gen: self.gen })
    }

    /// New chat (asked first by the app): the old conversation ends.
    pub fn new_chat(&mut self, now: Instant) -> Option<Event> {
        let ended = self.end(now, runtime::END_TERM_AFTER, runtime::END_KILL_AFTER);
        // A chat that ended by itself starts afresh too: its title, its
        // line — said once (fix check N3).
        self.ended_by_itself = false;
        self.stopped_last = false;
        self.spoke = false;
        if (ended.is_some() || self.gen > 0) && !self.announced {
            self.say(Tone::Cockpit, NEW_CHAT);
            self.announced = true;
        }
        ended
    }

    /// One pass of the loop: what the runtime printed, the ending ones
    /// stepped, the watchdogs. Events for the app, in order.
    pub fn pump(&mut self, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        self.ending.retain_mut(|e| !e.step(now));
        let Some(rt) = self.live.as_mut() else {
            return events;
        };
        let msgs = rt.drain();
        // A leader that died while something of its group still holds its
        // pipes never brings EOF: every 2 s its state is looked at — it is
        // a zombie until reaped, so its group is still ours to end
        // (review PRO-11).
        if self.exited_at.is_none() && now.saturating_duration_since(self.probed) >= LEADER_PROBE {
            self.probed = now;
            if runtime::leader_exited(rt.pid) {
                self.exited_at = Some(now);
            }
        }
        // The lines it printed before it exited are read first: the end
        // waits a moment for its pipes (fix check N4).
        let gone = self
            .exited_at
            .is_some_and(|t| now.saturating_duration_since(t) >= LEADER_GRACE);
        let eof = rt.eof() || gone;
        for msg in msgs {
            match msg {
                Out::Line(line) => {
                    self.last_out = now;
                    self.quiet_warned = false;
                    let parsed = stream::parse(&line);
                    self.handle(parsed, now, &mut events);
                    if self.live.is_none() {
                        return events;
                    }
                }
                Out::Cut => {
                    self.last_out = now;
                    let offer = if self.turn {
                        " — Stop (Esc) if the turn is stuck"
                    } else {
                        ""
                    };
                    self.say(
                        Tone::Warn,
                        format!(
                            "a line from the runtime was over {} MiB and was dropped{offer}",
                            runtime::MAX_LINE_BYTES / (1024 * 1024)
                        ),
                    );
                }
                Out::Stderr(_) | Out::Eof(_) => {}
            }
        }
        if eof {
            // Why: the last turn's error, else stderr's last line (§1.4).
            let why = self.last_error.take().or_else(|| {
                self.live
                    .as_ref()
                    .and_then(|rt| rt.stderr_tail.back().cloned())
                    .filter(|l| !l.trim().is_empty())
            });
            self.say(
                Tone::Warn,
                match &why {
                    Some(line) => format!("the chat ended: {line}"),
                    None => "the chat ended".to_string(),
                },
            );
            if let Some(e) = self.end(now, Duration::ZERO, Duration::from_millis(300)) {
                events.push(e);
            }
            self.ended_by_itself = true;
            // Ended by itself: the walk-through, with what was seen (a
            // sign-in failure said before it stands).
            self.trouble(
                Trouble::Stopped,
                why.as_deref()
                    .unwrap_or("Claude Code ended without saying why"),
                &mut events,
            );
            return events;
        }
        // The watchdogs.
        if !self.saw_init
            && !self.start_warned
            && self
                .started
                .is_some_and(|s| now.saturating_duration_since(s) >= START_WAIT)
        {
            self.start_warned = true;
            self.say(
                Tone::Warn,
                "the chat has not started — a sign-in or keychain prompt may be waiting; run \
                 `claude` in a terminal to check. New chat: Ctrl-N",
            );
        }
        if self.saw_init
            && self.owes_output()
            && !self.quiet_warned
            && now.saturating_duration_since(self.last_out) >= QUIET
        {
            self.quiet_warned = true;
            self.say(
                Tone::Warn,
                "nothing from the model for a minute — Stop (Esc) or New chat (Ctrl-N)",
            );
        }
        events
    }

    /// End the chat for a reason found in its stream (a failed `init`
    /// check): said, and the process ended quickly.
    fn fail(&mut self, p: Problem, now: Instant, events: &mut Vec<Event>) {
        self.say(Tone::Bad, p.said());
        if let Some(e) = self.end(now, Duration::ZERO, Duration::from_millis(300)) {
            events.push(e);
        }
        // The title says "ended", not "not started" (fix check N8).
        self.ended_by_itself = true;
        self.trouble_with(p, events);
    }

    fn check_init(
        &self,
        permission_mode: Option<&str>,
        tools: &[String],
        servers: &[(String, String)],
    ) -> Result<(), Problem> {
        let version = self.version.as_deref().unwrap_or("?");
        if permission_mode != Some("default") {
            return Err(Problem::new(
                Trouble::Stopped,
                &format!(
                    "Claude Code {version} runs in permission mode {} — the chat is off",
                    permission_mode.unwrap_or("(none)")
                ),
            ));
        }
        match servers {
            [(name, status)] if name == "harness" => match status.as_str() {
                "connected" | "pending" => {}
                other => {
                    return Err(Problem::new(
                        Trouble::Stopped,
                        &format!(
                        "the harness tools did not start: harness-mcp is {other} — the chat is off"
                    ),
                    ))
                }
            },
            _ => {
                let names: Vec<&str> = servers.iter().map(|(n, _)| n.as_str()).collect();
                return Err(Problem::new(
                    Trouble::Stopped,
                    &format!(
                        "Claude Code {version} attached other tool servers ({}) — the chat is off",
                        names.join(", ")
                    ),
                ));
            }
        }
        if let Some(extra) = tools.iter().find(|t| !COCKPIT_TOOLS.contains(&t.as_str())) {
            return Err(Problem::new(
                Trouble::Stopped,
                &format!(
                "Claude Code {version} offers a tool the cockpit does not expect: {extra} — the \
                 chat is off"
            ),
            ));
        }
        let connected = servers.iter().all(|(_, s)| s == "connected");
        if connected {
            if let Some(missing) = COCKPIT_TOOLS
                .iter()
                .find(|t| !tools.iter().any(|x| x == *t))
            {
                // An older harness-mcp beside a newer cockpit: a rebuild,
                // said as one, never the sign-in walk-through.
                let mcp = match &self.bins {
                    Ok(b) => b.mcp.clone(),
                    Err(_) => PathBuf::from("harness-mcp"),
                };
                return Err(Problem::old_tools(&mcp, missing));
            }
        }
        Ok(())
    }

    fn handle(&mut self, msg: In, now: Instant, events: &mut Vec<Event>) {
        let gen = self.gen;
        match msg {
            In::Init {
                model,
                version,
                permission_mode,
                tools,
                servers,
                api_key_source,
            } => {
                let first = !self.saw_init;
                self.saw_init = true;
                self.version = version.clone();
                if !self.turn {
                    self.turn = true;
                    self.turn_started = Some(now);
                }
                if let Err(p) = self.check_init(permission_mode.as_deref(), &tools, &servers) {
                    return self.fail(p, now, events);
                }
                self.model = model;
                self.api_key_source = api_key_source;
                if first {
                    let v = version.as_deref().unwrap_or("?");
                    let tested = if v == TESTED_VERSION {
                        String::new()
                    } else {
                        format!(" (the cockpit was tested with {TESTED_VERSION})")
                    };
                    let key = match self.api_key_source.as_deref() {
                        Some("none") | None => String::new(),
                        Some(src) => format!(", credentials from {src}"),
                    };
                    let model = self.model.clone().unwrap_or_default();
                    self.say(Tone::Dim, format!("Claude Code {v}{tested} · {model}{key}"));
                }
            }
            In::MessageStart { id } => {
                if let Some(prev) = self.cur_msg.replace(id.clone()) {
                    self.transcript.prune_empty(&prev);
                }
                self.cur_block = None;
                self.transcript.message_start(&id);
            }
            In::BlockStart { index } => self.cur_block = Some(index),
            In::TextDelta { index, text } => {
                if let Some(m) = self.cur_msg.clone() {
                    self.unseen = true;
                    self.transcript.delta(&m, index, &text);
                }
            }
            In::Assistant {
                id,
                model,
                blocks,
                error,
            } => {
                let synthetic = model.as_deref() == Some("<synthetic>");
                // The runtime's own words, for a sign-in failure's "seen".
                let mut said: Option<String> = None;
                for b in blocks {
                    match b {
                        Block::Text(text) if synthetic => {
                            self.say(Tone::Cockpit, format!("Claude Code says: {text}"));
                            said.get_or_insert(text);
                        }
                        Block::Text(text) => {
                            self.unseen = true;
                            let index = if self.cur_msg.as_deref() == Some(id.as_str()) {
                                self.cur_block
                            } else {
                                None
                            };
                            self.transcript.block(&id, index, &text);
                        }
                        Block::ToolUse { id: call, name, .. } => {
                            self.calls.insert(call, (model.clone(), name));
                        }
                        Block::Thinking | Block::Other => {}
                    }
                }
                if let Some(e) = error {
                    let words = error_words(&e, self.model_flag.as_deref());
                    self.say(Tone::Bad, words);
                    if e == "authentication_failed" {
                        let seen = match &said {
                            Some(text) => format!("Claude Code said: {text}"),
                            None => format!("Claude Code said {e}"),
                        };
                        self.trouble(Trouble::NotSignedIn, &seen, events);
                    }
                }
            }
            In::ToolResults(results) => {
                for r in results {
                    let Some(call) = self.reads.remove(&r.tool_use_id) else {
                        continue;
                    };
                    if self.map_answers.remove(&r.tool_use_id) && !r.is_error {
                        events.push(Event::MapAnswered);
                    }
                    if let Some(workloads) = self.proposals.remove(&r.tool_use_id) {
                        if !r.is_error {
                            events.push(Event::ProposalWritten { workloads });
                        }
                    }
                    match (call.part, self.read_group.as_mut()) {
                        (Some(i), Some(g)) if g.line == call.line => {
                            if let Some(p) = g.parts.get_mut(i) {
                                p.1 = Some(!r.is_error);
                            }
                            let (tone, text) = g.line();
                            self.transcript.update_line(call.line, tone, text);
                        }
                        _ => {
                            let (tone, word) = if r.is_error {
                                (Tone::Bad, "failed")
                            } else {
                                (Tone::Dim, "done")
                            };
                            self.transcript.update_line(
                                call.line,
                                tone,
                                format!("{} — {word}", call.words),
                            );
                        }
                    }
                    if let (false, Some((attempt, key))) = (r.is_error, call.request) {
                        if last_page(&r.text, &key) {
                            self.keys_read.insert((gen, attempt.clone(), key.clone()));
                            events.push(Event::Read { gen, attempt, key });
                        }
                    }
                }
            }
            In::UserText { uuid, replay, text } => {
                let ours = replay && uuid.as_ref().is_some_and(|u| self.sent.contains(u));
                if ours {
                    if let Some(u) = &uuid {
                        self.unechoed.remove(u);
                    }
                    return;
                }
                // A Stop's own marker: the turn's result says "stopped".
                if !replay && self.markers > 0 && stream::is_interrupt_marker(&text) {
                    self.markers -= 1;
                    self.marked = true;
                    return;
                }
                self.say(
                    Tone::Warn,
                    format!(
                        "a message the cockpit did not send reached the chat: “{}” — its \
                         permission to continue a migration is withdrawn; New chat (Ctrl-N) \
                         starts over",
                        text.chars().take(300).collect::<String>()
                    ),
                );
                events.push(Event::Foreign { gen });
            }
            In::CanUseTool {
                request_id,
                tool,
                server,
                input,
                tool_use_id,
            } => self.can_use_tool(request_id, tool, server, input, tool_use_id, events),
            In::OtherRequest {
                request_id,
                subtype,
            } => {
                if let Some(rt) = &self.live {
                    rt.send(stream::error_response(
                        &request_id,
                        "the cockpit does not handle this request",
                    ));
                }
                self.say(
                    Tone::Warn,
                    if subtype.contains("consent") {
                        "this model needs consent — give it in a normal `claude` session, or \
                         choose another with --chat-model"
                            .to_string()
                    } else {
                        format!(
                            "the runtime asked the cockpit for `{subtype}`; answered: not handled"
                        )
                    },
                );
            }
            In::Cancel { request_id } => {
                if self.held.remove(&request_id) {
                    self.released();
                    events.push(Event::Withdrawn { gen, request_id });
                }
            }
            In::Response {
                request_id,
                cancelled,
                ..
            } => {
                if self.interrupts.remove(&request_id) {
                    for uuid in cancelled {
                        self.unechoed.remove(&uuid);
                        self.transcript.undelivered(&uuid);
                    }
                }
            }
            In::Lifecycle { uuid, state } => {
                // A message at its end is queued no more, echoed or not —
                // one cancelled between its start and its echo included
                // (fix check 3, F1).
                if matches!(state.as_str(), "completed" | "cancelled") {
                    self.unechoed.remove(&uuid);
                }
                match state.as_str() {
                    "started" => {
                        self.started_uuids.insert(uuid);
                    }
                    "cancelled"
                        if self.sent.contains(&uuid) && !self.started_uuids.contains(&uuid) =>
                    {
                        self.transcript.undelivered(&uuid);
                    }
                    _ => {}
                }
            }
            In::Result(r) => {
                // A turn's end ends its Stop, whatever is queued behind it:
                // the next turn is a new one, and can be stopped (review
                // PRO-2). It was stopped when it ended aborted, brought the
                // Stop's marker, or ended in error while stopping — never
                // for a Stop sent as it ended (fix check N4).
                let aborted = r
                    .terminal_reason
                    .as_deref()
                    .is_some_and(|t| t.starts_with("aborted"));
                let was_stopping = std::mem::take(&mut self.stopping);
                let stopped =
                    aborted || std::mem::take(&mut self.marked) || (was_stopping && r.is_error);
                // No marker comes after its stopped turn's end, nor once
                // the runtime is idle: nothing queued by its count, and every
                // message of the cockpit's echoed (its turn begun).
                if aborted || (r.queued_turn_count == 0 && self.unechoed.is_empty()) {
                    self.markers = 0;
                }
                if r.queued_turn_count > 0 {
                    if stopped {
                        self.say(Tone::Dim, "stopped");
                    }
                    return;
                }
                if let Some(m) = self.cur_msg.take() {
                    self.transcript.prune_empty(&m);
                }
                self.turn = false;
                self.stopped_last = stopped;
                let spent = r.total_cost_usd.map(|c| {
                    let d = (c - self.cost).max(0.0);
                    self.cost = c;
                    d
                });
                let secs = self
                    .turn_started
                    .take()
                    .map(|s| now.saturating_duration_since(s).as_secs_f64());
                let mut words = Vec::new();
                if let Some(s) = secs {
                    words.push(format!("{s:.1} s"));
                }
                if let Some(d) = spent {
                    let usage = if self.api_key_source.as_deref().is_none_or(|s| s == "none") {
                        " of plan usage"
                    } else {
                        ""
                    };
                    words.push(format!("${d:.3}{usage}"));
                }
                if stopped {
                    words.insert(0, "stopped".into());
                } else if r.is_error {
                    let why = r
                        .errors
                        .first()
                        .cloned()
                        .or(r.result.clone())
                        .unwrap_or_else(|| r.subtype.clone());
                    let words = result_words(&why, self.model_flag.as_deref());
                    // Said once: the turn's own sign-in failure was said
                    // with its message.
                    let said = signed_out(&why)
                        && self
                            .problem
                            .as_ref()
                            .is_some_and(|p| p.trouble == Trouble::NotSignedIn);
                    if signed_out(&why) {
                        self.trouble(
                            Trouble::NotSignedIn,
                            &format!("Claude Code said: {why}"),
                            events,
                        );
                    }
                    self.last_error = Some(words.clone());
                    if !said {
                        self.say(Tone::Bad, words);
                    }
                }
                self.say(Tone::Dim, format!("— {}", words.join(" · ")));
            }
            In::RateLimit {
                status,
                resets_at,
                error_code,
            } => match status.as_str() {
                "allowed_warning" => {
                    self.say(Tone::Warn, "nearing the plan's limit");
                }
                "rejected" => {
                    let when = resets_at.and_then(|at| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()?
                            .as_secs() as i64;
                        Some(((at - now).max(0) + 59) / 60)
                    });
                    self.say(
                        Tone::Bad,
                        match (when, error_code) {
                            (Some(m), _) => {
                                format!("the plan's limit is reached — resets in {m} min")
                            }
                            (None, Some(code)) => format!("the plan's limit is reached ({code})"),
                            (None, None) => "the plan's limit is reached".to_string(),
                        },
                    );
                }
                _ => {}
            },
            In::Ignored => {}
            In::Unparsed => {
                if !self.unparsed_said {
                    self.unparsed_said = true;
                    self.say(
                        Tone::Dim,
                        "the runtime printed a line the cockpit could not read",
                    );
                }
            }
        }
    }

    fn can_use_tool(
        &mut self,
        request_id: String,
        tool: String,
        server: Option<String>,
        input: Value,
        tool_use_id: Option<String>,
        events: &mut Vec<Event>,
    ) {
        let gen = self.gen;
        let short = tool.strip_prefix("mcp__harness__").filter(|_| {
            server.as_deref() == Some("harness") && COCKPIT_TOOLS.contains(&tool.as_str())
        });
        let deny_now = |chat: &mut Chat, why: &str| {
            if let Some(rt) = &chat.live {
                rt.send(stream::deny(&request_id, why));
            }
        };
        let Some(short) = short else {
            deny_now(self, "not available in the cockpit");
            self.say(Tone::Dim, format!("· {tool}: not available in the cockpit"));
            return;
        };
        let words = match short {
            "harness_status" => Some("· read the project's status".to_string()),
            // Read-only: the person's features file as it is, for the
            // chat's fix (docs/FEATURES-DESIGN.md §14.3).
            "harness_features_file" => Some("· read the features file".to_string()),
            "harness_workloads_file" => Some("· read the workloads file".to_string()),
            "harness_unit" => Some(match input.get("attempt").and_then(Value::as_str) {
                Some(a) => format!("· read {} of {}", short_attempt(a), arg(&input, "unit")),
                None => format!("· read piece {}", arg(&input, "unit")),
            }),
            "harness_request" => Some(format!(
                "· read the request of {} (page {})",
                short_attempt(&arg(&input, "attempt")),
                input.get("page").and_then(Value::as_u64).unwrap_or(1)
            )),
            "harness_map_questions" => Some(format!(
                "· read the project map's open question (page {})",
                input.get("page").and_then(Value::as_u64).unwrap_or(1)
            )),
            // The chat's answer to the map's question: advice, checked by
            // `project ask`'s own validators in harness-mcp and labelled as
            // the chat's model — allowed only under that model's name
            // (docs/CHAT-PANE-DESIGN.md §3.6). Nothing is built; the
            // person's Accept still picks.
            "harness_map_answer" => {
                let named = input.get("model").and_then(Value::as_str);
                match (&self.model, named) {
                    (Some(m), Some(n)) if m == n => {
                        if let Some(call) = &tool_use_id {
                            self.map_answers.insert(call.clone());
                        }
                        Some("· record the chat's answer to the project map's question".to_string())
                    }
                    _ => {
                        let why = format!(
                            "the advice is recorded under the chat's model: pass model = {}",
                            self.model.as_deref().unwrap_or("the model the chat runs")
                        );
                        deny_now(self, &why);
                        self.say(
                            Tone::Dim,
                            format!("· record the chat's answer: refused — {why}"),
                        );
                        return;
                    }
                }
            }
            // The chat's proposal of the person's features or workloads
            // (docs/COCKPIT-WRAPPER-DESIGN.md §9.6): checked and written by
            // harness-mcp to its own proposal file — allowed at once under
            // the chat's model, refused while a draft of the person's is
            // kept.
            "harness_features_propose" | "harness_workloads_propose" => {
                let workloads = short == "harness_workloads_propose";
                let noun = if workloads { "workloads" } else { "features" };
                let named = input.get("model").and_then(Value::as_str);
                let why = if self.drafts_kept[usize::from(workloads)] {
                    Some(format!(
                        "a {noun} draft of the person's is kept — they save or discard it first \
                         (the draft slot is theirs)"
                    ))
                } else {
                    match (&self.model, named) {
                        (Some(m), Some(n)) if m == n => None,
                        _ => Some(format!(
                            "the proposal is written under the chat's model: pass model = {}",
                            self.model.as_deref().unwrap_or("the model the chat runs")
                        )),
                    }
                };
                if let Some(why) = why {
                    deny_now(self, &why);
                    self.say(
                        Tone::Dim,
                        format!("· propose the {noun} file: refused — {why}"),
                    );
                    return;
                }
                if let Some(call) = &tool_use_id {
                    self.proposals.insert(call.clone(), workloads);
                }
                Some(format!("· write the chat's proposal of the {noun} file"))
            }
            _ => None,
        };
        if let Some(words) = words {
            if let Some(rt) = &self.live {
                rt.send(stream::allow(&request_id, &input));
            }
            // Consecutive reads fold into one line (§9.5): a read right
            // after the last one's line joins it.
            let read = words.strip_prefix("· read ").map(str::to_string);
            let last = self.transcript.cells.last().map(|c| c.id);
            let (line, part) = match (read, self.read_group.as_mut()) {
                (Some(what), Some(g)) if Some(g.line) == last => {
                    g.parts.push((what, None));
                    let (tone, text) = g.line();
                    self.transcript.update_line(g.line, tone, text);
                    self.unseen = true;
                    (g.line, Some(g.parts.len() - 1))
                }
                (Some(what), _) => {
                    let line = self.say(Tone::Dim, words.clone());
                    self.read_group = Some(ReadGroup {
                        line,
                        parts: vec![(what, None)],
                    });
                    (line, Some(0))
                }
                (None, _) => (self.say(Tone::Dim, words.clone()), None),
            };
            if let Some(call) = tool_use_id {
                let request = (short == "harness_request")
                    .then(|| (arg(&input, "attempt"), arg(&input, "request_key")));
                self.reads.insert(
                    call,
                    ReadCall {
                        line,
                        words,
                        request,
                        part,
                    },
                );
            }
            return;
        }
        if ACT_TOOLS.contains(&short) {
            self.held.insert(request_id.clone());
            let model = tool_use_id
                .as_ref()
                .and_then(|c| self.calls.get(c))
                .and_then(|(m, _)| m.clone());
            self.unseen = true;
            events.push(Event::Ask {
                gen,
                request_id,
                tool_use_id,
                tool: short.to_string(),
                input,
                model,
            });
            return;
        }
        deny_now(self, "not available in the cockpit");
    }
}

#[cfg(test)]
impl Chat {
    /// A live chat whose runtime appends what the cockpit writes to
    /// `dir/sent.jsonl` (a `cat`), its `init` seen with `model`: the app's
    /// tests drive the chat's side through it. Returns the log's path.
    pub(crate) fn test_live(&mut self, dir: &std::path::Path, model: &str) -> PathBuf {
        let log = dir.join("sent.jsonl");
        let argv: Vec<OsString> = vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("exec cat >> '{}'", log.display()).into(),
        ];
        let chat_dir = runtime::create_dir(&self.temp).expect("a chat directory");
        self.gen += 1;
        let env = [(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))];
        let rt = Runtime::spawn(self.gen, argv, chat_dir, &env, &self.procs).expect("the sink");
        self.live = Some(rt);
        self.saw_init = true;
        self.turn = true;
        self.model = Some(model.to_string());
        self.bins = Ok(Binaries {
            claude: "/bin/sh".into(),
            mcp: "/bin/sh".into(),
        });
        self.sent.clear();
        self.held.clear();
        log
    }
}

/// An attempt id, short (the first 8 characters of the hash).
pub fn short_attempt(id: &str) -> String {
    match id.split_once('-') {
        Some((p, rest)) if rest.len() > 8 => format!("{p}-{}…", &rest[..8]),
        _ => id.chars().take(40).collect(),
    }
}

/// Whether a `harness_request` result is the last page of `key`'s request
/// (`omitted` null, the key its own).
fn last_page(text: &str, key: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    v.get("error").is_none_or(Value::is_null)
        && v.get("omitted").is_some_and(Value::is_null)
        && v.get("request_key").and_then(Value::as_str) == Some(key)
}

/// A runtime error value in words (§1.3).
fn error_words(error: &str, model_flag: Option<&str>) -> String {
    match error {
        "authentication_failed" => {
            "claude is not signed in: run `claude` in a terminal and sign in, then send again"
                .to_string()
        }
        "model_not_found" | "invalid_model" => format!(
            "unknown model {} (--chat-model)",
            model_flag.unwrap_or("(the default)")
        ),
        "billing_error" => "the plan needs credits".to_string(),
        "rate_limit" => "the plan's limit is reached".to_string(),
        other => format!("the runtime reported an error: {other}"),
    }
}

/// A failed turn's error says Claude Code is not signed in ("Not logged in
/// · Please run /login", "Invalid API key", an authentication error).
fn signed_out(why: &str) -> bool {
    let lower = why.to_lowercase();
    lower.contains("not logged in")
        || lower.contains("/login")
        || lower.contains("authentication")
        || lower.contains("invalid api key")
}

/// A failed turn's error in words: the known ones translated.
fn result_words(why: &str, model_flag: Option<&str>) -> String {
    let lower = why.to_lowercase();
    if signed_out(why) {
        return error_words("authentication_failed", model_flag);
    }
    if lower.contains("model") && (lower.contains("not found") || lower.contains("invalid")) {
        return error_words("model_not_found", model_flag);
    }
    if lower.contains("credit") {
        return error_words("billing_error", model_flag);
    }
    format!(
        "the turn failed: {}",
        why.chars().take(300).collect::<String>()
    )
}

#[cfg(test)]
mod tests;
