//! The chat's fix (docs/FEATURES-DESIGN.md §14.3): on a feature whose
//! scenarios cannot run (any reason of §6.1's table but `not-in-program`),
//! **Fix the features file — ask in chat** types into the chat one line per
//! scenario that cannot run, with the reason in §6.1's words; the chat reads
//! the file (`harness_features_file`) and proposes the corrected whole file
//! (`harness_features_propose`), and nothing is saved until the person
//! saves (§9.6). On the Speed row the same for a workload perf marks
//! `input-unusable`, or whose C perf refuses to measure: **Fix the workloads
//! file — ask in chat**.
//!
//! How the cockpit knows a proposal is a fix: a proposal that exists while
//! the person's own file exists — and something still cannot run — is read
//! as the chat's fix; the Next step then reads "Read the chat's fix — Enter,
//! then Edit from the chat's proposal".

use super::features_edit::DraftKind;
use super::{notice, App, Focus};
use crate::featmap::{FeatureState, ScenarioView};
use crate::menu::{item, Action, Item};
use crate::next::Advice;
use crate::tree::Selection;
use harness_core::features::SkipReason;

/// The features item's words.
pub const FIX_FEATURES: &str = "Fix the features file — ask in chat";
/// The workloads item's words.
pub const FIX_WORKLOADS: &str = "Fix the workloads file — ask in chat";
/// The Next step once the chat's fix is written.
pub const READ_THE_FIX: &str = "Read the chat's fix — Enter, then Edit from the chat's proposal";

/// Why a scenario cannot be a check, in §6.1's words, or `None` when it
/// can (or only its unit is outside the program). The input's own path is
/// never in them: the question reaches a model, and the chat reads the
/// file itself.
fn scenario_reason(s: &ScenarioView, map_current: bool) -> Option<&'static str> {
    if s.unusable.is_some() {
        return Some(SkipReason::InputUnusable.words());
    }
    if let Some(reason) = s.skipped.filter(|r| r.is_c_side()) {
        return Some(reason.words());
    }
    let r = s.record.as_ref().filter(|_| map_current)?;
    let reason = if r.unusable().is_some() {
        SkipReason::InputUnusable
    } else if !r.stable {
        SkipReason::CSideUnstable
    } else if r.end.starts_with("exit ") {
        return None;
    } else if r.end.starts_with("signal ") {
        SkipReason::CSideCrashed
    } else {
        match r.end.as_str() {
            "timed out" => SkipReason::CSideTimedOut,
            "too much output" => SkipReason::CSideOverflow,
            _ => SkipReason::CSideExecFailed,
        }
    };
    Some(reason.words())
}

/// Why a workload cannot be measured, in words without its input's path,
/// from an outcome or a reason token perf records.
fn workload_reason(token: &str) -> Option<&'static str> {
    Some(match token {
        "input-unusable" => "its input file cannot be used",
        "c-unstable" => "the C ends or prints differently from one run to the next",
        "c-crashed" => "the C crashes on it",
        "c-timed-out" => "the C times out on it",
        "output-too-large" => "the C prints more than 64 MiB",
        "c-could-not-start" => "the C could not be started",
        _ => return None,
    })
}

impl App {
    /// The scenarios that cannot run, of feature `id`: `(feature/scenario,
    /// the reason's words)`, in file order — only while the feature's state
    /// says a scenario cannot run (§8.2's F2).
    pub fn cannot_run(&self, id: &str) -> Vec<(String, &'static str)> {
        let current = self.features.map.current();
        self.features
            .features
            .iter()
            .filter(|f| f.id == id && f.state == FeatureState::ScenarioCannotRun)
            .flat_map(|f| {
                f.scenarios.iter().filter_map(move |s| {
                    scenario_reason(s, current).map(|why| (format!("{}/{}", f.id, s.id), why))
                })
            })
            .collect()
    }

    /// The workloads perf cannot measure: `(workload, the reason's words)`,
    /// in file order — a C-alone row perf marked `input-unusable` or the
    /// C's own failure, or an input that cannot be used today.
    pub fn cannot_measure(&self) -> Vec<(String, &'static str)> {
        let speed = &self.speed;
        let mut out: Vec<(String, &'static str)> = Vec::new();
        for (w, _) in &speed.workloads {
            // Today's read of its input first: perf would refuse it now.
            if matches!(
                self.snapshot.perf.inputs.get(w),
                Some(crate::perfread::InputNow::Unusable(_))
            ) {
                out.push((w.clone(), "its input file cannot be used"));
                continue;
            }
            let why = speed
                .c_rows
                .iter()
                .chain(speed.program_rows.iter())
                .filter(|r| r.workload == *w)
                .find_map(|r| {
                    workload_reason(&r.outcome).or_else(|| {
                        r.out_of_date_tokens
                            .contains(&"input-unusable")
                            .then(|| workload_reason("input-unusable"))
                            .flatten()
                    })
                });
            if let Some(why) = why {
                out.push((w.clone(), why));
            }
        }
        out
    }

    /// The question a Fix item types: one line per scenario (or workload)
    /// that cannot run, then what to do — `None` when nothing needs a fix.
    pub fn fix_question(&self, sel: &Selection) -> Option<String> {
        let (noun, verb, file, list) = match sel {
            Selection::Feature(id) => ("Scenario", "be a check", "features", self.cannot_run(id)),
            Selection::Speed => (
                "Workload",
                "be measured",
                "workloads",
                self.cannot_measure(),
            ),
            _ => return None,
        };
        if list.is_empty() {
            return None;
        }
        let mut lines: Vec<String> = list
            .iter()
            .map(|(what, why)| format!("{noun} {what} cannot {verb}: {why}."))
            .collect();
        let them = if list.len() == 1 { "it" } else { "them" };
        if let Some(last) = lines.last_mut() {
            last.push_str(&format!(
                " Change or remove {them}, and propose the corrected {file} file"
            ));
        }
        Some(lines.join("\n"))
    }

    /// Fix the … file — ask in chat: the question typed into the input
    /// (never over a draft), the chat focused; the person reads and sends.
    pub(crate) fn chat_ask_fix(&mut self) {
        if !self.chat.input.is_empty() {
            self.say("the chat has a draft — send or clear it first");
            self.move_focus(Focus::Chat);
            return;
        }
        let Some(text) = self.fix_question(&self.selection.clone()) else {
            self.say("nothing here needs a fix now");
            return;
        };
        self.chat.input.insert(&text);
        self.move_focus(Focus::Chat);
        self.notice = notice(
            "the question is typed into the chat — press Enter to send it; the chat reads your \
             file and writes a corrected proposal you open, change and save",
        );
    }

    fn fix_kind(sel: &Selection) -> Option<DraftKind> {
        match sel {
            Selection::Feature(_) => Some(DraftKind::Features),
            Selection::Speed => Some(DraftKind::Workloads),
            _ => None,
        }
    }

    /// The Fix item, and on a feature row the item that opens the chat's
    /// fix, when something on the row cannot run.
    pub(crate) fn fix_items(&self, sel: &Selection, items: &mut Vec<Item>) {
        let Some(kind) = Self::fix_kind(sel) else {
            return;
        };
        if self.fix_question(sel).is_none() {
            return;
        }
        let workloads = kind == DraftKind::Workloads;
        let label = if workloads {
            FIX_WORKLOADS
        } else {
            FIX_FEATURES
        };
        let mut it = item(label, Action::FixFile { workloads }, None);
        it.model = true;
        it.greyed = self.propose_greyed(kind);
        items.push(it);
        // The Speed row lists it already (its proposal items).
        if !workloads && self.fix_waits(kind) {
            let mut it = item(
                "Edit from the chat's proposal…",
                Action::EditFromProposal { workloads },
                None,
            );
            if self.running {
                it.greyed = Some("a command is running (one at a time)".into());
            }
            items.push(it);
        }
    }

    /// A proposal waits beside the person's own file and no draft of
    /// theirs is kept: read as the chat's fix.
    fn fix_waits(&self, kind: DraftKind) -> bool {
        self.draft(kind).is_none() && self.has_own_file(kind) && self.has_proposal(kind)
    }

    /// The Next step on a row where something cannot run: the chat's fix to
    /// read, when one waits; else Fix, when the chat can be used — `None`
    /// leaves the row's own.
    pub(crate) fn fix_advice(&self, sel: &Selection) -> Option<Advice> {
        let kind = Self::fix_kind(sel)?;
        self.fix_question(sel)?;
        let workloads = kind == DraftKind::Workloads;
        if self.fix_waits(kind) {
            return Some(Advice {
                text: READ_THE_FIX.into(),
                action: Some(Action::EditFromProposal { workloads }),
                reason: None,
            });
        }
        if self.propose_greyed(kind).is_some() {
            return None;
        }
        Some(if workloads {
            Advice {
                text: "a workload cannot be measured — press Enter and choose Fix the workloads \
                       file, or Edit the workloads file yourself"
                    .into(),
                action: Some(Action::FixFile { workloads }),
                reason: None,
            }
        } else {
            Advice {
                text: "a scenario cannot run — press Enter and choose Fix the features file, or \
                       Edit the features file yourself"
                    .into(),
                action: Some(Action::FixFile { workloads }),
                reason: None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::propose::tests::with_chat;
    use crate::app::tests::app_of_path;
    use crate::app::Mode;
    use crate::testutil::{scratch_target_without_features, TmpDir};

    /// One feature whose decode scenario names a file that is not there,
    /// and one whose scenario runs.
    const FEATURES: &str = "schema_version = 1\n\
[[feature]]\nid = \"decode\"\nname = \"Decode a file\"\n\
[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\
[[scenario]]\nfeature = \"decode\"\nid = \"small\"\nargs = [\"-d\", \"{input}\"]\ninput = \"tests/small.lzg\"\n\
[[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n";

    const QUESTION: &str = "Scenario decode/small cannot be a check: its input file cannot be \
used. Change or remove it, and propose the corrected features file";

    fn target(tag: &str) -> (TmpDir, App) {
        let root = scratch_target_without_features("targets/zopfli", &format!("fix-{tag}"));
        let dir = root.join("migration/features");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("features.toml"), FEATURES).unwrap();
        harness_core::adopt::testing::adopt(&root);
        let app = app_of_path(&root);
        (TmpDir(root), app)
    }

    fn find(app: &App, label: &str) -> Option<Item> {
        app.menu_items().into_iter().find(|i| i.label == label)
    }

    /// A scenario whose input file is not there cannot run (§14.1): its
    /// feature says so from today's read, in the scenario's own words, and
    /// the other feature is untouched.
    #[test]
    fn a_missing_input_marks_its_scenario_from_todays_read() {
        let (_dir, app) = target("read");
        let f = app.features.feature("decode").unwrap();
        assert_eq!(f.state, FeatureState::ScenarioCannotRun);
        assert_eq!(f.cannot_run, 1);
        assert_eq!(
            f.scenarios[0].unusable.as_deref(),
            Some(
                "its input tests/small.lzg is not here — put the file back or change the \
                 scenario"
            )
        );
        assert_eq!(
            f.scenarios[0].input.as_deref(),
            Some("the file tests/small.lzg")
        );
        assert_ne!(
            app.features.feature("help").unwrap().state,
            FeatureState::ScenarioCannotRun
        );
        assert_eq!(
            app.cannot_run("decode"),
            [("decode/small".to_string(), "its input file cannot be used")]
        );
        assert!(app.cannot_run("help").is_empty());
    }

    /// With the chat off: the Fix item is listed under the model separator,
    /// greyed with the reason, never recommended; on a feature that can run
    /// it is not listed at all.
    #[test]
    fn with_the_chat_off_fix_is_greyed_and_never_recommended() {
        let (_dir, mut app) = target("off");
        let sel = Selection::Feature("decode".into());
        app.select(sel.clone());
        let it = find(&app, FIX_FEATURES).expect("the item");
        assert!(it.model, "{it:?}");
        assert_eq!(it.greyed.as_deref(), Some("the chat is off (--no-chat)"));
        let a = app.advice(&sel).unwrap();
        assert_eq!(a.action, Some(Action::EditFeatures), "{a:?}");
        app.select(Selection::Feature("help".into()));
        assert!(find(&app, FIX_FEATURES).is_none());
        app.select(Selection::Features);
        assert!(find(&app, FIX_FEATURES).is_none());
    }

    /// With the chat on: Fix is the Next step and the menu's first item,
    /// before Edit the features file; chosen, it types one line per
    /// scenario that cannot run (nothing sent, nothing run); over the
    /// chat's draft it is greyed. Once a proposal waits beside the file,
    /// the Next step is to read the chat's fix.
    #[test]
    fn with_the_chat_on_fix_is_recommended_types_its_question_and_leads_to_the_fix() {
        let (dir, mut app) = target("on");
        with_chat(&mut app);
        let sel = Selection::Feature("decode".into());
        app.select(sel.clone());
        let a = app.advice(&sel).unwrap();
        assert_eq!(
            a.action,
            Some(Action::FixFile { workloads: false }),
            "{a:?}"
        );
        app.open_menu();
        let Mode::Menu(m) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert_eq!(m.items[0].label, FIX_FEATURES, "{:?}", m.items);
        assert_eq!(m.reason.as_deref(), Some("a scenario cannot run"));
        let edit = m
            .items
            .iter()
            .position(|i| i.label.starts_with("Edit the features file"))
            .unwrap();
        assert!(edit > 0, "{:?}", m.items);
        let it = m.items[0].clone();
        app.mode = Mode::Normal;
        app.choose(&it);
        assert_eq!(app.focus, Focus::Chat);
        assert_eq!(app.chat.input.text, QUESTION);
        assert!(!app.running);
        assert_eq!(
            find(&app, FIX_FEATURES).unwrap().greyed.as_deref(),
            Some("the chat has a draft — send or clear it first")
        );
        app.chat.input = Default::default();

        // The chat's corrected proposal is written beside the file.
        let proposed = dir.0.join("migration/features/features.proposed.toml");
        std::fs::write(
            &proposed,
            FEATURES.replace("tests/small.lzg", "sample:rand"),
        )
        .unwrap();
        let a = app.advice(&sel).unwrap();
        assert_eq!(a.text, READ_THE_FIX);
        assert_eq!(
            a.action,
            Some(Action::EditFromProposal { workloads: false })
        );
        let it = find(&app, "Edit from the chat's proposal…").expect("listed on the feature");
        assert!(it.greyed.is_none(), "{it:?}");
        app.open_menu();
        let Mode::Menu(m) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert_eq!(m.items[0].label, "Edit from the chat's proposal…");
    }

    /// Several scenarios: one line each, the reason in §6.1's words — from
    /// today's read and from a current map that found one unstable.
    #[test]
    fn the_question_has_one_line_per_scenario_that_cannot_run() {
        let (dir, _) = target("lines");
        let text = format!(
            "{FEATURES}[[scenario]]\nfeature = \"decode\"\nid = \"clock\"\nargs = [\"-t\"]\n"
        );
        std::fs::write(dir.0.join("migration/features/features.toml"), text).unwrap();
        // A map with today's inputs: the clock scenario unstable, the
        // decode one not run for its input.
        let read = crate::load::read(&dir.0).unwrap();
        let record = |s: &str| harness_core::features::ScenarioRecord {
            feature: "decode".into(),
            scenario: s.into(),
            end: "exit 0".into(),
            stdout_bytes: 1,
            stderr_bytes: 0,
            stderr_head: String::new(),
            stable: false,
            probe_agrees: true,
            noted: "complete".into(),
            reason: None,
            functions: Vec::new(),
        };
        let map = harness_core::features::FeatureMap {
            schema: harness_core::features::MAP_SCHEMA_NAME.into(),
            schema_version: harness_core::features::MAP_SCHEMA_VERSION,
            inputs: read.map_now.expect("today's inputs"),
            unwatched: Vec::new(),
            unwatched_reasons: Vec::new(),
            scenarios: vec![
                harness_core::features::ScenarioRecord::input_unusable(
                    "decode",
                    "small",
                    harness_core::features::InputUnusable::Missing,
                ),
                record("clock"),
            ],
        };
        std::fs::write(
            harness_core::features::map_path(&harness_core::ledger::Ledger::new(&dir.0)),
            map.to_bytes().unwrap(),
        )
        .unwrap();
        let app = app_of_path(&dir.0);
        assert!(app.features.map.current(), "{:?}", app.features.map);
        assert_eq!(
            app.fix_question(&Selection::Feature("decode".into()))
                .as_deref(),
            Some(
                "Scenario decode/small cannot be a check: its input file cannot be used.\n\
                 Scenario decode/clock cannot be a check: the C program's output differs \
                 between runs. Change or remove them, and propose the corrected features file"
            )
        );
        // The file put back: the map's record still says why, until the
        // map is made again (it is out of date then).
        std::fs::create_dir_all(dir.0.join("tests")).unwrap();
        std::fs::write(dir.0.join("tests/small.lzg"), b"LZG").unwrap();
        let app = app_of_path(&dir.0);
        assert!(!app.features.map.current());
        let f = app.features.feature("decode").unwrap();
        assert_eq!(
            f.scenarios[0].unusable.as_deref(),
            Some(
                "its input tests/small.lzg is not here — put the file back or change the \
                 scenario"
            ),
            "from the last map"
        );
    }

    /// The Speed row: a workload whose input cannot be used offers Fix the
    /// workloads file, its question in the same shape.
    #[test]
    fn a_workload_whose_input_cannot_be_used_offers_the_workloads_fix() {
        let (dir, _) = target("speed");
        std::fs::create_dir_all(dir.0.join("migration/perf")).unwrap();
        std::fs::write(
            dir.0.join("migration/perf/workloads.toml"),
            "schema_version = 1\n[[workload]]\nid = \"big\"\nargs = [\"{input}\"]\n\
             input = \"bench/gone.txt\"\n",
        )
        .unwrap();
        let mut app = app_of_path(&dir.0);
        with_chat(&mut app);
        app.select(Selection::Speed);
        assert_eq!(
            app.fix_question(&Selection::Speed).as_deref(),
            Some(
                "Workload big cannot be measured: its input file cannot be used. Change or \
                 remove it, and propose the corrected workloads file"
            )
        );
        let it = find(&app, FIX_WORKLOADS).expect("the item");
        assert!(it.model && it.greyed.is_none(), "{it:?}");
        assert_eq!(
            app.advice(&Selection::Speed).unwrap().action,
            Some(Action::FixFile { workloads: true })
        );
    }
}
