//! perf's measurement (docs/PERF-DESIGN.md §3.2, §3.5–§3.7, §3.9): which
//! units can be measured, the sides built as verify builds them, each row
//! run — the C alone, then each side against the C — every run judged in
//! the design's order, and every row written by the one replace rule.
//! Information only: no verdict, no plan status.

use super::archive::{archive_facts, ArchiveFacts, PanicRuntime};
use super::build::{self, Hashed, Slot};
use super::launcher::{self, End, Launcher, Seen, Status};
use super::tools;
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
pub(crate) const STEP1_EXTRA_SECS: u64 = 60;

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

impl RowSide<'_> {
    /// The side in the progress lines' words.
    fn label(self) -> String {
        match self {
            RowSide::C => "the C".into(),
            RowSide::Program => "the program as it stands".into(),
            RowSide::Unit(id) => id.to_string(),
        }
    }
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

impl PerfRequest {
    /// Whether this run builds unit `id`'s crate and program (§3.10, §6
    /// *Cost*): with `--unit` alone, only the units asked for; otherwise
    /// every measurable unit, as the program as it stands holds them all.
    fn builds(&self, id: &str) -> bool {
        self.units.is_empty() || self.as_it_stands_only || self.units.iter().any(|u| u == id)
    }
}

/// What a perf run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerfSummary {
    /// Rows written.
    pub rows: usize,
    /// Rows measured in this run (baseline or measured) — an earlier row the
    /// replace rule kept is not counted.
    pub measured: usize,
    /// Rows this run found too short to time.
    pub too_short: usize,
    /// Rows this run found behaving differently.
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
        staticlib: Hashed,
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

impl Program {
    /// The units the program as it stands holds, when perf made one.
    fn units(&self) -> Option<&[UnitRef]> {
        match self {
            Program::Built { units, .. } | Program::SetUp { units, .. } => Some(units),
            Program::None => None,
        }
    }
}

/// The tool sandbox for each build step (§3.2 *Build* step 3): the version
/// probes write nowhere, the compiles only `.perf/obj`, each link only its
/// own slot, each cargo build only its crate's `target/` and `Cargo.lock` —
/// so no step (a crate's build script among them) can change what another
/// step built. The hash checks before each link catch what still could.
struct Steps<'a> {
    /// The runner every step's is made from (no profile of its own).
    runner: &'a Runner,
    host: &'a HostDirs,
    root: &'a Path,
}

impl Steps<'_> {
    /// A runner whose tools may write only `dirs` and `files` (and the
    /// temp folders every tool profile allows).
    fn writing(&self, dirs: &[PathBuf], files: &[PathBuf]) -> Result<Runner, Error> {
        let profile = sandbox::render_profile(&ProfileSpec {
            host: self.host,
            target_root: self.root,
            toolchain: true,
            write_dirs: dirs,
            write_files: files,
        })?;
        Ok(Runner {
            tool_profile: Some(profile),
            ..self.runner.clone()
        })
    }
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
    /// A stream passed the capture cap: `stdout` or `stderr`.
    Overflow {
        stream: &'static str,
    },
    /// The launcher's own words for why.
    Unmeasurable(String),
    NeverStarted(Option<i32>),
    /// A SIGKILL perfrun did not send.
    SigKilled,
}

impl Judged {
    /// The run's end as the results store it: a run over the output cap
    /// ends by the SIGKILL that stops it (`signal 9`), never a timeout it
    /// did not have. An unmeasurable run's end is never stored (its row
    /// keeps no step-1 facts).
    fn end_token(&self) -> String {
        match self {
            Judged::Ended { end, .. } => end.token(),
            Judged::TimedOut | Judged::Unmeasurable(_) => "timeout".into(),
            Judged::SigKilled | Judged::Overflow { .. } => "signal 9".into(),
            Judged::NeverStarted(_) => "never-started".into(),
        }
    }
}

/// Which stream passed the capture cap. The harness stops reading a stream
/// at the read that would pass the cap, so that stream holds within one
/// read of it and the other no more than the cap: the longer one (stdout
/// when both are as long).
fn overflowed_stream(stdout_len: usize, stderr_len: usize) -> &'static str {
    if stderr_len > stdout_len {
        "stderr"
    } else {
        "stdout"
    }
}

/// perfrun's deadline for a run (§3.3 *The harness side* step 6): the
/// target's `timeout_secs` for a timed run, and 60 s more in step 1, where
/// every new binary's first exec falls.
fn deadline_secs(timeout_secs: u64, step1: bool) -> u64 {
    timeout_secs + if step1 { STEP1_EXTRA_SECS } else { 0 }
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
    let m = launcher::run_measured(&launcher::RunSpec {
        launcher: ctx.launcher,
        profile: &profile,
        deadline_secs: deadline_secs(ctx.timeout_secs, step1),
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
        Seen::Overflow => Judged::Overflow {
            stream: overflowed_stream(m.stdout.len(), m.stderr.len()),
        },
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

/// The outcome of a C step-1 run that did not end as a program should, or
/// `None` when it ended (§3.3 *Judging a run*, §3.5 step 1): a signal is a
/// crash; a SIGKILL perfrun did not send is `stopped-by-sigkill`, worded
/// without asserting its cause (perf's sandbox kills a program that starts
/// another, but the C may have killed itself); a run the launcher could not
/// measure is `run-failed: unmeasurable`, for the row being measured only.
fn c_side(j: &Judged) -> Option<&'static str> {
    match j {
        Judged::Ended {
            end: End::Signal(_),
            ..
        } => Some("c-crashed"),
        Judged::SigKilled => Some("stopped-by-sigkill"),
        Judged::Ended { .. } => None,
        Judged::TimedOut => Some("c-timed-out"),
        Judged::Overflow { .. } => Some("output-too-large"),
        Judged::NeverStarted(_) => Some("c-could-not-start"),
        Judged::Unmeasurable(_) => Some("run-failed: unmeasurable"),
    }
}

/// Whether the linker's words are about `main` itself — none, or more than
/// one (§3.2 *Build* step 2: "the program needs one main()" only when the
/// link says so) — never a symbol merely called from `main` or an object
/// named after a `main.c`.
fn link_says_main(words: &str) -> bool {
    [
        // Apple's ld: no main; two.
        "\"_main\", referenced from",
        "duplicate symbol '_main'",
        // GNU ld.
        "undefined reference to `main'",
        "multiple definition of `main'",
        // lld.
        "undefined symbol: main",
        "duplicate symbol: main",
    ]
    .iter()
    .any(|says| words.contains(says))
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
/// fewer than 1e9 instructions (an uncounted run counts as under) and under
/// half a second of CPU (an unknown time counts as not under).
fn under_both(r: &Step1Run) -> bool {
    let ins = r
        .instructions
        .is_none_or(|i| (i as f64) < perf_words::FLOOR_INSTRUCTIONS);
    let cpu = r
        .cpu_us
        .is_some_and(|c| (c as f64) < perf_words::FLOOR_CPU_US);
    ins && cpu
}

/// What the floor makes of a row (§3.5 step 2, §7, §9: both legs for
/// too-short and short).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Floor {
    /// Every side under both legs: not timed.
    TooShort,
    /// One side under both legs: measured, marked a short run.
    Short,
    /// Measured in full.
    Full,
}

/// The floor from the C's step-1 run with fewer instructions and, on a
/// side's row, the other side's step-1 run. The C alone has one side: under
/// both legs it is too short, else measured in full — never a short run.
fn floor(c: &Step1Run, other: Option<&Step1Run>) -> Floor {
    match (under_both(c), other.map(under_both)) {
        (true, None | Some(true)) => Floor::TooShort,
        (false, None | Some(false)) => Floor::Full,
        _ => Floor::Short,
    }
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
    // The results first: a units folder that is a link or not a folder is
    // refused before anything is built, read or written (§3.9).
    let mut store = Store::load(perf_dir, workloads)?;
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

    // Each build step in its own sandbox (§3.2 *Build* step 3).
    let plain = Runner {
        cwd: root.clone(),
        allowlist: base.allowlist.clone(),
        timeout: base.timeout,
        max_output: exec::DEFAULT_MAX_OUTPUT,
        tool_profile: None,
        tool_tmpdir: None,
    };
    let steps = Steps {
        runner: &plain,
        host: &host,
        root: &root,
    };
    // The compilers read as perf show reads them (tools.rs): the same runs,
    // output cap and first line. One that cannot be read is stored as its
    // bare name, as ever (row_inputs).
    let probes = Runner {
        max_output: tools::VERSION_OUTPUT_CAP,
        ..steps.writing(&[], &[])?
    };
    let (cc, rustc) = tools::compiler_lines(&probes);
    let (cc, rustc) = (cc.unwrap_or_default(), rustc.unwrap_or_default());

    // The C: objects once, one link.
    progress.message("building the C program…");
    let c_files = program_c_files_in(&base, "perf")?;
    if let Some(odd) = crate::irregular_c_file(&c_files) {
        return Err(Error::Invariant(format!(
            "{} is not a regular file — perf builds the program from the top-level .c files",
            odd.display()
        )));
    }
    let compiler = steps.writing(std::slice::from_ref(&obj_dir), &[])?;
    let objects = match build::compile_objects(&base, &compiler, &c_files, &obj_dir)? {
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
    let c_slot = slot(Slot::C)?;
    let c_bin = match build::link_side(
        &base,
        &link_args,
        &steps.writing(std::slice::from_ref(&c_slot), &[])?,
        &c_slot.join(&name),
        &objects,
        &[],
        false,
    )? {
        Ok(b) => b,
        Err(words) => {
            let hint = if link_says_main(&words) {
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
    let kept_objects = |kept: &[PathBuf]| -> Vec<Hashed> {
        c_files
            .iter()
            .zip(&objects)
            .filter(|(c, _)| kept.contains(c))
            .map(|(_, o)| o.clone())
            .collect()
    };

    // The units: each crate built, checked against its verdict, linked —
    // with --unit alone, only the units asked for (§3.10).
    let measure_units = !req.as_it_stands_only;
    let mut sides: Vec<UnitSide> = Vec::new();
    for s in &selected {
        let Selected::Ready(c) = s else { continue };
        if !req.builds(&c.id) {
            continue;
        }
        progress.message(&format!("{} — building its Rust…", c.id));
        let target_dir = prepare_target_dir(&c.crate_dir)?;
        let cargo = steps.writing(
            std::slice::from_ref(&target_dir),
            &[c.crate_dir.join("Cargo.lock")],
        )?;
        let built = build_staticlib(
            &cargo,
            cargo.tool_profile.as_deref(),
            &c.crate_dir,
            &target_dir,
        );
        if let Err(Error::Interrupted) = built {
            return Err(Error::Interrupted);
        }
        // The crate's digest after the build, whether it built or not: the
        // files verify hashed (cargo may write Cargo.lock), so the unit's
        // rows stay current until the crate itself changes (§3.2, note 24).
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
        let staticlib = match built {
            Ok(lib) => Hashed::new(&lib)?,
            Err(e) => {
                log.push_str(&format!("{} — the crate does not build:\n{e}\n\n", c.id));
                sides.push(UnitSide::SetUp {
                    id: c.id.clone(),
                    outcome: "crate-does-not-build",
                    setup: SetupFacts {
                        log: Some(log_name.clone()),
                        ..SetupFacts::default()
                    },
                    crate_digest,
                });
                continue;
            }
        };
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
        let bytes = std::fs::read(&staticlib.path).map_err(|e| Error::io(&staticlib.path, e))?;
        let facts_of = archive_facts(&bytes).unwrap_or(ArchiveFacts {
            runtime: PanicRuntime::None,
            std: true,
            fat_lto: false,
        });
        let unit_slot = slot(Slot::Unit(c.position))?;
        match build::link_side(
            &base,
            &link_args,
            &steps.writing(std::slice::from_ref(&unit_slot), &[])?,
            &unit_slot.join(&name),
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
    // Each left-out unit with its closed reason for the rows, and in words
    // for the progress lines ("u-tree left out: its crate does not build").
    let mut left_out: Vec<LeftOut> = Vec::new();
    let mut left: Vec<String> = Vec::new();
    for s in &selected {
        if let Selected::NotVerified { id, reason, .. } = s {
            left_out.push(LeftOut {
                id: id.clone(),
                crate_digest: String::new(),
                reason: (*reason).into(),
            });
            left.push(format!("{id} left out: {}", left_out_words(reason)));
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
            let why = match *outcome {
                "not-verified" => setup.reason.as_deref().unwrap_or("not-fresh"),
                o => o,
            };
            let reason = if res::LEFT_OUT_REASONS.contains(&why) {
                why
            } else {
                "not-fresh"
            };
            left_out.push(LeftOut {
                id: id.clone(),
                crate_digest: crate_digest.clone(),
                reason: reason.into(),
            });
            left.push(format!("{id} left out: {}", left_out_words(why)));
        }
    }
    let program = if built.len() < 2 || (!req.units.is_empty() && !req.as_it_stands_only) {
        Program::None
    } else {
        let names: Vec<String> = held.iter().map(|u| u.id.clone()).collect();
        progress.message(&format!(
            "the program as it stands — {}{}",
            names.join(", "),
            if left.is_empty() {
                String::new()
            } else {
                format!(" ({})", left.join("; "))
            }
        ));
        let all_slot = slot(Slot::AsItStands)?;
        as_it_stands(
            &built,
            &held,
            &c_files,
            &base,
            &link_args,
            &steps.writing(std::slice::from_ref(&all_slot), &[])?,
            &all_slot.join(&name),
            &kept_objects,
            &log_name,
            &mut log,
        )?
    };
    if matches!(program, Program::None) && req.units.is_empty() && !req.as_it_stands_only {
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
                    // As every row of it, it records the units the program
                    // holds and those left out (§3.2, §3.9).
                    if let Some(units) = program.units() {
                        let row = set_up_row(
                            w,
                            "input-unusable",
                            setup.clone(),
                            row_inputs(
                                &shared,
                                w,
                                None,
                                RowKind::AsItStands,
                                None,
                                None,
                                Some(units.to_vec()),
                                Some(left_out.clone()),
                            ),
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
        // When the C fails here in step 1 (§3.5 step 1, build note 23), the
        // workload's other rows are not run and keep their earlier rows;
        // rows that run nothing (a unit perf cannot build or measure today)
        // are still written.
        let mut c_failed = false;
        let mut said = false;
        // The C alone (§3.6).
        if req.units.is_empty() && !req.as_it_stands_only {
            progress.message(&format!(
                "the C on {} — checking it ends the same way twice…",
                w.id
            ));
            let (row, failed) =
                c_alone_row(&ctx, &c_bin, w, input, &digest, runs, &shared, progress)?;
            c_failed = failed;
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
                            digest: crate_digest.clone(),
                        }]),
                        None,
                        None,
                        None,
                    ),
                ),
                UnitSide::Built { .. } if c_failed => {
                    say_not_run(w, &mut said, progress);
                    continue;
                }
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
                            c_failed = true;
                            store.put_c_failure(c_row, progress, &mut summary)?;
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
            Program::Built { .. } if c_failed => say_not_run(w, &mut said, progress),
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
                        store.put_c_failure(c_row, progress, &mut summary)?
                    }
                }
            }
        }
    }
    Ok(summary)
}

/// Said once on a workload where the C failed in step 1, when a row there
/// is not run (§3.5 step 1).
fn say_not_run(w: &Workload, said: &mut bool, progress: &mut dyn PerfProgress) {
    if !*said {
        progress.message(&format!(
            "the other rows on {} are not run — the C failed there",
            w.id
        ));
        *said = true;
    }
}

/// Why a unit is left out of the program as it stands, in words for the
/// progress lines (§3.2, §3.10) — from the reason it is left out for, which
/// the results keep as a closed token.
fn left_out_words(reason: &str) -> &'static str {
    match reason {
        "crate-does-not-build" => "its crate does not build",
        "does-not-link" => "its program does not link",
        "replaces-mismatch" => "a replaces entry names no top-level C file — Re-check it",
        "replaces-changed" => "its replaced files changed since verify — Re-check it",
        "rust-changed" => "its Rust changed since verify — Re-check it",
        "accept-interrupted" => "its Accept was interrupted — Re-check it to finish or undo it",
        // not-fresh, and anything else.
        _ => "verify it first",
    }
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
    kept_objects: &dyn Fn(&[PathBuf]) -> Vec<Hashed>,
    log_name: &str,
    log: &mut String,
) -> Result<Program, Error> {
    let mut runtimes = Vec::new();
    let mut found = std::collections::BTreeSet::new();
    let mut replaces: Vec<PathBuf> = Vec::new();
    let mut libs: Vec<Hashed> = Vec::new();
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

/// The C alone on `w` (§3.5 *The C alone*, §3.6), and whether the C failed
/// there in step 1 — one of the C's own outcomes, or a SIGKILL perfrun did
/// not send — so that the workload's other rows are not run. A run the
/// launcher could not measure is this row's only (§3.3 step 2).
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
) -> Result<(Row, bool), Error> {
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
            let unmeasurable = o == "run-failed: unmeasurable";
            if !unmeasurable && o != "c-could-not-start" {
                row.step1 = Some(step1);
            }
            return Ok((row, !unmeasurable));
        }
    }
    if difference(&c1, &c2).is_some() {
        let mut row = bare_row(w, "c-unstable", inputs);
        row.step1 = Some(step1);
        return Ok((row, true));
    }
    note_c_exit(&c1, w, progress);
    if floor(fewer(&step1.c_first, &step1.c_second), None) == Floor::TooShort {
        let mut row = bare_row(w, "too-short", inputs);
        row.step1 = Some(step1);
        return Ok((row, false));
    }
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
                return Ok((failed(w, inputs, "c", i + 1, &other, step1), false));
            }
        }
    }
    let metric = perf_words::choose_metric(
        &c_runs,
        &c_runs,
        platform(&c_runs, shared),
        shared.computer.two_kinds,
    );
    let row = Row {
        workload: w.id.clone(),
        outcome: "baseline".into(),
        // One side: never a short run (§3.5 step 2).
        short: Some(false),
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
    };
    Ok((row, false))
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
        Judged::Unmeasurable(_) | Judged::Overflow { .. } => {
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
    /// The C failed in step 1 — one of the C's own outcomes, or a SIGKILL
    /// perfrun did not send: for the C alone's row only (§3.6, note 23).
    CSide(Row),
}

/// The C's two step-1 runs on a side's row (§3.5 step 1), `None` when both
/// ended alike. A failure of the C is the C's, for the C alone's row
/// ([`SideResult::CSide`]); a run the launcher could not measure is this
/// row's own (§3.3 step 2: "that row only"). The runs are read in order, so
/// a first run's crash is never hidden by a second run the launcher lost.
fn c_in_step1(
    c1: &Judged,
    c2: &Judged,
    step1: &Step1,
    w: &Workload,
    inputs: &RowInputs,
    progress: &mut dyn PerfProgress,
) -> Option<SideResult> {
    let c_inputs = || {
        let mut i = inputs.clone();
        i.crates = None;
        i.replaces = None;
        i.units = None;
        i.left_out = None;
        i.compilers.rustc = None;
        i
    };
    let c_step1 = || Step1 {
        other: None,
        ..step1.clone()
    };
    for j in [c1, c2] {
        let Some(outcome) = c_side(j) else { continue };
        say_unmeasurable(j, "the C", w, progress);
        if let Judged::Unmeasurable(_) = j {
            return Some(SideResult::Row(bare_row(w, outcome, inputs.clone()), None));
        }
        let mut row = bare_row(w, outcome, c_inputs());
        if let Judged::NeverStarted(errno) = j {
            row.setup = Some(never_started(*errno));
        } else {
            row.step1 = Some(c_step1());
        }
        return Some(SideResult::CSide(row));
    }
    if difference(c1, c2).is_some() {
        let mut row = bare_row(w, "c-unstable", c_inputs());
        row.step1 = Some(c_step1());
        return Some(SideResult::CSide(row));
    }
    None
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
    if let Some(result) = c_in_step1(&c1, &c2, &step1, w, &inputs, progress) {
        return Ok(result);
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
        Judged::Overflow { stream } => {
            // Compared on the stream that passed the cap: the C's length
            // there; the other side ends by the SIGKILL that stopped it.
            let mut row = bare_row(w, "behaves-differently", inputs);
            let (c_end, c_len) = match &c1 {
                Judged::Ended {
                    end,
                    stdout,
                    stderr,
                    ..
                } => (
                    end.token(),
                    if *stream == "stderr" {
                        stderr.len()
                    } else {
                        stdout.len()
                    } as u64,
                ),
                other => (other.end_token(), 0),
            };
            row.first_difference = Some(Difference {
                stream: (*stream).into(),
                c_len,
                other_len: OUTPUT_CAP as u64,
                offset: 0,
                c_end,
                other_end: o.end_token(),
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
    let short = match floor(cf, Some(&of)) {
        Floor::TooShort => {
            let mut row = bare_row(w, "too-short", inputs);
            row.step1 = Some(step1);
            return Ok(SideResult::Row(row, None));
        }
        Floor::Short => true,
        Floor::Full => false,
    };
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
        RowSide::Unit(id) => build::sub_folder(
            &build::sub_folder(out_root, harness_core::perf::KEPT_UNITS)?,
            id,
        )?,
        RowSide::Program | RowSide::C => {
            build::sub_folder(out_root, harness_core::perf::KEPT_PROGRAM)?
        }
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

/// `migration/perf/units/` as it stands (§3.9): the results are ledger
/// state, not scratch, so a units folder that is a link or not a folder is
/// refused by name — never read through, never removed (perf's scratch
/// folders, [`build::sub_folder`], are replaced instead). `None` when it is
/// not there yet.
fn units_folder(perf_dir: &Path) -> Result<Option<PathBuf>, Error> {
    let dir = perf_dir.join(res::UNITS_DIR);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_dir() => Ok(Some(dir)),
        Ok(_) => Err(Error::Invariant(format!(
            "{}: must be a directory (a link is refused)",
            dir.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&dir, e)),
    }
}

impl Store {
    /// The results as they stand; a units folder that is a link or not a
    /// folder is refused before anything is read.
    fn load(dir: &Path, workloads: &Workloads) -> Result<Store, Error> {
        units_folder(dir)?;
        let program = res::read_program(&res::program_path(dir))?.unwrap_or_default();
        Ok(Store {
            dir: dir.to_path_buf(),
            program,
            units: std::collections::BTreeMap::new(),
            workloads: workloads.workloads.iter().map(|w| w.id.clone()).collect(),
        })
    }

    /// Write this run's row by the replace rule. The summary counts what
    /// this run found, and the progress sees this run's row — when the rule
    /// kept the earlier row (this try beside it as `last_try`), the
    /// progress line says so; the earlier row is never shown as new.
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
        let this_run = row.clone();
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
                // Checked again before this row's read and write: a link
                // made since the load is refused too, never followed.
                if units_folder(&self.dir)?.is_none() {
                    let dir = self.dir.join(res::UNITS_DIR);
                    std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
                }
                let path = res::unit_path(&self.dir, id);
                if !self.units.contains_key(id) {
                    let loaded =
                        res::read_unit(&path, id)?.unwrap_or_else(|| res::UnitResults::new(id));
                    self.units.insert(id.to_string(), loaded);
                }
                let file = self.units.get_mut(id).expect("loaded");
                let merged = place(&mut file.rows, row, RowKind::Unit);
                keep(&mut file.rows, &self.workloads);
                res::write_unit(&path, file)?;
                merged
            }
        };
        summary.rows += 1;
        match this_run.outcome.as_str() {
            "baseline" | "measured" => summary.measured += 1,
            "too-short" => summary.too_short += 1,
            "behaves-differently" => summary.behaves_differently += 1,
            _ => {}
        }
        // The replace rule keeps an earlier row only beside a set-up or C
        // outcome, whose outcome then differs from this run's.
        if written.outcome == this_run.outcome {
            progress.row(side, &written);
        } else {
            progress.row(side, &this_run);
            progress.message(&format!(
                "{} on {} — the earlier result is kept, with this try beside it",
                side.label(),
                this_run.workload
            ));
        }
        Ok(())
    }

    /// A failure of the C that a unit's or the program's step 1 found, for
    /// the C alone's row (§3.6, note 23): the C's own outcomes go by the
    /// replace rule (beside an earlier baseline as `last_try`). A SIGKILL
    /// perfrun did not send is not one the results keep as `last_try`, so
    /// it never replaces the C alone's earlier baseline: that row stays and
    /// the progress line says so.
    fn put_c_failure(
        &mut self,
        row: Row,
        progress: &mut dyn PerfProgress,
        summary: &mut PerfSummary,
    ) -> Result<(), Error> {
        let over_a_baseline = self
            .program
            .c_alone
            .iter()
            .any(|r| r.workload == row.workload && r.outcome == "baseline");
        if !res::is_c_side(&row.outcome) && over_a_baseline {
            progress.message(&format!(
                "the C on {} was stopped by a SIGKILL perf did not send — its earlier \
                 measurement is kept",
                row.workload
            ));
            return Ok(());
        }
        self.put(RowSide::C, row, progress, summary)
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
    /// The next day the tiny workload's input is gone: the run goes on — the
    /// too-short row stays with this try beside it, the next workload is
    /// measured (§3.7, note 23).
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
        put("bench/tiny.txt", "hi\n");
        std::fs::create_dir_all(root.join("migration/perf")).expect("perf dir");
        let target = TargetContext::load(&root).expect("target");
        let workloads = wl::parse(
            "schema_version = 1\n\
             [[workload]]\nid = \"tiny\"\nargs = [\"{input}\", \"10\"]\ninput = \"bench/tiny.txt\"\nruns = 5\n\
             [[workload]]\nid = \"long\"\nargs = [\"{input}\", \"400000000\"]\ninput = \"bench/in.txt\"\nruns = 5\n\
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
        assert_eq!(long.short, Some(false), "the C alone is never a short run");
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
                input: Some("bench/tiny.txt"),
            },
        );
        assert!(
            words.headline.starts_with("too short to time: the C ran"),
            "{}",
            words.headline
        );

        // The tiny workload's input is removed: the run still ends well.
        std::fs::remove_file(root.join("bench/tiny.txt")).expect("rm");
        let mut second = Seen {
            messages: Vec::new(),
            rows: Vec::new(),
        };
        let summary = perf_run(
            &target,
            &plan,
            &facts,
            &workloads,
            &perf_dir,
            &PerfRequest::default(),
            &mut second,
        )
        .expect("the run goes on past the missing input");
        let shown: Vec<(&str, &str)> = second
            .rows
            .iter()
            .map(|(s, r)| (s.as_str(), r.workload.as_str()))
            .collect();
        assert_eq!(shown, [("c", "tiny"), ("c", "long"), ("c", "gone")]);
        let outcome = |id: &str| {
            second
                .rows
                .iter()
                .find(|(_, r)| r.workload == id)
                .map(|(_, r)| r.outcome.as_str())
        };
        assert_eq!(outcome("tiny"), Some("input-unusable"));
        assert!(
            second.messages.iter().any(|m| m
                == "the C on tiny — the earlier result is kept, with this try beside it"),
            "{:?}",
            second.messages
        );
        // The next workload is still measured.
        assert_eq!(outcome("long"), Some("baseline"), "{:?}", second.messages);
        assert_eq!(summary.measured, 1);
        let program = res::read_program(&res::program_path(&perf_dir))
            .expect("reads")
            .expect("written");
        let tiny = program
            .c_alone
            .iter()
            .find(|r| r.workload == "tiny")
            .expect("row");
        assert_eq!(tiny.outcome, "too-short");
        let last = tiny.last_try.as_ref().expect("this try beside it");
        assert_eq!(
            (
                last.outcome.as_str(),
                last.setup.as_ref().and_then(|s| s.input.as_deref())
            ),
            ("input-unusable", Some("missing"))
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
        assert_eq!(row.short, Some(false), "both sides over the floor: in full");
        assert_eq!(row.other.as_ref().map(Vec::len), Some(5));
        res::check_row(&row, RowKind::Unit).expect("valid");
        // A side under both legs of the floor against a C over them:
        // measured, marked a short run (§3.5 step 2).
        let quick = c_program(
            &root,
            "quick",
            "#include <stdio.h>\nint main(void) { printf(\"same\\n\"); return 0; }\n",
        );
        let SideResult::Row(row, _) = side_row(
            &ctx,
            &c,
            &quick,
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
        assert_eq!(
            (row.outcome.as_str(), row.short),
            ("measured", Some(true)),
            "{row:?}"
        );
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
        let kept_objects = |kept: &[PathBuf]| -> Vec<Hashed> {
            c_files
                .iter()
                .zip(&objects)
                .filter(|(c, _)| kept.contains(c))
                .map(|(_, o)| o.clone())
                .collect()
        };
        let unit = |id: &str, func: &str, value: i32, abort: bool, file: &str| -> UnitSide {
            let crate_dir = crate::testutil::fixture_crate(
                &root,
                id,
                abort,
                &format!("#[no_mangle] pub extern \"C\" fn {func}() -> i32 {{ {value} }}\n"),
            );
            let lib = Hashed::new(&bench.build(&crate_dir)).expect("hash");
            let facts = archive_facts(&std::fs::read(&lib.path).expect("lib")).expect("facts");
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

    /// The floor (§3.5 step 2, §4 *The floor*): a side is under it only
    /// under both legs — fewer than 1e9 instructions and under half a
    /// second of CPU; a row is too short with every side under, a short run
    /// with one, else measured in full.
    #[test]
    fn the_floor_legs() {
        let r = |ins: Option<u64>, cpu: Option<u64>| Step1Run {
            instructions: ins,
            cpu_us: cpu,
            end: "exit 0".into(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        };
        // Each leg on either side of its line (strictly under).
        assert!(under_both(&r(Some(999_999_999), Some(499_999))));
        assert!(!under_both(&r(Some(1_000_000_000), Some(499_999))));
        assert!(!under_both(&r(Some(999_999_999), Some(500_000))));
        // An uncounted run is under the instruction leg; an unknown CPU
        // time is not under the time leg.
        assert!(under_both(&r(None, Some(499_999))));
        assert!(!under_both(&r(Some(1), None)));
        // A memory-bound C at 2.6 s and 0.9e9 instructions is measured in
        // full, not short — alone and against a side just like it.
        let memory_bound = r(Some(900_000_000), Some(2_600_000));
        assert_eq!(floor(&memory_bound, None), Floor::Full);
        assert_eq!(floor(&memory_bound, Some(&memory_bound)), Floor::Full);
        // A CPU-bound run past the instruction leg in under half a second
        // (4.9e9 at 0.34 s, 2e9 at 0.2 s) is measured in full too — and so
        // is an input made 7× bigger as the too-short words advise.
        let fast = r(Some(4_900_000_000), Some(340_000));
        assert_eq!(
            floor(&fast, Some(&r(Some(2_000_000_000), Some(200_000)))),
            Floor::Full
        );
        assert_eq!(
            floor(&r(Some(1_050_000_000), Some(70_000)), None),
            Floor::Full
        );
        // One side under both legs: a short run, whichever side it is.
        let tiny = r(Some(900_000_000), Some(400_000));
        assert_eq!(floor(&tiny, Some(&memory_bound)), Floor::Short);
        assert_eq!(floor(&memory_bound, Some(&tiny)), Floor::Short);
        // Every side under both: too short; the C alone is never short.
        assert_eq!(floor(&tiny, Some(&tiny)), Floor::TooShort);
        assert_eq!(floor(&tiny, None), Floor::TooShort);
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

    // ---- Helpers for the tests below -----------------------------------

    fn seen() -> Seen {
        Seen {
            messages: Vec::new(),
            rows: Vec::new(),
        }
    }

    fn shared() -> Shared {
        Shared {
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
        }
    }

    fn workload(id: &str) -> Workload {
        Workload {
            id: id.into(),
            args: Vec::new(),
            input: None,
            runs: 5,
        }
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
        std::fs::write(&p, text).expect("write");
    }

    /// A launcher built into a private cache under `root`.
    fn test_launcher(root: &Path) -> Launcher {
        let cache = root.join("cache");
        let owner = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(root).expect("meta"));
        std::fs::create_dir(&cache).expect("cache");
        std::fs::set_permissions(&cache, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("mode");
        launcher::launcher_in(&cache, owner, &mut |_| {}).expect("launcher")
    }

    /// A C program `name` in `root/bins` from `src`, hashed.
    fn c_program(root: &Path, name: &str, src: &str) -> Hashed {
        let bins = root.join("bins");
        std::fs::create_dir_all(&bins).expect("bins");
        let file = bins.join(format!("{name}.c"));
        std::fs::write(&file, src).expect("src");
        let bin = bins.join(name);
        assert!(std::process::Command::new("cc")
            .args(["-O0", "-o"])
            .arg(&bin)
            .arg(&file)
            .status()
            .expect("cc")
            .success());
        Hashed::new(&bin.canonicalize().expect("bin")).expect("hash")
    }

    fn timed(n: usize) -> Vec<Run> {
        (0..n)
            .map(|_| Run {
                cpu_us: Some(600_000),
                end: "exit 0".into(),
                ..Run::default()
            })
            .collect()
    }

    fn step(end: &str) -> Step1Run {
        Step1Run {
            instructions: None,
            cpu_us: None,
            end: end.into(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        }
    }

    /// The tokens of why `row` is out of date, against today's tree.
    #[allow(clippy::too_many_arguments)]
    fn out_of_date(
        root: &Path,
        target: &TargetContext,
        facts: &Facts,
        row: &Row,
        kind: RowKind,
        workload: Option<&str>,
        replaces: Option<&[String]>,
        measurable: &[String],
    ) -> Vec<&'static str> {
        let crate_now = |id: &str| -> Option<String> {
            harness_core::hash::unit_crate_file_set_hash(
                root,
                &root.join(format!("migration/units/{id}/{id}_rs")),
            )
            .ok()
        };
        let program = harness_core::features::program_digest_now(target, facts);
        let name = harness_core::features::program_name(&target.config);
        let today = harness_core::perf::currency::Today {
            workload,
            program: &program,
            crate_digest: &crate_now,
            replaces,
            program_name: &name,
            measurable: Some(measurable),
            computer: None,
            compilers: None,
        };
        harness_core::perf::currency::reasons(row, kind, &today)
            .into_iter()
            .map(|r| r.token)
            .collect()
    }

    /// A small target for perf: `main.c` adds what `int <id>(void)` in each
    /// `src/<id>.c` returns (1, 2, …), aborts on "crash" and starts `true`
    /// on "sys".
    fn mini_program(root: &Path, ids: &[&str]) {
        put(
            root,
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"tool\"\nsource_dir = \"src\"\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\ntimeout_secs = 60\n",
        );
        let decls: String = ids.iter().map(|id| format!("int {id}(void);\n")).collect();
        let sum: Vec<String> = ids.iter().map(|id| format!("{id}()")).collect();
        put(
            root,
            "src/main.c",
            &format!(
                "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n{decls}\
                 int main(int argc, char **argv) {{\n\
                 if (argc > 1 && strcmp(argv[1], \"crash\") == 0) abort();\n\
                 if (argc > 1 && strcmp(argv[1], \"sys\") == 0) return system(\"true\");\n\
                 printf(\"%d\\n\", {});\n return 0;\n}}\n",
                sum.join(" + ")
            ),
        );
        for (i, id) in ids.iter().enumerate() {
            put(
                root,
                &format!("src/{id}.c"),
                &format!("int {id}(void) {{ return {}; }}\n", i + 1),
            );
        }
    }

    /// The facts a scan records for the top-level C files under `src/`.
    fn facts_of(root: &Path) -> Facts {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(root.join("src")).expect("src") {
            let p = entry.expect("entry").path();
            if p.extension().is_some_and(|x| x == "c") {
                let name = p.file_name().and_then(|n| n.to_str()).expect("name");
                files.push(harness_core::facts::FileRecord {
                    path: format!("src/{name}"),
                    hash: harness_core::hash::file_hash(&p).expect("hash"),
                    includes: Vec::new(),
                });
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Facts {
            files,
            ..Facts::default()
        }
    }

    /// A unit's crate as verify leaves it, `migration/units/<id>/<id>_rs`:
    /// a staticlib exporting `<id>()` (returning `value`), panic = "abort",
    /// and — with `lock` — its Cargo.lock as cargo writes it.
    fn unit_crate(root: &Path, id: &str, value: usize, build_rs: Option<&str>, lock: bool) {
        let dir = root.join(format!("migration/units/{id}/{id}_rs"));
        put(
            &dir,
            "Cargo.toml",
            &format!(
                "[package]\nname = \"{id}_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n\
                 [profile.release]\npanic = \"abort\"\n"
            ),
        );
        put(
            &dir,
            "src/lib.rs",
            &format!("#[no_mangle]\npub extern \"C\" fn {id}() -> i32 {{\n    {value}\n}}\n"),
        );
        if let Some(text) = build_rs {
            put(&dir, "build.rs", text);
        }
        if lock {
            assert!(std::process::Command::new("cargo")
                .args(["generate-lockfile", "--offline", "--manifest-path"])
                .arg(dir.join("Cargo.toml"))
                .status()
                .expect("cargo")
                .success());
        }
    }

    /// One unit's plan entry, replacing `src/<id>.c` with its crate; for a
    /// verified or merged unit with a crate, its verdict too — green or red,
    /// its inputs hashed from the tree now, as verify writes them.
    fn plan_unit(
        root: &Path,
        facts: &Facts,
        id: &str,
        status: &str,
        green: bool,
        verdict_replaces: Option<&str>,
    ) -> String {
        let file = format!("src/{id}.c");
        let files = vec![file.clone()];
        let source =
            harness_core::hash::file_set_hash_on_disk(root, &facts.include_closure(&files))
                .expect("hash");
        let ledger = Ledger::new(root.to_path_buf());
        let crate_dir = ledger.unit_dir(id).join(format!("{id}_rs"));
        if crate_dir.is_dir() && matches!(status, "verified" | "merged") {
            let inputs = harness_core::verdict::VerdictInputs {
                unit_source: source.clone(),
                rust_crate: harness_core::hash::unit_crate_file_set_hash(root, &crate_dir)
                    .expect("crate hash"),
                replaces: vec![verdict_replaces.unwrap_or(&file).to_string()],
                ..Default::default()
            };
            let checks = vec![harness_core::verdict::Check {
                name: "differential".into(),
                passed: green,
                detail: String::new(),
            }];
            harness_core::Verdict::new(id, inputs, checks)
                .store(&ledger.verdict_latest_path(id))
                .expect("verdict");
        }
        format!(
            "\n[[unit]]\nid = \"{id}\"\nstatus = \"{status}\"\nfiles = [\"{file}\"]\n\
             source_hash = \"{source}\"\n\n[unit.oracle]\nkind = \"c-abi-differential\"\n\
             rust_crate = \"{id}_rs\"\nreplaces = [\"{file}\"]\n"
        )
    }

    /// A fresh folder beside the test binary — outside the temp folders
    /// every tool profile may write, so a build step's own write set is
    /// what decides — removed when dropped.
    struct Beside(PathBuf);

    impl Beside {
        fn new(tag: &str) -> Beside {
            let exe = std::env::current_exe().expect("test exe");
            let dir = exe.parent().expect("exe folder").join(format!(
                "perf-{tag}-{}-{}",
                std::process::id(),
                harness_core::hash::random_hex(4)
            ));
            std::fs::create_dir_all(&dir).expect("dir");
            Beside(dir.canonicalize().expect("canonical"))
        }

        /// Whether the folder lies in a temp folder after all (a target dir
        /// there): every tool may write it, so no write set is tested.
        fn in_temp(&self) -> bool {
            let tmp = std::env::temp_dir().canonicalize().ok();
            self.0.starts_with("/private/tmp")
                || self.0.starts_with("/private/var/folders")
                || tmp.is_some_and(|t| self.0.starts_with(t))
        }
    }

    impl Drop for Beside {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ---- Tests ------------------------------------------------------------

    /// perf's own selection (§3.2, note 24): a verified or merged unit is
    /// measurable only when its verdict is green and fresh, no Accept of it
    /// was interrupted and its replaced files are the ones verify saw; any
    /// other verified unit says why; pending, blocked and in-progress units
    /// are never named.
    #[test]
    fn the_selection_reads_each_rule() {
        let tmp = crate::testutil::TempDir::new("perf-select");
        let root = tmp.path().to_path_buf();
        let ids = ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];
        mini_program(&root, &ids);
        for (i, id) in ids.iter().enumerate().take(6) {
            unit_crate(&root, id, i + 1, None, false);
        }
        let facts = facts_of(&root);
        let mut plan = "schema_version = 1\ntarget = \"tool\"\n".to_string();
        plan += &plan_unit(&root, &facts, "s1", "verified", true, None);
        plan += &plan_unit(&root, &facts, "s2", "verified", true, None);
        plan += &plan_unit(&root, &facts, "s3", "merged", true, None);
        plan += &plan_unit(&root, &facts, "s4", "verified", false, None);
        plan += &plan_unit(&root, &facts, "s5", "verified", true, None);
        plan += &plan_unit(&root, &facts, "s6", "verified", true, Some("src/s1.c"));
        plan += &plan_unit(&root, &facts, "s7", "pending", false, None);
        plan += &plan_unit(&root, &facts, "s8", "blocked", false, None);
        plan += &plan_unit(&root, &facts, "s9", "in-progress", false, None);
        put(&root, "migration/plan.toml", &plan);
        // s2: an Accept of a-1234 interrupted; s3: one from before the
        // markers (a bare `.<crate>.prev`); s5: its C changed since verify.
        std::fs::create_dir_all(root.join("migration/units/s2/.promote-a-1234")).expect("marker");
        std::fs::create_dir_all(root.join("migration/units/s3/.s3_rs.prev")).expect("legacy");
        put(&root, "src/s5.c", "int s5(void) { return 50; }\n");
        let plan = Plan::load(&root.join("migration/plan.toml")).expect("plan");
        let target = TargetContext::load(&root).expect("target");
        let ledger = Ledger::new(root.clone());
        let got: Vec<(String, String)> = select(&target, &ledger, &facts, &plan)
            .expect("selects")
            .into_iter()
            .map(|s| match s {
                Selected::Ready(c) => {
                    assert_eq!(
                        c.verdict_crate,
                        harness_core::hash::unit_crate_file_set_hash(&root, &c.crate_dir)
                            .expect("hash")
                    );
                    (c.id, "ready".to_string())
                }
                Selected::NotVerified {
                    id,
                    reason,
                    attempt,
                } => (
                    id,
                    format!("{reason} {}", attempt.as_deref().unwrap_or("-")),
                ),
            })
            .collect();
        let want = [
            ("s1", "ready"),
            ("s2", "accept-interrupted a-1234"),
            ("s3", "accept-interrupted legacy"),
            ("s4", "not-fresh -"),
            ("s5", "not-fresh -"),
            ("s6", "replaces-changed -"),
        ];
        assert_eq!(
            got,
            want.map(|(a, b)| (a.to_string(), b.to_string())).to_vec()
        );
        assert_eq!(
            perf_measurable(&target, &plan, &facts).expect("reads"),
            vec!["s1".to_string()]
        );
    }

    /// A request builds what it measures (§3.10, §6 *Cost*): `--unit` alone
    /// builds and links only the units asked for; a full run and the
    /// program as it stands build every measurable unit.
    #[test]
    fn a_request_builds_only_the_units_it_measures() {
        let full = PerfRequest::default();
        assert!(full.builds("u001") && full.builds("u002"));
        let one = PerfRequest {
            units: vec!["u001".into()],
            ..PerfRequest::default()
        };
        assert!(one.builds("u001"));
        assert!(!one.builds("u002"));
        let together = PerfRequest {
            units: vec!["u001".into()],
            as_it_stands_only: true,
            ..PerfRequest::default()
        };
        assert!(together.builds("u002"));
    }

    /// A run's summary and its progress tell what this run found (§3.10):
    /// a set-up outcome kept beside an earlier measured row is not counted
    /// as measured, and the progress shows this run's row and says the
    /// earlier one is kept. On the C alone, a SIGKILL a unit's step 1 met
    /// never replaces the baseline; the C's own outcomes go beside it.
    #[test]
    fn the_summary_and_progress_tell_this_runs_rows() {
        let tmp = crate::testutil::TempDir::new("perf-store");
        let dir = tmp.path().join("perf");
        std::fs::create_dir_all(&dir).expect("dir");
        let workloads = wl::parse(
            "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = []\n",
            Path::new("w.toml"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let w = workloads.workloads[0].clone();
        let shared = shared();
        let unit_inputs = row_inputs(
            &shared,
            &w,
            None,
            RowKind::Unit,
            Some(vec![CrateDigest {
                id: "u001".into(),
                digest: harness_core::hash::bytes_hash(b"crate"),
            }]),
            Some(vec!["src/a.c".into()]),
            None,
            None,
        );
        let mut store = Store::load(&dir, &workloads).expect("loads");
        let mut seen = seen();
        let mut summary = PerfSummary::default();
        let measured = Row {
            short: Some(false),
            runs: Some(5),
            platform_metrics: Some("cpu-time".into()),
            c: Some(timed(5)),
            other: Some(timed(5)),
            ..bare_row(&w, "measured", unit_inputs.clone())
        };
        store
            .put(RowSide::Unit("u001"), measured, &mut seen, &mut summary)
            .expect("written");
        assert_eq!((summary.rows, summary.measured), (1, 1));
        // The input is gone the next time.
        let mut summary = PerfSummary::default();
        let gone = set_up_row(
            &w,
            "input-unusable",
            SetupFacts {
                input: Some("missing".into()),
                ..SetupFacts::default()
            },
            unit_inputs,
        );
        store
            .put(RowSide::Unit("u001"), gone, &mut seen, &mut summary)
            .expect("written");
        assert_eq!((summary.rows, summary.measured), (1, 0), "nothing measured");
        let (side, shown) = seen.rows.last().expect("a row");
        assert_eq!(
            (side.as_str(), shown.outcome.as_str()),
            ("u001", "input-unusable")
        );
        assert_eq!(
            seen.messages.last().map(String::as_str),
            Some("u001 on w — the earlier result is kept, with this try beside it")
        );
        let file = res::read_unit(&res::unit_path(&dir, "u001"), "u001")
            .expect("reads")
            .expect("written");
        assert_eq!(file.rows[0].outcome, "measured");
        assert_eq!(
            file.rows[0].last_try.as_ref().map(|t| t.outcome.as_str()),
            Some("input-unusable")
        );
        // The C alone's baseline.
        let c_inputs = row_inputs(&shared, &w, None, RowKind::CAlone, None, None, None, None);
        let baseline = Row {
            short: Some(false),
            runs: Some(5),
            platform_metrics: Some("cpu-time".into()),
            c: Some(timed(5)),
            ..bare_row(&w, "baseline", c_inputs.clone())
        };
        store
            .put(RowSide::C, baseline, &mut seen, &mut summary)
            .expect("written");
        let c_row = |outcome: &str, first: &str| {
            let mut row = bare_row(&w, outcome, c_inputs.clone());
            row.step1 = Some(Step1 {
                c_first: step(first),
                other: None,
                c_second: step("exit 0"),
            });
            row
        };
        // A SIGKILL a unit's step 1 met: the baseline stays as it was.
        let rows_before = seen.rows.len();
        store
            .put_c_failure(
                c_row("stopped-by-sigkill", "signal 9"),
                &mut seen,
                &mut summary,
            )
            .expect("kept");
        assert_eq!(seen.rows.len(), rows_before, "no row written");
        assert_eq!(
            seen.messages.last().map(String::as_str),
            Some(
                "the C on w was stopped by a SIGKILL perf did not send — its earlier \
                 measurement is kept"
            )
        );
        let program = res::read_program(&res::program_path(&dir))
            .expect("reads")
            .expect("written");
        assert_eq!(program.c_alone[0].outcome, "baseline");
        assert!(program.c_alone[0].last_try.is_none());
        // A crash: beside the baseline, shown as this run's.
        store
            .put_c_failure(c_row("c-crashed", "signal 11"), &mut seen, &mut summary)
            .expect("kept");
        let program = res::read_program(&res::program_path(&dir))
            .expect("reads")
            .expect("written");
        assert_eq!(program.c_alone[0].outcome, "baseline");
        assert_eq!(
            program.c_alone[0]
                .last_try
                .as_ref()
                .map(|t| t.outcome.as_str()),
            Some("c-crashed")
        );
        assert_eq!(
            seen.rows
                .last()
                .map(|(s, r)| (s.as_str(), r.outcome.as_str())),
            Some(("c", "c-crashed"))
        );
    }

    /// The results are ledger state (§3.9): a units folder that is a link or
    /// not a folder is refused by name — when the run starts, before
    /// anything is built, and again before each unit row's read and write —
    /// never read through (another folder's file would be this target's
    /// earlier rows) and never removed; the link's target is left as it
    /// was.
    #[test]
    fn a_linked_units_folder_is_refused_never_read_or_removed() {
        use std::os::unix::fs::symlink;
        let refused = |r: &Result<(), Error>| match r {
            Err(Error::Invariant(w)) => {
                w.ends_with("migration/perf/units: must be a directory (a link is refused)")
            }
            _ => false,
        };
        let tmp = crate::testutil::TempDir::new("perf-units-link");
        let root = tmp.path().to_path_buf();
        let perf_dir = root.join("migration/perf");
        std::fs::create_dir_all(&perf_dir).expect("perf dir");
        let workloads = wl::parse(
            "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = []\n",
            Path::new("w.toml"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let w = workloads.workloads[0].clone();
        let unit_inputs = row_inputs(
            &shared(),
            &w,
            None,
            RowKind::Unit,
            Some(vec![CrateDigest {
                id: "u001".into(),
                digest: harness_core::hash::bytes_hash(b"crate"),
            }]),
            Some(vec!["src/a.c".into()]),
            None,
            None,
        );
        let measured = Row {
            short: Some(false),
            runs: Some(5),
            platform_metrics: Some("cpu-time".into()),
            c: Some(timed(5)),
            other: Some(timed(5)),
            ..bare_row(&w, "measured", unit_inputs.clone())
        };
        // Another folder holding a unit file with a row of its own.
        let outside = root.join("outside");
        std::fs::create_dir(&outside).expect("outside");
        let mut theirs = res::UnitResults::new("u001");
        theirs.rows.push(measured.clone());
        res::write_unit(&outside.join("u001.json"), &theirs).expect("their file");
        let their_bytes = std::fs::read(outside.join("u001.json")).expect("bytes");
        let untouched = |link: &Path| {
            let m = std::fs::symlink_metadata(link).expect("still there");
            assert!(m.file_type().is_symlink(), "the link was removed");
            assert_eq!(std::fs::read_link(link).expect("link"), outside);
            let names: Vec<_> = std::fs::read_dir(&outside)
                .expect("outside")
                .map(|e| e.expect("entry").file_name())
                .collect();
            assert_eq!(names, ["u001.json"]);
            assert_eq!(
                std::fs::read(outside.join("u001.json")).expect("bytes"),
                their_bytes
            );
        };
        let units = perf_dir.join(res::UNITS_DIR);

        // Linked when the run starts: refused before anything is read.
        symlink(&outside, &units).expect("link");
        let loaded = Store::load(&perf_dir, &workloads).map(|_| ());
        assert!(refused(&loaded), "{loaded:?}");
        untouched(&units);
        assert!(!res::program_path(&perf_dir).exists());

        // Linked after the load: the unit row's write refuses it before
        // reading; no row is shown or counted.
        std::fs::remove_file(&units).expect("unlink");
        let mut store = Store::load(&perf_dir, &workloads).expect("loads");
        symlink(&outside, &units).expect("link");
        let mut seen = seen();
        let mut summary = PerfSummary::default();
        let put = store.put(RowSide::Unit("u001"), measured, &mut seen, &mut summary);
        assert!(refused(&put), "{put:?}");
        untouched(&units);
        assert!(seen.rows.is_empty() && summary.rows == 0);

        // A file in its place: refused the same way, the file left as it was.
        std::fs::remove_file(&units).expect("unlink");
        std::fs::write(&units, b"not a folder").expect("file");
        let loaded = Store::load(&perf_dir, &workloads).map(|_| ());
        assert!(refused(&loaded), "{loaded:?}");
        assert_eq!(std::fs::read(&units).expect("file"), b"not a folder");

        // The run refuses it before building anything.
        if cfg!(target_os = "macos") && sandbox::sandbox_mode() == "sandbox-exec" {
            std::fs::remove_file(&units).expect("rm");
            symlink(&outside, &units).expect("link");
            mini_program(&root, &["ua"]);
            let target = TargetContext::load(&root).expect("target");
            let plan = Plan {
                schema_version: 1,
                target: "tool".into(),
                units: Vec::new(),
            };
            let run = perf_run(
                &target,
                &plan,
                &facts_of(&root),
                &workloads,
                &perf_dir,
                &PerfRequest::default(),
                &mut seen,
            )
            .map(|_| ());
            assert!(refused(&run), "{run:?}");
            untouched(&units);
            assert!(!root.join("migration/build").exists(), "nothing built");
        }
    }

    /// The C's step-1 runs on a side's row (§3.3, §3.5 step 1, note 23): a
    /// run the launcher lost is the side's own row ("that row only"), with
    /// the side's inputs; a crash — in either run — or a SIGKILL perfrun did
    /// not send is the C's, for the C alone's row, the SIGKILL worded
    /// without "crash"; the first run that failed decides.
    #[test]
    fn the_cs_step1_failures_go_to_the_right_row() {
        let w = workload("w");
        let shared = shared();
        let inputs = row_inputs(
            &shared,
            &w,
            None,
            RowKind::Unit,
            Some(vec![CrateDigest {
                id: "u001".into(),
                digest: harness_core::hash::bytes_hash(b"crate"),
            }]),
            Some(vec!["src/a.c".into()]),
            None,
            None,
        );
        let ended = |end: End| Judged::Ended {
            end,
            stdout: b"same\n".to_vec(),
            stderr: Vec::new(),
            run: Box::default(),
        };
        let ok = ended(End::Exit(0));
        let crash = ended(End::Signal(11));
        let lost = Judged::Unmeasurable("perfrun ended before its record was complete".into());
        let step1 = |c1: &Judged, c2: &Judged| Step1 {
            c_first: step1_run(c1),
            other: Some(step1_run(&ok)),
            c_second: step1_run(c2),
        };
        let mut seen = seen();
        // The launcher lost the C's first run: the unit's own row.
        let Some(SideResult::Row(row, None)) =
            c_in_step1(&lost, &ok, &step1(&lost, &ok), &w, &inputs, &mut seen)
        else {
            panic!("the unit's own row")
        };
        assert_eq!(row.outcome, "run-failed: unmeasurable");
        assert_eq!(
            row.inputs.crates.as_ref().map(|c| c[0].id.as_str()),
            Some("u001")
        );
        res::check_row(&row, RowKind::Unit).expect("valid on the unit's row");
        assert!(
            seen.messages
                .iter()
                .any(|m| m.starts_with("the C on w: the launcher stopped before measuring")),
            "{:?}",
            seen.messages
        );
        // A crash in the first run outranks a second run the launcher lost.
        let Some(SideResult::CSide(row)) =
            c_in_step1(&crash, &lost, &step1(&crash, &lost), &w, &inputs, &mut seen)
        else {
            panic!("the C's row")
        };
        assert_eq!(row.outcome, "c-crashed");
        res::check_row(&row, RowKind::CAlone).expect("valid on the C alone");
        // Only the second run crashed: still the C's.
        let Some(SideResult::CSide(row)) =
            c_in_step1(&ok, &crash, &step1(&ok, &crash), &w, &inputs, &mut seen)
        else {
            panic!("the C's row")
        };
        assert_eq!(row.outcome, "c-crashed");
        // A SIGKILL perfrun did not send: stopped-by-sigkill on the C alone.
        let killed = Judged::SigKilled;
        let Some(SideResult::CSide(row)) =
            c_in_step1(&killed, &ok, &step1(&killed, &ok), &w, &inputs, &mut seen)
        else {
            panic!("the C's row")
        };
        assert_eq!(row.outcome, "stopped-by-sigkill");
        res::check_row(&row, RowKind::CAlone).expect("valid on the C alone");
        let words = perf_words::words(
            &row,
            &perf_words::Context {
                side: perf_words::Side::C,
                workload: "w",
                input: None,
            },
        );
        assert!(
            words.headline.starts_with("the C was stopped by a SIGKILL"),
            "{}",
            words.headline
        );
        assert!(!words.headline.contains("crash"), "{}", words.headline);
        // The C alone's own step 1 says the same.
        assert_eq!(c_side(&Judged::SigKilled), Some("stopped-by-sigkill"));
        // Both ended alike: nothing of the C's.
        assert!(c_in_step1(&ok, &ok, &step1(&ok, &ok), &w, &inputs, &mut seen).is_none());
    }

    /// "The program needs one main()" only when the link is about main
    /// (§3.2 *Build* step 2) — the texts are Apple ld's, GNU ld's and lld's.
    #[test]
    fn the_main_hint_only_when_the_link_is_about_main() {
        let no_main = "cc failed (exit status: 1):\nUndefined symbols for architecture arm64:\n  \
                       \"_main\", referenced from:\n      <initial-undefines>\n\
                       ld: symbol(s) not found for architecture arm64\n";
        let two = "cc failed (exit status: 1):\nduplicate symbol '_main' in:\n    \
                   /t/migration/build/.perf/obj/002-m2.o\n    /t/migration/build/.perf/obj/001-m1.o\n\
                   ld: 1 duplicate symbols\n";
        let called_from_main = "cc failed (exit status: 1):\nUndefined symbols for architecture \
                                arm64:\n  \"_compressBound\", referenced from:\n      _main in \
                                003-main.o\nld: symbol(s) not found for architecture arm64\n";
        let main_folder = "cc failed (exit status: 1):\nduplicate symbol '_helper' in:\n    \
                           /t/domain/main/001-a.o\n    /t/domain/main/002-b.o\n";
        assert!(link_says_main(no_main));
        assert!(link_says_main(two));
        assert!(link_says_main(
            "/usr/bin/ld: crt1.o: in function `_start':\n(.text+0x1b): undefined reference to `main'\n"
        ));
        assert!(link_says_main(
            "/usr/bin/ld: b.o: in function `main':\nmultiple definition of `main'; a.o: first defined here\n"
        ));
        assert!(link_says_main("ld.lld: error: undefined symbol: main\n"));
        assert!(!link_says_main(called_from_main));
        assert!(!link_says_main(main_folder));
        // The line that says so is among the lines shown.
        assert!(first_lines(no_main, 3).contains("\"_main\", referenced from"));
        assert!(first_lines(two, 3).contains("duplicate symbol '_main'"));
    }

    /// Every reason a unit is left out has words for the progress lines
    /// (§3.2, §3.10) — never a closed token.
    #[test]
    fn every_left_out_reason_has_words() {
        let tokens: Vec<&str> = res::LEFT_OUT_REASONS
            .iter()
            .chain(res::NOT_VERIFIED_REASONS)
            .copied()
            .collect();
        for reason in &tokens {
            let words = left_out_words(reason);
            assert!(
                !words.is_empty() && !tokens.iter().any(|t| words.contains(t)),
                "{reason}: {words}"
            );
        }
        assert_eq!(left_out_words("not-fresh"), "verify it first");
        assert_eq!(
            left_out_words("crate-does-not-build"),
            "its crate does not build"
        );
    }

    /// Step 1 has a minute more than the target's timeout (§3.3 step 6):
    /// every new binary's first exec falls there.
    #[test]
    fn step_one_has_a_minute_more() {
        assert_eq!(deadline_secs(5, true), 65);
        assert_eq!(deadline_secs(5, false), 5);
    }

    /// A fresh binary with a short `timeout_secs` is not timed out in step 1
    /// (§4 [m17, n4]); a timed run gets the target's timeout alone.
    #[test]
    fn a_fresh_binary_is_not_timed_out_in_step_one() {
        if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-deadline");
        let root = tmp.path().canonicalize().expect("root");
        let l = test_launcher(&root);
        let host = HostDirs::from_env().expect("host");
        let slow = c_program(
            &root,
            "slow",
            "#include <stdio.h>\n#include <unistd.h>\nint main(void) { sleep(2); printf(\"slept\\n\"); return 0; }\n",
        );
        let ctx = RunCtx {
            launcher: &l,
            host: &host,
            root: &root,
            name: "tool",
            timeout_secs: 1,
        };
        let w = workload("w");
        let first = run_once(&ctx, &slow, &w, None, true).expect("runs");
        assert!(
            matches!(&first, Judged::Ended { end: End::Exit(0), stdout, .. } if stdout == b"slept\n"),
            "{first:?}"
        );
        let timed = run_once(&ctx, &slow, &w, None, false).expect("runs");
        assert!(matches!(timed, Judged::TimedOut), "{timed:?}");
    }

    /// Output over the cap on the other side only (§3.3 step 1, §3.9): the
    /// row compares the stream that passed the cap — stderr here, where the
    /// C prints 3 MiB and its stdout matches — and stores the end perf saw
    /// (the SIGKILL that stopped the run), never an exit it did not see.
    #[test]
    fn output_over_the_cap_is_compared_on_its_own_stream() {
        assert_eq!(overflowed_stream(OUTPUT_CAP - 100, 5), "stdout");
        assert_eq!(overflowed_stream(5, OUTPUT_CAP - 100), "stderr");
        if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-cap");
        let root = tmp.path().canonicalize().expect("root");
        let l = test_launcher(&root);
        let host = HostDirs::from_env().expect("host");
        let program = |name: &str, blocks: usize| {
            c_program(
                &root,
                name,
                &format!(
                    "#include <stdio.h>\n#include <string.h>\nint main(void) {{ static char b[65536]; \
                     memset(b, 'x', sizeof b); printf(\"same\\n\"); fflush(stdout); \
                     for (int i = 0; i < {blocks}; i++) fwrite(b, 1, sizeof b, stderr); return 0; }}\n"
                ),
            )
        };
        let c = program("c", 48);
        let flood = program("flood", 1200);
        let ctx = RunCtx {
            launcher: &l,
            host: &host,
            root: &root,
            name: "tool",
            timeout_secs: 60,
        };
        let w = workload("w");
        let shared = shared();
        let inputs = row_inputs(&shared, &w, None, RowKind::Unit, None, None, None, None);
        let mut seen = seen();
        let SideResult::Row(row, None) = side_row(
            &ctx, &c, &flood, &w, None, 5, inputs, "u001", &shared, &mut seen,
        )
        .expect("runs") else {
            panic!("the unit's row")
        };
        assert_eq!(row.outcome, "behaves-differently");
        let d = row.first_difference.as_ref().expect("difference");
        assert!(d.over_cap);
        assert_eq!((d.stream.as_str(), d.c_len), ("stderr", 48 * 65536));
        let other_end = row
            .step1
            .as_ref()
            .and_then(|s| s.other.as_ref())
            .map(|o| o.end.clone());
        assert_eq!(other_end.as_deref(), Some("signal 9"));
        assert_eq!(Some(d.other_end.clone()), other_end, "the row agrees");
        res::check_row(&row, RowKind::Unit).expect("valid");
    }

    /// The program as it stands says why it does not link (§3.2, note 15,
    /// §4): a no-std unit beside a std unit, in both orders — and never a
    /// mixed-panic finding for a unit with no runtime found; a fat-LTO unit
    /// beside a std unit, in both orders; otherwise "unknown", naming every
    /// unit, with the linker's lines in the log.
    #[test]
    fn the_program_as_it_stands_says_why_it_does_not_link() {
        let bench = crate::testutil::ToolBench::new("perf-causes");
        let root = bench.root().canonicalize().expect("root");
        put(
            &root,
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"tool\"\nsource_dir = \"src\"\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        );
        put(&root, "src/main.c", "#include <stdio.h>\nint a(void); int b(void);\nint main(void) { printf(\"%d\\n\", a() + b()); return 0; }\n");
        put(&root, "src/a.c", "int a(void) { return 1; }\n");
        put(&root, "src/b.c", "int b(void) { return 2; }\n");
        let target = harness_core::TargetContext::load(&root).expect("target");
        let base = Base::resolve(&target, "perf", &["cc"]).expect("base");
        let scratch = build::perf_scratch(&root).expect("scratch");
        let obj = build::sub_folder(&scratch, "obj").expect("obj");
        let c_files = program_c_files_in(&base, "perf").expect("files");
        let objects = build::compile_objects(&base, bench.runner(), &c_files, &obj)
            .expect("runs")
            .expect("compiles");
        let kept_objects = |kept: &[PathBuf]| -> Vec<Hashed> {
            c_files
                .iter()
                .zip(&objects)
                .filter(|(c, _)| kept.contains(c))
                .map(|(_, o)| o.clone())
                .collect()
        };
        // A unit's crate: its own release profile and lib.rs; it replaces
        // `file`.
        let unit = |id: &str, profile: &str, lib_rs: &str, file: &str| -> UnitSide {
            let dir = root.join("fixtures").join(id);
            put(
                &dir,
                "Cargo.toml",
                &format!(
                    "[package]\nname = \"{id}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                     [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\n{profile}"
                ),
            );
            put(&dir, "src/lib.rs", lib_rs);
            let lib = Hashed::new(&bench.build(&dir.canonicalize().expect("dir"))).expect("hash");
            let facts = archive_facts(&std::fs::read(&lib.path).expect("lib")).expect("facts");
            UnitSide::Built {
                candidate: Candidate {
                    id: id.into(),
                    position: 1,
                    replaces_rel: vec![format!("src/{file}")],
                    replaces: vec![root.join("src").join(file).canonicalize().expect("c")],
                    crate_dir: dir,
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
        // A std unit's body pulls std in (a Vec, an index that may panic).
        let uses_std = |func: &str| {
            format!(
                "#[no_mangle]\npub extern \"C\" fn {func}() -> i32 {{\n    \
                 let v: Vec<i32> = std::hint::black_box(vec![10]);\n    v[0]\n}}\n"
            )
        };
        let std_a = unit("stda", "panic = \"abort\"\n", &uses_std("a"), "a.c");
        let unwind_a = unit("unwinda", "", &uses_std("a"), "a.c");
        let lto_b = unit(
            "ltob",
            "panic = \"abort\"\nlto = true\n",
            &uses_std("b"),
            "b.c",
        );
        let nostd_b = unit(
            "nostdb",
            "panic = \"abort\"\n",
            "#![no_std]\n#[panic_handler]\nfn on_panic(_: &core::panic::PanicInfo) -> ! {\n    loop {}\n}\n\
             #[no_mangle]\npub extern \"C\" fn b() -> i32 {\n    20\n}\n",
            "b.c",
        );
        let not_a = unit(
            "nota",
            "panic = \"abort\"\n",
            "#[no_mangle]\npub extern \"C\" fn not_a() -> i32 {\n    10\n}\n",
            "a.c",
        );
        let out = build::sub_folder(&scratch, "bin/pall")
            .expect("slot")
            .join("tool");
        let link = |sides: &[&UnitSide]| -> (Program, String) {
            let held: Vec<UnitRef> = sides
                .iter()
                .map(|s| UnitRef {
                    id: s.id().into(),
                    crate_digest: harness_core::hash::bytes_hash(s.id().as_bytes()),
                })
                .collect();
            let mut log = String::new();
            let p = as_it_stands(
                sides,
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
            (p, log)
        };
        let cause = |sides: &[&UnitSide]| -> (String, Vec<String>, String) {
            match link(sides) {
                (
                    Program::SetUp {
                        outcome: "does-not-link",
                        setup,
                        ..
                    },
                    log,
                ) => (
                    setup.cause.unwrap_or_default(),
                    setup.units.unwrap_or_default(),
                    log,
                ),
                (Program::SetUp { outcome, .. }, log) => panic!("{outcome}: {log}"),
                (_, log) => panic!("it linked: {log}"),
            }
        };
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // A no-std unit beside a std one, in both orders.
        let (c, u, _) = cause(&[&std_a, &nostd_b]);
        assert_eq!((c.as_str(), u), ("no-std", ids(&["nostdb"])));
        let (c, u, _) = cause(&[&nostd_b, &std_a]);
        assert_eq!((c.as_str(), u), ("no-std", ids(&["nostdb"])));
        // Beside an unwinding unit too: no runtime found is never part of a
        // mixed-panic finding.
        let (c, _, _) = cause(&[&unwind_a, &nostd_b]);
        assert_eq!(c, "no-std");
        // A fat-LTO unit beside a std unit, in both orders.
        let (c, u, _) = cause(&[&std_a, &lto_b]);
        assert_eq!((c.as_str(), u), ("lto", ids(&["ltob"])));
        let (c, u, _) = cause(&[&lto_b, &std_a]);
        assert_eq!((c.as_str(), u), ("lto", ids(&["ltob"])));
        // Otherwise unknown, naming every unit, the linker's lines kept.
        let std_b = unit("stdb", "panic = \"abort\"\n", &uses_std("b"), "b.c");
        let (c, u, log) = cause(&[&not_a, &std_b]);
        assert_eq!((c.as_str(), u), ("unknown", ids(&["nota", "stdb"])));
        assert!(log.contains("_a"), "{log}");
    }

    /// `perf run` end to end on a small target of five verified units and a
    /// pending one (§3.2, §3.5, §3.10): no build step writes outside its own
    /// folder; the progress names the left-out units in words; when the C
    /// fails on a workload in step 1 its other rows are not run; a C that
    /// starts another program is stopped by a SIGKILL perf did not send,
    /// never "crashes"; a crate that does not build keeps its real digest,
    /// so its rows stay current; one changed by its build is caught after
    /// it; an as-it-stands row on a missing input records its units; and
    /// `--unit` builds only the units asked for.
    #[test]
    fn a_run_over_verified_units_end_to_end() {
        if !cfg!(target_os = "macos") || sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let dir = Beside::new("e2e");
        let root = dir.0.clone();
        let ids = ["ua", "ub", "uc", "ud", "ue"];
        mini_program(&root, &ids);
        // ud's build script tries to write the C's objects and ua's
        // staticlib folder, then fails; ue has no Cargo.lock yet, so its
        // build writes one into the files verify hashed.
        let obj_probe = root.join("migration/build/.perf/obj/written-by-a-build-script");
        let lib_probe = root.join("migration/units/ua/ua_rs/target/written-by-a-build-script");
        let build_rs = format!(
            "fn main() {{\n    let _ = std::fs::write({obj_probe:?}, b\"x\");\n    \
             let _ = std::fs::write({lib_probe:?}, b\"x\");\n    \
             panic!(\"this crate does not build\");\n}}\n"
        );
        for (i, id) in ids.iter().enumerate() {
            let script = (*id == "ud").then_some(build_rs.as_str());
            unit_crate(&root, id, i + 1, script, *id != "ue");
        }
        let facts = facts_of(&root);
        let mut plan = "schema_version = 1\ntarget = \"tool\"\n".to_string();
        for id in ids {
            plan += &plan_unit(&root, &facts, id, "verified", true, None);
        }
        plan += "\n[[unit]]\nid = \"up\"\nstatus = \"pending\"\n";
        put(&root, "migration/plan.toml", &plan);
        let plan = Plan::load(&root.join("migration/plan.toml")).expect("plan");
        let target = TargetContext::load(&root).expect("target");
        let perf_dir = root.join("migration/perf");
        std::fs::create_dir_all(&perf_dir).expect("perf dir");
        let workloads = wl::parse(
            "schema_version = 1\n\
             [[workload]]\nid = \"tiny\"\nargs = []\nruns = 5\n\
             [[workload]]\nid = \"crash\"\nargs = [\"crash\"]\nruns = 5\n\
             [[workload]]\nid = \"sys\"\nargs = [\"sys\"]\nruns = 5\n\
             [[workload]]\nid = \"gone\"\nargs = [\"{input}\"]\ninput = \"bench/gone.txt\"\nruns = 5\n",
            Path::new("w.toml"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let run = |req: PerfRequest, seen: &mut Seen| {
            perf_run(&target, &plan, &facts, &workloads, &perf_dir, &req, seen)
        };

        // An unknown id is refused naming the verified units — never the
        // pending one.
        let refused = run(
            PerfRequest {
                units: vec!["nope".into()],
                ..PerfRequest::default()
            },
            &mut seen(),
        );
        let Err(Error::InvalidPlan(words)) = &refused else {
            panic!("{refused:?}")
        };
        assert!(
            words.ends_with("the verified units are ua, ub, uc, ud, ue"),
            "{words}"
        );

        // A full run.
        let mut first = seen();
        let summary = run(PerfRequest::default(), &mut first).expect("perf runs");
        let says = |s: &Seen, text: &str| s.messages.iter().any(|m| m == text);
        let rows_on = |s: &Seen, w: &str| -> Vec<(String, String)> {
            s.rows
                .iter()
                .filter(|(_, r)| r.workload == w)
                .map(|(side, r)| (side.clone(), r.outcome.clone()))
                .collect()
        };
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        // Each build step wrote only its own folder (§3.2 *Build* step 3).
        if !dir.in_temp() {
            assert!(!obj_probe.exists(), "a build script wrote the C's objects");
            assert!(
                !lib_probe.exists(),
                "a build script wrote another unit's target"
            );
        }
        // The pending unit is never measured, recorded or named.
        assert!(!first.rows.iter().any(|(side, _)| side == "up"));
        assert!(!first.messages.iter().any(|m| m
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == "up")));
        assert!(!res::unit_path(&perf_dir, "up").exists());
        // The left-out units, in words.
        assert!(
            says(
                &first,
                "the program as it stands — ua, ub, uc (ud left out: its crate does not build; \
                 ue left out: its Rust changed since verify — Re-check it)"
            ),
            "{:#?}",
            first.messages
        );
        // The C crashes on "crash": shown once, under the C; the units and
        // the program as it stands are not run there, and the units perf
        // cannot build still say why.
        assert_eq!(
            rows_on(&first, "crash"),
            pairs(&[
                ("c", "c-crashed"),
                ("ud", "crate-does-not-build"),
                ("ue", "not-verified")
            ])
        );
        assert!(says(
            &first,
            "the other rows on crash are not run — the C failed there"
        ));
        assert!(!first
            .messages
            .iter()
            .any(|m| m.starts_with("ua on crash")
                || m.starts_with("the program as it stands on crash")));
        // A C that starts another program is stopped by a SIGKILL perfrun
        // did not send: worded without asserting why, never as a crash.
        assert_eq!(
            rows_on(&first, "sys"),
            pairs(&[
                ("c", "stopped-by-sigkill"),
                ("ud", "crate-does-not-build"),
                ("ue", "not-verified")
            ])
        );
        let program = res::read_program(&res::program_path(&perf_dir))
            .expect("reads")
            .expect("written");
        let sys = program
            .c_alone
            .iter()
            .find(|r| r.workload == "sys")
            .expect("row");
        assert!(sys.step1.is_some());
        let words = perf_words::words(
            sys,
            &perf_words::Context {
                side: perf_words::Side::C,
                workload: "sys",
                input: None,
            },
        );
        assert!(!words.headline.contains("crash"), "{}", words.headline);
        assert_eq!(summary.measured, 0);
        // Today: ue's build wrote its Cargo.lock, so it is not fresh now.
        let measurable = perf_measurable(&target, &plan, &facts).expect("reads");
        assert_eq!(measurable, ["ua", "ub", "uc", "ud"]);
        let reasons = |row: &Row, kind: RowKind, w: Option<&str>, replaces: Option<&[String]>| {
            out_of_date(&root, &target, &facts, row, kind, w, replaces, &measurable)
        };
        // A missing input: the program as it stands still records the units
        // it holds and those left out, so only its input reads out of date.
        let gone = program
            .as_it_stands
            .iter()
            .find(|r| r.workload == "gone")
            .expect("row");
        assert_eq!(gone.outcome, "input-unusable");
        let held: Vec<&str> = gone
            .inputs
            .units
            .iter()
            .flatten()
            .map(|u| u.id.as_str())
            .collect();
        assert_eq!(held, ["ua", "ub", "uc"]);
        assert_eq!(
            reasons(gone, RowKind::AsItStands, None, None),
            ["workload-gone"]
        );
        // A crate that does not build keeps its real digest: its row and the
        // program's left-out entry stay current (§3.2, note 24).
        let tiny = wl::digest(&workloads.workloads[0], None);
        let unit_row = |id: &str, w: &str| -> Row {
            res::read_unit(&res::unit_path(&perf_dir, id), id)
                .expect("reads")
                .expect("written")
                .rows
                .into_iter()
                .find(|r| r.workload == w)
                .expect("row")
        };
        // The compilers this run stored are the lines perf show reads on the
        // same target (tools.rs), so unchanged compilers compare equal.
        let ua = unit_row("ua", "tiny").inputs.compilers;
        assert_eq!(
            tools::perf_compilers(&target, false),
            Some((ua.cc, ua.rustc.expect("a unit row names its rustc")))
        );
        let ud = unit_row("ud", "tiny");
        assert_eq!(ud.outcome, "crate-does-not-build");
        let ud_replaces = vec!["src/ud.c".to_string()];
        assert!(
            reasons(&ud, RowKind::Unit, Some(&tiny), Some(&ud_replaces)).is_empty(),
            "{ud:?}"
        );
        let all = program
            .as_it_stands
            .iter()
            .find(|r| r.workload == "tiny")
            .expect("row");
        assert!(
            reasons(all, RowKind::AsItStands, Some(&tiny), None).is_empty(),
            "{all:?}"
        );
        // ue's crate changed in its build: caught after it (§3.2).
        let ue = unit_row("ue", "tiny");
        assert_eq!(
            (
                ue.outcome.as_str(),
                ue.setup.as_ref().and_then(|s| s.reason.as_deref())
            ),
            ("not-verified", Some("rust-changed"))
        );

        // --unit ua --unit ub on "crash": only those two are built and
        // linked; ua's step 1 finds the C crashing — written on the C
        // alone's row only — and ub is not run there.
        let ua_before = std::fs::read(res::unit_path(&perf_dir, "ua")).expect("ua");
        let ub_before = std::fs::read(res::unit_path(&perf_dir, "ub")).expect("ub");
        let mut second = seen();
        run(
            PerfRequest {
                units: vec!["ua".into(), "ub".into()],
                workloads: vec!["crash".into()],
                ..PerfRequest::default()
            },
            &mut second,
        )
        .expect("perf runs");
        assert!(
            says(&second, "ua — building its Rust…") && says(&second, "ub — building its Rust…")
        );
        assert!(
            !second
                .messages
                .iter()
                .any(|m| ["uc —", "ud —", "ue —"].iter().any(|p| m.starts_with(p))),
            "{:#?}",
            second.messages
        );
        let bins = root.join("migration/build/.perf/bin");
        assert!(bins.join("p001").is_dir() && bins.join("p002").is_dir());
        assert!(!bins.join("p003").exists() && !bins.join("pall").exists());
        assert_eq!(rows_on(&second, "crash"), pairs(&[("c", "c-crashed")]));
        assert!(says(&second, "ua on crash — C, ua, C…"));
        assert!(!second.messages.iter().any(|m| m.starts_with("ub on crash")));
        assert!(says(
            &second,
            "the other rows on crash are not run — the C failed there"
        ));
        assert_eq!(
            std::fs::read(res::unit_path(&perf_dir, "ua")).expect("ua"),
            ua_before
        );
        assert_eq!(
            std::fs::read(res::unit_path(&perf_dir, "ub")).expect("ub"),
            ub_before
        );
    }
}
