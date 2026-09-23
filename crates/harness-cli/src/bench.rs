//! `harness bench …` — benchmark suites (docs/SCHEMAS.md "M4 additions").
//!
//! A suite dir (e.g. `targets/tractor/`) holds `suite.toml`, `corpus.lock`,
//! one harness target per case under `cases/`, the held-out scoring material
//! under `heldout/`, and the committed `scores.json`.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use harness_core::bench::{
    classify_case, compare, CaseInputs, CasePipeline, CaseScore, CorpusLock, Counts, Scores, Suite,
    SuiteCase, VectorScore, SCORES_SCHEMA_NAME, SCORES_SCHEMA_VERSION,
};
use harness_core::driver::DriverValidation;
use harness_core::ledger::Ledger;
use harness_core::plan as plan_mod;
use harness_core::traits::OracleStrategy;
use harness_core::UnitStatus;
use harness_core::{attempts, hash, Facts, Plan, TargetContext};
use harness_oracle::bench::{CSide, Scorer};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::out;

#[derive(Subcommand)]
pub enum BenchCmd {
    /// Vendor the suite from a checkout of the pinned upstream commit and
    /// (re)write suite.toml's derived case list and corpus.lock
    Vendor {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
        /// Upstream checkout, detached at the pinned commit
        #[arg(long)]
        from: PathBuf,
    },
    /// Verify every vendored file against corpus.lock
    VerifyCorpus {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
    },
    /// Make every case a harness target: harness.toml, scan, plan, and the
    /// unit's [unit.oracle] table where absent (idempotent)
    Init {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
        /// Report drift from freshly generated config/tables; write nothing
        #[arg(long)]
        check: bool,
    },
    /// Per-case pipeline progress from the case ledgers (no builds)
    Status {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
    },
    /// Score every case (or the named ones) with the corpus's own runner on
    /// the held-out vectors: C baseline, verified Rust, latest unverified
    /// candidate
    Score {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
        /// Only these cases (upstream path or case dir name); repeatable
        #[arg(long = "case")]
        cases: Vec<String>,
        /// Record the result as the suite's scores.json (whole suite only)
        #[arg(long)]
        write: bool,
    },
    /// Regression check against the committed scores.json: re-verify every
    /// verified unit, re-validate every generated driver, re-score, compare
    /// per vector (exit 10 on a regression, 1 if incomparable)
    Check {
        /// Suite dir (contains suite.toml)
        #[arg(long, default_value = ".")]
        suite: PathBuf,
        /// Also replay-verify every recorded driver and migrate attempt from
        /// its traces (zero tokens; slower)
        #[arg(long)]
        replay: bool,
    },
}

pub fn run(cmd: BenchCmd) -> Result<ExitCode> {
    match cmd {
        BenchCmd::Vendor { suite, from } => cmd_vendor(&suite, &from),
        BenchCmd::VerifyCorpus { suite } => cmd_verify_corpus(&suite),
        BenchCmd::Init { suite, check } => cmd_init(&suite, check),
        BenchCmd::Status { suite } => cmd_status(&suite),
        BenchCmd::Score {
            suite,
            cases,
            write,
        } => cmd_score(&suite, &cases, write),
        BenchCmd::Check { suite, replay } => cmd_check(&suite, replay),
    }
}

/// Load `suite.toml` and `corpus.lock` and require the corpus to verify.
pub fn load_verified(suite_dir: &Path) -> Result<(Suite, CorpusLock)> {
    let suite = Suite::load(&suite_dir.join("suite.toml"))?;
    let lock = CorpusLock::load(&suite_dir.join("corpus.lock"))?;
    lock.require_verified(suite_dir, &suite)?;
    Ok((suite, lock))
}

fn cmd_vendor(suite_dir: &Path, from: &Path) -> Result<ExitCode> {
    let suite = Suite::load(&suite_dir.join("suite.toml"))?;
    let (derived, lock) = harness_core::bench::vendor(suite_dir, &suite, from)?;
    derived.store(&suite_dir.join("suite.toml"))?;
    lock.store(&suite_dir.join("corpus.lock"))?;
    let violations = lock.verify(suite_dir, &derived)?;
    if !violations.is_empty() {
        bail!(
            "vendored tree does not verify ({} violation(s)); first: {}",
            violations.len(),
            violations[0]
        );
    }
    out(format!(
        "bench vendor: {} case(s), {} excluded, {} locked file(s) from {}@{}",
        derived.cases.len(),
        derived.excluded.len(),
        lock.files.len(),
        derived.upstream.tag,
        &derived.upstream.commit[..12]
    ));
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify_corpus(suite_dir: &Path) -> Result<ExitCode> {
    let (suite, lock) = load_verified(suite_dir)?;
    out(format!(
        "bench verify-corpus: OK — {} locked file(s), {} case(s), {}@{}",
        lock.files.len(),
        suite.cases.len(),
        suite.upstream.tag,
        &suite.upstream.commit[..12]
    ));
    Ok(ExitCode::SUCCESS)
}

/// The generated `harness.toml` of a case (docs/SCHEMAS.md "M4 additions").
/// `include_dirs` lists `test_case/include` when the upstream case has one.
fn case_config(case: &SuiteCase, has_include: bool) -> String {
    let include = if has_include {
        "include_dirs = [\"test_case/include\"]\n"
    } else {
        ""
    };
    format!(
        "# Generated by `harness bench init` for suite case {path}.\n\
         # Regenerate rather than hand-edit (`harness bench init --check` reports drift).\n\
         schema_version = 1\n\n\
         [target]\n\
         name = \"{name}\"\n\
         source_dir = \"test_case\"\n\
         {include}\n\
         [oracle]\n\
         allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n\n\
         [llm]\n\
         provider = \"external\"\n\
         max_tokens = 16384\n",
        path = case.path,
        name = case.name(),
    )
}

fn cmd_init(suite_dir: &Path, check: bool) -> Result<ExitCode> {
    let (suite, _lock) = load_verified(suite_dir)?;
    let mut drift: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut written = 0usize;
    for case in &suite.cases {
        let root = case.target_root(suite_dir);
        let has_include = root.join("test_case/include").is_dir();
        let config = case_config(case, has_include);
        let config_path = root.join("harness.toml");
        let current = std::fs::read_to_string(&config_path).ok();
        if current.as_deref() != Some(config.as_str()) {
            if check {
                drift.push(format!(
                    "{}: harness.toml differs from generated",
                    case.path
                ));
                continue;
            }
            harness_core::ledger::write_atomic(&config_path, config.as_bytes())?;
            written += 1;
        }
        if check {
            continue;
        }
        // A case the pipeline cannot take on (e.g. a frontend limitation) is
        // a FINDING: it stays in the suite and scores as not attempted.
        if let Err(e) = init_case(&root, case) {
            failed.push(format!("{}: {e:#}", case.path));
        }
    }
    if check {
        for d in &drift {
            out(format!("bench init --check: {d}"));
        }
        if !drift.is_empty() {
            return Ok(ExitCode::FAILURE);
        }
        out(format!(
            "bench init --check: {} case(s) match",
            suite.cases.len()
        ));
        return Ok(ExitCode::SUCCESS);
    }
    for f in &failed {
        out(format!("bench init: NOT initialized — {f}"));
    }
    out(format!(
        "bench init: {} of {} case(s) initialized ({written} harness.toml written); {} not \
         initialized (they stay in the suite and score as not attempted)",
        suite.cases.len() - failed.len(),
        suite.cases.len(),
        failed.len()
    ));
    Ok(ExitCode::SUCCESS)
}

/// Scan + plan one case target and give each unit its default oracle table.
fn init_case(root: &Path, case: &SuiteCase) -> Result<()> {
    let ctx = TargetContext::load(root)?;
    crate::scan_target(&ctx)?;
    crate::plan_target(&ctx)?;
    let ledger = Ledger::new(&ctx.root);
    let plan = Plan::load(&ledger.plan_path())?;
    // R3: the runner's symbol must be one the plan migrates.
    if !plan.units.iter().any(|u| u.symbols.contains(&case.symbol)) {
        bail!(
            "runner symbol `{}` is not a public symbol of any planned unit (the C frontend \
             found {} unit(s))",
            case.symbol,
            plan.units.len()
        );
    }
    for unit in &plan.units {
        plan_mod::set_oracle_table_if_absent(
            &ledger.plan_path(),
            &unit.id,
            &crate::gen_driver::default_oracle_entries(&unit.id, &unit.files),
        )?;
    }
    Ok(())
}

/// Driver state of a unit: `validated` (fresh green record), `stale`,
/// `failed` (record exists, red), or `missing`.
pub fn driver_state(
    ctx: &TargetContext,
    facts: &Facts,
    unit: &harness_core::Unit,
) -> Result<String> {
    let ledger = Ledger::new(&ctx.root);
    let path = ledger.driver_validation_path(&unit.id);
    if !path.exists() {
        return Ok("missing".into());
    }
    let v = DriverValidation::load(&path)?;
    if !v.green {
        return Ok("failed".into());
    }
    let closure = facts.include_closure(&unit.files);
    let unit_source = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    let driver_path = ledger.driver_path(&unit.id);
    let driver = if driver_path.exists() {
        hash::file_hash(&driver_path)?
    } else {
        String::new()
    };
    Ok(if v.is_fresh_green(&unit_source, &driver) {
        "validated".into()
    } else {
        "stale".into()
    })
}

fn cmd_status(suite_dir: &Path) -> Result<ExitCode> {
    let (suite, _lock) = load_verified(suite_dir)?;
    let mut tally: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for case in &suite.cases {
        let root = case.target_root(suite_dir);
        let line = match case_status(&root) {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        };
        *tally
            .entry(line.split(' ').next().unwrap_or("").to_string())
            .or_default() += 1;
        out(format!("{} [{}] {line}", case.path, case.split));
    }
    let summary: Vec<String> = tally.iter().map(|(k, v)| format!("{k}={v}")).collect();
    out(format!(
        "bench status: {} case(s): {}",
        suite.cases.len(),
        summary.join(" ")
    ));
    Ok(ExitCode::SUCCESS)
}

/// One case's pipeline line: `<unit-status> driver=<state> driver-attempts=N migrate-attempts=N (<last outcome>)`.
fn case_status(root: &Path) -> Result<String> {
    if !root.join("harness.toml").exists() {
        return Ok("uninitialized".into());
    }
    let ctx = TargetContext::load(root)?;
    let ledger = Ledger::new(&ctx.root);
    if !ledger.plan_path().exists() || !ledger.facts_path().exists() {
        return Ok("unplanned".into());
    }
    if Plan::load(&ledger.plan_path())?.units.is_empty() {
        return Ok("no-units (frontend found no unit)".into());
    }
    let facts = Facts::load(&ledger.facts_path())?;
    let plan = Plan::load(&ledger.plan_path())?;
    let mut parts = Vec::new();
    for unit in &plan.units {
        let d = attempts::load_unit_driver_attempts(&ledger, &unit.id)?;
        let m = attempts::load_unit_attempts(&ledger, &unit.id)?;
        let last = |v: &[attempts::AttemptRecord]| {
            v.last()
                .map(|r| r.outcome.clone())
                .unwrap_or_else(|| "-".into())
        };
        parts.push(format!(
            "{} {} driver={} driver-attempts={}({}) migrate-attempts={}({})",
            unit.status.as_str(),
            unit.id,
            driver_state(&ctx, &facts, unit)?,
            d.len(),
            last(&d),
            m.len(),
            last(&m)
        ));
    }
    Ok(parts.join("; "))
}

// ------------------------------------------------------------------ scoring

fn tally(results: &[&str]) -> Counts {
    let mut c = Counts::default();
    for r in results {
        match *r {
            "pass" => c.pass += 1,
            "skip" => c.skip += 1,
            "not-run" => c.not_run += 1,
            _ => c.fail += 1,
        }
    }
    c
}

/// Everything scored for one case, plus problems a `check` must fail on.
struct Scored {
    score: CaseScore,
    problems: Vec<String>,
}

/// Score one case. `recheck`: also re-run the oracle on a verified unit and
/// `validate_driver` on a generated driver (never trusting committed
/// records — R6/R10); a red there is a PROBLEM for `bench check`.
fn score_one(scorer: &Scorer, suite_dir: &Path, case: &SuiteCase, recheck: bool) -> Result<Scored> {
    let root = case.target_root(suite_dir).canonicalize()?;
    let mut problems = Vec::new();
    // The C side: every .c of the case, with the case's include dirs.
    let src_dir = root.join("test_case/src");
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&src_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("c"))
        .collect();
    sources.sort();
    let mut include_dirs = Vec::new();
    for d in ["test_case/include", "test_case/src"] {
        if root.join(d).is_dir() {
            include_dirs.push(root.join(d));
        }
    }
    let cside = CSide {
        sources,
        include_dirs,
    };

    let mut pipeline = CasePipeline::default();
    let mut inputs = CaseInputs::default();
    let mut verified = false;
    let mut rust_lib: Option<PathBuf> = None;
    let mut candidate_lib: Option<PathBuf> = None;
    if root.join("harness.toml").exists() && Ledger::new(&root).plan_path().exists() {
        let ctx = TargetContext::load(&root)?;
        let ledger = Ledger::new(&ctx.root);
        let plan = Plan::load(&ledger.plan_path())?;
        let facts = Facts::load(&ledger.facts_path())?;
        if let Some(unit) = plan.units.iter().find(|u| u.symbols.contains(&case.symbol)) {
            pipeline.unit = unit.id.clone();
            pipeline.status = unit.status.as_str().to_string();
            pipeline.driver = driver_state(&ctx, &facts, unit)?;
            let closure = facts.include_closure(&unit.files);
            inputs.unit_source = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
            let driver_path = ledger.driver_path(&unit.id);
            if driver_path.exists() {
                inputs.driver = hash::file_hash(&driver_path)?;
            }
            let vpath = ledger.driver_validation_path(&unit.id);
            if vpath.exists() {
                inputs.validation = hash::file_hash(&vpath)?;
                let v = DriverValidation::load(&vpath)?;
                pipeline.mutation = v.mutation.as_ref().map(|m| (m.killed, m.compiled));
                if recheck && driver_path.exists() {
                    let again = harness_oracle::validate_driver(&ctx, unit, &driver_path)?;
                    if !again.green {
                        problems.push(format!("{}: driver no longer validates", case.path));
                    }
                }
            }
            let d_attempts = attempts::load_unit_driver_attempts(&ledger, &unit.id)?;
            pipeline.driver_turns = d_attempts
                .iter()
                .find(|r| r.outcome == "green" && r.candidate_digest == inputs.driver)
                .map(|r| r.turns.len() as u32);
            let m_attempts = attempts::load_unit_attempts(&ledger, &unit.id)?;
            pipeline.migrate_turns = m_attempts
                .iter()
                .find(|r| r.promoted && r.outcome == "green")
                .map(|r| r.turns.len() as u32);
            pipeline.migrate_outcome = m_attempts
                .last()
                .map(|r| r.outcome.clone())
                .unwrap_or_default();
            verified = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
            if verified {
                let crate_name = unit.oracle_param_str("rust_crate").unwrap_or_default();
                let crate_dir = ledger.unit_dir(&unit.id).join(crate_name);
                inputs.rust_crate = hash::unit_crate_file_set_hash(&ctx.root, &crate_dir)?;
                if recheck {
                    let v = harness_oracle::CAbiDifferential.verify(&ctx, unit)?;
                    if !v.green {
                        problems.push(format!("{}: verified unit's oracle is RED", case.path));
                    }
                }
                rust_lib = harness_oracle::build_crate_staticlib(&ctx, &crate_dir).ok();
            } else if let Some(last) = m_attempts.iter().rfind(|r| !r.candidate_digest.is_empty()) {
                let dir = attempts::attempt_dir(&ledger, &unit.id, &last.id).join("candidate");
                if dir.is_dir() {
                    inputs.candidate = last.candidate_digest.clone();
                    candidate_lib = harness_oracle::build_crate_staticlib(&ctx, &dir).ok();
                }
            }
        }
    }
    let runs = scorer.score_case(case, &cside, rust_lib.as_deref(), candidate_lib.as_deref())?;
    let rust_missing = if verified { "fail:build" } else { "not-run" };
    let vectors: Vec<VectorScore> = runs
        .iter()
        .map(|r| VectorScore {
            name: r.name.clone(),
            c: r.c.clone(),
            rust: r.rust.clone().unwrap_or_else(|| rust_missing.to_string()),
            candidate: if inputs.candidate.is_empty() {
                None
            } else {
                Some(
                    r.candidate
                        .clone()
                        .unwrap_or_else(|| "fail:build".to_string()),
                )
            },
        })
        .collect();
    let class = classify_case(&vectors, verified).to_string();
    let c_baseline = tally(&vectors.iter().map(|v| v.c.as_str()).collect::<Vec<_>>());
    let rust = tally(&vectors.iter().map(|v| v.rust.as_str()).collect::<Vec<_>>());
    let candidate = (!inputs.candidate.is_empty()).then(|| {
        tally(
            &vectors
                .iter()
                .map(|v| v.candidate.as_deref().unwrap_or("not-run"))
                .collect::<Vec<_>>(),
        )
    });
    Ok(Scored {
        score: CaseScore {
            case: case.path.clone(),
            split: case.split.clone(),
            class,
            pipeline,
            inputs,
            c_baseline,
            rust,
            candidate,
            vectors,
        },
        problems,
    })
}

/// Score `cases` (all when empty); returns the finalized scores + problems.
fn compute(suite_dir: &Path, only: &[String], recheck: bool) -> Result<(Scores, Vec<String>)> {
    let (suite, lock) = load_verified(suite_dir)?;
    let scorer = Scorer::prepare(suite_dir, &suite, &lock)?;
    let mut scores = Scores {
        schema: SCORES_SCHEMA_NAME.into(),
        schema_version: SCORES_SCHEMA_VERSION,
        suite: suite.name.clone(),
        corpus_lock: hash::file_hash(&suite_dir.join("corpus.lock"))?,
        scorer_lock: hash::file_hash(&suite_dir.join("heldout/Cargo.lock"))?,
        environment: scorer.environment().to_vec(),
        totals: Vec::new(),
        cases: Vec::new(),
    };
    let mut problems = Vec::new();
    for case in &suite.cases {
        if !only.is_empty() && !only.iter().any(|o| *o == case.path || o == case.name()) {
            continue;
        }
        let scored = score_one(&scorer, suite_dir, case, recheck)
            .with_context(|| format!("scoring {}", case.path))?;
        out(format!(
            "bench: {} [{}] {} — C {}/{} Rust {}/{}{}",
            case.path,
            case.split,
            scored.score.class,
            scored.score.c_baseline.pass,
            scored.score.vectors.len() as u32 - scored.score.c_baseline.skip,
            scored.score.rust.pass,
            scored.score.vectors.len() as u32 - scored.score.c_baseline.skip,
            scored
                .score
                .candidate
                .as_ref()
                .map(|k| format!(" (candidate {}/{})", k.pass, k.pass + k.fail))
                .unwrap_or_default()
        ));
        problems.extend(scored.problems);
        scores.cases.push(scored.score);
    }
    scores.finalize();
    Ok((scores, problems))
}

fn print_totals(scores: &Scores) {
    let pct = |n: u32, d: u32| {
        if d == 0 {
            "n/a".to_string()
        } else {
            format!("{:.1}%", f64::from(n) * 100.0 / f64::from(d))
        }
    };
    for t in &scores.totals {
        out(format!(
            "bench totals [{}]: strict-pass {}/{} scorable cases ({}) · verified {} (blind spots \
             {}) · oracle false negatives {} · non-UB vectors passed {}/{} ({}) · unscorable {} \
             · C-baseline-invalid {} · cases {}",
            t.split,
            t.strict_pass,
            t.scorable,
            pct(t.strict_pass, t.scorable),
            t.verified,
            t.blind_spots,
            t.oracle_false_negatives,
            t.vectors_passed,
            t.vectors,
            pct(t.vectors_passed, t.vectors),
            t.unscorable,
            t.c_baseline_invalid,
            t.cases
        ));
    }
}

fn cmd_score(suite_dir: &Path, cases: &[String], write: bool) -> Result<ExitCode> {
    if write && !cases.is_empty() {
        bail!("--write records the WHOLE suite; drop --case");
    }
    let (scores, _) = compute(suite_dir, cases, false)?;
    print_totals(&scores);
    if write {
        scores.store(&suite_dir.join("scores.json"))?;
        out(format!(
            "bench score: wrote {}",
            suite_dir.join("scores.json").display()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_check(suite_dir: &Path, replay: bool) -> Result<ExitCode> {
    let baseline = Scores::load(&suite_dir.join("scores.json"))
        .context("loading the committed scores.json (run `bench score --write` first)")?;
    let mut problems = Vec::new();
    if replay {
        problems.extend(replay_all(suite_dir)?);
    }
    let (now, recheck_problems) = compute(suite_dir, &[], true)?;
    problems.extend(recheck_problems);
    print_totals(&now);
    let cmp = compare(&baseline, &now);
    for (label, list) in [
        ("INCOMPARABLE", &cmp.incomparable),
        ("inputs changed (re-score required)", &cmp.input_changes),
        ("membership", &cmp.membership),
        ("environment drift (C side)", &cmp.drift),
        ("REGRESSION", &cmp.regressions),
        ("improvement", &cmp.improvements),
        ("PROBLEM", &problems),
    ] {
        for item in list.iter() {
            out(format!("bench check: {label}: {item}"));
        }
    }
    if !cmp.incomparable.is_empty() || !cmp.input_changes.is_empty() || !cmp.membership.is_empty() {
        out("bench check: INCOMPARABLE — re-baseline with `bench score --write`".into());
        return Ok(ExitCode::FAILURE);
    }
    if !cmp.regressions.is_empty() || !problems.is_empty() {
        out(format!(
            "bench check: FAILED — {} regression(s), {} problem(s)",
            cmp.regressions.len(),
            problems.len()
        ));
        return Ok(ExitCode::from(crate::EXIT_ORACLE_RED));
    }
    out(format!(
        "bench check: OK — no regression ({} improvement(s))",
        cmp.improvements.len()
    ));
    Ok(ExitCode::SUCCESS)
}

/// Replay-verify every recorded driver and migrate attempt of every case
/// from its traces (zero tokens). Returns one problem per attempt that no
/// longer reproduces.
fn replay_all(suite_dir: &Path) -> Result<Vec<String>> {
    let (suite, _lock) = load_verified(suite_dir)?;
    let mut problems = Vec::new();
    for case in &suite.cases {
        let root = case.target_root(suite_dir);
        if !root.join("harness.toml").exists() || !Ledger::new(&root).plan_path().exists() {
            continue;
        }
        let ctx = TargetContext::load(&root)?;
        let ledger = Ledger::new(&ctx.root);
        let plan = Plan::load(&ledger.plan_path())?;
        let facts = Facts::load(&ledger.facts_path())?;
        for unit in &plan.units {
            let llm = &ctx.config.llm;
            for (stage, records, traces_sub, section) in [
                (
                    "driver",
                    attempts::load_unit_driver_attempts(&ledger, &unit.id)?,
                    "driver-traces",
                    llm.driver.as_ref(),
                ),
                (
                    "migrate",
                    attempts::load_unit_attempts(&ledger, &unit.id)?,
                    "traces",
                    llm.migrate.as_ref(),
                ),
            ] {
                let traces = ledger.unit_dir(&unit.id).join(traces_sub);
                let max_tokens = section.and_then(|m| m.max_tokens).unwrap_or(llm.max_tokens);
                let max_repairs = section.and_then(|m| m.max_repairs).unwrap_or(3);
                for rec in records.iter().filter(|r| r.outcome != "in-progress") {
                    let resolved = harness_llm::providers::resolve("replay", &traces)?;
                    let params = harness_llm::MigrateParams {
                        provider: &resolved,
                        model: &rec.model,
                        max_tokens,
                        max_repairs,
                        traces_dir: &traces,
                        retry: false,
                        attempt: Some(&rec.id),
                    };
                    let result = if stage == "driver" {
                        let judge = |p: &Path| harness_oracle::validate_driver(&ctx, unit, p);
                        harness_llm::run_driver_generation(
                            &params, &judge, &ctx, &facts, &plan, unit,
                        )
                        .map(|_| ())
                    } else {
                        harness_llm::run_migration(
                            &params,
                            &harness_oracle::CAbiDifferential,
                            &ctx,
                            &facts,
                            &plan,
                            unit,
                            &[],
                        )
                        .map(|_| ())
                    };
                    match result {
                        Ok(()) => out(format!(
                            "bench replay: {} {stage} {} reproduces",
                            case.path, rec.id
                        )),
                        Err(e) => problems.push(format!(
                            "{} {stage} attempt {} does not replay: {e}",
                            case.path, rec.id
                        )),
                    }
                }
            }
        }
    }
    Ok(problems)
}
