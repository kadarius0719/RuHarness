//! The cockpit's Speed (docs/PERF-DESIGN.md §3.11): the group's state, every
//! stored row's words — the same `harness-core` words the CLI prints — with
//! why it is out of date, the units ordered worst first, the View's header,
//! a unit's header line and the project summary's line. Built from the
//! snapshot alone: nothing here starts a process.

use crate::model::{short_id, ProvenanceView, Snapshot, UnitView};
use crate::perfread::InputNow;
use harness_core::perf::currency::{self, Today};
use harness_core::perf::estimate::{self, Job};
use harness_core::perf::results::{self, Difference, Row, RowKind};
use harness_core::perf::words::{self as words, RowWords, Side};
use harness_core::perf::workloads::WorkloadsState;
use std::collections::BTreeMap;

/// The Speed group's state (its tree label, build note 28).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    /// No workloads file.
    NoFile,
    /// A file with no workload (the starter).
    NoWorkload,
    /// A file with an error, in its words.
    FileError(String),
    /// Workloads, nothing measured yet.
    NotYetRun,
    /// Only the C alone measured.
    COnly,
    /// `measured` of the `of` verified units have rows.
    Units {
        /// Units with rows.
        measured: usize,
        /// Units measurable now (at least `measured`).
        of: usize,
    },
}

/// Which side a row measures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideKey {
    /// The C alone.
    C,
    /// The program as it stands.
    AsItStands,
    /// A unit.
    Unit(String),
}

/// One stored row, worded.
#[derive(Debug, Clone)]
pub struct SpeedRow {
    /// Its workload.
    pub workload: String,
    /// Its outcome.
    pub outcome: String,
    /// Its words.
    pub words: RowWords,
    /// Why it is out of date; empty when current as far as the cockpit can
    /// tell (the computer and the compilers are not checked here).
    pub out_of_date: Vec<String>,
    /// The same reasons' tokens (`harness_core::perf::currency::REASONS`).
    pub out_of_date_tokens: Vec<&'static str>,
    /// The computer and compilers the row records, in words ("measured on
    /// Apple M3, 15.6 24G84, with rustc 1.94.1 and Apple clang …") — what
    /// the View's "see each row" points to; not checked here.
    pub measured_on: String,
    /// Where the outputs differed: the row's own (behaves-differently), or
    /// the one found before that a later measure did not clear.
    pub difference: Option<Difference>,
    /// `difference` is the one found before.
    pub found_before: bool,
    /// The stored row itself (its runs, metrics and numbers).
    pub row: Row,
}

impl SpeedRow {
    /// The unit's (or any held unit's) Rust changed since the row.
    fn rust_changed(&self) -> bool {
        self.out_of_date
            .iter()
            .any(|w| w.ends_with("'s Rust changed since"))
    }

    /// Measured with the same output as the C, and current.
    fn same_output_now(&self) -> bool {
        self.out_of_date.is_empty() && matches!(self.outcome.as_str(), "measured" | "too-short")
    }

    /// Why a difference may be old, in words (empty when it is the row's
    /// own and current).
    fn difference_age(&self) -> String {
        if self.rust_changed() {
            " — found before the unit's Rust changed — measure this unit again to check".into()
        } else if self.found_before {
            " — found before; the last measure ended another way — measure again to check".into()
        } else if !self.out_of_date.is_empty() {
            format!(
                " — found before ({}) — measure again to check",
                self.out_of_date.join(", ")
            )
        } else {
            String::new()
        }
    }
}

/// What a unit's Speed rows ask of the person (§3.11).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Advice {
    /// Each difference, a fact in words.
    pub differences: Vec<String>,
    /// The next step: after a difference, "Compare the outputs; then …";
    /// on a slower row, "perf times the Rust in use. …".
    pub next: Option<String>,
    /// How to change the unit's Rust, from where it came from.
    pub change: Option<String>,
}

/// How to change a unit's Rust, from its provenance (§3.11 *A slower row's
/// next step*, build notes 22 and 30): Modify the model's attempt and
/// Replace (and back); Hand edit the crate and Replace (and back) — a hand
/// edit alone is recorded, never accepted, so perf would time the same
/// crate —; or, for code the cockpit did not record, commit, edit, verify.
/// Each act named changes the crate perf measures (the unit's verified
/// crate). Without a provider Modify is greyed, in the cockpit's own words.
pub fn change_words(unit: &UnitView, has_provider: bool) -> String {
    let id = &unit.unit.id;
    let model_made = match &unit.provenance {
        ProvenanceView::Pipeline(a) | ProvenanceView::Steered(a) | ProvenanceView::Chat(a) => {
            Some(a.clone())
        }
        // Several attempts share the crate: the lowest id is named.
        ProvenanceView::Ambiguous(ids) => ids.iter().min().cloned(),
        _ => None,
    };
    if let Some(a) = model_made {
        let a = short_id(&a);
        let modify = format!(
            "Modify {a} with a note about speed (give these numbers), then Replace {id}'s \
             verified crate with the new attempt, measure this unit again — and if it is not \
             faster, Replace it back with {a}"
        );
        return if has_provider {
            modify
        } else {
            format!(
                "Modify is greyed: {} — with one, {modify}",
                crate::model::NO_PROVIDER
            )
        };
    }
    let in_use = match &unit.provenance {
        ProvenanceView::Human { attempt, .. } => Some(short_id(attempt)),
        _ => None,
    };
    let hand_editable = unit
        .crate_dir
        .as_ref()
        .is_some_and(|d| d.join("src/logic.rs").is_file() && d.join("src/ffi.rs").is_file());
    if let Some(a) = in_use.filter(|_| hand_editable) {
        return format!(
            "Hand edit {id}'s crate, then Replace {id}'s verified crate with the new attempt and \
             measure this unit again — and if it is not faster, Replace it back with {a}"
        );
    }
    format!(
        "Commit the unit's crate first (git) — replacing it deletes it; edit it in your editor, \
         then run harness verify {id} in a terminal (the cockpit cannot Re-check code it did not \
         record), then measure again"
    )
}

/// A unit's rows.
#[derive(Debug, Clone)]
pub struct UnitSpeed {
    /// The unit.
    pub id: String,
    /// Its rows, worst first, out-of-date rows last.
    pub rows: Vec<SpeedRow>,
}

/// Speed, built from a snapshot.
#[derive(Debug, Clone)]
pub struct SpeedModel {
    /// The group's state.
    pub group: Group,
    /// The C alone's rows, in workload order.
    pub c_rows: Vec<SpeedRow>,
    /// The program as it stands's rows, in workload order.
    pub program_rows: Vec<SpeedRow>,
    /// Units the program as it stands holds / leaves out (by its rows).
    pub held: Vec<String>,
    /// The verified units it left out, with their reasons.
    pub left_out: Vec<(String, String)>,
    /// Each measured unit, worst first.
    pub units: Vec<UnitSpeed>,
    /// Results files that could not be read, in words.
    pub errors: Vec<String>,
    /// Units no longer in the plan that still have a results file (the
    /// first names).
    pub orphans: Vec<String>,
    /// How many more such files there are.
    pub orphans_more: usize,
    /// The View's header lines (computers and compilers the rows record).
    pub header: Vec<String>,
    /// A perf run holds the lock now.
    pub measuring: bool,
    /// The units perf would measure now (verified or merged, no interrupted
    /// Accept, verdict green and fresh, the plan's `replaces` still the
    /// verdict's, a crate folder: [`left_out_today`]), in plan order.
    pub measurable: Vec<String>,
    /// Each workload's id and runs a side, in file order.
    pub workloads: Vec<(String, u32)>,
    /// The C's clock time on each workload, in seconds, from its stored
    /// C-alone row (for the estimate).
    pub c_clock: BTreeMap<String, f64>,
    /// Why perf cannot measure, in the words `perf run` refuses with (the
    /// workloads file's state, §3.1): `None` when it can.
    pub blocker: Option<String>,
}

impl SpeedModel {
    /// The tree's label, at most 19 columns (§3.11, build note 28).
    pub fn label(&self) -> String {
        match &self.group {
            Group::NoFile => "Speed (no file)".into(),
            Group::NoWorkload => "Speed (no workload)".into(),
            Group::FileError(_) => "Speed (file error)".into(),
            Group::NotYetRun => "Speed (not yet run)".into(),
            Group::COnly => "Speed (C only)".into(),
            Group::Units { measured, of } => format!("Speed ({measured} of {of})"),
        }
    }

    /// The workloads whose `side` rows ask to be measured again with 31
    /// runs (could not tell, "probably", a close call), in file order.
    pub fn more_runs(&self, side: &SideKey) -> Vec<String> {
        let rows: &[SpeedRow] = match side {
            SideKey::C => &[],
            SideKey::AsItStands => &self.program_rows,
            SideKey::Unit(id) => self.unit(id).map_or(&[][..], |u| &u.rows),
        };
        self.workloads
            .iter()
            .map(|(id, _)| id)
            .filter(|id| {
                rows.iter()
                    .any(|r| r.workload == **id && r.words.offers_more_runs)
            })
            .cloned()
            .collect()
    }

    /// What a perf command over `only` (all workloads when `None`) at
    /// `runs` (each workload's own when `None`) measures, for its estimate.
    pub fn job(
        &self,
        only: Option<&[String]>,
        runs: Option<u32>,
        c_alone: bool,
        rows: u32,
        crates: u32,
        links: u32,
    ) -> Job {
        Job {
            workloads: self
                .workloads
                .iter()
                .filter(|(id, _)| only.is_none_or(|o| o.contains(id)))
                .map(|(id, n)| (runs.unwrap_or(*n), self.c_clock.get(id).copied()))
                .collect(),
            c_alone,
            rows,
            crates,
            links,
        }
    }

    /// What `unit`'s rows ask of the person: each difference as a fact
    /// and its next step, or a slower row's next step; and how to change
    /// the unit's Rust. `has_provider`: a model can Modify.
    pub fn advice(&self, unit: &UnitView, has_provider: bool) -> Advice {
        let id = &unit.unit.id;
        let mut advice = Advice::default();
        let Some(u) = self.unit(id) else {
            return advice;
        };
        for r in &u.rows {
            let Some(d) = &r.difference else {
                continue;
            };
            advice.differences.push(format!(
                "With {id}'s Rust the program {} on {} — verify does not run this workload{}",
                words::difference_words(d),
                r.workload,
                r.difference_age()
            ));
        }
        if !advice.differences.is_empty() {
            advice.next = Some(
                "Compare the outputs; then change the unit's Rust (below) and measure this unit \
                 again"
                    .into(),
            );
        } else if u.rows.iter().any(|r| {
            r.out_of_date.is_empty() && matches!(r.words.answer, "slower" | "probably-slower")
        }) {
            advice.next = Some(
                "perf times the Rust in use. If speed matters here — note these numbers first (or \
                 commit migration/perf/): measuring again replaces them —"
                    .into(),
            );
        }
        if advice.next.is_some() {
            advice.change = Some(change_words(unit, has_provider));
        }
        advice
    }

    /// The workloads on which `side`'s rows hold a difference whose outputs
    /// were kept, in the rows' order.
    pub fn comparable(&self, side: &SideKey) -> Vec<String> {
        let rows: &[SpeedRow] = match side {
            SideKey::C => &[],
            SideKey::AsItStands => &self.program_rows,
            SideKey::Unit(id) => self.unit(id).map_or(&[][..], |u| &u.rows),
        };
        rows.iter()
            .filter(|r| r.difference.as_ref().is_some_and(|d| !d.kept.is_empty()))
            .map(|r| r.workload.clone())
            .collect()
    }

    /// The row of `side` on `workload`.
    pub fn row(&self, side: &SideKey, workload: &str) -> Option<&SpeedRow> {
        let rows: &[SpeedRow] = match side {
            SideKey::C => &self.c_rows,
            SideKey::AsItStands => &self.program_rows,
            SideKey::Unit(id) => self.unit(id).map_or(&[][..], |u| &u.rows),
        };
        rows.iter().find(|r| r.workload == workload)
    }

    /// The program as it stands's differences, each a fact for its heading
    /// and the summary, with what tells which unit it is: "no unit's Rust
    /// differs alone" only when every held unit's own row on that workload
    /// is current and measured the same; else the commands that would
    /// measure the missing ones.
    pub fn program_differences(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for r in &self.program_rows {
            let Some(d) = &r.difference else {
                continue;
            };
            let fact = format!(
                "With the program as it stands ({}) the program {} on {}{}",
                self.held.join(", "),
                words::difference_words(d),
                r.workload,
                r.difference_age()
            );
            let missing: Vec<&String> = self
                .held
                .iter()
                .filter(|id| {
                    !self
                        .row(&SideKey::Unit((*id).clone()), &r.workload)
                        .is_some_and(SpeedRow::same_output_now)
                })
                .collect();
            let which = if missing.is_empty() {
                "no unit's Rust differs alone — it is how they work together".to_string()
            } else {
                format!(
                    "to find which unit, measure each alone: {}",
                    missing
                        .iter()
                        .map(|id| format!("harness perf run --unit {id} --workload {}", r.workload))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            };
            out.push((fact, which));
        }
        out
    }

    /// A unit's rows.
    pub fn unit(&self, id: &str) -> Option<&UnitSpeed> {
        self.units.iter().find(|u| u.id == id)
    }

    /// A unit's header (§3.11, build note 16): `Speed: <short> on
    /// <workload>` — the short form without its interval or "· parallel" —
    /// and, on its own line, the interval (whether or not the 26-column
    /// short form had room for it), "parallel" when the row uses several
    /// cores, "k of n workloads" and "out of date" — each within 54 columns.
    pub fn unit_header(&self, id: &str) -> Option<(String, String)> {
        let u = self.unit(id)?;
        let worst = u.rows.first()?;
        let short = worst.words.short.as_str();
        let short = short.strip_suffix(" · parallel").unwrap_or(short);
        let (head, _) = split_interval(short);
        let first = format!("Speed: {head} on {}", worst.workload);
        let n = u.rows.len();
        let alike = u
            .rows
            .iter()
            .filter(|r| r.words.answer == worst.words.answer)
            .count();
        let mut second: Vec<String> = Vec::new();
        if let Some(i) = headline_interval(&worst.words) {
            second.push(i.to_string());
        }
        if is_parallel(&worst.words) {
            second.push("parallel".into());
        }
        second.push(format!(
            "{alike} of {n} workload{}",
            if n == 1 { "" } else { "s" }
        ));
        if !worst.out_of_date.is_empty() {
            second.push("out of date".into());
        }
        Some((cut(&first, 54), cut(&second.join(" · "), 54)))
    }

    /// The project summary's line (§3.11), or `None` without a workloads
    /// file.
    pub fn summary_line(&self) -> Option<String> {
        match &self.group {
            Group::NoFile => None,
            Group::NoWorkload => Some("Speed: add a workload — see Speed".into()),
            Group::FileError(_) => {
                Some("Speed: the workloads file has an error — see Speed".into())
            }
            Group::NotYetRun => Some("Speed: not measured yet — see Speed".into()),
            Group::COnly => Some("Speed: only the C measured so far — see Speed".into()),
            Group::Units { measured, of } => {
                let mut counts: Vec<(&'static str, usize)> = Vec::new();
                let mut bump = |key: &'static str| match counts.iter_mut().find(|(k, _)| *k == key)
                {
                    Some((_, n)) => *n += 1,
                    None => counts.push((key, 1)),
                };
                let mut stale = 0;
                for u in &self.units {
                    let Some(worst) = u.rows.first() else {
                        continue;
                    };
                    bump(summary_kind(worst.words.answer));
                    if u.rows.iter().any(|r| is_parallel(&r.words)) {
                        bump("parallel");
                    }
                    if u.rows.iter().any(|r| !r.out_of_date.is_empty()) {
                        stale += 1;
                    }
                }
                let mut text = format!(
                    "Speed: {measured} of {of} unit{} measured",
                    if *of == 1 { "" } else { "s" }
                );
                if !counts.is_empty() {
                    text.push_str(" — ");
                    let parts: Vec<String> = counts
                        .iter()
                        .map(|(k, n)| match (*k, *n) {
                            ("behaves differently", n) if n > 1 => {
                                format!("{n} behave differently")
                            }
                            (k, n) => format!("{n} {k}"),
                        })
                        .collect();
                    text.push_str(&parts.join(", "));
                }
                if stale > 0 {
                    text.push_str(&format!(" · {stale} out of date"));
                }
                text.push_str(" — see Speed");
                Some(text)
            }
        }
    }
}

/// The summary's word for a unit's worst answer.
fn summary_kind(answer: &str) -> &'static str {
    match answer {
        "slower" => "slower",
        "probably-slower" => "probably slower",
        "close-call-slower" | "close-call-faster" => "close call",
        "about-as-fast" => "about as fast",
        "probably-faster" => "probably faster",
        "faster" => "faster",
        "behaves-differently" => "behaves differently",
        "no-clear-difference" => "no clear difference",
        "too-short" | "cant-tell-short-run" => "too short",
        a if a.starts_with("cant-tell") => "can't tell",
        _ => "not measured",
    }
}

/// A row that "uses several cores".
pub fn is_parallel(w: &RowWords) -> bool {
    w.details
        .iter()
        .any(|d| d.starts_with("uses several cores"))
}

/// The computer and compilers `row` records, in words (each value through
/// `safe_line`): what tells rows from different computers apart.
fn measured_on(row: &Row) -> String {
    let safe = |s: &str| harness_core::text::safe_line(s).to_string();
    let c = &row.inputs.computer;
    let mut compilers = Vec::new();
    if let Some(r) = &row.inputs.compilers.rustc {
        compilers.push(safe(r.split(" (").next().unwrap_or(r)));
    }
    compilers.push(safe(&row.inputs.compilers.cc));
    format!(
        "measured on {}, {} {}, with {} (not checked here)",
        safe(&c.cpu),
        safe(&c.os),
        safe(&c.build),
        compilers.join(" and ")
    )
}

/// The interval a time answer's headline gives — `(4.1–8.3 %)`, or
/// `(3.6–3.8×)` at 2× and more — the same text the words made; `None` for
/// an answer without one (about as fast, no clear difference, can't tell).
fn headline_interval(w: &RowWords) -> Option<&str> {
    if !matches!(
        w.answer,
        "slower"
            | "faster"
            | "probably-slower"
            | "probably-faster"
            | "close-call-slower"
            | "close-call-faster"
    ) {
        return None;
    }
    let start = w.headline.find(" (")? + 1;
    let len = w.headline[start..].find(')')? + 1;
    Some(&w.headline[start..start + len])
}

/// A short form split into its words and its parenthesised interval
/// (`slower 6.2 % (4.1–8.3 %)` → `slower 6.2 %`, `(4.1–8.3 %)`).
pub fn split_interval(short: &str) -> (&str, Option<&str>) {
    match short.rfind(" (") {
        Some(i) if short.ends_with(')') => (&short[..i], Some(&short[i + 1..])),
        _ => (short, None),
    }
}

fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut s: String = text.chars().take(width.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// Why perf would leave a verified or merged unit out today, as its
/// `left_out` reason — §3.2's selection, every condition perf's own
/// checks (build note 24): an interrupted Accept, a verdict not green and
/// fresh, the plan's `replaces` no longer the verdict's, no crate folder.
/// `None`: perf would measure it. Built from what the snapshot holds:
/// nothing more is read.
pub fn left_out_today(u: &UnitView) -> Option<&'static str> {
    if u.report.promotion_interrupted.is_some() {
        return Some("accept-interrupted");
    }
    if !u.report.fresh_green() {
        return Some("not-fresh");
    }
    match &u.verdict {
        Some(v) if v.inputs.replaces != u.unit.oracle_param_list("replaces") => {
            return Some("replaces-changed")
        }
        Some(_) => {}
        None => return Some("not-fresh"),
    }
    if u.crate_dir.is_none() {
        return Some("not-fresh");
    }
    None
}

/// Build Speed from `snapshot`.
pub fn build(snapshot: &Snapshot) -> SpeedModel {
    let perf = &snapshot.perf;
    let mut model = SpeedModel {
        group: Group::NoFile,
        c_rows: Vec::new(),
        program_rows: Vec::new(),
        held: Vec::new(),
        left_out: Vec::new(),
        units: Vec::new(),
        errors: perf.errors.clone(),
        orphans: perf.orphans.clone(),
        orphans_more: perf.orphans_more,
        header: Vec::new(),
        measuring: perf.measuring,
        measurable: Vec::new(),
        workloads: Vec::new(),
        c_clock: BTreeMap::new(),
        blocker: None,
    };
    let workloads = match &perf.workloads {
        Ok(WorkloadsState::Ready(w)) => w,
        Ok(state) => {
            model.blocker = state.blocker();
            model.group = match state {
                WorkloadsState::NoWorkload => Group::NoWorkload,
                WorkloadsState::Invalid(e) => Group::FileError(e.to_string()),
                _ => Group::NoFile,
            };
            return model;
        }
        // A file perf cannot load at all: its error, as `perf run` prints it.
        Err(e) => {
            model.blocker = Some(e.clone());
            model.group = Group::FileError(e.clone());
            return model;
        }
    };
    let order = |id: &str| {
        workloads
            .workloads
            .iter()
            .position(|w| w.id == id)
            .unwrap_or(usize::MAX)
    };
    let input_of = |id: &str| workloads.get(id).and_then(|w| w.input.clone());
    let program_name = snapshot.program_name.clone();
    let crate_digest = |id: &str| perf.crates.get(id).cloned();
    let measurable: Vec<String> = snapshot
        .units
        .iter()
        .filter(|u| {
            matches!(
                u.unit.status,
                harness_core::UnitStatus::Verified | harness_core::UnitStatus::Merged
            )
        })
        .filter(|u| left_out_today(u).is_none())
        .map(|u| u.unit.id.clone())
        .collect();
    model.measurable = measurable.clone();
    model.workloads = workloads
        .workloads
        .iter()
        .map(|w| (w.id.clone(), w.runs))
        .collect();
    let word =
        |row: &Row, side: Side<'_>, kind: RowKind, replaces: Option<Vec<String>>| -> SpeedRow {
            let input = input_of(&row.workload);
            let words = words::words(
                row,
                &words::Context {
                    side,
                    workload: &row.workload,
                    input: input.as_deref(),
                },
            );
            let today_workload = match perf.inputs.get(&row.workload) {
                Some(InputNow::Digest(d)) => Some(d.clone()),
                Some(InputNow::WhileMeasuring) | Some(InputNow::TooLarge) => {
                    Some(row.inputs.workload.clone())
                }
                _ => None,
            };
            // Without facts the C cannot be hashed here: only that one
            // comparison is skipped (the header says so) — the workload, the
            // recipe, the launcher and the Rust are still judged.
            let program_today = perf.program_now.as_deref().unwrap_or(&row.inputs.program);
            let today = Today {
                workload: today_workload.as_deref(),
                program: program_today,
                crate_digest: &crate_digest,
                replaces: replaces.as_deref(),
                program_name: &program_name,
                measurable: Some(&measurable),
                computer: None,
                compilers: None,
            };
            let mut why: Vec<currency::Reason> = currency::reasons(row, kind, &today);
            let reason = |token: &'static str, words: String| currency::Reason { token, words };
            match perf.inputs.get(&row.workload) {
                Some(InputNow::WhileMeasuring) => {
                    why.push(reason("measuring", "can't check while measuring".into()))
                }
                Some(InputNow::TooLarge) => why.push(reason(
                    "too-large",
                    "can't check: inputs too large to hash here".into(),
                )),
                Some(InputNow::Unusable(unusable)) => {
                    if let Some(input) = &input {
                        why.retain(|r| r.token != "workload-gone");
                        why.push(reason("input-unusable", unusable.words(input)));
                    }
                }
                _ => {}
            }
            let out_of_date_tokens = why.iter().map(|r| r.token).collect();
            let out_of_date = why.into_iter().map(|r| r.words).collect();
            SpeedRow {
                workload: row.workload.clone(),
                outcome: row.outcome.clone(),
                words,
                out_of_date,
                out_of_date_tokens,
                measured_on: measured_on(row),
                difference: row
                    .first_difference
                    .clone()
                    .or_else(|| row.found_before.clone()),
                found_before: row.first_difference.is_none() && row.found_before.is_some(),
                row: row.clone(),
            }
        };
    let mut computers: Vec<String> = Vec::new();
    let mut compilers: Vec<String> = Vec::new();
    let mut runs: Vec<u32> = Vec::new();
    let mut note = |row: &Row| {
        let c = harness_core::text::safe_line(&row.inputs.computer.cpu).to_string();
        if !computers.contains(&c) {
            computers.push(c);
        }
        if let Some(r) = &row.inputs.compilers.rustc {
            let v = r.split_whitespace().nth(1).unwrap_or(r).to_string();
            if !compilers.contains(&v) {
                compilers.push(v);
            }
        }
        if let Some(n) = row.runs {
            if !runs.contains(&n) {
                runs.push(n);
            }
        }
    };
    match &perf.program {
        Ok(Some(p)) => {
            for r in &p.c_alone {
                note(r);
                if let Some(c) = estimate::c_clock(r) {
                    model.c_clock.insert(r.workload.clone(), c);
                }
                model.c_rows.push(word(r, Side::C, RowKind::CAlone, None));
            }
            for r in &p.as_it_stands {
                note(r);
                model
                    .program_rows
                    .push(word(r, Side::AsItStands, RowKind::AsItStands, None));
                for u in r.inputs.units.iter().flatten() {
                    if !model.held.contains(&u.id) {
                        model.held.push(u.id.clone());
                    }
                }
                for l in r.inputs.left_out.iter().flatten() {
                    if !model.left_out.iter().any(|(id, _)| *id == l.id) {
                        model.left_out.push((l.id.clone(), l.reason.clone()));
                    }
                }
            }
        }
        Ok(None) => {}
        Err(e) => model.errors.push(format!(
            "program.json cannot be read: {}",
            harness_core::text::safe_line(e)
        )),
    }
    model.c_rows.sort_by_key(|r| order(&r.workload));
    model.program_rows.sort_by_key(|r| order(&r.workload));
    for u in &snapshot.units {
        let id = &u.unit.id;
        match perf.units.get(id) {
            Some(Ok(file)) => {
                let replaces = Some(u.unit.oracle_param_list("replaces"));
                let mut rows: Vec<SpeedRow> = file
                    .rows
                    .iter()
                    .map(|r| {
                        note(r);
                        word(r, Side::Unit(id), RowKind::Unit, replaces.clone())
                    })
                    .collect();
                rows.sort_by(|a, b| {
                    a.out_of_date
                        .is_empty()
                        .cmp(&b.out_of_date.is_empty())
                        .reverse()
                        .then(words::worst_first(&a.words, &b.words))
                });
                if !rows.is_empty() {
                    model.units.push(UnitSpeed {
                        id: id.clone(),
                        rows,
                    });
                }
            }
            Some(Err(e)) => model.errors.push(format!(
                "{id}'s results cannot be read: {}",
                harness_core::text::safe_line(e)
            )),
            None => {}
        }
    }
    model
        .units
        .sort_by(|a, b| match (a.rows.first(), b.rows.first()) {
            (Some(x), Some(y)) => words::worst_first(&x.words, &y.words),
            _ => std::cmp::Ordering::Equal,
        });
    model.group = if model.units.is_empty() && model.program_rows.is_empty() {
        if model.c_rows.is_empty() {
            Group::NotYetRun
        } else {
            Group::COnly
        }
    } else {
        // Measured: a unit with a row perf timed or ran (not only set-up
        // rows, which say "not measured"); of: those measurable today and
        // every unit with rows.
        let measured = model
            .units
            .iter()
            .filter(|u| u.rows.iter().any(|r| !results::is_set_up(&r.outcome)))
            .count();
        let of = model
            .units
            .iter()
            .filter(|u| !measurable.contains(&u.id))
            .count()
            + measurable.len();
        Group::Units { measured, of }
    };
    let on = match computers.len() {
        0 => String::new(),
        1 => format!("measured on {}", computers[0]),
        n => format!("measured on {n} kinds of computer"),
    };
    let with = match compilers.len() {
        0 => String::new(),
        1 => format!(" with rustc {} (not checked here)", compilers[0]),
        n => format!(" with {n} compilers"),
    };
    // Rows from different computers or compilers: each row names its own
    // (its details), said once.
    let with = if computers.len() > 1 || compilers.len() > 1 {
        format!("{with} — see each row")
    } else {
        with
    };
    let each = match runs.as_slice() {
        [n] => format!(" · {n} runs each"),
        _ => String::new(),
    };
    if !on.is_empty() {
        model.header.push(format!(
            "{on}{with} · as verify builds them{each} · compares what the program prints and how it ends"
        ));
        if perf.program_now.is_none() {
            model
                .header
                .push("the C is not checked here: no facts — run harness scan".into());
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_fit_19_columns() {
        let m = |group: Group| SpeedModel {
            group,
            c_rows: Vec::new(),
            program_rows: Vec::new(),
            held: Vec::new(),
            left_out: Vec::new(),
            units: Vec::new(),
            errors: Vec::new(),
            orphans: Vec::new(),
            orphans_more: 0,
            header: Vec::new(),
            measuring: false,
            measurable: Vec::new(),
            workloads: Vec::new(),
            c_clock: BTreeMap::new(),
            blocker: None,
        };
        for g in [
            Group::NoFile,
            Group::NoWorkload,
            Group::FileError("x".into()),
            Group::NotYetRun,
            Group::COnly,
            Group::Units {
                measured: 999,
                of: 999,
            },
        ] {
            let label = m(g).label();
            assert!(label.chars().count() <= 19, "{label}");
        }
        assert_eq!(
            m(Group::Units { measured: 2, of: 3 }).label(),
            "Speed (2 of 3)"
        );
    }
}
