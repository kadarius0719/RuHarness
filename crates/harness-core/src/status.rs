//! The per-unit staleness report behind `harness state status`
//! (docs/SCHEMAS.md "CLI contract"; docs/CLI-HARDENING.md §4): one struct
//! that the human line, the `--json` `unit` event, the review cockpit and
//! the MCP bridge all render from, so the four hash comparisons and the
//! contradiction rule live in exactly one place.

use crate::attempts;
use crate::error::Error;
use crate::hash;
use crate::ledger::{Holder, Ledger, WriterLock};
use crate::plan::{Plan, Unit, UnitStatus};
use crate::verdict::Verdict;
use crate::TargetContext;
use serde::Serialize;

/// Whether a unit's latest verdict exists and could be read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VerdictState {
    /// `oracle-latest.json` loaded.
    Present,
    /// No verdict yet.
    Missing,
    /// A verdict file that does not parse (corrupt?).
    Unreadable,
}

/// The latest verdict, as far as staleness is concerned.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerdictReport {
    /// Whether it exists and reads.
    pub state: VerdictState,
    /// Its colour (present only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub green: Option<bool>,
    /// Which of its inputs no longer match the tree: `source`, `rust-crate`,
    /// `driver` (present only; empty = fresh).
    pub stale: Vec<String>,
}

/// One recorded migrate attempt, for the list a client shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttemptSummary {
    /// The attempt id.
    pub id: String,
    /// Provider kind it ran under.
    pub provider_kind: String,
    /// Its outcome.
    pub outcome: String,
    /// Bound to the CURRENT unit source (the R-5 provenance rule).
    pub bound: bool,
}

/// Everything `harness state status` knows about one unit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnitReport {
    /// The unit id.
    pub id: String,
    /// Plan status (`planned`, `in-progress`, `verified`, `merged`, …).
    pub status: String,
    /// The plan's `source_hash` still matches the tree.
    pub source_fresh: bool,
    /// The latest verdict.
    pub verdict: VerdictReport,
    /// Status and verdict evidence disagree (a done-claiming status without
    /// fresh green evidence, or fresh green evidence the status never
    /// absorbed) — and no writer is at work.
    pub contradiction: bool,
    /// A LIVE writer holds the ledger and the unit looked inconsistent: the
    /// verdict+status pair is being written right now
    /// (docs/CLI-HARDENING.md §1). Reported INSTEAD of `contradiction`; a
    /// dead holder's leftover line is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_in_flight: Option<Holder>,
    /// A promotion of this unit was interrupted and no live writer is at
    /// work: the attempt id of its `.promote-<id>/` marker, or `legacy` for a
    /// bare `.<crate>.prev` (docs/TUI-DESIGN.md §2). The next writing command
    /// (`verify`, `migrate`, `promote`, `override`) recovers it by evidence;
    /// until then the unit is reported this way INSTEAD of `contradiction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_interrupted: Option<String>,
    /// The unit's recorded migrate attempts.
    pub attempts: Vec<AttemptSummary>,
}

impl UnitReport {
    /// The verdict is present, green and fresh.
    pub fn fresh_green(&self) -> bool {
        self.verdict.state == VerdictState::Present
            && self.verdict.green == Some(true)
            && self.verdict.stale.is_empty()
    }

    /// The human status line, exactly as `harness state status` prints it.
    pub fn render_line(&self) -> String {
        let desc = match self.verdict.state {
            VerdictState::Present => {
                let color = if self.verdict.green == Some(true) {
                    "green"
                } else {
                    "red"
                };
                if self.verdict.stale.is_empty() {
                    format!("{color} (fresh)")
                } else {
                    format!("{color} (STALE: {})", self.verdict.stale.join(", "))
                }
            }
            VerdictState::Missing => "no verdict".to_string(),
            VerdictState::Unreadable => "verdict UNREADABLE (corrupt?)".to_string(),
        };
        let tail = match &self.write_in_flight {
            Some(h) => format!(
                "  << write in flight (pid {}, `{}`) — re-check when it is done",
                h.pid, h.command
            ),
            None => match &self.promotion_interrupted {
                Some(id) => format!(
                    "  << promotion of {id} interrupted — the next writing command recovers it"
                ),
                None if self.contradiction => {
                    "  << CONTRADICTION: status and verdict evidence disagree".to_string()
                }
                None => String::new(),
            },
        };
        format!(
            "status: {} [{}] plan={} verdict={desc}{tail}",
            self.id,
            self.status,
            if self.source_fresh {
                "fresh"
            } else {
                "SOURCE-STALE"
            },
        )
    }

    /// The human attempts line (`None` when there are no attempts).
    pub fn render_attempts_line(&self) -> Option<String> {
        if self.attempts.is_empty() {
            return None;
        }
        let bound = self.attempts.iter().filter(|a| a.bound).count();
        let summary: Vec<String> = self
            .attempts
            .iter()
            .map(|a| format!("{}:{}:{}", a.id, a.provider_kind, a.outcome))
            .collect();
        Some(format!(
            "status:   attempts: {} ({} bound to current source) [{}]",
            self.attempts.len(),
            bound,
            summary.join(", ")
        ))
    }
}

/// Compute the report for `unit`. Detection of a contradiction or a stale
/// input is two-phase: a lock-free reader's plan-then-verdict snapshot can
/// straddle a writer's non-atomic verdict+status pair, so a first hit is
/// re-checked once from fresh reads; if it holds and a writer holds the
/// ledger, the unit is reported `write_in_flight` instead
/// (docs/CLI-HARDENING.md §1 "What a lock-free reader can see").
pub fn unit_report(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &crate::Facts,
    unit: &Unit,
) -> Result<UnitReport, Error> {
    let mut report = compute(ctx, ledger, facts, unit)?;
    if report.contradiction || !report.verdict.stale.is_empty() {
        // Phase two: the plan entry and the verdict, re-read.
        let plan = Plan::load(&ledger.plan_path())?;
        let again = match plan.units.iter().find(|u| u.id == unit.id) {
            Some(fresh_unit) => compute(ctx, ledger, facts, fresh_unit)?,
            None => report.clone(),
        };
        report = again;
        if report.contradiction || !report.verdict.stale.is_empty() {
            // A holder that died without cleanup (a signal death never
            // truncates the line) is diagnostics, not a writer at work.
            if let Some(holder) = live_holder(ledger)? {
                report.contradiction = false;
                report.write_in_flight = Some(holder);
            }
        }
    }
    // An interrupted promotion (marker left behind, nobody at work) explains
    // whatever the unit looks like until the next writer recovers it.
    if report.write_in_flight.is_none() {
        if let Some(marker) = promotion_marker(ledger, unit)? {
            if live_holder(ledger)?.is_none() {
                report.contradiction = false;
                report.promotion_interrupted = Some(marker);
            }
        }
    }
    Ok(report)
}

/// The ledger's holder, when its process is alive.
fn live_holder(ledger: &Ledger) -> Result<Option<Holder>, Error> {
    Ok(WriterLock::holder(ledger)?.filter(|h| pid_alive(h.pid)))
}

/// The attempt id of the first `.promote-<id>/` marker in the unit dir, or
/// `legacy` for a bare `.<crate>.prev` (the pre-marker protocol).
fn promotion_marker(ledger: &Ledger, unit: &Unit) -> Result<Option<String>, Error> {
    let dir = ledger.unit_dir(&unit.id);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&dir, e)),
    };
    let mut markers: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix(".promote-"))
                .map(str::to_string)
        })
        .collect();
    markers.sort();
    if let Some(first) = markers.into_iter().next() {
        return Ok(Some(first));
    }
    let legacy = unit
        .oracle_param_str("rust_crate")
        .is_some_and(|name| dir.join(format!(".{name}.prev")).is_dir());
    Ok(legacy.then(|| "legacy".to_string()))
}

/// `kill -0 <pid>`: true while the process exists — the same unsafe-free
/// probe the oracle uses for its own process groups; it runs no target
/// code. A pid reused by an unrelated process reads as alive (the next
/// writer truncates the stale line anyway); a holder owned by another user
/// reads as dead (the ledger is single-user by design). If the probe itself
/// cannot run, assume alive: never invent a contradiction from a failed
/// probe.
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(true)
}

fn compute(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &crate::Facts,
    unit: &Unit,
) -> Result<UnitReport, Error> {
    let closure = facts.include_closure(&unit.files);
    let source_now = hash::file_set_hash_on_disk(&ctx.root, &closure)
        .unwrap_or_else(|_| "blake3:unreadable".into());
    let source_fresh = source_now == unit.source_hash;

    let verdict = match Verdict::load(&ledger.verdict_latest_path(&unit.id)) {
        Ok(v) => {
            let mut stale: Vec<String> = Vec::new();
            if v.inputs.unit_source != source_now {
                stale.push("source".into());
            }
            if !v.inputs.rust_crate.is_empty() {
                if let Some(crate_dir) = unit.oracle_param_str("rust_crate") {
                    let dir = ledger.unit_dir(&unit.id).join(crate_dir);
                    let now = hash::unit_crate_file_set_hash(&ctx.root, &dir)
                        .unwrap_or_else(|_| "blake3:unreadable".into());
                    if now != v.inputs.rust_crate {
                        stale.push("rust-crate".into());
                    }
                }
            }
            if !v.inputs.driver.is_empty() {
                if let Some(driver) = unit.oracle_param_str("driver") {
                    let now = hash::file_hash(&ctx.root.join(driver))
                        .unwrap_or_else(|_| "blake3:unreadable".into());
                    if now != v.inputs.driver {
                        stale.push("driver".into());
                    }
                }
            }
            VerdictReport {
                state: VerdictState::Present,
                green: Some(v.green),
                stale,
            }
        }
        Err(e) if e.is_not_found() => VerdictReport {
            state: VerdictState::Missing,
            green: None,
            stale: Vec::new(),
        },
        // Newer-schema refusals must surface, not read as "unreadable".
        Err(e @ Error::SchemaTooNew { .. }) => return Err(e),
        Err(_) => VerdictReport {
            state: VerdictState::Unreadable,
            green: None,
            stale: Vec::new(),
        },
    };

    // Verdicts are authoritative over plan status; flag both directions
    // (docs/SCHEMAS.md): a done-claiming status without FRESH green evidence
    // (red, or stale because crate/source/driver changed since), and fresh
    // green evidence the status never absorbed.
    let done_claimed = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
    let contradiction = match verdict.state {
        VerdictState::Present => {
            let green = verdict.green == Some(true);
            let fresh = verdict.stale.is_empty();
            (done_claimed && (!green || !fresh)) || (green && fresh && !done_claimed)
        }
        VerdictState::Missing | VerdictState::Unreadable => done_claimed,
    };

    let attempts = attempts::load_unit_attempts(ledger, &unit.id)?
        .into_iter()
        .map(|a| AttemptSummary {
            bound: a.unit_source == source_now,
            id: a.id,
            provider_kind: a.provider_kind,
            outcome: a.outcome,
        })
        .collect();

    Ok(UnitReport {
        id: unit.id.clone(),
        status: unit.status.as_str().to_string(),
        source_fresh,
        verdict,
        contradiction,
        write_in_flight: None,
        promotion_interrupted: None,
        attempts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: VerdictState, green: Option<bool>, stale: &[&str]) -> UnitReport {
        UnitReport {
            id: "u1".into(),
            status: "verified".into(),
            source_fresh: true,
            verdict: VerdictReport {
                state,
                green,
                stale: stale.iter().map(|s| s.to_string()).collect(),
            },
            contradiction: false,
            write_in_flight: None,
            promotion_interrupted: None,
            attempts: Vec::new(),
        }
    }

    #[test]
    fn the_human_line_keeps_its_bytes() {
        let r = report(VerdictState::Present, Some(true), &[]);
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=fresh verdict=green (fresh)"
        );
        let mut r = report(
            VerdictState::Present,
            Some(false),
            &["rust-crate", "driver"],
        );
        r.source_fresh = false;
        r.contradiction = true;
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=SOURCE-STALE verdict=red (STALE: rust-crate, driver)  \
             << CONTRADICTION: status and verdict evidence disagree"
        );
        r.write_in_flight = Some(Holder {
            pid: 7,
            command: "verify u1".into(),
            started: "t".into(),
        });
        assert!(r
            .render_line()
            .contains("write in flight (pid 7, `verify u1`)"));
        assert!(!r.render_line().contains("CONTRADICTION"));
        r.write_in_flight = None;
        r.promotion_interrupted = Some("a-0123456789ab".into());
        assert!(r.render_line().ends_with(
            "<< promotion of a-0123456789ab interrupted — the next writing command recovers it"
        ));
        assert!(!r.render_line().contains("CONTRADICTION"));
        assert_eq!(
            report(VerdictState::Missing, None, &[]).render_line(),
            "status: u1 [verified] plan=fresh verdict=no verdict"
        );
        assert_eq!(
            report(VerdictState::Unreadable, None, &[]).render_line(),
            "status: u1 [verified] plan=fresh verdict=verdict UNREADABLE (corrupt?)"
        );
    }

    #[test]
    fn the_event_shape_is_pinned() {
        let mut r = report(VerdictState::Present, Some(true), &["source"]);
        r.attempts.push(AttemptSummary {
            id: "a-1".into(),
            provider_kind: "external".into(),
            outcome: "green".into(),
            bound: false,
        });
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            "{\"id\":\"u1\",\"status\":\"verified\",\"source_fresh\":true,\"verdict\":{\"state\":\
             \"present\",\"green\":true,\"stale\":[\"source\"]},\"contradiction\":false,\
             \"attempts\":[{\"id\":\"a-1\",\"provider_kind\":\"external\",\"outcome\":\"green\",\
             \"bound\":false}]}"
        );
        assert_eq!(
            r.render_attempts_line().unwrap(),
            "status:   attempts: 1 (0 bound to current source) [a-1:external:green]"
        );
        let missing = report(VerdictState::Missing, None, &[]);
        assert!(serde_json::to_string(&missing)
            .unwrap()
            .contains("\"verdict\":{\"state\":\"missing\",\"stale\":[]}"));
    }
}
