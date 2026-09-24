//! The `harness` binary (docs/SCHEMAS.md "CLI contract").
//!
//! Exit codes: 0 ok/green · 1 harness error (including stale refusals) ·
//! 2 usage error (clap's own) · 10 oracle red.

#![forbid(unsafe_code)]

mod bench;
mod gen_driver;
mod promote;
mod report;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use harness_core::ledger::Ledger;
use harness_core::ledger::WriterLock;
use harness_core::traits::{LanguageFrontend, OracleStrategy};
use harness_core::{hash, plan, planner, Error, Facts, Plan, TargetContext, UnitStatus};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// Exit code for a red oracle verdict (distinct from clap's usage code 2).
pub(crate) const EXIT_ORACLE_RED: u8 = 10;

#[derive(Parser)]
#[command(
    name = "harness",
    version,
    about = "Incremental, verifiable migration to Rust"
)]
struct Cli {
    /// Machine-readable events on stdout (newline-delimited JSON,
    /// `ruharness-events`); human logs stay on stderr
    #[arg(long, global = true)]
    json: bool,
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
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
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
    /// Translate a unit through the configured LLM provider and verify it
    Migrate {
        /// Unit id from plan.toml
        unit: String,
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Provider profile override (built-in or user-level profile name)
        #[arg(long)]
        provider: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
        /// Promote a green candidate even when the unit is already verified
        #[arg(long, conflicts_with = "no_promote")]
        promote: bool,
        /// Record a green attempt without promoting it (`harness promote`
        /// does that later, explicitly)
        #[arg(long)]
        no_promote: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Record a NEW sample when this live attempt already finished
        #[arg(long)]
        retry: bool,
        /// Pin the recorded attempt a `--provider replay` run verifies
        #[arg(long)]
        attempt: Option<String>,
    },
    /// Promote a recorded green migrate attempt into the unit's crate and
    /// verify it in place (the explicit act a review's Accept is)
    Promote {
        /// Unit id from plan.toml
        unit: String,
        /// The attempt id (`a-…`, or `a-….r2` for a later sample)
        attempt: String,
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Replace the crate of an already verified unit, or re-promote an
        /// attempt that is already promoted
        #[arg(long)]
        replace: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
    },
    /// Benchmark suites: vendor, verify, score, regression-check
    Bench {
        #[command(subcommand)]
        cmd: bench::BenchCmd,
    },
    /// Generate a unit's differential driver through the configured LLM
    /// provider and self-validate it against the original C
    GenDriver {
        /// Unit id from plan.toml
        unit: String,
        /// Target repository root
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Provider profile override (built-in or user-level profile name)
        #[arg(long)]
        provider: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
        /// Replace the unit's existing GENERATED driver with a new green one
        #[arg(long)]
        promote: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Record a NEW sample when this live attempt already finished
        #[arg(long)]
        retry: bool,
        /// Pin the recorded attempt a `--provider replay` run verifies
        #[arg(long)]
        attempt: Option<String>,
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

/// Print a line to stdout (or, under `--json`, a `message` event), ignoring
/// EPIPE (`harness ... | head` must exit with a documented code, not a
/// broken-pipe panic).
pub(crate) fn out(line: String) {
    report::line(line);
}

/// Take the ledger's writer lock for a writing command
/// (docs/CLI-HARDENING.md §1); another holder is a `locked` error.
pub(crate) fn lock_ledger(ledger: &Ledger, command: &str) -> Result<WriterLock> {
    Ok(WriterLock::acquire(ledger, command)?)
}

/// On SIGINT/SIGTERM/SIGHUP: kill every live sandboxed process group, emit
/// the events-mode `result`, then die BY the signal (a normal exit 130
/// would make an interactive shell continue a loop over units). A failure
/// to install the handler leaves today's behaviour (the harness dies, the
/// children run to their own timeout) and is reported on stderr.
fn install_signal_handler() {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use std::io::IsTerminal;
    // SIGHUP only for the case it is registered for — a terminal (or tmux
    // pane) still attached to our output. `nohup` sets SIGHUP to SIG_IGN and
    // redirects stdout/stderr away from the terminal, and installing a
    // handler would silently override that inherited disposition (std cannot
    // read it without `unsafe`); a detached run keeps its own.
    let mut sigs = vec![SIGINT, SIGTERM];
    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        sigs.push(SIGHUP);
    }
    let mut signals = match signal_hook::iterator::Signals::new(&sigs) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(
                std::io::stderr(),
                "harness: cannot install the signal handler: {e}"
            );
            return;
        }
    };
    std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            let name = match sig {
                SIGINT => "SIGINT",
                SIGTERM => "SIGTERM",
                _ => "SIGHUP",
            };
            let killed = harness_oracle::kill_live_process_groups();
            // Courtesy output only (docs/CLI-HARDENING.md §3 "best-effort"):
            // the main thread may be parked in a write on a full stdout pipe
            // holding the stdout mutex, and a closed stderr would make
            // `eprintln!` panic — neither may delay or prevent dying BY the
            // signal. Write from a helper, wait a bounded time, re-raise.
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let helper = std::thread::Builder::new().spawn(move || {
                let _ = writeln!(
                    std::io::stderr(),
                    "harness: {name} — killed {killed} live process group(s); terminating"
                );
                report::result(128 + sig, Some(name));
                let _ = tx.send(());
            });
            if helper.is_ok() {
                let _ = rx.recv_timeout(std::time::Duration::from_millis(250));
            }
            let _ = signal_hook::low_level::emulate_default_handler(sig);
            std::process::exit(128 + sig);
        }
    });
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    install_signal_handler();
    if cli.json {
        report::init(report::Mode::Json);
        let _ = harness_llm::progress::install(Box::new(report::Progress));
        let argv: Vec<String> = std::env::args().skip(1).filter(|a| a != "--json").collect();
        let command = argv.first().cloned().unwrap_or_default();
        report::header(&command, argv.get(1..).unwrap_or(&[]));
    }
    let result = match cli.cmd {
        Cmd::Scan { target } => cmd_scan(target),
        Cmd::Plan { target } => cmd_plan(target),
        Cmd::Verify {
            unit,
            target,
            allow_unsandboxed,
        } => cmd_verify(unit, target, allow_unsandboxed),
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
        Cmd::Migrate {
            unit,
            target,
            provider,
            model,
            promote,
            no_promote,
            allow_unsandboxed,
            retry,
            attempt,
        } => cmd_migrate(MigrateArgs {
            unit,
            target,
            provider,
            model,
            promote,
            no_promote,
            allow_unsandboxed,
            retry,
            attempt,
        }),
        Cmd::Promote {
            unit,
            attempt,
            target,
            replace,
            allow_unsandboxed,
        } => promote::cmd_promote(unit, attempt, target, replace, allow_unsandboxed),
        Cmd::SyncRuntime { target, check } => cmd_sync_runtime(target, check),
        Cmd::Bench { cmd } => bench::run(cmd),
        Cmd::GenDriver {
            unit,
            target,
            provider,
            model,
            promote,
            allow_unsandboxed,
            retry,
            attempt,
        } => gen_driver::cmd_gen_driver(gen_driver::GenDriverArgs {
            unit,
            target,
            provider,
            model,
            promote,
            allow_unsandboxed,
            retry,
            attempt,
        }),
    };
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            // A closed stderr must not turn exit 1 into a panic (101).
            let _ = writeln!(std::io::stderr(), "error: {e:#}");
            report::error(&e);
            1
        }
    };
    report::result(i32::from(code), None);
    ExitCode::from(code)
}

fn cmd_scan(target: PathBuf) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "scan")?;
    let facts = scan_target(&ctx)?;
    out(format!(
        "scan: {} files, {} symbols, {} refs -> {}",
        facts.files.len(),
        facts.symbols.len(),
        facts.refs.len(),
        ledger.facts_path().display()
    ));
    Ok(0)
}

/// Scan `ctx` and write `facts.jsonl` (the body of `harness scan`).
pub(crate) fn scan_target(ctx: &TargetContext) -> Result<Facts> {
    let ledger = Ledger::new(&ctx.root);
    let facts = harness_scan::CFrontend.scan(ctx)?;
    std::fs::create_dir_all(ledger.dir()).context("creating migration dir")?;
    facts.store(&ledger.facts_path())?;
    Ok(facts)
}

/// The typed refusal for stale facts (`error.kind = "stale"`).
fn facts_stale(stale: usize) -> Error {
    Error::Stale {
        subject: "facts.jsonl".into(),
        hint: format!("{stale} file(s) changed on disk; run `harness scan` first"),
    }
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

fn cmd_plan(target: PathBuf) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let _lock = lock_ledger(&Ledger::new(&ctx.root), "plan")?;
    let (changes, order_line, units) = plan_target(&ctx)?;
    if changes.is_empty() {
        out(format!("plan: no changes ({units} units)"));
    } else {
        for c in &changes {
            out(format!("plan: {c}"));
        }
    }
    out(format!("plan: execution order: {order_line}"));
    Ok(0)
}

/// Reconcile and write `plan.toml` (the body of `harness plan`): returns the
/// change lines, the execution-order line and the unit count.
pub(crate) fn plan_target(ctx: &TargetContext) -> Result<(Vec<String>, String, usize)> {
    let ledger = Ledger::new(&ctx.root);
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    // Planning from stale facts would write stale hashes and strand verify
    // in a refusal loop — refuse up front instead.
    let stale = stale_fact_files(ctx, &facts);
    if stale > 0 {
        return Err(facts_stale(stale).into());
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
    Ok((changes, order_line, computed.len()))
}

/// Every command that builds or runs target- or model-derived code refuses
/// on platforms without a sandbox unless the user explicitly accepts the risk
/// (docs/SCHEMAS.md "Trust boundaries") — regardless of provider: an
/// `external` candidate and the target's own driver.c run just the same.
pub(crate) fn require_sandbox(allow_unsandboxed: bool, what: &str) -> Result<()> {
    if harness_oracle::sandbox_mode() == "none" && !allow_unsandboxed {
        bail!(
            "no sandbox is available on this platform; `{what}` builds and runs target- and \
             model-derived code unconfined — pass --allow-unsandboxed to accept that"
        );
    }
    Ok(())
}

/// A ledger directory that is guaranteed not to be (or pass through) a
/// symlink: created level by level under the canonical target root, refusing
/// any component that is not a real directory. Target-owned trees are hostile
/// — a committed `traces -> /elsewhere` must not redirect harness writes.
pub(crate) fn safe_ledger_dir(root: &std::path::Path, components: &[&str]) -> Result<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in components {
        cur = cur.join(comp);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                bail!(
                    "{} is not a real directory; refusing to write through it",
                    cur.display()
                )
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).with_context(|| format!("creating {}", cur.display()))?;
            }
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", cur.display())),
        }
    }
    Ok(cur)
}

fn cmd_verify(unit_id: String, target: PathBuf, allow_unsandboxed: bool) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness verify")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, &format!("verify {unit_id}"))?;
    let plan_path = ledger.plan_path();
    let plan_doc = Plan::load(&plan_path)?;
    // Structural validation on every load (docs/SCHEMAS.md): never run the
    // oracle against a plan with duplicate ids, missing deps, or cycles.
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before verifying")?;
    let unit = plan_doc.unit(&unit_id)?;
    promote::recover_promotion(&ctx, &ledger, unit)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;

    // Stale-plan refusal (docs/SCHEMAS.md): the tree must match what was planned.
    let closure = facts.include_closure(&unit.files);
    let current = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    if current != unit.source_hash {
        return Err(Error::Stale {
            subject: format!("unit `{unit_id}`"),
            hint: format!(
                "source changed since planning (plan {} vs tree {current}); run `harness scan`, \
                 then `harness plan`, review the diff, then re-verify",
                unit.source_hash
            ),
        }
        .into());
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
    report::verdict(&unit_id, &verdict, &ledger.verdict_latest_path(&unit_id));
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
        Ok(0)
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
        Ok(EXIT_ORACLE_RED)
    }
}

fn cmd_status(target: PathBuf) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);

    let facts = match Facts::load(&ledger.facts_path()) {
        Ok(f) => f,
        Err(e) if e.is_not_found() => {
            out("status: no facts — run `harness scan`".into());
            return Ok(0);
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
    #[derive(serde::Serialize)]
    struct FactsEvent {
        k: &'static str,
        files: usize,
        stale: usize,
    }
    report::event(&FactsEvent {
        k: "facts",
        files: facts.files.len(),
        stale: stale_files,
    });

    let plan_path = ledger.plan_path();
    if !plan_path.exists() {
        out("status: no plan — run `harness plan`".into());
        return Ok(0);
    }
    let plan_doc = Plan::load(&plan_path)?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid")?;
    #[derive(serde::Serialize)]
    struct UnitEvent<'a> {
        k: &'static str,
        #[serde(flatten)]
        report: &'a harness_core::status::UnitReport,
    }
    for unit in &plan_doc.units {
        // One computation renders both surfaces (docs/CLI-HARDENING.md §4).
        let r = harness_core::status::unit_report(&ctx, &ledger, &facts, unit)?;
        out(r.render_line());
        if let Some(line) = r.render_attempts_line() {
            out(line);
        }
        report::event(&UnitEvent {
            k: "unit",
            report: &r,
        });
    }
    Ok(0)
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

fn cmd_detect(target: PathBuf) -> Result<u8> {
    use harness_core::observer::{FindingsFile, ObserverPaths};
    use harness_core::traits::Detector;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "detect")?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let stale = stale_fact_files(&ctx, &facts);
    if stale > 0 {
        return Err(facts_stale(stale).into());
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
    Ok(0)
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
        return Err(Error::Stale {
            subject: "findings.jsonl".into(),
            hint: "it is bound to different facts; run `harness detect`".into(),
        }
        .into());
    }
    for f in &findings.findings {
        let now = hash::file_hash(&ctx.root.join(&f.file))
            .with_context(|| format!("hashing {}", f.file))?;
        if now != f.file_hash {
            return Err(Error::Stale {
                subject: format!("finding {}", f.id),
                hint: format!("{} changed since detect; run `harness detect`", f.file),
            }
            .into());
        }
    }
    let annotations = observer::load_annotations(&ObserverPaths::annotations(ledger))?;
    let triage = harness_core::observer::TriageFile::load(&ObserverPaths::triage(ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(ledger))?;
    Ok((facts, plan_doc, findings, annotations, triage, reviews))
}

fn cmd_observe(target: PathBuf) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "observe")?;
    let (facts, plan_doc, findings, annotations, _, reviews) = observer_inputs(&ctx, &ledger)?;

    let traces = safe_ledger_dir(&ctx.root, &["migration", "observer", "traces"])?;
    let llm = &ctx.config.llm;
    let resolved = harness_llm::providers::resolve(&llm.provider, &traces)?;
    let outcome = match harness_llm::run_triage(
        &resolved,
        &llm.model,
        llm.max_tokens,
        &ctx,
        &facts,
        &plan_doc,
        &findings,
        &traces,
    ) {
        Ok(o) => o,
        Err(e @ Error::Awaiting { .. }) => {
            eprintln!("{e:#}");
            eprintln!(
                "observe: external provider mode — supply the response file(s) under {} and re-run",
                traces.display()
            );
            if let Error::Awaiting { path, .. } = &e {
                report::event(&report::Awaiting {
                    k: "awaiting",
                    attempt: None,
                    path: path.display().to_string(),
                    resume: format!("harness observe --target {}", target.display()),
                });
            }
            return Err(e.into());
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
        resolved.adapter.name(),
        ObserverPaths::observations(&ledger).display(),
        usage_in,
        usage_out
    ));
    for r in risk.iter().take(5) {
        out(format!("observe: risk {} {}", r.score, r.unit));
    }
    Ok(0)
}

fn cmd_review(
    finding: String,
    uphold_dismiss: bool,
    reinstate: bool,
    note: String,
    target: PathBuf,
) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    if uphold_dismiss == reinstate {
        bail!("pass exactly one of --uphold-dismiss or --reinstate");
    }
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, &format!("review {finding}"))?;
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
    Ok(0)
}

fn cmd_sync_runtime(target: PathBuf, check: bool) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = if check {
        None
    } else {
        Some(lock_ledger(&ledger, "sync-runtime")?)
    };
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
            return Ok(0);
        }
        eprintln!(
            "sync-runtime: AGENTS.md managed block is out of date; run `harness sync-runtime`"
        );
        return Ok(1);
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
    Ok(0)
}

// ---------- M3: executor command ----------

/// The unit's confirmed hazards as `harness migrate` puts them in the
/// translate prompt (annotations are implicitly confirmed). Shared with
/// `bench check --replay`, which must pose the byte-identical prompt.
pub(crate) fn confirmed_hazards(
    ledger: &Ledger,
    plan_doc: &Plan,
    facts: &Facts,
    unit_id: &str,
) -> Result<Vec<harness_core::observer::Finding>> {
    use harness_core::observer::{self, FindingState, ObserverPaths};
    let mut hazards = Vec::new();
    let findings = match observer::FindingsFile::load(&ObserverPaths::findings(ledger)) {
        Ok(f) => f.findings,
        Err(e) if e.is_not_found() => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let annotations = observer::load_annotations(&ObserverPaths::annotations(ledger))?;
    let triage = observer::TriageFile::load(&ObserverPaths::triage(ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(ledger))?;
    for f in findings.iter().chain(annotations.iter()) {
        let affects = observer::affected_units(&f.file, plan_doc, facts).contains(&unit_id);
        let state = observer::finding_state(f, &triage, &reviews);
        if affects && matches!(state, FindingState::Confirmed | FindingState::Reinstated) {
            hazards.push(f.clone());
        }
    }
    Ok(hazards)
}

/// Arguments of `harness migrate`.
struct MigrateArgs {
    unit: String,
    target: PathBuf,
    provider: Option<String>,
    model: Option<String>,
    promote: bool,
    no_promote: bool,
    allow_unsandboxed: bool,
    retry: bool,
    attempt: Option<String>,
}

impl MigrateArgs {
    /// The exact command line that resumes this run (the `awaiting` hint
    /// and event carry it, so a resume keeps the same promotion flags).
    fn resume_command(&self) -> String {
        let mut cmd = format!("harness migrate {}", self.unit);
        cmd.push_str(&format!(" --target {}", self.target.display()));
        if let Some(p) = &self.provider {
            cmd.push_str(&format!(" --provider {p}"));
        }
        if let Some(m) = &self.model {
            cmd.push_str(&format!(" --model {m}"));
        }
        if self.promote {
            cmd.push_str(" --promote");
        }
        if self.no_promote {
            cmd.push_str(" --no-promote");
        }
        if self.allow_unsandboxed {
            cmd.push_str(" --allow-unsandboxed");
        }
        if self.retry {
            cmd.push_str(" --retry");
        }
        if let Some(a) = &self.attempt {
            cmd.push_str(&format!(" --attempt {a}"));
        }
        cmd
    }
}

fn cmd_migrate(args: MigrateArgs) -> Result<u8> {
    let resume = args.resume_command();
    let MigrateArgs {
        unit: unit_id,
        target,
        provider: provider_flag,
        model: model_flag,
        promote: promote_flag,
        no_promote,
        allow_unsandboxed,
        retry,
        attempt,
    } = args;
    require_sandbox(allow_unsandboxed, "harness migrate")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, &format!("migrate {unit_id}"))?;
    let plan_path = ledger.plan_path();
    let plan_doc = Plan::load(&plan_path)?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before migrating")?;
    let unit = plan_doc.unit(&unit_id)?;
    promote::recover_promotion(&ctx, &ledger, unit)?;

    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    // The plan's staleness rule and R6 (a generated driver must carry a
    // FRESH green validation) — shared with `harness promote`.
    promote::migrate_preconditions(&ctx, &ledger, &facts, unit, "migrate")?;

    // Stage routing (§13.2): flag > [llm.migrate] > [llm].
    let llm = &ctx.config.llm;
    let stage = llm.migrate.as_ref();
    let provider_name = provider_flag
        .or_else(|| stage.and_then(|m| m.provider.clone()))
        .unwrap_or_else(|| llm.provider.clone());
    let model = model_flag
        .or_else(|| stage.and_then(|m| m.model.clone()))
        .unwrap_or_else(|| llm.model.clone());
    let max_tokens = stage.and_then(|m| m.max_tokens).unwrap_or(llm.max_tokens);
    let max_repairs = stage.and_then(|m| m.max_repairs).unwrap_or(3);
    let promote_on_green = stage.and_then(|m| m.promote_on_green).unwrap_or(true);

    let traces = safe_ledger_dir(&ctx.root, &["migration", "units", &unit_id, "traces"])?;
    let resolved = harness_llm::providers::resolve(&provider_name, &traces)?;

    let hazards = confirmed_hazards(&ledger, &plan_doc, &facts, &unit_id)?;

    let oracle = harness_oracle::CAbiDifferential;
    let params = harness_llm::migrate::MigrateParams {
        provider: &resolved,
        model: &model,
        max_tokens,
        max_repairs,
        traces_dir: &traces,
        retry,
        attempt: attempt.as_deref(),
    };
    let outcome = match harness_llm::migrate::run_migration(
        &params, &oracle, &ctx, &facts, &plan_doc, unit, &hazards,
    ) {
        Ok(o) => o,
        Err(e @ Error::Awaiting { .. }) => {
            eprintln!("{e:#}");
            eprintln!(
                "migrate: external provider mode — supply the response file under {} and re-run: \
                 {resume}",
                traces.display()
            );
            if let Error::Awaiting { path, attempt } = &e {
                report::event(&report::Awaiting {
                    k: "awaiting",
                    attempt: attempt.as_deref(),
                    path: path.display().to_string(),
                    resume: resume.clone(),
                });
            }
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };
    let record = &outcome.record;
    for (i, t) in record.turns.iter().enumerate() {
        out(format!(
            "migrate: turn {} {} -> {} (tokens in/out: {}/{})",
            i + 1,
            t.kind,
            t.result,
            t.input_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into()),
            t.output_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into()),
        ));
    }
    out(format!(
        "migrate: {} attempt {} via `{}` ({}) model `{}` -> {}",
        unit_id,
        record.id,
        record.provider,
        record.provider_kind,
        record.model,
        record.outcome.to_uppercase()
    ));
    if let Some(drifted) = &outcome.drifted {
        out(format!(
            "migrate: verified from its recorded evidence; prompt: {}",
            harness_llm::conformance(drifted)
        ));
    }
    // docs/CLI-HARDENING.md §4: migrate's final verdict — one `check` per
    // check and the `verdict` line — when the last turn was judged by the
    // oracle in this run (a format/truncated/blocked/deny-scan tail stored
    // none; a verification stores nothing).
    if let Some(v) = &outcome.verdict {
        report::verdict(
            &unit_id,
            v,
            &outcome.attempt_dir.join("attempt-verdict.json"),
        );
    }
    let attempt_event = |promoted: bool, promotion: &str| {
        report::event(&report::AttemptEvent {
            k: "attempt",
            unit: &unit_id,
            id: &record.id,
            outcome: &record.outcome,
            provider: &record.provider,
            model: &record.model,
            promoted,
            promotion,
        });
    };
    if record.outcome != "green" {
        attempt_event(record.promoted, "not promoted: not green");
        return Ok(EXIT_ORACLE_RED);
    }

    // Promotion (docs/SCHEMAS.md; docs/CLI-HARDENING.md §2). Precedence:
    // --promote > --no-promote > [llm.migrate] promote_on_green > default;
    // never from a replay run (which writes nothing to the ledger).
    let already_done = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
    let (do_promote, reason) = if resolved.kind == "replay" {
        (false, "replay run")
    } else if promote_flag {
        (true, "--promote")
    } else if no_promote {
        (false, "--no-promote")
    } else if already_done {
        (false, "unit already verified — pass --promote to replace")
    } else if !promote_on_green {
        (false, "promote_on_green = false — run `harness promote`")
    } else {
        (true, "default")
    };
    let (Some(candidate), true) = (outcome.candidate_dir.as_ref(), do_promote) else {
        out(format!(
            "migrate: green attempt recorded; not promoted ({reason})"
        ));
        attempt_event(record.promoted, &format!("not promoted: {reason}"));
        return Ok(0);
    };
    match promote::promote_attempt(&ctx, &ledger, &oracle, unit, record, candidate)? {
        promote::Promotion::Verified => {
            out(format!(
                "migrate: {unit_id} promoted and verified — status set to verified"
            ));
            attempt_event(true, &format!("promoted: {reason}"));
            Ok(0)
        }
        promote::Promotion::RolledBack => {
            out("migrate: promoted candidate did not verify in place — rolled back".into());
            attempt_event(false, "not promoted: red in place — rolled back");
            Ok(EXIT_ORACLE_RED)
        }
    }
}
