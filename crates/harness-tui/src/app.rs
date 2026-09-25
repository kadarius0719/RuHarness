//! The cockpit's state and key handling (docs/COCKPIT-WRAPPER-DESIGN.md;
//! the engine of docs/TUI-DESIGN.md §2–§4 holds).
//!
//! One navigator — a tree of the target's C files and units, keyed by
//! [`Selection`] — one View showing the selection, and an always-visible
//! activity panel. [`App::on_key`] never performs an act: it returns a
//! [`Command`] for the event loop, and every act, quit and cancel goes
//! through an ARMED dialog ([`crate::dialog`]) that shows its exact argv —
//! there is no automatic spawn. The ledger is read on the loader thread
//! (the preflight first): after a spawned command is reaped, on `g`, and on
//! the 2 s tick while a command runs or a hand-off is outstanding.

use crate::dialog::{Choice, Dialog, Kind, Outcome};
use crate::events::{Event, EVENTS_SCHEMA, EVENTS_SCHEMA_VERSION};
use crate::files::{self, FileState, Files, TreeWalk, UnitState};
use crate::handedit;
use crate::highlight::{Highlighter, Lang, Pieces};
use crate::load::Read;
use crate::menu::{self, Action, Item};
use crate::model::{AttemptView, AuthorshipView, ProvenanceView, Snapshot, UnitView, NO_PLAN};
use crate::narrate::{Ending, Narrator};
use crate::pairs::{CSide, FunctionPair, RustNote, SourceSpan};
use crate::spawn::ChildMsg;
use crate::tree::{self, Expansion, Row, Selection};
use harness_core::attempts::{AttemptRecord, HUMAN_KIND};
use harness_core::ledger::{Holder, Ledger};
use harness_core::verdict::Verdict;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

/// The hand-off provider (a profile name or an adapter kind): its answers
/// are written by whoever answers the request file.
pub const EXTERNAL_PROVIDER: &str = "external";
/// Longest steer note accepted (the CLI's limit).
pub const MAX_NOTE_BYTES: usize = 2000;
/// Longest hand-edit note accepted (the CLI's limit).
pub const MAX_EDIT_NOTE_BYTES: usize = 400;
/// Run-panel lines kept.
const MAX_RUN_LINES: usize = 2000;
/// A notice clears on the next key or after this long.
pub const NOTICE_TTL: Duration = Duration::from_secs(8);
/// Largest file the View shows as C source; the rest is cut with a note.
pub const MAX_SOURCE_VIEW_BYTES: u64 = 1024 * 1024;

/// Wide or stacked pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Side by side when the View is wide enough, stacked below.
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
    /// The provider profiles a model act may use (`--provider`, repeatable;
    /// default `external` only). Modify passes the first; Retry runs only
    /// for a record whose provider is listed. The target's `harness.toml`
    /// never chooses the provider (docs/COCKPIT-WRAPPER-DESIGN.md §4.3).
    pub providers: Vec<String>,
}

/// The focused pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The file tree.
    Files,
    /// The View.
    View,
}

/// An act: every write is a spawned `harness --json …`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// `scan`: rewrites the facts.
    Scan,
    /// `plan`: reconciles the plan.
    Plan,
    /// `detect`: rewrites the observer findings.
    Detect,
    /// `verify <unit>`: Re-check with the oracle.
    Verify,
    /// `a`: promote a green attempt.
    Accept,
    /// `m`: a steer attempt seeded from an attempt.
    Modify,
    /// `e`: a labelled hand edit.
    HandEdit,
    /// `r`: the attempt's own run shape, `--retry`.
    Retry,
    /// `R`: the stored argv of the run that ended awaiting.
    Resume,
}

impl Act {
    /// Its name in the activity panel.
    pub fn label(self) -> &'static str {
        match self {
            Act::Scan => "Scan the project",
            Act::Plan => "Refresh the plan",
            Act::Detect => "Find hazards",
            Act::Verify => "Re-check",
            Act::Accept => "Accept",
            Act::Modify => "Modify",
            Act::HandEdit => "Hand edit",
            Act::Retry => "Retry",
            Act::Resume => "Resume",
        }
    }

    /// Its accelerator.
    pub fn accel(self) -> Option<&'static str> {
        match self {
            Act::Accept => Some("a"),
            Act::Modify => Some("m"),
            Act::HandEdit => Some("e"),
            Act::Retry => Some("r"),
            Act::Resume => Some("R"),
            Act::Scan | Act::Plan | Act::Detect | Act::Verify => None,
        }
    }

    /// It calls a model.
    pub fn model(self) -> bool {
        matches!(self, Act::Modify | Act::Retry | Act::Resume)
    }
}

/// A command waiting for its dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// Which act.
    pub act: Act,
    /// The exact argv, the resolved binary first.
    pub argv: Vec<OsString>,
    /// The act in words, for the activity panel ("Re-check u-lib").
    pub label: String,
    /// The unit it acts on, when it is unit-wide.
    pub unit: Option<String>,
    /// The attempt it acts on, when there is one.
    pub attempt: Option<String>,
    /// A hand edit's temp dir, removed once the override recorded it.
    pub cleanup: Option<PathBuf>,
    /// A resume: the attempt its first `turn-start` must name.
    pub expect_attempt: Option<String>,
    /// Modify: the note, remembered for its attempt whatever happens.
    pub note: Option<String>,
    /// Re-check: the unit crate's content hash the View showed when the
    /// dialog opened — confirm refuses unless the crate on disk still has it
    /// ("unchanged since shown"; captured once, never updated by a load).
    pub shown_digest: Option<String>,
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
    /// `/bin/kill -INT` the running command's group.
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

/// Why a read of the ledger was asked for (the requests fold, the strongest
/// reason winning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoadWhy {
    /// The 2 s tick while a command runs or a hand-off is outstanding.
    Tick,
    /// A spawned command was reaped: the pairs are re-read too.
    Reaped,
    /// `g`: the pairs are re-read, and the outcome is said.
    Key,
}

/// A read finished: which reason to apply it with, or `None` when it must
/// be dropped. `asked` holds the reasons of the reads asked for, by their
/// request number; the requests up to `seq` are answered by this read (the
/// loader folds them). A read is dropped when a LATER read was asked for
/// after a command was reaped or on `g` (review PROC-1): it started before
/// that command ended, and applying it would undo what the command did
/// (a hand-off just posed, the lock just released); its reasons carry over
/// to that later read.
pub fn fold_loaded(asked: &mut Vec<(u64, LoadWhy)>, seq: u64) -> Option<LoadWhy> {
    let why = asked
        .iter()
        .filter(|(s, _)| *s <= seq)
        .map(|(_, w)| *w)
        .max()
        .unwrap_or(LoadWhy::Tick);
    asked.retain(|(s, _)| *s > seq);
    match asked.iter_mut().find(|(_, w)| *w >= LoadWhy::Reaped) {
        Some((_, later)) => {
            *later = (*later).max(why);
            None
        }
        None => Some(why),
    }
}

/// What a dialog is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// An act (or a hand edit's override).
    Act(Pending),
    /// Quit while a command runs.
    Quit,
    /// Cancel the running command.
    Cancel,
}

/// An open dialog: its words and its latch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// The latch and the buttons.
    pub dialog: Dialog,
    /// A question naming the object.
    pub title: String,
    /// What it writes and changes, in words (untrusted values filtered by
    /// the view).
    pub body: Vec<String>,
    /// What it is for.
    pub purpose: Purpose,
}

/// The open menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// Its items.
    pub items: Vec<Item>,
    /// The focused item.
    pub focus: usize,
    /// The full reason of a greyed item `Enter` was pressed on.
    pub footer: Option<String>,
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
        /// The unit.
        unit: String,
        /// The attempt it seeds from.
        attempt: String,
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
    /// `Enter`: the action menu.
    Menu(Menu),
    /// An armed confirmation.
    Dialog(Box<Confirm>),
    /// `c`: the activity details (today's run panel).
    Details {
        /// First row shown.
        scroll: usize,
    },
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

/// The last (or current) spawned command.
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
    /// The plain-language narration.
    pub narrator: Narrator,
    /// When it started.
    pub started: Instant,
    /// `plan`'s change lines, for the summary notice.
    pub plan_changes: usize,
    /// The command as it was confirmed (Try again offers it again, whole).
    pub pending: Pending,
}

/// An `external` hand-off waiting for its response file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiting {
    /// The awaited attempt (none for triage).
    pub attempt: Option<String>,
    /// The response file.
    pub path: PathBuf,
    /// The argv of the run that ended awaiting (re-spawned by Resume).
    pub argv: Vec<OsString>,
    /// Its label.
    pub label: String,
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
    /// First row of each pair in the View.
    pub pair_rows: Vec<usize>,
    /// Rows the View has in total.
    pub total_rows: usize,
    /// Rows visible at once in the View.
    pub page: usize,
    /// Rows visible at once in the tree.
    pub tree_page: usize,
    /// Below 80 columns: one pane at a time.
    pub single_pane: bool,
}

/// What a click at a spot would mean (Build B): the view records each
/// clickable region as it draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    /// A tree row's node.
    Row(Selection),
    /// A pane.
    Pane(Focus),
    /// A menu item.
    MenuItem(usize),
    /// A dialog button.
    Button(usize),
    /// A hint-bar entry: the key it stands for.
    Hint(&'static str),
    /// Inside the dialog (a click there does nothing).
    Dialog,
}

/// A transient notice (activity row 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Its words (untrusted parts filtered by the view).
    pub text: String,
    /// When it was posted (it clears after [`NOTICE_TTL`]).
    pub at: Instant,
}

/// The View's C source of a file no unit owns (or a header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceView {
    /// Repo-relative path.
    pub path: String,
    /// Highlighted lines.
    pub lines: Vec<CodeLine>,
    /// What was cut, or why nothing is shown.
    pub note: Option<String>,
}

/// The cockpit.
#[derive(Debug)]
pub struct App {
    /// How it was started.
    pub config: Config,
    /// The ledger, as last read.
    pub snapshot: Snapshot,
    /// The source tree, as last walked.
    pub walk: TreeWalk,
    /// The states of files, functions and units.
    pub files: Files,
    /// What is selected.
    pub selection: Selection,
    /// Which nodes are folded or opened.
    pub expansion: Expansion,
    /// The flattened tree.
    pub rows: Vec<Row>,
    /// Where a jump came from (`Esc`/`Backspace` go back).
    pub back: Vec<Selection>,
    /// The focused pane.
    pub focus: Focus,
    /// First tree row shown.
    pub tree_offset: usize,
    /// First View row shown.
    pub scroll: usize,
    /// First View column shown (code lines).
    pub hscroll: usize,
    /// The View's links (the project summary's and a directory's files, the
    /// units), as the last draw laid them out.
    pub links: Vec<Selection>,
    /// The focused link in the View: none until the user moves to one, so
    /// `Enter` there opens the menu as the Next step says (review USE-13).
    pub link: Option<usize>,
    /// Overlay, menu or dialog.
    pub mode: Mode,
    /// A transient notice.
    pub notice: Option<Notice>,
    /// After `plan`: one summary line kept until the next command starts.
    pub plan_notice: Option<String>,
    /// The last (or current) spawned command.
    pub run: Option<RunPanel>,
    /// A command is running.
    pub running: bool,
    /// The idle line: "Last: …".
    pub last: Option<String>,
    /// `t`: the command to offer again after a `locked` refusal or a failed
    /// start.
    pub try_again: Option<Pending>,
    /// The outstanding `external` hand-offs this cockpit posed.
    pub awaiting: Vec<Awaiting>,
    /// The pairs the View shows, highlighted.
    pub pairs: Vec<PairView>,
    /// The unit crate's content hash the pairs were built from.
    pub pairs_digest: Option<String>,
    /// The unit whose crate the pairs show (none for an attempt's crate or
    /// C source).
    pub pairs_unit: Option<String>,
    /// The target's effective migrate model, as the last read found it
    /// (named in the model acts' dialogs).
    pub migrate_model: String,
    /// The C source the View shows (a file no unit owns, a header, an
    /// internal function's file).
    pub source: Option<SourceView>,
    /// The widest code line the View holds, in columns (tabs as 8):
    /// horizontal scroll stops there (review USE-14).
    pub code_cols: usize,
    /// What the last draw laid out.
    pub layout: Layout,
    /// Clickable regions of the last frame.
    pub hits: Vec<(ratatui::layout::Rect, Hit)>,
    /// Every staged hand edit not yet recorded, oldest first; never removed
    /// unless the override recorded it or the user discarded it.
    pub kept_edits: Vec<KeptEdit>,
    /// Temp dirs kept only for what an editor left in them.
    pub leftovers: Vec<PathBuf>,
    /// Modify's last note per attempt (cancelled, declined or refused).
    pub notes: BTreeMap<String, String>,
    /// The writer lock's live holder, as last read.
    pub holder: Option<Holder>,
    /// Why the writer lock could not be read, when it could not (busy).
    pub holder_error: Option<String>,
    /// A read of the ledger the event loop should start.
    pub load_request: Option<LoadWhy>,
    /// A read is under way (set by the event loop).
    pub loading: bool,
    /// The open diff wrapped at `.0` columns (a view cache).
    pub diff_rows: Option<(usize, Vec<ratatui::text::Line<'static>>)>,
    /// The time of the last input, as the event loop reported it.
    pub now: Instant,
    last_load_error: Option<String>,
    pairs_key: Option<String>,
    pending_bracket: Option<char>,
    highlighter: Highlighter,
    run_awaiting: Option<Awaiting>,
    plan_before: Option<usize>,
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

/// Whether the awaited response file is there to resume from: a regular
/// file within the ledger cap (never a FIFO, a device or a huge file read
/// on the UI thread), non-empty, that parses as a JSON object.
pub fn response_present(path: &Path) -> bool {
    let regular = std::fs::metadata(path)
        .is_ok_and(|m| m.is_file() && m.len() <= crate::preflight::MAX_LEDGER_FILE_BYTES);
    regular
        && std::fs::read(path).is_ok_and(|bytes| {
            !bytes.is_empty()
                && serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|v| v.is_object())
        })
}

/// An unseeded attempt of the `external` provider: its retry would pose a
/// BLIND hand-off, which only the audited protocol may answer (SAFE-3).
pub fn blind(r: &AttemptRecord) -> bool {
    r.seeded_from.is_none()
        && r.steer_note.is_none()
        && r.provider_kind != HUMAN_KIND
        && (r.provider == EXTERNAL_PROVIDER || r.provider_kind == EXTERNAL_PROVIDER)
}

/// Why Retry refuses `r` under `providers`, if it does: a hand edit, an
/// unfinished attempt, a half-seeded record (CHK-13), a blind hand-off
/// (SAFE-3), a provider not on the list (SAFE-12).
pub fn retry_refusal(r: &AttemptRecord, providers: &[String]) -> Option<String> {
    if r.provider_kind == HUMAN_KIND {
        return Some("a hand edit has no run to retry".into());
    }
    if r.outcome == "in-progress" {
        return Some(format!("attempt {} is not finished", r.id));
    }
    if r.seeded_from.is_some() != r.steer_note.is_some() {
        return Some(format!(
            "attempt {} records only half of a steer (its seed or its note): inconsistent, \
             not retried",
            r.id
        ));
    }
    if blind(r) {
        return Some(format!(
            "attempt {} is a blind `external` hand-off: only the audited protocol \
             (targets/tractor/handoff-tools) retries it",
            r.id
        ));
    }
    if !providers.contains(&r.provider) {
        return Some(format!(
            "provider `{}` is not allowed — start with `--provider {}`",
            r.provider, r.provider
        ));
    }
    None
}

fn notice(text: impl Into<String>) -> Option<Notice> {
    Some(Notice {
        text: text.into(),
        at: Instant::now(),
    })
}

impl App {
    /// A cockpit over what the first read found.
    pub fn new(config: Config, read: Read) -> App {
        let files = files::build(&read.snapshot, &read.walk);
        let holder = read.holder;
        let migrate_model = read.migrate_model;
        let mut app = App {
            config,
            snapshot: read.snapshot,
            walk: read.walk,
            files,
            selection: Selection::Project,
            expansion: Expansion::default(),
            rows: Vec::new(),
            back: Vec::new(),
            focus: Focus::Files,
            tree_offset: 0,
            scroll: 0,
            hscroll: 0,
            links: Vec::new(),
            link: None,
            mode: Mode::Normal,
            notice: None,
            plan_notice: None,
            run: None,
            running: false,
            last: None,
            try_again: None,
            awaiting: Vec::new(),
            pairs: Vec::new(),
            pairs_digest: None,
            pairs_unit: None,
            migrate_model,
            source: None,
            code_cols: 0,
            layout: Layout::default(),
            hits: Vec::new(),
            kept_edits: Vec::new(),
            leftovers: Vec::new(),
            notes: BTreeMap::new(),
            holder,
            holder_error: None,
            load_request: None,
            loading: false,
            diff_rows: None,
            now: Instant::now(),
            last_load_error: None,
            pairs_key: None,
            pending_bracket: None,
            highlighter: Highlighter::new(),
            run_awaiting: None,
            plan_before: None,
        };
        app.rebuild_rows();
        app.refresh_view(true);
        app
    }

    /// Post a transient notice (activity row 2).
    pub fn say(&mut self, text: impl Into<String>) {
        self.notice = notice(text);
    }

    // ----- the selection -------------------------------------------------

    /// The unit the selection belongs to.
    pub fn unit_view(&self) -> Option<&UnitView> {
        self.owning_unit(&self.selection)
            .and_then(|u| self.snapshot.units.get(u))
    }

    /// The selected attempt, when an attempt is selected.
    pub fn shown_attempt(&self) -> Option<&AttemptView> {
        match &self.selection {
            Selection::Attempt(_, id) => self.unit_view()?.attempt(id),
            _ => None,
        }
    }

    /// The crate the View's pairs read: an attempt's, else the unit crate.
    pub fn shown_crate(&self) -> Option<PathBuf> {
        match &self.selection {
            Selection::Attempt(..) => self.shown_attempt()?.crate_dir().map(Path::to_path_buf),
            _ => self.unit_view()?.crate_dir.clone(),
        }
    }

    /// The verdict of what is shown: an attempt's, else the unit's latest.
    pub fn shown_verdict(&self) -> Option<&Verdict> {
        match &self.selection {
            Selection::Attempt(..) => self.shown_attempt()?.verdict.as_ref(),
            _ => self.unit_view()?.verdict.as_ref(),
        }
    }

    /// The selected row, when it is shown.
    pub fn cursor(&self) -> Option<usize> {
        tree::row_of(&self.rows, &self.selection)
    }

    fn rebuild_rows(&mut self) {
        self.rows = tree::rows(&self.snapshot, &self.files, &self.walk, &self.expansion);
    }

    /// Select `sel` (revealing it), re-reading the View when it changed.
    pub fn select(&mut self, sel: Selection) {
        if sel == self.selection {
            return;
        }
        tree::reveal(&mut self.expansion, &sel);
        self.selection = sel;
        self.rebuild_rows();
        self.scroll = 0;
        self.hscroll = 0;
        self.link = None;
        self.refresh_view(false);
        // A function's source opens at the function (review USE-4) — on
        // selection only: a re-read keeps where the user scrolled (NEW-7).
        if let (Selection::Function(p, name), Some(_)) = (&self.selection, &self.source) {
            let line = self
                .files
                .file(p)
                .and_then(|f| f.functions.iter().find(|x| &x.name == name))
                .map_or(1, |x| x.line as usize);
            self.scroll = line.saturating_sub(1);
        }
    }

    /// A jump: the current selection goes onto the back stack.
    pub fn jump(&mut self, sel: Selection) {
        if sel != self.selection {
            self.back.push(self.selection.clone());
            self.select(sel);
        }
    }

    fn go_back(&mut self) -> bool {
        while let Some(prev) = self.back.pop() {
            if tree::exists(&self.snapshot, &self.files, &prev) {
                self.select(prev);
                return true;
            }
        }
        false
    }

    fn move_rows(&mut self, by: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selection().is_some())
            .map(|(i, _)| i)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let here = self.cursor().unwrap_or(0);
        let at = selectable.partition_point(|i| *i < here);
        let target = (at as isize + by).clamp(0, selectable.len() as isize - 1) as usize;
        if let Some(sel) = self.rows[selectable[target]].selection().cloned() {
            self.select(sel);
        }
    }

    fn set_open(&mut self, sel: &Selection, open: bool) {
        self.expansion.set(sel, open);
        self.rebuild_rows();
    }

    fn next_unit(&mut self, forward: bool) {
        let n = self.snapshot.units.len();
        if n == 0 {
            return;
        }
        let here = self.owning_unit(&self.selection);
        let next = match (here, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(u), true) => (u + 1).min(n - 1),
            (Some(u), false) => u.saturating_sub(1),
        };
        let id = self.snapshot.units[next].unit.id.clone();
        self.select(Selection::Unit(id));
    }

    // ----- reading -------------------------------------------------------

    /// Ask the event loop for a read of the ledger (folded with any request
    /// not yet started).
    pub fn request_load(&mut self, why: LoadWhy) {
        self.load_request = Some(self.load_request.map_or(why, |w| w.max(why)));
    }

    /// Re-read the ledger now, on this thread (the preflight, the snapshot,
    /// the walk): what a test does in place of the loader.
    pub fn reload(&mut self, pairs: bool) -> bool {
        self.load_request = None;
        let result = crate::load::read(&self.config.target);
        self.on_loaded(result, if pairs { LoadWhy::Key } else { LoadWhy::Tick })
    }

    /// Serve the pending load request now, on this thread (tests).
    pub fn load_now(&mut self) -> bool {
        match self.load_request.take() {
            Some(why) => {
                let result = crate::load::read(&self.config.target);
                self.on_loaded(result, why)
            }
            None => false,
        }
    }

    /// A read finished: the selection is kept by key (a vanished node
    /// gives way to its parent); after a reaped command or `g` the shown
    /// crate is re-read too; the awaited responses are looked for. A failed
    /// read keeps the last snapshot and says why — once, until the reason
    /// changes (`g` always says it). `false` when it failed.
    pub fn on_loaded(&mut self, result: Result<Read, String>, why: LoadWhy) -> bool {
        let read = match result {
            Ok(read) => read,
            Err(e) => {
                if why == LoadWhy::Key || self.last_load_error.as_deref() != Some(e.as_str()) {
                    self.notice = notice(format!("unreadable: {e}"));
                }
                self.last_load_error = Some(e);
                return false;
            }
        };
        self.last_load_error = None;
        self.snapshot = read.snapshot;
        self.walk = read.walk;
        self.holder = read.holder;
        self.holder_error = None;
        self.migrate_model = read.migrate_model;
        self.files = files::build(&self.snapshot, &self.walk);
        self.selection = tree::surviving(&self.snapshot, &self.files, &self.selection);
        // A finished (or vanished) attempt no longer awaits anything.
        let units = &self.snapshot.units;
        self.awaiting.retain(|aw| match &aw.attempt {
            Some(id) => units.iter().any(|u| {
                u.attempt(id)
                    .is_some_and(|a| a.record.outcome == "in-progress")
            }),
            None => true,
        });
        self.rebuild_rows();
        self.refresh_view(why >= LoadWhy::Reaped);
        self.check_response();
        if let Some(n) = self.plan_before.take() {
            self.plan_notice = Some(if n == 0 {
                "plan: no changes".into()
            } else {
                format!(
                    "plan changed {n} unit{} — review `git diff migration/plan.toml` (c for the \
                     lines)",
                    if n == 1 { "" } else { "s" }
                )
            });
        }
        if why == LoadWhy::Key {
            self.notice = notice("re-read");
        }
        true
    }

    /// Read the writer lock's live holder now (the menu opens with it, a
    /// dialog confirms with it).
    pub fn refresh_holder(&mut self) {
        match harness_core::status::live_holder(&Ledger::new(&self.config.target)) {
            Ok(holder) => {
                self.holder = holder;
                self.holder_error = None;
            }
            // Fail closed: a lock file that cannot be read is not "free"
            // (review SAFE-10).
            Err(e) => {
                self.holder = None;
                self.holder_error = Some(e.to_string());
            }
        }
    }

    /// The key of what the View shows, as far as the snapshot tells: a
    /// change re-reads it (Accept never promotes unseen code; an outside C
    /// edit shows the stale label, an outside Rust edit the new code).
    fn view_key(&self) -> String {
        let unit = self.unit_view();
        let stale = unit.map_or_else(String::new, |u| {
            let stale: &[String] = self
                .snapshot
                .facts_state
                .as_ref()
                .map_or(&[], |s| &s.stale_paths);
            u.unit
                .files
                .iter()
                .filter(|f| stale.contains(f))
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        });
        let crate_part = match self.shown_attempt() {
            Some(a) => {
                let r = &a.record;
                format!("{}|{}|{}", r.outcome, r.turns.len(), r.candidate_digest)
            }
            None => unit.map_or_else(String::new, |u| {
                format!(
                    "{}|{:?}|{}|{:?}",
                    u.report.status, u.crate_digest, u.report.source_fresh, u.provenance
                )
            }),
        };
        format!("{:?}|{stale}|{crate_part}", self.selection)
    }

    /// Recompute what the View shows when the selection or its inputs
    /// changed (or `force`).
    pub fn refresh_view(&mut self, force: bool) {
        let key = self.view_key();
        if !force && self.pairs_key.as_ref() == Some(&key) {
            return;
        }
        self.build_view(key);
        let lines = self
            .source
            .iter()
            .flat_map(|s| s.lines.iter())
            .chain(self.pairs.iter().flat_map(|p| p.c.iter().chain(&p.rust)));
        self.code_cols = lines.map(code_width).max().unwrap_or(0);
        self.hscroll = self.hscroll.min(self.code_cols);
    }

    fn build_view(&mut self, key: String) {
        self.pairs_key = Some(key);
        self.pairs.clear();
        self.pairs_digest = None;
        self.pairs_unit = None;
        self.source = None;
        let sel = self.selection.clone();
        // C source: a file no unit owns, a header, an internal function.
        let source_of = match &sel {
            Selection::File(p) => {
                // An owned file with no function in its unit (a header) has
                // no pairs to show: its source (review NEW-10).
                let pairs = self.files.file(p).is_some_and(|f| {
                    matches!(f.state, FileState::Owned(_)) && f.functions.iter().any(|x| x.in_unit)
                });
                (!pairs).then(|| p.clone())
            }
            Selection::Function(p, name) => {
                let public = self
                    .files
                    .file(p)
                    .is_some_and(|f| f.functions.iter().any(|x| &x.name == name && x.in_unit));
                (!public).then(|| p.clone())
            }
            _ => None,
        };
        if let Some(path) = source_of {
            self.source = Some(self.read_source(&path));
            return;
        }
        let Some(u) = self.owning_unit(&sel) else {
            return;
        };
        let unit = &self.snapshot.units[u];
        let crate_dir = self.shown_crate();
        let mut raw = self.snapshot.pairs(unit, crate_dir.as_deref());
        match &sel {
            Selection::File(p) => {
                let names: Vec<String> = self
                    .files
                    .file(p)
                    .map(|f| {
                        f.functions
                            .iter()
                            .filter(|x| x.in_unit)
                            .map(|x| x.name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                raw.retain(|pair| names.contains(&pair.symbol));
            }
            Selection::Function(_, name) => raw.retain(|pair| &pair.symbol == name),
            _ => {}
        }
        if !matches!(sel, Selection::Attempt(..)) {
            self.pairs_digest = unit.crate_digest.clone();
            self.pairs_unit = Some(unit.unit.id.clone());
        }
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

    /// A file's C source for the View: display-filtered by the view,
    /// highlighted, at most [`MAX_SOURCE_VIEW_BYTES`], a regular file only.
    fn read_source(&mut self, path: &str) -> SourceView {
        let full = self.config.target.join(path);
        let mut view = SourceView {
            path: path.to_string(),
            lines: Vec::new(),
            note: None,
        };
        let meta = match std::fs::metadata(&full) {
            Ok(m) => m,
            Err(e) => {
                view.note = Some(format!("{path}: {e}"));
                return view;
            }
        };
        if !meta.is_file() {
            view.note = Some(format!("{path} is not a regular file; not shown"));
            return view;
        }
        use std::io::Read as _;
        let mut bytes = Vec::new();
        let read = std::fs::File::open(&full)
            .and_then(|f| f.take(MAX_SOURCE_VIEW_BYTES).read_to_end(&mut bytes));
        if let Err(e) = read {
            view.note = Some(format!("{path}: {e}"));
            return view;
        }
        if meta.len() > MAX_SOURCE_VIEW_BYTES {
            view.note = Some(format!(
                "cut at {} KiB of {} KiB",
                MAX_SOURCE_VIEW_BYTES / 1024,
                meta.len() / 1024
            ));
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        view.lines = self
            .highlighter
            .lines(Lang::C, &lines)
            .into_iter()
            .enumerate()
            .map(|(i, pieces)| CodeLine::Code {
                number: i + 1,
                pieces,
            })
            .collect();
        view
    }

    // ----- the running command ------------------------------------------

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
        let ev = match msg {
            ChildMsg::Stderr(line) => {
                push(run, Tone::Dim, line);
                return;
            }
            ChildMsg::Eof(_) => return,
            ChildMsg::Event(ev) => ev,
        };
        run.narrator.on_event(&ev);
        match ev {
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
            Event::Message { text } => {
                if run.act == Act::Plan
                    && text.starts_with("plan: ")
                    && !text.starts_with("plan: no changes")
                    && !text.starts_with("plan: execution order")
                {
                    run.plan_changes += 1;
                }
                push(run, Tone::Plain, text)
            }
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
                // Only a steer attempt's hand-off is this cockpit's to resume:
                // a blind one belongs to the audited protocol (SAFE-12).
                let steer = run
                    .argv
                    .iter()
                    .any(|a| a.to_string_lossy().starts_with("--steer="));
                if !steer {
                    return;
                }
                self.run_awaiting = Some(Awaiting {
                    attempt,
                    path: PathBuf::from(path),
                    argv: run.argv.clone(),
                    label: run.narrator.label.clone(),
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
        }
    }

    /// A confirmed act was spawned.
    pub fn on_spawned(&mut self, pending: &Pending) {
        self.running = true;
        self.run_awaiting = None;
        self.try_again = None;
        self.plan_notice = None;
        self.run = Some(RunPanel {
            argv: pending.argv.clone(),
            lines: Vec::new(),
            exit: None,
            saw_result: false,
            expect_attempt: pending.expect_attempt.clone(),
            act: pending.act,
            recorded: false,
            cleanup: pending.cleanup.clone(),
            narrator: Narrator::new(&pending.label, &pending.argv),
            started: Instant::now(),
            plan_changes: 0,
            pending: pending.clone(),
        });
        self.notice = None;
    }

    /// The confirmed act could not be started (a hand edit stays kept):
    /// `t` offers it again.
    pub fn on_spawn_failed(&mut self, pending: Pending, why: &str) {
        self.notice = notice(if pending.act == Act::HandEdit {
            format!("could not start the command: {why}; the hand edit is kept — E offers it again")
        } else {
            format!("could not start the command: {why} — t tries again")
        });
        self.try_again = Some(pending);
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
        // Its Try again would offer an edit that is gone (review NEW-8).
        if self
            .try_again
            .as_ref()
            .is_some_and(|p| p.cleanup.as_deref() == Some(tmp))
        {
            self.try_again = None;
        }
    }

    /// The spawned command is over (both pipes at EOF, reaped): say how it
    /// ended, and ask for a read of the ledger. Returns the hand-edit temp
    /// dir to remove — only when the override RECORDED the edit.
    pub fn on_child_exit(&mut self, status: ExitStatus) -> Option<PathBuf> {
        use std::os::unix::process::ExitStatusExt;
        self.running = false;
        let signal = status.signal().map(signal_name);
        if let Some(run) = self.run.as_mut() {
            let text = match (status.code(), &signal) {
                (_, Some(sig)) => format!("interrupted ({sig})"),
                (Some(code), _) if !run.saw_result => {
                    format!("exited without result (exit {code})")
                }
                (Some(code), _) => format!("exit {code}"),
                (None, None) => "ended".into(),
            };
            run.exit = Some(text);
            self.last = Some(run.narrator.last(
                status.code(),
                signal.as_deref(),
                run.started.elapsed(),
            ));
            let ending = run.narrator.ending(status.code(), signal.as_deref());
            if ending == Ending::Locked {
                // The same command, whole: its unit, attempt, note and hand
                // edit (review USE-2/ENG-1/SAFE-4). Its dialog re-checks
                // everything again at confirm.
                let mut again = run.pending.clone();
                again.cleanup = run.cleanup.clone();
                self.try_again = Some(again);
            }
            if run.act == Act::Plan && ending == Ending::Done {
                self.plan_before = Some(run.plan_changes);
            }
        }
        if let Some(aw) = self.run_awaiting.take() {
            // One entry per awaited attempt.
            self.awaiting.retain(|o| o.attempt != aw.attempt);
            self.awaiting.push(aw);
        }
        let mut remove = None;
        if let Some(run) = self.run.as_mut() {
            if let (Act::HandEdit, Some(tmp)) = (run.act, run.cleanup.take()) {
                if run.recorded {
                    self.kept_edits.retain(|k| k.tmp != tmp);
                    if let Some(t) = self.try_again.as_mut() {
                        t.cleanup = None;
                    }
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
        self.request_load(LoadWhy::Reaped);
        remove
    }

    fn check_response(&mut self) {
        for aw in &mut self.awaiting {
            aw.response_present = response_present(&aw.path);
        }
    }

    /// The 2 s watcher: while a command runs or a hand-off is outstanding,
    /// ask for a read (on the loader). Never spawns.
    pub fn tick(&mut self) {
        if self.running || !self.awaiting.is_empty() {
            self.request_load(LoadWhy::Tick);
        }
    }

    // ----- the argv of every act ------------------------------------------

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

    /// The argv of an act on the project, a unit (by id) or one of its
    /// attempts, or why it is not available — the ONE argv builder: the
    /// menu, the accelerators and the dialogs all use it, and no tree path
    /// ever enters it.
    pub fn act_argv(
        &self,
        act: Act,
        unit: Option<&str>,
        attempt: Option<&str>,
        note: Option<&str>,
    ) -> Result<Pending, String> {
        if self.running {
            return Err("a command is running (x cancels it)".into());
        }
        // Ids reach the argv as positionals: plain path segments only.
        for id in [unit, attempt].into_iter().flatten() {
            if !harness_core::plan::is_clean_segment(id) {
                return Err(format!("{id:?} is not a plain id; not passed to a command"));
            }
        }
        let find_unit = || -> Result<&UnitView, String> {
            let id = unit.ok_or("no unit")?;
            self.snapshot
                .unit(id)
                .ok_or_else(|| format!("unit {id} is gone"))
        };
        let find_attempt = || -> Result<(&UnitView, &AttemptView), String> {
            let u = find_unit()?;
            let id = attempt.ok_or("select an attempt first")?;
            u.attempt(id)
                .map(|a| (u, a))
                .ok_or_else(|| format!("attempt {id} is gone"))
        };
        let pending = |argv, label: String, unit: Option<&str>, attempt: Option<&str>| Pending {
            act,
            argv,
            label,
            unit: unit.map(str::to_string),
            attempt: attempt.map(str::to_string),
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
        };
        match act {
            Act::Scan | Act::Plan | Act::Detect => {
                let sub = match act {
                    Act::Scan => "scan",
                    Act::Plan => "plan",
                    _ => "detect",
                };
                let argv = self.harness_argv(&[os(sub), self.target_arg()])?;
                Ok(pending(argv, act.label().to_string(), None, None))
            }
            Act::Verify => {
                let u = find_unit()?;
                let argv = self.with_sandbox_flag(self.harness_argv(&[
                    os("verify"),
                    os(&u.unit.id),
                    self.target_arg(),
                ])?);
                Ok(pending(
                    argv,
                    format!("Re-check {}", u.unit.id),
                    Some(&u.unit.id),
                    None,
                ))
            }
            Act::Accept => {
                let (u, a) = find_attempt()?;
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
                    r.promoted || matches!(u.report.status.as_str(), "verified" | "merged");
                let mut rest = vec![os("promote"), os(&u.unit.id), os(&r.id), self.target_arg()];
                if replace {
                    rest.push(os("--replace"));
                }
                Ok(pending(
                    self.with_sandbox_flag(self.harness_argv(&rest)?),
                    format!("Accept {} into {}", short_id(&r.id), u.unit.id),
                    Some(&u.unit.id),
                    Some(&r.id),
                ))
            }
            Act::Modify => {
                let (u, a) = find_attempt()?;
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
                let note = note.ok_or("no note")?;
                let provider = self
                    .config
                    .providers
                    .first()
                    .ok_or("no provider is allowed (start with --provider <name>)")?;
                let mut from = os("--from=");
                from.push(&r.id);
                let mut steer = os("--steer=");
                steer.push(note);
                // The model the dialog names is the one that runs: pinned
                // from the last read, never re-chosen by a harness.toml edited
                // meanwhile (review NEW-3).
                let rest = vec![
                    os("migrate"),
                    os(&u.unit.id),
                    self.target_arg(),
                    os("--no-promote"),
                    os(format!("--provider={provider}")),
                    os(format!("--model={}", self.migrate_model)),
                    from,
                    steer,
                ];
                let mut p = pending(
                    self.with_sandbox_flag(self.harness_argv(&rest)?),
                    format!("Modify {}", short_id(&r.id)),
                    Some(&u.unit.id),
                    Some(&r.id),
                );
                p.note = Some(note.to_string());
                Ok(p)
            }
            Act::Retry => {
                let (u, a) = find_attempt()?;
                let r = &a.record;
                if let Some(why) = retry_refusal(r, &self.config.providers) {
                    return Err(why);
                }
                let mut rest = vec![
                    os("migrate"),
                    os(&u.unit.id),
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
                    format!("Retry {}", short_id(&r.id)),
                    Some(&u.unit.id),
                    Some(&r.id),
                ))
            }
            Act::Resume => {
                let id = attempt.ok_or("select the paused attempt first")?;
                let aw = self
                    .awaiting
                    .iter()
                    .find(|aw| aw.attempt.as_deref() == Some(id))
                    .ok_or_else(|| {
                        format!("attempt {id} is not waiting on a hand-off this cockpit posed")
                    })?;
                let in_progress = self.snapshot.units.iter().any(|u| {
                    u.attempt(id)
                        .is_some_and(|a| a.record.outcome == "in-progress")
                });
                if !in_progress {
                    return Err(format!("attempt {id} is no longer in progress"));
                }
                if !response_present(&aw.path) {
                    return Err(format!(
                        "no response yet at {} (it must be a JSON object)",
                        aw.path.display()
                    ));
                }
                let mut p = pending(
                    aw.argv.clone(),
                    format!("Resume {}", short_id(id)),
                    unit,
                    Some(id),
                );
                p.expect_attempt = Some(id.to_string());
                Ok(p)
            }
            Act::HandEdit => Err("the hand edit is prepared by the editor flow".into()),
        }
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
            label: format!("Record the hand edit of {unit}"),
            unit: Some(unit.to_string()),
            attempt: None,
            cleanup: Some(tmp),
            expect_attempt: None,
            note: None,
            shown_digest: None,
        })
    }

    /// The unit and crate a hand edit of the selection would edit.
    pub fn hand_edit_target(&self) -> Result<(String, PathBuf), String> {
        if self.running {
            return Err("a command is running".into());
        }
        self.harness_argv(&[])?;
        let unit = self.unit_view().ok_or("no unit")?;
        let dir = self
            .shown_crate()
            .ok_or("nothing to edit: the crate does not exist")?;
        if !handedit::editable(&dir) {
            return Err(
                "this crate is not in the executor layout (src/logic.rs + src/ffi.rs)".into(),
            );
        }
        // The unit crate as it is NOW: a hand edit of code the harness does
        // not know would record all of it as a human's (review SAFE-9).
        if !matches!(self.selection, Selection::Attempt(..)) {
            let now = harness_core::hash::crate_content_hash(&dir)
                .map_err(|e| format!("the crate could not be read: {e}"))?;
            if unit.crate_digest.as_deref() != Some(now.as_str()) {
                return Err("the crate changed on disk since the cockpit read it — press g".into());
            }
            let known = self
                .snapshot
                .units
                .iter()
                .position(|u| u.unit.id == unit.unit.id)
                .and_then(|i| self.files.units.get(i))
                .is_some_and(|i| i.known_code);
            if !known {
                return Err(format!(
                    "{}'s crate holds code the harness does not know — record it with `harness \
                     override` or restore it first (see Help)",
                    unit.unit.id
                ));
            }
        }
        Ok((unit.unit.id.clone(), dir))
    }

    /// Whether `d` can compare `a` with the provenance attempt.
    pub fn diff_available(&self, unit: &UnitView, a: &AttemptView) -> bool {
        unit.provenance.attempt().is_some_and(|p| {
            p != a.record.id && unit.attempt(p).is_some_and(|b| b.crate_dir().is_some())
        }) && a.crate_dir().is_some()
    }

    fn diff(&self) -> Result<Mode, String> {
        let unit = self.unit_view().ok_or("no unit")?;
        let shown = self.shown_attempt().ok_or("select an attempt to compare")?;
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

    // ----- dialogs ---------------------------------------------------------

    /// The words of an act's dialog: a question naming the object, and every
    /// file it writes and what changes (§5.1).
    pub fn dialog_words(&self, p: &Pending) -> (String, Vec<String>) {
        let unit = p.unit.as_deref().unwrap_or("the unit");
        // Full ids in the dialog: a short id may name two attempts (SAFE-13).
        let attempt = p.attempt.clone().unwrap_or_default();
        let routing = || self.migrate_model.clone();
        let (title, mut body) = match p.act {
            Act::Scan => (
                "Scan the project?".to_string(),
                vec![
                    "Reads every C file and rewrites migration/facts.jsonl.".into(),
                    "A changed C file makes the verdicts that used it out of date.".into(),
                ],
            ),
            Act::Plan => (
                "Refresh the plan?".into(),
                vec![
                    "Rewrites migration/plan.toml: re-approves the changed sources of every \
                     unit whose C changed, verified units included; adds and removes units; \
                     blocks units whose files left."
                        .into(),
                    "Review `git diff migration/plan.toml` afterwards.".into(),
                ],
            ),
            Act::Detect => (
                "Find hazards in the project?".into(),
                vec!["Runs the detectors and rewrites the observer findings.".into()],
            ),
            Act::Verify => (
                format!("Re-check {unit} with the oracle?"),
                vec![
                    if self.config.allow_unsandboxed {
                        format!("Builds {unit}'s Rust and runs it against the C.")
                    } else {
                        format!("Builds {unit}'s Rust and runs it against the C in the sandbox.")
                    },
                    format!(
                        "Writes units/{unit}/oracle-latest.json and .md (and \
                         oracle-last-green.json when green) and {unit}'s status in plan.toml: \
                         green → verified; red → a verified unit becomes \"in progress\" (a \
                         merged one keeps its status)."
                    ),
                    "Changes no code. The crate is unchanged since the cockpit last showed it."
                        .into(),
                ],
            ),
            Act::Accept => {
                let replace = p.argv.iter().any(|a| a == "--replace");
                (
                    if replace {
                        format!("Replace {unit}'s verified crate with {attempt}?")
                    } else {
                        format!("Accept {attempt} into {unit}?")
                    },
                    vec![
                        format!(
                            "Replaces {unit}'s crate with {attempt}'s candidate and verifies it \
                             in place; writes the oracle files and {unit}'s status in plan.toml, \
                             and marks the attempt promoted."
                        ),
                        "If it does not verify in place, the old crate is put back.".into(),
                    ],
                )
            }
            Act::Modify | Act::Retry | Act::Resume => {
                let provider = p.argv.iter().find_map(|a| {
                    a.to_string_lossy()
                        .strip_prefix("--provider=")
                        .map(str::to_string)
                });
                let model = p.argv.iter().find_map(|a| {
                    a.to_string_lossy()
                        .strip_prefix("--model=")
                        .map(str::to_string)
                });
                let (provider, model) = match p.act {
                    Act::Retry => (
                        format!("{} (from the attempt record)", provider.unwrap_or_default()),
                        format!("{} (from the attempt record)", model.unwrap_or_default()),
                    ),
                    // Modify pins the target's migrate model as last read.
                    Act::Modify => (
                        provider.unwrap_or_default(),
                        format!(
                            "{} (the target's migrate routing)",
                            model.unwrap_or_else(routing)
                        ),
                    ),
                    _ => (
                        provider.unwrap_or_else(|| "as the paused run".into()),
                        model.unwrap_or_else(|| "as the paused run".into()),
                    ),
                };
                let title = match p.act {
                    Act::Modify => format!("Modify {attempt} with your note?"),
                    Act::Retry => format!("Retry {attempt}?"),
                    _ => format!("Resume {attempt} with the hand-off's answer?"),
                };
                let mut body = vec![
                    format!("A model call: provider {provider}, model {model}."),
                    format!(
                        "Records a new attempt of {unit}; never promotes it. Can take minutes."
                    ),
                ];
                if p.act == Act::Modify {
                    if let Some(n) = &p.note {
                        body.push(format!("Your note: {n}"));
                    }
                }
                (title, body)
            }
            Act::HandEdit => (
                format!("Record your hand edit of {unit}?"),
                vec![
                    "Records a human attempt, judged by the oracle; never promotes it.".into(),
                    "The edit is staged as exactly src/logic.rs + src/ffi.rs. Keep for later \
                     keeps it (E offers it again); Discard removes it."
                        .into(),
                ],
            ),
        };
        if p.argv.iter().any(|a| a == "--allow-unsandboxed") {
            body.push("--allow-unsandboxed: runs code WITHOUT the sandbox.".into());
        }
        (title, body)
    }

    fn open_dialog(&mut self, purpose: Purpose) {
        let (kind, title, body) = match &purpose {
            Purpose::Act(p) => {
                let (title, body) = self.dialog_words(p);
                let kind = if p.act == Act::HandEdit {
                    Kind::Override
                } else {
                    Kind::Act
                };
                (kind, title, body)
            }
            Purpose::Quit => (
                Kind::Quit,
                "Quit while a command runs?".into(),
                vec![
                    "A command is running.".into(),
                    "Quit, let it finish: it runs on to its end on its own.".into(),
                    "Stop it and quit: it is interrupted (SIGINT) first.".into(),
                ],
            ),
            Purpose::Cancel => (
                Kind::Cancel,
                "Stop the running command?".into(),
                vec![
                    "It is interrupted (SIGINT to its process group) and cleans up; what it \
                     finished stays recorded."
                        .into(),
                ],
            ),
        };
        self.mode = Mode::Dialog(Box::new(Confirm {
            dialog: Dialog::new(kind, self.now),
            title,
            body,
            purpose,
        }));
    }

    /// Open an act's dialog (nothing runs yet). A Re-check captures the
    /// crate digest the View shows now: confirm compares the disk with it.
    pub fn ask(&mut self, mut pending: Pending) {
        if pending.act == Act::Verify && pending.shown_digest.is_none() {
            pending.shown_digest = match (&pending.unit, &self.pairs_unit) {
                (Some(u), Some(shown)) if u == shown => self.pairs_digest.clone(),
                _ => None,
            };
            // Nothing shown, nothing to re-check (review NEW-2/NEW-8): say
            // so now rather than refuse at confirm with a false reason.
            if pending.shown_digest.is_none() {
                let unit = pending.unit.as_deref().unwrap_or("the unit");
                self.notice = notice(format!(
                    "{}: open {unit} (or its crate) first — the cockpit re-checks only code it \
                     shows",
                    pending.label
                ));
                return;
            }
        }
        self.open_dialog(Purpose::Act(pending));
    }

    /// The event loop's check after a draw: arm the open dialog when drawn
    /// whole, quiet for 300 ms, with no input `pending`.
    pub fn arm(&mut self, now: Instant, pending: bool) {
        if let Mode::Dialog(c) = &mut self.mode {
            c.dialog.arm(now, pending);
        }
    }

    /// A dialog waits to be armed (the loop polls for pending input then).
    pub fn dialog_waiting(&self) -> bool {
        matches!(&self.mode, Mode::Dialog(c) if c.dialog.waiting())
    }

    /// An input event was read at `now` (the dialog's quiet time restarts).
    pub fn on_input(&mut self, now: Instant) {
        self.now = now;
        if let Mode::Dialog(c) = &mut self.mode {
            c.dialog.input(now);
        }
    }

    /// The cockpit's own gates, again at confirm time on a FRESH read (§4.3):
    /// the preflight first (every read below is then of a regular file within
    /// its cap, and the crate holds no link — review SAFE-2), the lock
    /// holder; Re-check only on the crate the plan names NOW, known code,
    /// unchanged since the dialog opened; Retry's refusals and Resume's
    /// conditions on the record as it is now.
    fn confirm_gate(&mut self, p: &Pending) -> Result<(), String> {
        self.refresh_holder();
        if self.running {
            return Err("a command is running (one at a time)".into());
        }
        if let Some(why) = self.busy() {
            return Err(why);
        }
        let target = self.config.target.clone();
        // The confirms below read the ledger and hash a crate: the preflight
        // first, so each read is of a regular file within its cap and the
        // crate holds no link (review SAFE-2). The other acts read nothing
        // here — a broken file never refuses the Scan that could repair it
        // (review NEW-6).
        if matches!(p.act, Act::Verify | Act::Accept | Act::Retry | Act::Resume) {
            crate::preflight::preflight(&target)
                .map_err(|why| format!("the project cannot be read safely: {why}"))?;
        }
        let ledger = Ledger::new(&target);
        let record = |unit: &str, attempt: &str| {
            let dir = harness_core::attempts::attempt_dir(&ledger, unit, attempt);
            AttemptRecord::load(&dir)
                .map_err(|e| format!("attempt {attempt} could not be read: {e}"))
        };
        match p.act {
            Act::Verify => {
                let id = p.unit.as_deref().ok_or("no unit")?;
                // The crate `verify` will judge: the one the plan names now.
                let plan = harness_core::plan::Plan::load(&ledger.plan_path())
                    .map_err(|e| format!("the plan could not be read: {e}"))?;
                let dir = plan
                    .units
                    .iter()
                    .find(|u| u.id == id)
                    .and_then(|u| u.oracle_param_str("rust_crate"))
                    .map(|name| ledger.unit_dir(id).join(name))
                    .ok_or_else(|| format!("{id} has no crate in the plan"))?;
                let shown = self.snapshot.unit(id).and_then(|u| u.crate_dir.clone());
                if shown.as_deref() != Some(dir.as_path()) {
                    return Err(format!(
                        "the plan now names another crate for {id} — press g and look again"
                    ));
                }
                let now = harness_core::hash::crate_content_hash(&dir)
                    .map_err(|e| format!("the crate could not be read: {e}"))?;
                if p.shown_digest.as_deref() != Some(now.as_str()) {
                    return Err(
                        "the crate changed on disk since the cockpit showed it — press g and \
                         look again"
                            .into(),
                    );
                }
                self.known_now(id, &dir, &now)
            }
            // Accept replaces the unit crate: never code the harness does
            // not know, checked on the crate as it is now (review SAFE-8).
            Act::Accept => {
                let id = p.unit.as_deref().ok_or("no unit")?;
                let Some(dir) = self.snapshot.unit(id).and_then(|u| u.crate_dir.clone()) else {
                    return Ok(());
                };
                if !dir.join("Cargo.toml").is_file() {
                    return Ok(());
                }
                let now = harness_core::hash::crate_content_hash(&dir)
                    .map_err(|e| format!("the crate could not be read: {e}"))?;
                self.known_now(id, &dir, &now).map_err(|_| {
                    format!(
                        "{id}'s crate holds code the harness does not know — Accept would \
                         replace it; record it with `harness override` or restore it first (see \
                         Help)"
                    )
                })
            }
            Act::Retry => {
                let (Some(u), Some(a)) = (p.unit.as_deref(), p.attempt.as_deref()) else {
                    return Err("no attempt".into());
                };
                match retry_refusal(&record(u, a)?, &self.config.providers) {
                    Some(why) => Err(why),
                    None => Ok(()),
                }
            }
            Act::Resume => {
                let (Some(u), Some(a)) = (p.unit.as_deref(), p.attempt.as_deref()) else {
                    return Err("no attempt".into());
                };
                let r = record(u, a)?;
                if r.outcome != "in-progress" {
                    return Err(format!("attempt {a} is no longer in progress"));
                }
                // Only a steer attempt's hand-off: a blind one is the audited
                // protocol's (SAFE-12).
                if r.seeded_from.is_none() || r.steer_note.is_none() {
                    return Err(format!(
                        "attempt {a} is not a steer attempt: its hand-off is the audited \
                         protocol's to answer"
                    ));
                }
                let aw = self
                    .awaiting
                    .iter()
                    .find(|aw| aw.attempt.as_deref() == Some(a))
                    .ok_or_else(|| format!("attempt {a} is not waiting on this cockpit"))?;
                if !response_present(&aw.path) {
                    return Err(format!("no response yet at {}", aw.path.display()));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Whether the unit crate at `dir`, whose content hash is `now`, is code
    /// the harness knows — read fresh: a recorded attempt's candidate, or
    /// what the oracle last judged (§4.3).
    fn known_now(&self, id: &str, dir: &Path, now: &str) -> Result<(), String> {
        let ledger = Ledger::new(&self.config.target);
        let recorded = self.snapshot.unit(id).is_some_and(|unit| {
            unit.attempts.iter().any(|a| {
                let fresh = AttemptRecord::load(&harness_core::attempts::attempt_dir(
                    &ledger,
                    id,
                    &a.record.id,
                ));
                fresh.is_ok_and(|r| r.candidate_digest == now)
            })
        });
        let judged = Verdict::load(&ledger.verdict_latest_path(id))
            .ok()
            .zip(harness_core::hash::unit_crate_file_set_hash(&self.config.target, dir).ok())
            .is_some_and(|(v, set)| !v.inputs.rust_crate.is_empty() && v.inputs.rust_crate == set);
        if recorded || judged {
            Ok(())
        } else {
            Err(
                "the crate differs from every recorded attempt and from what the oracle last \
                 judged — restore it, or record it with `harness override` (see Help)"
                    .into(),
            )
        }
    }

    fn close_dialog(&mut self, confirm: Confirm, choice: Choice) -> Command {
        match (confirm.purpose, choice) {
            (Purpose::Act(p), Choice::Run | Choice::Record) => match self.confirm_gate(&p) {
                Ok(()) => Command::Spawn(p),
                Err(why) => {
                    self.notice = notice(format!("{}: {why}", p.label));
                    if let Some(tmp) = &p.cleanup {
                        self.notice = notice(format!(
                            "{}: {why}; the hand edit is kept in {} — E offers it again",
                            p.label,
                            tmp.join("edit").display()
                        ));
                    }
                    Command::None
                }
            },
            (Purpose::Act(p), Choice::Discard) => {
                let tmp = p.cleanup.clone().unwrap_or_default();
                self.forget_edit(&tmp);
                self.notice = notice("hand edit discarded");
                Command::Cleanup(tmp)
            }
            (Purpose::Act(p), _) => {
                self.notice = notice(match p.cleanup {
                    Some(_) => format!(
                        "{}: not run; the hand edit is kept — E offers it again",
                        p.label
                    ),
                    None => format!("{}: not run", p.label),
                });
                Command::None
            }
            (Purpose::Quit, Choice::QuitLeave) => Command::Quit,
            (Purpose::Quit, Choice::QuitStop) => Command::CancelAndQuit,
            (Purpose::Cancel, Choice::Stop) => Command::Cancel,
            (Purpose::Quit | Purpose::Cancel, _) => Command::None,
        }
    }

    // ----- the menu ----------------------------------------------------------

    /// `Enter`: the selection's menu, opened with a fresh check of the lock
    /// holder, focused on the recommended item.
    pub fn open_menu(&mut self) {
        self.refresh_holder();
        let items = self.menu_items();
        let next = self.next_step().and_then(|(_, act)| act);
        let focus = menu::recommended(&items, &self.selection, next);
        self.mode = Mode::Menu(Menu {
            items,
            focus,
            footer: None,
        });
    }

    /// Act on a menu item (or an accelerator's item).
    pub(crate) fn choose(&mut self, it: &Item) -> Command {
        if let Some(why) = &it.greyed {
            self.notice = notice(format!("{}: {why}", it.label));
            return Command::None;
        }
        match &it.action {
            Action::Open => {
                self.focus = Focus::View;
                Command::None
            }
            Action::Fold => {
                let sel = self.selection.clone();
                let open = !self.expansion.is_open(&sel);
                self.set_open(&sel, open);
                Command::None
            }
            Action::Reread => Command::Reload,
            Action::Act(_) => {
                if let Some(p) = &it.pending {
                    self.ask(p.clone());
                }
                Command::None
            }
            Action::OpenUnit(id) => {
                self.jump(Selection::Unit(id.clone()));
                Command::None
            }
            Action::ChooseAttempt(id) => {
                let unit = Selection::Unit(id.clone());
                self.expansion.set(&unit, true);
                // The menu's rule exactly (review ENG-5/NEW-4).
                let first = self.snapshot.unit(id).and_then(|u| {
                    u.attempts
                        .iter()
                        .find(|a| {
                            a.record.outcome == "green"
                                && a.last_result() == "green"
                                && a.bound
                                && !a.record.promoted
                        })
                        .map(|a| a.record.id.clone())
                });
                match first {
                    Some(a) => self.jump(Selection::Attempt(id.clone(), a)),
                    None => self.jump(unit),
                }
                self.notice = notice("look over its code, then Enter and Accept (or a)");
                Command::None
            }
            Action::ShowChecks => {
                if self.shown_verdict().is_some_and(|v| !v.checks.is_empty()) {
                    self.mode = Mode::Verdict {
                        selected: 0,
                        scroll: 0,
                    };
                } else {
                    self.notice = notice("no verdict for what is shown");
                }
                Command::None
            }
            Action::Compare => {
                match self.diff() {
                    Ok(mode) => {
                        self.diff_rows = None;
                        self.mode = mode;
                    }
                    Err(why) => self.notice = notice(why),
                }
                Command::None
            }
            Action::HandEdit => match self.hand_edit_target() {
                Ok((unit, crate_dir)) => Command::Edit { unit, crate_dir },
                Err(why) => {
                    self.notice = notice(why);
                    Command::None
                }
            },
            Action::Modify => {
                if let (Some(u), Some(a)) = (
                    self.unit_view().map(|u| u.unit.id.clone()),
                    self.shown_attempt().map(|a| a.record.id.clone()),
                ) {
                    let input = self.notes.get(&a).cloned().unwrap_or_default();
                    self.mode = Mode::Note {
                        input,
                        unit: u,
                        attempt: a,
                    };
                }
                Command::None
            }
            Action::ContinueKept => {
                if let Some(k) = self.kept_edits.last().cloned() {
                    self.mode = Mode::EditNote {
                        input: k.note,
                        unit: k.unit,
                        stage: k.stage,
                        tmp: k.tmp,
                    };
                }
                Command::None
            }
            Action::DiscardKept => {
                if let Some(k) = self.kept_edits.last().cloned() {
                    match self.hand_edit_argv(&k.unit, &k.stage, k.tmp.clone(), Some(&k.note)) {
                        Ok(p) => {
                            self.ask(p);
                            if let Mode::Dialog(c) = &mut self.mode {
                                c.title = format!(
                                    "Your kept hand edit of {}: keep it, record it, or discard it?",
                                    k.unit
                                );
                            }
                        }
                        Err(why) => {
                            self.notice = notice(format!(
                                "{why}; the hand edit is kept in {}",
                                k.tmp.join("edit").display()
                            ))
                        }
                    }
                }
                Command::None
            }
            Action::Cancel => {
                if self.running {
                    self.open_dialog(Purpose::Cancel);
                }
                Command::None
            }
            Action::Migrate => Command::None,
        }
    }

    /// The accelerator `key`: the selection's menu item bound to it.
    fn accelerator(&mut self, key: &str) -> Command {
        self.refresh_holder();
        let items = self.menu_items();
        match items.iter().find(|i| i.accel == Some(key)) {
            Some(it) => {
                let it = it.clone();
                self.choose(&it)
            }
            None => {
                self.notice = notice(format!("{key}: nothing to do for the selection"));
                Command::None
            }
        }
    }

    // ----- the project summary --------------------------------------------

    /// The Next step, stated as a fact (§3), and the act it points to — never
    /// model work.
    pub fn next_step(&self) -> Option<(String, Option<Act>)> {
        let Some(state) = &self.snapshot.facts_state else {
            return Some((
                "Nothing is scanned yet — press Enter and choose Scan the project".into(),
                Some(Act::Scan),
            ));
        };
        // The stale paths hold the missing files too (review ENG-6).
        let changed = state.stale
            + self
                .files
                .files
                .iter()
                .filter(|f| f.state == FileState::New)
                .count();
        if changed > 0 {
            return Some((
                format!(
                    "{changed} file{} changed since the scan — Scan the project again",
                    if changed == 1 { "" } else { "s" }
                ),
                Some(Act::Scan),
            ));
        }
        // Only a missing plan: a plan with no units is a plan (review ENG-3).
        if self.snapshot.units.is_empty() && self.snapshot.note.as_deref() == Some(NO_PLAN) {
            return Some(("No plan yet — Refresh the plan".into(), Some(Act::Plan)));
        }
        // A blocked unit's files left: refreshing the plan never makes it
        // fresh, so it is no next step.
        if let Some(u) = self
            .snapshot
            .units
            .iter()
            .find(|u| !u.report.source_fresh && u.report.status != "blocked")
        {
            return Some((
                format!(
                    "{}'s C changed since it was planned — scan, refresh the plan, then review \
                     its diff",
                    u.unit.id
                ),
                Some(Act::Plan),
            ));
        }
        None
    }

    // ----- keys ---------------------------------------------------------------

    /// A bracketed paste: text for a note being typed (line breaks become
    /// spaces), ignored anywhere else — a paste never answers a dialog.
    pub fn on_paste(&mut self, text: &str) {
        let clean: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let (dropped, max) = match &mut self.mode {
            Mode::Note { input, .. } => {
                (push_bounded(input, &clean, MAX_NOTE_BYTES), MAX_NOTE_BYTES)
            }
            Mode::EditNote { input, .. } => (
                push_bounded(input, &clean, MAX_EDIT_NOTE_BYTES),
                MAX_EDIT_NOTE_BYTES,
            ),
            _ => {
                self.notice = notice("paste ignored outside a note");
                return;
            }
        };
        if dropped > 0 {
            self.notice = notice(format!(
                "the paste did not fit: {dropped} bytes dropped (a note is at most {max} bytes)"
            ));
        }
    }

    fn stash_note(&mut self, tmp: &Path, note: String) {
        if let Some(k) = self.kept_edits.iter_mut().find(|k| k.tmp == tmp) {
            k.note = note;
        }
    }

    /// Handle one key press read at `now`.
    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Command {
        self.now = now;
        // A notice clears on the next key (a new one may replace it).
        if self.notice.as_ref().is_some_and(|n| n.at < now) {
            self.notice = None;
        }
        self.on_key_inner(key, now)
    }

    /// Clear a notice older than [`NOTICE_TTL`].
    pub fn expire_notice(&mut self, now: Instant) {
        if self
            .notice
            .as_ref()
            .is_some_and(|n| now.saturating_duration_since(n.at) >= NOTICE_TTL)
        {
            self.notice = None;
        }
    }

    fn on_key_inner(&mut self, key: KeyEvent, now: Instant) -> Command {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        // A key with Ctrl or Alt is never text, never an answer.
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => {}
            Mode::Dialog(mut confirm) => {
                return match confirm.dialog.on_key(key, now) {
                    Outcome::Stay => {
                        self.mode = Mode::Dialog(confirm);
                        Command::None
                    }
                    Outcome::Close(choice) => self.close_dialog(*confirm, choice),
                };
            }
            Mode::Menu(mut m) => {
                let n = m.items.len();
                match key.code {
                    KeyCode::Esc => return Command::None,
                    _ if ctrl_c => return Command::None,
                    KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                        m.focus = (m.focus + n - 1) % n;
                        m.footer = None;
                    }
                    KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                        m.focus = (m.focus + 1) % n;
                        m.footer = None;
                    }
                    KeyCode::Home => m.focus = 0,
                    KeyCode::End => m.focus = n.saturating_sub(1),
                    KeyCode::Enter => {
                        if let Some(it) = m.items.get(m.focus).cloned() {
                            if let Some(why) = &it.greyed {
                                m.footer = Some(why.clone());
                            } else {
                                return self.choose(&it);
                            }
                        }
                    }
                    KeyCode::Char(c) if plain => {
                        let s = c.to_string();
                        if let Some(it) = m
                            .items
                            .iter()
                            .find(|i| i.accel == Some(s.as_str()))
                            .cloned()
                        {
                            if let Some(why) = &it.greyed {
                                m.footer = Some(why.clone());
                            } else {
                                return self.choose(&it);
                            }
                        }
                    }
                    _ => {}
                }
                self.mode = Mode::Menu(m);
                return Command::None;
            }
            Mode::Details { scroll } => {
                self.mode = match key.code {
                    KeyCode::Up | KeyCode::Char('k') => Mode::Details {
                        scroll: scroll.saturating_sub(1),
                    },
                    KeyCode::Down | KeyCode::Char('j') => Mode::Details {
                        scroll: scroll.saturating_add(1),
                    },
                    KeyCode::PageUp => Mode::Details {
                        scroll: scroll.saturating_sub(10),
                    },
                    KeyCode::PageDown | KeyCode::Char(' ') => Mode::Details {
                        scroll: scroll.saturating_add(10),
                    },
                    KeyCode::Home => Mode::Details { scroll: 0 },
                    KeyCode::End => Mode::Details {
                        scroll: usize::MAX / 2,
                    },
                    KeyCode::Esc | KeyCode::Char('c') => Mode::Normal,
                    _ => {
                        // Other keys act as in the panes (x cancels, q quits).
                        self.mode = Mode::Details { scroll };
                        let command = self.normal_key(key, ctrl_c, plain);
                        if matches!(self.mode, Mode::Normal) {
                            self.mode = Mode::Details { scroll };
                        }
                        return command;
                    }
                };
                return Command::None;
            }
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
                    _ if ctrl_c => Mode::Normal,
                    _ => Mode::Verdict { selected, scroll },
                };
                return Command::None;
            }
            Mode::Diff {
                scroll,
                lines,
                title,
            } => {
                let scroll = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => scroll.saturating_add(1),
                    KeyCode::Char('k') | KeyCode::Up => scroll.saturating_sub(1),
                    KeyCode::PageDown | KeyCode::Char(' ') => scroll.saturating_add(20),
                    KeyCode::PageUp => scroll.saturating_sub(20),
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('d') => return Command::None,
                    _ if ctrl_c => return Command::None,
                    _ => scroll,
                };
                self.mode = Mode::Diff {
                    scroll,
                    lines,
                    title,
                };
                return Command::None;
            }
            Mode::Note {
                mut input,
                unit,
                attempt,
            } => {
                let keep = |app: &mut App, input: String, unit: String, attempt: String| {
                    app.mode = Mode::Note {
                        input,
                        unit,
                        attempt,
                    };
                };
                match key.code {
                    KeyCode::Esc => {
                        self.notes.insert(attempt, input);
                        self.notice = notice("Modify cancelled; your note is kept for next time");
                    }
                    _ if ctrl_c => {
                        self.notes.insert(attempt, input);
                        self.notice = notice("Modify cancelled; your note is kept for next time");
                    }
                    KeyCode::Enter => {
                        self.notes.insert(attempt.clone(), input.clone());
                        if input.trim().is_empty() {
                            self.notice = notice("an empty note steers nothing");
                            keep(self, input, unit, attempt);
                        } else if let Some(why) = note_problem(&input, MAX_NOTE_BYTES) {
                            // The CLI would refuse it: say so here, keep typing.
                            self.notice = notice(why);
                            keep(self, input, unit, attempt);
                        } else {
                            match self.act_argv(
                                Act::Modify,
                                Some(&unit),
                                Some(&attempt),
                                Some(&input),
                            ) {
                                Ok(p) => self.ask(p),
                                Err(why) => self.notice = notice(why),
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        keep(self, input, unit, attempt);
                    }
                    KeyCode::Char(c) if plain && !c.is_control() => {
                        if input.len() + c.len_utf8() <= MAX_NOTE_BYTES {
                            input.push(c);
                        } else {
                            self.notice =
                                notice(format!("a note is at most {MAX_NOTE_BYTES} bytes"));
                        }
                        keep(self, input, unit, attempt);
                    }
                    _ => keep(self, input, unit, attempt),
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
                        self.notice = notice("hand edit kept — E offers it again");
                        return Command::None;
                    }
                    _ if ctrl_c => {
                        self.stash_note(&tmp, input);
                        self.notice = notice("hand edit kept — E offers it again");
                        return Command::None;
                    }
                    KeyCode::Enter
                        if !input.trim().is_empty()
                            && note_problem(&input, MAX_EDIT_NOTE_BYTES).is_some() =>
                    {
                        self.notice = note_problem(&input, MAX_EDIT_NOTE_BYTES).and_then(notice);
                    }
                    KeyCode::Enter => {
                        self.stash_note(&tmp, input.clone());
                        match self.hand_edit_argv(&unit, &stage, tmp.clone(), Some(&input)) {
                            Ok(p) => {
                                self.ask(p);
                                return Command::None;
                            }
                            Err(why) => {
                                self.notice = notice(format!(
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
                            self.notice = notice(format!(
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
        }
        self.normal_key(key, ctrl_c, plain)
    }

    fn quit(&mut self) -> Command {
        if self.running {
            self.open_dialog(Purpose::Quit);
            Command::None
        } else {
            Command::Quit
        }
    }

    fn normal_key(&mut self, key: KeyEvent, ctrl_c: bool, plain: bool) -> Command {
        if let Some(open) = self.pending_bracket.take() {
            if key.code == KeyCode::Char('f') {
                self.scroll_to_pair(open == ']');
                return Command::None;
            }
        }
        if ctrl_c {
            return self.quit();
        }
        if !plain {
            return Command::None;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return self.quit(),
            KeyCode::Char('?') | KeyCode::F(1) => self.mode = Mode::Help { scroll: 0 },
            KeyCode::Char('c') => {
                self.mode = Mode::Details {
                    scroll: usize::MAX / 2,
                }
            }
            KeyCode::Char('g') => return Command::Reload,
            KeyCode::Char('t') => match self.try_again.clone() {
                Some(p) if !self.running => self.ask(p),
                Some(_) => self.notice = notice("a command is running (one at a time)"),
                None => self.notice = notice("nothing to try again"),
            },
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Files => Focus::View,
                    Focus::View => Focus::Files,
                }
            }
            KeyCode::Char(']') => self.pending_bracket = Some(']'),
            KeyCode::Char('[') => self.pending_bracket = Some('['),
            KeyCode::Char('J') => self.next_unit(true),
            KeyCode::Char('K') => self.next_unit(false),
            KeyCode::Enter
                if self.focus == Focus::View
                    && self.link.and_then(|l| self.links.get(l)).is_some() =>
            {
                if let Some(sel) = self.link.and_then(|l| self.links.get(l)).cloned() {
                    self.jump(sel);
                    self.focus = Focus::Files;
                }
            }
            KeyCode::Enter => self.open_menu(),
            KeyCode::Char('x') => {
                if self.running {
                    self.open_dialog(Purpose::Cancel);
                } else {
                    self.notice = notice("nothing is running");
                }
            }
            // The kept hand edit, from anywhere (review USE-11).
            KeyCode::Char('E') => match self.kept_edits.last().cloned() {
                None => self.notice = notice("no kept hand edit"),
                Some(_) if self.running => {
                    self.notice = notice("a command is running (one at a time)")
                }
                Some(k) => {
                    self.mode = Mode::EditNote {
                        input: k.note,
                        unit: k.unit,
                        stage: k.stage,
                        tmp: k.tmp,
                    }
                }
            },
            KeyCode::Char(c @ ('a' | 'm' | 'e' | 'r' | 'R' | 'd' | 'v')) => {
                return self.accelerator(&c.to_string());
            }
            KeyCode::Backspace => {
                if !self.go_back() {
                    self.notice = notice("nothing to go back to");
                }
            }
            _ => match self.focus {
                Focus::Files => self.tree_key(key.code),
                Focus::View => self.view_key_press(key.code),
            },
        }
        Command::None
    }

    fn tree_key(&mut self, code: KeyCode) {
        let page = self.layout.tree_page.max(1) as isize;
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_rows(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_rows(1),
            KeyCode::PageUp => self.move_rows(-page),
            KeyCode::PageDown => self.move_rows(page),
            KeyCode::Home => self.move_rows(isize::MIN / 2),
            KeyCode::End => self.move_rows(isize::MAX / 2),
            KeyCode::Left => {
                let sel = self.selection.clone();
                let row = self.cursor().and_then(|i| self.rows.get(i)).cloned();
                match row {
                    Some(r) if r.open => self.set_open(&sel, false),
                    _ => {
                        if let Some(p) = sel.parent() {
                            self.select(p);
                        }
                    }
                }
            }
            KeyCode::Right => {
                let sel = self.selection.clone();
                let row = self.cursor().and_then(|i| self.rows.get(i)).cloned();
                match row {
                    Some(r) if r.expandable && !r.open => self.set_open(&sel, true),
                    _ => self.focus = Focus::View,
                }
            }
            KeyCode::Esc => {
                if !self.go_back() {
                    self.notice = None;
                }
            }
            _ => {}
        }
    }

    fn max_scroll(&self) -> usize {
        self.layout.total_rows.saturating_sub(1)
    }

    fn view_key_press(&mut self, code: KeyCode) {
        let page = self.layout.page.max(1);
        if !self.links.is_empty() {
            let last = self.links.len() - 1;
            let at = self.link;
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.link = Some(at.map_or(0, |l| l.saturating_sub(1)))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.link = Some(at.map_or(0, |l| (l + 1).min(last)))
                }
                KeyCode::PageUp => self.link = Some(at.map_or(0, |l| l.saturating_sub(page))),
                KeyCode::PageDown => self.link = Some(at.map_or(0, |l| (l + page).min(last))),
                KeyCode::Home => self.link = Some(0),
                KeyCode::End => self.link = Some(last),
                KeyCode::Left | KeyCode::Esc => {
                    self.link = None;
                    self.focus = Focus::Files;
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = (self.scroll + 1).min(self.max_scroll())
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll = (self.scroll + page).min(self.max_scroll())
            }
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),
            KeyCode::Left => {
                if self.hscroll == 0 {
                    self.focus = Focus::Files;
                } else {
                    self.hscroll = self.hscroll.saturating_sub(8);
                }
            }
            // Never past the longest code line (review USE-14).
            KeyCode::Right => {
                let room = self.code_cols.saturating_sub(8);
                self.hscroll = (self.hscroll + 8).min(room);
            }
            KeyCode::Esc => self.focus = Focus::Files,
            _ => {}
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

    // ----- the hand edit -------------------------------------------------------

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

    /// The glyph and word of the node `sel` (for rows, titles and tests).
    pub fn node_label(&self, sel: &Selection) -> (&'static str, String) {
        match sel {
            Selection::Project | Selection::Dir(_) | Selection::Units => ("", String::new()),
            Selection::File(p) => match self.files.file(p) {
                Some(f) => files::file_label(&self.files, &f.state),
                None => ("", String::new()),
            },
            Selection::Function(p, name) => {
                let Some(f) = self.files.file(p) else {
                    return ("", String::new());
                };
                let in_unit = f.functions.iter().any(|x| &x.name == name && x.in_unit);
                match (in_unit, f.owner.and_then(|u| self.files.units.get(u))) {
                    // It takes its unit's state (§2.3), whatever its file's.
                    (true, Some(info)) => (info.state.glyph(), info.state.word()),
                    _ => ("", "internal".into()),
                }
            }
            Selection::Unit(id) => {
                match self.snapshot.units.iter().position(|u| &u.unit.id == id) {
                    Some(u) => {
                        let s: &UnitState = &self.files.units[u].state;
                        (s.glyph(), s.word())
                    }
                    None => ("", String::new()),
                }
            }
            Selection::Crate(id) => match self.snapshot.unit(id) {
                Some(u) => (
                    "",
                    if u.crate_dir.is_some() {
                        provenance_words(u)
                    } else {
                        "no crate yet".into()
                    },
                ),
                None => ("", String::new()),
            },
            Selection::Attempt(u, a) => match self.snapshot.unit(u).and_then(|x| x.attempt(a)) {
                Some(at) => {
                    let unit = self.snapshot.unit(u).expect("found above");
                    let waiting = self
                        .awaiting
                        .iter()
                        .any(|aw| aw.attempt.as_deref() == Some(a.as_str()));
                    let mut word = at.record.outcome.replace('-', " ");
                    if waiting {
                        word = "waiting for your answer".into();
                    }
                    let tags = attempt_tags(unit, at);
                    if !tags.is_empty() {
                        word = format!("{word} {}", tags.join(" "));
                    }
                    ("", word)
                }
                None => ("", String::new()),
            },
        }
    }
}

/// A code line's width in columns, tabs taken as 8.
fn code_width(line: &CodeLine) -> usize {
    match line {
        CodeLine::Code { pieces, .. } => {
            pieces
                .iter()
                .map(|(_, t)| {
                    t.chars()
                        .map(|c| if c == '\t' { 8 } else { 1 })
                        .sum::<usize>()
                })
                .sum::<usize>()
                + 8
        }
        CodeLine::Link(_) | CodeLine::Note(_) => 0,
    }
}

/// A unit crate's provenance in words.
pub fn provenance_words(unit: &UnitView) -> String {
    match &unit.provenance {
        ProvenanceView::None if matches!(unit.report.status.as_str(), "verified" | "merged") => {
            "origin not recorded".into()
        }
        ProvenanceView::None => "no attempt produced it".into(),
        ProvenanceView::Pipeline(id) => format!("from attempt {} (pipeline)", short_id(id)),
        ProvenanceView::Ambiguous(ids) => format!("ambiguous: {} attempts match", ids.len()),
        ProvenanceView::Steered(id) => format!("from attempt {} (steered)", short_id(id)),
        ProvenanceView::Human { attempt, origin } if attempt == origin => {
            format!("from hand edit {}", short_id(origin))
        }
        ProvenanceView::Human { attempt, origin } => format!(
            "from {} (a steer of hand edit {})",
            short_id(attempt),
            short_id(origin)
        ),
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
            // The file's own name: the View's title names its path, and a
            // cut heading keeps the line number (review USE-8).
            format!(
                "{} ({}:{})",
                span.name,
                span.file.rsplit('/').next().unwrap_or(&span.file),
                span.first_line
            ),
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

/// An attempt's tags: provenance, supersession, lineage.
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
pub(crate) mod tests {
    use super::*;
    use crate::testutil::{scratch_target, READ_SCALEFACTORS};
    use harness_core::attempts;
    use std::os::unix::process::ExitStatusExt;

    pub(crate) const PROVENANCE: &str = "a-13c941dfff95";
    pub(crate) const SUPERSEDED: &str = "a-28d8ddc411f9";
    pub(crate) const RED: &str = "a-d2e5513cdfa6";
    pub(crate) const HARNESS: &str = "/opt/ruharness/bin/harness";
    pub(crate) const LIB_C: &str = "test_case/src/lib.c";

    pub(crate) fn app_of(rel: &str, tag: &str) -> App {
        let target = scratch_target(rel, tag);
        let read = crate::load::read(&target).unwrap();
        App::new(
            Config {
                target,
                harness: Some(PathBuf::from(HARNESS)),
                allow_unsandboxed: false,
                layout: LayoutMode::Auto,
                providers: vec!["external".into()],
            },
            read,
        )
    }

    pub(crate) fn app(tag: &str) -> App {
        app_of(READ_SCALEFACTORS, tag)
    }

    pub(crate) fn key(app: &mut App, c: char) -> Command {
        app.on_key(KeyEvent::from(KeyCode::Char(c)), Instant::now())
    }

    pub(crate) fn code(app: &mut App, code: KeyCode) -> Command {
        app.on_key(KeyEvent::from(code), Instant::now())
    }

    pub(crate) fn attempt(app: &mut App, id: &str) {
        app.select(Selection::Attempt("u-lib".into(), id.into()));
    }

    pub(crate) fn strs(argv: &[OsString]) -> Vec<String> {
        argv.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// What the event loop does once the dialog was drawn whole, quiet, with
    /// no input pending.
    pub(crate) fn arm(app: &mut App) {
        if let Mode::Dialog(c) = &mut app.mode {
            c.dialog.seen = true;
            c.dialog.armed = true;
        }
    }

    pub(crate) fn dialog_argv(app: &App) -> Vec<String> {
        match &app.mode {
            Mode::Dialog(c) => match &c.purpose {
                Purpose::Act(p) => strs(&p.argv),
                other => panic!("not an act: {other:?}"),
            },
            other => panic!("no dialog: {other:?} (notice {:?})", app.notice),
        }
    }

    fn said(app: &App) -> String {
        app.notice
            .as_ref()
            .map(|n| n.text.clone())
            .unwrap_or_default()
    }

    fn record(app: &mut App, id: &str, f: impl FnOnce(&mut AttemptRecord)) {
        let ledger = Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", id);
        let mut rec = AttemptRecord::load(&dir).unwrap();
        f(&mut rec);
        rec.store(&dir).unwrap();
        assert!(app.reload(true));
    }

    /// §4, §5: an act shows its exact argv in an armed dialog; Run spawns
    /// exactly that; the safe button spawns nothing.
    #[test]
    fn acts_show_their_exact_argv_in_an_armed_dialog() {
        let mut app = app("acts");
        let root = app.config.target.display().to_string();
        attempt(&mut app, PROVENANCE);
        // Accept on a verified unit replaces.
        assert_eq!(key(&mut app, 'a'), Command::None);
        let argv = dialog_argv(&app);
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
        let Mode::Dialog(c) = &app.mode else {
            unreachable!()
        };
        assert_eq!(
            c.title,
            "Replace u-lib's verified crate with a-13c941dfff95?"
        );
        // Unarmed, `y` is dropped.
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(matches!(app.mode, Mode::Dialog(_)));
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn once armed: {:?}", app.notice);
        };
        assert_eq!(strs(&p.argv), argv);
        assert_eq!(app.mode, Mode::Normal);
        // Modify: a note that starts with `-` travels attached; the provider
        // is the cockpit's.
        key(&mut app, 'm');
        app.on_paste("- keep the wrapping add");
        code(&mut app, KeyCode::Enter);
        assert_eq!(
            dialog_argv(&app),
            [
                HARNESS,
                "--json",
                "migrate",
                "u-lib",
                &format!("--target={root}"),
                "--no-promote",
                "--provider=external",
                "--model=claude-sonnet-5",
                &format!("--from={PROVENANCE}"),
                "--steer=- keep the wrapping add"
            ]
        );
        assert_eq!(
            code(&mut app, KeyCode::Esc),
            Command::None,
            "Esc spawns nothing"
        );
        assert_eq!(app.mode, Mode::Normal);
        // With --allow-unsandboxed the acts that run code pass it on, and
        // the dialog says so in words.
        app.config.allow_unsandboxed = true;
        key(&mut app, 'a');
        assert_eq!(
            dialog_argv(&app).last().map(String::as_str),
            Some("--allow-unsandboxed")
        );
        let Mode::Dialog(c) = &app.mode else {
            unreachable!()
        };
        assert!(c.body.iter().any(|b| b.contains("WITHOUT the sandbox")));
        code(&mut app, KeyCode::Esc);
        // Read-only without a harness binary: greyed, said.
        app.config.harness = None;
        key(&mut app, 'a');
        assert_eq!(app.mode, Mode::Normal);
        assert!(said(&app).contains("read-only"), "{}", said(&app));
    }

    /// The project-wide acts and Re-check: their argv carries no tree path.
    #[test]
    fn project_and_unit_acts_have_their_argv() {
        let app = app("projacts");
        let root = app.config.target.display().to_string();
        for (act, sub) in [
            (Act::Scan, "scan"),
            (Act::Plan, "plan"),
            (Act::Detect, "detect"),
        ] {
            let p = app.act_argv(act, None, None, None).unwrap();
            assert_eq!(
                strs(&p.argv),
                [HARNESS, "--json", sub, &format!("--target={root}")]
            );
        }
        let p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        assert_eq!(
            strs(&p.argv),
            [
                HARNESS,
                "--json",
                "verify",
                "u-lib",
                &format!("--target={root}")
            ]
        );
        assert_eq!(p.label, "Re-check u-lib");
    }

    /// Accelerators act on the selection; what does not apply says so.
    #[test]
    fn accelerators_act_on_the_selection() {
        let mut app = app("accel");
        // The project: no attempt to accept.
        assert_eq!(key(&mut app, 'a'), Command::None);
        assert!(said(&app).contains("nothing to do"), "{}", said(&app));
        // A red attempt: Accept does not apply; Modify does.
        attempt(&mut app, RED);
        key(&mut app, 'a');
        assert_eq!(app.mode, Mode::Normal);
        assert!(said(&app).contains("nothing to do"));
        key(&mut app, 'm');
        assert!(matches!(app.mode, Mode::Note { .. }));
        code(&mut app, KeyCode::Esc);
        // Retry is never offered for a blind external attempt.
        key(&mut app, 'r');
        assert!(said(&app).contains("nothing to do"), "{}", said(&app));
        // x with nothing running, R with nothing awaiting.
        key(&mut app, 'x');
        assert!(said(&app).contains("nothing is running"));
        key(&mut app, 'R');
        assert!(said(&app).contains("nothing to do"));
        // While a command runs, no act starts; x opens the cancel dialog.
        app.running = true;
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        assert!(
            said(&app).contains("a command is running"),
            "{}",
            said(&app)
        );
        assert_eq!(key(&mut app, 'x'), Command::None);
        assert!(matches!(&app.mode, Mode::Dialog(c) if c.purpose == Purpose::Cancel));
        arm(&mut app);
        assert_eq!(key(&mut app, 'x'), Command::Cancel);
    }

    /// SAFE-11: `Q` is `q`; while a command runs both open the quit dialog,
    /// which acts only once armed; `Ctrl-C` too.
    #[test]
    fn quit_goes_through_an_armed_dialog_while_a_command_runs() {
        let mut app = app("quitarm");
        app.running = true;
        for c in ['q', 'Q'] {
            assert_eq!(key(&mut app, c), Command::None, "{c}");
            assert!(matches!(&app.mode, Mode::Dialog(d) if d.purpose == Purpose::Quit));
            for k in ['q', 'Q', 'x'] {
                assert_eq!(key(&mut app, k), Command::None, "{k} unarmed");
            }
            code(&mut app, KeyCode::Esc);
        }
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.on_key(ctrl_c, Instant::now()), Command::None);
        arm(&mut app);
        assert_eq!(key(&mut app, 'q'), Command::Quit);
        key(&mut app, 'q');
        arm(&mut app);
        assert_eq!(key(&mut app, 'x'), Command::CancelAndQuit);
        app.running = false;
        assert_eq!(key(&mut app, 'Q'), Command::Quit);
        assert_eq!(app.on_key(ctrl_c, Instant::now()), Command::Quit);
    }

    /// Mutation-checked rule, through the whole app: a held `Enter` on a
    /// tree row opens the menu, chooses the recommended item, opens its
    /// dialog — and never runs it.
    #[test]
    fn a_held_enter_never_runs_anything() {
        let mut app = app("heldenter");
        // The recommended item of a stale project is Scan.
        app.select(Selection::Project);
        let lib = app.config.target.join(LIB_C);
        let text = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{text}\n")).unwrap();
        assert!(app.reload(true));
        let t0 = Instant::now();
        let mut t = t0;
        let mut spawned = false;
        for i in 0..200 {
            t += if i == 1 {
                Duration::from_millis(600)
            } else {
                Duration::from_millis(30)
            };
            app.on_input(t);
            if let Command::Spawn(_) = app.on_key(KeyEvent::from(KeyCode::Enter), t) {
                spawned = true;
            }
            // The loop draws and tries to arm between reads.
            if let Mode::Dialog(c) = &mut app.mode {
                c.dialog.seen = true;
            }
            app.arm(t + Duration::from_millis(29), false);
        }
        assert!(!spawned);
    }

    /// SAFE-3, SAFE-12, CHK-13: Retry is never offered for a blind attempt,
    /// refuses a half-seeded record and a provider not on the list — in the
    /// menu, and again at confirm on a fresh read.
    #[test]
    fn retry_refuses_blind_half_seeded_and_unlisted_providers() {
        let mut app = app("retryrules");
        let root = app.config.target.display().to_string();
        attempt(&mut app, PROVENANCE);
        let retry = |app: &App| {
            app.menu_items()
                .into_iter()
                .find(|i| i.action == Action::Act(Act::Retry))
        };
        assert!(retry(&app).is_none(), "never offered for a blind hand-off");
        // A profile of the external kind under another name: blind too.
        record(&mut app, PROVENANCE, |r| r.provider = "handoff".into());
        app.config.providers.push("handoff".into());
        assert!(retry(&app).is_none());
        // Half seeded: greyed, never retried unseeded.
        for (seed, note) in [(Some("a-000000000000"), None), (None, Some("- use iter()"))] {
            record(&mut app, PROVENANCE, |r| {
                r.provider = "external".into();
                r.seeded_from = seed.map(String::from);
                r.steer_note = note.map(String::from);
            });
            let it = retry(&app).expect("offered, greyed");
            assert!(it.greyed.as_deref().unwrap().contains("half"), "{it:?}");
            key(&mut app, 'r');
            assert_eq!(app.mode, Mode::Normal);
        }
        // A steer attempt of a listed provider: its own run shape.
        record(&mut app, PROVENANCE, |r| {
            r.seeded_from = Some("a-000000000000".into());
            r.steer_note = Some("- use iter()".into());
        });
        key(&mut app, 'r');
        let model = app.shown_attempt().unwrap().record.model.clone();
        assert_eq!(
            dialog_argv(&app),
            [
                HARNESS,
                "--json",
                "migrate",
                "u-lib",
                &format!("--target={root}"),
                "--no-promote",
                "--retry",
                "--provider=external",
                &format!("--model={model}"),
                "--from=a-000000000000",
                "--steer=- use iter()",
            ]
        );
        // The record changes under the open dialog: refused at confirm.
        let ledger = Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", PROVENANCE);
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.steer_note = None;
        rec.store(&dir).unwrap();
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(said(&app).contains("half"), "{}", said(&app));
        // A provider not on the list: greyed, naming the flag.
        record(&mut app, PROVENANCE, |r| {
            r.steer_note = Some("- use iter()".into());
            r.provider = "anthropic-live".into();
            r.provider_kind = "anthropic".into();
        });
        let it = retry(&app).unwrap();
        assert!(it
            .greyed
            .unwrap()
            .contains("start with `--provider anthropic-live`"));
        // An unseeded attempt of a listed live provider is a human's retry.
        app.config.providers.push("anthropic-live".into());
        record(&mut app, PROVENANCE, |r| {
            r.seeded_from = None;
            r.steer_note = None;
        });
        key(&mut app, 'r');
        assert!(dialog_argv(&app).contains(&"--provider=anthropic-live".to_string()));
    }

    /// CHK-1: Modify passes the first listed provider; its dialog names the
    /// provider and the target's model in words; the note is remembered per
    /// attempt, whatever happens to the dialog.
    #[test]
    fn modify_passes_the_listed_provider_and_remembers_the_note() {
        let mut app = app("modprov");
        app.config.providers = vec!["local".into(), "external".into()];
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'm');
        app.on_paste("tighten the loop");
        code(&mut app, KeyCode::Enter);
        let argv = dialog_argv(&app);
        assert_eq!(
            argv.iter()
                .filter(|a| a.starts_with("--provider"))
                .collect::<Vec<_>>(),
            ["--provider=local"]
        );
        let Mode::Dialog(c) = &app.mode else {
            unreachable!()
        };
        assert!(c.body[0].contains("provider local"), "{:?}", c.body);
        assert!(c.body[0].contains("migrate routing"), "{:?}", c.body);
        code(&mut app, KeyCode::Esc);
        key(&mut app, 'm');
        assert!(matches!(&app.mode, Mode::Note { input, .. } if input == "tighten the loop"));
        code(&mut app, KeyCode::Esc);
        key(&mut app, 'm');
        assert!(matches!(&app.mode, Mode::Note { input, .. } if input == "tighten the loop"));
    }

    /// §4.3, §6.3: Re-check runs only on code the harness knows, unchanged
    /// since the cockpit showed it — checked again at confirm.
    #[test]
    fn recheck_needs_known_code_unchanged_since_shown() {
        let mut app = app("recheck");
        app.select(Selection::Unit("u-lib".into()));
        let recheck = |app: &App| {
            app.menu_items()
                .into_iter()
                .find(|i| i.action == Action::Act(Act::Verify))
                .unwrap()
        };
        assert_eq!(recheck(&app).greyed, None);
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        let i = m
            .items
            .iter()
            .position(|i| i.action == Action::Act(Act::Verify))
            .unwrap();
        for _ in 0..i {
            code(&mut app, KeyCode::Down);
        }
        if let Mode::Menu(m) = &mut app.mode {
            m.focus = i;
        }
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Dialog(_)), "{:?}", app.notice);
        // The crate changes on disk after it was shown: refused at confirm.
        let logic = app.snapshot.units[0]
            .crate_dir
            .clone()
            .unwrap()
            .join("src/logic.rs");
        let text = std::fs::read_to_string(&logic).unwrap();
        std::fs::write(&logic, format!("{text}\n// later\n")).unwrap();
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(
            said(&app).contains("changed on disk since the cockpit showed it"),
            "{}",
            said(&app)
        );
        // Re-read: now it is unknown code — greyed with the reason.
        assert!(app.reload(true));
        let it = recheck(&app);
        assert!(it
            .greyed
            .unwrap()
            .contains("differs from every recorded attempt"));
        // Put back: known again.
        std::fs::write(&logic, text).unwrap();
        assert!(app.reload(true));
        assert_eq!(recheck(&app).greyed, None);
    }

    /// CHK-7: a live holder of the writer lock greys every spawning item,
    /// read when the menu opens; and it refuses at confirm.
    #[test]
    fn a_live_holder_greys_the_acts_and_refuses_at_confirm() {
        let mut app = app("busy");
        let lock = Ledger::new(&app.config.target).lock_path();
        let holder = format!(
            "{{\"pid\":{},\"command\":\"verify u-lib\",\"started\":\"2026-09-25T00:00:00Z\"}}\n",
            std::process::id()
        );
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        assert!(matches!(app.mode, Mode::Dialog(_)));
        std::fs::write(&lock, &holder).unwrap();
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(
            said(&app).contains("busy: `verify u-lib`"),
            "{}",
            said(&app)
        );
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        let accept = m
            .items
            .iter()
            .find(|i| i.action == Action::Act(Act::Accept))
            .unwrap();
        assert!(accept.greyed.as_deref().unwrap().contains("busy"));
        std::fs::remove_file(&lock).unwrap();
        code(&mut app, KeyCode::Esc);
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        let accept = m
            .items
            .iter()
            .find(|i| i.action == Action::Act(Act::Accept))
            .unwrap();
        assert_eq!(accept.greyed, None, "re-read when the menu opens");
    }

    fn in_progress_attempt(app: &App, id: &str) {
        let ledger = Ledger::new(&app.config.target);
        let seed = &app.snapshot.units[0].attempt(PROVENANCE).unwrap().record;
        let record = AttemptRecord {
            id: id.into(),
            outcome: "in-progress".into(),
            turns: vec![],
            candidate_digest: String::new(),
            promoted: false,
            seeded_from: Some(PROVENANCE.into()),
            steer_note: Some("- a note".into()),
            ..seed.clone()
        };
        let dir = attempts::attempt_dir(&ledger, "u-lib", id);
        std::fs::create_dir_all(&dir).unwrap();
        record.store(&dir).unwrap();
    }

    fn pending(argv: Vec<OsString>, act: Act) -> Pending {
        Pending {
            act,
            argv,
            label: act.label().into(),
            unit: Some("u-lib".into()),
            attempt: None,
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
        }
    }

    /// §4 Resume: the watcher only marks "response present" (it never
    /// spawns); Resume re-spawns the STORED argv of this cockpit's hand-off;
    /// it stays available after a failed resume; a finished attempt ends it.
    #[test]
    fn resume_is_gated_on_state_and_survives_a_failed_resume() {
        let mut app = app("resume");
        let awaited = "a-000000000abc";
        in_progress_attempt(&app, awaited);
        app.reload(true);
        let response = app.config.target.join("response.json");
        let argv: Vec<OsString> = [
            HARNESS,
            "--json",
            "migrate",
            "u-lib",
            "--no-promote",
            "--steer=x",
        ]
        .map(OsString::from)
        .to_vec();
        app.on_spawned(&pending(argv.clone(), Act::Modify));
        app.on_child_msg(ChildMsg::Event(Event::Awaiting {
            attempt: Some(awaited.into()),
            path: response.display().to_string(),
            resume: "harness migrate u-lib …".into(),
            args: None,
        }));
        assert!(app
            .run
            .as_ref()
            .unwrap()
            .narrator
            .step()
            .starts_with("Paused: waiting"));
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        app.load_now();
        assert_eq!(app.awaiting.len(), 1);
        assert_eq!(
            app.last.as_deref().map(|l| l.contains("Paused")),
            Some(true),
            "{:?}",
            app.last
        );
        attempt(&mut app, awaited);
        assert_eq!(
            app.node_label(&app.selection.clone()).1,
            "waiting for your answer steer ← a-13c9"
        );
        // No response yet, then a torn one: not resumable; the tick spawns
        // nothing either way.
        let resume = |app: &App| {
            app.menu_items()
                .into_iter()
                .find(|i| i.action == Action::Act(Act::Resume))
                .unwrap()
        };
        assert!(resume(&app).greyed.unwrap().contains("no response yet"));
        std::fs::write(&response, "{\"text\": ").unwrap();
        app.tick();
        app.load_now();
        assert!(!app.awaiting[0].response_present);
        std::fs::write(&response, "{\"text\": \"x\"}").unwrap();
        app.tick();
        app.load_now();
        assert!(app.awaiting[0].response_present);
        assert_eq!(app.mode, Mode::Normal, "the watcher never spawns or asks");
        key(&mut app, 'R');
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("R must ask, then spawn: {:?}", app.notice);
        };
        assert_eq!(p.argv, argv);
        assert_eq!(p.expect_attempt.as_deref(), Some(awaited));
        // The resumed run fails without a new hand-off: Resume stays.
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
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        app.load_now();
        key(&mut app, 'R');
        assert!(matches!(app.mode, Mode::Dialog(_)), "{:?}", app.notice);
        code(&mut app, KeyCode::Esc);
        // The attempt finished (another answerer resumed it): gone.
        let ledger = Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", awaited);
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.outcome = "red".into();
        rec.store(&dir).unwrap();
        app.tick();
        app.load_now();
        assert!(app.awaiting.is_empty());
    }

    /// §4: the ledger is re-read after the command is REAPED — never on its
    /// `result` event — and on the loader, not the UI thread.
    #[test]
    fn the_ledger_is_reread_after_reaping_not_on_result() {
        let mut app = app("reap");
        let late = "a-0000000000ff";
        let p = pending(vec![OsString::from(HARNESS)], Act::Retry);
        app.on_spawned(&p);
        in_progress_attempt(&app, late);
        app.on_child_msg(ChildMsg::Event(Event::Result {
            exit: 130,
            signal: Some("SIGINT".into()),
        }));
        assert!(
            app.snapshot.units[0].attempt(late).is_none(),
            "re-read on `result`"
        );
        app.on_child_exit(ExitStatus::from_raw(2));
        assert!(
            app.snapshot.units[0].attempt(late).is_none(),
            "read on the UI thread"
        );
        assert_eq!(app.load_request, Some(LoadWhy::Reaped));
        app.load_now();
        assert!(app.snapshot.units[0].attempt(late).is_some());
        assert_eq!(
            app.run.as_ref().unwrap().exit.as_deref(),
            Some("interrupted (SIGINT)")
        );
        assert!(
            app.last.as_deref().unwrap().contains("Stopped (SIGINT)"),
            "{:?}",
            app.last
        );
        app.on_spawned(&p);
        app.on_child_exit(ExitStatus::from_raw(3 << 8));
        assert_eq!(
            app.run.as_ref().unwrap().exit.as_deref(),
            Some("exited without result (exit 3)")
        );
    }

    /// §6.1: a `locked` refusal offers Try again (`t`), which opens the same
    /// dialog with the same argv; a failed start does too.
    #[test]
    fn a_locked_refusal_offers_try_again() {
        let mut app = app("tryagain");
        let p = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.on_spawned(&p);
        app.on_child_msg(ChildMsg::Event(Event::Error {
            kind: "locked".into(),
            message: "ledger is locked".into(),
            holder: Some(crate::events::Holder {
                pid: Some(1),
                command: "verify u-lib".into(),
                started: String::new(),
            }),
        }));
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        assert!(app
            .last
            .as_deref()
            .unwrap()
            .starts_with("Scan the project — Refused"));
        key(&mut app, 't');
        assert_eq!(dialog_argv(&app), strs(&p.argv));
        code(&mut app, KeyCode::Esc);
        app.try_again = None;
        app.on_spawn_failed(p.clone(), "no such file");
        key(&mut app, 't');
        assert_eq!(dialog_argv(&app), strs(&p.argv));
    }

    /// §2.2, §8: the tree moves by rows, folds and opens, goes to the parent
    /// and into the View; a jump pushes the back stack; J/K move by unit.
    #[test]
    fn navigation_moves_folds_jumps_and_goes_back() {
        let mut app = app("nav");
        assert_eq!(app.selection, Selection::Project);
        code(&mut app, KeyCode::Down);
        assert_eq!(app.selection, Selection::Dir("test_case".into()));
        // ← folds an open node, ← again goes to the parent.
        code(&mut app, KeyCode::Left);
        assert!(!app.expansion.is_open(&Selection::Dir("test_case".into())));
        code(&mut app, KeyCode::Left);
        assert_eq!(app.selection, Selection::Project);
        code(&mut app, KeyCode::Down);
        code(&mut app, KeyCode::Right);
        assert!(app.expansion.is_open(&Selection::Dir("test_case".into())));
        // Down to lib.c, → opens it (its functions), → again into the View.
        while app.selection != Selection::File(LIB_C.into()) {
            code(&mut app, KeyCode::Down);
        }
        code(&mut app, KeyCode::Right);
        assert!(app.rows.iter().any(|r| r.selection()
            == Some(&Selection::Function(
                LIB_C.into(),
                "read_scalefactors".into()
            ))));
        code(&mut app, KeyCode::Right);
        assert_eq!(app.focus, Focus::View);
        // ← at column 0 returns to Files.
        code(&mut app, KeyCode::Left);
        assert_eq!(app.focus, Focus::Files);
        // A jump from the menu: Open unit, then Esc goes back.
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        let i = m
            .items
            .iter()
            .position(|i| matches!(i.action, Action::OpenUnit(_)))
            .unwrap();
        if let Mode::Menu(m) = &mut app.mode {
            m.focus = i;
        }
        code(&mut app, KeyCode::Enter);
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        code(&mut app, KeyCode::Esc);
        assert_eq!(app.selection, Selection::File(LIB_C.into()));
        // J/K stay within the units.
        key(&mut app, 'J');
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        key(&mut app, 'K');
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
        // Help, and the details.
        key(&mut app, '?');
        assert_eq!(app.mode, Mode::Help { scroll: 0 });
        key(&mut app, 'j');
        assert_eq!(app.mode, Mode::Help { scroll: 1 });
        key(&mut app, 'z');
        assert_eq!(app.mode, Mode::Normal);
        key(&mut app, 'c');
        assert!(matches!(app.mode, Mode::Details { .. }));
        key(&mut app, 'c');
        assert_eq!(app.mode, Mode::Normal);
    }

    /// A reload keeps the selection by key; a vanished node gives way to its
    /// parent.
    #[test]
    fn a_reload_keeps_the_selection_or_moves_to_the_parent() {
        let mut app = app("keepsel");
        attempt(&mut app, PROVENANCE);
        in_progress_attempt(&app, "a-000000000001");
        assert!(app.reload(false));
        assert_eq!(
            app.selection,
            Selection::Attempt("u-lib".into(), PROVENANCE.into())
        );
        let ledger = Ledger::new(&app.config.target);
        std::fs::remove_dir_all(attempts::attempt_dir(&ledger, "u-lib", PROVENANCE)).unwrap();
        assert!(app.reload(false));
        assert_eq!(app.selection, Selection::Unit("u-lib".into()));
    }

    /// Review STATE-2, ENG-3, §13 Freshness: the pairs follow what they show
    /// — an attempt's new candidate on the tick, an outside Rust edit to the
    /// unit crate, and an outside C edit shows the stale label.
    #[test]
    fn the_view_follows_outside_edits() {
        let mut app = app("fresh");
        app.select(Selection::Unit("u-lib".into()));
        let crate_dir = app.snapshot.units[0].crate_dir.clone().unwrap();
        let ffi = crate_dir.join("src/ffi.rs");
        let text = std::fs::read_to_string(&ffi).unwrap();
        std::fs::write(
            &ffi,
            text.replacen(
                "fn read_scalefactors(",
                "fn read_scalefactors( // outside",
                1,
            ),
        )
        .unwrap();
        app.running = true;
        app.tick();
        app.load_now();
        let has = |app: &App, needle: &str| {
            app.pairs.iter().any(|p| {
                p.rust.iter().chain(&p.c).any(|l| match l {
                    CodeLine::Code { pieces, .. } => pieces.iter().any(|(_, t)| t.contains(needle)),
                    CodeLine::Note(t) | CodeLine::Link(t) => t.contains(needle),
                })
            })
        };
        assert!(
            has(&app, "// outside"),
            "an outside Rust edit shows the new code"
        );
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("/* moved */\n{c}")).unwrap();
        app.tick();
        app.load_now();
        assert!(
            has(&app, "facts predate test_case/src/lib.c"),
            "an outside C edit shows the stale label"
        );
        // The shown attempt's candidate changes on disk (a judged turn).
        attempt(&mut app, PROVENANCE);
        let ledger = Ledger::new(&app.config.target);
        let dir = attempts::attempt_dir(&ledger, "u-lib", PROVENANCE);
        let cand = dir.join("candidate/src/ffi.rs");
        let t = std::fs::read_to_string(&cand).unwrap();
        std::fs::write(
            &cand,
            t.replacen(
                "fn read_scalefactors(",
                "fn read_scalefactors( // judged",
                1,
            ),
        )
        .unwrap();
        let mut rec = AttemptRecord::load(&dir).unwrap();
        rec.candidate_digest = "blake3:changed".into();
        rec.store(&dir).unwrap();
        app.tick();
        app.load_now();
        assert!(has(&app, "// judged"));
    }

    /// §3: a file no unit owns shows its C source; an owned file its pairs
    /// and its internal functions; a public function one pair; an internal
    /// function its file's C.
    #[test]
    fn the_view_shows_the_selection() {
        let mut app = app("viewsel");
        app.select(Selection::File(LIB_C.into()));
        assert_eq!(app.pairs.len(), 1);
        assert!(app.source.is_none());
        app.select(Selection::File("test_case/include/lib.h".into()));
        assert!(app.pairs.is_empty());
        let src = app.source.as_ref().unwrap();
        assert!(!src.lines.is_empty());
        app.select(Selection::Function(
            LIB_C.into(),
            "read_scalefactors".into(),
        ));
        assert_eq!(app.pairs.len(), 1);
        app.select(Selection::Function(
            LIB_C.into(),
            "test_case/src/lib.c::get_bits".into(),
        ));
        assert!(app.source.is_some());
        app.select(Selection::Attempt("u-lib".into(), RED.into()));
        assert_eq!(app.pairs.len(), 1);
        assert!(
            app.pairs_digest.is_none(),
            "an attempt's crate is not the unit crate"
        );
    }

    /// §3 Next step: stated as a fact, first match wins; never model work.
    #[test]
    fn the_next_step_is_a_fact() {
        let mut app = app("next");
        assert_eq!(app.next_step(), None);
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        app.reload(true);
        let (text, act) = app.next_step().unwrap();
        assert_eq!(
            text,
            "1 file changed since the scan — Scan the project again"
        );
        assert_eq!(act, Some(Act::Scan));
        std::fs::remove_file(app.config.target.join("migration/plan.toml")).unwrap();
        std::fs::write(&lib, c).unwrap();
        app.reload(true);
        assert_eq!(app.next_step().unwrap().1, Some(Act::Plan));
        std::fs::remove_file(app.config.target.join("migration/facts.jsonl")).unwrap();
        app.reload(true);
        assert_eq!(app.next_step().unwrap().1, Some(Act::Scan));
        // The project menu opens focused on it.
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        assert_eq!(m.items[m.focus].action, Action::Act(Act::Scan));
    }

    /// §4 `e`, §R2 5: after a changed edit the optional note is asked for,
    /// and travels attached; Keep for later keeps the edit (E offers it
    /// again, with its note); only an armed Discard removes it.
    #[test]
    fn a_staged_hand_edit_asks_for_its_note_then_its_dialog() {
        let mut app = app("handedit");
        let root = app.config.target.display().to_string();
        let (stage, tmp) = (PathBuf::from("/tmp/h/stage"), PathBuf::from("/tmp/h"));
        app.edit_staged("u-lib".into(), stage.clone(), tmp.clone());
        app.on_paste("-by hand");
        code(&mut app, KeyCode::Enter);
        assert_eq!(
            dialog_argv(&app),
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
        let Mode::Dialog(c) = &app.mode else {
            unreachable!()
        };
        assert_eq!(c.dialog.kind, crate::dialog::Kind::Override);
        assert_eq!(c.dialog.buttons[0].label, "Keep for later");
        // Keep for later (Esc): kept, with its note.
        assert_eq!(code(&mut app, KeyCode::Esc), Command::None);
        assert_eq!(app.kept_paths(), [tmp.join("edit")]);
        key(&mut app, 'E');
        assert!(
            matches!(&app.mode, Mode::EditNote { input, .. } if input == "-by hand"),
            "{:?}",
            app.mode
        );
        assert_eq!(code(&mut app, KeyCode::Esc), Command::None);
        assert_eq!(app.kept_edits.len(), 1, "Esc keeps it too");
        // The project menu offers the kept edit and its discard.
        app.select(Selection::Project);
        let items = app.menu_items();
        assert!(items.iter().any(|i| i.action == Action::ContinueKept));
        let discard = items
            .iter()
            .find(|i| i.action == Action::DiscardKept)
            .unwrap()
            .clone();
        app.open_menu();
        if let Mode::Menu(m) = &mut app.mode {
            m.focus = m.items.iter().position(|i| *i == discard).unwrap();
        }
        code(&mut app, KeyCode::Enter);
        let Mode::Dialog(c) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert_eq!(c.dialog.focus, 0, "focused on Keep");
        // D discards — only once armed.
        assert_eq!(key(&mut app, 'D'), Command::None);
        assert!(matches!(app.mode, Mode::Dialog(_)));
        arm(&mut app);
        assert_eq!(key(&mut app, 'D'), Command::Cleanup(tmp.clone()));
        assert!(app.kept_edits.is_empty());
        key(&mut app, 'E');
        assert!(said(&app).contains("no kept hand edit"), "{}", said(&app));
        // The crate must be in the executor layout for `e` at all.
        attempt(&mut app, PROVENANCE);
        assert!(matches!(key(&mut app, 'e'), Command::Edit { .. }));
    }

    /// Review ACTS-2 / STATE-3 / PROC-1: an override that recorded nothing
    /// keeps the edit, E offers it again; only a recorded one frees its dir.
    #[test]
    fn an_unrecorded_override_keeps_the_edit_and_reoffers_it() {
        let mut app = app("keepedit");
        let tmp = PathBuf::from("/tmp/k");
        app.edit_staged("u-lib".into(), tmp.join("stage"), tmp.clone());
        code(&mut app, KeyCode::Enter);
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn: {:?}", app.notice);
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
        key(&mut app, 'E');
        code(&mut app, KeyCode::Enter);
        assert_eq!(dialog_argv(&app), strs(&argv));
        arm(&mut app);
        let Command::Spawn(p) = key(&mut app, 'y') else {
            panic!("y must spawn");
        };
        app.on_spawn_failed(p.clone(), "no such file");
        assert_eq!(app.kept_edits.len(), 1);
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

    /// Review ACTS-2: notes the CLI refuses are refused at the prompt; a
    /// paste that does not fit says so; a paste never answers a dialog.
    #[test]
    fn notes_and_pastes() {
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
        assert!(said(&app).contains("section header"));
        code(&mut app, KeyCode::Esc);
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'm');
        app.on_paste(&"x".repeat(MAX_NOTE_BYTES + 10));
        assert!(said(&app).contains("10 bytes dropped"));
        code(&mut app, KeyCode::Esc);
        key(&mut app, 'a');
        app.on_paste("y\ny");
        assert!(matches!(app.mode, Mode::Dialog(_)));
        assert!(said(&app).contains("paste ignored"));
    }

    /// Review ACTS-3: every outstanding hand-off is tracked; Resume resumes
    /// the selected attempt's.
    #[test]
    fn several_hand_offs_are_tracked() {
        let mut app = app("handoffs");
        let (a, b) = ("a-00000000000a", "a-00000000000b");
        in_progress_attempt(&app, a);
        in_progress_attempt(&app, b);
        app.reload(true);
        for (id, n) in [(a, 1), (b, 2)] {
            let path = app.config.target.join(format!("r{n}.json"));
            std::fs::write(&path, "{}").unwrap();
            app.on_spawned(&pending(
                vec![
                    OsString::from(format!("run-{n}")),
                    OsString::from("--steer=x"),
                ],
                Act::Modify,
            ));
            app.on_child_msg(ChildMsg::Event(Event::Awaiting {
                attempt: Some(id.into()),
                path: path.display().to_string(),
                resume: String::new(),
                args: None,
            }));
            app.on_child_exit(ExitStatus::from_raw(1 << 8));
            app.load_now();
        }
        assert_eq!(app.awaiting.len(), 2);
        attempt(&mut app, a);
        key(&mut app, 'R');
        assert_eq!(dialog_argv(&app), ["run-1", "--steer=x"]);
        code(&mut app, KeyCode::Esc);
        attempt(&mut app, b);
        key(&mut app, 'R');
        assert_eq!(dialog_argv(&app), ["run-2", "--steer=x"]);
    }

    /// §6.2: a plan run leaves one summary line until the next command.
    #[test]
    fn a_plan_run_leaves_its_summary() {
        let mut app = app("plansum");
        let p = app.act_argv(Act::Plan, None, None, None).unwrap();
        app.on_spawned(&p);
        for text in ["plan: u-lib: re-approved", "plan: execution order: u-lib"] {
            app.on_child_msg(ChildMsg::Event(Event::Message { text: text.into() }));
        }
        app.on_child_exit(ExitStatus::from_raw(0));
        app.load_now();
        assert_eq!(
            app.plan_notice.as_deref(),
            Some("plan changed 1 unit — review `git diff migration/plan.toml` (c for the lines)")
        );
        app.on_spawned(&app.act_argv(Act::Scan, None, None, None).unwrap());
        assert_eq!(app.plan_notice, None);
    }

    /// §6.3: a failed read keeps the last snapshot and says why once; `g`
    /// always says it; requests fold.
    #[test]
    fn a_failed_read_keeps_the_last_snapshot() {
        let mut app = app("failread");
        let units = app.snapshot.units.len();
        assert!(!app.on_loaded(Err("boom".into()), LoadWhy::Tick));
        assert_eq!(app.snapshot.units.len(), units);
        assert_eq!(said(&app), "unreadable: boom");
        app.notice = None;
        assert!(!app.on_loaded(Err("boom".into()), LoadWhy::Tick));
        assert_eq!(app.notice, None, "said once");
        assert!(!app.on_loaded(Err("boom".into()), LoadWhy::Key));
        assert_eq!(said(&app), "unreadable: boom");
        app.request_load(LoadWhy::Reaped);
        app.request_load(LoadWhy::Tick);
        assert_eq!(app.load_request, Some(LoadWhy::Reaped));
    }

    /// Review STATE-1, STATE-9: the diff covers every file of both sides,
    /// is bounded as a whole, and refuses huge files.
    #[test]
    fn the_diff_is_complete_and_bounded() {
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
        assert!(lines.last().unwrap().contains("the diff stops here"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `d` compares an attempt with the provenance attempt.
    #[test]
    fn compare_is_offered_against_the_provenance_attempt() {
        let mut app = app("diff");
        attempt(&mut app, PROVENANCE);
        assert!(
            !app.menu_items().iter().any(|i| i.action == Action::Compare),
            "not with itself"
        );
        attempt(&mut app, SUPERSEDED);
        key(&mut app, 'd');
        let Mode::Diff { title, .. } = &app.mode else {
            panic!("{:?}", app.notice);
        };
        assert!(title.contains(PROVENANCE));
    }

    fn locked(app: &mut App, p: &Pending) {
        app.on_spawned(p);
        app.on_child_msg(ChildMsg::Event(Event::Error {
            kind: "locked".into(),
            message: "ledger is locked".into(),
            holder: Some(crate::events::Holder {
                pid: Some(1),
                command: "verify u-lib".into(),
                started: String::new(),
            }),
        }));
        app.on_child_exit(ExitStatus::from_raw(1 << 8));
        app.load_now();
    }

    /// Review USE-2/ENG-1/SAFE-4: Try again offers the WHOLE command again —
    /// a Re-check keeps its unit, runs its gates, names its object.
    #[test]
    fn try_again_offers_the_whole_command() {
        let mut app = app("tryagain2");
        app.select(Selection::Unit("u-lib".into()));
        let p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        locked(&mut app, &p);
        assert!(app
            .last
            .as_deref()
            .unwrap()
            .contains("another command is changing this project"));
        key(&mut app, 't');
        let Mode::Dialog(c) = &app.mode else {
            panic!("{:?}", app.notice)
        };
        assert_eq!(c.title, "Re-check u-lib with the oracle?");
        arm(&mut app);
        let Command::Spawn(again) = key(&mut app, 'y') else {
            panic!("refused: {}", said(&app));
        };
        assert_eq!(again.argv, p.argv);
        assert_eq!(again.unit.as_deref(), Some("u-lib"));
    }

    /// Review PROC-1: a read that started before a command was reaped is
    /// dropped when the reaped (or asked-for) read follows; its reasons
    /// carry over; a lone tick applies.
    #[test]
    fn a_read_superseded_by_the_reaped_read_is_dropped() {
        let mut asked = vec![(1, LoadWhy::Tick), (2, LoadWhy::Reaped)];
        assert_eq!(fold_loaded(&mut asked, 1), None);
        assert_eq!(asked, [(2, LoadWhy::Reaped)]);
        assert_eq!(fold_loaded(&mut asked, 2), Some(LoadWhy::Reaped));
        let mut asked = vec![(3, LoadWhy::Key), (4, LoadWhy::Reaped)];
        assert_eq!(fold_loaded(&mut asked, 3), None);
        assert_eq!(asked, [(4, LoadWhy::Key)], "the Key carries over");
        let mut asked = vec![(5, LoadWhy::Tick), (6, LoadWhy::Tick)];
        assert_eq!(
            fold_loaded(&mut asked, 5),
            Some(LoadWhy::Tick),
            "ticks never starve"
        );
        let mut asked = vec![(7, LoadWhy::Tick), (8, LoadWhy::Reaped)];
        assert_eq!(fold_loaded(&mut asked, 8), Some(LoadWhy::Reaped), "folded");
        assert!(asked.is_empty());
    }

    /// Review SAFE-12: a blind hand-off is never tracked for Resume, and a
    /// Resume confirms only for a steer attempt still in progress.
    #[test]
    fn resume_is_only_for_steer_hand_offs_rechecked_at_confirm() {
        let mut app = app("resumeblind");
        let awaited = "a-000000000abd";
        in_progress_attempt(&app, awaited);
        app.reload(true);
        let response = app.config.target.join("response.json");
        std::fs::write(&response, "{\"text\": \"x\"}").unwrap();
        let blind = vec![OsString::from(HARNESS), OsString::from("migrate")];
        let awaiting = |app: &mut App, argv: Vec<OsString>| {
            app.on_spawned(&pending(argv, Act::Retry));
            app.on_child_msg(ChildMsg::Event(Event::Awaiting {
                attempt: Some(awaited.into()),
                path: response.display().to_string(),
                resume: String::new(),
                args: None,
            }));
            app.on_child_exit(ExitStatus::from_raw(1 << 8));
            app.load_now();
        };
        awaiting(&mut app, blind);
        assert!(
            app.awaiting.is_empty(),
            "a blind hand-off is the audited protocol's"
        );
        let steer = vec![OsString::from(HARNESS), OsString::from("--steer=x")];
        awaiting(&mut app, steer);
        assert_eq!(app.awaiting.len(), 1);
        attempt(&mut app, awaited);
        key(&mut app, 'R');
        assert!(matches!(app.mode, Mode::Dialog(_)), "{}", said(&app));
        // The record loses its seed under the open dialog: refused.
        record(&mut app, awaited, |r| r.seeded_from = None);
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(said(&app).contains("not a steer attempt"), "{}", said(&app));
    }

    /// Review SAFE-1, SAFE-2: at confirm the plan is read again — a unit
    /// whose plan now names another crate is refused — and the preflight
    /// runs first, so a link planted in the crate is refused, never hashed.
    #[test]
    fn recheck_confirms_on_a_fresh_plan_and_a_preflight() {
        let mut app = app("recheckfresh");
        app.select(Selection::Unit("u-lib".into()));
        let open = |app: &mut App| {
            let p = app
                .act_argv(Act::Verify, Some("u-lib"), None, None)
                .unwrap();
            app.ask(p);
            arm(app);
        };
        open(&mut app);
        let plan = app.config.target.join("migration/plan.toml");
        let text = std::fs::read_to_string(&plan).unwrap();
        std::fs::write(
            &plan,
            text.replace("rust_crate = \"u_lib_rs\"", "rust_crate = \"other_rs\""),
        )
        .unwrap();
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(said(&app).contains("another crate"), "{}", said(&app));
        std::fs::write(&plan, &text).unwrap();
        open(&mut app);
        let src = app.snapshot.units[0].crate_dir.clone().unwrap().join("src");
        std::os::unix::fs::symlink("/dev/zero", src.join("zero.rs")).unwrap();
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(
            said(&app).contains("cannot be read safely"),
            "{}",
            said(&app)
        );
        std::fs::remove_file(src.join("zero.rs")).unwrap();
        open(&mut app);
        assert!(
            matches!(key(&mut app, 'y'), Command::Spawn(_)),
            "{}",
            said(&app)
        );
    }

    /// Review SAFE-3/ENG-8: the digest shown is captured when the dialog
    /// opens; a load behind the open dialog cannot move it.
    #[test]
    fn a_load_behind_the_dialog_never_moves_what_was_shown() {
        let mut app = app("digestpin");
        app.select(Selection::Unit("u-lib".into()));
        let p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        app.ask(p);
        // Another KNOWN crate replaces it (a recorded candidate).
        let unit_crate = app.snapshot.units[0].crate_dir.clone().unwrap();
        let ledger = Ledger::new(&app.config.target);
        let cand = attempts::attempt_dir(&ledger, "u-lib", RED).join("candidate/src/logic.rs");
        std::fs::copy(&cand, unit_crate.join("src/logic.rs")).unwrap();
        let cand_ffi = attempts::attempt_dir(&ledger, "u-lib", RED).join("candidate/src/ffi.rs");
        std::fs::copy(&cand_ffi, unit_crate.join("src/ffi.rs")).unwrap();
        // A tick load lands while the dialog is open.
        app.request_load(LoadWhy::Reaped);
        app.load_now();
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(
            said(&app).contains("changed on disk since the cockpit showed it"),
            "{}",
            said(&app)
        );
    }

    /// Review SAFE-10: a lock file that cannot be read is busy, not free.
    #[test]
    fn an_unreadable_lock_is_busy() {
        let mut app = app("lockdir");
        let lock = Ledger::new(&app.config.target).lock_path();
        let _ = std::fs::remove_file(&lock);
        std::fs::create_dir_all(&lock).unwrap();
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            said(&app).contains("the writer lock could not be read"),
            "{}",
            said(&app)
        );
    }

    /// Review ENG-3, ENG-6: the Next step skips a blocked unit and knows an
    /// empty plan from none; a missing file counts once.
    #[test]
    fn the_next_step_skips_blocked_units_and_counts_once() {
        let mut app = app("nextstep2");
        let plan = app.config.target.join("migration/plan.toml");
        let text = std::fs::read_to_string(&plan).unwrap();
        std::fs::write(
            &plan,
            text.replace("status = \"verified\"", "status = \"blocked\"")
                .replace("source_hash = \"blake3:fd", "source_hash = \"blake3:00"),
        )
        .unwrap();
        assert!(app.reload(true));
        assert!(!app.snapshot.units[0].report.source_fresh);
        assert_eq!(app.next_step(), None, "a blocked unit is no next step");
        let head = text.split("[[unit]]").next().unwrap().to_string();
        std::fs::write(&plan, head).unwrap();
        assert!(app.reload(true));
        assert!(app.snapshot.units.is_empty());
        assert_eq!(app.next_step(), None, "an empty plan is a plan");
        std::fs::remove_file(app.config.target.join("test_case/include/lib.h")).unwrap();
        assert!(app.reload(true));
        assert_eq!(
            app.next_step().unwrap().0,
            "1 file changed since the scan — Scan the project again"
        );
    }

    /// Review USE-4: an internal function's source opens at the function.
    #[test]
    fn a_function_opens_at_its_line() {
        let mut app = app("fnline");
        app.select(Selection::Function(
            LIB_C.into(),
            "test_case/src/lib.c::get_bits".into(),
        ));
        assert_eq!(app.scroll, 2, "get_bits starts at line 3");
    }

    /// Review USE-13, USE-14: Enter in the View opens the menu until a link
    /// was chosen; sideways scroll stops at the widest line; Space does not
    /// page the tree; Ctrl-C closes the checks.
    #[test]
    fn view_keys_behave() {
        let mut app = app("viewkeys");
        let lib = app.config.target.join(LIB_C);
        let c = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("{c}\n")).unwrap();
        assert!(app.reload(true));
        app.links = vec![Selection::File(LIB_C.into())];
        app.focus = Focus::View;
        code(&mut app, KeyCode::Enter);
        assert!(
            matches!(app.mode, Mode::Menu(_)),
            "no link chosen: the menu"
        );
        code(&mut app, KeyCode::Esc);
        code(&mut app, KeyCode::Down);
        code(&mut app, KeyCode::Enter);
        assert_eq!(app.selection, Selection::File(LIB_C.into()));
        app.select(Selection::Unit("u-lib".into()));
        app.focus = Focus::View;
        app.links.clear();
        for _ in 0..1000 {
            code(&mut app, KeyCode::Right);
        }
        assert!(
            app.hscroll <= app.code_cols,
            "{} > {}",
            app.hscroll,
            app.code_cols
        );
        assert!(app.hscroll > 0);
        app.focus = Focus::Files;
        let before = app.selection.clone();
        key(&mut app, ' ');
        assert_eq!(app.selection, before);
        key(&mut app, 'v');
        assert!(matches!(app.mode, Mode::Verdict { .. }));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.on_key(ctrl_c, Instant::now());
        assert_eq!(app.mode, Mode::Normal);
    }

    /// Review USE-9, SAFE-13, USE-11: the words follow the sandbox flag;
    /// ids that are not plain never reach an argv; E works from anywhere
    /// and Discard's dialog says what it is.
    #[test]
    fn words_ids_and_the_kept_edit() {
        let mut app = app("words");
        app.config.allow_unsandboxed = true;
        let p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        let (_, body) = app.dialog_words(&p);
        assert!(
            !body.iter().any(|b| b.contains("in the sandbox")),
            "{body:?}"
        );
        // An attempt whose id is not a plain segment (a copied record named
        // `-x`): it exists, and still never reaches the argv.
        let ledger = Ledger::new(&app.config.target);
        let from = attempts::attempt_dir(&ledger, "u-lib", PROVENANCE);
        let odd = from.parent().unwrap().join("-x");
        assert!(std::process::Command::new("cp")
            .arg("-R")
            .arg(&from)
            .arg(&odd)
            .status()
            .unwrap()
            .success());
        let mut rec = AttemptRecord::load(&odd).unwrap();
        rec.id = "-x".into();
        rec.store(&odd).unwrap();
        assert!(app.reload(true));
        assert!(
            app.snapshot.units[0].attempt("-x").is_some(),
            "the odd attempt is read"
        );
        let err = app
            .act_argv(Act::Accept, Some("u-lib"), Some("-x"), None)
            .unwrap_err();
        assert!(err.contains("not a plain id"), "{err}");
        app.edit_staged(
            "u-lib".into(),
            PathBuf::from("/tmp/w/stage"),
            PathBuf::from("/tmp/w"),
        );
        code(&mut app, KeyCode::Esc);
        app.select(Selection::File(LIB_C.into()));
        key(&mut app, 'E');
        assert!(
            matches!(app.mode, Mode::EditNote { .. }),
            "E from a file row"
        );
        code(&mut app, KeyCode::Esc);
        app.select(Selection::Project);
        app.open_menu();
        if let Mode::Menu(m) = &mut app.mode {
            m.focus = m
                .items
                .iter()
                .position(|i| i.action == Action::DiscardKept)
                .unwrap();
        }
        code(&mut app, KeyCode::Enter);
        let Mode::Dialog(c) = &app.mode else { panic!() };
        assert!(
            c.title.contains("keep it, record it, or discard it"),
            "{}",
            c.title
        );
    }

    /// Second fix pass, SAFE-8/NEW-2: Accept re-checks the unit crate at
    /// confirm — an outside edit after the dialog opened is refused, from
    /// the menu and from Try again.
    #[test]
    fn accept_refuses_unknown_code_at_confirm() {
        let mut app = app("acceptknown");
        attempt(&mut app, PROVENANCE);
        key(&mut app, 'a');
        assert!(matches!(app.mode, Mode::Dialog(_)));
        let logic = app.snapshot.units[0]
            .crate_dir
            .clone()
            .unwrap()
            .join("src/logic.rs");
        let text = std::fs::read_to_string(&logic).unwrap();
        std::fs::write(&logic, format!("{text}\n// by someone\n")).unwrap();
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(said(&app).contains("does not know"), "{}", said(&app));
        // Try again of a lock-refused Accept: the same gate.
        std::fs::write(&logic, &text).unwrap();
        let p = app
            .act_argv(Act::Accept, Some("u-lib"), Some(PROVENANCE), None)
            .unwrap();
        locked(&mut app, &p);
        std::fs::write(&logic, format!("{text}\n// again\n")).unwrap();
        key(&mut app, 't');
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(said(&app).contains("does not know"), "{}", said(&app));
    }

    /// Second fix pass, NEW-6: the confirm-time preflight runs only where
    /// confirm reads — a broken record elsewhere never refuses the Scan that
    /// could repair things; it refuses a Re-check.
    #[test]
    fn a_broken_record_never_refuses_a_scan() {
        let mut app = app("scanpf");
        let big = attempts::attempt_dir(&Ledger::new(&app.config.target), "u-lib", RED)
            .join("attempt-verdict.json");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(crate::preflight::MAX_LEDGER_FILE_BYTES + 1)
            .unwrap();
        let scan = app.act_argv(Act::Scan, None, None, None).unwrap();
        app.ask(scan);
        arm(&mut app);
        assert!(
            matches!(key(&mut app, 'y'), Command::Spawn(_)),
            "{}",
            said(&app)
        );
        app.select(Selection::Unit("u-lib".into()));
        let verify = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        app.ask(verify);
        arm(&mut app);
        assert_eq!(key(&mut app, 'y'), Command::None);
        assert!(
            said(&app).contains("cannot be read safely"),
            "{}",
            said(&app)
        );
    }

    /// Second fix pass, NEW-2/NEW-8: a Re-check with nothing shown opens no
    /// dialog and says why; one shown keeps its digest through Try again.
    #[test]
    fn a_recheck_needs_its_code_shown() {
        let mut app = app("recheckshown");
        let p = app
            .act_argv(Act::Verify, Some("u-lib"), None, None)
            .unwrap();
        app.ask(p.clone());
        assert_eq!(app.mode, Mode::Normal);
        assert!(said(&app).contains("open u-lib"), "{}", said(&app));
        app.select(Selection::Unit("u-lib".into()));
        app.ask(p);
        let Mode::Dialog(c) = &app.mode else { panic!() };
        let Purpose::Act(shown) = &c.purpose else {
            panic!()
        };
        let shown = shown.clone();
        code(&mut app, KeyCode::Esc);
        locked(&mut app, &shown);
        app.select(Selection::Project);
        key(&mut app, 't');
        arm(&mut app);
        assert!(
            matches!(key(&mut app, 'y'), Command::Spawn(_)),
            "{}",
            said(&app)
        );
    }

    /// Second fix pass, NEW-5, NEW-7: a stale link never swallows Enter; a
    /// re-read keeps where the user scrolled a function's source.
    #[test]
    fn stale_links_and_rereads() {
        let mut app = app("stalelink");
        app.focus = Focus::View;
        app.link = Some(3);
        app.links.clear();
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Menu(_)));
        code(&mut app, KeyCode::Esc);
        app.focus = Focus::Files;
        app.select(Selection::Function(
            LIB_C.into(),
            "test_case/src/lib.c::get_bits".into(),
        ));
        app.scroll = 15;
        assert!(app.reload(true));
        assert_eq!(app.scroll, 15);
    }

    /// Second fix pass, NEW-8: discarding a kept edit also drops its Try
    /// again; SAFE-9: a hand edit re-reads the crate first.
    #[test]
    fn discard_drops_try_again_and_edits_reread_the_crate() {
        let mut app = app("discardtry");
        let tmp = PathBuf::from("/tmp/dt");
        app.edit_staged("u-lib".into(), tmp.join("stage"), tmp.clone());
        code(&mut app, KeyCode::Enter);
        let Mode::Dialog(c) = &app.mode else { panic!() };
        let Purpose::Act(p) = &c.purpose else {
            panic!()
        };
        let p = p.clone();
        code(&mut app, KeyCode::Esc);
        locked(&mut app, &p);
        assert!(app.try_again.is_some());
        key(&mut app, 't');
        arm(&mut app);
        assert_eq!(key(&mut app, 'D'), Command::Cleanup(tmp));
        assert!(app.try_again.is_none());
        app.select(Selection::Crate("u-lib".into()));
        assert!(app.hand_edit_target().is_ok());
        let logic = app.snapshot.units[0]
            .crate_dir
            .clone()
            .unwrap()
            .join("src/logic.rs");
        let text = std::fs::read_to_string(&logic).unwrap();
        std::fs::write(&logic, format!("{text}\n// later\n")).unwrap();
        assert!(app
            .hand_edit_target()
            .unwrap_err()
            .contains("changed on disk"));
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
