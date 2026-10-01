//! perf's measurement (docs/PERF-DESIGN.md §3.2, §3.5–§3.7, §3.9): which
//! units can be measured, the sides built as verify builds them, each row
//! run — the C alone, then each side against the C — every run judged in
//! the design's order, and every row written by the one replace rule.
//! Information only: no verdict, no plan status.

use super::archive::{archive_facts, ArchiveFacts, PanicRuntime};
use super::build::{self, Hashed, Slot};
use super::launcher::{self, End, Launcher, Seen, Status};
use crate::exec::{self, Runner};
use crate::sandbox::{self, HostDirs, PerfSpec, ProfileSpec};
use crate::{build_staticlib, extra_link_args, prepare_target_dir, program_c_files_in, Base};
use harness_core::error::Error;
use harness_core::ledger::Ledger;
use harness_core::perf::results::{
    self as res, CrateDigest, Difference, FailedRun, KeptFile, LeftOut, ProfileSetting, Row,
    RowInputs, RowKind, Run, SetupFacts, Step1, Step1Run, UnitRef, UnitRuntime,
};
use harness_core::perf::words::{self as perf_words, Platform};
use harness_core::perf::workloads::{self as wl, Workload, Workloads};
use harness_core::{Facts, Plan, TargetContext, Unit, UnitStatus};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Bytes kept per stream.
pub(crate) const OUTPUT_CAP: usize = 64 * 1024 * 1024;
/// The bound before a run's go-ahead.
const ALLOWANCE: Duration = Duration::from_secs(60);
/// Extra deadline for step-1 runs (a new binary's first exec falls there).
const STEP1_EXTRA_SECS: u64 = 60;

/// What a perf run tells its caller as it goes (§3.10).
pub trait PerfProgress {
    /// A progress line in words.
    fn message(&mut self, text: &str);
    /// A row written: its side, the workload, the row.
    fn row(&mut self, side: RowSide<'_>, row: &Row);
}

/// Which side a written row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSide<'a> {
    /// The C alone.
    C,
    /// The program as it stands.
    Program,
    /// A unit's program.
    Unit(&'a str),
}

/// What to measure (§3.10).
#[derive(Debug, Clone, Default)]
pub struct PerfRequest {
    /// Only these units (empty: the C alone, the program as it stands and
    /// every measurable unit).
    pub units: Vec<String>,
    /// Only these workloads (empty: all).
    pub workloads: Vec<String>,
    /// Runs a side, over each workload's own.
    pub runs: Option<u32>,
    /// Only the program as it stands.
    pub as_it_stands_only: bool,
}

/// What a perf run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerfSummary {
    /// Rows written.
    pub rows: usize,
    /// Rows measured (baseline or measured).
    pub measured: usize,
    /// Rows too short to time.
    pub too_short: usize,
    /// Rows that behave differently.
    pub behaves_differently: usize,
}

// ---- Selection (§3.2) --------------------------------------------------

/// A unit perf can build and measure.
#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    /// 1-based plan position.
    position: usize,
    replaces_rel: Vec<String>,
    replaces: Vec<PathBuf>,
    crate_dir: PathBuf,
    /// The verdict's `inputs.rust_crate`.
    verdict_crate: String,
}

/// A verified unit's state today.
#[derive(Debug, Clone)]
enum Selected {
    Ready(Candidate),
    /// Not measurable: a `not-verified` reason, and the attempt of an
    /// interrupted Accept.
    NotVerified {
        id: String,
        reason: &'static str,
        attempt: Option<String>,
    },
}

impl Selected {
    fn id(&self) -> &str {
        match self {
            Selected::Ready(c) => &c.id,
            Selected::NotVerified { id, .. } => id,
        }
    }
}

/// The plan's verified or merged units, each measurable or why not; pending
/// and blocked units are never named (§3.2). Called under perf's writer
/// lock: an interrupted Accept is read directly (`unit_report` would see
/// perf's own lock as a writer at work).
fn select(
    target: &TargetContext,
    ledger: &Ledger,
    facts: &Facts,
    plan: &Plan,
) -> Result<Vec<Selected>, Error> {
    let mut out = Vec::new();
    for (i, unit) in plan.units.iter().enumerate() {
        if !matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged) {
            continue;
        }
        if let Some(attempt) = harness_core::status::promotion_marker(ledger, unit)? {
            out.push(Selected::NotVerified {
                id: unit.id.clone(),
                reason: "accept-interrupted",
                attempt: Some(attempt),
            });
            continue;
        }
        let report = harness_core::status::unit_report(target, ledger, facts, unit, None)?;
        if !report.fresh_green() {
            out.push(Selected::NotVerified {
                id: unit.id.clone(),
                reason: "not-fresh",
                attempt: None,
            });
            continue;
        }
        let verdict = harness_core::Verdict::load(&ledger.verdict_latest_path(&unit.id))?;
        let replaces_rel = unit.oracle_param_list("replaces");
        if verdict.inputs.replaces != replaces_rel {
            out.push(Selected::NotVerified {
                id: unit.id.clone(),
                reason: "replaces-changed",
                attempt: None,
            });
            continue;
        }
        let Some(candidate) = candidate(
            target,
            ledger,
            unit,
            i + 1,
            replaces_rel,
            verdict.inputs.rust_crate,
        )?
        else {
            out.push(Selected::NotVerified {
                id: unit.id.clone(),
                reason: "not-fresh",
                attempt: None,
            });
            continue;
        };
        out.push(Selected::Ready(candidate));
    }
    Ok(out)
}

fn candidate(
    target: &TargetContext,
    ledger: &Ledger,
    unit: &Unit,
    position: usize,
    replaces_rel: Vec<String>,
    verdict_crate: String,
) -> Result<Option<Candidate>, Error> {
    let Some(rust_crate) = unit.oracle_param_str("rust_crate") else {
        return Ok(None);
    };
    let unit_dir = ledger.unit_dir(&unit.id);
    let raw = unit_dir.join(rust_crate);
    if !raw.is_dir() {
        return Ok(None);
    }
    let unit_dir = unit_dir
        .canonicalize()
        .map_err(|e| Error::io(&unit_dir, e))?;
    let crate_dir = crate::inside(&unit.id, "rust_crate", &raw, &unit_dir)?;
    let root = &target.root;
    let mut replaces = Vec::new();
    for rel in &replaces_rel {
        let p = root.join(rel);
        match p.canonicalize() {
            Ok(c) => replaces.push(c),
            Err(_) => replaces.push(p),
        }
    }
    Ok(Some(Candidate {
        id: unit.id.clone(),
        position,
        replaces_rel,
        replaces,
        crate_dir,
        verdict_crate,
    }))
}

// ---- A side, built -----------------------------------------------------

/// A unit's built program, or why it could not be built.
enum UnitSide {
    Built {
        candidate: Candidate,
        bin: Hashed,
        staticlib: PathBuf,
        facts: ArchiveFacts,
        crate_digest: String,
        profile: Vec<ProfileSetting>,
    },
    /// A set-up outcome with its facts.
    SetUp {
        id: String,
        outcome: &'static str,
        setup: SetupFacts,
        crate_digest: String,
    },
}

impl UnitSide {
    fn id(&self) -> &str {
        match self {
            UnitSide::Built { candidate, .. } => &candidate.id,
            UnitSide::SetUp { id, .. } => id,
        }
    }
}

/// The program as it stands: built, a set-up outcome, or not built (fewer
/// than two measurable units).
enum Program {
    Built {
        bin: Hashed,
        units: Vec<UnitRef>,
    },
    SetUp {
        outcome: &'static str,
        setup: SetupFacts,
        units: Vec<UnitRef>,
    },
    None,
}

/// A crate manifest's `[profile.release]` settings away from Cargo's
/// defaults (build note 25).
fn profile_settings(crate_dir: &Path) -> Vec<ProfileSetting> {
    let Ok(text) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) else {
        return Vec::new();
    };
    harness_core::perf::manifest_profile(&text)
        .into_iter()
        .map(|(key, value)| ProfileSetting { key, value })
        .collect()
}

// ---- One run, judged (§3.3 *Judging a run*) ----------------------------

/// A run as the design judges it.
#[derive(Debug, Clone)]
enum Judged {
    /// The program's own end, its (rewritten) streams and its counters.
    Ended {
        end: End,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        run: Box<Run>,
    },
    TimedOut,
    Overflow,
    /// The launcher's own words for why.
    Unmeasurable(String),
    NeverStarted(Option<i32>),
    /// A SIGKILL perfrun did not send.
    SigKilled,
}

impl Judged {
    fn end_token(&self) -> String {
        match self {
            Judged::Ended { end, .. } => end.token(),
            Judged::TimedOut => "timeout".into(),
            Judged::SigKilled => "signal 9".into(),
            Judged::NeverStarted(_) => "never-started".into(),
            _ => "timeout".into(),
        }
    }
}

/// Everything one run needs.
struct RunCtx<'a> {
    launcher: &'a Launcher,
    host: &'a HostDirs,
    root: &'a Path,
    name: &'a str,
    timeout_secs: u64,
}

/// `$` → `$$`, the run's temp dir → `$TMPDIR`, the program's folder →
/// `$PROGDIR`: two sides' streams compare whatever their paths (§1).
fn rewrite(raw: &[u8], tmp: &Path, bin: &Path) -> Vec<u8> {
    let escaped = replace_bytes(raw, b"$", b"$$");
    let tmp_done = replace_bytes(&escaped, tmp.as_os_str().as_encoded_bytes(), b"$TMPDIR");
    match bin.parent() {
        Some(dir) => replace_bytes(&tmp_done, dir.as_os_str().as_encoded_bytes(), b"$PROGDIR"),
        None => tmp_done,
    }
}

fn replace_bytes(hay: &[u8], needle: &[u8], with: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return hay.to_vec();
    }
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(needle) {
            out.extend_from_slice(with);
            i += needle.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// A fresh 0700 run folder, registered for the signal's cleanup.
struct RunDir(PathBuf);

impl RunDir {
    fn create() -> Result<RunDir, Error> {
        let base_raw = std::env::temp_dir();
        let base = base_raw
            .canonicalize()
            .map_err(|e| Error::io(&base_raw, e))?;
        crate::featuremap::make_live_dir(|| {
            for _ in 0..1000 {
                let dir = base.join(format!(
                    "ruharness-perf-{:010}-{}",
                    std::process::id(),
                    harness_core::hash::random_hex(5)
                ));
                let mut builder = std::fs::DirBuilder::new();
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match builder.create(&dir) {
                    Ok(()) => return Ok(dir),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(Error::io(&dir, e)),
                }
            }
            Err(Error::Invariant(
                "could not create a fresh perf run folder".into(),
            ))
        })
        .map(RunDir)
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        crate::featuremap::drop_live_dir(&self.0);
    }
}

/// Run `bin` once on `workload` and judge it (§3.3): the harness's own end
/// first, then a launcher that failed, the record's timeout, a program that
/// never started, a SIGKILL perfrun did not send, else the program's own end.
fn run_once(
    ctx: &RunCtx<'_>,
    bin: &Hashed,
    workload: &Workload,
    input: Option<&[u8]>,
    step1: bool,
) -> Result<Judged, Error> {
    bin.check()?;
    let dir = RunDir::create()?;
    let tmp = &dir.0;
    let cwd = tmp.join(crate::confine::SCENARIO_CWD);
    std::fs::create_dir(&cwd).map_err(|e| Error::io(&cwd, e))?;
    let input_name = workload
        .input
        .as_deref()
        .and_then(|i| i.rsplit('/').next())
        .unwrap_or("input");
    if let Some(bytes) = input {
        let path = cwd.join(input_name);
        std::fs::write(&path, bytes).map_err(|e| Error::io(&path, e))?;
        let f = std::fs::File::options()
            .write(true)
            .open(&path)
            .map_err(|e| Error::io(&path, e))?;
        f.set_modified(std::time::UNIX_EPOCH)
            .map_err(|e| Error::io(&path, e))?;
    }
    let args: Vec<String> = workload
        .args
        .iter()
        .map(|a| {
            if a == wl::INPUT_ARG {
                input_name.to_string()
            } else {
                a.clone()
            }
        })
        .collect();
    let profile = sandbox::render_perf_profile(&PerfSpec {
        host: ctx.host,
        target_root: ctx.root,
        bin: &bin.path,
        perfgo: &ctx.launcher.perfgo.path,
        tmpdir: tmp,
    })?;
    let deadline = ctx.timeout_secs + if step1 { STEP1_EXTRA_SECS } else { 0 };
    let m = launcher::run_measured(&launcher::RunSpec {
        launcher: ctx.launcher,
        profile: &profile,
        deadline_secs: deadline,
        allowance: ALLOWANCE,
        program: &bin.path,
        name: ctx.name,
        args: &args,
        cwd: &cwd,
        tmpdir: tmp,
        capture: step1,
        max_output: OUTPUT_CAP,
    })?;
    Ok(match m.seen {
        Seen::TimedOut => Judged::TimedOut,
        Seen::Overflow => Judged::Overflow,
        Seen::NoRecord(why) => Judged::Unmeasurable(why),
        Seen::Record(_) if m.launcher_exit != Some(0) => {
            Judged::Unmeasurable("perfrun did not exit cleanly".into())
        }
        Seen::Record(r) => match (&r.status, r.end) {
            (Status::Launcher(words), _) => Judged::Unmeasurable(words.clone()),
            (Status::Timeout, _) => Judged::TimedOut,
            (Status::Stopped, _) => return Err(Error::Interrupted),
            (Status::NeverStarted(errno), _) => Judged::NeverStarted(*errno),
            (Status::Ok, Some(End::Signal(9))) if !r.killed => Judged::SigKilled,
            (Status::Ok, Some(end)) => Judged::Ended {
                end,
                stdout: rewrite(&m.stdout, tmp, &bin.path),
                stderr: rewrite(&m.stderr, tmp, &bin.path),
                run: Box::new(Run {
                    instructions: r.instructions.filter(|v| *v > 0),
                    cycles: r.cycles.filter(|v| *v > 0),
                    cpu_us: r.cpu_us.filter(|v| *v > 0),
                    wall_us: r.wall_us.filter(|v| *v > 0),
                    memory: r.memory.filter(|v| *v > 0),
                    p_instructions: r.p_instructions,
                    p_cycles: r.p_cycles,
                    switches_voluntary: None,
                    switches_involuntary: None,
                    load: r.load,
                    end: end.token(),
                }),
            },
            (Status::Ok, None) => Judged::Unmeasurable("the record has no end".into()),
        },
    })
}

/// A step-1 run's facts.
fn step1_run(j: &Judged) -> Step1Run {
    match j {
        Judged::Ended {
            stdout,
            stderr,
            run,
            end,
        } => Step1Run {
            instructions: run.instructions,
            cpu_us: run.cpu_us,
            end: end.token(),
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: stderr.len() as u64,
        },
        other => Step1Run {
            instructions: None,
            cpu_us: None,
            end: other.end_token(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        },
    }
}

/// The C's own outcome from a step-1 run that did not end as a program
/// should, or `None` when it ended (§3.5 step 1).
/// The launcher's words for an unmeasurable run, in the progress line.
fn say_unmeasurable(j: &Judged, label: &str, w: &Workload, progress: &mut dyn PerfProgress) {
    if let Judged::Unmeasurable(why) = j {
        progress.message(&format!(
            "{label} on {}: the launcher stopped before measuring ({})",
            w.id,
            harness_core::text::safe_line(why)
        ));
    }
}

fn c_side(j: &Judged) -> Option<&'static str> {
    match j {
        Judged::Ended {
            end: End::Signal(_),
            ..
        }
        | Judged::SigKilled => Some("c-crashed"),
        Judged::Ended { .. } => None,
        Judged::TimedOut => Some("c-timed-out"),
        Judged::Overflow => Some("output-too-large"),
        Judged::NeverStarted(_) => Some("c-could-not-start"),
        Judged::Unmeasurable(_) => Some("run-failed: unmeasurable"),
    }
}

/// Where two ended runs first differ: their end, then stdout, then stderr.
fn difference(c: &Judged, other: &Judged) -> Option<Difference> {
    let (
        Judged::Ended {
            end: ce,
            stdout: co,
            stderr: cr,
            ..
        },
        Judged::Ended {
            end: oe,
            stdout: oo,
            stderr: or,
            ..
        },
    ) = (c, other)
    else {
        return None;
    };
    let first = |a: &[u8], b: &[u8]| {
        a.iter()
            .zip(b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len())) as u64
    };
    let (stream, c_len, other_len, offset) = if ce != oe {
        ("exit", 0, 0, 0)
    } else if co != oo {
        ("stdout", co.len() as u64, oo.len() as u64, first(co, oo))
    } else if cr != or {
        ("stderr", cr.len() as u64, or.len() as u64, first(cr, or))
    } else {
        return None;
    };
    Some(Difference {
        stream: stream.into(),
        c_len,
        other_len,
        offset,
        c_end: ce.token(),
        other_end: oe.token(),
        over_cap: false,
        kept: Vec::new(),
    })
}

/// Whether a step-1 run is under both legs of the floor (§3.5 step 2):
/// fewer than 1e9 instructions (when counted) and under half a second of
/// CPU.
fn under_both(r: &Step1Run) -> bool {
    let ins = r
        .instructions
        .is_none_or(|i| (i as f64) < perf_words::FLOOR_INSTRUCTIONS);
    let cpu = r
        .cpu_us
        .is_some_and(|c| (c as f64) < perf_words::FLOOR_CPU_US);
    ins && cpu
}

/// Whether a step-1 run is under either leg.
fn under_either(r: &Step1Run) -> bool {
    let ins = r
        .instructions
        .is_some_and(|i| (i as f64) < perf_words::FLOOR_INSTRUCTIONS);
    let cpu = r
        .cpu_us
        .is_some_and(|c| (c as f64) < perf_words::FLOOR_CPU_US);
    ins || cpu
}

/// The C's step-1 run with fewer instructions (§3.5 step 2).
fn fewer<'a>(a: &'a Step1Run, b: &'a Step1Run) -> &'a Step1Run {
    if b.instructions.unwrap_or(u64::MAX) < a.instructions.unwrap_or(u64::MAX) {
        b
    } else {
        a
    }
}

// ---- The run as a whole ------------------------------------------------

/// The facts every row's inputs share.
struct Shared {
    program_digest: String,
    program_name: String,
    computer: res::Computer,
    cc: String,
    rustc: String,
    platform: Platform,
}

/// Measure (§3.10 `harness perf run`): the caller holds perf's writer lock,
/// loaded the plan and the (fresh) facts, validated the workloads, and
/// resolved `perf_dir` (`migration/perf/`, links refused).
pub fn perf_run(
    target: &TargetContext,
    plan: &Plan,
    facts: &Facts,
    workloads: &Workloads,
    perf_dir: &Path,
    req: &PerfRequest,
    progress: &mut dyn PerfProgress,
) -> Result<PerfSummary, Error> {
    if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
        return Err(Error::Invariant(
            "perf runs on macOS only for now — the Linux launcher is not built yet".into(),
        ));
    }
    if harness_core::features::program_digest_now(target, facts)
        == harness_core::features::STALE_PROGRAM
    {
        return Err(Error::Invariant(
            "the facts are out of date — run harness scan first, then measure".into(),
        ));
    }
    let ledger = Ledger::new(target.root.clone());
    let host = HostDirs::from_env()?;
    let launcher = launcher::launcher(&host, &mut |w: &str| progress.message(w))?;
    let link_args = extra_link_args(target)?;
    let base = Base::resolve(target, "perf", &["cc", "cargo", "rustc"])?;
    let root = base.root.clone();
    let selected = select(target, &ledger, facts, plan)?;
    // Unknown ids: refused naming the known ones.
    for id in &req.units {
        if !selected.iter().any(|s| s.id() == id) {
            let known: Vec<&str> = selected.iter().map(Selected::id).collect();
            return Err(Error::InvalidPlan(format!(
                "perf: no verified unit {id:?} — the verified units are {}",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            )));
        }
    }
    for id in &req.workloads {
        if workloads.get(id).is_none() {
            let known: Vec<&str> = workloads.workloads.iter().map(|w| w.id.as_str()).collect();
            return Err(Error::InvalidPlan(format!(
                "perf: no workload {id:?} — the workloads are {}",
                known.join(", ")
            )));
        }
    }
    let scratch = build::perf_scratch(&root)?;
    let obj_dir = build::sub_folder(&scratch, "obj")?;
    let logs = build::perf_logs(&root)?;
    let log_name = format!("perf-{}.log", harness_core::hash::random_hex(6));
    let log_path = logs.join(&log_name);
    let mut log = String::new();

    // The tool profile: the scratch folder, each crate's target/ and lock.
    let wanted: Vec<&Candidate> = selected
        .iter()
        .filter_map(|s| match s {
            Selected::Ready(c) => Some(c),
            _ => None,
        })
        .collect();
    let mut target_dirs = Vec::new();
    for c in &wanted {
        target_dirs.push(prepare_target_dir(&c.crate_dir)?);
    }
    let mut write_dirs = vec![scratch.clone()];
    write_dirs.extend(target_dirs.iter().cloned());
    let write_files: Vec<PathBuf> = wanted
        .iter()
        .map(|c| c.crate_dir.join("Cargo.lock"))
        .collect();
    let tool_profile = sandbox::render_profile(&ProfileSpec {
        host: &host,
        target_root: &root,
        toolchain: true,
        write_dirs: &write_dirs,
        write_files: &write_files,
    })?;
    let runner = Runner {
        cwd: root.clone(),
        allowlist: base.allowlist.clone(),
        timeout: base.timeout,
        max_output: exec::DEFAULT_MAX_OUTPUT,
        tool_profile: Some(tool_profile),
        tool_tmpdir: None,
    };
    let first_line = |argv: &[&str]| -> String {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        runner
            .tool(&argv)
            .map(|o| {
                String::from_utf8_lossy(&o)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(160)
                    .collect()
            })
            .unwrap_or_default()
    };
    let cc = first_line(&["cc", "--version"]);
    let rustc = first_line(&["rustc", "-V"]);

    // The C: objects once, one link.
    progress.message("building the C program…");
    let c_files = program_c_files_in(&base, "perf")?;
    if let Some(odd) = crate::irregular_c_file(&c_files) {
        return Err(Error::Invariant(format!(
            "{} is not a regular file — perf builds the program from the top-level .c files",
            odd.display()
        )));
    }
    let objects = match build::compile_objects(&base, &runner, &c_files, &obj_dir)? {
        Ok(o) => o,
        Err(words) => {
            return Err(Error::Invariant(format!(
                "the C does not build: {}",
                first_lines(&words, 3)
            )))
        }
    };
    let slot = |s: Slot| -> Result<PathBuf, Error> {
        build::sub_folder(&scratch, &format!("bin/{}", s.name()?))
    };
    let name = harness_core::features::program_name(&target.config);
    let object_paths: Vec<PathBuf> = objects.iter().map(|h| h.path.clone()).collect();
    let c_bin = match build::link_side(
        &base,
        &link_args,
        &runner,
        &slot(Slot::C)?.join(&name),
        &object_paths,
        &[],
        false,
    )? {
        Ok(b) => b,
        Err(words) => {
            let hint = if words.contains("_main") || words.contains("main") {
                " — the program needs one main()"
            } else {
                ""
            };
            return Err(Error::Invariant(format!(
                "the C does not link: {}{hint}",
                first_lines(&words, 3)
            )));
        }
    };
    let kept_objects = |kept: &[PathBuf]| -> Vec<PathBuf> {
        c_files
            .iter()
            .zip(&objects)
            .filter(|(c, _)| kept.contains(c))
            .map(|(_, o)| o.path.clone())
            .collect()
    };

    // The units: each crate built, checked against its verdict, linked.
    let measure_units = !req.as_it_stands_only;
    let mut sides: Vec<UnitSide> = Vec::new();
    for s in &selected {
        let Selected::Ready(c) = s else { continue };
        progress.message(&format!("{} — building its Rust…", c.id));
        let target_dir = prepare_target_dir(&c.crate_dir)?;
        let staticlib = match build_staticlib(
            &runner,
            runner.tool_profile.as_deref(),
            &c.crate_dir,
            &target_dir,
        ) {
            Ok(lib) => lib,
            Err(Error::Interrupted) => return Err(Error::Interrupted),
            Err(e) => {
                log.push_str(&format!("{} — the crate does not build:\n{e}\n\n", c.id));
                sides.push(UnitSide::SetUp {
                    id: c.id.clone(),
                    outcome: "crate-does-not-build",
                    setup: SetupFacts {
                        log: Some(log_name.clone()),
                        ..SetupFacts::default()
                    },
                    crate_digest: String::new(),
                });
                continue;
            }
        };
        let crate_digest = harness_core::hash::unit_crate_file_set_hash(&root, &c.crate_dir)?;
        if crate_digest != c.verdict_crate {
            sides.push(UnitSide::SetUp {
                id: c.id.clone(),
                outcome: "not-verified",
                setup: SetupFacts {
                    reason: Some("rust-changed".into()),
                    ..SetupFacts::default()
                },
                crate_digest,
            });
            continue;
        }
        let kept = match build::kept_c_files(&c_files, &c.replaces) {
            Ok(k) => k,
            Err(i) => {
                sides.push(UnitSide::SetUp {
                    id: c.id.clone(),
                    outcome: "replaces-mismatch",
                    setup: SetupFacts {
                        index: Some(i as u32),
                        ..SetupFacts::default()
                    },
                    crate_digest,
                });
                continue;
            }
        };
        let bytes = std::fs::read(&staticlib).map_err(|e| Error::io(&staticlib, e))?;
        let facts_of = archive_facts(&bytes).unwrap_or(ArchiveFacts {
            runtime: PanicRuntime::None,
            std: true,
            fat_lto: false,
        });
        let out = slot(Slot::Unit(c.position))?.join(&name);
        match build::link_side(
            &base,
            &link_args,
            &runner,
            &out,
            &kept_objects(&kept),
            std::slice::from_ref(&staticlib),
            false,
        )? {
            Ok(bin) => sides.push(UnitSide::Built {
                candidate: c.clone(),
                bin,
                staticlib,
                facts: facts_of,
                crate_digest,
                profile: profile_settings(&c.crate_dir),
            }),
            Err(words) => {
                log.push_str(&format!(
                    "{} — the program does not link:\n{words}\n\n",
                    c.id
                ));
                sides.push(UnitSide::SetUp {
                    id: c.id.clone(),
                    outcome: "does-not-link",
                    setup: SetupFacts {
                        cause: Some("unknown".into()),
                        units: Some(vec![c.id.clone()]),
                        log: Some(log_name.clone()),
                        ..SetupFacts::default()
                    },
                    crate_digest,
                });
            }
        }
    }

    // The program as it stands (§3.2).
    let built: Vec<&UnitSide> = sides
        .iter()
        .filter(|s| matches!(s, UnitSide::Built { .. }))
        .collect();
    let held: Vec<UnitRef> = built
        .iter()
        .filter_map(|s| match s {
            UnitSide::Built {
                candidate,
                crate_digest,
                ..
            } => Some(UnitRef {
                id: candidate.id.clone(),
                crate_digest: crate_digest.clone(),
            }),
            _ => None,
        })
        .collect();
    let mut left_out: Vec<LeftOut> = Vec::new();
    for s in &selected {
        match s {
            Selected::NotVerified { id, reason, .. } => left_out.push(LeftOut {
                id: id.clone(),
                crate_digest: String::new(),
                reason: (*reason).into(),
            }),
            Selected::Ready(_) => {}
        }
    }
    for s in &sides {
        if let UnitSide::SetUp {
            id,
            outcome,
            setup,
            crate_digest,
        } = s
        {
            let reason = match *outcome {
                "not-verified" => setup.reason.as_deref().unwrap_or("not-fresh").to_string(),
                o => o.to_string(),
            };
            let reason = if res::LEFT_OUT_REASONS.contains(&reason.as_str()) {
                reason
            } else {
                "not-fresh".into()
            };
            left_out.push(LeftOut {
                id: id.clone(),
                crate_digest: crate_digest.clone(),
                reason,
            });
        }
    }
    let program = if built.len() < 2 || (!req.units.is_empty() && !req.as_it_stands_only) {
        Program::None
    } else {
        let names: Vec<String> = held.iter().map(|u| u.id.clone()).collect();
        let left: Vec<String> = left_out
            .iter()
            .map(|l| format!("{} left out: {}", l.id, l.reason))
            .collect();
        progress.message(&format!(
            "the program as it stands — {}{}",
            names.join(", "),
            if left.is_empty() {
                String::new()
            } else {
                format!(" ({})", left.join("; "))
            }
        ));
        as_it_stands(
            &built,
            &held,
            &c_files,
            &base,
            &link_args,
            &runner,
            &slot(Slot::AsItStands)?.join(&name),
            &kept_objects,
            &log_name,
            &mut log,
        )?
    };
    if matches!(program, Program::None) && req.units.is_empty() && !req.as_it_stands_only {
        let left: Vec<String> = left_out
            .iter()
            .map(|l| format!("{} left out: {}", l.id, l.reason))
            .collect();
        let tail = if left.is_empty() {
            String::new()
        } else {
            format!(" — {}", left.join("; "))
        };
        progress.message(&match held.len() {
            0 => format!("no accepted unit to compare yet{tail}"),
            _ => format!(
                "one unit measured ({}){tail} — the program as it stands needs two",
                held.iter()
                    .map(|u| u.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if req.as_it_stands_only && matches!(program, Program::None) {
        let why = match built.len() {
            0 => "no accepted unit to compare yet".to_string(),
            _ => format!(
                "one unit measured ({}) — the program as it stands needs two",
                held.iter()
                    .map(|u| u.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        return Err(Error::Invariant(why));
    }
    if !log.is_empty() {
        let _ = harness_core::ledger::write_atomic(&log_path, log.as_bytes());
    }

    // The computer, read once (and on every record after).
    let facts_now = launcher::computer_facts(&launcher)?;
    let shared = Shared {
        program_digest: harness_core::features::program_digest_now(target, facts),
        program_name: name.clone(),
        computer: res::Computer {
            os: facts_now.os.clone(),
            build: facts_now.build.clone(),
            arch: facts_now.arch.clone(),
            cpu: facts_now.cpu.clone(),
            two_kinds: facts_now.two_kinds,
            fast_cores: facts_now.fast_cores,
        },
        cc,
        rustc,
        platform: Platform::MacV6,
    };
    let ctx = RunCtx {
        launcher: &launcher,
        host: &host,
        root: &root,
        name: &name,
        timeout_secs: base.timeout.as_secs().max(1),
    };
    let out_root = build::perf_out(&root)?;
    let mut store = Store::load(perf_dir, workloads)?;
    let mut summary = PerfSummary::default();

    let chosen: Vec<&Workload> = workloads
        .workloads
        .iter()
        .filter(|w| req.workloads.is_empty() || req.workloads.contains(&w.id))
        .collect();
    let unit_wanted =
        |id: &str| measure_units && (req.units.is_empty() || req.units.iter().any(|u| u == id));
    for w in chosen {
        let input = match &w.input {
            Some(rel) => match wl::read_input(&root, rel) {
                Ok(bytes) => Some(bytes),
                Err(reason) => {
                    progress.message(&reason.words(rel));
                    let setup = SetupFacts {
                        input: Some(reason.token().into()),
                        ..SetupFacts::default()
                    };
                    let inputs = |kind: RowKind, crates: Option<Vec<CrateDigest>>| {
                        row_inputs(&shared, w, None, kind, crates, None, None, None)
                    };
                    if req.units.is_empty() && !req.as_it_stands_only {
                        let row = set_up_row(
                            w,
                            "input-unusable",
                            setup.clone(),
                            inputs(RowKind::CAlone, None),
                        );
                        store.put(RowSide::C, row, progress, &mut summary)?;
                    }
                    for s in &sides {
                        if unit_wanted(s.id()) {
                            let row = set_up_row(
                                w,
                                "input-unusable",
                                setup.clone(),
                                inputs(RowKind::Unit, None),
                            );
                            store.put(RowSide::Unit(s.id()), row, progress, &mut summary)?;
                        }
                    }
                    if !matches!(program, Program::None) {
                        let row = set_up_row(
                            w,
                            "input-unusable",
                            setup.clone(),
                            inputs(RowKind::AsItStands, None),
                        );
                        store.put(RowSide::Program, row, progress, &mut summary)?;
                    }
                    continue;
                }
            },
            None => None,
        };
        let input = input.as_deref();
        let digest = wl::digest(w, input);
        let runs = req.runs.unwrap_or(w.runs);
        // The C alone (§3.6).
        if req.units.is_empty() && !req.as_it_stands_only {
            progress.message(&format!(
                "the C on {} — checking it ends the same way twice…",
                w.id
            ));
            let row = c_alone_row(&ctx, &c_bin, w, input, &digest, runs, &shared, progress)?;
            store.put(RowSide::C, row, progress, &mut summary)?;
        }
        // A verified unit perf cannot measure today: its own row says why.
        for s in &selected {
            if let Selected::NotVerified {
                id,
                reason,
                attempt,
            } = s
            {
                if !unit_wanted(id) {
                    continue;
                }
                let setup = SetupFacts {
                    reason: Some((*reason).into()),
                    attempt: attempt.clone(),
                    ..SetupFacts::default()
                };
                let inputs = row_inputs(
                    &shared,
                    w,
                    Some(&digest),
                    RowKind::Unit,
                    None,
                    None,
                    None,
                    None,
                );
                let row = set_up_row(w, "not-verified", setup, inputs);
                store.put(RowSide::Unit(id), row, progress, &mut summary)?;
            }
        }
        // Each unit.
        for s in &sides {
            if !unit_wanted(s.id()) {
                continue;
            }
            let row = match s {
                UnitSide::SetUp {
                    id,
                    outcome,
                    setup,
                    crate_digest,
                } => set_up_row(
                    w,
                    outcome,
                    setup.clone(),
                    row_inputs(
                        &shared,
                        w,
                        Some(&digest),
                        RowKind::Unit,
                        Some(vec![CrateDigest {
                            id: id.clone(),
                            digest: if crate_digest.is_empty() {
                                empty_digest()
                            } else {
                                crate_digest.clone()
                            },
                        }]),
                        None,
                        None,
                        None,
                    ),
                ),
                UnitSide::Built {
                    candidate,
                    bin,
                    facts: af,
                    crate_digest,
                    profile,
                    ..
                } => {
                    progress.message(&format!(
                        "{} on {} — C, {}, C…",
                        candidate.id, w.id, candidate.id
                    ));
                    let inputs = row_inputs(
                        &shared,
                        w,
                        Some(&digest),
                        RowKind::Unit,
                        Some(vec![CrateDigest {
                            id: candidate.id.clone(),
                            digest: crate_digest.clone(),
                        }]),
                        Some(candidate.replaces_rel.clone()),
                        None,
                        None,
                    );
                    match side_row(
                        &ctx,
                        &c_bin,
                        bin,
                        w,
                        input,
                        runs,
                        inputs,
                        &candidate.id,
                        &shared,
                        progress,
                    )? {
                        SideResult::Row(mut row, outputs) => {
                            row.std = Some(af.std || af.fat_lto);
                            row.fat_lto = Some(af.fat_lto);
                            if !profile.is_empty() {
                                row.profile = Some(profile.clone());
                            }
                            keep_outputs(
                                &out_root,
                                RowSide::Unit(&candidate.id),
                                &mut row,
                                outputs,
                            )?;
                            row
                        }
                        SideResult::CSide(c_row) => {
                            store.put(RowSide::C, c_row, progress, &mut summary)?;
                            continue;
                        }
                    }
                }
            };
            store.put(RowSide::Unit(s.id()), row, progress, &mut summary)?;
        }
        // The program as it stands.
        match &program {
            Program::None => {}
            Program::SetUp {
                outcome,
                setup,
                units,
            } => {
                let row = set_up_row(
                    w,
                    outcome,
                    setup.clone(),
                    row_inputs(
                        &shared,
                        w,
                        Some(&digest),
                        RowKind::AsItStands,
                        None,
                        None,
                        Some(units.clone()),
                        Some(left_out.clone()),
                    ),
                );
                store.put(RowSide::Program, row, progress, &mut summary)?;
            }
            Program::Built { bin, units } => {
                progress.message(&format!("the program as it stands on {} — C, it, C…", w.id));
                let crates: Vec<CrateDigest> = units
                    .iter()
                    .map(|u| CrateDigest {
                        id: u.id.clone(),
                        digest: u.crate_digest.clone(),
                    })
                    .collect();
                let inputs = row_inputs(
                    &shared,
                    w,
                    Some(&digest),
                    RowKind::AsItStands,
                    Some(crates),
                    None,
                    Some(units.clone()),
                    Some(left_out.clone()),
                );
                match side_row(
                    &ctx,
                    &c_bin,
                    bin,
                    w,
                    input,
                    runs,
                    inputs,
                    "the program as it stands",
                    &shared,
                    progress,
                )? {
                    SideResult::Row(mut row, outputs) => {
                        row.std = Some(built.iter().any(|s| matches!(s, UnitSide::Built { facts, .. } if facts.std || facts.fat_lto)));
                        keep_outputs(&out_root, RowSide::Program, &mut row, outputs)?;
                        store.put(RowSide::Program, row, progress, &mut summary)?;
                    }
                    SideResult::CSide(c_row) => {
                        store.put(RowSide::C, c_row, progress, &mut summary)?
                    }
                }
            }
        }
    }
    // Rows of units now named only as left out stay as they were.
    let _ = (&held, &left_out);
    Ok(summary)
}

/// The units perf could measure today, in plan order (read-only: no lock,
/// no build) — for `perf show`'s currency of the program as it stands.
pub fn perf_measurable(
    target: &TargetContext,
    plan: &Plan,
    facts: &Facts,
) -> Result<Vec<String>, Error> {
    let ledger = Ledger::new(target.root.clone());
    Ok(select(target, &ledger, facts, plan)?
        .into_iter()
        .filter_map(|s| match s {
            Selected::Ready(c) => Some(c.id),
            Selected::NotVerified { .. } => None,
        })
        .collect())
}

/// The computer's facts when perf's launcher cache is current (never
/// building it): `None` → "computer not checked — run harness perf run
/// once" (§3.9).
pub fn perf_computer_if_cached() -> Option<res::Computer> {
    let host = HostDirs::from_env().ok()?;
    let launcher = launcher::existing_launcher(&host)?;
    let f = launcher::computer_facts(&launcher).ok()?;
    Some(res::Computer {
        os: f.os,
        build: f.build,
        arch: f.arch,
        cpu: f.cpu,
        two_kinds: f.two_kinds,
        fast_cores: f.fast_cores,
    })
}

fn empty_digest() -> String {
    harness_core::hash::bytes_hash(b"")
}

fn first_lines(words: &str, n: usize) -> String {
    words
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(n)
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The program as it stands: refused with words before the link when the
/// units' panic runtimes differ (mixed-panic); else linked, a failure
/// worded from the archives' facts (build note 15) or the linker's lines.
#[allow(clippy::too_many_arguments)]
fn as_it_stands(
    built: &[&UnitSide],
    held: &[UnitRef],
    c_files: &[PathBuf],
    base: &Base,
    link_args: &[String],
    runner: &Runner,
    out: &Path,
    kept_objects: &dyn Fn(&[PathBuf]) -> Vec<PathBuf>,
    log_name: &str,
    log: &mut String,
) -> Result<Program, Error> {
    let mut runtimes = Vec::new();
    let mut found = std::collections::BTreeSet::new();
    let mut replaces: Vec<PathBuf> = Vec::new();
    let mut libs: Vec<PathBuf> = Vec::new();
    let mut no_std: Vec<String> = Vec::new();
    let mut lto: Vec<String> = Vec::new();
    let mut std_units = 0;
    for s in built {
        if let UnitSide::Built {
            candidate,
            staticlib,
            facts,
            ..
        } = s
        {
            runtimes.push(UnitRuntime {
                id: candidate.id.clone(),
                runtime: facts.runtime.token().into(),
            });
            if facts.runtime != PanicRuntime::None {
                found.insert(facts.runtime.token());
            }
            replaces.extend(candidate.replaces.iter().cloned());
            libs.push(staticlib.clone());
            if facts.no_std() {
                no_std.push(candidate.id.clone());
            } else {
                std_units += 1;
            }
            if facts.fat_lto {
                lto.push(candidate.id.clone());
            }
        }
    }
    if found.len() > 1 {
        return Ok(Program::SetUp {
            outcome: "mixed-panic",
            setup: SetupFacts {
                runtimes: Some(runtimes),
                ..SetupFacts::default()
            },
            units: held.to_vec(),
        });
    }
    let kept = match build::kept_c_files(c_files, &replaces) {
        Ok(k) => k,
        Err(_) => c_files
            .iter()
            .filter(|c| !replaces.contains(c))
            .cloned()
            .collect(),
    };
    let group = !cfg!(target_os = "macos");
    match build::link_side(
        base,
        link_args,
        runner,
        out,
        &kept_objects(&kept),
        &libs,
        group,
    )? {
        Ok(bin) => Ok(Program::Built {
            bin,
            units: held.to_vec(),
        }),
        Err(words) => {
            log.push_str(&format!(
                "the program as it stands — does not link:\n{words}\n\n"
            ));
            let (cause, units) = if no_std.len() >= 2 {
                ("two-no-std", no_std)
            } else if !no_std.is_empty() && std_units > 0 {
                ("no-std", no_std)
            } else if lto.len() >= 2 {
                ("two-lto", lto)
            } else if !lto.is_empty() && std_units > 1 {
                ("lto", lto)
            } else {
                ("unknown", held.iter().map(|u| u.id.clone()).collect())
            };
            Ok(Program::SetUp {
                outcome: "does-not-link",
                setup: SetupFacts {
                    cause: Some(cause.into()),
                    units: Some(units),
                    log: Some(log_name.into()),
                    ..SetupFacts::default()
                },
                units: held.to_vec(),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn row_inputs(
    shared: &Shared,
    w: &Workload,
    digest: Option<&str>,
    kind: RowKind,
    crates: Option<Vec<CrateDigest>>,
    replaces: Option<Vec<String>>,
    units: Option<Vec<UnitRef>>,
    left_out: Option<Vec<LeftOut>>,
) -> RowInputs {
    RowInputs {
        workload: digest.map_or_else(|| wl::digest(w, None), str::to_string),
        program: shared.program_digest.clone(),
        crates: crates.filter(|_| kind != RowKind::CAlone),
        replaces: replaces.filter(|_| kind == RowKind::Unit),
        program_name: shared.program_name.clone(),
        units: units.filter(|_| kind == RowKind::AsItStands),
        left_out: left_out.filter(|l| kind == RowKind::AsItStands && !l.is_empty()),
        recipe: harness_core::perf::PERF_RECIPE.into(),
        launcher: harness_core::perf::PERF_LAUNCHER.into(),
        computer: shared.computer.clone(),
        compilers: res::Compilers {
            cc: if shared.cc.is_empty() {
                "cc".into()
            } else {
                shared.cc.clone()
            },
            rustc: (kind != RowKind::CAlone).then(|| {
                if shared.rustc.is_empty() {
                    "rustc".into()
                } else {
                    shared.rustc.clone()
                }
            }),
        },
    }
}

fn set_up_row(w: &Workload, outcome: &str, setup: SetupFacts, inputs: RowInputs) -> Row {
    let has = setup != SetupFacts::default();
    Row {
        workload: w.id.clone(),
        outcome: outcome.into(),
        short: None,
        runs: None,
        platform_metrics: None,
        inputs,
        c: None,
        other: None,
        std: None,
        fat_lto: None,
        profile: None,
        step1: None,
        failed_run: None,
        setup: has.then_some(setup),
        first_difference: None,
        found_before: None,
        last_try: None,
    }
}

fn bare_row(w: &Workload, outcome: &str, inputs: RowInputs) -> Row {
    set_up_row(w, outcome, SetupFacts::default(), inputs)
}

/// The C alone on `w` (§3.5 *The C alone*, §3.6).
#[allow(clippy::too_many_arguments)]
fn c_alone_row(
    ctx: &RunCtx<'_>,
    c: &Hashed,
    w: &Workload,
    input: Option<&[u8]>,
    digest: &str,
    runs: u32,
    shared: &Shared,
    progress: &mut dyn PerfProgress,
) -> Result<Row, Error> {
    let inputs = row_inputs(
        shared,
        w,
        Some(digest),
        RowKind::CAlone,
        None,
        None,
        None,
        None,
    );
    let c1 = run_once(ctx, c, w, input, true)?;
    let c2 = run_once(ctx, c, w, input, true)?;
    let step1 = Step1 {
        c_first: step1_run(&c1),
        other: None,
        c_second: step1_run(&c2),
    };
    for j in [&c1, &c2] {
        if let Some(o) = c_side(j) {
            say_unmeasurable(j, "the C", w, progress);
            let mut row = bare_row(w, o, inputs);
            if let Judged::NeverStarted(errno) = j {
                row.setup = Some(never_started(*errno));
            }
            if o != "run-failed: unmeasurable" && o != "c-could-not-start" {
                row.step1 = Some(step1);
            }
            return Ok(row);
        }
    }
    if difference(&c1, &c2).is_some() {
        let mut row = bare_row(w, "c-unstable", inputs);
        row.step1 = Some(step1);
        return Ok(row);
    }
    note_c_exit(&c1, w, progress);
    let floor = fewer(&step1.c_first, &step1.c_second);
    if under_both(floor) {
        let mut row = bare_row(w, "too-short", inputs);
        row.step1 = Some(step1);
        return Ok(row);
    }
    let short = under_either(floor);
    progress.message("keep the computer quiet while it measures");
    let expected = c1.end_token();
    let mut c_runs = Vec::with_capacity(runs as usize);
    for i in 0..runs {
        progress.message(&format!(
            "the C on {} — timed run {} of {runs}…",
            w.id,
            i + 1
        ));
        let j = run_once(ctx, c, w, input, false)?;
        match j {
            Judged::Ended { run, .. } if run.end == expected => c_runs.push(*run),
            other => {
                say_unmeasurable(&other, "the C", w, progress);
                return Ok(failed(w, inputs, "c", i + 1, &other, step1));
            }
        }
    }
    let metric = perf_words::choose_metric(
        &c_runs,
        &c_runs,
        platform(&c_runs, shared),
        shared.computer.two_kinds,
    );
    Ok(Row {
        workload: w.id.clone(),
        outcome: "baseline".into(),
        short: Some(short),
        runs: Some(runs),
        platform_metrics: Some(metric.into()),
        inputs,
        c: Some(c_runs),
        other: None,
        std: None,
        fat_lto: None,
        profile: None,
        step1: None,
        failed_run: None,
        setup: None,
        first_difference: None,
        found_before: None,
        last_try: None,
    })
}

fn never_started(errno: Option<i32>) -> SetupFacts {
    SetupFacts {
        never_started: Some(errno.map_or_else(|| "no-ready".to_string(), |e| e.to_string())),
        ..SetupFacts::default()
    }
}

/// The platform family the runs show: V4 when two kinds of cores but no
/// performance-core counts.
fn platform(runs: &[Run], shared: &Shared) -> Platform {
    let p = runs.iter().any(|r| r.p_cycles.is_some());
    if shared.computer.two_kinds && !p {
        Platform::MacV4
    } else {
        shared.platform
    }
}

/// "the C exits 1 on big-text: its error path is timed — <its first stderr
/// line>" (§3.10, build note 26).
fn note_c_exit(c: &Judged, w: &Workload, progress: &mut dyn PerfProgress) {
    if let Judged::Ended {
        end: End::Exit(code),
        stderr,
        ..
    } = c
    {
        if *code != 0 {
            let first: String = String::from_utf8_lossy(stderr)
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .map(|c| {
                    if harness_core::text::unsafe_to_show(c) {
                        '?'
                    } else {
                        c
                    }
                })
                .take(80)
                .collect();
            progress.message(&format!(
                "the C exits {code} on {}: its error path is timed{}",
                w.id,
                if first.is_empty() {
                    String::new()
                } else {
                    format!(" — {first}")
                }
            ));
        }
    }
}

fn failed(
    w: &Workload,
    inputs: RowInputs,
    side: &str,
    index: u32,
    j: &Judged,
    step1: Step1,
) -> Row {
    let (outcome, end) = match j {
        Judged::TimedOut => ("run-failed: timeout", "timeout".to_string()),
        Judged::Ended {
            end: End::Exit(n), ..
        } => ("run-failed: exit", format!("exit {n}")),
        Judged::Ended {
            end: End::Signal(n),
            ..
        } => ("run-failed: signal", format!("signal {n}")),
        Judged::SigKilled => ("stopped-by-sigkill", "signal 9".to_string()),
        Judged::NeverStarted(_) => ("run-failed: unmeasurable", "never-started".to_string()),
        Judged::Unmeasurable(_) | Judged::Overflow => {
            ("run-failed: unmeasurable", "timeout".to_string())
        }
    };
    let mut row = bare_row(w, outcome, inputs);
    if outcome.starts_with("run-failed: ") && outcome != "run-failed: unmeasurable" {
        row.failed_run = Some(FailedRun {
            side: side.into(),
            index,
            end,
        });
        row.step1 = Some(step1);
    }
    row
}

/// The two step-1 runs' streams of a behaves-differently row, to keep:
/// the C's stdout and stderr, then the other side's.
type Outputs = [Vec<u8>; 4];

enum SideResult {
    /// The side's row, with the outputs to keep when it behaves differently.
    Row(Row, Option<Outputs>),
    /// A C-side outcome found in step 1: written to the C-alone row only.
    CSide(Row),
}

/// One side against the C on `w` (§3.5 *Per side against the C*).
#[allow(clippy::too_many_arguments)]
fn side_row(
    ctx: &RunCtx<'_>,
    c: &Hashed,
    other: &Hashed,
    w: &Workload,
    input: Option<&[u8]>,
    runs: u32,
    inputs: RowInputs,
    label: &str,
    shared: &Shared,
    progress: &mut dyn PerfProgress,
) -> Result<SideResult, Error> {
    let c1 = run_once(ctx, c, w, input, true)?;
    let o = run_once(ctx, other, w, input, true)?;
    let c2 = run_once(ctx, c, w, input, true)?;
    let step1 = Step1 {
        c_first: step1_run(&c1),
        other: Some(step1_run(&o)),
        c_second: step1_run(&c2),
    };
    let c_inputs = || {
        let mut i = inputs.clone();
        i.crates = None;
        i.replaces = None;
        i.units = None;
        i.left_out = None;
        i.compilers.rustc = None;
        i
    };
    for j in [&c1, &c2] {
        if let Some(outcome) = c_side(j) {
            say_unmeasurable(j, "the C", w, progress);
            let mut row = bare_row(w, outcome, c_inputs());
            if let Judged::NeverStarted(errno) = j {
                row.setup = Some(never_started(*errno));
            }
            if outcome != "run-failed: unmeasurable" && outcome != "c-could-not-start" {
                row.step1 = Some(Step1 {
                    other: None,
                    ..step1.clone()
                });
            }
            return Ok(SideResult::CSide(row));
        }
    }
    if difference(&c1, &c2).is_some() {
        let mut row = bare_row(w, "c-unstable", c_inputs());
        row.step1 = Some(Step1 {
            other: None,
            ..step1
        });
        return Ok(SideResult::CSide(row));
    }
    note_c_exit(&c1, w, progress);
    // The other side in step 1.
    match &o {
        Judged::TimedOut => {
            let mut row = bare_row(w, "run-failed: timeout", inputs);
            row.failed_run = None;
            row.step1 = Some(step1);
            return Ok(SideResult::Row(row, None));
        }
        Judged::Overflow => {
            let mut row = bare_row(w, "behaves-differently", inputs);
            let (c_end, c_len) = match &c1 {
                Judged::Ended { end, stdout, .. } => (end.token(), stdout.len() as u64),
                other => (other.end_token(), 0),
            };
            row.first_difference = Some(Difference {
                stream: "stdout".into(),
                c_len,
                other_len: OUTPUT_CAP as u64,
                offset: 0,
                c_end,
                other_end: "exit 0".into(),
                over_cap: true,
                kept: Vec::new(),
            });
            row.step1 = Some(step1);
            return Ok(SideResult::Row(row, None));
        }
        Judged::NeverStarted(errno) => {
            let mut row = bare_row(w, "could-not-start", inputs);
            row.setup = Some(never_started(*errno));
            return Ok(SideResult::Row(row, None));
        }
        Judged::SigKilled => {
            let mut row = bare_row(w, "stopped-by-sigkill", inputs);
            row.step1 = Some(step1);
            return Ok(SideResult::Row(row, None));
        }
        Judged::Unmeasurable(_) => {
            say_unmeasurable(&o, label, w, progress);
            return Ok(SideResult::Row(
                bare_row(w, "run-failed: unmeasurable", inputs),
                None,
            ));
        }
        Judged::Ended { .. } => {}
    }
    if let Some(d) = difference(&c1, &o) {
        let mut row = bare_row(w, "behaves-differently", inputs);
        row.first_difference = Some(d);
        row.step1 = Some(step1);
        let outputs = match (c1, o) {
            (
                Judged::Ended {
                    stdout: co,
                    stderr: ce,
                    ..
                },
                Judged::Ended {
                    stdout: oo,
                    stderr: oe,
                    ..
                },
            ) => Some([co, ce, oo, oe]),
            _ => None,
        };
        return Ok(SideResult::Row(row, outputs));
    }
    // The floor (§3.5 step 2).
    let cf = fewer(&step1.c_first, &step1.c_second);
    let of = step1.other.clone().expect("the other side ran");
    if under_both(cf) && under_both(&of) {
        let mut row = bare_row(w, "too-short", inputs);
        row.step1 = Some(step1);
        return Ok(SideResult::Row(row, None));
    }
    let short = under_either(cf) || under_either(&of);
    progress.message("keep the computer quiet while it measures");
    let (c_end, o_end) = (c1.end_token(), o.end_token());
    let mut c_runs = Vec::with_capacity(runs as usize);
    let mut o_runs = Vec::with_capacity(runs as usize);
    for i in 0..runs {
        progress.message(&format!(
            "{label} on {} — timed run {} of {}…",
            w.id,
            2 * i + 1,
            2 * runs
        ));
        match run_once(ctx, c, w, input, false)? {
            Judged::Ended { run, .. } if run.end == c_end => c_runs.push(*run),
            j => {
                say_unmeasurable(&j, "the C", w, progress);
                return Ok(SideResult::Row(
                    failed(w, inputs, "c", i + 1, &j, step1),
                    None,
                ));
            }
        }
        match run_once(ctx, other, w, input, false)? {
            Judged::Ended { run, .. } if run.end == o_end => o_runs.push(*run),
            j => {
                say_unmeasurable(&j, label, w, progress);
                return Ok(SideResult::Row(
                    failed(w, inputs, "other", i + 1, &j, step1),
                    None,
                ));
            }
        }
    }
    let all: Vec<Run> = c_runs.iter().chain(o_runs.iter()).cloned().collect();
    let metric = perf_words::choose_metric(
        &c_runs,
        &o_runs,
        platform(&all, shared),
        shared.computer.two_kinds,
    );
    Ok(SideResult::Row(
        Row {
            workload: w.id.clone(),
            outcome: "measured".into(),
            short: Some(short),
            runs: Some(runs),
            platform_metrics: Some(metric.into()),
            inputs,
            c: Some(c_runs),
            other: Some(o_runs),
            std: None,
            fat_lto: None,
            profile: None,
            step1: None,
            failed_run: None,
            setup: None,
            first_difference: None,
            found_before: None,
            last_try: None,
        },
        None,
    ))
}

/// Keep a behaves-differently row's outputs (§3.9 *Kept outputs*) and
/// record each file's size and blake3; a row that clears the finding
/// removes them.
fn keep_outputs(
    out_root: &Path,
    side: RowSide<'_>,
    row: &mut Row,
    outputs: Option<Outputs>,
) -> Result<(), Error> {
    let folder = match side {
        RowSide::Unit(id) => build::sub_folder(&build::sub_folder(out_root, "units")?, id)?,
        RowSide::Program | RowSide::C => build::sub_folder(out_root, "program")?,
    };
    let names = ["c.stdout", "c.stderr", "other.stdout", "other.stderr"]
        .map(|s| format!("{}.{s}", row.workload));
    match (&mut row.first_difference, outputs) {
        (Some(d), Some(streams)) => {
            for (name, bytes) in names.iter().zip(streams.iter()) {
                let path = folder.join(name);
                if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                    std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                }
                let cut = &bytes[..bytes.len().min(OUTPUT_CAP)];
                harness_core::ledger::write_atomic(&path, cut)?;
                d.kept.push(KeptFile {
                    name: name.clone(),
                    size: cut.len() as u64,
                    blake3: harness_core::hash::bytes_hash(cut),
                });
            }
        }
        _ if matches!(row.outcome.as_str(), "measured" | "too-short") => {
            for name in &names {
                let _ = std::fs::remove_file(folder.join(name));
            }
        }
        _ => {}
    }
    Ok(())
}

// ---- The results files -------------------------------------------------

/// The results as they are written, row by row.
struct Store {
    dir: PathBuf,
    program: res::ProgramResults,
    units: std::collections::BTreeMap<String, res::UnitResults>,
    /// The workloads still in the file: rows of removed ones are dropped on
    /// the next write.
    workloads: Vec<String>,
}

impl Store {
    fn load(dir: &Path, workloads: &Workloads) -> Result<Store, Error> {
        let program = res::read_program(&res::program_path(dir))?.unwrap_or_default();
        Ok(Store {
            dir: dir.to_path_buf(),
            program,
            units: std::collections::BTreeMap::new(),
            workloads: workloads.workloads.iter().map(|w| w.id.clone()).collect(),
        })
    }

    fn put(
        &mut self,
        side: RowSide<'_>,
        row: Row,
        progress: &mut dyn PerfProgress,
        summary: &mut PerfSummary,
    ) -> Result<(), Error> {
        let keep = |rows: &mut Vec<Row>, workloads: &[String]| {
            rows.retain(|r| workloads.contains(&r.workload))
        };
        let written = match side {
            RowSide::C | RowSide::Program => {
                let (list, kind) = match side {
                    RowSide::C => (&mut self.program.c_alone, RowKind::CAlone),
                    _ => (&mut self.program.as_it_stands, RowKind::AsItStands),
                };
                let merged = place(list, row, kind);
                keep(&mut self.program.c_alone, &self.workloads);
                keep(&mut self.program.as_it_stands, &self.workloads);
                res::write_program(&res::program_path(&self.dir), &self.program)?;
                merged
            }
            RowSide::Unit(id) => {
                if !self.units.contains_key(id) {
                    let path = res::unit_path(&self.dir, id);
                    let loaded =
                        res::read_unit(&path, id)?.unwrap_or_else(|| res::UnitResults::new(id));
                    self.units.insert(id.to_string(), loaded);
                }
                let file = self.units.get_mut(id).expect("loaded");
                let merged = place(&mut file.rows, row, RowKind::Unit);
                keep(&mut file.rows, &self.workloads);
                let units_dir = build::sub_folder(&self.dir, res::UNITS_DIR)?;
                res::write_unit(&units_dir.join(format!("{id}.json")), file)?;
                merged
            }
        };
        summary.rows += 1;
        match written.outcome.as_str() {
            "baseline" | "measured" => summary.measured += 1,
            "too-short" => summary.too_short += 1,
            "behaves-differently" => summary.behaves_differently += 1,
            _ => {}
        }
        progress.row(side, &written);
        Ok(())
    }
}

/// Merge `row` into `list` by the replace rule; returns the stored row.
fn place(list: &mut Vec<Row>, row: Row, kind: RowKind) -> Row {
    let at = list.iter().position(|r| r.workload == row.workload);
    let merged = res::merge(at.map(|i| &list[i]), row, kind);
    match at {
        Some(i) => list[i] = merged.clone(),
        None => list.push(merged.clone()),
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seen {
        messages: Vec<String>,
        rows: Vec<(String, Row)>,
    }

    impl PerfProgress for Seen {
        fn message(&mut self, text: &str) {
            self.messages.push(text.to_string());
        }
        fn row(&mut self, side: RowSide<'_>, row: &Row) {
            let side = match side {
                RowSide::C => "c".to_string(),
                RowSide::Program => "program".to_string(),
                RowSide::Unit(id) => id.to_string(),
            };
            self.rows.push((side, row.clone()));
        }
    }

    /// The C alone on day one (§3.6), end to end through the launcher: a
    /// baseline row on a long enough workload, too-short on a tiny one, an
    /// unusable input in its own words, the results file written strictly.
    #[test]
    fn the_c_alone_end_to_end() {
        if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-c-alone");
        let root = tmp.path().canonicalize().expect("root");
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(&p, text).expect("write");
        };
        put(
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"tool\"\nsource_dir = \"src\"\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\ntimeout_secs = 60\n",
        );
        put(
            "src/main.c",
            "#include <stdio.h>\n#include <stdlib.h>\nint main(int argc, char **argv) {\n\
             FILE *f = fopen(argv[1], \"r\"); long n = 0; int c; if (!f) return 2;\n\
             while ((c = fgetc(f)) != EOF) n += c; fclose(f);\n\
             volatile unsigned long x = 0; unsigned long k = strtoul(argv[2], 0, 10);\n\
             for (unsigned long i = 0; i < k; i++) x += i ^ (unsigned long)n;\n\
             printf(\"%ld %s\\n\", n, argv[0]); return 0; }\n",
        );
        put("bench/in.txt", "hello\n");
        std::fs::create_dir_all(root.join("migration/perf")).expect("perf dir");
        let target = TargetContext::load(&root).expect("target");
        let workloads = wl::parse(
            "schema_version = 1\n\
             [[workload]]\nid = \"long\"\nargs = [\"{input}\", \"400000000\"]\ninput = \"bench/in.txt\"\nruns = 5\n\
             [[workload]]\nid = \"tiny\"\nargs = [\"{input}\", \"10\"]\ninput = \"bench/in.txt\"\nruns = 5\n\
             [[workload]]\nid = \"gone\"\nargs = [\"{input}\"]\ninput = \"bench/gone.txt\"\n",
            Path::new("w.toml"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let plan = Plan {
            schema_version: 1,
            target: "tool".into(),
            units: Vec::new(),
        };
        let mut seen = Seen {
            messages: Vec::new(),
            rows: Vec::new(),
        };
        let perf_dir = root.join("migration/perf");
        // Stale facts are refused: scan first.
        let stale = perf_run(
            &target,
            &plan,
            &Facts::default(),
            &workloads,
            &perf_dir,
            &PerfRequest::default(),
            &mut seen,
        );
        assert!(
            matches!(&stale, Err(Error::Invariant(w)) if w.contains("harness scan")),
            "{stale:?}"
        );
        let facts = Facts {
            files: vec![harness_core::facts::FileRecord {
                path: "src/main.c".into(),
                hash: harness_core::hash::file_hash(&root.join("src/main.c")).expect("hash"),
                includes: Vec::new(),
            }],
            ..Facts::default()
        };
        let summary = perf_run(
            &target,
            &plan,
            &facts,
            &workloads,
            &perf_dir,
            &PerfRequest::default(),
            &mut seen,
        )
        .expect("perf runs");
        let outcome = |id: &str| {
            seen.rows
                .iter()
                .find(|(_, r)| r.workload == id)
                .map(|(s, r)| (s.clone(), r.outcome.clone()))
        };
        assert_eq!(
            outcome("long"),
            Some(("c".into(), "baseline".into())),
            "{:?}",
            seen.messages
        );
        assert_eq!(outcome("tiny"), Some(("c".into(), "too-short".into())));
        assert_eq!(outcome("gone"), Some(("c".into(), "input-unusable".into())));
        assert!(
            seen.messages
                .iter()
                .any(|m| m
                    == "bench/gone.txt is not here — put the file back or remove the workload"),
            "{:?}",
            seen.messages
        );
        assert_eq!(summary.measured, 1);
        assert_eq!(summary.too_short, 1);
        // The results file reads back strictly, and its words are rebuilt.
        let program = res::read_program(&res::program_path(&perf_dir))
            .expect("reads")
            .expect("written");
        let long = program
            .c_alone
            .iter()
            .find(|r| r.workload == "long")
            .expect("row");
        assert_eq!(long.c.as_ref().map(Vec::len), Some(5));
        let words = perf_words::words(
            long,
            &perf_words::Context {
                side: perf_words::Side::C,
                workload: "long",
                input: Some("bench/in.txt"),
            },
        );
        assert!(
            words.headline.starts_with("CPU about "),
            "{}",
            words.headline
        );
        let tiny = program
            .c_alone
            .iter()
            .find(|r| r.workload == "tiny")
            .expect("row");
        let words = perf_words::words(
            tiny,
            &perf_words::Context {
                side: perf_words::Side::C,
                workload: "tiny",
                input: Some("bench/in.txt"),
            },
        );
        assert!(
            words.headline.starts_with("too short to time: the C ran"),
            "{}",
            words.headline
        );
    }

    /// One side against the C (§3.5), on hand-built programs: a side that
    /// prints differently is behaves-differently with its first difference
    /// and kept outputs; one that crashes reads "Rust crashed"; an identical
    /// side is measured with both sides' runs; a C that differs between its
    /// own two runs is c-unstable, for the C alone's row only.
    #[test]
    fn a_side_against_the_c() {
        if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-side");
        let root = tmp.path().canonicalize().expect("root");
        let cache = root.join("cache");
        let owner = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&root).expect("meta"));
        std::fs::create_dir(&cache).expect("cache");
        std::fs::set_permissions(&cache, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("mode");
        let l = launcher::launcher_in(&cache, owner, &mut |_| {}).expect("launcher");
        let host = HostDirs::from_env().expect("host");
        let bins = root.join("bins");
        std::fs::create_dir(&bins).expect("bins");
        let build = |name: &str, body: &str| -> Hashed {
            let src = bins.join(format!("{name}.c"));
            std::fs::write(&src, format!("#include <stdio.h>\n#include <stdlib.h>\n#include <time.h>\nint main(void) {{ volatile unsigned long x = 0; for (unsigned long i = 0; i < 300000000UL; i++) x += i; {body} }}\n")).expect("src");
            let bin = bins.join(name);
            assert!(std::process::Command::new("cc")
                .args(["-O0", "-o"])
                .arg(&bin)
                .arg(&src)
                .status()
                .expect("cc")
                .success());
            Hashed::new(&bin.canonicalize().expect("bin")).expect("hash")
        };
        let c = build("c", "printf(\"same\\n\"); return 0;");
        let same = build("same", "printf(\"same\\n\"); return 0;");
        let differs = build("differs", "printf(\"other\\n\"); return 0;");
        let crashes = build("crashes", "printf(\"same\\n\"); fflush(stdout); abort();");
        let unstable = build(
            "unstable",
            "printf(\"%ld\\n\", (long)clock() ^ (long)time(0) ^ (long)&x); return 0;",
        );
        let ctx = RunCtx {
            launcher: &l,
            host: &host,
            root: &root,
            name: "tool",
            timeout_secs: 60,
        };
        let w = Workload {
            id: "w".into(),
            args: Vec::new(),
            input: None,
            runs: 5,
        };
        let shared = Shared {
            program_digest: harness_core::hash::bytes_hash(b"p"),
            program_name: "tool".into(),
            computer: res::Computer {
                os: "x".into(),
                build: "x".into(),
                arch: "x".into(),
                cpu: "x".into(),
                two_kinds: true,
                fast_cores: 4,
            },
            cc: "cc".into(),
            rustc: "rustc".into(),
            platform: Platform::MacV6,
        };
        let inputs = row_inputs(&shared, &w, None, RowKind::Unit, None, None, None, None);
        let mut seen = Seen {
            messages: Vec::new(),
            rows: Vec::new(),
        };
        let out_root = root.join("out");
        std::fs::create_dir(&out_root).expect("out");
        // Prints differently: the first difference, the outputs kept.
        let SideResult::Row(mut row, outputs) = side_row(
            &ctx,
            &c,
            &differs,
            &w,
            None,
            5,
            inputs.clone(),
            "u001",
            &shared,
            &mut seen,
        )
        .expect("runs") else {
            panic!("a row")
        };
        assert_eq!(row.outcome, "behaves-differently");
        keep_outputs(&out_root, RowSide::Unit("u001"), &mut row, outputs).expect("kept");
        let d = row.first_difference.as_ref().expect("difference");
        assert_eq!((d.stream.as_str(), d.offset), ("stdout", 0));
        assert_eq!(d.kept.len(), 4);
        assert_eq!(
            std::fs::read(out_root.join("units/u001/w.other.stdout")).expect("kept"),
            b"other\n"
        );
        res::check_row(&row, RowKind::Unit).expect("valid");
        // A crash: "Rust crashed" by its end.
        let SideResult::Row(row, _) = side_row(
            &ctx,
            &c,
            &crashes,
            &w,
            None,
            5,
            inputs.clone(),
            "u001",
            &shared,
            &mut seen,
        )
        .expect("runs") else {
            panic!("a row")
        };
        assert_eq!(row.outcome, "behaves-differently");
        let words = perf_words::words(
            &row,
            &perf_words::Context {
                side: perf_words::Side::Unit("u001"),
                workload: "w",
                input: None,
            },
        );
        assert_eq!(words.short, "Rust crashed");
        // The same program: measured, five runs a side.
        let SideResult::Row(row, _) = side_row(
            &ctx,
            &c,
            &same,
            &w,
            None,
            5,
            inputs.clone(),
            "u001",
            &shared,
            &mut seen,
        )
        .expect("runs") else {
            panic!("a row")
        };
        assert_eq!(row.outcome, "measured", "{row:?}");
        assert_eq!(row.other.as_ref().map(Vec::len), Some(5));
        res::check_row(&row, RowKind::Unit).expect("valid");
        // A C that differs from itself: the C alone's row.
        let SideResult::CSide(row) = side_row(
            &ctx, &unstable, &same, &w, None, 5, inputs, "u001", &shared, &mut seen,
        )
        .expect("runs") else {
            panic!("the C's row")
        };
        assert_eq!(row.outcome, "c-unstable");
        res::check_row(&row, RowKind::CAlone).expect("valid on the C alone");
    }

    /// The program as it stands (§3.2) from two hand-built units: mixed
    /// panic runtimes are refused in words before the link (naming each
    /// unit's runtime); matching ones link, every unit's staticlib in, every
    /// replaced C file out.
    #[test]
    fn the_program_as_it_stands_links_or_says_why() {
        let bench = crate::testutil::ToolBench::new("perf-ais");
        let root = bench.root().canonicalize().expect("root");
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(&p, text).expect("write");
        };
        put(
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"tool\"\nsource_dir = \"src\"\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        );
        put("src/main.c", "#include <stdio.h>\nint a(void); int b(void);\nint main(void) { printf(\"%d\\n\", a() + b()); return 0; }\n");
        put("src/a.c", "int a(void) { return 1; }\n");
        put("src/b.c", "int b(void) { return 2; }\n");
        let target = harness_core::TargetContext::load(&root).expect("target");
        let base = Base::resolve(&target, "perf", &["cc"]).expect("base");
        let scratch = build::perf_scratch(&root).expect("scratch");
        let obj = build::sub_folder(&scratch, "obj").expect("obj");
        let c_files = program_c_files_in(&base, "perf").expect("files");
        let objects = build::compile_objects(&base, bench.runner(), &c_files, &obj)
            .expect("runs")
            .expect("compiles");
        let kept_objects = |kept: &[PathBuf]| -> Vec<PathBuf> {
            c_files
                .iter()
                .zip(&objects)
                .filter(|(c, _)| kept.contains(c))
                .map(|(_, o)| o.path.clone())
                .collect()
        };
        let unit = |id: &str, func: &str, value: i32, abort: bool, file: &str| -> UnitSide {
            let crate_dir = crate::testutil::fixture_crate(
                &root,
                id,
                abort,
                &format!("#[no_mangle] pub extern \"C\" fn {func}() -> i32 {{ {value} }}\n"),
            );
            let lib = bench.build(&crate_dir);
            let facts = archive_facts(&std::fs::read(&lib).expect("lib")).expect("facts");
            UnitSide::Built {
                candidate: Candidate {
                    id: id.into(),
                    position: 1,
                    replaces_rel: vec![format!("src/{file}")],
                    replaces: vec![root.join("src").join(file).canonicalize().expect("c")],
                    crate_dir,
                    verdict_crate: String::new(),
                },
                bin: Hashed {
                    path: root.join("unused"),
                    digest: String::new(),
                },
                staticlib: lib,
                facts,
                crate_digest: harness_core::hash::bytes_hash(id.as_bytes()),
                profile: Vec::new(),
            }
        };
        let ua = unit("ua", "a", 10, true, "a.c");
        let ub_unwind = unit("ub", "b", 20, false, "b.c");
        let held = vec![
            UnitRef {
                id: "ua".into(),
                crate_digest: harness_core::hash::bytes_hash(b"ua"),
            },
            UnitRef {
                id: "ub".into(),
                crate_digest: harness_core::hash::bytes_hash(b"ub"),
            },
        ];
        let mut log = String::new();
        let out = build::sub_folder(&scratch, "bin/pall")
            .expect("slot")
            .join("tool");
        let p = as_it_stands(
            &[&ua, &ub_unwind],
            &held,
            &c_files,
            &base,
            &[],
            bench.runner(),
            &out,
            &kept_objects,
            "x.log",
            &mut log,
        )
        .expect("runs");
        let Program::SetUp { outcome, setup, .. } = p else {
            panic!("mixed-panic")
        };
        assert_eq!(outcome, "mixed-panic");
        let words = perf_words::set_up_words(
            outcome,
            Some(&setup),
            &perf_words::Context {
                side: perf_words::Side::AsItStands,
                workload: "w",
                input: None,
            },
        );
        assert!(words.starts_with("ua aborts; ub unwinds — "), "{words}");
        assert!(
            words.ends_with("to ub's Cargo.toml, then run harness verify ub"),
            "{words}"
        );
        // Both abort: the program links, both units in, a.c and b.c out.
        let ub = unit("ub2", "b", 20, true, "b.c");
        let p = as_it_stands(
            &[&ua, &ub],
            &held,
            &c_files,
            &base,
            &[],
            bench.runner(),
            &out,
            &kept_objects,
            "x.log",
            &mut log,
        )
        .expect("runs");
        let Program::Built { bin, .. } = p else {
            panic!("links: {log}")
        };
        let printed = std::process::Command::new(&bin.path)
            .output()
            .expect("runs")
            .stdout;
        assert_eq!(printed, b"30\n");
    }

    #[test]
    fn streams_are_rewritten_one_to_one() {
        let tmp = Path::new("/private/var/folders/T/ruharness-perf-1-abc");
        let bin = Path::new("/t/migration/build/.perf/bin/p001/tool");
        let raw = b"$HOME /private/var/folders/T/ruharness-perf-1-abc/run /t/migration/build/.perf/bin/p001/tool";
        assert_eq!(
            rewrite(raw, tmp, bin),
            b"$$HOME $TMPDIR/run $PROGDIR/tool".to_vec()
        );
    }

    #[test]
    fn the_floor_legs() {
        let r = |ins: Option<u64>, cpu: Option<u64>| Step1Run {
            instructions: ins,
            cpu_us: cpu,
            end: "exit 0".into(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        };
        assert!(under_both(&r(Some(1_000), Some(1_000))));
        assert!(
            !under_both(&r(Some(2_000_000_000), Some(1_000))),
            "a memory-bound run past one leg"
        );
        assert!(under_either(&r(Some(2_000_000_000), Some(1_000))));
        assert!(!under_either(&r(Some(2_000_000_000), Some(600_000))));
        // The C's run with fewer instructions decides.
        let a = r(Some(5), Some(9));
        let b = r(Some(3), Some(9));
        assert_eq!(fewer(&a, &b).instructions, Some(3));
    }

    #[test]
    fn a_difference_is_found_by_end_then_stream() {
        let ended = |end: End, out: &[u8], err: &[u8]| Judged::Ended {
            end,
            stdout: out.to_vec(),
            stderr: err.to_vec(),
            run: Box::default(),
        };
        let c = ended(End::Exit(0), b"abc", b"");
        assert!(difference(&c, &ended(End::Exit(0), b"abc", b"")).is_none());
        let d = difference(&c, &ended(End::Exit(1), b"abc", b"")).expect("differs");
        assert_eq!(d.stream, "exit");
        let d = difference(&c, &ended(End::Exit(0), b"abd", b"")).expect("differs");
        assert_eq!((d.stream.as_str(), d.offset), ("stdout", 2));
        let d = difference(&c, &ended(End::Exit(0), b"abc", b"x")).expect("differs");
        assert_eq!(d.stream, "stderr");
        let d = difference(&c, &ended(End::Exit(0), b"ab", b"")).expect("differs");
        assert_eq!((d.offset, d.c_len, d.other_len), (2, 3, 2));
    }
}
