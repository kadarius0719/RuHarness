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

use crate::exec::Runner;
use crate::sandbox::{self, HostDirs, ProfileSpec, RunSpec};
use harness_core::bench::{CorpusLock, Suite, SuiteCase, HELDOUT_DIR};
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

/// One vector's three results (`None` = that side was not scored).
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
            &[
                "cargo".into(),
                "build".into(),
                "--release".into(),
                "--frozen".into(),
                "--workspace".into(),
                "--bins".into(),
                "--manifest-path".into(),
                crate::path_str(&snapshot_root.join("Cargo.toml"))?.into(),
                "--target-dir".into(),
                crate::path_str(&target_dir)?.into(),
                "--config".into(),
                "source.crates-io.replace-with=\"vendored-sources\"".into(),
                "--config".into(),
                format!("source.vendored-sources.directory=\"{vendor_str}\""),
            ],
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
        let environment = environment(&tools)?;
        Ok(Scorer {
            _lock: bench_lock,
            suite_dir,
            snapshot: snapshot_root,
            runners: target_dir.join("release"),
            host,
            tools,
            environment,
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
    ) -> Result<Vec<VectorRun>, Error> {
        let vectors_dir = self.snapshot.join(&case.path).join("test_vectors");
        if !vectors_dir.is_dir() {
            // No scorable vector upstream (e.g. only a `.bak` file): nothing
            // was vendored, and the case is `unscorable`, not an error.
            return Ok(Vec::new());
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

        let c_results = self.side(case, &runner, &vectors_dir, &names, Side::C(c))?;
        let rust_results = match rust {
            Some(lib) => Some(self.side(case, &runner, &vectors_dir, &names, Side::Rust(lib))?),
            None => None,
        };
        let cand_results = match candidate {
            Some(lib) => Some(self.side(case, &runner, &vectors_dir, &names, Side::Rust(lib))?),
            None => None,
        };
        Ok(names
            .iter()
            .enumerate()
            .map(|(i, name)| VectorRun {
                name: name.clone(),
                c: c_results[i].clone(),
                rust: rust_results.as_ref().map(|r| r[i].clone()),
                candidate: cand_results.as_ref().map(|r| r[i].clone()),
            })
            .collect())
    }

    /// Build one side's dylib into a fresh run root and run every vector.
    fn side(
        &self,
        case: &SuiteCase,
        runner: &Path,
        vectors_dir: &Path,
        names: &[String],
        side: Side<'_>,
    ) -> Result<Vec<String>, Error> {
        let run = RunRoot::create(case.name())?;
        let root = run.case_root();
        let (lib_dir, rust_flag) = match side {
            Side::C(_) => (root.join("build-ninja"), false),
            Side::Rust(_) => (root.join("translated_rust/target/release"), true),
        };
        std::fs::create_dir_all(&lib_dir).map_err(|e| Error::io(&lib_dir, e))?;
        let tv = root.join("test_vectors");
        std::fs::create_dir_all(&tv).map_err(|e| Error::io(&tv, e))?;
        for name in names {
            let dst = tv.join(name);
            std::fs::copy(vectors_dir.join(name), &dst).map_err(|e| Error::io(&dst, e))?;
        }
        let dylib = lib_dir.join(format!("lib{}.dylib", case.library));
        let mut argv: Vec<String> = vec!["cc".into()];
        match side {
            Side::C(c) => {
                argv.extend(BASELINE_CFLAGS.iter().map(|s| (*s).to_string()));
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
        let built = self.tools.tool_with_env(&argv, profile.as_deref(), &[]);
        if built.is_err() {
            // A side that does not even link is a failure of every vector,
            // not a harness error (e.g. a candidate lacking a symbol).
            return Ok(names
                .iter()
                .map(|_| "fail:dylib-build".to_string())
                .collect());
        }
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let vector = tv.join(name);
            let mut result = self.run_vector(runner, &root, &vector, rust_flag)?;
            if result == "timeout" {
                result = self.run_vector(runner, &root, &vector, rust_flag)?;
            }
            out.push(result);
        }
        Ok(out)
    }

    /// One confined runner invocation for one vector.
    fn run_vector(
        &self,
        runner: &Path,
        root: &Path,
        vector: &Path,
        rust: bool,
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
        let env = [("TMPDIR", tmp.path().as_os_str())];
        let status = exec.built_status(runner, &arg_refs, profile.as_deref(), &env)?;
        Ok(match status {
            RunStatus::TimedOut => "timeout".into(),
            RunStatus::Killed => "fail:runner-killed".into(),
            RunStatus::Exited(code) if code == 0 || code == 1 => parse_report(&report),
            RunStatus::Exited(code) => format!("fail:runner-exit-{code}"),
        })
    }
}

/// An exclusive lock file (`create_new`), removed on drop. A crashed run
/// leaves it behind; the error says so.
#[derive(Debug)]
struct BenchLock(PathBuf);

impl BenchLock {
    fn acquire(path: &Path) -> Result<BenchLock, Error> {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => Ok(BenchLock(path.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Error::Invariant(format!(
                    "another bench scoring run holds {} (runs share the verified snapshot); if no \
                 run is active it is stale from a crash — remove it",
                    path.display()
                )))
            }
            Err(e) => Err(Error::io(path, e)),
        }
    }
}

impl Drop for BenchLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

enum Side<'a> {
    C(&'a CSide),
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
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return "fail:no-report".into();
    };
    if !meta.file_type().is_file() || meta.len() > MAX_REPORT_BYTES {
        return "fail:bad-report".into();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return "fail:bad-report".into();
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        return "fail:bad-report".into();
    };
    if map.len() != 1 {
        return "fail:bad-report".into();
    }
    let result = map
        .values()
        .next()
        .and_then(|v| v.get("result"))
        .and_then(|r| r.as_str())
        .unwrap_or("");
    match result {
        "Pass" => "pass".into(),
        "Skip" => "skip".into(),
        r if RESULT_TYPES.contains(&r) => format!("fail:{r}"),
        _ => "fail:bad-report".into(),
    }
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
        let dir = std::env::temp_dir().join(format!(
            "ruharness-report-{}-{}",
            std::process::id(),
            RUN_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.json");
        std::fs::write(&p, text).unwrap();
        let out = parse_report(&p);
        let _ = std::fs::remove_dir_all(&dir);
        out
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
