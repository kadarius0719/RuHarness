//! A model proposes the features and the workloads
//! (docs/COCKPIT-WRAPPER-DESIGN.md §9.6): the chat writes a proposal
//! beside the person's file through harness-mcp
//! (`harness_features_propose`, `harness_workloads_propose`); the cockpit
//! opens it in the person's editor as their draft, and their save writes
//! the file. Writing stays the person's; proposing is the model's.
//!
//! - **Propose features — ask in chat** (the Features row) and **Propose a
//!   workload — ask in chat** (the Speed row) type their question into the
//!   chat, as the Project view's items do; greyed with the reason while the
//!   chat cannot be used, and recommended only on their own row's Next step
//!   while it can;
//! - the Next step on a row whose proposal waits and whose file does not
//!   exist: read the proposal (Write your … file opens it).

use super::features_edit::DraftKind;
use super::{notice, App, Focus};
use crate::menu::{item, Action, Item};
use crate::next::Advice;
use crate::tree::Selection;
use harness_core::text::safe_line;

/// The chat's proposal as the Features (Speed) view shows it, so the person
/// reads the draft before opening the editor: its counts, its file, and one
/// line per scenario (workload) — the chat's comment above it, else its
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The proposal's file, root-relative.
    pub path: String,
    /// "3 features, 8 scenarios" or "2 workloads".
    pub counts: String,
    /// A feature's count alone ("3 features"), or the workloads' count.
    pub what: String,
    /// `feature/scenario` (or the workload's id) and what it does, each
    /// filtered for the screen.
    pub rows: Vec<(String, String)>,
}

/// The `#` comment lines right above each `[[table]]` header in `text`,
/// in file order (empty when there is none; a blank line ends a comment).
fn comments_above(text: &str, table: &str) -> Vec<String> {
    let header = format!("[[{table}]]");
    let mut out = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(c) = t.strip_prefix('#') {
            run.push(c.trim().to_string());
        } else if t == header {
            out.push(run.join(" "));
            run.clear();
        } else {
            run.clear();
        }
    }
    out
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// The proposal's `text` read as its file (`kind`): its preview, or the
/// loader's message in words.
pub fn preview(kind: DraftKind, text: &str, path: String) -> Result<Preview, String> {
    use harness_core::features;
    use harness_core::perf::workloads;
    let what_it_does = |comment: Option<&String>, args: &[String]| match comment {
        Some(c) if !c.is_empty() => safe_line(c),
        _ if args.is_empty() => "no arguments".to_string(),
        _ => safe_line(&args.join(" ")),
    };
    match kind {
        DraftKind::Features => {
            const MARK: &str = "\u{1}";
            let f = features::parse(text, std::path::Path::new(features::PROPOSED_FILE), MARK)
                .map_err(|e| match e {
                    harness_core::error::Error::InvalidPlan(m) => m
                        .split_once(": ")
                        .filter(|(head, _)| head.starts_with(MARK))
                        .map_or(m.clone(), |(_, rest)| rest.to_string()),
                    other => other.to_string(),
                })?;
            let comments = comments_above(text, "scenario");
            let rows = f
                .scenarios
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    (
                        safe_line(&format!("{}/{}", s.feature, s.id)),
                        what_it_does(comments.get(i), &s.args),
                    )
                })
                .collect();
            Ok(Preview {
                path,
                counts: format!(
                    "{}, {}",
                    plural(f.features.len(), "feature"),
                    plural(f.scenarios.len(), "scenario")
                ),
                what: plural(f.features.len(), "feature"),
                rows,
            })
        }
        DraftKind::Workloads => {
            let w = workloads::parse(text, std::path::Path::new(workloads::PROPOSED_FILE))
                .map_err(|e| match e {
                    workloads::ParseError::Rule(e) => {
                        format!("line {}, column {}: {}", e.line, e.column, e.message)
                    }
                    workloads::ParseError::TooNew(e) => e.to_string(),
                })?;
            let comments = comments_above(text, "workload");
            let rows = w
                .workloads
                .iter()
                .enumerate()
                .map(|(i, wl)| (safe_line(&wl.id), what_it_does(comments.get(i), &wl.args)))
                .collect();
            let n = plural(w.workloads.len(), "workload");
            Ok(Preview {
                path,
                counts: n.clone(),
                what: n,
                rows,
            })
        }
        DraftKind::BuildConfig => Err("not a proposal of features or workloads".into()),
    }
}

impl App {
    /// The chat's proposal of `kind`'s file, read with the opening's
    /// safety (a regular file, the file's cap, UTF-8): `None` when there is
    /// none; else its preview, or why it does not read, in words.
    pub fn proposal_preview(&self, kind: DraftKind) -> Option<Result<Preview, String>> {
        let full = kind.proposed_path(&self.config)?;
        let path = full
            .strip_prefix(&self.config.target)
            .unwrap_or(&full)
            .display()
            .to_string();
        match self.read_proposal(kind) {
            Ok(None) => None,
            Ok(Some(text)) => Some(preview(kind, &text, path)),
            Err(why) => Some(Err(why)),
        }
    }

    /// The hint line for the Features (Speed) row while a proposal waits
    /// (§9.6): "The chat proposed 3 features; Write your features file
    /// opens them as your draft."
    pub fn proposal_hint(&self, kind: DraftKind) -> Option<String> {
        let p = self.proposal_preview(kind)?.ok()?;
        let noun = kind.noun();
        Some(if self.has_own_file(kind) {
            format!(
                "The chat proposed {}; Edit from the chat's proposal… opens them against your \
                 {noun} file.",
                p.what
            )
        } else {
            format!(
                "The chat proposed {}; Write your {noun} file opens them as your draft.",
                p.what
            )
        })
    }

    /// Tell the chat which of the person's drafts are kept, before it
    /// reads what the runtime printed: a proposal of that file is refused
    /// while one is (the draft slot is theirs).
    pub(crate) fn sync_drafts_kept(&mut self) {
        self.chat.drafts_kept = [
            self.features_draft.is_some(),
            self.workloads_draft.is_some(),
        ];
    }

    /// The chat's proposal is written: said, with the item that opens it.
    pub(crate) fn chat_proposal_written(&mut self, workloads: bool) {
        let kind = if workloads {
            DraftKind::Workloads
        } else {
            DraftKind::Features
        };
        let noun = kind.noun();
        let opens = if self.has_own_file(kind) {
            "Edit from the chat's proposal…".to_string()
        } else {
            format!("Write your {noun} file")
        };
        self.notice = notice(format!(
            "the chat's proposal of your {noun} file is written — {opens} opens it as your draft"
        ));
    }

    /// The chat's proposal of `kind`'s file is there (a regular file; the
    /// read that opens it checks again).
    pub fn has_proposal(&self, kind: DraftKind) -> bool {
        kind.proposed_path(&self.config)
            .is_some_and(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()))
    }

    /// The person's own `kind` file exists (valid or not).
    pub(crate) fn has_own_file(&self, kind: DraftKind) -> bool {
        match kind {
            DraftKind::Features => !matches!(
                self.snapshot.features,
                harness_core::features::FeatureSnapshot::None
            ),
            DraftKind::Workloads => self.speed.group != crate::speed::Group::NoFile,
            DraftKind::BuildConfig => false,
        }
    }

    /// The program's name in a question: the tool's id, else the folder's.
    pub fn program_word(&self) -> String {
        match &self.config.tool {
            Some(id) => safe_line(id),
            None => self.config.target.file_name().map_or_else(
                || "this program".into(),
                |n| safe_line(&n.to_string_lossy()),
            ),
        }
    }

    /// The question a Propose item types (§9.6).
    pub fn proposal_question(&self, workloads: bool) -> String {
        let name = self.program_word();
        if workloads {
            format!("Propose a workload for {name}: a run long enough to time, with a real input")
        } else {
            format!(
                "Propose features for {name}: what a person does with it and can see the result \
                 of"
            )
        }
    }

    /// Propose features (or a workload) — ask in chat: the question typed
    /// into the input (never over a draft), the chat focused; the person
    /// reads it and sends it.
    pub(crate) fn chat_ask_proposal(&mut self, workloads: bool) {
        if !self.chat.input.is_empty() {
            self.say("the chat has a draft — send or clear it first");
            self.move_focus(Focus::Chat);
            return;
        }
        let text = self.proposal_question(workloads);
        self.chat.input.insert(&text);
        self.move_focus(Focus::Chat);
        self.notice = notice(
            "the question is typed into the chat — press Enter to send it; the chat reads the \
             program through the harness and writes a proposal you open, change and save",
        );
    }

    /// Why a Propose item cannot be chosen now, if so: the chat unusable,
    /// a draft in its input, a draft of the person's kept, nothing scanned.
    fn propose_greyed(&self, kind: DraftKind) -> Option<String> {
        self.chat_unavailable()
            .or_else(|| {
                (!self.chat.input.is_empty())
                    .then(|| "the chat has a draft — send or clear it first".to_string())
            })
            .or_else(|| {
                self.draft(kind)
                    .is_some()
                    .then(|| "a draft of yours is kept — save or discard it first".to_string())
            })
            .or_else(|| {
                self.snapshot.facts.is_none().then(|| {
                    "nothing is scanned yet — Scan the project first: the chat reads the \
                     program through it"
                        .to_string()
                })
            })
    }

    /// The Features row's and the Speed row's proposal items.
    pub(crate) fn proposal_items(&self, sel: &Selection, items: &mut Vec<Item>) {
        let (kind, label, action) = match sel {
            Selection::Features if !self.features.no_single_main => (
                DraftKind::Features,
                "Propose features — ask in chat",
                Action::Propose { workloads: false },
            ),
            Selection::Speed => (
                DraftKind::Workloads,
                "Propose a workload — ask in chat",
                Action::Propose { workloads: true },
            ),
            _ => return,
        };
        let mut it = item(label, action, None);
        it.model = true;
        it.greyed = self.propose_greyed(kind);
        items.push(it);
        // With the person's file there, the proposal opens against it.
        if self.has_own_file(kind) && self.has_proposal(kind) && self.draft(kind).is_none() {
            let mut it = item(
                "Edit from the chat's proposal…",
                Action::EditFromProposal {
                    workloads: kind == DraftKind::Workloads,
                },
                None,
            );
            if self.running {
                it.greyed = Some("a command is running (one at a time)".into());
            }
            items.push(it);
        }
    }

    /// The Next step on the Features (or Speed) row with no file of the
    /// person's: the proposal to read, when one waits; else Propose, when
    /// the chat can be used — `None` leaves the row's own.
    pub(crate) fn proposal_advice(&self, kind: DraftKind) -> Option<Advice> {
        if self.draft(kind).is_some() || self.has_own_file(kind) {
            return None;
        }
        let noun = kind.noun();
        if self.has_proposal(kind) {
            return Some(Advice {
                text: format!("read the chat's proposal — Enter, then Write your {noun} file"),
                action: Some(match kind {
                    DraftKind::Workloads => Action::EditWorkloads,
                    _ => Action::EditFeatures,
                }),
                reason: None,
            });
        }
        if self.propose_greyed(kind).is_some() {
            return None;
        }
        Some(match kind {
            DraftKind::Workloads => Advice {
                text: "no workloads file yet — press Enter and choose Propose a workload, or \
                       Write your workloads file yourself"
                    .into(),
                action: Some(Action::Propose { workloads: true }),
                reason: None,
            },
            _ => Advice {
                text: "no features yet — press Enter and choose Propose features, or Write your \
                       features file yourself"
                    .into(),
                action: Some(Action::Propose { workloads: false }),
                reason: None,
            },
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::tests::app_of_without_features;
    use crate::app::Mode;

    /// The chat on, its `claude` found and answering: its items can be
    /// chosen (as the Project view's tests set it).
    pub(crate) fn with_chat(app: &mut App) {
        app.set_chat(true);
        app.chat.bins = Ok(crate::chat::runtime::Binaries {
            claude: "/bin/sh".into(),
            mcp: "/bin/sh".into(),
        });
        app.chat.problem = None;
        app.chat.installed = true;
    }

    fn find(app: &App, label: &str) -> Option<Item> {
        app.menu_items().into_iter().find(|i| i.label == label)
    }

    const FEATURES: &str = "Propose features — ask in chat";
    const WORKLOAD: &str = "Propose a workload — ask in chat";

    /// With the chat off, both items are listed under the model separator,
    /// greyed with the reason, and the Next step stays Write.
    #[test]
    fn with_the_chat_off_the_items_are_greyed_and_never_recommended() {
        let mut app = app_of_without_features("targets/zopfli", "propose-off");
        for (sel, label) in [
            (Selection::Features, FEATURES),
            (Selection::Speed, WORKLOAD),
        ] {
            app.select(sel.clone());
            let it = find(&app, label).unwrap_or_else(|| panic!("{label}"));
            assert!(it.model, "{it:?}");
            assert_eq!(it.greyed.as_deref(), Some("the chat is off (--no-chat)"));
        }
        let a = app.advice(&Selection::Features).unwrap();
        assert_eq!(a.action, Some(Action::EditFeatures), "{a:?}");
        let a = app.advice(&Selection::Speed).unwrap();
        assert_eq!(a.action, Some(Action::EditWorkloads), "{a:?}");
        // Neither on another row.
        app.select(Selection::Project);
        assert!(find(&app, FEATURES).is_none());
        assert!(find(&app, WORKLOAD).is_none());
    }

    /// With the chat on and no file: Propose is the Next step and the
    /// menu's recommended item, before Write; chosen, its question is
    /// typed into the chat (nothing sent, nothing run); then it is greyed
    /// over the chat's draft, and over a draft of the person's.
    #[test]
    fn with_the_chat_on_propose_is_recommended_and_types_its_question() {
        let mut app = app_of_without_features("targets/zopfli", "propose-on");
        with_chat(&mut app);
        let name = app.program_word();
        for (sel, label, workloads, reason, question) in [
            (
                Selection::Features,
                FEATURES,
                false,
                "no features yet",
                format!(
                    "Propose features for {name}: what a person does with it and can see the \
                     result of"
                ),
            ),
            (
                Selection::Speed,
                WORKLOAD,
                true,
                "no workloads file yet",
                format!(
                    "Propose a workload for {name}: a run long enough to time, with a real input"
                ),
            ),
        ] {
            app.select(sel.clone());
            let a = app.advice(&sel).unwrap();
            assert_eq!(a.action, Some(Action::Propose { workloads }), "{a:?}");
            app.open_menu();
            let Mode::Menu(m) = &app.mode else {
                panic!("{:?}", app.mode)
            };
            assert_eq!(m.items[0].label, label, "{:?}", m.items);
            assert_eq!(m.reason.as_deref(), Some(reason));
            let write = m
                .items
                .iter()
                .position(|i| i.label.starts_with("Write your "))
                .unwrap();
            assert!(write > 0, "{:?}", m.items);
            let it = m.items[0].clone();
            app.mode = Mode::Normal;
            app.choose(&it);
            assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
            assert_eq!(app.focus, Focus::Chat);
            assert_eq!(app.chat.input.text, question);
            assert!(!app.running);
            // Over the chat's draft: greyed, and the Next step is Write.
            let it = find(&app, label).unwrap();
            assert_eq!(
                it.greyed.as_deref(),
                Some("the chat has a draft — send or clear it first")
            );
            assert_ne!(
                app.advice(&sel).unwrap().action,
                Some(Action::Propose { workloads })
            );
            app.chat.input = Default::default();
        }
        // A features draft of the person's is kept: greyed, saying so, and
        // the chat is told (it refuses the proposal itself).
        app.select(Selection::Features);
        app.features_draft = Some(crate::app::features_edit::Draft {
            kind: DraftKind::Features,
            tmp: "/nonexistent".into(),
            file: "/nonexistent/features.toml".into(),
            expect: "none".into(),
            origin: String::new(),
            before: String::new(),
            error_line: None,
            started: None,
            from_proposal: false,
        });
        let it = find(&app, FEATURES).unwrap();
        assert_eq!(
            it.greyed.as_deref(),
            Some("a draft of yours is kept — save or discard it first")
        );
        app.sync_drafts_kept();
        assert_eq!(app.chat.drafts_kept, [true, false]);
        app.features_draft = None;
    }

    /// A proposal waiting and no file: the Next step says to read it, and
    /// Write is the item it recommends.
    #[test]
    fn a_waiting_proposal_is_the_next_step() {
        let mut app = app_of_without_features("targets/zopfli", "propose-next");
        with_chat(&mut app);
        for kind in [DraftKind::Features, DraftKind::Workloads] {
            let path = kind.proposed_path(&app.config).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "schema_version = 1\n").unwrap();
            assert!(app.has_proposal(kind));
        }
        let a = app.advice(&Selection::Features).unwrap();
        assert_eq!(
            a.line(),
            "Next: Read the chat's proposal — Enter, then Write your features file"
        );
        assert_eq!(a.action, Some(Action::EditFeatures));
        let a = app.advice(&Selection::Speed).unwrap();
        assert_eq!(
            a.line(),
            "Next: Read the chat's proposal — Enter, then Write your workloads file"
        );
        assert_eq!(a.action, Some(Action::EditWorkloads));
        // The chat's written proposal is said, with what opens it.
        app.chat_proposal_written(false);
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|n| n.text.contains("Write your features file opens it")),
            "{:?}",
            app.notice.as_ref().map(|n| &n.text)
        );
    }

    /// A proposal as harness-mcp writes it: its header, then the chat's
    /// text (two scenarios in two features).
    const PROPOSAL: &str =
        "# The chat's proposal for your features file, written by m on 2026-10-20.\n\n\
schema_version = 1\n\n[[feature]]\nid = \"gzip\"\nname = \"Compress to gzip\"\n\n\
[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\n\
# Compresses English text.\n[[scenario]]\nfeature = \"gzip\"\nid = \"text\"\n\
args = [\"-c\", \"{input}\"]\ninput = \"sample:text\"\n\n\
# The help.\n[[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n";

    fn ok() -> std::io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(0))
    }

    fn write_proposal(app: &App, kind: DraftKind, text: &str) -> std::path::PathBuf {
        let path = kind.proposed_path(&app.config).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The editor dialog, then its Open: the draft's file.
    fn open(app: &mut App, from_proposal: bool) -> (String, Vec<String>, std::path::PathBuf) {
        if from_proposal {
            app.start_edit_from_proposal(DraftKind::Features);
        } else {
            app.start_features_edit();
        }
        let confirm = match std::mem::replace(&mut app.mode, Mode::Normal) {
            Mode::Dialog(c) => *c,
            other => panic!(
                "the editor dialog: {other:?} ({:?})",
                app.notice.as_ref().map(|n| &n.text)
            ),
        };
        let (title, body) = (confirm.title.clone(), confirm.body.clone());
        match app.close_features_dialog(confirm.purpose, crate::dialog::Choice::Run) {
            crate::app::Command::EditFeatures { file, .. } => (title, body, file),
            other => panic!("{other:?}"),
        }
    }

    /// No file and a proposal: Write opens the proposal as the draft, the
    /// dialog says so; left as it is, it is offered for saving; once saved
    /// the proposal goes.
    #[test]
    fn write_opens_the_proposal_as_the_draft_and_the_save_removes_it() {
        let mut app = app_of_without_features("targets/zopfli", "propose-open");
        let path = write_proposal(&app, DraftKind::Features, PROPOSAL);
        let name = crate::app::features_edit::editor_name(&app.features_editor());
        let (title, body, file) = open(&mut app, false);
        assert_eq!(title, format!("Open the chat's proposal in {name}?"));
        assert_eq!(
            body[0],
            format!(
                "The chat's proposal, 2 scenarios in 2 features, opens in {name}: read it, change \
                 what you like, save."
            )
        );
        assert!(
            body[1].contains("is written only when you save"),
            "{body:?}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), PROPOSAL);
        let d = app.features_draft.as_ref().unwrap();
        assert!(d.from_proposal);
        assert_eq!(d.expect, "none");
        // Back from the editor with it as it was: the Save dialog, not "No
        // change".
        app.features_draft.as_mut().unwrap().started =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
        app.features_edited(ok());
        let Mode::Dialog(c) = &app.mode else {
            panic!("the save dialog: {:?}", app.mode)
        };
        let crate::app::Purpose::Act(p) = &c.purpose else {
            panic!("{:?}", c.purpose)
        };
        assert_eq!(p.act, crate::app::Act::SaveFeatures);
        assert_eq!(p.stdin.as_deref(), Some(PROPOSAL));
        app.mode = Mode::Normal;
        // Saved: the draft and the proposal go.
        assert!(app
            .features_save_ended(DraftKind::Features, true, PROPOSAL)
            .is_some());
        assert!(app.features_draft.is_none());
        assert!(!path.exists(), "the proposal is removed once saved");
    }

    /// A proposal rewritten by the chat while the person edited stays
    /// after their save; a failed save keeps both.
    #[test]
    fn a_newer_proposal_is_kept_after_the_save() {
        let mut app = app_of_without_features("targets/zopfli", "propose-newer");
        let path = write_proposal(&app, DraftKind::Features, PROPOSAL);
        let (_, _, file) = open(&mut app, false);
        let mine = PROPOSAL.replace("Show the help", "Help");
        std::fs::write(&file, &mine).unwrap();
        assert!(app
            .features_save_ended(DraftKind::Features, false, &mine)
            .is_none());
        assert!(path.exists());
        std::fs::write(&path, PROPOSAL.replace("gzip", "zip")).unwrap();
        assert!(app
            .features_save_ended(DraftKind::Features, true, &mine)
            .is_some());
        assert!(path.exists(), "a newer proposal stays");
    }

    /// With the file there: Write opens the file, and the heading's menu
    /// offers Edit from the chat's proposal…, which opens the proposal
    /// against the file's digest.
    #[test]
    fn with_a_file_the_proposal_opens_from_its_own_item() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "propose-edit-from");
        app.select(Selection::Features);
        assert!(find(&app, "Edit from the chat's proposal…").is_none());
        write_proposal(&app, DraftKind::Features, PROPOSAL);
        let it = find(&app, "Edit from the chat's proposal…").expect("the item");
        assert_eq!(it.action, Action::EditFromProposal { workloads: false });
        assert!(!it.model);
        // The Next step stays the row's own.
        assert_ne!(
            app.advice(&Selection::Features).unwrap().action,
            Some(Action::EditFeatures)
        );
        let (title, body, file) = open(&mut app, true);
        assert!(title.starts_with("Open the chat's proposal in "), "{title}");
        assert!(body[1].contains("saving replaces"), "{body:?}");
        assert_eq!(std::fs::read_to_string(file).unwrap(), PROPOSAL);
        let d = app.features_draft.as_ref().unwrap();
        let current =
            std::fs::read(harness_core::features::features_path(&app.config.ledger())).unwrap();
        assert_eq!(d.expect, harness_core::hash::bytes_hash(&current));
        // A kept draft: the item is gone (Continue my draft stands).
        assert!(find(&app, "Edit from the chat's proposal…").is_none());
        // Write (Edit) on the file opens the file, not the proposal.
        let tmp = app.features_draft.take().unwrap().tmp;
        let _ = std::fs::remove_dir_all(tmp);
        let (title, _, file) = open(&mut app, false);
        assert!(title.starts_with("Open the features file in "), "{title}");
        assert_eq!(std::fs::read(file).unwrap(), current);
    }

    /// A proposal reached through a link is never read: refused, said.
    #[test]
    fn a_linked_proposal_is_refused() {
        let mut app = app_of_without_features("targets/zopfli", "propose-link");
        let path = DraftKind::Workloads.proposed_path(&app.config).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let elsewhere = app.config.target.join("elsewhere.toml");
        std::fs::write(&elsewhere, "schema_version = 1\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        app.start_edit(DraftKind::Workloads);
        assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|n| n.text.contains("the chat's proposal cannot be read")),
            "{:?}",
            app.notice.as_ref().map(|n| &n.text)
        );
        assert!(app.workloads_draft.is_none());
        // The workloads proposal's words.
        assert_eq!(
            crate::app::features_edit::proposal_words(
                DraftKind::Workloads,
                "schema_version = 1\n[[workload]]\nid = \"a\"\nargs = [\"-h\"]\n",
                "nano"
            ),
            "The chat's proposal, 1 workload, opens in nano: read it, change what you like, save."
        );
    }

    /// The brief holds both files' syntax as rules, what a feature and a
    /// workload are, and that the chat proposes and the person writes; its
    /// two examples are valid files.
    #[test]
    fn the_brief_teaches_both_files() {
        let brief = crate::chat::BRIEF;
        for words in [
            "You PROPOSE, the person writes",
            "harness_features_propose",
            "harness_workloads_propose",
            "A feature is something a person does with the program and can see the result of",
            "three to six scenarios",
            "one wrong input",
            "half a second or more of the C",
            "one sentence per scenario",
            "features.toml, the rules:",
            "workloads.toml, the rules:",
            "^[a-z0-9][a-z0-9-]{0,23}$",
            "sample:text",
            "runs: 5 to 31",
        ] {
            // As read: its lines wrapped.
            let flat = brief.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat.contains(words), "the brief lacks: {words}");
        }
        let section = brief
            .split("features.toml, the rules:")
            .nth(1)
            .expect("the features rules");
        let example = |after: &str| -> String {
            let rest = after.split("Example:\n").nth(1).expect("an example");
            rest.split("\n\n").next().unwrap().to_string()
        };
        harness_core::features::parse(
            &example(section),
            std::path::Path::new("features.toml"),
            "migration",
        )
        .unwrap();
        let section = brief.split("workloads.toml, the rules:").nth(1).unwrap();
        harness_core::perf::workloads::parse(
            &example(section),
            std::path::Path::new("workloads.toml"),
        )
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    }
}
