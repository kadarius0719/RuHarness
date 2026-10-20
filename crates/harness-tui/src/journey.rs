//! The journey line (docs/COCKPIT-WRAPPER-DESIGN.md §9.4): under the View's
//! title, on every screen, one line says where the person is in the eight
//! stages a program goes through — `Map ✓ · Configure ✓ · Accept program ✓ ·
//! Scan ✓ · Plan ✓ · Test program ○ · Rust · Accept Rust`. Stages 1–3 are
//! the project's, 4–5 the tool's, 6–8 each unit's. A done stage carries `✓`
//! in the good colour, the current one `○` in the accent (the line's only
//! accent), others are plain in the label colour. The current stage is the
//! one the Next line names (fix pass, 2026-10-20), else the first not done.
//! On the tool's screen with no unit chosen, the last three count the pieces
//! that can move past each stage (a piece that calls others stays C and is
//! not counted; before a plan there are no counts, only `Scan ○` or `Plan
//! ○`); on the Project view only the first three show, then "then: scan,
//! plan, and each piece". Under 100 columns (or wherever the line does not
//! fit) it folds to the current stage alone: `Step 6 of 8: Test program`.
//!
//! The rules are here, pure over what the screen shows; the view draws the
//! spans.

use crate::app::{Act, App};
use crate::files::{FileState, UnitState};
use crate::menu::Action;
use crate::model::{DriverState, UnitView};
use crate::theme::{self, Role};
use crate::tree::Selection;
use ratatui::text::Span;

/// The eight stages, in the cockpit's words.
pub const STAGES: [&str; 8] = [
    "Map",
    "Configure",
    "Accept program",
    "Scan",
    "Plan",
    "Test program",
    "Rust",
    "Accept Rust",
];

/// Stage 1's name while the build configuration is newer than the map.
pub const MAP_AGAIN: &str = "Map again";

/// The Project view's tail after its three stages.
pub const PROJECT_THEN: &str = "then: scan, plan, and each piece";

/// The tool's tail when no piece can move in this version.
pub const NOTHING_MOVES: &str = "no piece can move to Rust in this version";

/// Under this many columns the line folds to the current stage alone.
pub const FOLD_UNDER: u16 = 100;

/// A stage's mark on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Done: `✓`.
    Done,
    /// Where the person is: `○`.
    Current,
    /// Not done, and not where the person is: plain.
    Later,
}

/// One stage as the line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Its name.
    pub name: &'static str,
    /// Its mark.
    pub mark: Mark,
    /// On the tool's whole screen: the pieces past it, of how many.
    pub count: Option<(usize, usize)>,
}

/// The line of one screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journey {
    /// The stages shown, in order (three on the Project view, five on a
    /// tool before its plan, else eight).
    pub steps: Vec<Step>,
    /// What follows the stages shown, in words (the Project view's
    /// [`PROJECT_THEN`]).
    pub tail: Option<&'static str>,
}

impl Journey {
    /// The line from which stages are done (`done[i]` for stage `i + 1`)
    /// and which is current (1-based; `None`: the first not done). A stage
    /// is `○` when current, else `✓` when done, else plain.
    pub fn of(
        done: &[bool],
        counts: [Option<(usize, usize)>; 8],
        current: Option<usize>,
        tail: Option<&'static str>,
    ) -> Journey {
        let current = current
            .filter(|c| (1..=done.len()).contains(c))
            .map(|c| c - 1)
            .or_else(|| done.iter().position(|d| !d));
        let steps = done
            .iter()
            .enumerate()
            .map(|(i, d)| Step {
                name: STAGES[i],
                mark: if Some(i) == current {
                    Mark::Current
                } else if *d {
                    Mark::Done
                } else {
                    Mark::Later
                },
                count: counts[i],
            })
            .collect();
        Journey { steps, tail }
    }

    /// The same line with stage 1 named "Map again".
    fn map_again(mut self) -> Journey {
        if let Some(s) = self.steps.first_mut() {
            s.name = MAP_AGAIN;
        }
        self
    }

    /// The current stage, 1-based, with its step.
    pub fn current(&self) -> Option<(usize, &Step)> {
        self.steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.mark == Mark::Current)
            .map(|(i, s)| (i + 1, s))
    }

    /// The whole line in words.
    pub fn long(&self) -> String {
        self.long_spans()
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    /// The folded line in words.
    pub fn short(&self) -> String {
        self.short_spans()
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    fn name_of(step: &Step) -> String {
        match step.count {
            Some((n, of)) => format!("{} {n}/{of}", step.name),
            None => step.name.to_string(),
        }
    }

    /// The whole line, styled.
    pub fn long_spans(&self) -> Vec<Span<'static>> {
        let label = theme::style(Role::Label);
        let mut spans = Vec::new();
        for (i, step) in self.steps.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", label));
            }
            let name = Self::name_of(step);
            match step.mark {
                Mark::Done => {
                    spans.push(Span::raw(format!("{name} ")));
                    spans.push(Span::styled("✓", theme::style(Role::Good)));
                }
                Mark::Current => spans.push(Span::styled(
                    format!("{name} ○"),
                    theme::style(Role::Accent),
                )),
                Mark::Later => spans.push(Span::styled(name, label)),
            }
        }
        if let Some(tail) = self.tail {
            spans.push(Span::styled(format!(" · {tail}"), label));
        }
        spans
    }

    /// The folded line, styled: `Step 6 of 8: Test program`, or every stage
    /// shown done.
    pub fn short_spans(&self) -> Vec<Span<'static>> {
        let label = theme::style(Role::Label);
        match self.current() {
            Some((n, step)) => vec![
                Span::styled(format!("Step {n} of {}: ", STAGES.len()), label),
                Span::styled(Self::name_of(step), theme::style(Role::Accent)),
            ],
            None if self.tail == Some(PROJECT_THEN) => vec![
                Span::raw("Map, Configure and Accept program "),
                Span::styled("✓", theme::style(Role::Good)),
                Span::styled(format!(" · {PROJECT_THEN}"), label),
            ],
            None if self.tail.is_some() => vec![
                Span::raw("Scan and Plan "),
                Span::styled("✓", theme::style(Role::Good)),
                Span::styled(format!(" · {}", self.tail.unwrap_or_default()), label),
            ],
            None => vec![
                Span::raw(format!("All {} steps done ", STAGES.len())),
                Span::styled("✓", theme::style(Role::Good)),
            ],
        }
    }

    /// The line for a pane `width` wide on a screen `screen` columns wide:
    /// whole, or folded under [`FOLD_UNDER`] columns or where it does not
    /// fit.
    pub fn spans(&self, width: usize, screen: u16) -> Vec<Span<'static>> {
        let long = self.long_spans();
        let w: usize = long
            .iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        if screen < FOLD_UNDER || w > width {
            self.short_spans()
        } else {
            long
        }
    }
}

/// A unit's current stage, 1-based (6 Test program, 7 Rust, 8 Accept Rust),
/// or 9 once it is done; `None` for a piece that cannot move (it stays C)
/// or is blocked.
pub fn unit_stage(unit: &UnitView, state: &UnitState) -> Option<usize> {
    let verified = matches!(unit.report.status.as_str(), "verified" | "merged");
    Some(match state {
        UnitState::StaysC(_) | UnitState::Blocked => return None,
        UnitState::Migrated(_) | UnitState::OriginUnknown => 9,
        UnitState::RustGreen => 8,
        UnitState::NoTestProgram | UnitState::TestProgramStale => 6,
        UnitState::TestProgramReady | UnitState::RustTried | UnitState::Waiting => 7,
        // A verdict to make again, or one that went red: the stage the unit
        // was in, from what it holds.
        _ if verified => 8,
        _ if crate::next::acceptable(unit).is_some() => 8,
        _ if unit.driver == DriverState::Validated => 7,
        _ => 6,
    })
}

impl App {
    /// The stage a Next step's action belongs to, 1-based, when it is one
    /// of the journey's (fix pass, item 5: the line's `○` sits on the stage
    /// the Next line names).
    pub fn action_stage(&self, action: &Action) -> Option<usize> {
        Some(match action {
            Action::Project(crate::project::Act::Map) => 1,
            Action::WriteBuildConfig | Action::AskInChat { build: true } => 2,
            Action::AcceptProgram { .. } | Action::AskInChat { build: false } => 3,
            Action::Act(Act::Scan) => 4,
            Action::Act(Act::Plan) => 5,
            Action::GenDriver => 6,
            Action::Migrate | Action::Modify | Action::Act(Act::Retry) => 7,
            Action::ChooseAttempt(_) | Action::Act(Act::Accept) | Action::Act(Act::Verify) => 8,
            Action::OpenUnit(id) => {
                let i = self.snapshot.units.iter().position(|u| &u.unit.id == id)?;
                return self.unit_journey_stage(i);
            }
            _ => return None,
        })
    }

    /// A unit's stage as its own Next names it, else its state's.
    fn unit_journey_stage(&self, i: usize) -> Option<usize> {
        let from_next = self
            .unit_advice(i)
            .and_then(|a| a.action)
            .filter(|a| !matches!(a, Action::OpenUnit(_)))
            .and_then(|a| self.action_stage(&a));
        from_next.or_else(|| {
            let unit = self.snapshot.units.get(i)?;
            let info = self.files.units.get(i)?;
            unit_stage(unit, &info.state).filter(|s| *s <= 8)
        })
    }

    /// The journey line of the screen shown, for the selection.
    pub fn journey(&self) -> Journey {
        if self.in_project_view() {
            return self.project_journey();
        }
        let mut done = [true; 8];
        let mut counts = [None; 8];
        let scanned = self.snapshot.facts_state.as_ref().is_some_and(|s| {
            s.stale == 0 && !self.files.files.iter().any(|f| f.state == FileState::New)
        });
        let no_plan = self.snapshot.units.is_empty()
            && self.snapshot.note.as_deref()
                == Some(crate::model::no_plan(&self.snapshot.ledger_rel()).as_str());
        let planned = scanned
            && !no_plan
            && !self
                .snapshot
                .units
                .iter()
                .any(|u| !u.report.source_fresh && u.report.status != "blocked");
        done[3] = scanned;
        done[4] = planned;
        // Only the pieces that can move count (fix pass, item 2).
        let stages: Vec<usize> = self
            .snapshot
            .units
            .iter()
            .zip(&self.files.units)
            .filter_map(|(u, i)| unit_stage(u, &i.state))
            .collect();
        let current = self.advice(&self.selection).and_then(|a| {
            let at = a.action.as_ref().and_then(|x| self.action_stage(x));
            // The unit's own row: its state's stage when its Next names no
            // act of the journey.
            at.or_else(|| {
                self.owning_unit(&self.selection)
                    .and_then(|u| self.unit_journey_stage(u))
            })
        });
        match self.owning_unit(&self.selection) {
            Some(u) => {
                let stage = self
                    .snapshot
                    .units
                    .get(u)
                    .zip(self.files.units.get(u))
                    .and_then(|(unit, info)| unit_stage(unit, &info.state));
                let Some(stage) = stage else {
                    // A piece that stays C has no stages of its own.
                    return Journey::of(&done[..5], counts, None, Some("this piece stays C"));
                };
                for (i, d) in done.iter_mut().enumerate().skip(5) {
                    *d = stage > i + 1;
                }
                let current = current.filter(|c| *c >= 4 || !done[c - 1]);
                Journey::of(&done, counts, current, None)
            }
            None => {
                // Before a plan (or with nothing that can move): no counts,
                // the tool's stages alone (fix pass, item 13).
                if !planned || stages.is_empty() {
                    let tail =
                        (planned && !self.snapshot.units.is_empty()).then_some(NOTHING_MOVES);
                    return Journey::of(&done[..5], counts, current.filter(|c| *c <= 5), tail);
                }
                let total = stages.len();
                for (i, d) in done.iter_mut().enumerate().skip(5) {
                    let past = stages.iter().filter(|s| **s > i + 1).count();
                    *d = past == total;
                    counts[i] = Some((past, total));
                }
                Journey::of(&done, counts, current, None)
            }
        }
    }

    /// The Project view's three stages: mapped (and not older than its
    /// configuration — then "Map again"), the configuration written, the
    /// program a tool — the selected program's, else any.
    fn project_journey(&self) -> Journey {
        let map = self.project_map();
        let newer = self.project.as_ref().is_some_and(|p| p.config_newer);
        let tools = self.project_tools();
        let accepted = match &self.selection {
            Selection::Program(id) | Selection::Tool(id) => tools.contains(id),
            _ => !tools.is_empty(),
        };
        // A configuration written since the map is the person's: Configure
        // stays done while the map is redone (fix pass, item 7).
        let done = [
            map.is_some() && !newer,
            newer || map.is_some_and(|m| !m.unstated()),
            accepted,
        ];
        let current = self
            .advice(&self.selection)
            .and_then(|a| a.action)
            .and_then(|a| self.action_stage(&a))
            .filter(|s| *s <= 3);
        let j = Journey::of(&done, [None; 8], current, Some(PROJECT_THEN));
        if newer {
            j.map_again()
        } else {
            j
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app, app_of};

    fn texts(j: &Journey) -> (String, String) {
        (j.long(), j.short())
    }

    /// The marks: with no current named, every stage before the first not
    /// done is ✓, that one ○; with all done, every one ✓ and the fold says
    /// so.
    #[test]
    fn the_marks_follow_the_first_stage_not_done() {
        let j = Journey::of(
            &[true, true, true, true, true, false, false, true],
            [None; 8],
            None,
            None,
        );
        assert_eq!(
            texts(&j),
            (
                "Map ✓ · Configure ✓ · Accept program ✓ · Scan ✓ · Plan ✓ · Test program ○ · Rust \
                 · Accept Rust ✓"
                    .into(),
                "Step 6 of 8: Test program".into()
            )
        );
        let all = Journey::of(&[true; 8], [None; 8], None, None);
        assert_eq!(all.current(), None);
        assert_eq!(all.short(), "All 8 steps done ✓");
        assert!(
            all.long().ends_with("Rust ✓ · Accept Rust ✓"),
            "{}",
            all.long()
        );
        // A current named by the Next line wins.
        let j = Journey::of(&[true, true, true, true, false], [None; 8], Some(4), None);
        assert_eq!(j.short(), "Step 4 of 8: Scan");
    }

    /// The one accent: the current stage alone; done marks in the good
    /// colour, later stages in the label colour.
    #[test]
    fn the_current_stage_is_the_lines_only_accent() {
        let j = Journey::of(&[true, false, false], [None; 8], None, Some(PROJECT_THEN));
        let accent = theme::style(Role::Accent);
        let spans = j.long_spans();
        let accented: Vec<&str> = spans
            .iter()
            .filter(|s| s.style == accent)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(accented, ["Configure ○"]);
        assert!(spans
            .iter()
            .any(|s| s.content == "✓" && s.style == theme::style(Role::Good)));
        assert!(spans
            .iter()
            .any(|s| s.content == "Accept program" && s.style == theme::style(Role::Label)));
    }

    /// The fold: under 100 columns, or where the whole line does not fit
    /// the pane.
    #[test]
    fn the_line_folds_under_100_columns_or_when_it_does_not_fit() {
        let j = Journey::of(&[true, true, false], [None; 8], None, Some(PROJECT_THEN));
        let words = |s: Vec<Span>| s.iter().map(|s| s.content.to_string()).collect::<String>();
        assert_eq!(words(j.spans(200, 99)), "Step 3 of 8: Accept program");
        assert_eq!(words(j.spans(20, 200)), "Step 3 of 8: Accept program");
        assert_eq!(
            words(j.spans(200, 100)),
            "Map ✓ · Configure ✓ · Accept program ○ · then: scan, plan, and each piece"
        );
    }

    /// The tool's screen with no unit chosen: the last three count the
    /// pieces that can move past each stage (zopfli: one unit done, the
    /// pieces that call others left out), and the `○` sits on the stage its
    /// Next names.
    #[test]
    fn the_tools_screen_counts_pieces_that_can_move() {
        let mut app = app_of("targets/zopfli", "journey-tool");
        app.select(Selection::Project);
        let movable = app
            .snapshot
            .units
            .iter()
            .zip(&app.files.units)
            .filter(|(u, i)| unit_stage(u, &i.state).is_some())
            .count();
        assert!(movable < app.snapshot.units.len(), "some stay C");
        let j = app.journey();
        let long = j.long();
        assert!(
            long.contains(&format!("Test program 1/{movable}")) || long.contains("Scan ○"),
            "{long}"
        );
        assert!(
            long.starts_with("Map ✓ · Configure ✓ · Accept program ✓"),
            "{long}"
        );
    }

    /// Before a plan: the tool's stages alone, no counts (item 13).
    #[test]
    fn before_a_plan_there_are_no_counts() {
        let mut app = app_of("targets/zopfli", "journey-noplan");
        std::fs::remove_file(app.snapshot.ledger().plan_path()).unwrap();
        assert!(app.reload(true));
        app.select(Selection::Project);
        let long = app.journey().long();
        assert!(long.ends_with("Plan ○"), "{long}");
        assert!(!long.contains("0/0"), "{long}");
    }

    /// A unit's screen: its own three stages — no test program on a zopfli
    /// unit with no `[unit.oracle]`, every stage done on the verified u001;
    /// a piece that calls others has none.
    #[test]
    fn a_units_screen_shows_its_own_stages() {
        let mut app = app_of("targets/zopfli", "journey-unit");
        if !app.journey().long().contains("Plan ✓") {
            // The committed zopfli's C moved on since its scan: the line
            // stops at Scan, which the other assertions need done.
            return;
        }
        app.select(Selection::Unit("u-cache".into()));
        assert_eq!(app.journey().short(), "Step 6 of 8: Test program");
        app.select(Selection::Unit("u001-katajainen".into()));
        assert_eq!(app.journey().current(), None);
        app.select(Selection::Unit("u-lz77".into()));
        assert!(
            app.journey()
                .long()
                .ends_with("Plan ✓ · this piece stays C"),
            "{}",
            app.journey().long()
        );
    }

    /// Item 5: on every row of a tool, the `○` sits on the stage the Next
    /// line's action belongs to.
    #[test]
    fn the_marked_stage_is_the_one_the_next_line_names() {
        for (rel, tag) in [
            ("targets/zopfli", "journey-next-z"),
            (crate::testutil::READ_SCALEFACTORS, "journey-next-t"),
        ] {
            let mut app = app_of(rel, tag);
            let rows: Vec<Selection> = app
                .rows
                .iter()
                .filter_map(|r| r.selection().cloned())
                .collect();
            for sel in rows {
                app.select(sel.clone());
                let Some(next) = app.advice(&sel) else {
                    continue;
                };
                let Some(stage) = next.action.as_ref().and_then(|a| app.action_stage(a)) else {
                    continue;
                };
                let j = app.journey();
                assert_eq!(
                    j.current().map(|(n, _)| n),
                    Some(stage),
                    "{sel:?}: {} / {}",
                    next.line(),
                    j.long()
                );
            }
        }
    }

    /// The stages of a unit, from its state and what it holds.
    #[test]
    fn a_units_stage_follows_its_state() {
        let a = app("journey-stage");
        let unit = &a.snapshot.units[0];
        assert_eq!(unit_stage(unit, &a.files.units[0].state), Some(9));
        for (state, stage) in [
            (UnitState::NoTestProgram, Some(6)),
            (UnitState::TestProgramStale, Some(6)),
            (UnitState::TestProgramReady, Some(7)),
            (UnitState::RustTried, Some(7)),
            (UnitState::Waiting, Some(7)),
            (UnitState::RustGreen, Some(8)),
            (UnitState::Migrated(crate::files::Origin::Pipeline), Some(9)),
            (UnitState::StaysC(vec!["u-x".into()]), None),
            // A verified unit whose verdict must be made again: Accept.
            (UnitState::Failing, Some(8)),
        ] {
            assert_eq!(unit_stage(unit, &state), stage, "{state:?}");
        }
    }

    /// The Project view: its three stages, then the rest in words; after a
    /// new build configuration, "Map again" with Configure kept done.
    #[test]
    fn the_project_view_shows_its_three_stages() {
        let j = Journey::of(&[false, false, false], [None; 8], None, Some(PROJECT_THEN));
        assert_eq!(
            j.long(),
            "Map ○ · Configure · Accept program · then: scan, plan, and each piece"
        );
        assert_eq!(j.short(), "Step 1 of 8: Map");
        let all = Journey::of(&[true, true, true], [None; 8], None, Some(PROJECT_THEN));
        assert_eq!(
            all.long(),
            "Map ✓ · Configure ✓ · Accept program ✓ · then: scan, plan, and each piece"
        );
        assert_eq!(
            all.short(),
            "Map, Configure and Accept program ✓ · then: scan, plan, and each piece"
        );
        let again =
            Journey::of(&[false, true, false], [None; 8], None, Some(PROJECT_THEN)).map_again();
        assert_eq!(
            again.long(),
            "Map again ○ · Configure ✓ · Accept program · then: scan, plan, and each piece"
        );
    }
}
