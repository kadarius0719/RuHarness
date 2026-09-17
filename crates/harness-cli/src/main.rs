//! The `harness` binary (docs/SCHEMAS.md "CLI contract").
//!
//! Exit codes: 0 ok/green · 1 harness error (including stale refusals) ·
//! 2 usage error (clap's own) · 10 oracle red.

#![forbid(unsafe_code)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use harness_core::ledger::Ledger;
use harness_core::traits::{LanguageFrontend, OracleStrategy};
use harness_core::{hash, plan, planner, Facts, Plan, TargetContext, UnitStatus, Verdict};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit code for a red oracle verdict (distinct from clap's usage code 2).
const EXIT_ORACLE_RED: u8 = 10;

#[derive(Parser)]
#[command(
    name = "harness",
    version,
    about = "Incremental, verifiable migration to Rust"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan the target and regenerate migration/facts.jsonl
    Scan {
        /// Target repository root (contains harness.toml)
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Reconcile migration/plan.toml against the facts
    Plan {
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Run a unit's oracle and record the verdict
    Verify {
        /// Unit id from plan.toml
        unit: String,
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Ledger state queries
    State {
        #[command(subcommand)]
        cmd: StateCmd,
    },
}

#[derive(Subcommand)]
enum StateCmd {
    /// Staleness report: facts vs tree, plan vs tree, verdicts vs tree
    Status {
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
}

/// Print a line to stdout, ignoring EPIPE (`harness ... | head` must exit
/// with a documented code, not a broken-pipe panic).
fn out(line: String) {
    let _ = writeln!(std::io::stdout(), "{line}");
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Scan { target } => cmd_scan(target),
        Cmd::Plan { target } => cmd_plan(target),
        Cmd::Verify { unit, target } => cmd_verify(unit, target),
        Cmd::State {
            cmd: StateCmd::Status { target },
        } => cmd_status(target),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_scan(target: PathBuf) -> Result<ExitCode> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let frontend = harness_scan::CFrontend;
    let facts = frontend.scan(&ctx)?;
    std::fs::create_dir_all(ledger.dir()).context("creating migration dir")?;
    facts.store(&ledger.facts_path())?;
    out(format!(
        "scan: {} files, {} symbols, {} refs -> {}",
        facts.files.len(),
        facts.symbols.len(),
        facts.refs.len(),
        ledger.facts_path().display()
    ));
    Ok(ExitCode::SUCCESS)
}

/// Count facts file records whose hash no longer matches the working tree.
fn stale_fact_files(ctx: &TargetContext, facts: &Facts) -> usize {
    facts
        .files
        .iter()
        .filter(|f| {
            hash::file_hash(&ctx.root.join(&f.path))
                .map(|h| h != f.hash)
                .unwrap_or(true)
        })
        .count()
}

fn cmd_plan(target: PathBuf) -> Result<ExitCode> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    // Planning from stale facts would write stale hashes and strand verify
    // in a refusal loop — refuse up front instead.
    let stale = stale_fact_files(&ctx, &facts);
    if stale > 0 {
        bail!("facts.jsonl is stale ({stale} file(s) changed on disk); run `harness scan` first");
    }
    let mut computed = planner::compute_units(&facts)?;
    let plan_path = ledger.plan_path();
    let existing_text = if plan_path.exists() {
        let existing = Plan::load(&plan_path)?;
        plan::adopt_existing_ids(&mut computed, &existing);
        Some(std::fs::read_to_string(&plan_path).context("re-reading plan")?)
    } else {
        None
    };
    // Reconcile in memory, validate, and only then write (a corrupt plan
    // must never reach disk).
    let (content, changes) = plan::reconcile_to_string(
        &plan_path,
        existing_text.as_deref(),
        &ctx.config.target.name,
        &computed,
    )?;
    let reconciled = Plan::parse(&plan_path, &content)?;
    let order = reconciled
        .execution_order()
        .context("reconciled plan failed validation; plan.toml was NOT modified")?;
    let order_line = order
        .iter()
        .map(|u| u.id.as_str())
        .collect::<Vec<_>>()
        .join(" -> ");
    harness_core::ledger::write_atomic(&plan_path, content.as_bytes())?;
    if changes.is_empty() {
        out(format!("plan: no changes ({} units)", computed.len()));
    } else {
        for c in &changes {
            out(format!("plan: {c}"));
        }
    }
    out(format!("plan: execution order: {order_line}"));
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify(unit_id: String, target: PathBuf) -> Result<ExitCode> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let plan_path = ledger.plan_path();
    let plan_doc = Plan::load(&plan_path)?;
    // Structural validation on every load (docs/SCHEMAS.md): never run the
    // oracle against a plan with duplicate ids, missing deps, or cycles.
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before verifying")?;
    let unit = plan_doc.unit(&unit_id)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;

    // Stale-plan refusal (docs/SCHEMAS.md): the tree must match what was planned.
    let closure = facts.include_closure(&unit.files);
    let current = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    if current != unit.source_hash {
        bail!(
            "unit `{unit_id}` is stale: source changed since planning \
             (plan {} vs tree {}); run `harness scan`, then `harness plan`, \
             review the diff, then re-verify",
            unit.source_hash,
            current
        );
    }

    let verdict = match unit.oracle_kind() {
        Some("c-abi-differential") => {
            let strategy = harness_oracle::CAbiDifferential;
            strategy.verify(&ctx, unit)?
        }
        Some(kind) => bail!("unknown oracle kind `{kind}` for unit `{unit_id}`"),
        None => bail!("unit `{unit_id}` has no [unit.oracle] configured"),
    };

    verdict.store(&ledger.verdict_latest_path(&unit_id))?;
    harness_core::ledger::write_atomic(
        &ledger.verdict_md_path(&unit_id),
        verdict.render_md().as_bytes(),
    )?;
    for c in &verdict.checks {
        out(format!(
            "verify: [{}] {} — {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        ));
    }
    if verdict.green {
        verdict.store(&ledger.verdict_last_green_path(&unit_id))?;
        plan::set_status(&plan_path, &unit_id, UnitStatus::Verified)?;
        out(format!("verify: {unit_id} GREEN — status set to verified"));
        Ok(ExitCode::SUCCESS)
    } else {
        match unit.status {
            UnitStatus::Verified => {
                plan::set_status(&plan_path, &unit_id, UnitStatus::InProgress)?;
                out(format!(
                    "verify: {unit_id} RED — status demoted verified -> in-progress"
                ));
            }
            UnitStatus::Merged => {
                // Never auto-demote a merged unit; surface loudly instead.
                out(format!(
                    "verify: {unit_id} RED — unit is `merged` but its latest \
                     evidence is now red; investigate (status left untouched)"
                ));
            }
            _ => out(format!("verify: {unit_id} RED")),
        }
        Ok(ExitCode::from(EXIT_ORACLE_RED))
    }
}

fn cmd_status(target: PathBuf) -> Result<ExitCode> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);

    let facts = match Facts::load(&ledger.facts_path()) {
        Ok(f) => f,
        Err(e) if e.is_not_found() => {
            out("status: no facts — run `harness scan`".into());
            return Ok(ExitCode::SUCCESS);
        }
        // Parse errors and newer-schema refusals must surface, not read as
        // "no facts" (docs/SCHEMAS.md versioning rules).
        Err(e) => return Err(e.into()),
    };
    let stale_files = stale_fact_files(&ctx, &facts);
    out(format!(
        "status: facts {} ({} files, {} stale vs tree)",
        if stale_files == 0 {
            "fresh"
        } else {
            "STALE — run `harness scan`"
        },
        facts.files.len(),
        stale_files
    ));

    let plan_path = ledger.plan_path();
    if !plan_path.exists() {
        out("status: no plan — run `harness plan`".into());
        return Ok(ExitCode::SUCCESS);
    }
    let plan_doc = Plan::load(&plan_path)?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid")?;
    for unit in &plan_doc.units {
        let closure = facts.include_closure(&unit.files);
        let source_now = hash::file_set_hash_on_disk(&ctx.root, &closure)
            .unwrap_or_else(|_| "blake3:unreadable".into());
        let plan_fresh = source_now == unit.source_hash;

        let latest_path = ledger.verdict_latest_path(&unit.id);
        let (latest, verdict_desc) = match Verdict::load(&latest_path) {
            Ok(v) => {
                let mut stale: Vec<&str> = Vec::new();
                if v.inputs.unit_source != source_now {
                    stale.push("source");
                }
                if !v.inputs.rust_crate.is_empty() {
                    if let Some(crate_dir) = unit.oracle_param_str("rust_crate") {
                        let dir = ledger.unit_dir(&unit.id).join(crate_dir);
                        let now = hash::unit_crate_file_set_hash(&ctx.root, &dir)
                            .unwrap_or_else(|_| "blake3:unreadable".into());
                        if now != v.inputs.rust_crate {
                            stale.push("rust-crate");
                        }
                    }
                }
                if !v.inputs.driver.is_empty() {
                    if let Some(driver) = unit.oracle_param_str("driver") {
                        let now = hash::file_hash(&ctx.root.join(driver))
                            .unwrap_or_else(|_| "blake3:unreadable".into());
                        if now != v.inputs.driver {
                            stale.push("driver");
                        }
                    }
                }
                let color = if v.green { "green" } else { "red" };
                let desc = if stale.is_empty() {
                    format!("{color} (fresh)")
                } else {
                    format!("{color} (STALE: {})", stale.join(", "))
                };
                (VerdictState::Present { green: v.green }, desc)
            }
            Err(e) if e.is_not_found() => (VerdictState::Missing, "no verdict".to_string()),
            Err(e @ harness_core::Error::SchemaTooNew { .. }) => return Err(e.into()),
            Err(_) => (
                VerdictState::Unreadable,
                "verdict UNREADABLE (corrupt?)".to_string(),
            ),
        };

        // Verdicts are authoritative over plan status; flag both directions
        // (docs/SCHEMAS.md): a done-claiming status without green evidence,
        // and green evidence the status never absorbed.
        let done_claimed = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
        let contradiction = match latest {
            VerdictState::Present { green } => {
                (done_claimed && !green)
                    || (green && verdict_desc.ends_with("(fresh)") && !done_claimed)
            }
            VerdictState::Missing => done_claimed,
            VerdictState::Unreadable => done_claimed,
        };
        out(format!(
            "status: {} [{}] plan={} verdict={}{}",
            unit.id,
            unit.status.as_str(),
            if plan_fresh { "fresh" } else { "SOURCE-STALE" },
            verdict_desc,
            if contradiction {
                "  << CONTRADICTION: status and verdict evidence disagree"
            } else {
                ""
            }
        ));
    }
    Ok(ExitCode::SUCCESS)
}

enum VerdictState {
    Present { green: bool },
    Missing,
    Unreadable,
}
