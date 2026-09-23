//! `harness bench …` — benchmark suites (docs/SCHEMAS.md "M4 additions").
//!
//! A suite dir (e.g. `targets/tractor/`) holds `suite.toml`, `corpus.lock`,
//! one harness target per case under `cases/`, the held-out scoring material
//! under `heldout/`, and the committed `scores.json`.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use harness_core::bench::{
    classify_case, compare, is_infra_result, is_unmarked_ub, CaseInputs, CasePipeline, CaseScore,
    CorpusLock, Counts, Scores, Suite, SuiteCase, VectorScore, Verification, SCORES_SCHEMA_NAME,
    SCORES_SCHEMA_VERSION,
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
        /// Run model-written and third-party code even though no sandbox is
        /// available (the scorer loads candidates next to held-out vectors)
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Cases scored in parallel (default: half the available cores)
        #[arg(long)]
        jobs: Option<usize>,
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
        /// Run model-written and third-party code even though no sandbox is
        /// available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Cases scored in parallel (default: half the available cores)
        #[arg(long)]
        jobs: Option<usize>,
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
            allow_unsandboxed,
            jobs,
        } => {
            // The scorer builds and loads model-written candidates and the
            // corpus's third-party scorer next to held-out vectors: the same
            // sandbox floor as every other code-running command (M4 review).
            crate::require_sandbox(allow_unsandboxed, "harness bench score")?;
            cmd_score(&suite, &cases, write, default_jobs(jobs))
        }
        BenchCmd::Check {
            suite,
            replay,
            allow_unsandboxed,
            jobs,
        } => {
            crate::require_sandbox(allow_unsandboxed, "harness bench check")?;
            cmd_check(&suite, replay, default_jobs(jobs))
        }
    }
}

/// `--jobs`, defaulting to half the available cores (at least 1).
fn default_jobs(jobs: Option<usize>) -> usize {
    jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
    })
    .max(1)
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
    // The driver the ORACLE runs: the configured `[unit.oracle] driver`
    // (M4 review: the default path could differ from what verify uses).
    let driver_path = match unit.oracle_param_str("driver") {
        Some(rel) => ctx.root.join(rel),
        None => ledger.driver_path(&unit.id),
    };
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

/// Counts of one side; `(result, excused)` per vector — an `unmarked-ub`
/// vector is counted apart on every side (R-A4), whatever its result.
fn tally(results: &[(&str, bool)]) -> Counts {
    let mut c = Counts::default();
    for (r, excused) in results {
        if *excused && *r != "skip" {
            c.unmarked_ub += 1;
            continue;
        }
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
    let mut verification = Verification::No;
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
            let driver_path = match unit.oracle_param_str("driver") {
                Some(rel) => ctx.root.join(rel),
                None => ledger.driver_path(&unit.id),
            };
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
            // "Verified" for scoring = plan status AND the latest verdict is
            // green over the CURRENT digests AND the driver is freshly
            // validated (R6). Anything less is `stale-verified`, never scored
            // as verified (review, M4).
            let claims = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
            if claims {
                let fresh_green =
                    match harness_core::Verdict::load(&ledger.verdict_latest_path(&unit.id)) {
                        Ok(v) => {
                            let now = harness_oracle::compute_inputs(&ctx, unit, &facts)?;
                            v.green
                                && v.inputs.unit_source == now.unit_source
                                && v.inputs.driver == now.driver
                                && v.inputs.rust_crate == now.rust_crate
                        }
                        Err(_) => false,
                    };
                verification = if fresh_green && pipeline.driver == "validated" {
                    Verification::Verified
                } else {
                    Verification::Stale
                };
            }
            if verification == Verification::Verified {
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
            } else if let Some(cand) = m_attempts
                .iter()
                .filter(|r| {
                    // A FINISHED attempt the oracle rejected on BEHAVIOR:
                    // its last candidate built, ran, and differed from the C.
                    r.outcome == "red"
                        && r.turns.last().is_some_and(|t| t.result == "oracle")
                        && !r.candidate_digest.is_empty()
                })
                .min_by(|a, b| a.id.cmp(&b.id))
            {
                let dir = attempts::attempt_dir(&ledger, &unit.id, &cand.id).join("candidate");
                if dir.is_dir() {
                    inputs.candidate = cand.candidate_digest.clone();
                    pipeline.candidate_attempt = cand.id.clone();
                    candidate_lib = harness_oracle::build_crate_staticlib(&ctx, &dir).ok();
                }
            }
        }
    }
    let runs = scorer.score_case(case, &cside, rust_lib.as_deref(), candidate_lib.as_deref())?;
    let verified = verification == Verification::Verified;
    let rust_missing = if verified { "fail:build" } else { "not-run" };
    // §A.2 disclosure: a fallback to ASan-only (or no build at all) is
    // printed, so a flaky bounds-safety compile is never silent.
    match runs.sanitized_build.as_deref() {
        Some("asan") => out(format!(
            "bench: {}: sanitized C built WITHOUT -fbounds-safety (it does not compile with it)",
            case.path
        )),
        Some("none") => problems.push(format!(
            "{}: the sanitized C did not build (a harness problem; nothing excused)",
            case.path
        )),
        _ => {}
    }
    let vectors: Vec<VectorScore> = runs
        .vectors
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
            c_sanitized: r.c_sanitized.clone(),
        })
        .collect();
    for v in &vectors {
        match v.c_sanitized.as_deref() {
            // R-A11: the sanitized side failing to build/run is never silent.
            Some(s) if is_infra_result(s) => problems.push(format!(
                "{}/{}: sanitized C pass: {s} (a harness problem; the vector is not excused)",
                case.path, v.name
            )),
            // R-A12: every excusal is disclosed with what it excused.
            Some(s) if s.starts_with("ub:") => out(format!(
                "bench: {}/{}: unmarked-ub ({}) — excused; plain C {}, Rust {}{}",
                case.path,
                v.name,
                s.trim_start_matches("ub:"),
                v.c,
                v.rust,
                v.candidate
                    .as_deref()
                    .map(|k| format!(", candidate {k}"))
                    .unwrap_or_default()
            )),
            _ => {}
        }
    }
    let class = classify_case(&vectors, verification).to_string();
    let side = |f: &dyn Fn(&VectorScore) -> &str| -> Counts {
        tally(
            &vectors
                .iter()
                .map(|v| (f(v), is_unmarked_ub(v)))
                .collect::<Vec<_>>(),
        )
    };
    let c_baseline = side(&|v| v.c.as_str());
    let rust = side(&|v| v.rust.as_str());
    let candidate = (!inputs.candidate.is_empty())
        .then(|| side(&|v| v.candidate.as_deref().unwrap_or("not-run")));
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
            sanitized_build: runs.sanitized_build.clone(),
            vectors,
        },
        problems,
    })
}

/// Score `cases` (all when empty); returns the finalized scores + problems.
fn compute(
    suite_dir: &Path,
    only: &[String],
    recheck: bool,
    jobs: usize,
) -> Result<(Scores, Vec<String>)> {
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
    let selected: Vec<&SuiteCase> = suite
        .cases
        .iter()
        .filter(|case| only.is_empty() || only.iter().any(|o| *o == case.path || o == case.name()))
        .collect();
    // Cases are independent targets (own ledgers, own crates, per-run temp
    // dirs), so they are scored on a few worker threads; results are
    // collected by index and the record is sorted canonically afterwards.
    let jobs = jobs.clamp(1, selected.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<Scored>>>> = selected
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(case) = selected.get(i) else { break };
                let result = score_one(&scorer, suite_dir, case, recheck)
                    .with_context(|| format!("scoring {}", case.path));
                if let Ok(scored) = &result {
                    out(format!(
                        "bench: {} [{}] {} — C {}/{} Rust {}/{}{}",
                        case.path,
                        case.split,
                        scored.score.class,
                        scored.score.c_baseline.pass,
                        scored.score.vectors.len() as u32
                            - scored.score.c_baseline.skip
                            - scored.score.c_baseline.unmarked_ub,
                        scored.score.rust.pass,
                        scored.score.vectors.len() as u32
                            - scored.score.c_baseline.skip
                            - scored.score.c_baseline.unmarked_ub,
                        scored
                            .score
                            .candidate
                            .as_ref()
                            .map(|k| format!(" (candidate {}/{})", k.pass, k.pass + k.fail))
                            .unwrap_or_default()
                    ));
                }
                if let Ok(mut slot) = slots[i].lock() {
                    *slot = Some(result);
                }
            });
        }
    });
    for (case, slot) in selected.iter().zip(slots) {
        let scored = slot
            .into_inner()
            .ok()
            .flatten()
            .unwrap_or_else(|| Err(anyhow::anyhow!("scoring {}: worker lost", case.path)))?;
        problems.extend(scored.problems);
        if scored.score.class == "infra-error" {
            problems.push(format!(
                "{}: infra-error (a side failed to build/link/run or report — a harness problem, \
                 not a measurement)",
                case.path
            ));
        }
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
             {}) · stale-verified {} · vector-pass/oracle-red {} · non-UB vectors passed {}/{} \
             ({}) · unmarked-UB excused {} · unscorable {} · C-baseline-invalid {} · infra-error \
             {} · cases {}",
            t.split,
            t.strict_pass,
            t.scorable,
            pct(t.strict_pass, t.scorable),
            t.verified,
            t.blind_spots,
            t.stale_verified,
            t.vector_pass_oracle_red,
            t.vectors_passed,
            t.vectors,
            pct(t.vectors_passed, t.vectors),
            t.vectors_unmarked_ub,
            t.unscorable,
            t.c_baseline_invalid,
            t.infra_errors,
            t.cases
        ));
    }
}

fn cmd_score(suite_dir: &Path, cases: &[String], write: bool, jobs: usize) -> Result<ExitCode> {
    if write && !cases.is_empty() {
        bail!("--write records the WHOLE suite; drop --case");
    }
    // Recording a baseline re-verifies every verified unit and re-validates
    // every generated driver first (review, M4): a red unit must never be
    // written into the regression baseline as verified.
    let (scores, problems) = compute(suite_dir, cases, write, jobs)?;
    print_totals(&scores);
    for p in &problems {
        out(format!("bench score: PROBLEM: {p}"));
    }
    if !problems.is_empty() {
        if write {
            bail!(
                "refusing to record scores.json with {} problem(s)",
                problems.len()
            );
        }
        return Ok(ExitCode::FAILURE);
    }
    if write {
        scores.store(&suite_dir.join("scores.json"))?;
        out(format!(
            "bench score: wrote {}",
            suite_dir.join("scores.json").display()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_check(suite_dir: &Path, replay: bool, jobs: usize) -> Result<ExitCode> {
    let baseline = Scores::load(&suite_dir.join("scores.json"))
        .context("loading the committed scores.json (run `bench score --write` first)")?;
    let mut problems = Vec::new();
    if replay {
        problems.extend(replay_all(suite_dir)?);
    }
    let (now, recheck_problems) = compute(suite_dir, &[], true, jobs)?;
    problems.extend(recheck_problems);
    print_totals(&now);
    let cmp = compare(&baseline, &now);
    for (label, list) in [
        ("INCOMPARABLE", &cmp.incomparable),
        ("inputs changed (re-score required)", &cmp.input_changes),
        ("membership", &cmp.membership),
        ("environment drift (C side)", &cmp.drift),
        ("totals changed", &cmp.totals_changes),
        ("REGRESSION", &cmp.regressions),
        ("improvement", &cmp.improvements),
        ("PROBLEM", &problems),
    ] {
        for item in list.iter() {
            out(format!("bench check: {label}: {item}"));
        }
    }
    // An environment/lock mismatch makes vector comparisons meaningless.
    if !cmp.incomparable.is_empty() {
        out("bench check: INCOMPARABLE — re-baseline with `bench score --write`".into());
        return Ok(ExitCode::FAILURE);
    }
    // Regressions on unchanged cases (and lost verified status anywhere) are
    // judged per case: other cases' changed inputs never mask them.
    if !cmp.regressions.is_empty() || !problems.is_empty() {
        out(format!(
            "bench check: FAILED — {} regression(s), {} problem(s)",
            cmp.regressions.len(),
            problems.len()
        ));
        return Ok(ExitCode::from(crate::EXIT_ORACLE_RED));
    }
    if !cmp.input_changes.is_empty() || !cmp.membership.is_empty() {
        out(
            "bench check: no regression on unchanged cases; re-score required for changed \
             ones (`bench score --write`)"
                .into(),
        );
        return Ok(ExitCode::FAILURE);
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
/// `(base id, sample number)` of an attempt id: `<base>` is sample 1,
/// `<base>.r<N>` sample N.
fn sample_of(id: &str) -> (&str, u32) {
    id.rsplit_once(".r")
        .and_then(|(base, n)| {
            n.parse::<u32>()
                .ok()
                // Canonical numbers only (`.r02` is not sample 2).
                .filter(|v| *v >= 2 && v.to_string() == n)
                .map(|v| (base, v))
        })
        .unwrap_or((id, 1))
}

/// For a hand-off (`external`) record: the id of a later sample of the same
/// base, if one exists. A trace-backed trajectory is a function of the
/// response files and the judge, so a later sample exists only because this
/// one stopped reproducing (`--retry` records one in no other case) — e.g.
/// the oracle began comparing stderr. Only the latest sample must replay;
/// live samples are independent and all must.
fn superseding_sample(
    rec: &attempts::AttemptRecord,
    records: &[attempts::AttemptRecord],
) -> Option<String> {
    if rec.provider_kind != "external" {
        return None;
    }
    let (base, number) = sample_of(&rec.id);
    records
        .iter()
        .filter(|other| other.provider_kind == "external")
        .map(|other| (sample_of(&other.id), &other.id))
        .filter(|((other_base, n), _)| *other_base == base && *n > number)
        .max_by_key(|((_, n), _)| *n)
        .map(|(_, id)| id.clone())
}

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
                // Only attempts bound to the CURRENT inputs can reproduce: a
                // record for superseded source or a replaced driver is stale
                // evidence, reported, never a replay failure (M4 review).
                let closure = facts.include_closure(&unit.files);
                let unit_source = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
                let driver_now = match unit.oracle_param_str("driver") {
                    Some(rel) if ctx.root.join(rel).exists() => {
                        hash::file_hash(&ctx.root.join(rel))?
                    }
                    _ => String::new(),
                };
                let hazards = crate::confirmed_hazards(&ledger, &plan, &facts, &unit.id)?;
                for rec in records.iter().filter(|r| r.outcome != "in-progress") {
                    let stale = rec.unit_source != unit_source
                        || (stage == "migrate" && rec.driver != driver_now);
                    if stale {
                        out(format!(
                            "bench replay: {} {stage} {} skipped (bound to superseded inputs)",
                            case.path, rec.id
                        ));
                        continue;
                    }
                    if let Some(newer) = superseding_sample(rec, &records) {
                        out(format!(
                            "bench replay: {} {stage} {} skipped (superseded by sample {newer})",
                            case.path, rec.id
                        ));
                        continue;
                    }
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
                            &hazards,
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

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::attempts::{AttemptRecord, ATTEMPT_SCHEMA_NAME};

    fn rec(id: &str, kind: &str) -> AttemptRecord {
        AttemptRecord {
            schema: ATTEMPT_SCHEMA_NAME.into(),
            schema_version: 1,
            id: id.into(),
            unit: "u".into(),
            stage: None,
            provider: kind.into(),
            provider_kind: kind.into(),
            model: "m".into(),
            prompt_digest: String::new(),
            unit_source: String::new(),
            driver: String::new(),
            toolchain: vec![],
            outcome: "green".into(),
            turns: vec![],
            candidate_digest: String::new(),
            promoted: false,
        }
    }

    /// R-A4: an excused vector counts as `unmarked_ub` on every side,
    /// whatever its result, so totals agree with the class.
    #[test]
    fn excused_vectors_are_tallied_apart() {
        let c = tally(&[
            ("pass", false),
            ("pass", true),
            ("fail:Panic", true),
            ("skip", false),
            ("fail:Timeout", false),
        ]);
        assert_eq!((c.pass, c.fail, c.skip, c.unmarked_ub), (1, 1, 1, 2));
    }

    #[test]
    fn sample_numbers() {
        assert_eq!(sample_of("a-0123456789ab"), ("a-0123456789ab", 1));
        assert_eq!(sample_of("a-0123456789ab.r2"), ("a-0123456789ab", 2));
        assert_eq!(sample_of("a-0123456789ab.r13"), ("a-0123456789ab", 13));
        assert_eq!(sample_of("a-0123456789ab.r02"), ("a-0123456789ab.r02", 1));
        assert_eq!(sample_of("a-0123456789ab.r1"), ("a-0123456789ab.r1", 1));
    }

    /// Only the latest hand-off sample of a base must replay (a later one
    /// exists only because the earlier one stopped reproducing); live
    /// samples are independent, and other bases never supersede.
    #[test]
    fn only_later_external_samples_supersede() {
        let records = vec![
            rec("a-aaaaaaaaaaaa", "external"),
            rec("a-aaaaaaaaaaaa.r2", "external"),
            rec("a-aaaaaaaaaaaa.r3", "external"),
            rec("a-bbbbbbbbbbbb", "external"),
            rec("a-cccccccccccc", "openai-compat"),
            rec("a-cccccccccccc.r2", "openai-compat"),
        ];
        let sup = |i: usize| superseding_sample(&records[i], &records);
        assert_eq!(sup(0).as_deref(), Some("a-aaaaaaaaaaaa.r3"));
        assert_eq!(sup(1).as_deref(), Some("a-aaaaaaaaaaaa.r3"));
        assert_eq!(sup(2), None);
        assert_eq!(sup(3), None);
        assert_eq!(sup(4), None, "live samples all replay");
        assert_eq!(sup(5), None);
    }
}
