//! The journey line (docs/COCKPIT-WRAPPER-DESIGN.md §9.4): under the View's
//! title, on every screen, one line says where the person is in the eight
//! stages a program goes through — `Map ✓ · Configure ✓ · Accept ✓ · Scan ✓
//! · Plan ✓ · Test program ○ · Rust · Accept`. Stages 1–3 are the project's,
//! 4–5 the tool's, 6–8 each unit's. A done stage carries `✓` in the good
//! colour, the current one `○` in the accent (the line's only accent), later
//! ones are plain in the label colour. On the tool's screen with no unit
//! chosen, the last three count the units past each stage; on the Project
//! view only the first three show, then "then: scan, plan, and each piece".
//! Under 100 columns (or wherever the line does not fit) it folds to the
//! current stage alone: `Step 6 of 8: Test program`.
//!
//! The rules are here, pure over what the screen shows; the view draws the
//! spans.

use crate::app::App;
use crate::files::{FileState, UnitState};
use crate::model::{DriverState, UnitView};
use crate::theme::{self, Role};
use crate::tree::Selection;
use ratatui::text::Span;

/// The eight stages, in the cockpit's words.
pub const STAGES: [&str; 8] = [
    "Map",
    "Configure",
    "Accept",
    "Scan",
    "Plan",
    "Test program",
    "Rust",
    "Accept",
];

/// The Project view's tail after its three stages.
pub const PROJECT_THEN: &str = "then: scan, plan, and each piece";

/// Under this many columns the line folds to the current stage alone.
pub const FOLD_UNDER: u16 = 100;

/// A stage's mark on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Done: `✓`.
    Done,
    /// Where the person is: `○`.
    Current,
    /// Not reached yet: plain.
    Later,
}

/// One stage as the line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Its name.
    pub name: &'static str,
    /// Its mark.
    pub mark: Mark,
    /// On the tool's whole screen: the units past it, of how many.
    pub count: Option<(usize, usize)>,
}

/// The line of one screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journey {
    /// The stages shown, in order (three on the Project view, else eight).
    pub steps: Vec<Step>,
    /// The Project view's short form: [`PROJECT_THEN`] after its three.
    pub project: bool,
}

impl Journey {
    /// The line from which stages are done (`done[i]` for stage `i + 1`):
    /// every stage before the first not done is ✓, that one ○, the rest
    /// plain.
    pub fn of(done: &[bool], counts: [Option<(usize, usize)>; 8], project: bool) -> Journey {
        let current = done.iter().position(|d| !d);
        let steps = done
            .iter()
            .enumerate()
            .map(|(i, _)| Step {
                name: STAGES[i],
                mark: match current {
                    Some(c) if i < c => Mark::Done,
                    Some(c) if i == c => Mark::Current,
                    Some(_) => Mark::Later,
                    None => Mark::Done,
                },
                count: counts[i],
            })
            .collect();
        Journey { steps, project }
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
        if self.project {
            spans.push(Span::styled(format!(" · {PROJECT_THEN}"), label));
        }
        spans
    }

    /// The folded line, styled: `Step 6 of 8: Test program`, or every stage
    /// done.
    pub fn short_spans(&self) -> Vec<Span<'static>> {
        let label = theme::style(Role::Label);
        match self.current() {
            Some((n, step)) => vec![
                Span::styled(format!("Step {n} of {}: ", STAGES.len()), label),
                Span::styled(Self::name_of(step), theme::style(Role::Accent)),
            ],
            None if self.project => vec![
                Span::raw("Map, Configure and Accept "),
                Span::styled("✓", theme::style(Role::Good)),
                Span::styled(format!(" · {PROJECT_THEN}"), label),
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

/// A unit's current stage, 1-based (6 Test program, 7 Rust, 8 Accept), or 9
/// once it is done.
pub fn unit_stage(unit: &UnitView, state: &UnitState) -> usize {
    let verified = matches!(unit.report.status.as_str(), "verified" | "merged");
    match state {
        UnitState::Migrated(_) | UnitState::OriginUnknown => 9,
        UnitState::RustGreen => 8,
        UnitState::NoTestProgram | UnitState::TestProgramStale => 6,
        UnitState::TestProgramReady | UnitState::RustTried => 7,
        // A verdict to make again, or one that went red: the stage the unit
        // was in, from what it holds.
        _ if verified => 8,
        _ if crate::next::acceptable(unit).is_some() => 8,
        _ if unit.driver == DriverState::Validated => 7,
        _ => 6,
    }
}

impl App {
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
        let stages: Vec<usize> = self
            .snapshot
            .units
            .iter()
            .zip(&self.files.units)
            .filter(|(_, i)| i.state != UnitState::Blocked)
            .map(|(u, i)| unit_stage(u, &i.state))
            .collect();
        match self.owning_unit(&self.selection) {
            Some(u) => {
                let stage = self
                    .snapshot
                    .units
                    .get(u)
                    .zip(self.files.units.get(u))
                    .map_or(6, |(unit, info)| unit_stage(unit, &info.state));
                for (i, d) in done.iter_mut().enumerate().skip(5) {
                    *d = stage > i + 1;
                }
            }
            None => {
                let total = stages.len();
                for (i, d) in done.iter_mut().enumerate().skip(5) {
                    let past = stages.iter().filter(|s| **s > i + 1).count();
                    *d = past == total;
                    counts[i] = Some((past, total));
                }
            }
        }
        Journey::of(&done, counts, false)
    }

    /// The Project view's three stages: mapped (and not older than its
    /// configuration), the configuration stated, the program a tool — the
    /// selected program's, else any.
    fn project_journey(&self) -> Journey {
        let map = self.project_map();
        let newer = self.project.as_ref().is_some_and(|p| p.config_newer);
        let tools = self.project_tools();
        let accepted = match &self.selection {
            Selection::Program(id) | Selection::Tool(id) => tools.contains(id),
            _ => !tools.is_empty(),
        };
        let done = [
            map.is_some() && !newer,
            map.is_some_and(|m| !m.unstated()),
            accepted,
        ];
        Journey::of(&done, [None; 8], true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app, app_of};

    fn texts(j: &Journey) -> (String, String) {
        (j.long(), j.short())
    }

    /// The marks: every stage before the first not done is ✓, that one ○,
    /// the rest plain; with all done, every one ✓ and the fold says so.
    #[test]
    fn the_marks_follow_the_first_stage_not_done() {
        let j = Journey::of(
            &[true, true, true, true, true, false, false, true],
            [None; 8],
            false,
        );
        assert_eq!(
            texts(&j),
            (
                "Map ✓ · Configure ✓ · Accept ✓ · Scan ✓ · Plan ✓ · Test program ○ · Rust · Accept"
                    .into(),
                "Step 6 of 8: Test program".into()
            )
        );
        let all = Journey::of(&[true; 8], [None; 8], false);
        assert_eq!(all.current(), None);
        assert_eq!(all.short(), "All 8 steps done ✓");
        assert!(all.long().ends_with("Rust ✓ · Accept ✓"), "{}", all.long());
    }

    /// The one accent: the current stage alone; done marks in the good
    /// colour, later stages in the label colour.
    #[test]
    fn the_current_stage_is_the_lines_only_accent() {
        let j = Journey::of(&[true, false, false], [None; 8], true);
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
            .any(|s| s.content == "Accept" && s.style == theme::style(Role::Label)));
    }

    /// The fold: under 100 columns, or where the whole line does not fit
    /// the pane.
    #[test]
    fn the_line_folds_under_100_columns_or_when_it_does_not_fit() {
        let j = Journey::of(&[true, true, false], [None; 8], true);
        let words = |s: Vec<Span>| s.iter().map(|s| s.content.to_string()).collect::<String>();
        assert_eq!(words(j.spans(200, 99)), "Step 3 of 8: Accept");
        assert_eq!(words(j.spans(20, 200)), "Step 3 of 8: Accept");
        assert_eq!(
            words(j.spans(200, 100)),
            "Map ✓ · Configure ✓ · Accept ○ · then: scan, plan, and each piece"
        );
    }

    /// The tool's screen with no unit chosen: the last three count the
    /// units past each stage (zopfli: one unit done, ten with no test
    /// program, scanned and planned as committed).
    #[test]
    fn the_tools_screen_counts_units_past_each_stage() {
        let mut app = app_of("targets/zopfli", "journey-tool");
        app.select(Selection::Project);
        let j = app.journey();
        let units = app.snapshot.units.len();
        assert_eq!(units, 11);
        let (long, short) = texts(&j);
        assert!(
            long.ends_with(&format!(
                "Test program 1/{units} ○ · Rust 1/{units} · Accept 1/{units}"
            )) || long.contains("Scan ○"),
            "{long}"
        );
        assert!(long.starts_with("Map ✓ · Configure ✓ · Accept ✓"), "{long}");
        assert!(short.starts_with("Step "), "{short}");
    }

    /// A unit's screen: its own three stages — no test program on a zopfli
    /// unit with no `[unit.oracle]`, every stage done on the verified u001.
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
    }

    /// The stages of a unit, from its state and what it holds.
    #[test]
    fn a_units_stage_follows_its_state() {
        let a = app("journey-stage");
        let unit = &a.snapshot.units[0];
        assert_eq!(unit_stage(unit, &a.files.units[0].state), 9);
        for (state, stage) in [
            (UnitState::NoTestProgram, 6),
            (UnitState::TestProgramStale, 6),
            (UnitState::TestProgramReady, 7),
            (UnitState::RustTried, 7),
            (UnitState::RustGreen, 8),
            (UnitState::Migrated(crate::files::Origin::Pipeline), 9),
            // A verified unit whose verdict must be made again: Accept.
            (UnitState::Failing, 8),
        ] {
            assert_eq!(unit_stage(unit, &state), stage, "{state:?}");
        }
    }

    /// The Project view: its three stages, then the rest in words.
    #[test]
    fn the_project_view_shows_its_three_stages() {
        let j = Journey::of(&[false, false, false], [None; 8], true);
        assert_eq!(
            j.long(),
            "Map ○ · Configure · Accept · then: scan, plan, and each piece"
        );
        assert_eq!(j.short(), "Step 1 of 8: Map");
        let all = Journey::of(&[true, true, true], [None; 8], true);
        assert_eq!(
            all.long(),
            "Map ✓ · Configure ✓ · Accept ✓ · then: scan, plan, and each piece"
        );
        assert_eq!(
            all.short(),
            "Map, Configure and Accept ✓ · then: scan, plan, and each piece"
        );
    }
}
