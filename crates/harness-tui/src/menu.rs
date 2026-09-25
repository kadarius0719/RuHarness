//! `Enter`: the action menu (docs/COCKPIT-WRAPPER-DESIGN.md §4). The items
//! that fit the selected node in its current state: items that do not apply
//! are not listed; items that apply but cannot run now are greyed with their
//! reason; model-backed items sit under a separator. Every command is
//! project-wide or unit-wide — no tree path ever enters an argv — and each
//! item's argv is exactly what its dialog would show (built by
//! [`App::act_argv`], the one argv builder).

use crate::app::{Act, App, Pending};
use crate::files::{FileState, UnitState};
use crate::model::UnitView;
use crate::tree::Selection;

/// What an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open the node: focus moves into the View.
    Open,
    /// Fold a directory (or unfold it).
    Fold,
    /// Re-read the project (`g`).
    Reread,
    /// A writing act on the project or a unit: its dialog.
    Act(Act),
    /// Jump to a unit node.
    OpenUnit(String),
    /// Jump to a unit's attempts, to choose one to accept.
    ChooseAttempt(String),
    /// Show the checks (`v`).
    ShowChecks,
    /// Compare with the promoted attempt (`d`).
    Compare,
    /// Hand edit the crate (`e`).
    HandEdit,
    /// Modify: ask for a note, then its dialog (`m`).
    Modify,
    /// Continue the kept hand edit (`E`).
    ContinueKept,
    /// Discard the kept hand edit: its override dialog, focused on Keep.
    DiscardKept,
    /// Cancel the running command (`x`): its dialog.
    Cancel,
    /// Greyed pointer to chat (not built yet).
    Migrate,
}

/// One menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Its words.
    pub label: String,
    /// What it does.
    pub action: Action,
    /// Its accelerator, shown beside it.
    pub accel: Option<&'static str>,
    /// Why it cannot run now (greyed); `None` = it can.
    pub greyed: Option<String>,
    /// It calls a model (listed under the separator).
    pub model: bool,
    /// The command it spawns (acts only; shown whole in its dialog).
    pub pending: Option<Pending>,
}

fn item(label: impl Into<String>, action: Action, accel: Option<&'static str>) -> Item {
    Item {
        label: label.into(),
        action,
        accel,
        greyed: None,
        model: false,
        pending: None,
    }
}

/// The menu's separator text for model-backed items.
pub const MODEL_SEPARATOR: &str = "Uses a model — can take minutes";

/// The index of the recommended item (focus starts there).
pub fn recommended(items: &[Item], sel: &Selection, next_step: Option<Act>) -> usize {
    let find = |f: &dyn Fn(&Item) -> bool| items.iter().position(f);
    let open = find(&|i: &Item| matches!(i.action, Action::Open | Action::Fold));
    match sel {
        Selection::Project => next_step
            .and_then(|act| find(&|i: &Item| i.action == Action::Act(act) && i.greyed.is_none()))
            .or(open)
            .unwrap_or(0),
        _ => open.unwrap_or(0),
    }
}

impl App {
    /// The unit a node belongs to (itself, its crate or attempts, or the
    /// owned file or function), by index.
    pub fn owning_unit(&self, sel: &Selection) -> Option<usize> {
        let by_id = |id: &str| self.snapshot.units.iter().position(|u| u.unit.id == id);
        match sel {
            Selection::Unit(id) | Selection::Crate(id) | Selection::Attempt(id, _) => by_id(id),
            Selection::File(path) | Selection::Function(path, _) => {
                self.files.file(path).and_then(|f| f.owner)
            }
            _ => None,
        }
    }

    /// Why no writing act can start now (busy, or read-only), if so. The
    /// holder is the one last read ([`App::refresh_holder`]).
    pub fn busy(&self) -> Option<String> {
        if self.running {
            return Some("a command is running (one at a time)".into());
        }
        if self.config.harness.is_none() {
            return Some("no `harness` binary found (PATH, or --harness <path>): read-only".into());
        }
        if let Some(e) = &self.holder_error {
            return Some(format!("the writer lock could not be read ({e})"));
        }
        self.holder
            .as_ref()
            .map(|h| format!("busy: `{}` (checked just now)", h.command))
    }

    /// Why the project's facts cannot feed `plan`/`detect`, if so.
    fn facts_gate(&self) -> Option<String> {
        let Some(state) = &self.snapshot.facts_state else {
            return Some("nothing is scanned yet — scan first".into());
        };
        // The stale paths hold the missing files already (review ENG-6).
        let new = self
            .files
            .files
            .iter()
            .filter(|f| f.state == FileState::New)
            .count();
        let n = state.stale + new;
        (n > 0).then(|| {
            format!(
                "{n} file{} changed or new since the scan — scan first",
                if n == 1 { "" } else { "s" }
            )
        })
    }

    /// An act item: its argv (or why not), greyed while busy.
    fn act_item(
        &self,
        label: String,
        act: Act,
        unit: Option<&UnitView>,
        attempt: Option<&str>,
    ) -> Item {
        let mut it = item(label, Action::Act(act), act.accel());
        it.model = act.model();
        match self.act_argv(act, unit.map(|u| u.unit.id.as_str()), attempt, None) {
            Ok(p) => it.pending = Some(p),
            Err(why) => it.greyed = Some(why),
        }
        if let Some(why) = self.busy() {
            it.greyed = Some(why);
        }
        it
    }

    /// The Re-check item's gate: only code the harness knows (§4.3).
    fn recheck_gate(&self, u: usize) -> Option<String> {
        let unit = &self.snapshot.units[u];
        if unit.crate_dir.is_none() {
            return Some("the unit has no crate yet".into());
        }
        if !unit.report.source_fresh {
            return Some(
                "the C changed since the unit was planned — scan, refresh the plan and review \
                 its diff first"
                    .into(),
            );
        }
        if !self.files.units.get(u).is_some_and(|i| i.known_code) {
            return Some(match unit.report.verdict.state {
                harness_core::status::VerdictState::Present => {
                    "the crate differs from every recorded attempt and from what the oracle last \
                     judged — restore it, or record it with `harness override` (see Help)"
                        .into()
                }
                _ => "the crate matches no recorded attempt and there is no verdict to compare \
                      it with — restore the verdict (git), or record the crate with `harness \
                      override` (see Help)"
                    .into(),
            });
        }
        // Only code the View shows (review ENG-4): an internal function or
        // a header shows C source, not the crate.
        if self.pairs_unit.as_deref() != Some(unit.unit.id.as_str()) || self.pairs_digest.is_none()
        {
            return Some(
                "open the unit (or its crate) first — the cockpit re-checks only code it shows"
                    .into(),
            );
        }
        None
    }

    /// Why an act that replaces or edits the unit crate must wait: the crate
    /// holds code the harness does not know (review SAFE-8, SAFE-9).
    fn unknown_crate(&self, u: usize) -> Option<String> {
        let unit = &self.snapshot.units[u];
        (unit.crate_dir.is_some() && !self.files.units.get(u).is_some_and(|i| i.known_code)).then(
            || {
                format!(
                    "{}'s crate holds code the harness does not know — record it with `harness \
                     override` or restore it first (see Help)",
                    unit.unit.id
                )
            },
        )
    }

    /// The unit's own items (unit, crate, owned file or function), labelled
    /// with the unit when `labelled`.
    fn unit_items(&self, u: usize, labelled: bool, items: &mut Vec<Item>) {
        let unit = &self.snapshot.units[u];
        let id = unit.unit.id.as_str();
        let of = |text: &str| {
            if labelled {
                format!("{text} ({id})")
            } else {
                text.to_string()
            }
        };
        if unit.verdict.as_ref().is_some_and(|v| !v.checks.is_empty()) {
            items.push(item(of("Show the checks"), Action::ShowChecks, Some("v")));
        }
        let mut recheck = self.act_item(
            of("Re-check with the oracle"),
            Act::Verify,
            Some(unit),
            None,
        );
        if recheck.greyed.is_none() {
            recheck.greyed = self.recheck_gate(u);
        }
        items.push(recheck);
        let acceptable: Vec<&str> = unit
            .attempts
            .iter()
            .filter(|a| {
                a.record.outcome == "green"
                    && a.last_result() == "green"
                    && a.bound
                    && !a.record.promoted
            })
            .map(|a| a.record.id.as_str())
            .collect();
        // Accept is confirmed on the attempt, where its code is shown — a
        // unit's view shows the unit crate (review ENG-5): the item opens it.
        match acceptable.as_slice() {
            [] => {}
            [one] => items.push(item(
                format!(
                    "Accept {} into {id} (opens it first)",
                    crate::app::short_id(one)
                ),
                Action::ChooseAttempt(id.to_string()),
                None,
            )),
            _ => items.push(item(
                of("Choose an attempt to accept…"),
                Action::ChooseAttempt(id.to_string()),
                None,
            )),
        }
    }

    /// The menu of the selection (see the module docs).
    pub fn menu_items(&self) -> Vec<Item> {
        let sel = &self.selection;
        let mut items = Vec::new();
        // Every node opens into the View (§4.2 "any | Open"); a directory
        // and the units group also fold.
        items.push(item("Open", Action::Open, None));
        if matches!(sel, Selection::Dir(_) | Selection::Units) {
            items.push(item(
                if self.expansion.is_open(sel) {
                    "Fold"
                } else {
                    "Unfold"
                },
                Action::Fold,
                None,
            ));
        }
        items.push(item("Re-read the project", Action::Reread, Some("g")));
        let file_state = match sel {
            Selection::File(p) | Selection::Function(p, _) => {
                self.files.file(p).map(|f| f.state.clone())
            }
            _ => None,
        };
        let project = *sel == Selection::Project;
        let is_file = matches!(sel, Selection::File(_));
        // Scan: the project, and a file the scan does not reflect.
        if project
            || (is_file
                && matches!(
                    file_state,
                    Some(FileState::Missing | FileState::Changed | FileState::New)
                ))
        {
            items.push(self.act_item("Scan the project".into(), Act::Scan, None, None));
        }
        let c_changed = self
            .owning_unit(sel)
            .is_some_and(|u| !self.snapshot.units[u].report.source_fresh);
        if project || (is_file && file_state == Some(FileState::NotInPlan)) || c_changed {
            let mut it = self.act_item("Refresh the plan".into(), Act::Plan, None, None);
            if it.greyed.is_none() {
                it.greyed = self.facts_gate();
            }
            items.push(it);
        }
        if project || is_file {
            let mut it = self.act_item(
                "Find hazards (run the detectors)".into(),
                Act::Detect,
                None,
                None,
            );
            if it.greyed.is_none() {
                it.greyed = self.facts_gate();
            }
            items.push(it);
        }
        let owner = self.owning_unit(sel);
        match sel {
            Selection::File(_) | Selection::Function(..) => {
                if let Some(u) = owner {
                    let id = self.snapshot.units[u].unit.id.clone();
                    items.push(item(format!("Open unit {id}"), Action::OpenUnit(id), None));
                    self.unit_items(u, true, &mut items);
                }
            }
            Selection::Unit(_) | Selection::Crate(_) => {
                if let Some(u) = owner {
                    self.unit_items(u, false, &mut items);
                }
            }
            _ => {}
        }
        if let (Selection::Crate(_) | Selection::Attempt(..), Some(u)) = (sel, owner) {
            let mut it = item("Hand edit", Action::HandEdit, Some("e"));
            if let Err(why) = self.hand_edit_target() {
                it.greyed = Some(why);
            } else if matches!(sel, Selection::Crate(_)) {
                // Editing unknown code would record all of it as a human's.
                it.greyed = self.unknown_crate(u);
            }
            items.push(it);
        }
        if let (Selection::Attempt(_, id), Some(u)) = (sel, owner) {
            self.attempt_items(u, id, &mut items);
        }
        // The kept hand edit.
        if let Some(k) = self.kept_edits.last() {
            let here = match sel {
                Selection::Project => true,
                Selection::Unit(id) | Selection::Crate(id) | Selection::Attempt(id, _) => {
                    *id == k.unit
                }
                _ => false,
            };
            if here {
                let mut cont = item(
                    format!("Continue my kept hand edit ({})", k.unit),
                    Action::ContinueKept,
                    Some("E"),
                );
                cont.greyed = self.busy();
                items.push(cont);
                // Never while a command runs: it may be the override
                // recording this very edit (review SAFE-7).
                let mut discard = item("Discard my kept hand edit", Action::DiscardKept, None);
                discard.greyed = self
                    .running
                    .then(|| "a command is running (one at a time)".to_string());
                items.push(discard);
            }
        }
        // Model work that happens in chat.
        let migratable = |u: &UnitView| {
            matches!(
                crate::files::unit_state(u),
                UnitState::Planned | UnitState::Tried | UnitState::Failing
            )
        };
        let offer_migrate = match sel {
            Selection::Project => self.snapshot.units.iter().any(migratable),
            Selection::Unit(_) => owner.is_some_and(|u| migratable(&self.snapshot.units[u])),
            _ => false,
        };
        if offer_migrate {
            let mut it = item("Migrate — ask in chat", Action::Migrate, None);
            it.model = true;
            it.greyed = Some(
                "model work happens in chat, which is not built yet; see Help (?) for today's \
                 route"
                    .into(),
            );
            items.push(it);
        }
        if self.running {
            items.push(item(
                "Cancel the running command",
                Action::Cancel,
                Some("x"),
            ));
        }
        // Model-backed items last, under the separator.
        items.sort_by_key(|i| i.model);
        items
    }

    fn attempt_items(&self, u: usize, id: &str, items: &mut Vec<Item>) {
        let unit = &self.snapshot.units[u];
        let Some(a) = unit.attempt(id) else {
            return;
        };
        let r = &a.record;
        let finished = r.outcome != "in-progress";
        if a.verdict.as_ref().is_some_and(|v| !v.checks.is_empty()) {
            items.push(item("Show the checks", Action::ShowChecks, Some("v")));
        }
        // Accept: a green attempt (greyed when it cannot be promoted now).
        if r.outcome == "green" {
            let replace =
                r.promoted || matches!(unit.report.status.as_str(), "verified" | "merged");
            let label = if replace {
                format!(
                    "Replace {}'s verified crate with {}",
                    unit.unit.id,
                    crate::app::short_id(id)
                )
            } else {
                format!("Accept {} into {}", crate::app::short_id(id), unit.unit.id)
            };
            let mut accept = self.act_item(label, Act::Accept, Some(unit), Some(id));
            if accept.greyed.is_none() {
                accept.greyed = self.unknown_crate(u);
            }
            items.push(accept);
        }
        if self.diff_available(unit, a) {
            items.push(item(
                "Compare with the promoted attempt",
                Action::Compare,
                Some("d"),
            ));
        }
        if a.is_human() {
            return;
        }
        // Model-backed.
        if finished {
            let mut it = item("Modify with a note", Action::Modify, Some("m"));
            it.model = true;
            match self.act_argv(Act::Modify, Some(&unit.unit.id), Some(id), Some("-")) {
                Ok(_) => it.greyed = self.busy(),
                Err(why) => it.greyed = Some(why),
            }
            items.push(it);
        }
        // Retry: never offered for a blind `external` hand-off.
        if !crate::app::blind(r) {
            items.push(self.act_item("Retry".into(), Act::Retry, Some(unit), Some(id)));
        }
        if self
            .awaiting
            .iter()
            .any(|aw| aw.attempt.as_deref() == Some(id))
        {
            items.push(self.act_item("Resume".into(), Act::Resume, Some(unit), Some(id)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app, app_of, strs};
    use crate::app::{Command, Mode};
    use crate::tree::Expansion;

    /// Every node of the app's tree, everything unfolded.
    fn every_node(app: &mut App) -> Vec<Selection> {
        let seen;
        loop {
            let rows: Vec<Selection> = app
                .rows
                .iter()
                .filter_map(|r| r.selection().cloned())
                .collect();
            let closed: Vec<Selection> = app
                .rows
                .iter()
                .filter(|r| r.expandable && !r.open)
                .filter_map(|r| r.selection().cloned())
                .collect();
            if closed.is_empty() {
                seen = rows;
                break;
            }
            for sel in closed {
                app.expansion.set(&sel, true);
            }
            app.select(Selection::Units);
            app.select(Selection::Project);
            app.rows = crate::tree::rows(&app.snapshot, &app.files, &app.walk, &app.expansion);
        }
        seen
    }

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| match &i.greyed {
                Some(_) => format!("({})", i.label),
                None => i.label.clone(),
            })
            .collect()
    }

    /// §4.2, the table: the items of each node of the tractor case.
    #[test]
    fn the_items_fit_each_node() {
        let mut app = app("menutable");
        let mut at = |sel: Selection| {
            app.select(sel);
            labels(&app.menu_items())
        };
        assert_eq!(
            at(Selection::Project),
            [
                "Open",
                "Re-read the project",
                "Scan the project",
                "Refresh the plan",
                "Find hazards (run the detectors)"
            ]
        );
        assert_eq!(
            at(Selection::Dir("test_case".into())),
            ["Open", "Fold", "Re-read the project"]
        );
        assert_eq!(
            at(Selection::File("test_case/src/lib.c".into())),
            [
                "Open",
                "Re-read the project",
                "Find hazards (run the detectors)",
                "Open unit u-lib",
                "Show the checks (u-lib)",
                "Re-check with the oracle (u-lib)"
            ]
        );
        assert_eq!(
            at(Selection::File("test_case/include/lib.h".into())),
            [
                "Open",
                "Re-read the project",
                "Find hazards (run the detectors)"
            ]
        );
        assert_eq!(
            at(Selection::Unit("u-lib".into())),
            [
                "Open",
                "Re-read the project",
                "Show the checks",
                "Re-check with the oracle"
            ]
        );
        assert_eq!(
            at(Selection::Crate("u-lib".into())),
            [
                "Open",
                "Re-read the project",
                "Show the checks",
                "Re-check with the oracle",
                "Hand edit"
            ]
        );
        assert_eq!(
            at(Selection::Attempt("u-lib".into(), "a-13c941dfff95".into())),
            [
                "Open",
                "Re-read the project",
                "Hand edit",
                "Show the checks",
                "Replace u-lib's verified crate with a-13c9",
                "Modify with a note"
            ]
        );
        assert_eq!(
            at(Selection::Attempt("u-lib".into(), "a-28d8ddc411f9".into())),
            [
                "Open",
                "Re-read the project",
                "Hand edit",
                "Show the checks",
                "Replace u-lib's verified crate with a-28d8",
                "Compare with the promoted attempt",
                "Modify with a note"
            ]
        );
        assert_eq!(
            at(Selection::Attempt("u-lib".into(), "a-d2e5513cdfa6".into())),
            [
                "Open",
                "Re-read the project",
                "Hand edit",
                "Show the checks",
                "Compare with the promoted attempt",
                "Modify with a note"
            ]
        );
    }

    /// Model-backed items come last, under the separator; a greyed item
    /// says why in the footer and never spawns.
    #[test]
    fn model_items_come_last_and_greyed_items_explain_themselves() {
        let mut app = app("menuorder");
        app.running = true;
        app.select(Selection::Attempt("u-lib".into(), "a-13c941dfff95".into()));
        let items = app.menu_items();
        let first_model = items.iter().position(|i| i.model).unwrap();
        assert!(items[first_model..].iter().all(|i| i.model));
        assert!(items.iter().any(|i| i.action == Action::Cancel));
        app.open_menu();
        let Mode::Menu(m) = &app.mode else { panic!() };
        let greyed = m.items.iter().position(|i| i.greyed.is_some()).unwrap();
        if let Mode::Menu(m) = &mut app.mode {
            m.focus = greyed;
        }
        assert_eq!(
            app.on_key(
                ratatui::crossterm::event::KeyEvent::from(
                    ratatui::crossterm::event::KeyCode::Enter
                ),
                std::time::Instant::now()
            ),
            Command::None
        );
        let Mode::Menu(m) = &app.mode else {
            panic!("the menu stays open")
        };
        assert!(m
            .footer
            .as_deref()
            .unwrap()
            .contains("a command is running"));
    }

    fn find(app: &App, action: &Action) -> Item {
        app.menu_items()
            .into_iter()
            .find(|i| i.action == *action)
            .unwrap_or_else(|| panic!("no {action:?} in {:?}", labels(&app.menu_items())))
    }

    /// Review SAFE-8, SAFE-9: with the unit crate changed outside the
    /// harness, Accept (which would replace it) and a hand edit of it (which
    /// would record it all as a human's) wait until it is recorded or
    /// restored. Review ENG-4: Re-check needs the unit's code on screen.
    #[test]
    fn unknown_code_is_never_replaced_or_relabelled() {
        let mut app = app("unknownmenu");
        let logic = app.snapshot.units[0]
            .crate_dir
            .clone()
            .unwrap()
            .join("src/logic.rs");
        let text = std::fs::read_to_string(&logic).unwrap();
        std::fs::write(&logic, format!("{text}\n// someone\n")).unwrap();
        assert!(app.reload(true));
        app.select(Selection::Attempt("u-lib".into(), "a-13c941dfff95".into()));
        let accept = find(&app, &Action::Act(crate::app::Act::Accept));
        assert!(accept.greyed.unwrap().contains("does not know"));
        app.select(Selection::Crate("u-lib".into()));
        assert!(find(&app, &Action::HandEdit)
            .greyed
            .unwrap()
            .contains("does not know"));
        std::fs::write(&logic, &text).unwrap();
        assert!(app.reload(true));
        assert_eq!(find(&app, &Action::HandEdit).greyed, None);
        // An internal function shows C source: Re-check waits for the unit.
        app.select(Selection::Function(
            "test_case/src/lib.c".into(),
            "test_case/src/lib.c::get_bits".into(),
        ));
        let recheck = find(&app, &Action::Act(crate::app::Act::Verify));
        assert!(recheck.greyed.unwrap().contains("open the unit"));
        app.select(Selection::Unit("u-lib".into()));
        assert_eq!(
            find(&app, &Action::Act(crate::app::Act::Verify)).greyed,
            None
        );
    }

    /// Review ENG-5: Accept from a unit opens the attempt first (where its
    /// code is shown); review SAFE-7: Discard waits while a command runs.
    #[test]
    fn unit_accept_opens_the_attempt_and_discard_waits() {
        let mut app = app("unitaccept");
        let ledger = harness_core::ledger::Ledger::new(&app.config.target);
        let dir = harness_core::attempts::attempt_dir(&ledger, "u-lib", "a-28d8ddc411f9");
        let mut r = harness_core::attempts::AttemptRecord::load(&dir).unwrap();
        r.promoted = false;
        r.store(&dir).unwrap();
        // An earlier green attempt whose last turn is not green: not
        // acceptable — the jump must skip it as the menu does (ENG-5).
        let first = harness_core::attempts::attempt_dir(&ledger, "u-lib", "a-13c941dfff95");
        let mut r = harness_core::attempts::AttemptRecord::load(&first).unwrap();
        r.promoted = false;
        r.turns.last_mut().unwrap().result = "oracle".into();
        r.store(&first).unwrap();
        assert!(app.reload(true));
        app.select(Selection::Unit("u-lib".into()));
        let it = app
            .menu_items()
            .into_iter()
            .find(|i| i.label.starts_with("Accept a-28d8 into u-lib"))
            .expect("the unit's Accept");
        assert_eq!(it.action, Action::ChooseAttempt("u-lib".into()));
        assert!(
            it.pending.is_none(),
            "nothing is accepted from the unit's view"
        );
        app.choose(&it);
        assert_eq!(
            app.selection,
            Selection::Attempt("u-lib".into(), "a-28d8ddc411f9".into())
        );
        app.edit_staged("u-lib".into(), "/tmp/d/stage".into(), "/tmp/d".into());
        app.mode = Mode::Normal;
        app.select(Selection::Project);
        app.running = true;
        assert!(find(&app, &Action::DiscardKept).greyed.is_some());
    }

    /// Second fix pass, ENG-6: a missing file counts once in the greyed
    /// reason too; NEW-10: an owned header with no function shows its
    /// source, and Re-check waits for the unit.
    #[test]
    fn counts_and_owned_headers() {
        let mut app = app("countsmenu");
        std::fs::remove_file(app.config.target.join("test_case/include/lib.h")).unwrap();
        assert!(app.reload(true));
        let plan = find(&app, &Action::Act(crate::app::Act::Plan));
        assert_eq!(
            plan.greyed.as_deref(),
            Some("1 file changed or new since the scan — scan first")
        );
        let mut app = crate::app::tests::app("ownedhmenu");
        let p = app.config.target.join("migration/plan.toml");
        let text = std::fs::read_to_string(&p).unwrap();
        std::fs::write(
            &p,
            text.replace(
                "files = [\"test_case/src/lib.c\"]",
                "files = [\"test_case/src/lib.c\", \"test_case/include/lib.h\"]",
            ),
        )
        .unwrap();
        assert!(app.reload(true));
        app.select(Selection::File("test_case/include/lib.h".into()));
        assert!(app.source.is_some(), "its source, not an empty pairs view");
        let recheck = find(&app, &Action::Act(crate::app::Act::Verify));
        assert!(recheck.greyed.unwrap().contains("open the unit"));
    }

    /// Mutation-checked rules, over every node of both committed targets:
    /// no tree path ever enters an argv; a greyed item never spawns; Modify
    /// always carries `--provider=`.
    #[test]
    fn no_tree_path_in_any_argv_and_greyed_items_never_spawn() {
        for (rel, tag) in [
            (crate::testutil::READ_SCALEFACTORS, "menuprop"),
            ("targets/zopfli", "menupropz"),
        ] {
            let mut app = app_of(rel, tag);
            app.expansion = Expansion::default();
            let nodes = every_node(&mut app);
            assert!(nodes.len() > 5, "{rel}: {nodes:?}");
            let paths: Vec<String> = app
                .files
                .files
                .iter()
                .map(|f| f.path.clone())
                .chain(nodes.iter().filter_map(|n| match n {
                    Selection::Dir(d) => Some(d.clone()),
                    _ => None,
                }))
                .collect();
            let mut acts = 0;
            for sel in &nodes {
                app.select(sel.clone());
                for it in app.menu_items() {
                    if let Some(p) = &it.pending {
                        acts += 1;
                        for arg in strs(&p.argv) {
                            for path in &paths {
                                assert!(
                                    !arg.contains(path.as_str()),
                                    "{sel:?} {}: {arg} holds {path}",
                                    it.label
                                );
                            }
                        }
                    }
                    if it.greyed.is_some() {
                        let before = app.mode.clone();
                        assert_eq!(app.choose(&it), Command::None, "{sel:?} {}", it.label);
                        assert!(!matches!(app.mode, Mode::Dialog(_)), "{sel:?} {}", it.label);
                        app.mode = before;
                    }
                }
                if let Selection::Attempt(u, a) = sel {
                    if let Ok(p) =
                        app.act_argv(crate::app::Act::Modify, Some(u), Some(a), Some("x"))
                    {
                        assert!(strs(&p.argv).iter().any(|s| s.starts_with("--provider=")));
                    }
                }
            }
            assert!(acts > 3, "{rel}: the property saw acts");
        }
    }
}
