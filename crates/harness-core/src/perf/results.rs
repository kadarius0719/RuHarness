//! perf's results (docs/PERF-DESIGN.md §3.7, §3.9): `migration/perf/
//! program.json` (the C alone and the program as it stands) and
//! `migration/perf/units/<id>.json`, schema `ruharness-perf` v1 — numbers
//! and closed values only, never words (the words are rebuilt on every
//! read), read strictly by outcome, and the one rule for what replaces a
//! row. A results file is evidence, not ledger truth: it gates nothing and
//! can be forged.

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The results files' schema name.
pub const RESULTS_SCHEMA: &str = "ruharness-perf";
/// The results files' schema version this build reads.
pub const RESULTS_SCHEMA_VERSION: u64 = 1;
/// The program's results file, in `migration/perf/`.
pub const PROGRAM_FILE: &str = "program.json";
/// The folder of the units' results files, in `migration/perf/`.
pub const UNITS_DIR: &str = "units";
/// Largest results file read.
pub const MAX_RESULTS_BYTES: u64 = 4 * 1024 * 1024;
/// Longest free text a row may hold (a CPU's name, a compiler's line, a
/// log's name).
pub const MAX_TEXT: usize = 160;
/// The output cap: the most of one stream perf keeps and compares (§3.3,
/// §3.9). A difference's lengths and its kept files are never longer.
pub const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
/// A replaces-mismatch's index is below this: far past any unit's
/// `replaces`, and small enough that the words can count from it.
pub const MAX_REPLACES_INDEX: u32 = 65_536;
/// Most paths a unit row's `replaces` holds: as many as an index below
/// [`MAX_REPLACES_INDEX`] can name, and no more.
pub const MAX_REPLACES: usize = MAX_REPLACES_INDEX as usize;
/// Most entries one list of units in a row holds (`crates`, `units`,
/// `left_out`, a set-up's `runtimes` and `units`, a last try's `units`):
/// perf measures a plan of at most 999 units, one slot each (`p001`…`p999`,
/// §3.2).
pub const MAX_UNITS: usize = 999;
/// Most files a difference keeps: each side's stdout and stderr.
pub const MAX_KEPT: usize = 4;

/// `program.json` in the resolved `migration/perf/` folder.
pub fn program_path(perf_dir: &Path) -> PathBuf {
    perf_dir.join(PROGRAM_FILE)
}

/// `units/<id>.json` in the resolved `migration/perf/` folder.
pub fn unit_path(perf_dir: &Path, unit: &str) -> PathBuf {
    perf_dir.join(UNITS_DIR).join(format!("{unit}.json"))
}

/// The closed set of outcomes (§3.7, note 23).
pub const OUTCOMES: &[&str] = &[
    // Measured.
    "baseline",
    "measured",
    // What the code does.
    "behaves-differently",
    "stopped-by-sigkill",
    "too-short",
    "run-failed: timeout",
    "run-failed: exit",
    "run-failed: signal",
    // The C's own (only on the C-alone rows).
    "c-unstable",
    "c-crashed",
    "c-timed-out",
    "output-too-large",
    "c-could-not-start",
    // What the set-up could not do.
    "not-verified",
    "replaces-mismatch",
    "crate-does-not-build",
    "does-not-link",
    "mixed-panic",
    "input-unusable",
    "could-not-start",
    "run-failed: unmeasurable",
];

/// The C's own outcomes, written only to a workload's C-alone row.
pub const C_SIDE: &[&str] = &[
    "c-unstable",
    "c-crashed",
    "c-timed-out",
    "output-too-large",
    "c-could-not-start",
];

/// What the set-up could not do: these never replace an earlier row that
/// is not one of them (§3.7).
pub const SET_UP: &[&str] = &[
    "not-verified",
    "replaces-mismatch",
    "crate-does-not-build",
    "does-not-link",
    "mixed-panic",
    "input-unusable",
    "could-not-start",
    "run-failed: unmeasurable",
];

/// The platform's metric a measured row used (§3.9).
pub const PLATFORM_METRICS: &[&str] = &[
    "macos-v6-pnorm",
    "macos-v6-cycles",
    "macos-v6-cycles-phases",
    "macos-v6-share",
    "macos-v4-cycles",
    "linux-cycles",
    "linux-hybrid-summed",
    "cpu-time",
];

/// Why a unit was left out of the program as it stands (§3.2, note 24).
pub const LEFT_OUT_REASONS: &[&str] = &[
    "not-fresh",
    "replaces-mismatch",
    "replaces-changed",
    "crate-does-not-build",
    "does-not-link",
    "accept-interrupted",
];

/// Why a unit is `not-verified` (§3.2).
pub const NOT_VERIFIED_REASONS: &[&str] = &[
    "not-fresh",
    "replaces-changed",
    "rust-changed",
    "accept-interrupted",
];

/// A does-not-link cause (§3.2, note 15).
pub const LINK_CAUSES: &[&str] = &["no-std", "two-no-std", "lto", "two-lto", "unknown"];

/// A panic runtime read from a unit's archive.
pub const RUNTIMES: &[&str] = &["abort", "unwind", "none"];

/// The `[profile.release]` keys a crate's manifest may set away from the
/// defaults, named on its row (note 25).
pub const PROFILE_KEYS: &[&str] = &["opt-level", "lto", "codegen-units", "panic"];

/// The C-alone and as-it-stands rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramResults {
    /// [`RESULTS_SCHEMA`].
    pub schema: String,
    /// [`RESULTS_SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The C alone, one row per workload.
    pub c_alone: Vec<Row>,
    /// The program as it stands, one row per workload.
    pub as_it_stands: Vec<Row>,
}

impl Default for ProgramResults {
    fn default() -> Self {
        ProgramResults {
            schema: RESULTS_SCHEMA.into(),
            schema_version: RESULTS_SCHEMA_VERSION,
            c_alone: Vec::new(),
            as_it_stands: Vec::new(),
        }
    }
}

/// One unit's rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitResults {
    /// [`RESULTS_SCHEMA`].
    pub schema: String,
    /// [`RESULTS_SCHEMA_VERSION`].
    pub schema_version: u64,
    /// The unit's id.
    pub unit: String,
    /// One row per workload.
    pub rows: Vec<Row>,
}

impl UnitResults {
    /// An empty file for `unit`.
    pub fn new(unit: &str) -> UnitResults {
        UnitResults {
            schema: RESULTS_SCHEMA.into(),
            schema_version: RESULTS_SCHEMA_VERSION,
            unit: unit.into(),
            rows: Vec::new(),
        }
    }
}

/// One side measured on one workload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// The workload's id.
    pub workload: String,
    /// One of [`OUTCOMES`].
    pub outcome: String,
    /// A short run (baseline and measured rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<bool>,
    /// Runs a side, 5–31 (baseline and measured rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<u32>,
    /// One of [`PLATFORM_METRICS`] (baseline and measured rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_metrics: Option<String>,
    /// What the row was measured from.
    pub inputs: RowInputs,
    /// The C's timed runs (baseline and measured rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c: Option<Vec<Run>>,
    /// The other side's timed runs (measured rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Vec<Run>>,
    /// The other side's archives carry std (unit and as-it-stands rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub std: Option<bool>,
    /// A unit's archive shows fat LTO.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_lto: Option<bool>,
    /// The crate's manifest's `[profile.release]` keys away from the
    /// defaults (note 25).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<ProfileSetting>>,
    /// What step 1 measured, on other outcomes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step1: Option<Step1>,
    /// The timed run that ended differently (`run-failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_run: Option<FailedRun>,
    /// The set-up's facts, closed values only, so the words rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupFacts>,
    /// Where the outputs first differ (behaves-differently).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_difference: Option<Difference>,
    /// A behaves-differently finding a later re-measure did not clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_before: Option<Difference>,
    /// A later set-up (or C-side) outcome kept beside this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_try: Option<LastTry>,
}

/// What a row was measured from: each its own reason when it changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowInputs {
    /// The workload's digest.
    pub workload: String,
    /// The C program's digest.
    pub program: String,
    /// The crates measured, with their digests (unit and as-it-stands
    /// rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crates: Option<Vec<CrateDigest>>,
    /// The unit's `replaces` (unit rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<String>>,
    /// The program's name.
    pub program_name: String,
    /// The units the program as it stands holds, in plan order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<UnitRef>>,
    /// The verified units it left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_out: Option<Vec<LeftOut>>,
    /// [`super::PERF_RECIPE`] when measured.
    pub recipe: String,
    /// [`super::PERF_LAUNCHER`] when measured.
    pub launcher: String,
    /// The computer.
    pub computer: Computer,
    /// The compilers.
    pub compilers: Compilers,
}

/// A crate and its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateDigest {
    /// The unit's id.
    pub id: String,
    /// `unit_crate_file_set_hash` of its crate.
    pub digest: String,
}

/// A unit the program as it stands holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitRef {
    /// The unit's id.
    pub id: String,
    /// Its crate's digest.
    #[serde(rename = "crate")]
    pub crate_digest: String,
}

/// A verified unit left out of the program as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeftOut {
    /// The unit's id.
    pub id: String,
    /// Its crate's digest, when it had one.
    #[serde(rename = "crate")]
    pub crate_digest: String,
    /// One of [`LEFT_OUT_REASONS`].
    pub reason: String,
}

/// The computer a row was measured on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Computer {
    /// The OS's product version.
    pub os: String,
    /// The OS's build.
    pub build: String,
    /// The arch.
    pub arch: String,
    /// The CPU's name.
    pub cpu: String,
    /// Two kinds of cores.
    pub two_kinds: bool,
    /// How many fast (performance) cores.
    pub fast_cores: u32,
}

/// The compilers' first lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compilers {
    /// `cc --version`'s first line.
    pub cc: String,
    /// `rustc -V` (absent on the C-alone rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rustc: Option<String>,
}

/// One timed run of one side.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Instructions (≥ 1 when present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<u64>,
    /// Cycles (≥ 1 when present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles: Option<u64>,
    /// CPU time, user + system, in microseconds (≥ 1 when present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_us: Option<u64>,
    /// Clock time from go to the end, in microseconds (≥ 1 when present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_us: Option<u64>,
    /// The memory footprint's lifetime maximum, in bytes (≥ 1 when
    /// present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<u64>,
    /// Performance-core instructions (with `p_cycles`, both or neither).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_instructions: Option<u64>,
    /// Performance-core cycles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_cycles: Option<u64>,
    /// Voluntary context switches (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switches_voluntary: Option<u64>,
    /// Involuntary context switches (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switches_involuntary: Option<u64>,
    /// The 1-minute load average, in hundredths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<u32>,
    /// `exit N` (0–255) or `signal N`.
    pub end: String,
}

/// What step 1 measured, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step1 {
    /// The C's first run.
    pub c_first: Step1Run,
    /// The other side's run (absent on the C-alone rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Step1Run>,
    /// The C's second run.
    pub c_second: Step1Run,
}

/// One step-1 run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step1Run {
    /// Instructions, when counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<u64>,
    /// CPU time in microseconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_us: Option<u64>,
    /// How it ended: `exit N`, `signal N`, `timeout` or `never-started`.
    pub end: String,
    /// Bytes on stdout.
    pub stdout_bytes: u64,
    /// Bytes on stderr.
    pub stderr_bytes: u64,
}

/// The timed run that ended differently from its side's step-1 runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedRun {
    /// `c` or `other`.
    pub side: String,
    /// 1-based index of the run on its side.
    pub index: u32,
    /// `exit N`, `signal N` or `timeout`.
    pub end: String,
}

/// A crate manifest's `[profile.release]` setting away from the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSetting {
    /// One of [`PROFILE_KEYS`].
    pub key: String,
    /// The value as written (bounded, shown safely).
    pub value: String,
}

/// The set-up's facts: closed values, numbers and ids.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupFacts {
    /// mixed-panic: each unit's runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtimes: Option<Vec<UnitRuntime>>,
    /// does-not-link: one of [`LINK_CAUSES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The units a cause names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<String>>,
    /// replaces-mismatch: the entry's index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// The run's log under `migration/build/perf-logs/` (not committed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// input-unusable: one of [`super::workloads::InputUnusable`]'s tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// not-verified: one of [`NOT_VERIFIED_REASONS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// accept-interrupted: the attempt (or `legacy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<String>,
    /// could-not-start / c-could-not-start: the exec's errno, or
    /// `no-ready`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub never_started: Option<String>,
}

/// A unit's panic runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitRuntime {
    /// The unit's id.
    pub id: String,
    /// One of [`RUNTIMES`].
    pub runtime: String,
}

/// Where two sides' outputs first differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Difference {
    /// `stdout`, `stderr` or `exit`.
    pub stream: String,
    /// The C's length of that stream.
    pub c_len: u64,
    /// The other side's length.
    pub other_len: u64,
    /// The first differing byte (0-based).
    pub offset: u64,
    /// How the C ended.
    pub c_end: String,
    /// How the other side ended.
    pub other_end: String,
    /// The other side wrote over the output cap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub over_cap: bool,
    /// The kept outputs, each with its size and blake3.
    #[serde(default)]
    pub kept: Vec<KeptFile>,
}

/// A kept output file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeptFile {
    /// `<workload>.{c,other}.{stdout,stderr}`.
    pub name: String,
    /// Its size.
    pub size: u64,
    /// Its blake3.
    pub blake3: String,
}

/// A later set-up or C-side outcome kept beside a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastTry {
    /// The later outcome.
    pub outcome: String,
    /// Its set-up facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupFacts>,
    /// The units it held (as-it-stands rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Vec<UnitRef>>,
}

/// Which list a row is in: its rules differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// `program.json`'s `c_alone`.
    CAlone,
    /// `program.json`'s `as_it_stands`.
    AsItStands,
    /// A unit's file.
    Unit,
}

fn bad(path: &Path, why: String) -> Error {
    Error::InvalidPlan(format!("{}: {why}", path.display()))
}

/// Read and check `program.json`; `Ok(None)` when absent. A link, a file
/// over 4 MiB, a wrong shape or a broken rule is an error naming the file.
pub fn read_program(path: &Path) -> Result<Option<ProgramResults>, Error> {
    let Some(bytes) = read(path)? else {
        return Ok(None);
    };
    let value: ProgramResults = parse_json(path, &bytes)?;
    check_header(path, &value.schema, value.schema_version)?;
    check_rows(path, &value.c_alone, RowKind::CAlone)?;
    check_rows(path, &value.as_it_stands, RowKind::AsItStands)?;
    Ok(Some(value))
}

/// Read and check a unit's results file; `Ok(None)` when absent. Its
/// `unit` must be `unit`.
pub fn read_unit(path: &Path, unit: &str) -> Result<Option<UnitResults>, Error> {
    let Some(bytes) = read(path)? else {
        return Ok(None);
    };
    let value: UnitResults = parse_json(path, &bytes)?;
    check_header(path, &value.schema, value.schema_version)?;
    if value.unit != unit {
        return Err(bad(
            path,
            format!("unit {:?} where {unit:?} was expected", value.unit),
        ));
    }
    check_rows(path, &value.rows, RowKind::Unit)?;
    Ok(Some(value))
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
        Ok(_) => crate::ledger::read_regular(path, MAX_RESULTS_BYTES).map(Some),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(path: &Path, bytes: &[u8]) -> Result<T, Error> {
    // The version first: a newer file is its own error, whatever its shape.
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if let Some(found) = map.get("schema_version").and_then(|v| v.as_u64()) {
            if found > RESULTS_SCHEMA_VERSION {
                return Err(Error::SchemaTooNew {
                    path: path.to_path_buf(),
                    found,
                    supported: RESULTS_SCHEMA_VERSION,
                });
            }
        }
    }
    serde_json::from_slice(bytes).map_err(|e| bad(path, format!("not a perf results file: {e}")))
}

fn check_header(path: &Path, schema: &str, version: u64) -> Result<(), Error> {
    if schema != RESULTS_SCHEMA {
        return Err(bad(
            path,
            format!("schema {schema:?}, not {RESULTS_SCHEMA:?}"),
        ));
    }
    if version != RESULTS_SCHEMA_VERSION {
        return Err(bad(path, format!("schema_version {version}")));
    }
    Ok(())
}

/// Write `program.json` atomically (the caller resolved the folder and
/// refused links).
pub fn write_program(path: &Path, value: &ProgramResults) -> Result<(), Error> {
    check_rows(path, &value.c_alone, RowKind::CAlone)?;
    check_rows(path, &value.as_it_stands, RowKind::AsItStands)?;
    write(path, value)
}

/// Write a unit's results file atomically.
pub fn write_unit(path: &Path, value: &UnitResults) -> Result<(), Error> {
    check_rows(path, &value.rows, RowKind::Unit)?;
    write(path, value)
}

fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|e| Error::Invariant(format!("perf results: {e}")))?;
    text.push('\n');
    crate::ledger::write_atomic(path, text.as_bytes())
}

/// Whether `outcome` is a measured one.
pub fn is_measured(outcome: &str) -> bool {
    matches!(outcome, "baseline" | "measured")
}

/// Whether `outcome` is the C's own.
pub fn is_c_side(outcome: &str) -> bool {
    C_SIDE.contains(&outcome)
}

/// Whether `outcome` is a set-up one.
pub fn is_set_up(outcome: &str) -> bool {
    SET_UP.contains(&outcome)
}

fn check_rows(path: &Path, rows: &[Row], kind: RowKind) -> Result<(), Error> {
    let mut seen: Vec<&str> = Vec::new();
    for row in rows {
        if seen.contains(&row.workload.as_str()) {
            return Err(bad(
                path,
                format!("workload {:?} has two rows", row.workload),
            ));
        }
        seen.push(&row.workload);
        check_row(row, kind).map_err(|why| bad(path, format!("row {:?}: {why}", row.workload)))?;
    }
    Ok(())
}

/// Check one row's rules (§3.9); the error says which.
pub fn check_row(row: &Row, kind: RowKind) -> Result<(), String> {
    if !super::workloads::is_id(&row.workload) {
        return Err("the workload id is not one".into());
    }
    let o = row.outcome.as_str();
    if !OUTCOMES.contains(&o) {
        return Err(format!("unknown outcome {o:?}"));
    }
    match kind {
        RowKind::CAlone => {
            if o == "measured"
                || matches!(
                    o,
                    "not-verified"
                        | "replaces-mismatch"
                        | "crate-does-not-build"
                        | "does-not-link"
                        | "mixed-panic"
                        | "could-not-start"
                        | "behaves-differently"
                )
            {
                return Err(format!("{o} is not a C-alone outcome"));
            }
        }
        RowKind::AsItStands | RowKind::Unit => {
            if o == "baseline" || is_c_side(o) {
                return Err(format!("{o} is only the C alone's"));
            }
        }
    }
    check_inputs(&row.inputs, kind)?;
    let measured = is_measured(o);
    if measured {
        let runs = row.runs.ok_or("a measured row needs runs")?;
        if !(5..=31).contains(&runs) {
            return Err(format!("runs {runs}: 5 to 31"));
        }
        if row.short.is_none() {
            return Err("a measured row needs short".into());
        }
        match row.platform_metrics.as_deref() {
            Some(m) if PLATFORM_METRICS.contains(&m) => {}
            _ => return Err("a measured row needs platform_metrics".into()),
        }
        let c = row.c.as_ref().ok_or("a measured row needs the C's runs")?;
        check_runs(c, runs)?;
        match (o, &row.other) {
            ("baseline", None) => {}
            ("baseline", Some(_)) => return Err("a baseline has no other side".into()),
            (_, Some(other)) => check_runs(other, runs)?,
            (_, None) => return Err("a measured row needs the other side's runs".into()),
        }
        if row.step1.is_some() || row.failed_run.is_some() || row.setup.is_some() {
            return Err("a measured row holds no step-1, failed-run or set-up facts".into());
        }
    } else {
        if row.c.is_some() || row.other.is_some() || row.runs.is_some() || row.short.is_some() {
            return Err(format!("{o} holds no timed runs"));
        }
        if row.platform_metrics.is_some() {
            return Err(format!("{o} holds no platform_metrics"));
        }
    }
    if let Some(s) = &row.step1 {
        if is_set_up(o) && o != "could-not-start" {
            return Err(format!("{o} holds no step-1 facts"));
        }
        for run in [Some(&s.c_first), s.other.as_ref(), Some(&s.c_second)]
            .into_iter()
            .flatten()
        {
            check_end(&run.end, true)?;
        }
        if kind == RowKind::CAlone && s.other.is_some() {
            return Err("the C alone has no other side".into());
        }
    }
    if let Some(f) = &row.failed_run {
        if !o.starts_with("run-failed: ") || o == "run-failed: unmeasurable" {
            return Err("only a run-failed row names its failed run".into());
        }
        if !matches!(f.side.as_str(), "c" | "other") || f.index == 0 || f.index > 31 {
            return Err("failed_run's side is c or other, its index 1 to 31".into());
        }
        check_end(&f.end, true)?;
    }
    check_setup(o, row.setup.as_ref())?;
    if let (Some(i), Some(r)) = (
        row.setup.as_ref().and_then(|s| s.index),
        &row.inputs.replaces,
    ) {
        if i as usize >= r.len() {
            return Err("replaces-mismatch's index is past the unit's replaces".into());
        }
    }
    match (o, &row.first_difference) {
        ("behaves-differently", Some(d)) => check_difference(d)?,
        ("behaves-differently", None) => {
            return Err("behaves-differently needs first_difference".into())
        }
        (_, Some(_)) => return Err("only behaves-differently has a first_difference".into()),
        _ => {}
    }
    if let Some(d) = &row.found_before {
        if matches!(o, "behaves-differently") || kind == RowKind::CAlone {
            return Err("found_before is kept only beside another row".into());
        }
        check_difference(d)?;
    }
    if let Some(t) = &row.last_try {
        // The replace rule's own rows (§3.7, note 23): a set-up outcome is
        // kept beside any row that is not itself a set-up row — a too-short
        // or C-side row on the C alone among them; the C's own outcome is
        // kept beside a C-alone row only where it does not replace it, so
        // never beside a too-short or C-side row.
        let c_try = kind == RowKind::CAlone && is_c_side(&t.outcome);
        let ok = is_set_up(&t.outcome) || c_try;
        if !ok || is_set_up(o) || (c_try && (is_c_side(o) || o == "too-short")) {
            return Err(
                "last_try is a later set-up (or the C's) outcome beside an earlier row".into(),
            );
        }
        if let Some(u) = &t.units {
            at_most(u, MAX_UNITS, "last_try's units")?;
        }
        check_setup(&t.outcome, t.setup.as_ref())?;
    }
    if let Some(p) = &row.profile {
        at_most(p, PROFILE_KEYS.len(), "profile")?;
        for s in p {
            if !PROFILE_KEYS.contains(&s.key.as_str()) || !text_ok(&s.value, 32) {
                return Err("profile settings are opt-level, lto, codegen-units and panic".into());
            }
        }
    }
    Ok(())
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(crate::text::unsafe_to_show)
}

/// A list a row holds is no longer than any run writes: checked before its
/// entries are walked, so a forged file's long list is refused at once and
/// never handed on (its words, the once-each checks).
fn at_most<T>(list: &[T], max: usize, what: &str) -> Result<(), String> {
    if list.len() > max {
        return Err(format!(
            "{what} holds {} entries, at most {max}",
            list.len()
        ));
    }
    Ok(())
}

fn is_digest(s: &str) -> bool {
    s.strip_prefix(crate::hash::HASH_PREFIX)
        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn check_inputs(i: &RowInputs, kind: RowKind) -> Result<(), String> {
    if !is_digest(&i.workload) || !is_digest(&i.program) {
        return Err("the workload and program digests are blake3 digests".into());
    }
    if !text_ok(&i.program_name, 64) || !text_ok(&i.recipe, 64) || !text_ok(&i.launcher, 64) {
        return Err("program_name, recipe and launcher are short plain text".into());
    }
    let c = &i.computer;
    for s in [&c.os, &c.build, &c.arch, &c.cpu] {
        if !text_ok(s, MAX_TEXT) {
            return Err("the computer's facts are short plain text".into());
        }
    }
    if !text_ok(&i.compilers.cc, MAX_TEXT) {
        return Err("the compilers' lines are short plain text".into());
    }
    match (&i.compilers.rustc, kind) {
        (Some(_), RowKind::CAlone) => return Err("the C alone records no rustc".into()),
        (Some(r), _) if !text_ok(r, MAX_TEXT) => {
            return Err("the compilers' lines are short plain text".into())
        }
        _ => {}
    }
    let unit_ok = |id: &str| crate::plan::is_clean_segment(id);
    if let Some(crates) = &i.crates {
        if kind == RowKind::CAlone {
            return Err("the C alone has no crate".into());
        }
        at_most(crates, MAX_UNITS, "crates")?;
        for c in crates {
            if !unit_ok(&c.id) || !is_digest(&c.digest) {
                return Err("a crate is a unit id and a digest".into());
            }
        }
    }
    if let Some(r) = &i.replaces {
        at_most(r, MAX_REPLACES, "replaces")?;
        if kind != RowKind::Unit || r.iter().any(|p| !crate::plan::is_clean_relative_path(p)) {
            return Err("replaces are a unit's clean paths".into());
        }
    }
    let mut ids: Vec<&str> = Vec::new();
    if let Some(units) = &i.units {
        if kind != RowKind::AsItStands {
            return Err("only the program as it stands holds units".into());
        }
        at_most(units, MAX_UNITS, "units")?;
        for u in units {
            if !unit_ok(&u.id) || !is_digest(&u.crate_digest) || ids.contains(&u.id.as_str()) {
                return Err("units are unit ids, once each, with crate digests".into());
            }
            ids.push(&u.id);
        }
    }
    if let Some(left) = &i.left_out {
        if kind != RowKind::AsItStands {
            return Err("only the program as it stands leaves units out".into());
        }
        at_most(left, MAX_UNITS, "left_out")?;
        for u in left {
            let digest_ok = u.crate_digest.is_empty() || is_digest(&u.crate_digest);
            if !unit_ok(&u.id)
                || !digest_ok
                || !LEFT_OUT_REASONS.contains(&u.reason.as_str())
                || ids.contains(&u.id.as_str())
            {
                return Err("left_out holds unit ids, once each, with closed reasons".into());
            }
            ids.push(&u.id);
        }
    }
    Ok(())
}

fn check_runs(runs: &[Run], n: u32) -> Result<(), String> {
    if runs.len() != n as usize {
        return Err(format!("{} runs where runs is {n}", runs.len()));
    }
    for r in runs {
        for v in [r.instructions, r.cycles, r.cpu_us, r.wall_us, r.memory]
            .into_iter()
            .flatten()
        {
            if v == 0 {
                return Err("a counter, a time or a memory is at least 1 when present".into());
            }
        }
        if r.p_instructions.is_some() != r.p_cycles.is_some() {
            return Err("the performance-core counts are both present or both absent".into());
        }
        check_end(&r.end, false)?;
    }
    Ok(())
}

/// Whether `end` reads `exit N` (0–255) or `signal N` (1–64) — or, where
/// a step-1 or failed run may say so, `timeout` or `never-started`.
fn check_end(end: &str, more: bool) -> Result<(), String> {
    if more && matches!(end, "timeout" | "never-started") {
        return Ok(());
    }
    let ok = match end.split_once(' ') {
        Some(("exit", n)) => n.parse::<u8>().is_ok_and(|v| v.to_string() == n),
        Some(("signal", n)) => n
            .parse::<u8>()
            .is_ok_and(|v| (1..=64).contains(&v) && v.to_string() == n),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("end {end:?} is not exit N or signal N"))
    }
}

fn check_setup(outcome: &str, setup: Option<&SetupFacts>) -> Result<(), String> {
    let Some(s) = setup else {
        return match outcome {
            "not-verified" | "replaces-mismatch" | "does-not-link" | "mixed-panic"
            | "input-unusable" => Err(format!("{outcome} needs its set-up facts")),
            _ => Ok(()),
        };
    };
    let none = SetupFacts::default();
    let only = |allowed: &SetupFacts| -> bool {
        (s.runtimes.is_none() || allowed.runtimes.is_some())
            && (s.cause.is_none() || allowed.cause.is_some())
            && (s.units.is_none() || allowed.units.is_some())
            && (s.index.is_none() || allowed.index.is_some())
            && (s.input.is_none() || allowed.input.is_some())
            && (s.reason.is_none() || allowed.reason.is_some())
            && (s.attempt.is_none() || allowed.attempt.is_some())
            && (s.never_started.is_none() || allowed.never_started.is_some())
    };
    let some_str = Some(String::new());
    let allowed = match outcome {
        "mixed-panic" => SetupFacts {
            runtimes: Some(Vec::new()),
            ..none
        },
        "does-not-link" => SetupFacts {
            cause: some_str.clone(),
            units: Some(Vec::new()),
            ..none
        },
        "replaces-mismatch" => SetupFacts {
            index: Some(0),
            ..none
        },
        "input-unusable" => SetupFacts {
            input: some_str.clone(),
            ..none
        },
        "not-verified" => SetupFacts {
            reason: some_str.clone(),
            attempt: some_str.clone(),
            ..none
        },
        "could-not-start" | "c-could-not-start" => SetupFacts {
            never_started: some_str.clone(),
            ..none
        },
        "crate-does-not-build" | "run-failed: unmeasurable" => none,
        _ => return Err(format!("{outcome} holds no set-up facts")),
    };
    if !only(&allowed) {
        return Err(format!("{outcome} holds set-up facts that are not its own"));
    }
    if let Some(r) = &s.runtimes {
        at_most(r, MAX_UNITS, "runtimes")?;
    }
    if let Some(u) = &s.units {
        at_most(u, MAX_UNITS, "the set-up's units")?;
    }
    if let Some(log) = &s.log {
        if !crate::plan::is_clean_segment(log) || log.len() > 64 {
            return Err("a log is a plain file name".into());
        }
    }
    let unit_ok = |id: &str| crate::plan::is_clean_segment(id);
    match outcome {
        "mixed-panic" => {
            let r = s.runtimes.as_deref().unwrap_or_default();
            if r.len() < 2
                || r.iter()
                    .any(|u| !unit_ok(&u.id) || !RUNTIMES.contains(&u.runtime.as_str()))
            {
                return Err("mixed-panic names two or more units and their runtimes".into());
            }
        }
        "does-not-link" => {
            if !s.cause.as_deref().is_some_and(|c| LINK_CAUSES.contains(&c))
                || s.units
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|u| !unit_ok(u))
            {
                return Err("does-not-link names a closed cause and unit ids".into());
            }
        }
        "replaces-mismatch" => {
            if s.index.is_none_or(|i| i >= MAX_REPLACES_INDEX) {
                return Err(format!(
                    "replaces-mismatch names the entry's index, below {MAX_REPLACES_INDEX}"
                ));
            }
        }
        "input-unusable" => {
            let known = s
                .input
                .as_deref()
                .and_then(super::workloads::InputUnusable::from_token);
            if known.is_none() {
                return Err("input-unusable names a closed reason".into());
            }
        }
        "not-verified" => {
            if !s
                .reason
                .as_deref()
                .is_some_and(|r| NOT_VERIFIED_REASONS.contains(&r))
            {
                return Err("not-verified names a closed reason".into());
            }
            match (s.reason.as_deref(), s.attempt.as_deref()) {
                (Some("accept-interrupted"), Some(a)) if unit_ok(a) => {}
                (Some("accept-interrupted"), _) => {
                    return Err("an interrupted Accept names its attempt".into())
                }
                (_, Some(_)) => return Err("only an interrupted Accept names an attempt".into()),
                _ => {}
            }
        }
        "could-not-start" | "c-could-not-start" => {
            if let Some(n) = &s.never_started {
                if n != "no-ready" && n.parse::<u16>().is_err() {
                    return Err("never_started is an errno or no-ready".into());
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_difference(d: &Difference) -> Result<(), String> {
    if !matches!(d.stream.as_str(), "stdout" | "stderr" | "exit") {
        return Err("a difference's stream is stdout, stderr or exit".into());
    }
    check_end(&d.c_end, true)?;
    check_end(&d.other_end, true)?;
    // What perf itself writes: both lengths within the output cap, the
    // first differing byte no further than the shorter one ends, and an
    // exit difference with no lengths at all.
    if d.c_len > MAX_OUTPUT_BYTES
        || d.other_len > MAX_OUTPUT_BYTES
        || d.offset > d.c_len.min(d.other_len)
    {
        return Err(
            "a difference's lengths are at most 64 MiB and its byte within the shorter".into(),
        );
    }
    if d.stream == "exit" && (d.c_len, d.other_len, d.offset) != (0, 0, 0) {
        return Err("an exit difference has no lengths and no byte".into());
    }
    at_most(&d.kept, MAX_KEPT, "kept")?;
    for k in &d.kept {
        let name_ok = crate::plan::is_clean_segment(&k.name)
            && [".c.stdout", ".c.stderr", ".other.stdout", ".other.stderr"]
                .iter()
                .any(|s| k.name.ends_with(s));
        if !name_ok || !is_digest(&k.blake3) || k.size > MAX_OUTPUT_BYTES {
            return Err(
                "a kept file is <workload>.{c,other}.{stdout,stderr} with its blake3, at most \
                 64 MiB"
                    .into(),
            );
        }
    }
    Ok(())
}

/// The one rule for what a new row does to the workload's earlier row in
/// the same list (§3.7, note 23), returning the row to store:
/// - a set-up outcome never replaces an earlier row that is not itself a
///   set-up row — it is kept beside it as `last_try`; it replaces an
///   earlier set-up row; with no earlier row it is the row;
/// - on the C-alone rows, the C's own outcomes replace a C-side or
///   too-short row and are otherwise kept beside the baseline as
///   `last_try`;
/// - a behaves-differently finding survives every re-measure that does not
///   end `measured` or `too-short` (both compare the output the same):
///   kept beside the new row as `found_before`.
pub fn merge(earlier: Option<&Row>, new: Row, kind: RowKind) -> Row {
    let Some(old) = earlier else {
        return new;
    };
    let beside = |old: &Row, new: &Row| -> Row {
        let mut kept = old.clone();
        kept.last_try = Some(LastTry {
            outcome: new.outcome.clone(),
            setup: new.setup.clone(),
            units: new.inputs.units.clone(),
        });
        kept
    };
    let o = new.outcome.clone();
    if kind == RowKind::CAlone && is_c_side(&o) {
        let replaceable =
            is_set_up(&old.outcome) || is_c_side(&old.outcome) || old.outcome == "too-short";
        return if replaceable { new } else { beside(old, &new) };
    }
    if is_set_up(&o) {
        return if is_set_up(&old.outcome) {
            new
        } else {
            beside(old, &new)
        };
    }
    let mut new = new;
    let clears = matches!(o.as_str(), "behaves-differently" | "measured" | "too-short");
    if !clears && new.found_before.is_none() {
        new.found_before = old
            .first_difference
            .clone()
            .or_else(|| old.found_before.clone());
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(c: char) -> String {
        format!("blake3:{}", c.to_string().repeat(64))
    }

    fn inputs(kind: RowKind) -> RowInputs {
        RowInputs {
            workload: digest('a'),
            program: digest('b'),
            crates: (kind != RowKind::CAlone).then(|| {
                vec![CrateDigest {
                    id: "u001".into(),
                    digest: digest('c'),
                }]
            }),
            replaces: (kind == RowKind::Unit).then(|| vec!["src/a.c".into()]),
            program_name: "tool".into(),
            units: None,
            left_out: None,
            recipe: super::super::PERF_RECIPE.into(),
            launcher: super::super::PERF_LAUNCHER.into(),
            computer: Computer {
                os: "15.6".into(),
                build: "24G84".into(),
                arch: "arm64".into(),
                cpu: "Apple M3".into(),
                two_kinds: true,
                fast_cores: 4,
            },
            compilers: Compilers {
                cc: "Apple clang version 17.0.0".into(),
                rustc: (kind != RowKind::CAlone).then(|| "rustc 1.94.1".into()),
            },
        }
    }

    fn run(i: u64) -> Run {
        Run {
            instructions: Some(1_000_000_000 + i),
            cycles: Some(400_000_000 + i),
            cpu_us: Some(500_000),
            wall_us: Some(510_000),
            memory: Some(1 << 20),
            p_instructions: Some(1_000_000_000),
            p_cycles: Some(400_000_000),
            load: Some(250),
            end: "exit 0".into(),
            ..Run::default()
        }
    }

    fn measured(kind: RowKind) -> Row {
        let baseline = kind == RowKind::CAlone;
        Row {
            workload: "big-text".into(),
            outcome: if baseline { "baseline" } else { "measured" }.into(),
            short: Some(false),
            runs: Some(5),
            platform_metrics: Some("macos-v6-pnorm".into()),
            inputs: inputs(kind),
            c: Some((0..5).map(run).collect()),
            other: (!baseline).then(|| (0..5).map(run).collect()),
            std: None,
            fat_lto: None,
            profile: None,
            step1: None,
            failed_run: None,
            setup: None,
            first_difference: None,
            found_before: None,
            last_try: None,
        }
    }

    fn other(kind: RowKind, outcome: &str, setup: Option<SetupFacts>) -> Row {
        Row {
            outcome: outcome.into(),
            short: None,
            runs: None,
            platform_metrics: None,
            c: None,
            other: None,
            setup,
            ..measured(kind)
        }
    }

    fn difference() -> Difference {
        Difference {
            stream: "stdout".into(),
            c_len: 10,
            other_len: 12,
            offset: 4,
            c_end: "exit 0".into(),
            other_end: "exit 0".into(),
            over_cap: false,
            kept: vec![KeptFile {
                name: "big-text.c.stdout".into(),
                size: 10,
                blake3: digest('d'),
            }],
        }
    }

    #[test]
    fn measured_rows_read_by_their_rules() {
        for kind in [RowKind::CAlone, RowKind::Unit, RowKind::AsItStands] {
            check_row(&measured(kind), kind).expect("a valid row");
        }
        let mut r = measured(RowKind::Unit);
        r.runs = Some(4);
        assert!(check_row(&r, RowKind::Unit).is_err(), "runs 5 to 31");
        let mut r = measured(RowKind::Unit);
        r.c.as_mut().expect("runs").pop();
        assert!(check_row(&r, RowKind::Unit).unwrap_err().contains("4 runs"));
        let mut r = measured(RowKind::Unit);
        r.other.as_mut().expect("runs")[0].instructions = Some(0);
        assert!(check_row(&r, RowKind::Unit).is_err(), "a zero counter");
        let mut r = measured(RowKind::Unit);
        r.other.as_mut().expect("runs")[0].p_cycles = None;
        assert!(check_row(&r, RowKind::Unit).unwrap_err().contains("both"));
        let mut r = measured(RowKind::Unit);
        r.c.as_mut().expect("runs")[0].end = "exit 256".into();
        assert!(check_row(&r, RowKind::Unit).is_err());
        let mut r = measured(RowKind::Unit);
        r.platform_metrics = Some("magic".into());
        assert!(check_row(&r, RowKind::Unit).is_err());
        // A baseline is only the C alone's; measured never is.
        assert!(check_row(&measured(RowKind::CAlone), RowKind::Unit).is_err());
        assert!(check_row(&measured(RowKind::Unit), RowKind::CAlone).is_err());
    }

    #[test]
    fn the_c_side_outcomes_live_only_on_the_c_alone_rows() {
        for o in C_SIDE {
            check_row(&other(RowKind::CAlone, o, None), RowKind::CAlone).expect("c-alone");
            for kind in [RowKind::Unit, RowKind::AsItStands] {
                assert!(check_row(&other(kind, o, None), kind).is_err(), "{o}");
            }
        }
    }

    #[test]
    fn set_up_facts_are_closed() {
        let k = RowKind::Unit;
        let mix = SetupFacts {
            runtimes: Some(vec![
                UnitRuntime {
                    id: "u001".into(),
                    runtime: "unwind".into(),
                },
                UnitRuntime {
                    id: "u002".into(),
                    runtime: "abort".into(),
                },
            ]),
            ..SetupFacts::default()
        };
        check_row(
            &other(RowKind::AsItStands, "mixed-panic", Some(mix.clone())),
            RowKind::AsItStands,
        )
        .expect("mixed-panic");
        assert!(check_row(&other(k, "mixed-panic", None), k).is_err());
        let link = SetupFacts {
            cause: Some("lto".into()),
            units: Some(vec!["u002".into()]),
            ..SetupFacts::default()
        };
        check_row(&other(k, "does-not-link", Some(link)), k).expect("does-not-link");
        let bad_cause = SetupFacts {
            cause: Some("cosmic rays".into()),
            ..SetupFacts::default()
        };
        assert!(check_row(&other(k, "does-not-link", Some(bad_cause)), k).is_err());
        let input = SetupFacts {
            input: Some("too-large".into()),
            ..SetupFacts::default()
        };
        check_row(&other(k, "input-unusable", Some(input)), k).expect("input-unusable");
        let wrong = SetupFacts {
            input: Some("too-large".into()),
            ..SetupFacts::default()
        };
        assert!(check_row(&other(k, "replaces-mismatch", Some(wrong)), k).is_err());
        let nv = SetupFacts {
            reason: Some("accept-interrupted".into()),
            attempt: Some("a-1234".into()),
            ..SetupFacts::default()
        };
        check_row(&other(k, "not-verified", Some(nv)), k).expect("not-verified");
        let nv = SetupFacts {
            reason: Some("accept-interrupted".into()),
            ..SetupFacts::default()
        };
        assert!(
            check_row(&other(k, "not-verified", Some(nv)), k).is_err(),
            "needs the attempt"
        );
        let log = SetupFacts {
            log: Some("../../etc".into()),
            ..SetupFacts::default()
        };
        assert!(check_row(&other(k, "crate-does-not-build", Some(log)), k).is_err());
    }

    #[test]
    fn a_difference_and_free_text_are_checked() {
        let k = RowKind::Unit;
        let mut r = other(k, "behaves-differently", None);
        assert!(check_row(&r, k).is_err(), "needs first_difference");
        r.first_difference = Some(difference());
        check_row(&r, k).expect("valid");
        r.first_difference.as_mut().expect("d").kept[0].name = "../x".into();
        assert!(check_row(&r, k).is_err());
        let mut r = measured(k);
        r.inputs.computer.cpu = "Apple\u{202E}M3".into();
        assert!(check_row(&r, k).is_err(), "unsafe text refused");
    }

    #[test]
    fn the_replace_rule() {
        let k = RowKind::Unit;
        let set_up = other(k, "crate-does-not-build", None);
        // No earlier row: the new row is the row.
        assert_eq!(merge(None, set_up.clone(), k), set_up);
        // A set-up outcome never replaces a measured row: it is its last try.
        let m = merge(Some(&measured(k)), set_up.clone(), k);
        assert_eq!(m.outcome, "measured");
        assert_eq!(
            m.last_try.as_ref().map(|t| t.outcome.as_str()),
            Some("crate-does-not-build")
        );
        check_row(&m, k).expect("valid");
        // A set-up row is replaced by a newer set-up row.
        let newer = other(
            k,
            "replaces-mismatch",
            Some(SetupFacts {
                index: Some(1),
                ..SetupFacts::default()
            }),
        );
        assert_eq!(merge(Some(&set_up), newer.clone(), k), newer);
        // A behaves-differently finding survives a run-failed re-measure ...
        let mut bd = other(k, "behaves-differently", None);
        bd.first_difference = Some(difference());
        let failed = other(k, "run-failed: timeout", None);
        let kept = merge(Some(&bd), failed, k);
        assert_eq!(kept.found_before, Some(difference()));
        check_row(&kept, k).expect("valid");
        // ... and a set-up one (kept as the row, beside it) ...
        let kept = merge(Some(&bd), set_up.clone(), k);
        assert_eq!(kept.outcome, "behaves-differently");
        // ... and clears only on measured or too-short.
        assert_eq!(merge(Some(&bd), measured(k), k).found_before, None);
        assert_eq!(
            merge(Some(&bd), other(k, "too-short", None), k).found_before,
            None
        );
        // On the C alone: the C's own outcome beside a baseline, over a
        // C-side or too-short row.
        let c = RowKind::CAlone;
        let unstable = other(c, "c-unstable", None);
        let m = merge(Some(&measured(c)), unstable.clone(), c);
        assert_eq!(m.outcome, "baseline");
        assert_eq!(
            m.last_try.as_ref().map(|t| t.outcome.as_str()),
            Some("c-unstable")
        );
        check_row(&m, c).expect("valid");
        assert_eq!(
            merge(Some(&other(c, "too-short", None)), unstable.clone(), c),
            unstable
        );
        assert_eq!(
            merge(Some(&other(c, "c-crashed", None)), unstable.clone(), c),
            unstable
        );
    }

    #[test]
    fn a_set_up_try_is_kept_beside_a_too_short_or_c_side_c_alone_row() {
        // A missing input, or a launcher failure, after a C-alone row that
        // was too short or one of the C's own: the row stays, the try goes
        // beside it, and the files take it (a run goes on to the next
        // workload).
        let c = RowKind::CAlone;
        let missing = other(
            c,
            "input-unusable",
            Some(SetupFacts {
                input: Some("missing".into()),
                ..SetupFacts::default()
            }),
        );
        let unmeasurable = other(c, "run-failed: unmeasurable", None);
        let dir = std::env::temp_dir().join(format!("perf-t-{}", crate::hash::random_hex(6)));
        std::fs::create_dir_all(&dir).expect("dir");
        let p = program_path(&dir);
        for earlier in std::iter::once("too-short").chain(C_SIDE.iter().copied()) {
            let old = other(c, earlier, None);
            for new in [&missing, &unmeasurable] {
                let m = merge(Some(&old), new.clone(), c);
                assert_eq!(m.outcome, earlier);
                assert_eq!(
                    m.last_try.as_ref().map(|t| t.outcome.as_str()),
                    Some(new.outcome.as_str())
                );
                check_row(&m, c).unwrap_or_else(|e| panic!("{earlier} + {}: {e}", new.outcome));
                let mut program = ProgramResults::default();
                program.c_alone.push(m);
                write_program(&p, &program).expect("written");
                assert_eq!(read_program(&p).expect("read"), Some(program));
            }
        }
        // The C's own outcome never sits beside such a row: it replaces it.
        let mut m = other(c, "too-short", None);
        m.last_try = Some(LastTry {
            outcome: "c-crashed".into(),
            setup: None,
            units: None,
        });
        assert!(check_row(&m, c).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_last_try_sits_only_where_the_replace_rule_puts_it() {
        // The rows the replace rule writes (§3.7, note 23), and none other:
        // a reader takes a last try only where merge could have put it.
        let try_of = |outcome: &str| {
            Some(LastTry {
                outcome: outcome.into(),
                setup: None,
                units: None,
            })
        };
        let unmeasurable = "run-failed: unmeasurable";
        for kind in [RowKind::CAlone, RowKind::Unit, RowKind::AsItStands] {
            // A set-up outcome sits beside a measured row ...
            let mut m = measured(kind);
            m.last_try = try_of(unmeasurable);
            check_row(&m, kind).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            // ... never beside a set-up row: a newer set-up row replaces it.
            let mut s = other(kind, unmeasurable, None);
            s.last_try = try_of(unmeasurable);
            assert!(
                check_row(&s, kind).is_err(),
                "{kind:?}: beside a set-up row"
            );
            // A last try is a set-up or C outcome, never one a run measures.
            for outcome in ["measured", "too-short", "run-failed: timeout"] {
                let mut m = measured(kind);
                m.last_try = try_of(outcome);
                assert!(check_row(&m, kind).is_err(), "{kind:?}: {outcome}");
            }
        }
        // The C's own outcome replaces a C-side row: never kept beside one.
        let c = RowKind::CAlone;
        for earlier in C_SIDE {
            let mut r = other(c, earlier, None);
            r.last_try = try_of("c-unstable");
            assert!(check_row(&r, c).is_err(), "c-unstable beside {earlier}");
        }
        // The units' and the program's rows are left as they were: no C
        // outcome is ever kept beside them.
        for kind in [RowKind::Unit, RowKind::AsItStands] {
            let mut m = measured(kind);
            m.last_try = try_of("c-crashed");
            assert!(check_row(&m, kind).is_err(), "{kind:?}: the C's own");
        }
    }

    #[test]
    fn a_last_try_keeps_the_set_up_facts_and_units_it_was_measured_with() {
        // The program as it stands, measured with u001; a later try with
        // u001 and u002 does not link: the row stays, and its last try says
        // why and which units it held.
        let s = RowKind::AsItStands;
        let unit = |id: &str| UnitRef {
            id: id.into(),
            crate_digest: digest('c'),
        };
        let mut earlier = measured(s);
        earlier.inputs.units = Some(vec![unit("u001")]);
        let link = SetupFacts {
            cause: Some("lto".into()),
            units: Some(vec!["u002".into()]),
            ..SetupFacts::default()
        };
        let mut new = other(s, "does-not-link", Some(link.clone()));
        new.inputs.units = Some(vec![unit("u001"), unit("u002")]);
        let m = merge(Some(&earlier), new, s);
        assert_eq!(m.outcome, "measured");
        assert_eq!(
            m.last_try,
            Some(LastTry {
                outcome: "does-not-link".into(),
                setup: Some(link),
                units: Some(vec![unit("u001"), unit("u002")]),
            })
        );
        check_row(&m, s).expect("valid");
    }

    #[test]
    fn the_cs_own_outcome_replaces_an_earlier_set_up_c_alone_row() {
        // A missing input on the C alone, then (the input back) the C
        // crashes: the crash is the row, not a try beside the set-up row.
        let c = RowKind::CAlone;
        let missing = other(
            c,
            "input-unusable",
            Some(SetupFacts {
                input: Some("missing".into()),
                ..SetupFacts::default()
            }),
        );
        let unmeasurable = other(c, "run-failed: unmeasurable", None);
        for earlier in [&missing, &unmeasurable] {
            for o in C_SIDE {
                let new = other(c, o, None);
                assert_eq!(merge(Some(earlier), new.clone(), c), new, "{o}");
            }
        }
    }

    #[test]
    fn a_differences_numbers_and_a_replaces_index_are_bounded() {
        let k = RowKind::Unit;
        let with = |d: Difference| {
            let mut r = other(k, "behaves-differently", None);
            r.first_difference = Some(d);
            r
        };
        check_row(&with(difference()), k).expect("valid");
        // The byte may be where the shorter output ends ...
        check_row(
            &with(Difference {
                offset: 10,
                ..difference()
            }),
            k,
        )
        .expect("the shorter's end");
        let mut kept = measured(k);
        kept.found_before = Some(difference());
        check_row(&kept, k).expect("a finding kept beside a row");
        // ... but never past it, past the cap, or on an exit difference.
        for bad in [
            Difference {
                offset: u64::MAX,
                ..difference()
            },
            Difference {
                offset: 11,
                ..difference()
            },
            Difference {
                c_len: MAX_OUTPUT_BYTES + 1,
                ..difference()
            },
            Difference {
                other_len: u64::MAX,
                ..difference()
            },
            Difference {
                stream: "exit".into(),
                ..difference()
            },
        ] {
            assert!(check_row(&with(bad.clone()), k).is_err(), "{bad:?}");
            // found_before is read by the same rule.
            let mut r = kept.clone();
            r.found_before = Some(bad);
            assert!(check_row(&r, k).is_err());
        }
        let mut d = difference();
        d.kept[0].size = MAX_OUTPUT_BYTES + 1;
        assert!(check_row(&with(d), k).is_err(), "a kept file over the cap");
        check_row(
            &with(Difference {
                stream: "exit".into(),
                c_len: 0,
                other_len: 0,
                offset: 0,
                other_end: "exit 1".into(),
                kept: Vec::new(),
                ..difference()
            }),
            k,
        )
        .expect("an exit difference");
        // A replaces-mismatch's index: within the unit's replaces when the
        // row holds them, and below the bound everywhere, its last try too.
        let index = |i: u32| {
            Some(SetupFacts {
                index: Some(i),
                ..SetupFacts::default()
            })
        };
        let mut r = other(k, "replaces-mismatch", index(0));
        check_row(&r, k).expect("the first entry");
        r.setup = index(1);
        assert!(check_row(&r, k).is_err(), "past the one entry it holds");
        r.inputs.replaces = None;
        check_row(&r, k).expect("no replaces held");
        r.setup = index(u32::MAX);
        assert!(check_row(&r, k).is_err());
        r.setup = index(MAX_REPLACES_INDEX);
        assert!(check_row(&r, k).is_err());
        let mut m = measured(k);
        m.last_try = Some(LastTry {
            outcome: "replaces-mismatch".into(),
            setup: index(u32::MAX),
            units: None,
        });
        assert!(check_row(&m, k).is_err(), "a last try's index");
        // Read back from a file, a huge byte is refused.
        let dir = std::env::temp_dir().join(format!("perf-b-{}", crate::hash::random_hex(6)));
        std::fs::create_dir_all(dir.join(UNITS_DIR)).expect("dir");
        let u = unit_path(&dir, "u001");
        let mut unit = UnitResults::new("u001");
        unit.rows.push(with(difference()));
        write_unit(&u, &unit).expect("write");
        let text = std::fs::read_to_string(&u).expect("read");
        std::fs::write(
            &u,
            text.replacen("\"offset\": 4", "\"offset\": 18446744073709551615", 1),
        )
        .expect("write");
        assert!(read_unit(&u, "u001").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A valid row whose list `what` holds `n` entries, and which list of
    /// rows it lives in.
    fn with_list(what: &str, n: usize) -> (Row, RowKind) {
        let (u, s) = (RowKind::Unit, RowKind::AsItStands);
        let unit = |i: usize| UnitRef {
            id: format!("u{i:04}"),
            crate_digest: digest('c'),
        };
        match what {
            "replaces" => {
                let mut r = measured(u);
                r.inputs.replaces = Some(vec!["src/a.c".into(); n]);
                (r, u)
            }
            "crates" => {
                let mut r = measured(s);
                let c = CrateDigest {
                    id: "u001".into(),
                    digest: digest('c'),
                };
                r.inputs.crates = Some(vec![c; n]);
                (r, s)
            }
            "units" => {
                let mut r = measured(s);
                r.inputs.units = Some((0..n).map(unit).collect());
                (r, s)
            }
            "left_out" => {
                let mut r = measured(s);
                let left = (0..n).map(|i| LeftOut {
                    id: format!("u{i:04}"),
                    crate_digest: String::new(),
                    reason: "not-fresh".into(),
                });
                r.inputs.left_out = Some(left.collect());
                (r, s)
            }
            "runtimes" => {
                let rt = UnitRuntime {
                    id: "u001".into(),
                    runtime: "abort".into(),
                };
                let setup = SetupFacts {
                    runtimes: Some(vec![rt; n]),
                    ..SetupFacts::default()
                };
                (other(s, "mixed-panic", Some(setup)), s)
            }
            "the set-up's units" => {
                let setup = SetupFacts {
                    cause: Some("unknown".into()),
                    units: Some(vec!["u001".into(); n]),
                    ..SetupFacts::default()
                };
                (other(u, "does-not-link", Some(setup)), u)
            }
            "last_try's units" => {
                let mut r = measured(s);
                r.last_try = Some(LastTry {
                    outcome: "crate-does-not-build".into(),
                    setup: None,
                    units: Some((0..n).map(unit).collect()),
                });
                (r, s)
            }
            "profile" => {
                let mut r = measured(u);
                let p = PROFILE_KEYS.iter().cycle().take(n).map(|k| ProfileSetting {
                    key: (*k).into(),
                    value: "1".into(),
                });
                r.profile = Some(p.collect());
                (r, u)
            }
            "kept" => {
                let mut r = other(u, "behaves-differently", None);
                let mut d = difference();
                d.kept = vec![d.kept[0].clone(); n];
                r.first_difference = Some(d);
                (r, u)
            }
            _ => panic!("no list {what}"),
        }
    }

    /// A forged file holding `row`, written as is (no check on the way),
    /// then read back by the reader.
    fn forged(dir: &Path, row: &Row, kind: RowKind) -> Result<(), Error> {
        let (path, text) = if kind == RowKind::Unit {
            let mut f = UnitResults::new("u001");
            f.rows.push(row.clone());
            (unit_path(dir, "u001"), serde_json::to_string(&f))
        } else {
            let mut f = ProgramResults::default();
            if kind == RowKind::CAlone {
                f.c_alone.push(row.clone());
            } else {
                f.as_it_stands.push(row.clone());
            }
            (program_path(dir), serde_json::to_string(&f))
        };
        std::fs::write(&path, text.expect("json")).expect("write");
        if kind == RowKind::Unit {
            read_unit(&path, "u001").map(|_| ())
        } else {
            read_program(&path).map(|_| ())
        }
    }

    #[test]
    fn every_list_a_row_holds_is_capped() {
        // A forged file one entry over a list's cap is refused by name,
        // before its entries are walked; one at the cap is read.
        let dir = std::env::temp_dir().join(format!("perf-l-{}", crate::hash::random_hex(6)));
        std::fs::create_dir_all(dir.join(UNITS_DIR)).expect("dir");
        for (what, cap) in [
            ("replaces", MAX_REPLACES),
            ("crates", MAX_UNITS),
            ("units", MAX_UNITS),
            ("left_out", MAX_UNITS),
            ("runtimes", MAX_UNITS),
            ("the set-up's units", MAX_UNITS),
            ("last_try's units", MAX_UNITS),
            ("profile", PROFILE_KEYS.len()),
            ("kept", MAX_KEPT),
        ] {
            let (row, kind) = with_list(what, cap);
            forged(&dir, &row, kind).unwrap_or_else(|e| panic!("{what} at its cap: {e}"));
            let (row, kind) = with_list(what, cap + 1);
            let err = forged(&dir, &row, kind).expect_err(what).to_string();
            let words = format!("{what} holds {} entries, at most {cap}", cap + 1);
            assert!(err.contains(&words), "{what}: {err}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_round_trip_and_read_strictly() {
        let dir = std::env::temp_dir().join(format!("perf-r-{}", crate::hash::random_hex(6)));
        std::fs::create_dir_all(dir.join(UNITS_DIR)).expect("dir");
        let p = program_path(&dir);
        assert_eq!(read_program(&p).expect("absent"), None);
        let mut program = ProgramResults::default();
        program.c_alone.push(measured(RowKind::CAlone));
        program.as_it_stands.push(measured(RowKind::AsItStands));
        write_program(&p, &program).expect("write");
        assert_eq!(read_program(&p).expect("read"), Some(program.clone()));
        let u = unit_path(&dir, "u001");
        let mut unit = UnitResults::new("u001");
        unit.rows.push(measured(RowKind::Unit));
        write_unit(&u, &unit).expect("write");
        assert_eq!(read_unit(&u, "u001").expect("read"), Some(unit));
        assert!(read_unit(&u, "u002").is_err(), "another unit's file");
        // An unknown key, a newer version, a link.
        let text = std::fs::read_to_string(&p).expect("read");
        std::fs::write(
            &p,
            text.replacen("\"c_alone\"", "\"extra\": 1, \"c_alone\"", 1),
        )
        .expect("write");
        assert!(read_program(&p).is_err());
        std::fs::write(
            &p,
            text.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1),
        )
        .expect("write");
        assert!(matches!(read_program(&p), Err(Error::SchemaTooNew { .. })));
        std::fs::remove_file(&p).expect("rm");
        std::os::unix::fs::symlink(&u, &p).expect("link");
        assert!(read_program(&p).is_err(), "a link is refused");
        std::fs::remove_dir_all(&dir).ok();
    }
}
