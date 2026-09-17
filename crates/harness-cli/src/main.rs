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
    /// Run the hazard detectors and regenerate observer findings
    Detect {
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Triage findings (LLM pass) and render observations.md
    Observe {
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Record a human review of a triaged finding
    Review {
        /// Finding id (f-...)
        finding: String,
        /// Uphold the model's dismissal
        #[arg(long, conflicts_with = "reinstate")]
        uphold_dismiss: bool,
        /// Reinstate a dismissed finding
        #[arg(long)]
        reinstate: bool,
        /// Optional note
        #[arg(long, default_value = "")]
        note: String,
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Refresh the generated runtime view (AGENTS.md managed block)
    SyncRuntime {
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Exit 1 if regeneration would change the block (CI mode)
        #[arg(long)]
        check: bool,
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
        Cmd::Detect { target } => cmd_detect(target),
        Cmd::Observe { target } => cmd_observe(target),
        Cmd::Review {
            finding,
            uphold_dismiss,
            reinstate,
            note,
            target,
        } => cmd_review(finding, uphold_dismiss, reinstate, note, target),
        Cmd::SyncRuntime { target, check } => cmd_sync_runtime(target, check),
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

// ---------- M2: observer commands ----------

fn facts_records_hash(facts: &Facts) -> String {
    let pairs: Vec<(String, String)> = facts
        .files
        .iter()
        .map(|f| (f.path.clone(), f.hash.clone()))
        .collect();
    hash::file_set_hash(&pairs)
}

fn cmd_detect(target: PathBuf) -> Result<ExitCode> {
    use harness_core::observer::{FindingsFile, ObserverPaths};
    use harness_core::traits::Detector;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let stale = stale_fact_files(&ctx, &facts);
    if stale > 0 {
        bail!("facts.jsonl is stale ({stale} file(s) changed on disk); run `harness scan` first");
    }
    let suite = harness_detect::CTreeSitterSuite;
    let findings = suite.detect(&ctx, &facts)?;
    let file = FindingsFile {
        detector_suite: suite.name().to_string(),
        facts_hash: facts_records_hash(&facts),
        findings,
    };
    let path = ObserverPaths::findings(&ledger);
    file.store(&path)?;
    let mut by_category: std::collections::BTreeMap<&str, usize> = Default::default();
    for f in &file.findings {
        *by_category.entry(f.category.as_str()).or_insert(0) += 1;
    }
    out(format!(
        "detect: {} finding(s) -> {}",
        file.findings.len(),
        path.display()
    ));
    for (cat, n) in by_category {
        out(format!("detect:   {cat}: {n}"));
    }
    Ok(ExitCode::SUCCESS)
}

/// Everything `observe` needs, loaded and freshness-checked.
type ObserverInputs = (
    Facts,
    Plan,
    harness_core::observer::FindingsFile,
    Vec<harness_core::observer::Finding>,
    harness_core::observer::TriageFile,
    Vec<harness_core::observer::Review>,
);

fn observer_inputs(ctx: &TargetContext, ledger: &Ledger) -> Result<ObserverInputs> {
    use harness_core::observer::{self, ObserverPaths};
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid")?;
    let findings = harness_core::observer::FindingsFile::load(&ObserverPaths::findings(ledger))
        .context("loading findings (run `harness detect` first)")?;
    if findings.facts_hash != facts_records_hash(&facts) {
        bail!("findings.jsonl is bound to different facts; run `harness detect`");
    }
    for f in &findings.findings {
        let now = hash::file_hash(&ctx.root.join(&f.file))
            .with_context(|| format!("hashing {}", f.file))?;
        if now != f.file_hash {
            bail!(
                "finding {} is stale ({} changed since detect); run `harness detect`",
                f.id,
                f.file
            );
        }
    }
    let annotations = observer::load_annotations(&ObserverPaths::annotations(ledger))?;
    let triage = harness_core::observer::TriageFile::load(&ObserverPaths::triage(ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(ledger))?;
    Ok((facts, plan_doc, findings, annotations, triage, reviews))
}

fn cmd_observe(target: PathBuf) -> Result<ExitCode> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let (facts, plan_doc, findings, annotations, _, reviews) = observer_inputs(&ctx, &ledger)?;

    let traces = ObserverPaths::traces(&ledger);
    let llm = &ctx.config.llm;
    let adapter: Box<dyn harness_core::traits::ProviderAdapter> = match llm.provider.as_str() {
        "anthropic" => Box::new(harness_llm::AnthropicAdapter::from_env(&llm.api_key_env)?),
        "replay" => Box::new(harness_llm::TraceAdapter::new(traces.clone(), false)),
        "external" => Box::new(harness_llm::TraceAdapter::new(traces.clone(), true)),
        other => bail!("unknown [llm] provider `{other}` (external | anthropic | replay)"),
    };
    let outcome = match harness_llm::run_triage(
        adapter.as_ref(),
        &llm.model,
        llm.max_tokens,
        &ctx,
        &facts,
        &plan_doc,
        &findings,
        &traces,
    ) {
        Ok(o) => o,
        Err(e) if e.to_string().contains("awaiting response") => {
            eprintln!("{e:#}");
            eprintln!(
                "observe: external provider mode — supply the response file(s) under {} and re-run",
                traces.display()
            );
            return Ok(ExitCode::FAILURE);
        }
        Err(e) => return Err(e.into()),
    };
    outcome.triage.store(&ObserverPaths::triage(&ledger))?;

    let mut all_findings: Vec<harness_core::observer::Finding> = findings.findings.clone();
    all_findings.extend(annotations.iter().cloned());
    let risk = harness_core::risk::score_units(
        &facts,
        &plan_doc,
        &all_findings,
        &outcome.triage,
        &reviews,
    );
    let rendered = observer::render_observations(&observer::ObservationsInput {
        findings: &findings,
        annotations: &annotations,
        triage: &outcome.triage,
        reviews: &reviews,
        plan: &plan_doc,
        facts: &facts,
        risk: &risk,
    })?;
    harness_core::ledger::write_atomic(&ObserverPaths::observations(&ledger), rendered.as_bytes())?;
    let usage_in: u64 = outcome.usage.iter().map(|(_, i, _)| i).sum();
    let usage_out: u64 = outcome.usage.iter().map(|(_, _, o)| o).sum();
    out(format!(
        "observe: {} verdict(s) via `{}` -> {} (tokens in/out: {}/{})",
        outcome.triage.verdicts.len(),
        adapter.name(),
        ObserverPaths::observations(&ledger).display(),
        usage_in,
        usage_out
    ));
    for r in risk.iter().take(5) {
        out(format!("observe: risk {} {}", r.score, r.unit));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_review(
    finding: String,
    uphold_dismiss: bool,
    reinstate: bool,
    note: String,
    target: PathBuf,
) -> Result<ExitCode> {
    use harness_core::observer::{self, ObserverPaths};
    if uphold_dismiss == reinstate {
        bail!("pass exactly one of --uphold-dismiss or --reinstate");
    }
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let findings = harness_core::observer::FindingsFile::load(&ObserverPaths::findings(&ledger))
        .context("loading findings (run `harness detect` first)")?;
    let annotations = observer::load_annotations(&ObserverPaths::annotations(&ledger))?;
    if !findings.findings.iter().any(|f| f.id == finding)
        && !annotations.iter().any(|f| f.id == finding)
    {
        bail!("unknown finding `{finding}`");
    }
    let action = if uphold_dismiss {
        "uphold-dismiss"
    } else {
        "reinstate"
    };
    observer::append_review(
        &ObserverPaths::reviews(&ledger),
        &observer::Review {
            finding: finding.clone(),
            action: action.into(),
            note,
        },
    )?;
    out(format!(
        "review: {finding} {action} recorded — re-run `harness observe` to re-render"
    ));
    Ok(ExitCode::SUCCESS)
}

fn cmd_sync_runtime(target: PathBuf, check: bool) -> Result<ExitCode> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    // Risk from whatever observer state exists: a MISSING file is fine
    // (empty), but parse errors and newer-schema refusals must propagate —
    // the agent-facing view must never be built from silently-dropped data.
    let findings =
        match harness_core::observer::FindingsFile::load(&ObserverPaths::findings(&ledger)) {
            Ok(f) => f,
            Err(e) if e.is_not_found() => Default::default(),
            Err(e) => return Err(e.into()),
        };
    let annotations = observer::load_annotations(&ObserverPaths::annotations(&ledger))?;
    let triage = harness_core::observer::TriageFile::load(&ObserverPaths::triage(&ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(&ledger))?;
    let mut all_findings = findings.findings.clone();
    all_findings.extend(annotations);
    let risk = harness_core::risk::score_units(&facts, &plan_doc, &all_findings, &triage, &reviews);

    let body =
        harness_core::runtime_view::render_block_body(&ctx.config.target.name, &plan_doc, &risk);
    let block = harness_core::runtime_view::wrap_block(&body);
    let agents_path = ctx.root.join("AGENTS.md");
    let existing = std::fs::read_to_string(&agents_path).ok();
    let updated = harness_core::runtime_view::apply(existing.as_deref(), &block)?;
    if check {
        if existing.as_deref() == Some(updated.as_str()) {
            out("sync-runtime: up to date".into());
            return Ok(ExitCode::SUCCESS);
        }
        eprintln!(
            "sync-runtime: AGENTS.md managed block is out of date; run `harness sync-runtime`"
        );
        return Ok(ExitCode::FAILURE);
    }
    harness_core::ledger::write_atomic(&agents_path, updated.as_bytes())?;
    // Claude Code bridge: CLAUDE.md imports AGENTS.md (per §14.3 spike evidence).
    let claude_path = ctx.root.join("CLAUDE.md");
    let claude = std::fs::read_to_string(&claude_path).unwrap_or_default();
    if !claude.contains("@AGENTS.md") {
        let mut updated_claude = claude;
        if !updated_claude.is_empty() && !updated_claude.ends_with('\n') {
            updated_claude.push('\n');
        }
        updated_claude.push_str("@AGENTS.md\n");
        harness_core::ledger::write_atomic(&claude_path, updated_claude.as_bytes())?;
    }
    out(format!("sync-runtime: {} updated", agents_path.display()));
    Ok(ExitCode::SUCCESS)
}
