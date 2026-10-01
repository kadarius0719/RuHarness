//! Measuring speed from the cockpit (docs/PERF-DESIGN.md §3.11): the
//! Measure acts' argv and gates, and their confirm dialogs' words — what
//! runs, how many times, about how long (§6), what it writes, no verdict,
//! the ledger's lock, that Cancel keeps finished rows, and to keep the
//! computer quiet.

use super::{os, Act, App, Pending};
use crate::speed::{Group, SideKey};
use harness_core::perf::estimate::{self, Estimate, Job};
use std::ffi::OsString;

impl App {
    /// Why perf cannot measure now, from the workloads file's state (and
    /// the platform); `None` when it can.
    pub fn speed_gate(&self) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return Some(
                "perf runs on macOS only for now — the Linux launcher is not built yet".into(),
            );
        }
        match &self.speed.group {
            Group::NoFile => Some("write your workloads file first".into()),
            Group::NoWorkload => Some("add a workload to your workloads file first".into()),
            Group::FileError(_) => Some("the workloads file has an error — Edit it first".into()),
            _ => None,
        }
    }

    /// Why `id` is not one perf would measure now, in words.
    fn not_measurable(&self, id: &str) -> String {
        let Some(u) = self.snapshot.unit(id) else {
            return format!("unit {id} is gone");
        };
        if !matches!(
            u.unit.status,
            harness_core::UnitStatus::Verified | harness_core::UnitStatus::Merged
        ) {
            return "perf measures a verified unit's Rust — accept an attempt first".into();
        }
        if u.report.promotion_interrupted.is_some() {
            return "an Accept was interrupted — Re-check it first".into();
        }
        "its verdict is not green and fresh — Re-check it first".into()
    }

    /// A Measure act's argv and its label (the one argv builder's part for
    /// them): `perf run`, `--unit=ID` for a unit, `--as-it-stands-only` for
    /// the program as it stands, `--workload=W`… `--runs=31` to measure
    /// again.
    pub(super) fn measure_argv(
        &self,
        act: Act,
        unit: Option<&str>,
    ) -> Result<(Vec<OsString>, String), String> {
        if let Some(why) = self.speed_gate() {
            return Err(why);
        }
        let measurable = |id: &str| -> Result<(), String> {
            if self.speed.measurable.iter().any(|m| m == id) {
                Ok(())
            } else {
                Err(self.not_measurable(id))
            }
        };
        let mut rest = vec![os("perf"), os("run"), self.target_arg()];
        let label = match act {
            Act::Measure => match unit {
                Some(id) => {
                    measurable(id)?;
                    rest.push(os(format!("--unit={id}")));
                    format!("Measure {id}'s speed")
                }
                None => act.label().to_string(),
            },
            Act::MeasureProgram => {
                if self.speed.measurable.len() < 2 {
                    return Err(
                        "the program as it stands needs two verified units — with one, that \
                         unit's own row measures the same program"
                            .into(),
                    );
                }
                rest.push(os("--as-it-stands-only"));
                act.label().to_string()
            }
            Act::MeasureMore => {
                let side = unit.map_or(SideKey::AsItStands, |u| SideKey::Unit(u.to_string()));
                let workloads = self.speed.more_runs(&side);
                if workloads.is_empty() {
                    return Err("no row asks to be measured again".into());
                }
                match unit {
                    Some(id) => {
                        measurable(id)?;
                        rest.push(os(format!("--unit={id}")));
                    }
                    None => rest.push(os("--as-it-stands-only")),
                }
                for w in &workloads {
                    rest.push(os(format!("--workload={w}")));
                }
                rest.push(os("--runs=31"));
                format!(
                    "Measure {} again with 31 runs",
                    unit.unwrap_or("the program as it stands")
                )
            }
            _ => return Err("not a Measure act".into()),
        };
        Ok((self.with_sandbox_flag(self.harness_argv(&rest)?), label))
    }

    /// The Measure dialogs' words (§3.11).
    pub(super) fn measure_words(&self, p: &Pending) -> (String, Vec<String>) {
        let m = &self.speed;
        let units = &m.measurable;
        let program = units.len() >= 2;
        let unit = p.unit.as_deref();
        let all: Vec<String> = m.workloads.iter().map(|(id, _)| id.clone()).collect();
        let more = match p.act {
            Act::MeasureMore => {
                m.more_runs(&unit.map_or(SideKey::AsItStands, |u| SideKey::Unit(u.into())))
            }
            _ => Vec::new(),
        };
        let measured = if p.act == Act::MeasureMore {
            &more
        } else {
            &all
        };
        let n_runs = |only: &[String]| -> String {
            if p.act == Act::MeasureMore {
                return "31 runs a side".into();
            }
            let runs: Vec<u32> = m
                .workloads
                .iter()
                .filter(|(id, _)| only.contains(id))
                .map(|(_, n)| *n)
                .collect();
            match runs.first() {
                Some(n) if runs.iter().all(|r| r == n) => format!("{n} runs a side"),
                _ => "each workload's own runs a side".into(),
            }
        };
        let on = format!(
            "{} workload{} ({})",
            measured.len(),
            if measured.len() == 1 { "" } else { "s" },
            measured.join(", ")
        );
        let (title, what, job, built): (String, String, Job, Vec<String>) = match (p.act, unit) {
            (Act::Measure, None) => {
                let mut sides = vec!["the C alone".to_string()];
                if !units.is_empty() {
                    sides.push(format!(
                        "the program with each verified unit's Rust alone ({})",
                        units.join(", ")
                    ));
                }
                if program {
                    sides.push("the program as it stands (all of them together)".into());
                }
                let rows = units.len() as u32 + program as u32;
                (
                    "Measure speed?".into(),
                    format!(
                        "Runs your program on {on}: {} — {}.",
                        sides.join(", then "),
                        n_runs(measured)
                    ),
                    m.job(None, None, true, rows, units.len() as u32, rows),
                    units.clone(),
                )
            }
            (Act::Measure, Some(id)) => (
                format!("Measure {id}'s speed?"),
                format!(
                    "Runs your program on {on}: the C and the program with {id}'s Rust alone, \
                     taking turns — {}.",
                    n_runs(measured)
                ),
                m.job(None, None, false, 1, 1, 1),
                vec![id.to_string()],
            ),
            (Act::MeasureProgram, _) => (
                "Measure the program as it stands?".into(),
                format!(
                    "Runs your program on {on}: the C and the program with every verified unit's \
                     Rust ({}), taking turns — {}.",
                    units.join(", "),
                    n_runs(measured)
                ),
                m.job(None, None, false, 1, units.len() as u32, 1),
                units.clone(),
            ),
            (_, unit) => {
                let who = unit.map_or("the program as it stands".to_string(), |u| {
                    format!("{u}'s Rust")
                });
                let crates = if unit.is_some() {
                    1
                } else {
                    units.len() as u32
                };
                (
                    format!(
                        "Measure {} again with 31 runs?",
                        unit.unwrap_or("the program as it stands")
                    ),
                    format!(
                        "Runs the C and {who} again on {on} — the rows that could not tell, were \
                         \"probably\" or a close call — 31 runs a side. On a busy computer it \
                         may still not tell.",
                    ),
                    m.job(Some(&more), Some(31), false, 1, crates, 1),
                    unit.map_or_else(|| units.clone(), |u| vec![u.to_string()]),
                )
            }
        };
        let mut body = vec![what];
        let cold: Vec<String> = built
            .iter()
            .filter(|id| {
                self.snapshot
                    .unit(id)
                    .and_then(|u| u.crate_dir.as_deref())
                    .is_some_and(estimate::crate_cold)
            })
            .cloned()
            .collect();
        let cold_words = if cold.is_empty() {
            String::new()
        } else {
            format!(
                ", plus building {}'s Rust, which may take minutes",
                cold.join("'s and ")
            )
        };
        body.push(match job.estimate() {
            e @ Estimate::Seconds(_) => {
                format!("Takes {}, builds included{cold_words}.", e.words())
            }
            e => {
                let w = e.words();
                let mut c = w.chars();
                let first = c.next().map(|f| f.to_uppercase().collect::<String>());
                format!("{}{}{cold_words}.", first.unwrap_or_default(), c.as_str())
            }
        });
        body.push(
            "Writes migration/perf/ (its rows replace the ones they measure again) and scratch \
             folders under migration/build/ (.perf, .perf-out with the kept outputs, perf-logs); \
             the first time, or after an update, it builds perf's launcher into \
             ~/Library/Caches/ruharness/perf."
                .into(),
        );
        body.push(
            "No verdict changes: perf only measures. It holds the ledger's lock while it runs; \
             Cancel keeps the rows already finished."
                .into(),
        );
        body.push(
            "Keep the computer quiet while it measures — other work makes the numbers noisier."
                .into(),
        );
        (title, body)
    }
}
