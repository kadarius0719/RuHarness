//! The cockpit's state and key handling (docs/TUI-DESIGN.md §3, §4, §6).
//!
//! [`App::on_key`] never performs an act: it returns a [`Command`] for the
//! event loop, and every act first shows its exact argv and asks `y/n` —
//! there is no automatic spawn (the resume watcher only marks "response
//! present"). The ledger is re-read — it is the truth — after a spawned
//! command has been reaped, on `g`, and on the 2 s watcher tick.

use crate::events::{Event, EVENTS_SCHEMA, EVENTS_SCHEMA_VERSION};
use crate::handedit;
use crate::highlight::{Highlighter, Lang, Pieces};
use crate::model::{AttemptView, AuthorshipView, ProvenanceView, Snapshot, UnitView};
use crate::pairs::{CSide, FunctionPair, RustNote, SourceSpan};
use crate::spawn::ChildMsg;
use harness_core::attempts::HUMAN_KIND;
use harness_core::verdict::Verdict;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

/// Longest steer note accepted (the CLI's limit).
pub const MAX_NOTE_BYTES: usize = 2000;
/// Longest hand-edit note accepted (the CLI's limit).
pub const MAX_EDIT_NOTE_BYTES: usize = 400;
/// Run-panel lines kept.
const MAX_RUN_LINES: usize = 2000;

/// Wide or stacked pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Side by side at ≥ [`crate::view::WIDE_MIN_COLUMNS`] columns, stacked below.
    Auto,
    /// Always side by side.
    Split,
    /// Always stacked (C, then Rust, per pair).
    Stacked,
}

/// How the cockpit was started.
#[derive(Debug, Clone)]
pub struct Config {
    /// The target root (canonical), passed to every spawn as `--target=`.
    pub target: PathBuf,
    /// The `harness` binary every act spawns; `None` = read-only.
    pub harness: Option<PathBuf>,
    /// Pass `--allow-unsandboxed` to acts that run code.
    pub allow_unsandboxed: bool,
    /// Pair layout.
    pub layout: LayoutMode,
}

/// Which pane `j`/`k` move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The rail's attempt list.
    Rail,
    /// The function pairs.
    Pairs,
}

/// What the pairs panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// The unit's crate.
    Crate,
    /// An attempt's crate (its candidate, or a hand edit's kept files).
    Attempt(String),
}

/// An act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// `a`: promote a green attempt.
    Accept,
    /// `m`: a steer attempt seeded from the shown one.
    Modify,
    /// `e`: a labelled hand edit.
    HandEdit,
    /// `r`: the attempt's own run shape, `--retry`.
    Retry,
    /// `R`: the stored argv of the run that ended awaiting.
    Resume,
}

impl Act {
    /// Its name in prompts.
    pub fn label(self) -> &'static str {
        match self {
            Act::Accept => "Accept (promote)",
            Act::Modify => "Modify (steer)",
            Act::HandEdit => "Hand edit (override)",
            Act::Retry => "Retry",
            Act::Resume => "Resume",
        }
    }
}

/// A command waiting for `y`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// Which act.
    pub act: Act,
    /// The exact argv, the resolved binary first.
    pub argv: Vec<OsString>,
    /// A hand edit's temp dir, removed once the command was reaped (or the
    /// prompt declined).
    pub cleanup: Option<PathBuf>,
    /// A resume: the attempt its first `turn-start` must name.
    pub expect_attempt: Option<String>,
}

/// What the event loop must do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Nothing.
    None,
    /// Leave (a running command runs on to completion).
    Quit,
    /// Interrupt the running command, wait ≤ 1 s, leave.
    CancelAndQuit,
    /// Spawn a confirmed act.
    Spawn(Pending),
    /// `/bin/kill -INT` the running command.
    Cancel,
    /// Re-read the ledger.
    Reload,
    /// Suspend, run the editor over a copy of `crate_dir`'s two files.
    Edit {
        /// The unit.
        unit: String,
        /// The crate whose `src/logic.rs` + `src/ffi.rs` are edited.
        crate_dir: PathBuf,
    },
    /// Remove a temp dir nobody needs any more.
    Cleanup(PathBuf),
}

/// An overlay or prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The panes.
    Normal,
    /// `?`.
    Help {
        /// First row shown.
        scroll: usize,
    },
    /// `v`: the checks, one selected.
    Verdict {
        /// Selected check.
        selected: usize,
        /// First row of the overlay shown (a long detail scrolls).
        scroll: usize,
    },
    /// `d`: the Rust diff.
    Diff {
        /// First line shown.
        scroll: usize,
        /// The unified diff, raw lines.
        lines: Vec<String>,
        /// What is compared.
        title: String,
    },
    /// `m`: typing the steer note.
    Note {
        /// The note so far.
        input: String,
    },
    /// After a changed hand edit: typing its optional note.
    EditNote {
        /// The note so far (empty = none).
        input: String,
        /// The unit.
        unit: String,
        /// The staged DIR (`<tmp>/stage`).
        stage: PathBuf,
        /// The temp dir to remove afterwards.
        tmp: PathBuf,
    },
    /// An act's argv, waiting for `y`/`n`.
    Confirm(Pending),
    /// `q` while a command runs.
    QuitConfirm,
}

/// One line of the run panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLine {
    /// How to colour it.
    pub tone: Tone,
    /// Raw text (display-filtered when rendered).
    pub text: String,
}

/// Run-panel line colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Ordinary.
    Plain,
    /// Something passed / went green.
    Good,
    /// Something failed / went red.
    Bad,
    /// Worth a look.
    Warn,
    /// Background (stderr, the argv).
    Dim,
}

/// The last (or current) spawned command, as the run panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPanel {
    /// The argv as spawned.
    pub argv: Vec<OsString>,
    /// Event lines.
    pub lines: Vec<RunLine>,
    /// Set once reaped.
    pub exit: Option<String>,
    /// Saw the `result` event.
    pub saw_result: bool,
    /// A resume: the attempt its first `turn-start` must name.
    pub expect_attempt: Option<String>,
    /// The act that spawned it.
    pub act: Act,
    /// It emitted its `attempt` event (for `override`: the edit is stored).
    pub recorded: bool,
    /// A hand edit's temp dir, decided on when the command is over.
    pub cleanup: Option<PathBuf>,
}

/// An `external` hand-off waiting for its response file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiting {
    /// The awaited attempt (none for triage).
    pub attempt: Option<String>,
    /// The response file.
    pub path: PathBuf,
    /// The argv of the run that ended awaiting (re-spawned by `R`).
    pub argv: Vec<OsString>,
    /// The CLI's human hint (text only).
    pub resume_hint: String,
    /// The response file exists, is non-empty and parses as a JSON object.
    pub response_present: bool,
}

/// A staged hand edit the ledger has not recorded yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptEdit {
    /// The unit.
    pub unit: String,
    /// The staged DIR (`<tmp>/stage`).
    pub stage: PathBuf,
    /// The temp dir holding it (`<tmp>/edit` is the edited copy).
    pub tmp: PathBuf,
    /// The note typed for it so far.
    pub note: String,
}

/// One line of a side of a pair, ready for the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeLine {
    /// Source: its 1-based line number and highlighted raw pieces.
    Code {
        /// Line number in its file.
        number: usize,
        /// The highlighted raw text.
        pieces: Pieces,
    },
    /// A separator between the shim and its logic function.
    Link(String),
    /// Why there is no code.
    Note(String),
}

/// One pair, highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairView {
    /// The plan symbol.
    pub symbol: String,
    /// The C side's heading.
    pub c_title: String,
    /// The Rust side's heading.
    pub rust_title: String,
    /// The C side.
    pub c: Vec<CodeLine>,
    /// The Rust side (shim, link, logic).
    pub rust: Vec<CodeLine>,
}

/// What the last draw laid out (the view writes it; keys read it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// First row of each pair in the pairs panel.
    pub pair_rows: Vec<usize>,
    /// Rows the pairs panel has in total.
    pub total_rows: usize,
    /// Rows visible at once.
    pub page: usize,
}

/// The cockpit.
#[derive(Debug)]
pub struct App {
    /// How it was started.
    pub config: Config,
    /// The ledger, as last read.
    pub snapshot: Snapshot,
    /// Selected unit (index into `snapshot.units`).
    pub unit: usize,
    /// Rail cursor: 0 = the unit crate, `i` = attempt `i - 1`.
    pub rail: usize,
    /// What the pairs panel shows.
    pub shown: Shown,
    /// Which pane `j`/`k` move.
    pub focus: Focus,
    /// First pairs-panel row shown.
    pub scroll: usize,
    /// Overlay or prompt.
    pub mode: Mode,
    /// A one-line message for the status bar.
    pub notice: Option<String>,
    /// The last (or current) spawned command.
    pub run: Option<RunPanel>,
    /// A command is running.
    pub running: bool,
    /// The outstanding `external` hand-offs, one per awaited attempt.
    pub awaiting: Vec<Awaiting>,
    /// The pairs of the shown crate, highlighted.
    pub pairs: Vec<PairView>,
    /// What the last draw laid out.
    pub layout: Layout,
    /// Every staged hand edit not yet recorded (asked for, running, refused,
    /// declined, or kept after an editor abort), oldest first. The cockpit
    /// never removes one unless the override recorded it or the user
    /// discarded it explicitly (`D` at its armed prompt); `E` offers the
    /// latest again; their paths are printed on exit.
    pub kept_edits: Vec<KeptEdit>,
    /// Temp dirs kept only for what an editor left in them (a swap or
    /// `.save` file after an abort): never re-offered, printed on exit.
    pub leftovers: Vec<PathBuf>,
    /// First row of the Confirm overlay shown (a long argv scrolls).
    pub confirm_scroll: usize,
    /// The view showed the Confirm overlay's whole argv (set by the view).
    pub confirm_seen: bool,
    /// The Confirm prompt may take its `y`: armed by the event loop once it
    /// was drawn whole with no input pending, so typed-ahead or pasted input
    /// can never answer it.
    pub confirm_armed: bool,
    /// The open diff wrapped at `.0` columns (a view cache: a diff is
    /// wrapped once per width, not every frame).
    pub diff_rows: Option<(usize, Vec<ratatui::text::Line<'static>>)>,
    pairs_key: Option<(String, Shown, String)>,
    pending_bracket: Option<char>,
    highlighter: Highlighter,
    run_awaiting: Option<Awaiting>,
}

fn os(s: impl Into<OsString>) -> OsString {
    s.into()
}

/// A POSIX-shell-quoted rendering of an argv, for display only.
pub fn shell_line(argv: &[OsString]) -> String {
    argv.iter()
        .map(|a| {
            let s = a.to_string_lossy();
            let safe = !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
            if safe {
                s.into_owned()
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the awaited response file is there to resume from: it exists,
/// is non-empty, and parses as a JSON object (a half-written file does not).
pub fn response_present(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| {
        !bytes.is_empty()
            && serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|v| v.is_object())
    })
}

impl App {
    /// A cockpit over `snapshot`.
    pub fn new(config: Config, snapshot: Snapshot) -> App {
        let mut app = App {
            config,
            snapshot,
            unit: 0,
            rail: 0,
            shown: Shown::Crate,
            focus: Focus::Pairs,
            scroll: 0,
            mode: Mode::Normal,
            notice: None,
            run: None,
            running: false,
            awaiting: Vec::new(),
            pairs: Vec::new(),
            layout: Layout::default(),
            kept_edits: Vec::new(),
            leftovers: Vec::new(),
            diff_rows: None,
            confirm_scroll: 0,
            confirm_seen: false,
            confirm_armed: false,
            pairs_key: None,
            pending_bracket: None,
            highlighter: Highlighter::new(),
            run_awaiting: None,
        };
        app.refresh_pairs(true);
        app
    }

    /// The selected unit.
    pub fn unit_view(&self) -> Option<&UnitView> {
        self.snapshot.units.get(self.unit)
    }

    /// The shown attempt, when an attempt is shown.
    pub fn shown_attempt(&self) -> Option<&AttemptView> {
        match &self.shown {
            Shown::Attempt(id) => self.unit_view()?.attempt(id),
            Shown::Crate => None,
        }
    }

    /// The crate the pairs panel reads.
    pub fn shown_crate(&self) -> Option<PathBuf> {
        match &self.shown {
            Shown::Crate => self.unit_view()?.crate_dir.clone(),
            Shown::Attempt(_) => self.shown_attempt()?.crate_dir().map(Path::to_path_buf),
        }
    }

    /// The verdict of what is shown: the unit's latest, or the attempt's.
    pub fn shown_verdict(&self) -> Option<&Verdict> {
        match &self.shown {
            Shown::Crate => self.unit_view()?.verdict.as_ref(),
            Shown::Attempt(_) => self.shown_attempt()?.verdict.as_ref(),
        }
    }

    /// Re-read the ledger, keeping the selection by id; `pairs` also
    /// re-reads the shown crate (after a command was reaped, and on `g`);
    /// the pairs are re-read anyway when the shown crate changed. `false`
    /// when the ledger could not be read (the notice says why; the last
    /// snapshot stays).
    pub fn reload(&mut self, pairs: bool) -> bool {
        let unit_id = self.unit_view().map(|u| u.unit.id.clone());
        let rail_id = self.rail_attempt_id();
        match Snapshot::load(&self.config.target) {
            Ok(snapshot) => self.snapshot = snapshot,
            Err(e) => {
                self.notice = Some(format!("reload failed: {e}"));
                return false;
            }
        }
        self.unit = unit_id
            .and_then(|id| self.snapshot.units.iter().position(|u| u.unit.id == id))
            .unwrap_or(0);
        let attempts = self.unit_view().map_or(0, |u| u.attempts.len());
        self.rail = match rail_id {
            Some(id) => self
                .unit_view()
                .and_then(|u| u.attempts.iter().position(|a| a.record.id == id))
                .map_or(0, |i| i + 1),
            None => 0,
        }
        .min(attempts);
        if let Shown::Attempt(id) = &self.shown {
            if self.unit_view().and_then(|u| u.attempt(id)).is_none() {
                self.shown = Shown::Crate;
            }
        }
        // A finished (or vanished) attempt no longer awaits anything.
        let units = &self.snapshot.units;
        self.awaiting.retain(|aw| match &aw.attempt {
            Some(id) => units.iter().any(|u| {
                u.attempt(id)
                    .is_some_and(|a| a.record.outcome == "in-progress")
            }),
            None => true,
        });
        self.refresh_pairs(pairs);
        true
    }

    /// What the shown crate's bytes are, as far as the snapshot tells: an
    /// attempt's record is rewritten after its `candidate/` on every judged
    /// turn; the unit crate's verdict names its digest. A change re-reads
    /// the pairs, so Accept never promotes code the panel did not show.
    fn shown_fingerprint(&self) -> String {
        match &self.shown {
            Shown::Attempt(_) => self.shown_attempt().map_or_else(String::new, |a| {
                let r = &a.record;
                format!("{}|{}|{}", r.outcome, r.turns.len(), r.candidate_digest)
            }),
            Shown::Crate => self.unit_view().map_or_else(String::new, |u| {
                let digest = u
                    .verdict
                    .as_ref()
                    .map_or("", |v| v.inputs.rust_crate.as_str());
                format!("{}|{}|{:?}", u.report.status, digest, u.provenance)
            }),
        }
    }

    fn rail_attempt_id(&self) -> Option<String> {
        let unit = self.unit_view()?;
        self.rail
            .checked_sub(1)
            .and_then(|i| unit.attempts.get(i))
            .map(|a| a.record.id.clone())
    }

    /// Recompute the highlighted pairs when what is shown changed (or
    /// `force`).
    pub fn refresh_pairs(&mut self, force: bool) {
        let key = self.unit_view().map(|u| {
            (
                u.unit.id.clone(),
                self.shown.clone(),
                self.shown_fingerprint(),
            )
        });
        if !force && key == self.pairs_key {
            return;
        }
        self.pairs_key = key;
        let Some(unit) = self.snapshot.units.get(self.unit) else {
            self.pairs.clear();
            return;
        };
        let crate_dir = match &self.shown {
            Shown::Crate => unit.crate_dir.clone(),
            Shown::Attempt(id) => unit
                .attempt(id)
                .and_then(|a| a.crate_dir().map(Path::to_path_buf)),
        };
        let raw = self.snapshot.pairs(unit, crate_dir.as_deref());
        let crate_name = crate_dir
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the crate".into());
        self.pairs = raw
            .iter()
            .map(|p| pair_view(&mut self.highlighter, p, &crate_name))
            .collect();
        self.scroll = self.scroll.min(self.layout.total_rows);
    }

    /// Feed one message of the running command.
    pub fn on_child_msg(&mut self, msg: ChildMsg) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let push = |run: &mut RunPanel, tone: Tone, text: String| {
            run.lines.push(RunLine { tone, text });
            if run.lines.len() > MAX_RUN_LINES {
                run.lines.drain(..run.lines.len() - MAX_RUN_LINES);
            }
        };
        match msg {
            ChildMsg::Stderr(line) => push(run, Tone::Dim, line),
            ChildMsg::Eof(_) => {}
            ChildMsg::Event(ev) => match ev {
                Event::Header {
                    schema,
                    schema_version,
                    ..
                } => {
                    if schema != EVENTS_SCHEMA || schema_version > EVENTS_SCHEMA_VERSION {
                        push(
                            run,
                            Tone::Warn,
                            format!(
                                "events schema {schema} v{schema_version} is newer than this \
                                 cockpit knows (v{EVENTS_SCHEMA_VERSION}); reading what it can"
                            ),
                        );
                    }
                }
                Event::Message { text } => push(run, Tone::Plain, text),
                Event::TurnStart {
                    attempt,
                    index,
                    kind,
                    ..
                } => {
                    if let Some(expected) = run.expect_attempt.take() {
                        if expected != attempt {
                            push(
                                run,
                                Tone::Warn,
                                format!(
                                    "resumed run is on attempt {attempt}, not the awaited \
                                     {expected} — shown, not chased"
                                ),
                            );
                        }
                    }
                    push(
                        run,
                        Tone::Plain,
                        format!("turn {index} {kind} → …  ({attempt})"),
                    );
                }
                Event::TurnEnd {
                    index,
                    kind,
                    result,
                    ..
                } => {
                    let tone = if result == "green" {
                        Tone::Good
                    } else {
                        Tone::Bad
                    };
                    push(run, tone, format!("turn {index} {kind} → {result}"));
                }
                Event::Check {
                    name,
                    passed,
                    detail,
                    ..
                } => {
                    let first = detail.lines().next().unwrap_or("");
                    let (tone, mark) = if passed {
                        (Tone::Good, "✓")
                    } else {
                        (Tone::Bad, "✗")
                    };
                    push(run, tone, format!("[check] {name} {mark} {first}"));
                }
                Event::Verdict { green, path, .. } => push(
                    run,
                    if green { Tone::Good } else { Tone::Bad },
                    format!("verdict {} → {path}", if green { "GREEN" } else { "RED" }),
                ),
                Event::Attempt {
                    id,
                    outcome,
                    promotion,
                    ..
                } => {
                    // `override` emits it only once the human attempt is
                    // stored: the edit is in the ledger now.
                    run.recorded = true;
                    push(
                        run,
                        if outcome == "green" {
                            Tone::Good
                        } else {
                            Tone::Bad
                        },
                        format!("attempt {id} {outcome} ({promotion})"),
                    )
                }
                Event::Promote {
                    attempt, result, ..
                } => push(
                    run,
                    if result == "verified" {
                        Tone::Good
                    } else {
                        Tone::Bad
                    },
                    format!("promote {attempt}: {result}"),
                ),
                Event::Awaiting {
                    attempt,
                    path,
                    resume,
                    ..
                } => {
                    push(run, Tone::Warn, format!("awaiting response: {path}"));
                    push(run, Tone::Dim, format!("  hint: {resume}"));
                    self.run_awaiting = Some(Awaiting {
                        attempt,
                        path: PathBuf::from(path),
                        argv: run.argv.clone(),
                        resume_hint: resume,
                        response_present: false,
                    });
                }
                Event::Error {
                    kind,
                    message,
                    holder,
                } => {
                    push(run, Tone::Bad, format!("error ({kind}): {message}"));
                    if let Some(h) = holder {
                        push(
                            run,
                            Tone::Dim,
                            format!(
                                "  held by pid {} `{}` since {}",
                                h.pid.map_or("?".into(), |p| p.to_string()),
                                h.command,
                                h.started
                            ),
                        );
                    }
                }
                Event::Result { exit, signal } => {
                    run.saw_result = true;
                    push(
                        run,
                        Tone::Dim,
                        match signal {
                            Some(sig) => format!("result: exit {exit} ({sig})"),
                            None => format!("result: exit {exit}"),
                        },
                    );
                }
                Event::Other { k, line } => push(run, Tone::Dim, format!("[{k}] {line}")),
                Event::NotJson { line } => push(run, Tone::Warn, format!("? {line}")),
            },
        }
    }

    /// A confirmed act was spawned.
    pub fn on_spawned(&mut self, pending: &Pending) {
        self.running = true;
        self.run_awaiting = None;
        self.run = Some(RunPanel {
            argv: pending.argv.clone(),
            lines: Vec::new(),
            exit: None,
            saw_result: false,
            expect_attempt: pending.expect_attempt.clone(),
            act: pending.act,
            recorded: false,
            cleanup: pending.cleanup.clone(),
        });
        self.notice = None;
    }

    /// The confirmed act could not be started (a hand edit stays kept).
    pub fn on_spawn_failed(&mut self, pending: Pending, why: &str) {
        self.notice = Some(if pending.act == Act::HandEdit {
            format!("could not start the command: {why}; the hand edit is kept — E offers it again")
        } else {
            format!("could not start the command: {why}")
        });
    }

    /// The paths of every kept hand edit and editor leftover, for the exit
    /// message (and the signal path's mirror).
    pub fn kept_paths(&self) -> Vec<PathBuf> {
        self.kept_edits
            .iter()
            .map(|k| k.tmp.join("edit"))
            .chain(self.leftovers.iter().map(|t| t.join("edit")))
            .collect()
    }

    fn forget_edit(&mut self, tmp: &Path) {
        self.kept_edits.retain(|k| k.tmp != tmp);
    }

    /// The spawned command is over (both pipes at EOF, reaped): say how it
    /// ended, and re-read the ledger. Returns the hand-edit temp dir to
    /// remove — only when the override RECORDED the edit (its `attempt`
    /// event); otherwise the edit stays kept and `E` offers it again (a
    /// refusal — `locked`, stale inputs, an interrupt — must never cost the
    /// user their edit).
    pub fn on_child_exit(&mut self, status: ExitStatus) -> Option<PathBuf> {
        use std::os::unix::process::ExitStatusExt;
        self.running = false;
        if let Some(run) = self.run.as_mut() {
            let text = match (status.code(), status.signal()) {
                (_, Some(sig)) => format!("interrupted ({})", signal_name(sig)),
                (Some(code), _) if !run.saw_result => {
                    format!("exited without result (exit {code})")
                }
                (Some(code), _) => format!("exit {code}"),
                (None, None) => "ended".into(),
            };
            run.exit = Some(text);
        }
        if let Some(aw) = self.run_awaiting.take() {
            // One entry per awaited attempt: a re-await replaces its entry,
            // an earlier hand-off of another attempt is kept.
            self.awaiting.retain(|o| o.attempt != aw.attempt);
            self.awaiting.push(aw);
        }
        let mut remove = None;
        if let Some(run) = self.run.as_mut() {
            if let (Act::HandEdit, Some(tmp)) = (run.act, run.cleanup.take()) {
                if run.recorded {
                    self.kept_edits.retain(|k| k.tmp != tmp);
                    remove = Some(tmp);
                } else {
                    run.lines.push(RunLine {
                        tone: Tone::Warn,
                        text: format!(
                            "the override recorded nothing; the hand edit is kept in {} — E \
                             offers it again",
                            tmp.join("edit").display()
                        ),
                    });
                }
            }
        }
        self.reload(true);
        self.check_response();
        remove
    }

    fn check_response(&mut self) {
        for aw in &mut self.awaiting {
            aw.response_present = response_present(&aw.path);
        }
    }

    /// The 2 s watcher: while a command runs or a hand-off is outstanding,
    /// re-read the ledger and look for the response files. Never spawns.
    pub fn tick(&mut self) {
        if self.running || !self.awaiting.is_empty() {
            self.reload(false);
            self.check_response();
        }
    }

    fn harness_argv(&self, rest: &[OsString]) -> Result<Vec<OsString>, String> {
        let harness = self
            .config
            .harness
            .as_ref()
            .ok_or("no `harness` binary found (PATH, or --harness <path>): read-only")?;
        let mut argv = vec![harness.clone().into_os_string(), os("--json")];
        argv.extend_from_slice(rest);
        Ok(argv)
    }

    fn target_arg(&self) -> OsString {
        let mut arg = os("--target=");
        arg.push(&self.config.target);
        arg
    }

    fn with_sandbox_flag(&self, mut argv: Vec<OsString>) -> Vec<OsString> {
        if self.config.allow_unsandboxed {
            argv.push(os("--allow-unsandboxed"));
        }
        argv
    }

    fn needs_attempt(&self) -> Result<(&UnitView, &AttemptView), String> {
        let unit = self.unit_view().ok_or("no unit")?;
        match &self.shown {
            Shown::Crate => Err("select an attempt first (Tab, j/k, Enter)".into()),
            Shown::Attempt(id) => unit
                .attempt(id)
                .map(|a| (unit, a))
                .ok_or_else(|| format!("attempt {id} is gone")),
        }
    }

    /// The argv of an act on what is shown, or why it is not available.
    pub fn act_argv(&self, act: Act, note: Option<&str>) -> Result<Pending, String> {
        if self.running {
            return Err("a command is running (x cancels it)".into());
        }
        let pending = |argv, expect_attempt| Pending {
            act,
            argv,
            cleanup: None,
            expect_attempt,
        };
        match act {
            Act::Accept => {
                let (unit, a) = self.needs_attempt()?;
                let r = &a.record;
                if r.outcome != "green" || a.last_result() != "green" {
                    return Err(format!(
                        "attempt {} is {} — only a green attempt can be accepted",
                        r.id, r.outcome
                    ));
                }
                if !a.bound {
                    return Err(format!(
                        "attempt {} is bound to superseded inputs — re-run migrate",
                        r.id
                    ));
                }
                let replace =
                    r.promoted || matches!(unit.report.status.as_str(), "verified" | "merged");
                let mut rest = vec![
                    os("promote"),
                    os(&unit.unit.id),
                    os(&r.id),
                    self.target_arg(),
                ];
                if replace {
                    rest.push(os("--replace"));
                }
                Ok(pending(
                    self.with_sandbox_flag(self.harness_argv(&rest)?),
                    None,
                ))
            }
            Act::Modify => {
                let (unit, a) = self.needs_attempt()?;
                let r = &a.record;
                if r.outcome == "in-progress" {
                    return Err(format!("attempt {} is not finished", r.id));
                }
                if !a.bound {
                    return Err(format!(
                        "attempt {} is bound to superseded inputs — it cannot seed",
                        r.id
                    ));
                }
                if a.candidate.is_none() || a.verdict.is_none() {
                    return Err(format!(
                        "attempt {} has no candidate and verdict to revise",
                        r.id
                    ));
                }
                let Some(note) = note else {
                    return Err("no note".into());
                };
                let mut from = os("--from=");
                from.push(&r.id);
                let mut steer = os("--steer=");
                steer.push(note);
                let rest = vec![
                    os("migrate"),
                    os(&unit.unit.id),
                    self.target_arg(),
                    os("--no-promote"),
                    from,
                    steer,
                ];
                Ok(pending(
                    self.with_sandbox_flag(self.harness_argv(&rest)?),
                    None,
                ))
            }
            Act::Retry => {
                let (unit, a) = self.needs_attempt()?;
                let r = &a.record;
                if r.outcome == "in-progress" {
                    return Err(format!("attempt {} is not finished", r.id));
                }
                if r.provider_kind == HUMAN_KIND {
                    return Err("a hand edit has no run to retry".into());
                }
                let mut rest = vec![
                    os("migrate"),
                    os(&unit.unit.id),
                    self.target_arg(),
                    os("--no-promote"),
                    os("--retry"),
                    os(format!("--provider={}", r.provider)),
                    os(format!("--model={}", r.model)),
                ];
                if let (Some(seed), Some(note)) = (&r.seeded_from, &r.steer_note) {
                    rest.push(os(format!("--from={seed}")));
                    rest.push(os(format!("--steer={note}")));
                }
                Ok(pending(
                    self.with_sandbox_flag(self.harness_argv(&rest)?),
                    None,
                ))
            }
            Act::Resume => {
                if self.awaiting.is_empty() {
                    return Err("nothing is awaiting a response".into());
                }
                // The hand-off of the shown attempt, else the newest one
                // whose response is present.
                let shown = match &self.shown {
                    Shown::Attempt(id) => Some(id.as_str()),
                    Shown::Crate => None,
                };
                let aw = self
                    .awaiting
                    .iter()
                    .find(|aw| shown.is_some() && aw.attempt.as_deref() == shown)
                    .or_else(|| {
                        self.awaiting
                            .iter()
                            .rev()
                            .find(|aw| response_present(&aw.path))
                    })
                    .or(self.awaiting.last())
                    .ok_or("nothing is awaiting a response")?;
                if let Some(id) = &aw.attempt {
                    let in_progress = self.snapshot.units.iter().any(|u| {
                        u.attempt(id)
                            .is_some_and(|a| a.record.outcome == "in-progress")
                    });
                    if !in_progress {
                        return Err(format!("attempt {id} is no longer in progress"));
                    }
                }
                if !response_present(&aw.path) {
                    return Err(format!(
                        "no response yet at {} (it must be a JSON object)",
                        aw.path.display()
                    ));
                }
                Ok(pending(aw.argv.clone(), aw.attempt.clone()))
            }
            Act::HandEdit => Err("the hand edit is prepared by the editor flow".into()),
        }
    }

    /// Keep a staged hand edit without asking for it now (an editor that
    /// exited non-zero after saving): `E` offers it.
    pub fn keep_edit(&mut self, unit: String, stage: PathBuf, tmp: PathBuf) {
        self.forget_edit(&tmp);
        self.kept_edits.push(KeptEdit {
            unit,
            stage,
            tmp,
            note: String::new(),
        });
    }

    /// A changed hand edit is staged in `stage`: it is kept, and its
    /// optional note asked for.
    pub fn edit_staged(&mut self, unit: String, stage: PathBuf, tmp: PathBuf) {
        self.keep_edit(unit.clone(), stage.clone(), tmp.clone());
        self.mode = Mode::EditNote {
            input: String::new(),
            unit,
            stage,
            tmp,
        };
    }

    /// The override argv for a staged hand edit in `stage`, with its note
    /// attached as ONE element (`--note=<text>`: a note may start with `-`).
    pub fn hand_edit_argv(
        &self,
        unit: &str,
        stage: &Path,
        tmp: PathBuf,
        note: Option<&str>,
    ) -> Result<Pending, String> {
        let mut rest = vec![
            os("override"),
            os(unit),
            stage.as_os_str().to_owned(),
            self.target_arg(),
        ];
        if let Some(note) = note.filter(|n| !n.trim().is_empty()) {
            let mut arg = os("--note=");
            arg.push(note);
            rest.push(arg);
        }
        Ok(Pending {
            act: Act::HandEdit,
            argv: self.with_sandbox_flag(self.harness_argv(&rest)?),
            cleanup: Some(tmp),
            expect_attempt: None,
        })
    }

    fn hand_edit_target(&self) -> Result<(String, PathBuf), String> {
        if self.running {
            return Err("a command is running".into());
        }
        self.harness_argv(&[])?;
        let unit = self.unit_view().ok_or("no unit")?;
        let dir = self
            .shown_crate()
            .ok_or("nothing to edit: no crate is shown")?;
        if !handedit::editable(&dir) {
            return Err(
                "this crate is not in the executor layout (src/logic.rs + src/ffi.rs)".into(),
            );
        }
        Ok((unit.unit.id.clone(), dir))
    }

    fn select_unit(&mut self, unit: usize) {
        if unit < self.snapshot.units.len() && unit != self.unit {
            self.unit = unit;
            self.rail = 0;
            self.shown = Shown::Crate;
            self.scroll = 0;
            self.refresh_pairs(false);
        }
    }

    fn scroll_to_pair(&mut self, forward: bool) {
        let rows = &self.layout.pair_rows;
        let target = if forward {
            rows.iter().copied().find(|r| *r > self.scroll)
        } else {
            rows.iter().copied().rev().find(|r| *r < self.scroll)
        };
        if let Some(row) = target {
            self.scroll = row;
        }
    }

    fn max_scroll(&self) -> usize {
        self.layout.total_rows.saturating_sub(1)
    }

    fn diff(&self) -> Result<Mode, String> {
        let unit = self.unit_view().ok_or("no unit")?;
        let shown = self
            .shown_attempt()
            .ok_or("select an attempt to compare (Tab, j/k, Enter)")?;
        let base_id = unit
            .provenance
            .attempt()
            .ok_or("the crate's provenance names no single attempt to compare with")?;
        let base = unit
            .attempt(base_id)
            .and_then(AttemptView::crate_dir)
            .ok_or_else(|| format!("attempt {base_id} has no crate"))?;
        let this = shown
            .crate_dir()
            .ok_or_else(|| format!("attempt {} has no crate", shown.record.id))?;
        let mut lines = diff_crates((base, base_id), (this, &shown.record.id));
        if lines.is_empty() {
            lines.push("the Rust sources are identical".into());
        }
        Ok(Mode::Diff {
            scroll: 0,
            lines,
            title: format!("{} vs provenance {base_id}", shown.record.id),
        })
    }

    /// Whether the Confirm prompt waits to be armed (see
    /// [`App::confirm_armed`]).
    pub fn confirm_waiting(&self) -> bool {
        matches!(self.mode, Mode::Confirm(_)) && self.confirm_seen && !self.confirm_armed
    }

    /// Handle one key press.
    pub fn on_key(&mut self, key: KeyEvent) -> Command {
        let command = self.on_key_inner(key);
        if !matches!(self.mode, Mode::Confirm(_)) {
            // Every prompt opens unarmed, unseen, at its top.
            self.confirm_armed = false;
            self.confirm_seen = false;
            self.confirm_scroll = 0;
        }
        command
    }

    /// A bracketed paste: text for a note being typed (line breaks become
    /// spaces), ignored anywhere else — a paste never answers a prompt.
    pub fn on_paste(&mut self, text: &str) {
        let clean: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let (dropped, max) = match &mut self.mode {
            Mode::Note { input } => (push_bounded(input, &clean, MAX_NOTE_BYTES), MAX_NOTE_BYTES),
            Mode::EditNote { input, .. } => (
                push_bounded(input, &clean, MAX_EDIT_NOTE_BYTES),
                MAX_EDIT_NOTE_BYTES,
            ),
            _ => {
                self.notice = Some("paste ignored outside a note".into());
                return;
            }
        };
        if dropped > 0 {
            self.notice = Some(format!(
                "the paste did not fit: {dropped} bytes dropped (a note is at most {max} bytes)"
            ));
        }
    }

    fn stash_note(&mut self, tmp: &Path, note: String) {
        if let Some(k) = self.kept_edits.iter_mut().find(|k| k.tmp == tmp) {
            k.note = note;
        }
    }

    /// `n`/`Esc`: nothing runs; a hand edit stays kept.
    fn decline(&mut self, pending: &Pending) {
        self.notice = Some(match pending.cleanup {
            Some(_) => format!(
                "{}: not run; the hand edit is kept — E offers it again (D at its prompt discards it)",
                pending.act.label()
            ),
            None => format!("{}: not run", pending.act.label()),
        });
    }

    fn on_key_inner(&mut self, key: KeyEvent) -> Command {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        // A key with Ctrl or Alt is never text, never a `y`.
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => {}
            Mode::Help { scroll } => {
                self.mode = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Mode::Help {
                        scroll: scroll.saturating_add(1),
                    },
                    KeyCode::Char('k') | KeyCode::Up => Mode::Help {
                        scroll: scroll.saturating_sub(1),
                    },
                    KeyCode::PageDown | KeyCode::Char(' ') => Mode::Help {
                        scroll: scroll.saturating_add(10),
                    },
                    KeyCode::PageUp => Mode::Help {
                        scroll: scroll.saturating_sub(10),
                    },
                    _ => Mode::Normal,
                };
                return Command::None;
            }
            Mode::Verdict { selected, scroll } => {
                let n = self.shown_verdict().map_or(0, |v| v.checks.len());
                // The view clamps `scroll` to what the detail needs.
                self.mode = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Mode::Verdict {
                        selected: (selected + 1).min(n.saturating_sub(1)),
                        scroll: 0,
                    },
                    KeyCode::Char('k') | KeyCode::Up => Mode::Verdict {
                        selected: selected.saturating_sub(1),
                        scroll: 0,
                    },
                    KeyCode::PageDown | KeyCode::Char(' ') => Mode::Verdict {
                        selected,
                        scroll: scroll.saturating_add(10),
                    },
                    KeyCode::PageUp => Mode::Verdict {
                        selected,
                        scroll: scroll.saturating_sub(10),
                    },
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') | KeyCode::Enter => {
                        Mode::Normal
                    }
                    _ => Mode::Verdict { selected, scroll },
                };
                return Command::None;
            }
            Mode::Diff {
                scroll,
                lines,
                title,
            } => {
                // The view clamps `scroll` to the wrapped rows it has.
                self.mode = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Mode::Diff {
                        scroll: scroll.saturating_add(1),
                        lines,
                        title,
                    },
                    KeyCode::Char('k') | KeyCode::Up => Mode::Diff {
                        scroll: scroll.saturating_sub(1),
                        lines,
                        title,
                    },
                    KeyCode::PageDown | KeyCode::Char(' ') => Mode::Diff {
                        scroll: scroll.saturating_add(20),
                        lines,
                        title,
                    },
                    KeyCode::PageUp => Mode::Diff {
                        scroll: scroll.saturating_sub(20),
                        lines,
                        title,
                    },
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('d') => Mode::Normal,
                    _ => Mode::Diff {
                        scroll,
                        lines,
                        title,
                    },
                };
                return Command::None;
            }
            Mode::Note { mut input } => {
                match key.code {
                    KeyCode::Esc => self.notice = Some("steer cancelled".into()),
                    _ if ctrl_c => self.notice = Some("steer cancelled".into()),
                    KeyCode::Enter => {
                        if input.trim().is_empty() {
                            self.notice = Some("an empty note steers nothing".into());
                            self.mode = Mode::Note { input };
                        } else if let Some(why) = note_problem(&input, MAX_NOTE_BYTES) {
                            // The CLI would refuse it: say so here, keep typing.
                            self.notice = Some(why);
                            self.mode = Mode::Note { input };
                        } else {
                            match self.act_argv(Act::Modify, Some(&input)) {
                                Ok(p) => self.mode = Mode::Confirm(p),
                                Err(why) => self.notice = Some(why),
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.mode = Mode::Note { input };
                    }
                    KeyCode::Char(c) if plain && !c.is_control() => {
                        if input.len() + c.len_utf8() <= MAX_NOTE_BYTES {
                            input.push(c);
                        } else {
                            self.notice = Some(format!("a note is at most {MAX_NOTE_BYTES} bytes"));
                        }
                        self.mode = Mode::Note { input };
                    }
                    _ => self.mode = Mode::Note { input },
                }
                return Command::None;
            }
            Mode::EditNote {
                mut input,
                unit,
                stage,
                tmp,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        self.stash_note(&tmp, input);
                        self.notice = Some("hand edit kept — E offers it again".into());
                        return Command::None;
                    }
                    _ if ctrl_c => {
                        self.stash_note(&tmp, input);
                        self.notice = Some("hand edit kept — E offers it again".into());
                        return Command::None;
                    }
                    KeyCode::Enter
                        if !input.trim().is_empty()
                            && note_problem(&input, MAX_EDIT_NOTE_BYTES).is_some() =>
                    {
                        // The CLI would refuse it: say so here, keep typing.
                        self.notice = note_problem(&input, MAX_EDIT_NOTE_BYTES);
                    }
                    KeyCode::Enter => {
                        self.stash_note(&tmp, input.clone());
                        match self.hand_edit_argv(&unit, &stage, tmp.clone(), Some(&input)) {
                            Ok(p) => {
                                self.mode = Mode::Confirm(p);
                                return Command::None;
                            }
                            // Nothing to spawn it with (read-only): the edit
                            // stays where it is, and says so.
                            Err(why) => {
                                self.notice = Some(format!(
                                    "{why}; the hand edit is kept in {}",
                                    tmp.join("edit").display()
                                ));
                                return Command::None;
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(c) if plain && !c.is_control() => {
                        if input.len() + c.len_utf8() <= MAX_EDIT_NOTE_BYTES {
                            input.push(c);
                        } else {
                            self.notice = Some(format!(
                                "a hand-edit note is at most {MAX_EDIT_NOTE_BYTES} bytes"
                            ));
                        }
                    }
                    _ => {}
                }
                self.mode = Mode::EditNote {
                    input,
                    unit,
                    stage,
                    tmp,
                };
                return Command::None;
            }
            Mode::Confirm(pending) => {
                return match key.code {
                    // Only a plain `y` to a prompt that was shown whole and
                    // armed with no input pending (see `confirm_armed`).
                    KeyCode::Char('y') | KeyCode::Char('Y') if plain && self.confirm_armed => {
                        Command::Spawn(pending)
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') if plain => {
                        self.notice = Some(if self.confirm_seen {
                            "typed ahead of the prompt — read the command, then press y".into()
                        } else {
                            "the command continues below — j scrolls to its end, then y".into()
                        });
                        self.mode = Mode::Confirm(pending);
                        Command::None
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.decline(&pending);
                        Command::None
                    }
                    _ if ctrl_c => {
                        self.decline(&pending);
                        Command::None
                    }
                    // Discarding a hand edit is its own, armed key.
                    KeyCode::Char('D') if plain && pending.cleanup.is_some() => {
                        if self.confirm_armed {
                            let tmp = pending.cleanup.clone().unwrap_or_default();
                            self.forget_edit(&tmp);
                            self.notice = Some("hand edit discarded".into());
                            Command::Cleanup(tmp)
                        } else {
                            self.notice =
                                Some("typed ahead of the prompt — read it, then press D".into());
                            self.mode = Mode::Confirm(pending);
                            Command::None
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.confirm_scroll = self.confirm_scroll.saturating_add(1);
                        self.mode = Mode::Confirm(pending);
                        Command::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.confirm_scroll = self.confirm_scroll.saturating_sub(1);
                        self.mode = Mode::Confirm(pending);
                        Command::None
                    }
                    _ => {
                        self.mode = Mode::Confirm(pending);
                        Command::None
                    }
                };
            }
            Mode::QuitConfirm => {
                return match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => Command::Quit,
                    KeyCode::Char('Q') => Command::CancelAndQuit,
                    _ => Command::None,
                };
            }
        }

        // Normal mode.
        if let Some(open) = self.pending_bracket.take() {
            if key.code == KeyCode::Char('f') {
                self.scroll_to_pair(open == ']');
                return Command::None;
            }
        }
        if ctrl_c {
            return self.quit();
        }
        match key.code {
            KeyCode::Char('q') => return self.quit(),
            KeyCode::Char('Q') => {
                return if self.running {
                    Command::CancelAndQuit
                } else {
                    Command::Quit
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help { scroll: 0 },
            KeyCode::Esc => self.notice = None,
            KeyCode::Char(']') => self.pending_bracket = Some(']'),
            KeyCode::Char('[') => self.pending_bracket = Some('['),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Rail => Focus::Pairs,
                    Focus::Pairs => Focus::Rail,
                }
            }
            KeyCode::Char('J') => self.select_unit(self.unit + 1),
            KeyCode::Char('K') => {
                if let Some(prev) = self.unit.checked_sub(1) {
                    self.select_unit(prev);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => match self.focus {
                Focus::Rail => {
                    let n = self.unit_view().map_or(0, |u| u.attempts.len());
                    self.rail = (self.rail + 1).min(n);
                }
                Focus::Pairs => self.scroll = (self.scroll + 1).min(self.max_scroll()),
            },
            KeyCode::Char('k') | KeyCode::Up => match self.focus {
                Focus::Rail => self.rail = self.rail.saturating_sub(1),
                Focus::Pairs => self.scroll = self.scroll.saturating_sub(1),
            },
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll = (self.scroll + self.layout.page.max(1)).min(self.max_scroll())
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(self.layout.page.max(1)),
            KeyCode::Enter if self.focus == Focus::Rail => {
                self.shown = match self.rail_attempt_id() {
                    Some(id) => Shown::Attempt(id),
                    None => Shown::Crate,
                };
                self.scroll = 0;
                self.refresh_pairs(false);
            }
            KeyCode::Char('g') => return Command::Reload,
            KeyCode::Char('v') => {
                if self.shown_verdict().is_some_and(|v| !v.checks.is_empty()) {
                    self.mode = Mode::Verdict {
                        selected: 0,
                        scroll: 0,
                    };
                } else {
                    self.notice = Some("no verdict for what is shown".into());
                }
            }
            KeyCode::Char('d') => match self.diff() {
                Ok(mode) => {
                    self.diff_rows = None;
                    self.mode = mode;
                }
                Err(why) => self.notice = Some(why),
            },
            KeyCode::Char('a') => self.ask(Act::Accept),
            KeyCode::Char('r') => self.ask(Act::Retry),
            KeyCode::Char('R') => self.ask(Act::Resume),
            KeyCode::Char('m') => {
                // Check everything but the note before asking for it.
                match self.act_argv(Act::Modify, Some("-")) {
                    Ok(_) => {
                        self.mode = Mode::Note {
                            input: String::new(),
                        }
                    }
                    Err(why) => self.notice = Some(why),
                }
            }
            KeyCode::Char('E') => match self.kept_edits.last().cloned() {
                Some(_) if self.running => self.notice = Some("a command is running".into()),
                Some(k) => {
                    self.mode = Mode::EditNote {
                        input: k.note,
                        unit: k.unit,
                        stage: k.stage,
                        tmp: k.tmp,
                    }
                }
                None => self.notice = Some("no kept hand edit".into()),
            },
            KeyCode::Char('e') => match self.hand_edit_target() {
                Ok((unit, crate_dir)) => return Command::Edit { unit, crate_dir },
                Err(why) => self.notice = Some(why),
            },
            KeyCode::Char('x') => {
                if self.running {
                    return Command::Cancel;
                }
                self.notice = Some("nothing is running".into());
            }
            _ => {}
        }
        Command::None
    }

    fn ask(&mut self, act: Act) {
        match self.act_argv(act, None) {
            Ok(p) => self.mode = Mode::Confirm(p),
            Err(why) => self.notice = Some(format!("{}: {why}", act.label())),
        }
    }

    fn quit(&mut self) -> Command {
        if self.running {
            self.mode = Mode::QuitConfirm;
            Command::None
        } else {
            Command::Quit
        }
    }
}

/// Why the CLI would refuse `note` (harness_llm::validate_note's rules,
/// checked before the prompt closes so a note it refuses is never spawned):
/// over `max` bytes, a control character, or a line that looks like a
/// prompt section header (`[WORDS]`). The CLI stays the authority.
pub fn note_problem(note: &str, max: usize) -> Option<String> {
    if note.len() > max {
        return Some(format!("the note is longer than {max} bytes"));
    }
    if note.chars().any(char::is_control) {
        return Some("the note contains a control character".into());
    }
    let header = note.lines().any(|line| {
        let t = line.trim();
        t.len() >= 3
            && t.starts_with('[')
            && t.ends_with(']')
            && t[1..t.len() - 1]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == ' ' || c == '_')
    });
    header.then(|| "the note looks like a prompt section header ([WORDS]); reword it".into())
}

/// Append `text` to `input`, stopping (on a char boundary) at `max` bytes;
/// returns how many bytes of `text` did not fit.
fn push_bounded(input: &mut String, text: &str, max: usize) -> usize {
    for (i, c) in text.char_indices() {
        if input.len() + c.len_utf8() > max {
            return text.len() - i;
        }
        input.push(c);
    }
    0
}

/// Most diff lines kept for the overlay.
const MAX_DIFF_LINES: usize = 20_000;

/// The unified diff of the `.rs` files of two crates (every file of either
/// side), bounded as a WHOLE: one 500 ms deadline for every file, and at
/// most [`MAX_DIFF_LINES`] lines — whatever is cut is said.
fn diff_crates((base, base_id): (&Path, &str), (this, this_id): (&Path, &str)) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let files: std::collections::BTreeSet<String> = rust_files(base)
        .into_iter()
        .chain(rust_files(this))
        .collect();
    let mut lines = Vec::new();
    let total = files.len();
    for (done, rel) in files.into_iter().enumerate() {
        if std::time::Instant::now() >= deadline || lines.len() >= MAX_DIFF_LINES {
            lines.truncate(MAX_DIFF_LINES);
            lines.push(format!(
                "… the diff stops here: {} of {total} files not compared (time or size budget)",
                total - done
            ));
            break;
        }
        let (old, new) = match (read_small(&base.join(&rel)), read_small(&this.join(&rel))) {
            (Ok(o), Ok(n)) => (o, n),
            (Err(why), _) => {
                lines.push(format!("{base_id} {rel}: {why}"));
                continue;
            }
            (_, Err(why)) => {
                lines.push(format!("{this_id} {rel}: {why}"));
                continue;
            }
        };
        if old == new {
            continue;
        }
        // Myers on ledger text, on the UI thread: bounded in time.
        let diff = similar::TextDiff::configure()
            .deadline(deadline)
            .diff_lines(&old, &new);
        let text = diff
            .unified_diff()
            .context_radius(3)
            .header(&format!("{base_id} {rel}"), &format!("{this_id} {rel}"))
            .to_string();
        let room = MAX_DIFF_LINES.saturating_sub(lines.len());
        lines.extend(text.lines().take(room).map(str::to_string));
    }
    lines
}

/// Largest file the diff reads.
const MAX_DIFF_FILE_BYTES: u64 = 1024 * 1024;

/// A crate file for the diff: `""` when absent (the side does not have
/// it), refused when not a regular file or over [`MAX_DIFF_FILE_BYTES`].
fn read_small(path: &Path) -> Result<String, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
        Ok(m) if !m.file_type().is_file() => Err("not a regular file".into()),
        Ok(m) if m.len() > MAX_DIFF_FILE_BYTES => Err(format!(
            "larger than {MAX_DIFF_FILE_BYTES} bytes; not diffed"
        )),
        Ok(_) => std::fs::read(path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| e.to_string()),
    }
}

/// `.rs` files under `crate_dir/src`, crate-relative, sorted.
fn rust_files(crate_dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let rel = format!("{rel}/{name}");
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => walk(&path, &rel, out),
                Ok(m) if m.is_file() && name.ends_with(".rs") && out.len() < 256 => out.push(rel),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&crate_dir.join("src"), "src", &mut out);
    out.sort();
    out
}

fn signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        9 => "SIGKILL".into(),
        15 => "SIGTERM".into(),
        n => format!("signal {n}"),
    }
}

fn code_lines(highlighter: &mut Highlighter, lang: Lang, span: &SourceSpan) -> Vec<CodeLine> {
    highlighter
        .lines(lang, &span.lines)
        .into_iter()
        .enumerate()
        .map(|(i, pieces)| CodeLine::Code {
            number: span.first_line + i,
            pieces,
        })
        .collect()
}

fn pair_view(highlighter: &mut Highlighter, p: &FunctionPair, crate_name: &str) -> PairView {
    let short = p.symbol.rsplit("::").next().unwrap_or(&p.symbol);
    let (c_title, c) = match &p.c {
        CSide::Source(span) => (
            format!("{} ({}:{})", span.name, span.file, span.first_line),
            code_lines(highlighter, Lang::C, span),
        ),
        CSide::StaleFacts { file } => (
            short.to_string(),
            vec![CodeLine::Note(format!(
                "facts predate {file} — run `harness scan`"
            ))],
        ),
        CSide::NotInFacts => (
            short.to_string(),
            vec![CodeLine::Note("not in facts.jsonl".into())],
        ),
    };
    let mut rust = Vec::new();
    let mut rust_title = short.to_string();
    if let Some(shim) = &p.rust.shim {
        rust_title = format!("{} ({}:{})", shim.name, shim.file, shim.first_line);
        rust.extend(code_lines(highlighter, Lang::Rust, shim));
    }
    if let Some(logic) = &p.rust.logic {
        if p.rust.shim.is_some() {
            rust.push(CodeLine::Link(format!(
                "→ {} ({}:{})",
                logic.name, logic.file, logic.first_line
            )));
        } else {
            rust_title = format!("{} ({}:{})", logic.name, logic.file, logic.first_line);
        }
        rust.extend(code_lines(highlighter, Lang::Rust, logic));
    }
    match p.rust.note {
        Some(RustNote::NoCrate) => rust.push(CodeLine::Note("no crate".into())),
        Some(RustNote::NotFound) => rust.push(CodeLine::Note(format!("not found in {crate_name}"))),
        Some(RustNote::LogicNotIdentified) => {
            rust.push(CodeLine::Note("logic fn not identified".into()))
        }
        None => {}
    }
    PairView {
        symbol: p.symbol.clone(),
        c_title,
        rust_title,
        c,
        rust,
    }
}

/// The rail tags of an attempt: provenance, supersession, lineage.
pub fn attempt_tags(unit: &UnitView, a: &AttemptView) -> Vec<String> {
    let mut tags = Vec::new();
    let id = a.record.id.as_str();
    match &unit.provenance {
        ProvenanceView::Pipeline(p) if p == id => tags.push("*".into()),
        ProvenanceView::Steered(p) if p == id => tags.push("*s".into()),
        ProvenanceView::Human { attempt, .. } if attempt == id => tags.push("*h".into()),
        _ => {}
    }
    if a.superseded_by.is_some() {
        tags.push("superseded".into());
    }
    if a.record.provider_kind == HUMAN_KIND {
        tags.push("human".into());
    }
    if let Some(seed) = &a.record.seeded_from {
        tags.push(format!("steer ← {}", short_id(seed)));
    }
    if let (Some(_), AuthorshipView::Human(origin)) = (&a.record.seeded_from, &a.authorship) {
        tags.push(format!("(hand edit {})", short_id(origin)));
    }
    if !a.bound {
        tags.push("stale".into());
    }
    tags
}

/// `a-13c941dfff95` → `a-13c9`; samples keep their `.rN`.
pub fn short_id(id: &str) -> String {
    let (base, sample) = match id.split_once(".r") {
        Some((b, n)) => (b, format!(".r{n}")),
        None => (id, String::new()),
    };
    let cut: String = base.chars().take(6).collect();
    format!("{cut}{sample}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{scratch_target, READ_SCALEFACTORS};
    use harness_core::attempts::{self, AttemptRecord};
    use ratatui::crossterm::event::KeyEvent;

    const PROVENANCE: &str = "a-13c941dfff95";
    const HARNESS: &str = "/opt/ruharness/bin/harness";

    fn app(tag: &str) -> App {
        let target = scratch_target(READ_SCALEFACTORS, tag);
        let snapshot = Snapshot::load(&target).unwrap();
        App::new(
            Config {
                target,
                harness: Some(PathBuf::from(HARNESS)),
                allow_unsandboxed: false,
                layout: LayoutMode::Auto,
            },
            snapshot,
        )
    }

    fn key(app: &mut App, c: char) -> Command {
        app.on_key(KeyEvent::from(KeyCode::Char(c)))
    }

    fn code(app: &mut App, code: KeyCode) -> Command {
        app.on_key(KeyEvent::from(code))
    }

    fn show(app: &mut App, id: &str) {
        let i = app
            .unit_view()
            .unwrap()
            .attempts
            .iter()
            .position(|a| a.record.id == id)
            .unwrap();
        app.focus = Focus::Rail;
        app.rail = i + 1;
        code(app, KeyCode::Enter);
        assert_eq!(app.shown, Shown::Attempt(id.into()));
    }

    fn strs(argv: &[OsString]) -> Vec<String> {
        argv.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// What the event loop does once the prompt was drawn whole with no
    /// input pending.
    fn arm(app: &mut App) {
        app.confirm_seen = true;
        app.confirm_armed = true;
    }

    fn confirm_argv(app: &App) -> Vec<String> {
        match &app.mode {
            Mode::Confirm(p) => strs(&p.argv),
            other => panic!("not confirming: {other:?} (notice {:?})", app.notice),
        }
    }

    /// §4: every act shows its exact argv and asks; `y` spawns exactly that,
    /// `n` spawns nothing.
    #[test]
    fn acts_show_their_exact_argv_and_ask_first() {
        let mut app = app("acts");
        let root = app.config.target.display().to_string();
        // Nothing but the unit crate shown: acts on an attempt say so.
        assert_eq!(key(&mut app, 'a'), Command::None);
        assert!(app.notice.as_deref().unwrap().contains("select an attempt"));
        show(&mut app, PROVENANCE);
        // Accept on a verified unit replaces.
        assert_eq!(key(&mut app, 'a'), Command::None);
        let argv = confirm_argv(&app);
        assert_eq!(
            argv,
            [
                HARNESS,
                "--json",
                "promote",
                "u-lib",
                PROVENANCE,
                &format!("--target={root}"),
                "--replace"
            ]
        );
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn");
        };
        assert_eq!(strs(&p.argv), argv);
        assert_eq!(app.mode, Mode::Normal);
        // Modify: a note that starts with `-` travels attached (§R2 5).
        assert_eq!(key(&mut app, 'm'), Command::None);
        assert!(matches!(app.mode, Mode::Note { .. }));
        for c in "- keep the wrapping add".chars() {
            key(&mut app, c);
        }
        code(&mut app, KeyCode::Enter);
        assert_eq!(
            confirm_argv(&app),
            [
                HARNESS,
                "--json",
                "migrate",
                "u-lib",
                &format!("--target={root}"),
                "--no-promote",
                &format!("--from={PROVENANCE}"),
                "--steer=- keep the wrapping add"
            ]
        );
        assert_eq!(key(&mut app, 'n'), Command::None, "n spawns nothing");
        assert_eq!(app.mode, Mode::Normal);
        // Retry: the attempt's own run shape.
        key(&mut app, 'r');
        let argv = confirm_argv(&app);
        assert_eq!(
            argv[..7],
            [
                HARNESS,
                "--json",
                "migrate",
                "u-lib",
                &format!("--target={root}"),
                "--no-promote",
                "--retry"
            ]
        );
        assert!(
            argv.contains(&"--provider=external".to_string()),
            "{argv:?}"
        );
        code(&mut app, KeyCode::Esc);
        // With --allow-unsandboxed the acts that run code pass it on.
        app.config.allow_unsandboxed = true;
        key(&mut app, 'a');
        assert_eq!(
            confirm_argv(&app).last().map(String::as_str),
            Some("--allow-unsandboxed")
        );
        code(&mut app, KeyCode::Esc);
        // Read-only without a harness binary.
        app.config.harness = None;
        key(&mut app, 'a');
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.notice.as_deref().unwrap().contains("read-only"));
    }

    #[test]
    fn acts_are_refused_with_a_reason_when_they_do_not_apply() {
        let mut app = app("refusals");
        // The superseded attempt is green but the unit's crate is another's;
        // its sibling rules still hold. A red attempt: no Accept.
        let unit = app.unit_view().unwrap().clone();
        let other = unit
            .attempts
            .iter()
            .find(|a| a.record.id != PROVENANCE)
            .unwrap()
            .record
            .id
            .clone();
        show(&mut app, &other);
        // A red attempt: no Accept, but it can seed a steer.
        let red = unit
            .attempts
            .iter()
            .find(|a| a.record.outcome == "red")
            .expect("the fixture has a red attempt")
            .record
            .id
            .clone();
        show(&mut app, &red);
        key(&mut app, 'a');
        assert_eq!(app.mode, Mode::Normal);
        assert!(app
            .notice
            .as_deref()
            .unwrap()
            .contains("only a green attempt"));
        // Modify needs a finished, bound attempt with a candidate and verdict.
        let ledger = harness_core::ledger::Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", &red);
        std::fs::remove_dir_all(dir.join("candidate")).unwrap();
        app.reload(true);
        show(&mut app, &red);
        key(&mut app, 'm');
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.notice.as_deref().unwrap().contains("no candidate"),
            "{:?}",
            app.notice
        );
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.driver = "blake3:another-driver".into();
        rec.store(&dir).unwrap();
        app.reload(true);
        show(&mut app, &red);
        key(&mut app, 'm');
        assert!(
            app.notice.as_deref().unwrap().contains("superseded inputs"),
            "{:?}",
            app.notice
        );
        show(&mut app, &other);
        // x with nothing running, R with nothing awaiting.
        assert_eq!(key(&mut app, 'x'), Command::None);
        assert!(app
            .notice
            .as_deref()
            .unwrap()
            .contains("nothing is running"));
        key(&mut app, 'R');
        assert!(app
            .notice
            .as_deref()
            .unwrap()
            .contains("nothing is awaiting"));
        // While a command runs, no act starts.
        app.running = true;
        key(&mut app, 'a');
        assert!(app
            .notice
            .as_deref()
            .unwrap()
            .contains("a command is running"));
        assert_eq!(key(&mut app, 'x'), Command::Cancel);
        // q asks; Q cancels and quits.
        assert_eq!(key(&mut app, 'q'), Command::None);
        assert_eq!(app.mode, Mode::QuitConfirm);
        assert_eq!(key(&mut app, 'Q'), Command::CancelAndQuit);
        app.running = false;
        assert_eq!(key(&mut app, 'q'), Command::Quit);
    }

    fn in_progress_attempt(app: &App, id: &str) {
        let ledger = harness_core::ledger::Ledger::new(&app.config.target);
        let seed = &app.unit_view().unwrap().attempt(PROVENANCE).unwrap().record;
        let record = AttemptRecord {
            id: id.into(),
            outcome: "in-progress".into(),
            turns: vec![],
            candidate_digest: String::new(),
            promoted: false,
            ..seed.clone()
        };
        let dir = attempts::attempt_dir(&ledger, "u-lib", id);
        std::fs::create_dir_all(&dir).unwrap();
        record.store(&dir).unwrap();
    }

    /// §4 Resume, §7: the watcher only marks "response present" (it never
    /// spawns); `R` asks and re-spawns the STORED argv; it stays available
    /// after a failed resume; a finished attempt ends it.
    #[test]
    fn resume_is_gated_on_state_and_survives_a_failed_resume() {
        use std::os::unix::process::ExitStatusExt;
        let mut app = app("resume");
        let awaited = "a-000000000abc";
        in_progress_attempt(&app, awaited);
        app.reload(true);
        let response = app.config.target.join("response.json");
        let argv: Vec<OsString> = [HARNESS, "--json", "migrate", "u-lib", "--no-promote"]
            .map(OsString::from)
            .to_vec();
        let pending = Pending {
            act: Act::Modify,
            argv: argv.clone(),
            cleanup: None,
            expect_attempt: None,
        };
        app.on_spawned(&pending);
        app.on_child_msg(ChildMsg::Event(Event::Awaiting {
            attempt: Some(awaited.into()),
            path: response.display().to_string(),
            resume: "harness migrate u-lib …".into(),
            args: None,
        }));
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        assert_eq!(app.awaiting.len(), 1);
        // No response yet, then a torn one: not resumable; the tick spawns
        // nothing either way (it has no way to).
        key(&mut app, 'R');
        assert!(app.notice.as_deref().unwrap().contains("no response yet"));
        std::fs::write(&response, "{\"text\": ").unwrap();
        app.tick();
        assert!(!app.awaiting[0].response_present);
        std::fs::write(&response, "{\"text\": \"x\"}").unwrap();
        app.tick();
        assert!(app.awaiting[0].response_present);
        assert_eq!(app.mode, Mode::Normal, "the watcher never spawns or asks");
        // R: the stored argv, unchanged, expecting the awaited attempt.
        key(&mut app, 'R');
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("R must ask, then spawn");
        };
        assert_eq!(p.argv, argv);
        assert_eq!(p.expect_attempt.as_deref(), Some(awaited));
        // The resumed run fails without a new hand-off: R stays available.
        app.on_spawned(&p);
        app.on_child_msg(ChildMsg::Event(Event::TurnStart {
            unit: "u-lib".into(),
            attempt: "a-ffffffffffff".into(),
            index: 1,
            kind: "repair".into(),
        }));
        assert!(app
            .run
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .any(|l| l.text.contains("not the awaited")));
        app.on_child_msg(ChildMsg::Event(Event::Error {
            kind: "harness".into(),
            message: "boom".into(),
            holder: None,
        }));
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        key(&mut app, 'R');
        assert!(matches!(app.mode, Mode::Confirm(_)), "{:?}", app.notice);
        code(&mut app, KeyCode::Esc);
        // The attempt finished (another answerer resumed it): gone.
        let ledger = harness_core::ledger::Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", awaited);
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.outcome = "red".into();
        rec.store(&dir).unwrap();
        app.tick();
        assert!(app.awaiting.is_empty());
        key(&mut app, 'R');
        assert!(app
            .notice
            .as_deref()
            .unwrap()
            .contains("nothing is awaiting"));
    }

    /// §4: the ledger is re-read after the command is REAPED — never on its
    /// `result` event (the CLI emits it before dying on a signal).
    #[test]
    fn the_ledger_is_reread_after_reaping_not_on_result() {
        use std::os::unix::process::ExitStatusExt;
        let mut app = app("reap");
        let late = "a-0000000000ff";
        let pending = Pending {
            act: Act::Retry,
            argv: vec![OsString::from(HARNESS)],
            cleanup: None,
            expect_attempt: None,
        };
        app.on_spawned(&pending);
        in_progress_attempt(&app, late);
        app.on_child_msg(ChildMsg::Event(Event::Result {
            exit: 130,
            signal: Some("SIGINT".into()),
        }));
        assert!(
            app.unit_view().unwrap().attempt(late).is_none(),
            "re-read on `result`"
        );
        app.on_child_exit(ExitStatus::from_raw(2));
        assert!(app.unit_view().unwrap().attempt(late).is_some());
        let run = app.run.as_ref().unwrap();
        assert_eq!(run.exit.as_deref(), Some("interrupted (SIGINT)"));
        // Without a `result` the run says so.
        app.on_spawned(&pending);
        app.on_child_exit(ExitStatus::from_raw(3 << 8));
        assert_eq!(
            app.run.as_ref().unwrap().exit.as_deref(),
            Some("exited without result (exit 3)")
        );
    }

    #[test]
    fn navigation_keeps_to_the_panes() {
        let mut app = app("nav");
        let n = app.unit_view().unwrap().attempts.len();
        code(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Rail);
        for _ in 0..n + 5 {
            key(&mut app, 'j');
        }
        assert_eq!(app.rail, n, "the cursor stops at the last attempt");
        code(&mut app, KeyCode::Tab);
        app.layout = Layout {
            pair_rows: vec![0, 10, 25],
            total_rows: 40,
            page: 10,
        };
        key(&mut app, ']');
        key(&mut app, 'f');
        assert_eq!(app.scroll, 10);
        key(&mut app, ']');
        key(&mut app, 'f');
        assert_eq!(app.scroll, 25);
        key(&mut app, '[');
        key(&mut app, 'f');
        assert_eq!(app.scroll, 10);
        // J/K stay within the plan's units.
        let before = app.unit;
        key(&mut app, 'K');
        assert_eq!(app.unit, before);
        // v opens the verdict of what is shown; ? the keys.
        key(&mut app, 'v');
        assert!(matches!(app.mode, Mode::Verdict { selected: 0, .. }));
        code(&mut app, KeyCode::Esc);
        key(&mut app, '?');
        assert_eq!(app.mode, Mode::Help { scroll: 0 });
        key(&mut app, 'j');
        assert_eq!(app.mode, Mode::Help { scroll: 1 }, "j scrolls the help");
        key(&mut app, 'x');
        assert_eq!(app.mode, Mode::Normal, "any other key closes it");
    }

    #[test]
    fn diff_compares_with_the_provenance_attempt() {
        let mut app = app("diff");
        key(&mut app, 'd');
        assert!(app.notice.as_deref().unwrap().contains("select an attempt"));
        show(&mut app, PROVENANCE);
        key(&mut app, 'd');
        let Mode::Diff { lines, .. } = &app.mode else {
            panic!("{:?}", app.notice);
        };
        assert_eq!(lines, &["the Rust sources are identical"]);
    }

    /// §4 `e`, §R2 5: after a changed edit the optional note is asked
    /// for, and travels attached. Declining keeps the edit (`E` offers it
    /// again, with its note); only an armed `D` discards it.
    #[test]
    fn a_staged_hand_edit_asks_for_its_note_then_its_argv() {
        let mut app = app("handedit");
        let root = app.config.target.display().to_string();
        let (stage, tmp) = (PathBuf::from("/tmp/h/stage"), PathBuf::from("/tmp/h"));
        app.edit_staged("u-lib".into(), stage.clone(), tmp.clone());
        for c in "-by hand".chars() {
            key(&mut app, c);
        }
        code(&mut app, KeyCode::Enter);
        let argv = confirm_argv(&app);
        assert_eq!(
            argv,
            [
                HARNESS,
                "--json",
                "override",
                "u-lib",
                "/tmp/h/stage",
                &format!("--target={root}"),
                "--note=-by hand"
            ]
        );
        let Mode::Confirm(p) = &app.mode else {
            unreachable!()
        };
        assert_eq!(p.cleanup.as_deref(), Some(tmp.as_path()));
        // Declined (n, Esc): kept, with its note.
        assert_eq!(key(&mut app, 'n'), Command::None);
        assert_eq!(app.kept_edits.len(), 1);
        assert_eq!(app.kept_paths(), [tmp.join("edit")]);
        key(&mut app, 'E');
        assert!(
            matches!(&app.mode, Mode::EditNote { input, .. } if input == "-by hand"),
            "{:?}",
            app.mode
        );
        assert_eq!(code(&mut app, KeyCode::Esc), Command::None);
        assert_eq!(app.kept_edits.len(), 1, "Esc keeps it too");
        // No note: no --note at all.
        app.edit_staged("u-lib".into(), stage.clone(), tmp.clone());
        code(&mut app, KeyCode::Enter);
        assert!(!confirm_argv(&app).iter().any(|a| a.starts_with("--note")));
        assert_eq!(app.kept_edits.len(), 1, "one edit, one entry");
        // D discards — only once the prompt is armed.
        assert_eq!(key(&mut app, 'D'), Command::None);
        assert!(matches!(app.mode, Mode::Confirm(_)));
        arm(&mut app);
        assert_eq!(key(&mut app, 'D'), Command::Cleanup(tmp.clone()));
        assert!(app.kept_edits.is_empty());
        assert_eq!(app.mode, Mode::Normal);
        key(&mut app, 'E');
        assert!(app.notice.as_deref().unwrap().contains("no kept hand edit"));
        // The shown crate must be in the executor layout for `e` at all.
        show(&mut app, PROVENANCE);
        assert!(matches!(key(&mut app, 'e'), Command::Edit { .. }));
    }

    /// Review ACTS-1: typed-ahead or pasted input never answers a prompt; a
    /// Ctrl- or Alt-modified key is never text and never a `y`.
    #[test]
    fn a_prompt_takes_only_a_considered_plain_y() {
        use ratatui::crossterm::event::KeyModifiers;
        let mut app = app("typeahead");
        show(&mut app, PROVENANCE);
        key(&mut app, 'a');
        assert!(matches!(app.mode, Mode::Confirm(_)));
        // The `y` of a burst, before the prompt was armed: refused.
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(matches!(app.mode, Mode::Confirm(_)));
        // Seen but not armed (input was pending): still refused.
        app.confirm_seen = true;
        assert!(app.confirm_waiting());
        assert_eq!(key(&mut app, 'y'), Command::None);
        // Armed: Ctrl-Y is not a `y`, a paste never answers.
        app.confirm_armed = true;
        let ctrl_y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(app.on_key(ctrl_y), Command::None);
        app.on_paste("y\ny");
        assert!(matches!(app.mode, Mode::Confirm(_)));
        assert!(matches!(key(&mut app, 'y'), Command::Spawn(_)));
        // Leaving a prompt disarms the next one.
        key(&mut app, 'a');
        assert!(!app.confirm_armed && !app.confirm_seen);
        code(&mut app, KeyCode::Esc);
        // In a note: a paste is text (line breaks as spaces), Ctrl-J is not.
        key(&mut app, 'm');
        app.on_paste("keep\r\nthe loop");
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(
            app.mode,
            Mode::Note {
                input: "keep  the loop".into()
            }
        );
    }

    /// Review ACTS-2: a note the CLI would refuse is refused at the prompt.
    #[test]
    fn notes_the_cli_refuses_are_refused_at_the_prompt() {
        assert!(note_problem("[TASK]", 400).is_some());
        assert!(note_problem("fine [not a header]", 400).is_none());
        assert!(note_problem(&"x".repeat(401), 400).is_some());
        let mut app = app("notes");
        app.edit_staged(
            "u-lib".into(),
            PathBuf::from("/tmp/n/stage"),
            PathBuf::from("/tmp/n"),
        );
        app.on_paste("[GUIDANCE]");
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::EditNote { .. }), "{:?}", app.mode);
        assert!(app.notice.as_deref().unwrap().contains("section header"));
        assert_eq!(
            code(&mut app, KeyCode::Esc),
            Command::None,
            "kept, not discarded"
        );
        show(&mut app, PROVENANCE);
        key(&mut app, 'm');
        app.on_paste("[TASK]");
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Note { .. }));
    }

    /// Review ACTS-2 / STATE-3 / PROC-1 (and the verification round): an
    /// override that recorded nothing (refused, interrupted, failed to
    /// start) keeps the edit, `E` offers it again, and only a recorded one
    /// frees its temp dir.
    #[test]
    fn an_unrecorded_override_keeps_the_edit_and_reoffers_it() {
        use std::os::unix::process::ExitStatusExt;
        let mut app = app("keepedit");
        let tmp = PathBuf::from("/tmp/k");
        app.edit_staged("u-lib".into(), tmp.join("stage"), tmp.clone());
        code(&mut app, KeyCode::Enter);
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn");
        };
        let argv = p.argv.clone();
        app.on_spawned(&p);
        app.on_child_msg(ChildMsg::Event(Event::Error {
            kind: "locked".into(),
            message: "ledger is locked".into(),
            holder: None,
        }));
        assert_eq!(
            app.on_child_exit(ExitStatus::from_raw(1 << 8)),
            None,
            "kept"
        );
        assert_eq!(app.kept_paths(), [tmp.join("edit")]);
        // `E` offers it again (its note, then the same command).
        key(&mut app, 'E');
        code(&mut app, KeyCode::Enter);
        assert_eq!(confirm_argv(&app), strs(&argv));
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn");
        };
        // A failed start keeps it too.
        app.on_spawn_failed(p.clone(), "no such file");
        assert_eq!(app.kept_edits.len(), 1);
        // Recorded (its `attempt` event): the temp dir goes.
        app.on_spawned(&p);
        app.on_child_msg(ChildMsg::Event(Event::Attempt {
            unit: "u-lib".into(),
            id: "a-000000000abc".into(),
            outcome: "red".into(),
            provider: "human".into(),
            model: "-".into(),
            promoted: false,
            promotion: "not promoted: override never promotes".into(),
        }));
        assert_eq!(app.on_child_exit(ExitStatus::from_raw(10 << 8)), Some(tmp));
        assert!(app.kept_edits.is_empty());
    }

    /// The verification round: a paste that does not fit says so.
    #[test]
    fn a_paste_that_does_not_fit_is_reported() {
        let mut app = app("pastefit");
        show(&mut app, PROVENANCE);
        key(&mut app, 'm');
        app.on_paste(&"x".repeat(MAX_NOTE_BYTES + 10));
        assert!(app.notice.as_deref().unwrap().contains("10 bytes dropped"));
    }

    /// The verification round (STATE-10): after a command is reaped the
    /// pairs are re-read even when nothing the fingerprint sees changed
    /// (the unit crate edited on disk).
    #[test]
    fn a_reaped_command_rereads_the_shown_crate() {
        use std::os::unix::process::ExitStatusExt;
        let mut app = app("reap-pairs");
        let crate_dir = app.unit_view().unwrap().crate_dir.clone().unwrap();
        let ffi = crate_dir.join("src/ffi.rs");
        let text = std::fs::read_to_string(&ffi).unwrap();
        std::fs::write(
            &ffi,
            text.replacen(
                "fn read_scalefactors(",
                "fn read_scalefactors( // on disk",
                1,
            ),
        )
        .unwrap();
        let pending = Pending {
            act: Act::Retry,
            argv: vec![OsString::from(HARNESS)],
            cleanup: None,
            expect_attempt: None,
        };
        app.on_spawned(&pending);
        app.on_child_exit(ExitStatus::from_raw(0));
        let seen = app.pairs.iter().any(|p| {
            p.rust.iter().any(|l| matches!(l, CodeLine::Code { pieces, .. } if pieces.iter().any(|(_, t)| t.contains("// on disk"))))
        });
        assert!(seen);
    }

    /// The verification round: the diff is bounded as a whole, and says
    /// what it cut.
    #[test]
    fn the_diff_is_bounded_as_a_whole() {
        let base = std::env::temp_dir().join(format!("harness-tui-diffcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (old, new) = (base.join("old"), base.join("new"));
        for d in [&old, &new] {
            std::fs::create_dir_all(d.join("src")).unwrap();
        }
        for i in 0..3 {
            let body: String = (0..9_000).map(|n| format!("line {n}\n")).collect();
            std::fs::write(old.join(format!("src/f{i}.rs")), &body).unwrap();
            std::fs::write(
                new.join(format!("src/f{i}.rs")),
                body.replace("line", "LINE"),
            )
            .unwrap();
        }
        let lines = diff_crates((&old, "a-old"), (&new, "a-new"));
        assert!(lines.len() <= MAX_DIFF_LINES + 1, "{}", lines.len());
        assert!(
            lines.last().unwrap().contains("the diff stops here"),
            "{:?}",
            lines.last()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Review ACTS-3: every outstanding hand-off is tracked; `R` resumes the
    /// shown attempt's.
    #[test]
    fn several_hand_offs_are_tracked() {
        use std::os::unix::process::ExitStatusExt;
        let mut app = app("handoffs");
        let (a, b) = ("a-00000000000a", "a-00000000000b");
        in_progress_attempt(&app, a);
        in_progress_attempt(&app, b);
        app.reload(true);
        for (id, n) in [(a, 1), (b, 2)] {
            let path = app.config.target.join(format!("r{n}.json"));
            std::fs::write(&path, "{}").unwrap();
            let pending = Pending {
                act: Act::Modify,
                argv: vec![OsString::from(format!("run-{n}"))],
                cleanup: None,
                expect_attempt: None,
            };
            app.on_spawned(&pending);
            app.on_child_msg(ChildMsg::Event(Event::Awaiting {
                attempt: Some(id.into()),
                path: path.display().to_string(),
                resume: String::new(),
                args: None,
            }));
            app.on_child_exit(ExitStatus::from_raw(1 << 8));
        }
        assert_eq!(app.awaiting.len(), 2);
        show(&mut app, a);
        key(&mut app, 'R');
        assert_eq!(confirm_argv(&app), ["run-1"]);
        code(&mut app, KeyCode::Esc);
        show(&mut app, b);
        key(&mut app, 'R');
        assert_eq!(confirm_argv(&app), ["run-2"]);
    }

    /// Review STATE-2, STATE-10: a reload keeps the selection by id, and
    /// the pairs follow the shown crate's bytes — on the 2 s tick too.
    #[test]
    fn reloads_keep_the_selection_and_refresh_changed_pairs() {
        let mut app = app("refresh");
        show(&mut app, PROVENANCE);
        let rail = app.rail;
        // Another attempt appears (sorting before it): selection kept by id.
        in_progress_attempt(&app, "a-000000000001");
        assert!(app.reload(false));
        assert_eq!(app.shown, Shown::Attempt(PROVENANCE.into()));
        assert_eq!(app.rail_attempt_id().as_deref(), Some(PROVENANCE));
        assert!(
            app.rail != rail || app.unit_view().unwrap().attempts[rail - 1].record.id == PROVENANCE
        );
        // The shown attempt's candidate changes on disk (a judged turn):
        // the tick re-reads the pairs.
        let ledger = harness_core::ledger::Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", PROVENANCE);
        let ffi = dir.join("candidate/src/ffi.rs");
        let text = std::fs::read_to_string(&ffi).unwrap();
        std::fs::write(
            &ffi,
            text.replacen(
                "fn read_scalefactors(",
                "fn read_scalefactors( // changed",
                1,
            ),
        )
        .unwrap();
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.candidate_digest = "blake3:changed".into();
        rec.store(&dir).unwrap();
        app.running = true;
        app.tick();
        let seen = app.pairs.iter().any(|p| {
            p.rust.iter().any(|l| matches!(l, CodeLine::Code { pieces, .. } if pieces.iter().any(|(_, t)| t.contains("// changed"))))
        });
        assert!(seen, "the pairs still show the old candidate");
        // The shown attempt vanishes: back to the crate.
        std::fs::remove_dir_all(&dir).unwrap();
        app.reload(false);
        assert_eq!(app.shown, Shown::Crate);
    }

    /// Review STATE-1, STATE-9: the diff covers every file of both sides,
    /// whatever an earlier file's lines end with, and refuses huge files.
    #[test]
    fn the_diff_covers_every_file_and_refuses_huge_ones() {
        let base = std::env::temp_dir().join(format!("harness-tui-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (old, new) = (base.join("old"), base.join("new"));
        for d in [&old, &new] {
            std::fs::create_dir_all(d.join("src")).unwrap();
        }
        std::fs::write(old.join("src/ffi.rs"), "a\n").unwrap();
        std::fs::write(new.join("src/ffi.rs"), "// see src/logic.rs\n").unwrap();
        std::fs::write(old.join("src/logic.rs"), "b\n").unwrap();
        std::fs::write(new.join("src/logic.rs"), "c\n").unwrap();
        std::fs::write(new.join("src/extra.rs"), "d\n").unwrap();
        let lines = diff_crates((&old, "a-old"), (&new, "a-new"));
        for f in ["src/ffi.rs", "src/logic.rs", "src/extra.rs"] {
            assert!(
                lines.iter().any(|l| l.starts_with("+++") && l.ends_with(f)),
                "{f} missing: {lines:?}"
            );
        }
        std::fs::write(new.join("src/logic.rs"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
        let lines = diff_crates((&old, "a-old"), (&new, "a-new"));
        assert!(lines.iter().any(|l| l.contains("not diffed")), "{lines:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn short_ids_and_shell_lines() {
        assert_eq!(short_id("a-13c941dfff95"), "a-13c9");
        assert_eq!(short_id("a-13c941dfff95.r2"), "a-13c9.r2");
        assert_eq!(
            shell_line(&["harness", "--steer=it's", "a b", "--x"].map(OsString::from)),
            "harness '--steer=it'\\''s' 'a b' --x"
        );
    }
}
