//! The Next step (docs/reviews/2026-10-18-cockpit-usability-critique.md,
//! pass 1): every View says, in one line, what to do now, and the Enter
//! menu opens on that action, first and highlighted, with the same line as
//! its reason. One rule per node, read from the state the View shows; never
//! model work started on its own — a model item is only recommended, and
//! still goes through the chat and its dialog.

use crate::app::{Act, App};
use crate::featmap;
use crate::files::{Cause, FileState, UnitState};
use crate::menu::Action;
use crate::tree::Selection;

/// What to do now, in words, and the menu item that does it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// The Next step, a sentence (the View's "Next: …" line).
    pub text: String,
    /// The item the menu puts first, when one does it.
    pub action: Option<Action>,
    /// Why that item, for the menu's "recommended:" line, when the text's
    /// lead would only repeat the item (the Project view's).
    pub reason: Option<String>,
}

impl Advice {
    /// The View's line: "Next: " and the sentence, capitalised — not
    /// when it starts with a name ("u-lib's Rust…").
    pub fn line(&self) -> String {
        let first = self.text.split(' ').next().unwrap_or_default();
        let word = first.chars().all(|c| c.is_ascii_lowercase() || c == ',');
        let mut chars = self.text.chars();
        let text = match chars.next() {
            Some(c) if word => c.to_uppercase().chain(chars).collect(),
            _ => self.text.clone(),
        };
        format!("Next: {text}")
    }
}

fn advice(text: impl Into<String>, action: Option<Action>) -> Advice {
    Advice {
        text: text.into(),
        action,
        reason: None,
    }
}

/// The act that answers a cause, if a menu item does.
pub fn cause_action(cause: &Cause) -> Option<Action> {
    match cause {
        Cause::Contradiction
        | Cause::PromotionInterrupted { .. }
        | Cause::StaleVerdict
        | Cause::CalleeChanged
        | Cause::MadeElsewhere { .. } => Some(Action::Act(Act::Verify)),
        Cause::SourceChanged => Some(Action::Act(Act::Plan)),
        Cause::ChangedOutside | Cause::NoEvidence => None,
    }
}

pub use crate::files::acceptable;

impl App {
    /// The Next step of a node (the selection, usually): never a chat item
    /// while the chat is unavailable (docs/CHAT-PANE-DESIGN.md §1.6).
    pub fn advice(&self, sel: &Selection) -> Option<Advice> {
        self.advice_of(sel).map(|a| self.unless_chat_down(a))
    }

    fn advice_of(&self, sel: &Selection) -> Option<Advice> {
        match sel {
            Selection::Project | Selection::Units => self.next_step(),
            Selection::Dir(_) => None,
            Selection::Unit(_) | Selection::Crate(_) => {
                self.owning_unit(sel).and_then(|u| self.unit_advice(u))
            }
            Selection::Attempt(_, id) => self
                .owning_unit(sel)
                .and_then(|u| self.attempt_advice(u, id)),
            Selection::File(path) | Selection::Function(path, _) => {
                match self.files.file(path).map(|f| &f.state) {
                    Some(FileState::Changed | FileState::New | FileState::Missing) => Some(advice(
                        "the scan does not reflect this file — Scan the project",
                        Some(Action::Act(Act::Scan)),
                    )),
                    Some(FileState::NotInPlan) => Some(advice(
                        "this file is in no unit yet — Refresh the plan",
                        Some(Action::Act(Act::Plan)),
                    )),
                    Some(FileState::Owned(_)) => {
                        self.owning_unit(sel).and_then(|u| self.unit_advice(u))
                    }
                    _ => None,
                }
            }
            Selection::Features => self.features_advice(),
            Selection::Feature(id) => self.fix_advice(sel).or_else(|| self.feature_advice(id)),
            Selection::Speed => self.fix_advice(sel).or_else(|| self.speed_advice()),
            Selection::MapProject
            | Selection::Program(_)
            | Selection::Tool(_)
            | Selection::ProjectHome => self.project_row_advice(sel),
        }
    }

    /// A unit's Next step, from its state.
    pub fn unit_advice(&self, u: usize) -> Option<Advice> {
        let unit = self.snapshot.units.get(u)?;
        let state = &self.files.units.get(u)?.state;
        let id = unit.unit.id.as_str();
        Some(match state {
            UnitState::Blocked => advice("blocked in the plan — nothing to run on it", None),
            UnitState::Attention(cause) => advice(
                cause.words(&self.snapshot.ledger_rel()),
                cause_action(cause),
            ),
            UnitState::Failing => advice(
                "its last check was red — Show the checks to see why, then ask the chat to \
                 migrate it again",
                Some(Action::Migrate),
            ),
            // Why is the line above it on the card.
            UnitState::StaysC(_) => advice("nothing to do on it in this version", None),
            UnitState::Waiting => {
                // An attempt of this session's: Resume; one the command line
                // left: its answer file and the command again.
                if let Some(aw) = self.awaiting.iter().find(|aw| {
                    unit.attempts
                        .iter()
                        .any(|a| aw.attempt.as_ref() == Some(&a.record.id))
                }) {
                    advice(
                        crate::app::waiting_words(
                            &crate::app::short_file(&aw.path),
                            "open its attempt and choose Resume",
                        ),
                        aw.attempt.clone().map(Action::ChooseAttempt),
                    )
                } else {
                    match self.pending_answer(unit) {
                        // The command line's try: its answer is the
                        // person's (the chat answers only its own tries).
                        Some(path) => advice(
                            format!(
                                "waiting for your answer: {} — write the reply there, then run \
                                 the same migrate command again",
                                crate::app::short_file(&path)
                            ),
                            None,
                        ),
                        None => advice(
                            "an attempt has not finished — it may still be running elsewhere",
                            None,
                        ),
                    }
                }
            }
            UnitState::RustGreen
            | UnitState::RustTried
            | UnitState::TestProgramReady
            | UnitState::TestProgramStale
            | UnitState::NoTestProgram => {
                if let Some(a) = acceptable(unit) {
                    advice(
                        format!(
                            "try {} passed every check — open it, look over its code, then \
                             Accept it",
                            crate::app::short_id(a)
                        ),
                        Some(Action::ChooseAttempt(id.to_string())),
                    )
                } else if let Some(aw) = self.awaiting.iter().find(|aw| {
                    unit.attempts
                        .iter()
                        .any(|a| aw.attempt.as_ref() == Some(&a.record.id))
                }) {
                    advice(
                        format!(
                            "attempt {} waits for an answer to its hand-off — open it, then \
                             Resume",
                            aw.attempt
                                .as_deref()
                                .map(crate::app::short_id)
                                .unwrap_or_default()
                        ),
                        None,
                    )
                } else if !unit.unit.depends_on.is_empty() {
                    // The judge tests each unit alone (the guide's Part 7):
                    // never recommended, the Migrate item stays listed.
                    advice(
                        format!(
                            "it calls other units ({}), so in this version it stays C — the judge \
                             tests each unit alone",
                            unit.unit.depends_on.join(", ")
                        ),
                        None,
                    )
                } else if state.needs_test_program() {
                    // The stage before its Rust (§9.4): never Migrate, which
                    // the CLI would refuse.
                    advice(
                        if *state == UnitState::TestProgramStale {
                            "Make the test program again — Enter, then ask in chat"
                        } else {
                            "Make the test program — Enter, then ask in chat"
                        },
                        Some(Action::GenDriver),
                    )
                } else if matches!(state, UnitState::RustTried) {
                    advice(
                        "tried, none accepted — ask the chat to migrate it again",
                        Some(Action::Migrate),
                    )
                } else {
                    advice(
                        "no Rust yet — ask the chat to migrate it (a model writes the Rust, the \
                         judge checks it, you accept it)",
                        Some(Action::Migrate),
                    )
                }
            }
            UnitState::Migrated(_) | UnitState::OriginUnknown => {
                if self.features.recheck.iter().any(|r| r == id) {
                    advice(
                        "your features are not checked on it yet — Re-check with the judge",
                        Some(Action::Act(Act::Verify)),
                    )
                } else if let Some(w) = self
                    .speed
                    .comparable(&crate::speed::SideKey::Unit(id.to_string()))
                    .into_iter()
                    .next()
                {
                    advice(
                        format!(
                            "with its Rust the program behaves differently on {w} — Compare the \
                             outputs, then change its Rust as said below"
                        ),
                        Some(Action::CompareOutputs(
                            crate::speed::SideKey::Unit(id.to_string()),
                            w,
                        )),
                    )
                } else if let Some(speed) =
                    Some(self.speed.advice(unit, !self.config.providers.is_empty()))
                        .filter(|s| s.next.is_some())
                {
                    advice(
                        if speed.differences.is_empty() {
                            "its Rust is slower than the C — if speed matters here, change it as \
                             said below"
                        } else {
                            "with its Rust the program behaves differently on a workload — change \
                             its Rust as said below"
                        },
                        None,
                    )
                } else if self.speed.unit(id).is_none()
                    && !matches!(self.speed.group, crate::speed::Group::NoFile)
                {
                    advice(
                        "in Rust and checked — Measure this unit's speed to compare it with the C",
                        Some(Action::Act(Act::Measure)),
                    )
                } else {
                    advice("in Rust and checked — nothing to do", None)
                }
            }
            UnitState::Other(_) => return None,
        })
    }

    /// The answer file a unit's attempt waits on, when a run of the command
    /// line left one: the newest request under its traces with no
    /// response yet (its `.response.json`, not yet written).
    pub fn pending_answer(&self, unit: &crate::model::UnitView) -> Option<std::path::PathBuf> {
        let dir = self.snapshot.ledger().unit_dir(&unit.unit.id);
        let mut found: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
        for sub in ["traces", "traces/chat"] {
            let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
                continue;
            };
            for e in entries.flatten().take(10_000) {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(key) = name.strip_suffix(".request.json") else {
                    continue;
                };
                let response = dir.join(sub).join(format!("{key}.response.json"));
                if response.exists() {
                    continue;
                }
                let when = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                found.push((when, response));
            }
        }
        found.into_iter().max().map(|(_, p)| p)
    }

    /// An attempt's Next step.
    fn attempt_advice(&self, u: usize, id: &str) -> Option<Advice> {
        let unit = self.snapshot.units.get(u)?;
        let a = unit.attempt(id)?;
        let r = &a.record;
        if let Some(aw) = self
            .awaiting
            .iter()
            .find(|aw| aw.attempt.as_deref() == Some(id))
        {
            return Some(advice(
                crate::app::waiting_words(&aw.path.display().to_string(), "choose Resume"),
                Some(Action::Act(Act::Resume)),
            ));
        }
        if r.promoted {
            return Some(advice(
                "this is the Rust the unit uses — nothing to do",
                None,
            ));
        }
        Some(match r.outcome.as_str() {
            "green" => advice(
                format!(
                    "it passed every check (GREEN) — look over its code below, then Accept it into {}",
                    unit.unit.id
                ),
                Some(Action::Act(Act::Accept)),
            ),
            "in-progress" => advice("it has not finished", None),
            _ if a.is_human() => advice(
                "your hand edit did not pass — Show the checks, then Hand edit again",
                Some(Action::HandEdit),
            ),
            _ => advice(
                "it did not pass — Show the checks, then Modify it with a note, or Retry",
                Some(Action::Modify),
            ),
        })
    }

    /// The Features group's Next step (docs/FEATURES-DESIGN.md §8.5).
    fn features_advice(&self) -> Option<Advice> {
        let model = &self.features;
        if self.snapshot.facts.is_none() {
            return Some(advice(
                "nothing is scanned yet — Scan the project first (Enter on its top row)",
                None,
            ));
        }
        if model.no_single_main {
            return None;
        }
        Some(match &model.group {
            featmap::Group::NoFile => self
                .proposal_advice(crate::app::features_edit::DraftKind::Features)
                .unwrap_or_else(|| {
                    advice(
                        "no features yet — press Enter and choose Write your features file",
                        Some(Action::EditFeatures),
                    )
                }),
            featmap::Group::Invalid(_) => advice(
                "the features file has an error — press Enter and choose Edit the features file",
                Some(Action::EditFeatures),
            ),
            featmap::Group::Valid if model.features.is_empty() => advice(
                "no features in the file yet — press Enter and choose Edit the features file",
                Some(Action::EditFeatures),
            ),
            featmap::Group::Valid if !model.map.current() => advice(
                match model.map {
                    featmap::MapStatus::OutOfDate(_) => {
                        "the map is out of date — Map the features again"
                    }
                    _ => "not mapped yet — Map the features, so each says which units it runs",
                },
                Some(Action::Act(Act::MapFeatures)),
            ),
            featmap::Group::Valid => match model.recheck.first() {
                Some(u) => advice(
                    format!("Re-check {u} — your features are not checked on it"),
                    None,
                ),
                None => advice(
                    "nothing to do — choose a feature to see which units it runs",
                    None,
                ),
            },
        })
    }

    /// A feature's Next step, from its state (§8.2).
    fn feature_advice(&self, id: &str) -> Option<Advice> {
        use featmap::FeatureState as S;
        let f = self.features.feature(id)?;
        Some(match &f.state {
            S::Failing => advice("open the failing unit below and Show its checks", None),
            S::ScenarioCannotRun => advice(
                "change or remove the scenario that cannot run (Edit the features file)",
                Some(Action::EditFeatures),
            ),
            S::NeedsRecheck => advice(
                format!(
                    "Re-check {} — your features are not checked on {} yet",
                    f.recheck.join(", "),
                    if f.recheck.len() == 1 { "it" } else { "them" }
                ),
                None,
            ),
            S::NotMapped => advice(
                "not mapped yet — Map the features",
                Some(Action::Act(Act::MapFeatures)),
            ),
            S::MapOutOfDate => advice(
                "the map is out of date — Map the features again",
                Some(Action::Act(Act::MapFeatures)),
            ),
            S::MapIncomplete => advice("see the scenario flagged below", None),
            S::ReachesNoUnit => advice(
                "nothing to do — or add a scenario that reaches a unit",
                None,
            ),
            S::AllMigrated | S::HoldsSoFar { .. } => advice(
                "nothing to do — it keeps being checked as more units move to Rust",
                None,
            ),
            S::AllC => advice(
                "migrate one of its units below; every Re-check then checks this feature",
                None,
            ),
            S::SeeUnits => advice("see its units below", None),
        })
    }

    /// Speed's Next step (docs/PERF-DESIGN.md §3.11).
    fn speed_advice(&self) -> Option<Advice> {
        use crate::speed::{Group, SideKey};
        let model = &self.speed;
        Some(match &model.group {
            Group::NoFile => self
                .proposal_advice(crate::app::features_edit::DraftKind::Workloads)
                .unwrap_or_else(|| {
                    advice(
                        "no workloads file yet — press Enter and choose Write your workloads file",
                        Some(Action::EditWorkloads),
                    )
                }),
            Group::NoWorkload => advice(
                "no workload yet — press Enter and choose Edit the workloads file",
                Some(Action::EditWorkloads),
            ),
            Group::FileError(_) => advice(
                "the workloads file has an error — press Enter and choose Edit the workloads file",
                Some(Action::EditWorkloads),
            ),
            Group::NotYetRun => advice(
                "nothing measured yet — press Enter and choose Measure speed",
                Some(Action::Act(Act::Measure)),
            ),
            Group::COnly => advice(
                "only the C is measured: a unit is measured once its Rust is verified",
                None,
            ),
            Group::Units { .. } => {
                if let Some(w) = model.comparable(&SideKey::AsItStands).into_iter().next() {
                    advice(
                        "the program as it stands behaves differently — Compare the outputs",
                        Some(Action::CompareOutputs(SideKey::AsItStands, w)),
                    )
                } else if !model.more_runs(&SideKey::AsItStands).is_empty() {
                    advice(
                        "some rows cannot tell yet — Measure again with 31 runs",
                        Some(Action::Act(Act::MeasureMore)),
                    )
                } else if let Some(u) = model.units.iter().find(|u| {
                    u.rows.iter().any(|r| {
                        r.out_of_date.is_empty()
                            && matches!(r.words.answer, "slower" | "probably-slower")
                    })
                }) {
                    advice(
                        format!(
                            "{}'s Rust is slower than the C — open it under Units to see how to \
                             change it, if speed matters there",
                            u.id
                        ),
                        None,
                    )
                } else {
                    advice("nothing to do — Measure speed again after a change", None)
                }
            }
        })
    }
}
