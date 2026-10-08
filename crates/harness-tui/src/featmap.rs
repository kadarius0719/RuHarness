//! The person's features as the cockpit shows them (docs/FEATURES-DESIGN.md
//! §8.1–§8.2): pure, built on the load worker from the snapshot, the file
//! tree's states and the map. Per feature and unit it derives **the unit's
//! result for the feature** from the unit's latest verdict, and from those
//! the feature's state — never from anything the map alone could claim.

use crate::files::Files;
use crate::model::{Snapshot, UnitView};
use harness_core::config::TargetSection;
use harness_core::features::{
    self, FeatureMap, FeatureSnapshot, MapInputs, MapState, ScenarioRecord, SkipReason,
};
use harness_core::plan::UnitStatus;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A `[file, canonical id]` pair.
pub type Pair = (String, String);

/// A unit's result for one feature (§8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitResult {
    /// Every scenario of the feature has a passing check on a fresh verdict
    /// whose digests match today's.
    Passed,
    /// Some scenario of it failed there: the scenario ids.
    Failed(Vec<String>),
    /// Some scenario of it was skipped there, and none failed.
    CouldNotRun(SkipReason),
    /// Such a verdict has no check for some scenario of it.
    Absent,
    /// No verdict, a stale one, or one run under other features or other C.
    NotChecked,
    /// The unit's `replaces` are not all top-level `.c` of the program: its
    /// verdicts skip the features.
    Outside,
}

/// The state of a feature (§8.2), first match wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureState {
    /// F1: some unit's result is `Failed`.
    Failing,
    /// F2: a scenario can never be a check as written.
    ScenarioCannotRun,
    /// F3: a unit with Rust, not outside, is `NotChecked` or `Absent`.
    NeedsRecheck,
    /// F4: no map, the map lacks it, or the map is unreadable.
    NotMapped,
    /// F5: the map is out of date.
    MapOutOfDate,
    /// F6: a scenario's notes are unavailable, or its probe disagreed.
    MapIncomplete,
    /// F7: its functions touch no unit.
    ReachesNoUnit,
    /// F8: every unit it touches has Rust, each `Passed`.
    AllMigrated,
    /// F9: some units it touches have Rust, each `Passed`.
    HoldsSoFar {
        /// Units it touches that have Rust.
        rust: usize,
        /// Units it touches.
        of: usize,
    },
    /// F10: no unit it touches has Rust yet.
    AllC,
    /// F11: none of the above.
    SeeUnits,
}

impl FeatureState {
    /// Its glyph (the Features legend).
    pub fn glyph(&self) -> &'static str {
        match self {
            FeatureState::Failing => "✗",
            FeatureState::ScenarioCannotRun => "⚑",
            FeatureState::NeedsRecheck => "↻",
            FeatureState::NotMapped => "⋯",
            FeatureState::MapOutOfDate => "≃",
            FeatureState::MapIncomplete => "◔",
            FeatureState::ReachesNoUnit => "∅",
            FeatureState::AllMigrated => "✓",
            FeatureState::HoldsSoFar { .. } => "◉",
            FeatureState::AllC => "◌",
            FeatureState::SeeUnits => "·",
        }
    }

    /// Its word.
    pub fn word(&self) -> String {
        match self {
            FeatureState::Failing => "failing".into(),
            FeatureState::ScenarioCannotRun => "a scenario cannot run".into(),
            FeatureState::NeedsRecheck => "needs a re-check".into(),
            FeatureState::NotMapped => "not mapped yet".into(),
            FeatureState::MapOutOfDate => "map out of date".into(),
            FeatureState::MapIncomplete => "map incomplete".into(),
            FeatureState::ReachesNoUnit => "reaches no unit".into(),
            FeatureState::AllMigrated => "all its units migrated".into(),
            FeatureState::HoldsSoFar { rust, of } => {
                format!("holds so far · {rust} of {of} unit{}", plural(*of))
            }
            FeatureState::AllC => "all C".into(),
            FeatureState::SeeUnits => "see its units".into(),
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The map as the Views see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapStatus {
    /// None was made.
    None,
    /// It could not be read: why.
    Unreadable(String),
    /// Current.
    Current,
    /// Out of date: why, in words.
    OutOfDate(Vec<&'static str>),
}

impl MapStatus {
    /// It is current.
    pub fn current(&self) -> bool {
        *self == MapStatus::Current
    }
}

/// One scenario of a feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioView {
    /// Its id.
    pub id: String,
    /// The arguments as the program sees them (`{input}` = the file name).
    pub argv: Vec<String>,
    /// Its input, in words.
    pub input: Option<&'static str>,
    /// The map's record of it, when the map has one.
    pub record: Option<ScenarioRecord>,
    /// Why a unit's latest fresh, current verdict skipped it, if one did.
    pub skipped: Option<SkipReason>,
}

/// One unit a feature's functions touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRow {
    /// The unit's id.
    pub unit: String,
    /// How many of its functions the feature ran.
    pub ran: usize,
    /// How many functions it has (the facts' distinct pairs in its files).
    pub of: usize,
    /// The feature's result on it.
    pub result: UnitResult,
    /// The unit has Rust (verified or merged).
    pub has_rust: bool,
}

/// One feature, derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureView {
    /// Its id.
    pub id: String,
    /// The person's name for it (untrusted: display-filtered when shown).
    pub name: String,
    /// Its state.
    pub state: FeatureState,
    /// A condition below the state worth saying beside it.
    pub also: Option<String>,
    /// Its scenarios.
    pub scenarios: Vec<ScenarioView>,
    /// The units its functions touch, in plan order.
    pub units: Vec<UnitRow>,
    /// Functions it ran outside every unit.
    pub outside_units: usize,
    /// Units outside its functions whose checked verdict failed it.
    pub also_fails: Vec<String>,
    /// The functions only it runs (with at least two features).
    pub specific: Vec<Pair>,
    /// Units it touches with a red verdict (Rust not verified).
    pub red_units: usize,
    /// The units with Rust whose verdict does not cover it yet — what its
    /// "needs a re-check" means (the F3 rule), in plan order (review C3).
    pub recheck: Vec<String>,
    /// How many of its scenarios cannot run (a C-side skip, or a map
    /// record that did not exit or was unstable).
    pub cannot_run: usize,
}

/// How the features touch one unit (§8.4's unit line).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitFeatures {
    /// `(feature id, functions of the unit it ran)`, in file order.
    pub running: Vec<(String, usize)>,
    /// The unit's definitions the map could not watch.
    pub unwatched: usize,
    /// The unit is outside the program the features run.
    pub outside: bool,
}

/// What the features file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    /// There is none.
    NoFile,
    /// It cannot be used: why.
    Invalid(String),
    /// It validated.
    Valid,
}

/// Everything the cockpit shows about the features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureModel {
    /// The file.
    pub group: Group,
    /// The map.
    pub map: MapStatus,
    /// The features, in file order.
    pub features: Vec<FeatureView>,
    /// Per unit id.
    pub by_unit: BTreeMap<String, UnitFeatures>,
    /// Per function: the features that ran it (map current).
    pub by_function: BTreeMap<Pair, Vec<String>>,
    /// Functions some feature ran.
    pub ran: usize,
    /// Functions the map watches.
    pub watched: usize,
    /// Definitions it could not watch.
    pub unwatched: BTreeSet<Pair>,
    /// Why each unwatched function has no note, in plain words.
    pub unwatched_why: BTreeMap<Pair, String>,
    /// Pairs in the map today's facts did not know (dropped).
    pub unknown: usize,
    /// The facts record no single `main()` among the program's files.
    pub no_single_main: bool,
    /// The map is current and every scenario's record is complete (noted,
    /// and the run with notes agreed): only then may a View say what the
    /// features do NOT run (review C5/C6).
    pub complete: bool,
    /// Every unit with Rust some feature needs re-checked, in plan order.
    pub recheck: Vec<String>,
}

impl FeatureModel {
    /// The feature with `id`.
    pub fn feature(&self, id: &str) -> Option<&FeatureView> {
        self.features.iter().find(|f| f.id == id)
    }
}

/// A unit has Rust: its status is verified or merged.
fn has_rust(u: &UnitView) -> bool {
    matches!(u.unit.status, UnitStatus::Verified | UnitStatus::Merged)
}

/// The unit's `replaces` are not all the whole program's own `.c`: the
/// top-level `.c` of `source_dir`, or the listed `.c` of a file-list target.
fn outside(u: &UnitView, target: &TargetSection) -> bool {
    let replaces = u.unit.oracle_param_list("replaces");
    !replaces.is_empty()
        && !replaces.iter().all(|r| {
            target.is_program_file(r)
                && Path::new(r).extension().and_then(|e| e.to_str()) == Some("c")
        })
}

/// What a unit's current verdict says of its place in the program: `Some(
/// false)` when it ran feature checks, `Some(true)` when it skipped them as
/// not in the program, `None` without a verdict that says (then the plan's
/// paths decide).
fn verdict_outside(u: &UnitView, now_features: &str, now_program: &str) -> Option<bool> {
    let v = u.verdict.as_ref()?;
    if !u.report.verdict.stale.is_empty()
        || v.inputs.features != now_features
        || !features::same_program(&v.inputs.program, now_program)
    {
        return None;
    }
    if v.checks
        .iter()
        .any(|c| c.name.starts_with(features::CHECK_PREFIX))
    {
        return Some(false);
    }
    v.inputs
        .features_skipped
        .iter()
        .filter_map(|e| features::parse_skip(e))
        .any(|(_, _, r)| r == SkipReason::NotInProgram)
        .then_some(true)
}

/// The unit's result for feature `id` with scenarios `scenarios` (§8.1).
fn result(
    u: &UnitView,
    id: &str,
    scenarios: &[&str],
    now_features: &str,
    now_program: &str,
    target: &TargetSection,
) -> UnitResult {
    // A current verdict says whether the oracle ran the unit in the program
    // (it compares canonical paths); the plan's paths decide only without
    // one (review O5).
    let not_checked = || {
        if outside(u, target) {
            UnitResult::Outside
        } else {
            UnitResult::NotChecked
        }
    };
    let Some(v) = &u.verdict else {
        return not_checked();
    };
    if !u.report.verdict.stale.is_empty()
        || v.inputs.features != now_features
        || !features::same_program(&v.inputs.program, now_program)
        || now_features == features::INVALID_DIGEST
    {
        return not_checked();
    }
    let prefix = format!("{}{id}/", features::CHECK_PREFIX);
    let mut failed: Vec<String> = v
        .checks
        .iter()
        .filter(|c| !c.passed)
        .filter_map(|c| c.name.strip_prefix(&prefix).map(str::to_string))
        .collect();
    failed.sort();
    failed.dedup();
    if !failed.is_empty() {
        return UnitResult::Failed(failed);
    }
    let skipped = v
        .inputs
        .features_skipped
        .iter()
        .filter_map(|e| features::parse_skip(e))
        .find(|(f, _, _)| f == id)
        .map(|(_, _, reason)| reason);
    if let Some(reason) = skipped {
        return if reason == SkipReason::NotInProgram {
            UnitResult::Outside
        } else {
            UnitResult::CouldNotRun(reason)
        };
    }
    let all = scenarios.iter().all(|s| {
        v.checks
            .iter()
            .any(|c| c.passed && c.name == format!("{prefix}{s}"))
    });
    if all {
        UnitResult::Passed
    } else if outside(u, target) {
        UnitResult::Outside
    } else {
        UnitResult::Absent
    }
}

/// Build the model (§8.1). `now` is today's map inputs (`None` without facts
/// or a features file).
pub fn build(
    snapshot: &Snapshot,
    files: &Files,
    map: &MapState,
    now: Option<&MapInputs>,
) -> FeatureModel {
    let mut model = FeatureModel {
        group: Group::NoFile,
        map: MapStatus::None,
        features: Vec::new(),
        by_unit: BTreeMap::new(),
        by_function: BTreeMap::new(),
        ran: 0,
        watched: 0,
        unwatched: BTreeSet::new(),
        unwatched_why: BTreeMap::new(),
        unknown: 0,
        no_single_main: no_single_main(snapshot),
        complete: false,
        recheck: Vec::new(),
    };
    let (list, now_features) = match &snapshot.features {
        FeatureSnapshot::None => return model,
        FeatureSnapshot::Invalid(why) => {
            // Already named by the open tool's own path.
            model.group = Group::Invalid(why.clone());
            return model;
        }
        FeatureSnapshot::Valid { features, digest } => {
            model.group = Group::Valid;
            (features, digest.clone())
        }
    };
    let now_program = snapshot
        .features_now
        .as_ref()
        .map(|n| n.program.clone())
        .unwrap_or_default();
    let target = &snapshot.target;

    // The map, and whether it is current.
    let loaded: Option<&FeatureMap> = match map {
        MapState::None => None,
        MapState::Unreadable(why) => {
            model.map = MapStatus::Unreadable(why.clone());
            None
        }
        MapState::Loaded { map, unknown } => {
            model.unknown = *unknown;
            model.map = match now {
                Some(now) => {
                    let why = map.out_of_date(now);
                    if why.is_empty() {
                        MapStatus::Current
                    } else {
                        MapStatus::OutOfDate(why)
                    }
                }
                None => MapStatus::OutOfDate(vec!["the scan changed"]),
            };
            Some(map)
        }
    };

    // Functions → files → owning units.
    let owner_of_file: BTreeMap<&str, Option<usize>> = files
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.owner))
        .collect();
    let facts_pairs: BTreeSet<Pair> = snapshot
        .facts
        .as_ref()
        .map(|f| {
            f.symbols
                .iter()
                .map(|s| (s.file.clone(), s.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let unit_of =
        |pair: &Pair| -> Option<usize> { owner_of_file.get(pair.0.as_str()).copied().flatten() };
    let mut unit_size = vec![0usize; snapshot.units.len()];
    for p in &facts_pairs {
        if let Some(u) = unit_of(p) {
            unit_size[u] += 1;
        }
    }
    if let Some(m) = loaded {
        model.unwatched = m.unwatched.iter().cloned().collect();
        model.unwatched_why = m
            .unwatched_reasons
            .iter()
            .map(|r| {
                (
                    (r.file.clone(), r.id.clone()),
                    features::unwatched_words(&r.kind, &r.detail),
                )
            })
            .collect();
        model.watched = facts_pairs.len().saturating_sub(model.unwatched.len());
    }
    let records: BTreeMap<(String, String), &ScenarioRecord> = loaded
        .map(|m| {
            m.scenarios
                .iter()
                .map(|r| ((r.feature.clone(), r.scenario.clone()), r))
                .collect()
        })
        .unwrap_or_default();

    // Which scenarios a checked verdict skipped, and why.
    let mut skips: BTreeMap<(String, String), SkipReason> = BTreeMap::new();
    for u in &snapshot.units {
        let Some(v) = &u.verdict else { continue };
        if !u.report.verdict.stale.is_empty()
            || v.inputs.features != now_features
            || !features::same_program(&v.inputs.program, &now_program)
        {
            continue;
        }
        for (f, sc, reason) in v
            .inputs
            .features_skipped
            .iter()
            .filter_map(|e| features::parse_skip(e))
        {
            if reason != SkipReason::NotInProgram {
                skips.entry((f, sc)).or_insert(reason);
            }
        }
    }

    // Per feature: its functions (from the map), its results per unit.
    let mut per_feature_functions: Vec<BTreeSet<Pair>> = Vec::new();
    for f in &list.features {
        let mut funcs: BTreeSet<Pair> = BTreeSet::new();
        for s in list.scenarios_of(&f.id) {
            if let Some(r) = records.get(&(f.id.clone(), s.id.clone())) {
                funcs.extend(r.functions.iter().cloned());
            }
        }
        per_feature_functions.push(funcs);
    }
    let mut ran_by: BTreeMap<Pair, Vec<String>> = BTreeMap::new();
    for (f, funcs) in list.features.iter().zip(&per_feature_functions) {
        for p in funcs {
            ran_by.entry(p.clone()).or_default().push(f.id.clone());
        }
    }
    // What the last map says features ran — current or not (a View puts
    // it under "From the last map"); the per-function answer only from a
    // current one.
    if loaded.is_some() {
        model.ran = ran_by.len();
    }
    model.complete = model.map.current()
        && list.scenarios.iter().all(|s| {
            records
                .get(&(s.feature.clone(), s.id.clone()))
                .is_some_and(|r| r.noted == "complete" && r.probe_agrees)
        });
    if model.map.current() {
        model.by_function = ran_by.clone();
    }

    for (f, funcs) in list.features.iter().zip(&per_feature_functions) {
        let scenario_ids: Vec<&str> = list.scenarios_of(&f.id).map(|s| s.id.as_str()).collect();
        let results: Vec<UnitResult> = snapshot
            .units
            .iter()
            .map(|u| result(u, &f.id, &scenario_ids, &now_features, &now_program, target))
            .collect();
        // Which units its functions touch.
        let mut ran_in: BTreeMap<usize, usize> = BTreeMap::new();
        let mut outside_units = 0;
        for p in funcs {
            match unit_of(p) {
                Some(u) => *ran_in.entry(u).or_default() += 1,
                None => outside_units += 1,
            }
        }
        let units: Vec<UnitRow> = ran_in
            .iter()
            .map(|(&u, &ran)| UnitRow {
                unit: snapshot.units[u].unit.id.clone(),
                ran,
                of: unit_size[u],
                result: results[u].clone(),
                has_rust: has_rust(&snapshot.units[u]),
            })
            .collect();
        // Failing units the map does not show it touching (with no current,
        // complete map: every failing unit — the View words it neutrally).
        let also_fails: Vec<String> = results
            .iter()
            .enumerate()
            .filter(|(u, r)| matches!(r, UnitResult::Failed(_)) && !ran_in.contains_key(u))
            .map(|(u, _)| snapshot.units[u].unit.id.clone())
            .collect();
        let specific: Vec<Pair> = if list.features.len() >= 2 {
            funcs
                .iter()
                .filter(|p| ran_by.get(*p).is_some_and(|v| v.len() == 1))
                .take(20)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let red_units = ran_in
            .keys()
            .filter(|&&u| snapshot.units[u].verdict.as_ref().is_some_and(|v| !v.green))
            .count();

        // The scenarios, with what the map and the verdicts say of them.
        let scenarios: Vec<ScenarioView> = list
            .scenarios_of(&f.id)
            .map(|s| ScenarioView {
                id: s.id.clone(),
                argv: s.argv(),
                input: s.input.map(|i| i.words()),
                record: records
                    .get(&(f.id.clone(), s.id.clone()))
                    .map(|r| (*r).clone()),
                skipped: skips.get(&(f.id.clone(), s.id.clone())).copied(),
            })
            .collect();

        // The state, first match wins (§8.2).
        let c_side_skip = results
            .iter()
            .any(|r| matches!(r, UnitResult::CouldNotRun(reason) if reason.is_c_side()));
        let map_unusable = model.map.current()
            && scenarios.iter().any(|s| {
                s.record
                    .as_ref()
                    .is_some_and(|r| !r.stable || !r.end.starts_with("exit "))
            });
        let recheck_units: Vec<String> = snapshot
            .units
            .iter()
            .zip(&results)
            .filter(|(u, r)| {
                has_rust(u) && matches!(r, UnitResult::NotChecked | UnitResult::Absent)
            })
            .map(|(u, _)| u.unit.id.clone())
            .collect();
        let recheck = !recheck_units.is_empty();
        let cannot_run = scenarios
            .iter()
            .filter(|s| {
                s.skipped.is_some_and(|r| r.is_c_side())
                    || (model.map.current()
                        && s.record
                            .as_ref()
                            .is_some_and(|r| !r.stable || !r.end.starts_with("exit ")))
            })
            .count();
        let feature_mapped = scenarios.iter().all(|s| s.record.is_some());
        let incomplete = scenarios.iter().any(|s| {
            s.record
                .as_ref()
                .is_some_and(|r| r.noted != "complete" || !r.probe_agrees)
        });
        let touched: Vec<usize> = ran_in
            .keys()
            .copied()
            .filter(|&u| results[u] != UnitResult::Outside)
            .collect();
        let with_rust: Vec<usize> = touched
            .iter()
            .copied()
            .filter(|&u| has_rust(&snapshot.units[u]))
            .collect();
        let all_pass = with_rust.iter().all(|&u| results[u] == UnitResult::Passed);
        let failing = results.iter().any(|r| matches!(r, UnitResult::Failed(_)));
        let state = if failing {
            FeatureState::Failing
        } else if c_side_skip || map_unusable {
            FeatureState::ScenarioCannotRun
        } else if recheck {
            FeatureState::NeedsRecheck
        } else if matches!(model.map, MapStatus::None | MapStatus::Unreadable(_)) || !feature_mapped
        {
            FeatureState::NotMapped
        } else if !model.map.current() {
            FeatureState::MapOutOfDate
        } else if incomplete {
            FeatureState::MapIncomplete
        } else if ran_in.is_empty() {
            FeatureState::ReachesNoUnit
        } else if !touched.is_empty() && with_rust.len() == touched.len() && all_pass {
            FeatureState::AllMigrated
        } else if !with_rust.is_empty() && all_pass {
            FeatureState::HoldsSoFar {
                rust: with_rust.len(),
                of: touched.len(),
            }
        } else if with_rust.is_empty() && !touched.is_empty() {
            FeatureState::AllC
        } else {
            FeatureState::SeeUnits
        };
        let also = match state {
            FeatureState::Failing if c_side_skip || map_unusable => Some(format!(
                "{} scenario{} cannot run",
                cannot_run.max(1),
                plural(cannot_run.max(1))
            )),
            FeatureState::HoldsSoFar { .. } | FeatureState::AllC if red_units > 0 => Some(format!(
                "{red_units} unit{} with a red verdict",
                plural(red_units)
            )),
            _ => None,
        };
        model.features.push(FeatureView {
            id: f.id.clone(),
            name: f.name.clone(),
            state,
            also,
            scenarios,
            units,
            outside_units,
            also_fails,
            specific,
            red_units,
            recheck: recheck_units,
            cannot_run,
        });
    }
    for u in &snapshot.units {
        if model
            .features
            .iter()
            .any(|f| f.recheck.contains(&u.unit.id))
        {
            model.recheck.push(u.unit.id.clone());
        }
    }

    // Per unit: which features run its functions.
    for (i, u) in snapshot.units.iter().enumerate() {
        // The verdict first, as for the results (fix check N4).
        let mut entry = UnitFeatures {
            outside: verdict_outside(u, &now_features, &now_program)
                .unwrap_or_else(|| outside(u, target)),
            ..UnitFeatures::default()
        };
        for (f, funcs) in list.features.iter().zip(&per_feature_functions) {
            let n = funcs.iter().filter(|p| unit_of(p) == Some(i)).count();
            if n > 0 {
                entry.running.push((f.id.clone(), n));
            }
        }
        entry.unwatched = model
            .unwatched
            .iter()
            .filter(|p| unit_of(p) == Some(i))
            .count();
        model.by_unit.insert(u.unit.id.clone(), entry);
    }
    model
}

/// The facts record no single `main` among the program's own files (the
/// top-level `.c` of `source_dir`, or the listed files; advisory: the link
/// decides).
fn no_single_main(snapshot: &Snapshot) -> bool {
    let Some(facts) = &snapshot.facts else {
        return false;
    };
    let files: BTreeSet<&str> = facts
        .symbols
        .iter()
        .filter(|s| s.name == "main" && snapshot.target.is_program_file(&s.file))
        .map(|s| s.file.as_str())
        .collect();
    files.len() != 1
}

#[cfg(all(test, feature = "tui"))]
mod tests {
    use super::*;
    use crate::testutil::{scratch_target_without_features, TmpDir};
    use harness_core::features::ScenarioRecord;
    use harness_core::verdict::{Check, Verdict};

    const FEATURES: &str = "schema_version = 1\n\
[[feature]]\nid = \"gzip\"\nname = \"Compress to gzip\"\n\
[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\
[[feature]]\nid = \"inline\"\nname = \"Only header code\"\n\
[[scenario]]\nfeature = \"gzip\"\nid = \"text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"sample:text\"\n\
[[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n\
[[scenario]]\nfeature = \"inline\"\nid = \"x\"\nargs = [\"-x\"]\n";

    const U001: &str = "u001-katajainen";

    /// A file-list tool's program is its listed files, wherever their
    /// folders: a unit replacing a listed `.c` is in it, one replacing a
    /// file the list leaves out is outside; its one listed `main` is the
    /// program's.
    #[test]
    fn a_file_list_tools_listed_files_are_the_program() {
        use crate::testutil::{file_list_tool, LZG_TOOL};
        let dir = file_list_tool("featmap-file-list");
        let read = crate::load::read_tool(&dir.0, Some(LZG_TOOL)).expect("reads");
        let mut snap = read.snapshot.clone();
        assert!(!no_single_main(&snap));
        let at = snap
            .units
            .iter()
            .position(|u| u.unit.id == "u-encode")
            .unwrap();
        assert_eq!(
            snap.units[at].unit.oracle_param_list("replaces"),
            ["src/lib/encode.c"]
        );
        assert!(!outside(&snap.units[at], &snap.target));
        let replaces = snap.units[at]
            .unit
            .oracle
            .as_mut()
            .and_then(|t| t.get_mut("replaces"))
            .and_then(|v| v.as_array_mut())
            .unwrap();
        replaces[0] = "src/other/decode.c".into();
        assert!(outside(&snap.units[at], &snap.target));
    }

    struct Fx {
        _dir: TmpDir,
        root: std::path::PathBuf,
    }

    fn zopfli(tag: &str) -> Fx {
        let root = scratch_target_without_features("targets/zopfli", &format!("featmap-{tag}"));
        Fx {
            _dir: TmpDir(root.clone()),
            root,
        }
    }

    impl Fx {
        fn write_features(&self, text: &str) {
            let dir = self.root.join("migration/features");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("features.toml"), text).unwrap();
        }

        fn read(&self) -> crate::load::Read {
            harness_core::adopt::testing::adopt(&self.root);
            crate::load::read(&self.root).expect("reads")
        }

        fn model(&self) -> FeatureModel {
            let read = self.read();
            let files = crate::files::build(&read.snapshot, &read.walk);
            build(&read.snapshot, &files, &read.map, read.map_now.as_ref())
        }

        /// A map whose inputs are today's, with `functions` per scenario.
        fn write_map(
            &self,
            functions: &[(&str, &str, Vec<Pair>)],
            tweak: impl Fn(&mut FeatureMap),
        ) {
            let read = self.read();
            let now = read.map_now.expect("today's inputs");
            let mut map = FeatureMap {
                schema: features::MAP_SCHEMA_NAME.into(),
                schema_version: features::MAP_SCHEMA_VERSION,
                inputs: now,
                unwatched: Vec::new(),
                unwatched_reasons: Vec::new(),
                scenarios: functions
                    .iter()
                    .map(|(f, s, funcs)| ScenarioRecord {
                        feature: f.to_string(),
                        scenario: s.to_string(),
                        end: "exit 0".into(),
                        stdout_bytes: 1,
                        stderr_bytes: 0,
                        stderr_head: String::new(),
                        stable: true,
                        probe_agrees: true,
                        noted: "complete".into(),
                        reason: None,
                        functions: funcs.clone(),
                    })
                    .collect(),
            };
            tweak(&mut map);
            std::fs::write(
                features::map_path(&harness_core::ledger::Ledger::new(&self.root)),
                map.to_bytes().unwrap(),
            )
            .unwrap();
        }

        /// u001's verdict, rewritten as if run under today's features with
        /// `checks` (name, passed) and `skipped`.
        fn verdict(&self, checks: &[(&str, bool)], skipped: &[&str]) {
            let read = self.read();
            let now = read.snapshot.features_now.clone().expect("now");
            let path = self
                .root
                .join(format!("migration/units/{U001}/oracle-latest.json"));
            let mut v = Verdict::load(&path).unwrap();
            v.inputs.features = now.features;
            v.inputs.program = now.program;
            v.inputs.features_skipped = skipped.iter().map(|s| s.to_string()).collect();
            v.checks.retain(|c| !c.name.starts_with("feature:"));
            v.checks.extend(checks.iter().map(|(n, p)| Check {
                name: n.to_string(),
                passed: *p,
                detail: String::new(),
            }));
            v.green = v.checks.iter().all(|c| c.passed);
            v.store(&path).unwrap();
        }
    }

    fn pair(file: &str, name: &str) -> Pair {
        (file.to_string(), name.to_string())
    }

    fn kata() -> Pair {
        pair("src/zopfli/katajainen.c", "ZopfliLengthLimitedCodeLengths")
    }

    fn main_fn() -> Pair {
        pair("src/zopfli/zopfli_bin.c", "main")
    }

    fn inline() -> Pair {
        pair(
            "src/zopfli/symbols.h",
            "src/zopfli/symbols.h::ZopfliGetDistSymbol",
        )
    }

    fn state(m: &FeatureModel, id: &str) -> FeatureState {
        m.feature(id).expect(id).state.clone()
    }

    fn standard_map(fx: &Fx) {
        fx.write_map(
            &[
                ("gzip", "text", vec![main_fn(), kata()]),
                ("help", "flag", vec![main_fn()]),
                ("inline", "x", vec![inline()]),
            ],
            |_| {},
        );
    }

    #[test]
    fn no_file_and_an_invalid_file_are_groups_not_errors() {
        let fx = zopfli("nofile");
        assert_eq!(fx.model().group, Group::NoFile);
        fx.write_features("schema_version = 1\nnope = 1\n");
        let m = fx.model();
        assert!(matches!(&m.group, Group::Invalid(why) if why.contains("unknown key \"nope\"")));
        assert!(m.features.is_empty());
    }

    #[test]
    fn a_unit_with_rust_not_checked_on_the_features_needs_a_re_check_first() {
        let fx = zopfli("recheck");
        fx.write_features(FEATURES);
        // u001 is verified; its committed verdict predates the features.
        let m = fx.model();
        for id in ["gzip", "help", "inline"] {
            assert_eq!(state(&m, id), FeatureState::NeedsRecheck, "{id}");
        }
        assert_eq!(m.map, MapStatus::None);
    }

    /// The snapshot's `[target]` with the folder form at `dir`.
    fn folder_at(snap: &Snapshot, dir: &str) -> TargetSection {
        TargetSection {
            name: snap.target.name.clone(),
            form: harness_core::config::Form::Folder(harness_core::config::FolderForm {
                source_dir: dir.into(),
                include_dirs: Vec::new(),
            }),
        }
    }

    /// Review M1/O5: a `source_dir` of `.` is the root — every top-level
    /// `.c` is in the program, and its `main` is the program's.
    #[test]
    fn a_source_dir_of_dot_is_the_root() {
        let fx = zopfli("dot");
        fx.write_features(FEATURES);
        let read = fx.read();
        let mut snap = read.snapshot.clone();
        let strip = |s: &str| s.strip_prefix("src/zopfli/").unwrap_or(s).to_string();
        snap.target = folder_at(&snap, ".");
        for sym in &mut snap.facts.as_mut().unwrap().symbols {
            sym.file = strip(&sym.file);
        }
        for u in &mut snap.units {
            let replaces = u
                .unit
                .oracle
                .as_mut()
                .and_then(|t| t.get_mut("replaces"))
                .and_then(|v| v.as_array_mut());
            for v in replaces.into_iter().flatten() {
                *v = strip(v.as_str().unwrap()).into();
            }
        }
        assert!(!no_single_main(&snap));
        let u001 = snap.units.iter().find(|u| u.unit.id == U001).unwrap();
        assert_eq!(u001.unit.oracle_param_list("replaces"), ["katajainen.c"]);
        assert!(!outside(u001, &folder_at(&snap, ".")));
        assert!(!outside(u001, &folder_at(&snap, "./")));
        assert!(outside(u001, &folder_at(&snap, "src")));
        let files = crate::files::build(&snap, &read.walk);
        let m = build(&snap, &files, &read.map, read.map_now.as_ref());
        assert_eq!(state(&m, "gzip"), FeatureState::NeedsRecheck, "u001 counts");
    }

    #[test]
    fn a_current_verdict_says_whether_the_unit_ran_in_the_program() {
        let fx = zopfli("verdict-inside");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(&[("feature:help/flag", false)], &[]);
        let read = fx.read();
        let mut snap = read.snapshot.clone();
        // The plan's path no longer reads as a top-level `.c` (a symlinked
        // one, say), but the oracle ran the unit: its failure shows.
        snap.target = folder_at(&snap, "elsewhere");
        let files = crate::files::build(&snap, &read.walk);
        let m = build(&snap, &files, &read.map, read.map_now.as_ref());
        assert_eq!(state(&m, "help"), FeatureState::Failing);
        // The unit's own line says the same (fix check N4).
        assert!(!m.by_unit[U001].outside);
    }

    #[test]
    fn the_states_follow_the_units_results() {
        let fx = zopfli("states");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(
            &[
                ("feature:gzip/text", true),
                ("feature:help/flag", true),
                ("feature:inline/x", true),
            ],
            &[],
        );
        let m = fx.model();
        assert!(m.map.current(), "{:?}", m.map);
        // gzip runs u001 (Rust, passed) and u-zopfli_bin (C).
        match state(&m, "gzip") {
            FeatureState::HoldsSoFar { rust, of } => assert_eq!((rust, of), (1, 2)),
            other => panic!("{other:?}"),
        }
        let gzip = m.feature("gzip").unwrap();
        let row = gzip.units.iter().find(|r| r.unit == U001).unwrap();
        assert_eq!(row.result, UnitResult::Passed);
        assert_eq!((row.ran, row.of), (1, row.of));
        assert_eq!(state(&m, "help"), FeatureState::AllC);
        assert_eq!(state(&m, "inline"), FeatureState::ReachesNoUnit);
        assert_eq!(gzip.specific, [kata()], "only gzip runs katajainen");
        let u = &m.by_unit[U001];
        assert_eq!(u.running, [("gzip".to_string(), 1)]);
        assert_eq!(m.by_function[&main_fn()], ["gzip", "help"]);
    }

    #[test]
    fn a_failure_shows_first_even_on_a_unit_the_feature_does_not_run() {
        let fx = zopfli("failing");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(
            &[
                ("feature:gzip/text", true),
                ("feature:help/flag", false),
                ("feature:inline/x", true),
            ],
            &[],
        );
        let m = fx.model();
        assert_eq!(state(&m, "help"), FeatureState::Failing);
        assert_eq!(m.feature("help").unwrap().also_fails, [U001]);
        // F1 before F2: a failure shows even when a scenario also cannot
        // run (the map found it unstable), said beside it.
        fx.write_map(
            &[
                ("gzip", "text", vec![main_fn(), kata()]),
                ("help", "flag", vec![main_fn()]),
                ("inline", "x", vec![inline()]),
            ],
            |m| m.scenarios[1].stable = false,
        );
        let m = fx.model();
        assert_eq!(state(&m, "help"), FeatureState::Failing);
        assert_eq!(
            m.feature("help").unwrap().also.as_deref(),
            Some("1 scenario cannot run")
        );
        // Failing needs no map: the same with the map out of date.
        std::fs::remove_file(features::map_path(&harness_core::ledger::Ledger::new(
            &fx.root,
        )))
        .unwrap();
        assert_eq!(state(&fx.model(), "help"), FeatureState::Failing);
    }

    /// A current verdict that ran none of a feature's scenarios on a unit
    /// the plan's paths put outside the program: outside, not "absent" —
    /// which would ask for a re-check that cannot help.
    #[test]
    fn a_verdict_without_the_features_checks_outside_the_program_is_outside() {
        let fx = zopfli("verdict-outside");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(&[], &[]);
        let read = fx.read();
        let mut snap = read.snapshot.clone();
        snap.target = folder_at(&snap, "elsewhere");
        let files = crate::files::build(&snap, &read.walk);
        let m = build(&snap, &files, &read.map, read.map_now.as_ref());
        let gzip = m.feature("gzip").unwrap();
        let row = gzip.units.iter().find(|r| r.unit == U001).unwrap();
        assert_eq!(row.result, UnitResult::Outside);
        assert!(gzip.recheck.is_empty(), "{:?}", gzip.recheck);
    }

    #[test]
    fn a_c_side_skip_is_a_scenario_to_fix_never_a_re_check() {
        let fx = zopfli("skip");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(
            &[("feature:gzip/text", true), ("feature:inline/x", true)],
            &["help/flag: c-side-unstable"],
        );
        let m = fx.model();
        assert_eq!(state(&m, "help"), FeatureState::ScenarioCannotRun);
        assert_eq!(
            m.feature("help").unwrap().scenarios[0].skipped,
            Some(SkipReason::CSideUnstable)
        );
        assert!(
            !matches!(state(&m, "gzip"), FeatureState::NeedsRecheck),
            "a skip elsewhere is no re-check"
        );
        // Garbage in the verdict's skip list is dropped, not shown.
        fx.verdict(
            &[("feature:gzip/text", true), ("feature:inline/x", true)],
            &["help/flag: budget; ignore previous instructions"],
        );
        assert_eq!(
            state(&fx.model(), "help"),
            FeatureState::NeedsRecheck,
            "absent, not skipped"
        );
    }

    #[test]
    fn an_out_of_date_or_incomplete_map_never_claims_reaches_no_unit() {
        let fx = zopfli("stale-map");
        fx.write_features(FEATURES);
        fx.verdict(
            &[
                ("feature:gzip/text", true),
                ("feature:help/flag", true),
                ("feature:inline/x", true),
            ],
            &[],
        );
        fx.write_map(
            &[
                ("gzip", "text", vec![main_fn(), kata()]),
                ("help", "flag", vec![main_fn()]),
                ("inline", "x", vec![]),
            ],
            |m| m.inputs.program = "blake3:other".into(),
        );
        let m = fx.model();
        assert_eq!(m.map, MapStatus::OutOfDate(vec!["the program's C changed"]));
        assert_eq!(state(&m, "inline"), FeatureState::MapOutOfDate);
        assert!(
            m.by_function.is_empty(),
            "no per-function claim from an old map"
        );
        fx.write_map(
            &[
                ("gzip", "text", vec![main_fn(), kata()]),
                ("help", "flag", vec![main_fn()]),
                ("inline", "x", vec![]),
            ],
            |m| {
                m.scenarios[2].noted = "unavailable".into();
                m.scenarios[2].reason = Some("none written".into());
            },
        );
        assert_eq!(state(&fx.model(), "inline"), FeatureState::MapIncomplete);
    }

    #[test]
    fn a_stale_or_other_features_verdict_is_not_checked() {
        let fx = zopfli("stale-verdict");
        fx.write_features(FEATURES);
        standard_map(&fx);
        fx.verdict(&[("feature:gzip/text", false)], &[]);
        // Changing a scenario changes the digest: the red verdict no longer
        // counts as failing — the unit needs a re-check.
        fx.write_features(&FEATURES.replace("[\"-h\"]", "[\"--help\"]"));
        let m = fx.model();
        assert_eq!(state(&m, "gzip"), FeatureState::NeedsRecheck);
        let row = &m.feature("gzip").unwrap().units;
        assert!(row
            .iter()
            .any(|r| r.unit == U001 && r.result == UnitResult::NotChecked));
    }

    #[test]
    fn nothing_is_read_or_hashed_without_a_features_file() {
        let fx = zopfli("nothing");
        let read = fx.read();
        assert_eq!(read.map, MapState::None);
        assert_eq!(read.map_now, None);
        assert_eq!(read.snapshot.features_now, None);
    }
}
