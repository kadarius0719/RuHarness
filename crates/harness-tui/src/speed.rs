//! The cockpit's Speed (docs/PERF-DESIGN.md §3.11): the group's state, every
//! stored row's words — the same `harness-core` words the CLI prints — with
//! why it is out of date, the units ordered worst first, the View's header,
//! a unit's header line and the project summary's line. Built from the
//! snapshot alone: nothing here starts a process.

use crate::model::Snapshot;
use crate::perfread::InputNow;
use harness_core::perf::currency::{self, Today};
use harness_core::perf::results::{Row, RowKind};
use harness_core::perf::words::{self as words, RowWords, Side};
use harness_core::perf::workloads::WorkloadsState;

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
    /// Units no longer in the plan that still have a results file.
    pub orphans: Vec<String>,
    /// The View's header lines (computers and compilers the rows record).
    pub header: Vec<String>,
    /// A perf run holds the lock now.
    pub measuring: bool,
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

    /// A unit's rows.
    pub fn unit(&self, id: &str) -> Option<&UnitSpeed> {
        self.units.iter().find(|u| u.id == id)
    }

    /// A unit's header (§3.11): `Speed: <short> on <workload>` and, on its
    /// own line, the interval or detail and "k of n workloads" — each within
    /// 54 columns.
    pub fn unit_header(&self, id: &str) -> Option<(String, String)> {
        let u = self.unit(id)?;
        let worst = u.rows.first()?;
        let (head, interval) = split_interval(&worst.words.short);
        let first = format!("Speed: {head} on {}", worst.workload);
        let n = u.rows.len();
        let alike = u
            .rows
            .iter()
            .filter(|r| r.words.answer == worst.words.answer)
            .count();
        let count = format!("{alike} of {n} workload{}", if n == 1 { "" } else { "s" });
        let mut second = match interval {
            Some(i) => format!("{i} · {count}"),
            None => count,
        };
        if !worst.out_of_date.is_empty() {
            second.push_str(" · out of date");
        }
        Some((cut(&first, 54), cut(&second, 54)))
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
        "too-short" | "short-run" => "too short",
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
        errors: Vec::new(),
        orphans: perf.orphans.clone(),
        header: Vec::new(),
        measuring: perf.measuring,
    };
    let workloads = match &perf.workloads {
        Ok(WorkloadsState::NoFile) => return model,
        Ok(WorkloadsState::NoWorkload) => {
            model.group = Group::NoWorkload;
            return model;
        }
        Ok(WorkloadsState::Invalid(e)) => {
            model.group = Group::FileError(e.to_string());
            return model;
        }
        Err(e) => {
            model.group = Group::FileError(e.clone());
            return model;
        }
        Ok(WorkloadsState::Ready(w)) => w,
    };
    let order = |id: &str| {
        workloads
            .workloads
            .iter()
            .position(|w| w.id == id)
            .unwrap_or(usize::MAX)
    };
    let input_of = |id: &str| workloads.get(id).and_then(|w| w.input.clone());
    let program_now = perf.program_now.clone().unwrap_or_default();
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
        .filter(|u| u.report.fresh_green() && u.report.promotion_interrupted.is_none())
        .map(|u| u.unit.id.clone())
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
            let today = Today {
                workload: today_workload.as_deref(),
                program: &program_now,
                crate_digest: &crate_digest,
                replaces: replaces.as_deref(),
                program_name: &program_name,
                measurable: Some(&measurable),
                computer: None,
                compilers: None,
            };
            let mut out_of_date = if program_now.is_empty() {
                Vec::new()
            } else {
                currency::out_of_date(row, kind, &today)
            };
            match perf.inputs.get(&row.workload) {
                Some(InputNow::WhileMeasuring) => {
                    out_of_date.push("can't check while measuring".into())
                }
                Some(InputNow::TooLarge) => {
                    out_of_date.push("can't check: inputs too large to hash here".into())
                }
                Some(InputNow::Unusable(reason)) => {
                    if let Some(input) = &input {
                        out_of_date.retain(|w| !w.starts_with("the workload is gone"));
                        out_of_date.push(reason.words(input));
                    }
                }
                _ => {}
            }
            SpeedRow {
                workload: row.workload.clone(),
                outcome: row.outcome.clone(),
                words,
                out_of_date,
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
        Group::Units {
            measured: model.units.len(),
            of: measurable.len().max(model.units.len()),
        }
    };
    let on = match computers.len() {
        0 => String::new(),
        1 => format!("measured on {}", computers[0]),
        n => format!("measured on {n} kinds of computer — see each row"),
    };
    let with = match compilers.len() {
        0 => String::new(),
        1 => format!(" with rustc {} (not checked here)", compilers[0]),
        n => format!(" with {n} compilers — see each row"),
    };
    let each = match runs.as_slice() {
        [n] => format!(" · {n} runs each"),
        _ => String::new(),
    };
    if !on.is_empty() {
        model.header.push(format!(
            "{on}{with} · as verify builds them{each} · compares what the program prints and how it ends"
        ));
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
            header: Vec::new(),
            measuring: false,
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
