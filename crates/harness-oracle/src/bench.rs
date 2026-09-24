//! Held-out benchmark scoring (docs/SCHEMAS.md "M4 additions", docs/
//! M4-DESIGN.md §6 as amended by R2, R3, R4, R7, R9, R10).
//!
//! The corpus's OWN scorer (`cando2` + one runner per case) judges each
//! vector; this module only builds it and runs it, confined:
//!
//! - **Verify what you use (R9).** The locked `heldout/` files are copied into
//!   a fresh snapshot under `<suite>/.bench/snapshot/` and every copy is
//!   re-hashed against `corpus.lock` before anything is built from it.
//! - **Frozen, offline scorer build (R7).** `cargo build --frozen` against the
//!   vendored dependency sources (`<suite>/.scorer-vendor`, checksum-verified
//!   by cargo against the committed `heldout/Cargo.lock`), with an empty
//!   harness-owned `CARGO_HOME`, inside the tool sandbox (network denied).
//! - **Confined vector runs (R2).** Each case side gets a fresh run root in
//!   the SYSTEM temp dir holding only copies of the vectors and the one dylib;
//!   the runner runs under the run profile: it may `exec` only itself (cando's
//!   procspawn re-executes itself per vector), read nothing under the home
//!   directory or the suite except its own binary, and write only a fresh
//!   per-run `TMPDIR`, where its report is written. A report is accepted only
//!   after a NORMAL exit (0 = pass, 1 = vector failure) and parsed as a closed
//!   enum with a size cap.
//! - **The platform's C (R4).** The C baseline is `cc -shared -fPIC -O0
//!   -ffp-contract=off` (O0 = CMake's default for an empty build type; no FMA
//!   contraction, as on the reference Linux build).
//!
//! Result strings (closed): `pass`, `skip` (`has_ub`), `timeout` (twice),
//! `fail:<cando ResultType>`, `fail:runner-exit-<code>` / `fail:runner-killed`.
//!
//! **Sanitized C pass (docs/ORACLE-HARDENING.md §A, §A.R authoritative).**
//! A vector the plain C passed but the Rust or candidate did not is re-run
//! against the C built with `-fsanitize=address`, by the corpus's own runner
//! built a second time with the ASan runtime as a LINK dependency (so it is
//! resident before `dlopen`, with no `DYLD_*` variable — SIP's `sandbox-exec`
//! would purge one). cando captures each vector's child stderr into its
//! report; an allow-listed ASan report there excuses the vector as
//! `unmarked-ub`. Results: `clean` | `ub:<kind>` | `sanitizer:<kind>` |
//! `fail:<cando ResultType>` | `timeout` | the infra strings above.

use crate::exec::Runner;
use crate::sandbox::{self, HostDirs, ProfileSpec, RunSpec};
use harness_core::bench::{is_infra_result, CorpusLock, Suite, SuiteCase, HELDOUT_DIR};
use harness_core::error::Error;
use harness_core::hash;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Wall-clock limit of the scorer build.
const BUILD_TIMEOUT: Duration = Duration::from_secs(1800);
/// Wall-clock limit of one vector run (retried once on expiry).
const VECTOR_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest runner report accepted.
const MAX_REPORT_BYTES: u64 = 1 << 20;
/// The closed set of cando `result` strings.
const RESULT_TYPES: [&str; 9] = [
    "Pass",
    "Skip",
    "VectorComparisonFailed",
    "Panic",
    "SegmentationFault",
    "Timeout",
    "NoCompare",
    "UnknownFailure",
    "Benchmark",
];
/// Every C compile the scorer runs for the baseline.
const BASELINE_CFLAGS: [&str; 4] = ["-shared", "-fPIC", "-O0", "-ffp-contract=off"];
/// The sanitized C baseline (R-A2: ASan, never UBSan — UBSan fires on
/// intentional two's-complement idioms the Rust must reproduce). Tried with
/// [`BOUNDS_SAFETY_CFLAG`] first, then without (R-A13).
const SANITIZED_CFLAGS: [&str; 6] = [
    "-shared",
    "-fPIC",
    "-O0",
    "-ffp-contract=off",
    "-fsanitize=address",
    "-g",
];
/// Apple clang's bounds-carrying local pointers (§A.2): traps on an access
/// out of the bounds a pointer was derived from — inside memory the runner
/// owns, where ASan sees nothing. Unannotated code doing arithmetic on
/// parameter/member pointers does not compile with it (then: ASan only).
const BOUNDS_SAFETY_CFLAG: &str = "-fbounds-safety";
/// The signal a `-fbounds-safety` check raises on macOS arm64 (`brk`).
const SIGTRAP: i64 = 5;
/// The ASan runtime the sanitized runner links (macOS).
const ASAN_RUNTIME: &str = "libclang_rt.asan_osx_dynamic.dylib";
/// Harness-set, never inherited (R-A9): no `log_path` (reports stay on the
/// captured stream); `symbolize=0` (the run profile denies exec of `atos`).
const ASAN_OPTIONS: &str =
    "detect_leaks=0:abort_on_error=1:halt_on_error=1:symbolize=0:print_summary=1";
/// ASan report kinds that excuse a vector (R-A2): memory-access errors.
const EXCUSED_ASAN_KINDS: [&str; 9] = [
    "stack-buffer-overflow",
    "stack-buffer-underflow",
    "heap-buffer-overflow",
    "global-buffer-overflow",
    "dynamic-stack-buffer-overflow",
    "heap-use-after-free",
    "stack-use-after-return",
    "stack-use-after-scope",
    "use-after-poison",
];
/// Longest sanitizer kind recorded.
const MAX_KIND_BYTES: usize = 48;

static RUN_COUNTER: AtomicUsize = AtomicUsize::new(0);
/// Seconds after the epoch of the mtime every snapshot file gets (1970-01-02).
const PINNED_MTIME_SECS: u64 = 86_400;

/// The C side of a case: sources and include dirs (canonical paths).
#[derive(Debug, Clone)]
pub struct CSide {
    /// The case's `.c` files.
    pub sources: Vec<PathBuf>,
    /// `-I` dirs, in order.
    pub include_dirs: Vec<PathBuf>,
}

/// One vector's results (`None` = that side was not scored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorRun {
    /// Vector file name.
    pub name: String,
    /// C baseline result.
    pub c: String,
    /// Verified-Rust result.
    pub rust: Option<String>,
    /// Unverified-candidate result.
    pub candidate: Option<String>,
    /// Sanitized-C result (module docs), when the vector needed one.
    pub c_sanitized: Option<String>,
}

/// A case's runs: per vector, plus which sanitized C build ran (`None` =
/// the pass did not run for this case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRuns {
    /// Per vector, by name.
    pub vectors: Vec<VectorRun>,
    /// `asan+bounds-safety` | `asan` | `none` (docs/ORACLE-HARDENING.md §A.2).
    pub sanitized_build: Option<String>,
}

/// The sanitized pass of this scorer (R-A10: the reason is a closed set).
#[derive(Debug)]
enum Sanitized {
    /// Runners linked with the ASan runtime.
    Ready { runners: PathBuf },
    /// Not available here.
    Skipped(&'static str),
}

/// A built scorer: runner binaries plus the verified snapshot they came from.
#[derive(Debug)]
pub struct Scorer {
    /// Held for the scorer's lifetime: one scoring run per suite at a time
    /// (runs share and recreate the verified snapshot).
    _lock: BenchLock,
    suite_dir: PathBuf,
    snapshot: PathBuf,
    runners: PathBuf,
    host: Option<HostDirs>,
    tools: Runner,
    environment: Vec<String>,
    sanitized: Sanitized,
}

impl Scorer {
    /// Snapshot + verify the held-out material, then build every case runner
    /// (incremental after the first build).
    pub fn prepare(suite_dir: &Path, suite: &Suite, lock: &CorpusLock) -> Result<Scorer, Error> {
        let suite_dir = suite_dir
            .canonicalize()
            .map_err(|e| Error::io(suite_dir, e))?;
        lock.require_verified(&suite_dir, suite)?;
        let bench = suite_dir.join(".bench");
        std::fs::create_dir_all(&bench).map_err(|e| Error::io(&bench, e))?;
        let bench_lock = BenchLock::acquire(&bench.join("LOCK"))?;
        let snapshot = bench.join("snapshot");
        if snapshot.exists() {
            std::fs::remove_dir_all(&snapshot).map_err(|e| Error::io(&snapshot, e))?;
        }
        let prefix = format!("{HELDOUT_DIR}/");
        for f in lock.files.iter().filter(|f| f.path.starts_with(&prefix)) {
            let src = suite_dir.join(&f.path);
            let dst = snapshot.join(&f.path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::copy(&src, &dst).map_err(|e| Error::io(&src, e))?;
            // Pinned mtime: cargo's freshness check for path dependencies is
            // mtime-based, so a fresh snapshot would otherwise rebuild every
            // runner. Safe because the target dir is keyed by the lock hash:
            // within one key the bytes are fixed (verified just below).
            std::fs::File::options()
                .write(true)
                .open(&dst)
                .and_then(|f| {
                    f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(PINNED_MTIME_SECS))
                })
                .map_err(|e| Error::io(&dst, e))?;
            // Verify the COPY: what gets built is exactly what is locked.
            if hash::file_hash(&dst)? != f.hash {
                return Err(Error::Invariant(format!(
                    "snapshot copy of {} does not match corpus.lock",
                    f.path
                )));
            }
        }
        let vendor_raw = suite_dir.join(".scorer-vendor");
        let vendor = vendor_raw.canonicalize().map_err(|_| {
            Error::Invariant(format!(
                "scorer dependencies are not vendored at {} — run the one-time `cargo vendor` \
                 step (targets/tractor/README.md)",
                vendor_raw.display()
            ))
        })?;
        let cargo_home = bench.join("cargo-home");
        // One target dir per corpus.lock: a changed lock can never reuse
        // runners built from other bytes (see the pinned mtimes above).
        let lock_digest = hash::file_hash(&suite_dir.join("corpus.lock"))?;
        let lock_key = lock_digest.trim_start_matches("blake3:");
        let target_dir = bench.join(format!("scorer-target-{}", &lock_key[..12]));
        for dir in [&cargo_home, &target_dir] {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        // An empty CARGO_HOME: no user config can steer the build.
        for stray in ["config", "config.toml", "credentials", "credentials.toml"] {
            let p = cargo_home.join(stray);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| Error::io(&p, e))?;
            }
        }
        let host = match crate::sandbox_mode() {
            "sandbox-exec" => Some(HostDirs::from_env()?),
            _ => None,
        };
        let snapshot_root = snapshot.join(HELDOUT_DIR);
        let build_profile = match &host {
            Some(host) => Some(sandbox::render_profile(&ProfileSpec {
                host,
                target_root: &suite_dir,
                toolchain: true,
                write_dirs: &[cargo_home.clone(), target_dir.clone()],
                write_files: &[],
            })?),
            None => None,
        };
        let builder = Runner {
            cwd: snapshot_root.clone(),
            allowlist: vec!["cargo".into(), "cc".into(), "rustc".into()],
            timeout: BUILD_TIMEOUT,
            max_output: crate::exec::DEFAULT_MAX_OUTPUT,
            tool_profile: build_profile.clone(),
        };
        let vendor_str = crate::path_str(&vendor)?;
        builder.tool_with_env(
            &scorer_build_argv(&snapshot_root, &target_dir, vendor_str)?,
            build_profile.as_deref(),
            &[("CARGO_HOME", cargo_home.as_os_str())],
        )?;
        let tools = Runner {
            cwd: suite_dir.clone(),
            allowlist: vec!["cc".into(), "rustc".into()],
            timeout: Duration::from_secs(120),
            max_output: crate::exec::DEFAULT_MAX_OUTPUT,
            tool_profile: None, // rendered per case (its run root is the write dir)
        };
        let mut environment = environment(&tools)?;
        let sanitized = match asan_runtime(&tools, &suite_dir, host.as_ref())? {
            None if !cfg!(target_os = "macos") => Sanitized::Skipped("unsupported-platform"),
            None => Sanitized::Skipped("runtime-not-found"),
            Some(runtime) => match host_triple(&tools) {
                None => Sanitized::Skipped("unsupported-platform"),
                Some(triple) => {
                    let asan_dir = bench.join(format!("scorer-target-{}-asan", &lock_key[..12]));
                    std::fs::create_dir_all(&asan_dir).map_err(|e| Error::io(&asan_dir, e))?;
                    let profile = match &host {
                        Some(host) => Some(sandbox::render_profile(&ProfileSpec {
                            host,
                            target_root: &suite_dir,
                            toolchain: true,
                            write_dirs: &[cargo_home.clone(), asan_dir.clone()],
                            write_files: &[],
                        })?),
                        None => None,
                    };
                    // Checked by `asan_runtime`: `[A-Za-z0-9._/+-]` only, so the
                    // path cannot break out of the TOML string (R-A6).
                    let runtime_str = crate::path_str(&runtime)?;
                    let runtime_dir = crate::path_str(runtime.parent().unwrap_or(&runtime))?;
                    // `--target` scopes the flags to TARGET artifacts: build
                    // scripts and proc-macros are not linked with the runtime.
                    let mut argv = scorer_build_argv(&snapshot_root, &asan_dir, vendor_str)?;
                    argv.extend([
                        "--target".into(),
                        triple.clone(),
                        "--config".into(),
                        format!(
                            "target.{triple}.rustflags=[\"-C\", \"link-arg={runtime_str}\", \
                         \"-C\", \"link-arg=-Wl,-rpath,{runtime_dir}\"]"
                        ),
                    ]);
                    builder.tool_with_env(
                        &argv,
                        profile.as_deref(),
                        &[("CARGO_HOME", cargo_home.as_os_str())],
                    )?;
                    Sanitized::Ready {
                        runners: asan_dir.join(&triple).join("release"),
                    }
                }
            },
        };
        environment.push(match &sanitized {
            Sanitized::Ready { .. } => "sanitized-pass: asan+bounds-safety".to_string(),
            Sanitized::Skipped(reason) => format!("sanitized-pass: skipped ({reason})"),
        });
        Ok(Scorer {
            _lock: bench_lock,
            suite_dir,
            snapshot: snapshot_root,
            runners: target_dir.join("release"),
            host,
            tools,
            environment,
            sanitized,
        })
    }

    /// The environment fingerprint scores are bound to (R10).
    pub fn environment(&self) -> &[String] {
        &self.environment
    }

    /// Score one case: the C baseline always; the verified Rust staticlib and
    /// an unverified candidate staticlib when given. Vectors are the regular
    /// `*.json` files of the case's SNAPSHOT `test_vectors/`, by name.
    pub fn score_case(
        &self,
        case: &SuiteCase,
        c: &CSide,
        rust: Option<&Path>,
        candidate: Option<&Path>,
    ) -> Result<CaseRuns, Error> {
        let vectors_dir = self.snapshot.join(&case.path).join("test_vectors");
        if !vectors_dir.is_dir() {
            // No scorable vector upstream (e.g. only a `.bak` file): nothing
            // was vendored, and the case is `unscorable`, not an error.
            return Ok(CaseRuns {
                vectors: Vec::new(),
                sanitized_build: None,
            });
        }
        let mut names: Vec<String> = std::fs::read_dir(&vectors_dir)
            .map_err(|e| Error::io(&vectors_dir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                std::fs::symlink_metadata(e.path())
                    .map(|m| m.file_type().is_file())
                    .unwrap_or(false)
            })
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.ends_with(".json"))
            .collect();
        names.sort();
        let runner = self.runners.join(&case.runner);
        let runner = runner.canonicalize().map_err(|e| Error::io(&runner, e))?;

        let c_results = self
            .side(case, &runner, &vectors_dir, &names, Side::C(c))?
            .0;
        let rust_results = match rust {
            Some(lib) => Some(
                self.side(case, &runner, &vectors_dir, &names, Side::Rust(lib))?
                    .0,
            ),
            None => None,
        };
        let cand_results = match candidate {
            Some(lib) => Some(
                self.side(case, &runner, &vectors_dir, &names, Side::Rust(lib))?
                    .0,
            ),
            None => None,
        };
        // R-A1: only a vector the plain C passed and a Rust side did not (on
        // a real outcome) can change a class, so only it is re-run.
        let needs: Vec<usize> = (0..names.len())
            .filter(|&i| {
                c_results[i] == "pass"
                    && [&rust_results, &cand_results].iter().any(|side| {
                        side.as_ref()
                            .is_some_and(|r| r[i] != "pass" && !is_infra_result(&r[i]))
                    })
            })
            .collect();
        let mut sanitized: Vec<Option<String>> = vec![None; names.len()];
        let mut sanitized_build = None;
        if let (Sanitized::Ready { runners }, false) = (&self.sanitized, needs.is_empty()) {
            let asan_runner = runners.join(&case.runner);
            let asan_runner = asan_runner
                .canonicalize()
                .map_err(|e| Error::io(&asan_runner, e))?;
            let subset: Vec<String> = needs.iter().map(|&i| names[i].clone()).collect();
            let (results, build) = self.side(
                case,
                &asan_runner,
                &vectors_dir,
                &subset,
                Side::CSanitized(c),
            )?;
            for (&i, result) in needs.iter().zip(results) {
                sanitized[i] = Some(result);
            }
            sanitized_build = Some(build.to_string());
        }
        let vectors = names
            .iter()
            .enumerate()
            .map(|(i, name)| VectorRun {
                name: name.clone(),
                c: c_results[i].clone(),
                rust: rust_results.as_ref().map(|r| r[i].clone()),
                candidate: cand_results.as_ref().map(|r| r[i].clone()),
                c_sanitized: sanitized[i].take(),
            })
            .collect();
        Ok(CaseRuns {
            vectors,
            sanitized_build,
        })
    }

    /// Build one side's dylib into a fresh run root and run every vector.
    /// Also returns which build ran: `plain` for the C and Rust sides; for
    /// the sanitized C `asan+bounds-safety`, `asan`, or `none`.
    fn side(
        &self,
        case: &SuiteCase,
        runner: &Path,
        vectors_dir: &Path,
        names: &[String],
        side: Side<'_>,
    ) -> Result<(Vec<String>, &'static str), Error> {
        let run = RunRoot::create(case.name())?;
        let root = run.case_root();
        let (lib_dir, rust_flag) = match side {
            Side::C(_) | Side::CSanitized(_) => (root.join("build-ninja"), false),
            Side::Rust(_) => (root.join("translated_rust/target/release"), true),
        };
        let sanitized = matches!(side, Side::CSanitized(_));
        std::fs::create_dir_all(&lib_dir).map_err(|e| Error::io(&lib_dir, e))?;
        let tv = root.join("test_vectors");
        std::fs::create_dir_all(&tv).map_err(|e| Error::io(&tv, e))?;
        for name in names {
            let dst = tv.join(name);
            std::fs::copy(vectors_dir.join(name), &dst).map_err(|e| Error::io(&dst, e))?;
        }
        let dylib = lib_dir.join(format!("lib{}.dylib", case.library));
        let argv_with = |extra: &[&str]| -> Result<Vec<String>, Error> {
            let mut argv: Vec<String> = vec!["cc".into()];
            match side {
                Side::C(c) | Side::CSanitized(c) => {
                    let flags: &[&str] = if sanitized {
                        &SANITIZED_CFLAGS
                    } else {
                        &BASELINE_CFLAGS
                    };
                    argv.extend(flags.iter().chain(extra).map(|s| (*s).to_string()));
                    for dir in &c.include_dirs {
                        argv.push(format!("-I{}", crate::path_str(dir)?));
                    }
                    for src in &c.sources {
                        argv.push(crate::path_str(src)?.to_string());
                    }
                }
                Side::Rust(lib) => {
                    argv.push("-shared".into());
                    argv.push(format!("-Wl,-force_load,{}", crate::path_str(lib)?));
                }
            }
            argv.push("-lm".into());
            argv.push("-o".into());
            argv.push(crate::path_str(&dylib)?.to_string());
            Ok(argv)
        };
        let profile = match &self.host {
            Some(host) => Some(sandbox::render_profile(&ProfileSpec {
                host,
                target_root: &self.suite_dir,
                toolchain: true,
                write_dirs: std::slice::from_ref(&lib_dir),
                write_files: &[],
            })?),
            None => None,
        };
        // R-A13: the sanitized C with bounds-safety when it compiles, else
        // ASan alone; `bounds` says which (only then can a SIGTRAP be a
        // bounds check).
        let mut bounds = false;
        let mut built = Err(());
        if sanitized {
            built = self
                .tools
                .tool_with_env(&argv_with(&[BOUNDS_SAFETY_CFLAG])?, profile.as_deref(), &[])
                .map(drop)
                .map_err(drop);
            bounds = built.is_ok();
        }
        if built.is_err() {
            built = self
                .tools
                .tool_with_env(&argv_with(&[])?, profile.as_deref(), &[])
                .map(drop)
                .map_err(drop);
        }
        if built.is_err() {
            // A side that does not even link is a failure of every vector,
            // not a harness error (e.g. a candidate lacking a symbol).
            return Ok((
                names
                    .iter()
                    .map(|_| "fail:dylib-build".to_string())
                    .collect(),
                "none",
            ));
        }
        let build = match (sanitized, bounds) {
            (false, _) => "plain",
            (true, true) => "asan+bounds-safety",
            (true, false) => "asan",
        };
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let vector = tv.join(name);
            let mode = if sanitized {
                Report::Sanitized { bounds }
            } else {
                Report::Plain
            };
            let mut result = self.run_vector(runner, &root, &vector, rust_flag, mode)?;
            if result == "timeout" {
                result = self.run_vector(runner, &root, &vector, rust_flag, mode)?;
            }
            out.push(result);
        }
        Ok((out, build))
    }

    /// One confined runner invocation for one vector.
    fn run_vector(
        &self,
        runner: &Path,
        root: &Path,
        vector: &Path,
        rust: bool,
        mode: Report,
    ) -> Result<String, Error> {
        let tmp = RunRoot::create("tmp")?;
        let report = tmp.path().join("report.json");
        let mut args: Vec<String> = vec![
            "--log-level".into(),
            "none".into(),
            "--test-root-dir".into(),
            crate::path_str(root)?.into(),
            "--output".into(),
            crate::path_str(&report)?.into(),
            "-v".into(),
            crate::path_str(vector)?.into(),
        ];
        if rust {
            args.push("--rust".into());
        }
        args.push("lib".into());
        let profile = match &self.host {
            Some(host) => Some(sandbox::render_run_profile(&RunSpec {
                host,
                target_root: &self.suite_dir,
                bin: runner,
                read_files: &[],
                tmpdir: tmp.path(),
            })?),
            None => None,
        };
        let exec = Runner {
            cwd: tmp.path().to_path_buf(),
            allowlist: Vec::new(),
            timeout: VECTOR_TIMEOUT,
            max_output: crate::exec::DEFAULT_MAX_OUTPUT,
            tool_profile: None,
        };
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut env = vec![("TMPDIR", tmp.path().as_os_str())];
        if matches!(mode, Report::Sanitized { .. }) {
            env.push(("ASAN_OPTIONS", std::ffi::OsStr::new(ASAN_OPTIONS)));
        }
        let status = exec.built_status(runner, &arg_refs, profile.as_deref(), &env)?;
        Ok(match status {
            RunStatus::TimedOut => "timeout".into(),
            RunStatus::Killed => "fail:runner-killed".into(),
            RunStatus::Exited(code) if code == 0 || code == 1 => match mode {
                Report::Plain => parse_report(&report),
                Report::Sanitized { bounds } => parse_sanitized_report(&report, bounds),
            },
            RunStatus::Exited(code) => format!("fail:runner-exit-{code}"),
        })
    }
}

/// An exclusive `flock(2)` on `.bench/LOCK` (a permanent, harness-owned,
/// gitignored file), held on the open file description for the life of the
/// value: the kernel releases it when the holder exits or dies, so a
/// crashed run never leaves a stale lock (docs/CLI-HARDENING.md §1).
#[derive(Debug)]
struct BenchLock(#[allow(dead_code)] std::fs::File);

impl BenchLock {
    fn acquire(path: &Path) -> Result<BenchLock, Error> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        match file.try_lock() {
            Ok(()) => Ok(BenchLock(file)),
            Err(std::fs::TryLockError::WouldBlock) => Err(Error::Invariant(format!(
                "another bench scoring run holds {} (runs share the verified snapshot); wait for \
                 it to finish",
                path.display()
            ))),
            Err(std::fs::TryLockError::Error(e)) => Err(Error::io(path, e)),
        }
    }
}

/// How a vector run's report is read.
#[derive(Debug, Clone, Copy)]
enum Report {
    Plain,
    /// The sanitized C; `bounds` = built with [`BOUNDS_SAFETY_CFLAG`].
    Sanitized {
        bounds: bool,
    },
}

enum Side<'a> {
    C(&'a CSide),
    /// The C built with [`SANITIZED_CFLAGS`], run by the ASan-linked runner.
    CSanitized(&'a CSide),
    Rust(&'a Path),
}

/// How a built binary's run ended (the scorer needs the exit CODE: cando
/// exits 1 when a vector fails, which is a normal outcome, not an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunStatus {
    /// Exited with this code.
    Exited(i32),
    /// Terminated by a signal (or no code).
    Killed,
    /// Wall-clock limit hit (process group killed).
    TimedOut,
}

/// Parse a cando report: a JSON object with exactly one entry whose value
/// has a `result` in the closed set.
fn parse_report(path: &Path) -> String {
    match read_report(path) {
        Ok((result, _)) => result_string(&result),
        Err(infra) => infra,
    }
}

/// A sanitized run's report (R-A3, R-A14): `clean` on a pass; on any failure
/// but a timeout, `ub:<kind>` when the child's captured stderr carries a
/// matching ASan ERROR/SUMMARY pair of an allow-listed kind (`sanitizer:
/// <kind>` for another kind); else, for a `bounds` build, `ub:bounds-safety-
/// trap` when cando's `UnknownFailure` wait status says SIGTRAP; else the
/// plain `fail:<ResultType>` — never excused.
fn parse_sanitized_report(path: &Path, bounds: bool) -> String {
    let (result, entry) = match read_report(path) {
        Ok(parsed) => parsed,
        Err(infra) => return infra,
    };
    match result.as_str() {
        "Pass" => return "clean".into(),
        "Skip" | "Timeout" => return result_string(&result),
        _ => {}
    }
    let stderr = entry
        .get("output")
        .and_then(|o| o.get("stderr"))
        .and_then(|e| e.as_str())
        .unwrap_or("");
    match asan_kind(stderr) {
        Some(kind) if EXCUSED_ASAN_KINDS.contains(&kind.as_str()) => format!("ub:{kind}"),
        Some(kind) => format!("sanitizer:{kind}"),
        None if bounds && result == "UnknownFailure" && trapped(&entry) => {
            "ub:bounds-safety-trap".into()
        }
        None => result_string(&result),
    }
}

/// Whether cando's raw `wait_status` says the child was killed by SIGTRAP
/// (`WIFSIGNALED`: the low 7 bits are the signal; the 0x80 core bit aside).
fn trapped(entry: &serde_json::Value) -> bool {
    entry
        .get("wait_status")
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|status| status & 0x7f == SIGTRAP)
}

/// The report's single entry: its `result` (in the closed set) and the entry
/// itself; `Err` = the infra result string.
fn read_report(path: &Path) -> Result<(String, serde_json::Value), String> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Err("fail:no-report".into());
    };
    if !meta.file_type().is_file() || meta.len() > MAX_REPORT_BYTES {
        return Err("fail:bad-report".into());
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Err("fail:bad-report".into());
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        return Err("fail:bad-report".into());
    };
    if map.len() != 1 {
        return Err("fail:bad-report".into());
    }
    let Some(entry) = map.into_iter().next().map(|(_, v)| v) else {
        return Err("fail:bad-report".into());
    };
    let result = entry
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    if RESULT_TYPES.contains(&result.as_str()) {
        Ok((result, entry))
    } else {
        Err("fail:bad-report".into())
    }
}

/// The harness result string of a cando `result` in the closed set.
fn result_string(result: &str) -> String {
    match result {
        "Pass" => "pass".into(),
        "Skip" => "skip".into(),
        r => format!("fail:{r}"),
    }
}

/// The kind of an ASan report in `stderr`: a line `==<pid>==ERROR:
/// AddressSanitizer: <kind> …` AND a line `SUMMARY: AddressSanitizer:
/// <kind> …` naming the same kind. Kinds are `[a-z0-9-]`, at most
/// [`MAX_KIND_BYTES`]; a report whose two lines disagree or whose kind is
/// malformed reads `malformed-report` (recorded, never excused).
fn asan_kind(stderr: &str) -> Option<String> {
    let kind_of = |rest: &str| -> Option<String> {
        let kind = rest.split_whitespace().next()?;
        let ok = !kind.is_empty()
            && kind.len() <= MAX_KIND_BYTES
            && kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        Some(if ok { kind } else { "malformed-report" }.to_string())
    };
    let error = stderr.lines().find_map(|line| {
        let rest = line.strip_prefix("==")?;
        let (pid, rest) = rest.split_once("==")?;
        if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        kind_of(rest.strip_prefix("ERROR: AddressSanitizer: ")?)
    })?;
    let summary = stderr
        .lines()
        .find_map(|line| kind_of(line.strip_prefix("SUMMARY: AddressSanitizer: ")?));
    Some(if summary.as_deref() == Some(error.as_str()) {
        error
    } else {
        "malformed-report".to_string()
    })
}

/// `cargo build` of the scorer workspace (frozen, offline, vendored) into
/// `target_dir`.
fn scorer_build_argv(
    snapshot_root: &Path,
    target_dir: &Path,
    vendor_str: &str,
) -> Result<Vec<String>, Error> {
    Ok(vec![
        "cargo".into(),
        "build".into(),
        "--release".into(),
        "--frozen".into(),
        "--workspace".into(),
        "--bins".into(),
        "--manifest-path".into(),
        crate::path_str(&snapshot_root.join("Cargo.toml"))?.into(),
        "--target-dir".into(),
        crate::path_str(target_dir)?.into(),
        "--config".into(),
        "source.crates-io.replace-with=\"vendored-sources\"".into(),
        "--config".into(),
        format!("source.vendored-sources.directory=\"{vendor_str}\""),
    ])
}

/// The ASan runtime dylib, when this toolchain has one that is safe to
/// splice into cargo config (R-A6): `cc -print-file-name` echoes the bare
/// name when it finds nothing, so the answer must be an absolute, existing,
/// canonical file outside the home dir and the suite, made of
/// `[A-Za-z0-9._/+-]` only. `None` = no usable runtime (macOS only).
fn asan_runtime(
    tools: &Runner,
    suite_dir: &Path,
    host: Option<&HostDirs>,
) -> Result<Option<PathBuf>, Error> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    let out = tools.tool_with_env(
        &["cc".into(), format!("-print-file-name={ASAN_RUNTIME}")],
        None,
        &[],
    )?;
    let printed = String::from_utf8_lossy(&out).trim().to_string();
    let path = PathBuf::from(&printed);
    if !path.is_absolute() {
        return Ok(None);
    }
    let Ok(canonical) = path.canonicalize() else {
        return Ok(None);
    };
    let home = match host {
        Some(h) => Some(h.home.clone()),
        None => std::env::var_os("HOME").map(PathBuf::from),
    };
    let safe = canonical.is_file()
        && canonical.to_str().is_some_and(|s| {
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/+-".contains(&b))
        })
        && !canonical.starts_with(suite_dir)
        && !home.is_some_and(|h| canonical.starts_with(h));
    Ok(safe.then_some(canonical))
}

/// The host target triple (`rustc -vV`'s `host:` line), `[a-z0-9_-]` only;
/// `None` when rustc prints none usable (the pass is then skipped).
fn host_triple(tools: &Runner) -> Option<String> {
    let out = tools
        .tool_with_env(&["rustc".into(), "-vV".into()], None, &[])
        .ok()?;
    String::from_utf8_lossy(&out)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::trim)
        .filter(|t| {
            !t.is_empty()
                && t.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
        .map(str::to_string)
}

/// The environment fingerprint (R10): toolchain, OS, arch, sandbox, flags.
fn environment(tools: &Runner) -> Result<Vec<String>, Error> {
    let first = |argv: &[&str]| -> Result<String, Error> {
        let argv: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
        let out = tools.tool_with_env(&argv, None, &[])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string())
    };
    let os = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .ok()
        .and_then(|plist| {
            let key = plist.find("<key>ProductVersion</key>")?;
            let rest = &plist[key..];
            let start = rest.find("<string>")? + "<string>".len();
            let end = rest[start..].find("</string>")?;
            Some(format!("macOS {}", &rest[start..start + end]))
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    Ok(vec![
        first(&["rustc", "-V"])?,
        first(&["cc", "--version"])?,
        format!("os: {os} {}", std::env::consts::ARCH),
        format!("sandbox: {}", crate::sandbox_mode()),
        format!("baseline: cc {}", BASELINE_CFLAGS.join(" ")),
    ])
}

/// A fresh dir under the system temp dir, removed on drop. `case_root()` is
/// `<dir>/<case-name>`: cando derives a default library name from the
/// basename of `--test-root-dir`, so the root carries the case's name.
struct RunRoot {
    dir: PathBuf,
    name: String,
}

impl RunRoot {
    fn create(name: &str) -> Result<RunRoot, Error> {
        let base_raw = std::env::temp_dir();
        let base = base_raw
            .canonicalize()
            .map_err(|e| Error::io(&base_raw, e))?;
        for _ in 0..1000 {
            let dir = base.join(format!(
                "ruharness-score-{}-{}",
                std::process::id(),
                RUN_COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            match std::fs::create_dir(&dir) {
                Ok(()) => {
                    return Ok(RunRoot {
                        dir,
                        name: name.to_string(),
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io(&dir, e)),
            }
        }
        Err(Error::Invariant(
            "could not create a scoring run dir".into(),
        ))
    }
    fn path(&self) -> &Path {
        &self.dir
    }
    fn case_root(&self) -> PathBuf {
        self.dir.join(&self.name)
    }
}

impl Drop for RunRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(text: &str) -> String {
        parsed(text, parse_report)
    }

    fn parsed(text: &str, parse: fn(&Path) -> String) -> String {
        let dir = std::env::temp_dir().join(format!(
            "ruharness-report-{}-{}",
            std::process::id(),
            RUN_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.json");
        std::fs::write(&p, text).unwrap();
        let out = parse(&p);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    /// A sanitized report whose child stderr is `stderr`.
    fn sanitized(result: &str, stderr: &str) -> String {
        let entry = serde_json::json!({
            "/x/1.json": {"result": result, "wait_status": 6,
                          "output": {"stdout": "", "stderr": stderr}}
        });
        parsed(&entry.to_string(), |p| parse_sanitized_report(p, false))
    }

    const ASAN: &str = "=================================================================\n\
        ==4242==ERROR: AddressSanitizer: stack-buffer-overflow on address 0x16 at pc 0x1\n\
        WRITE of size 4 at 0x16 thread T0\n\
        SUMMARY: AddressSanitizer: stack-buffer-overflow (libdecorrelate.dylib:arm64+0x3f4)\n";

    /// docs/ORACLE-HARDENING.md R-A2/R-A3: only a matching ERROR/SUMMARY pair
    /// of an allow-listed kind on a non-timeout failure excuses a vector.
    #[test]
    fn sanitized_reports_excuse_only_allow_listed_asan_errors() {
        assert_eq!(
            sanitized("UnknownFailure", ASAN),
            "ub:stack-buffer-overflow"
        );
        assert_eq!(
            sanitized("SegmentationFault", ASAN),
            "ub:stack-buffer-overflow"
        );
        assert_eq!(sanitized("Pass", ASAN), "clean");
        assert_eq!(
            sanitized("Timeout", ASAN),
            "fail:Timeout",
            "a timeout is never excused"
        );
        assert_eq!(sanitized("UnknownFailure", ""), "fail:UnknownFailure");
        // A kind outside the allow-list is recorded, not excused.
        let other = ASAN.replace("stack-buffer-overflow", "alloc-dealloc-mismatch");
        assert_eq!(
            sanitized("UnknownFailure", &other),
            "sanitizer:alloc-dealloc-mismatch"
        );
        // Forgery-shaped reports: a SUMMARY alone, a kind mismatch, a bad pid.
        assert_eq!(
            sanitized(
                "UnknownFailure",
                "SUMMARY: AddressSanitizer: stack-buffer-overflow x\n"
            ),
            "fail:UnknownFailure"
        );
        let mismatch = ASAN.replace(
            "SUMMARY: AddressSanitizer: stack",
            "SUMMARY: AddressSanitizer: heap",
        );
        assert_eq!(
            sanitized("UnknownFailure", &mismatch),
            "sanitizer:malformed-report"
        );
        let bad_pid = ASAN.replace("==4242==", "==pid==");
        assert_eq!(sanitized("UnknownFailure", &bad_pid), "fail:UnknownFailure");
        let shouting = ASAN.replace("stack-buffer-overflow", "STACK");
        assert_eq!(
            sanitized("UnknownFailure", &shouting),
            "sanitizer:malformed-report"
        );
        // Infra results pass through unchanged (a PROBLEM upstream).
        assert_eq!(
            parsed("not json", |p| parse_sanitized_report(p, true)),
            "fail:bad-report"
        );
    }

    /// §A.2 / R-A14: a SIGTRAP is a bounds-safety check only in a build that
    /// has bounds-safety; SIGABRT (abort) or an exit status never is.
    #[test]
    fn a_sigtrap_excuses_only_under_bounds_safety() {
        let run = |status: i64, bounds: bool| {
            let entry = serde_json::json!({
                "/x/1.json": {"result": "UnknownFailure", "wait_status": status,
                              "output": {"stdout": "", "stderr": ""}}
            });
            let parse: fn(&Path) -> String = if bounds {
                |p| parse_sanitized_report(p, true)
            } else {
                |p| parse_sanitized_report(p, false)
            };
            parsed(&entry.to_string(), parse)
        };
        assert_eq!(run(5, true), "ub:bounds-safety-trap");
        assert_eq!(
            run(5 | 0x80, true),
            "ub:bounds-safety-trap",
            "core-dump bit"
        );
        assert_eq!(run(5, false), "fail:UnknownFailure");
        assert_eq!(
            run(6, true),
            "fail:UnknownFailure",
            "SIGABRT is not a bounds check"
        );
        assert_eq!(
            run(5 << 8, true),
            "fail:UnknownFailure",
            "exit status 5, not a signal"
        );
    }

    #[test]
    fn reports_parse_as_a_closed_enum() {
        assert_eq!(
            report(r#"{"/x/1.json":{"result":"Pass","output":{}}}"#),
            "pass"
        );
        assert_eq!(report(r#"{"/x/1.json":{"result":"Skip"}}"#), "skip");
        assert_eq!(
            report(r#"{"/x/1.json":{"result":"VectorComparisonFailed","diff":"d"}}"#),
            "fail:VectorComparisonFailed"
        );
        assert_eq!(
            report(r#"{"/x/1.json":{"result":"Totally Fine"}}"#),
            "fail:bad-report"
        );
        assert_eq!(
            report(r#"{"a":{"result":"Pass"},"b":{"result":"Pass"}}"#),
            "fail:bad-report"
        );
        assert_eq!(report("not json"), "fail:bad-report");
        assert_eq!(
            parse_report(Path::new("/nonexistent/r.json")),
            "fail:no-report"
        );
    }
}
