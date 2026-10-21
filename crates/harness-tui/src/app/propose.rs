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

impl App {
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
}
