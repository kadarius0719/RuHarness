//! Child-process discipline for the oracle (docs/SCHEMAS.md "Trust
//! boundaries"). Every child — allowlisted tools and the binaries the oracle
//! just built — is spawned through this module, which guarantees:
//!
//! - an explicit argv (never a shell), working directory pinned by the caller;
//! - a **scrubbed environment**: `env_clear()` plus a fixed list of variables
//!   copied from the parent when set ([`TOOL_ENV`] for tools, [`BUILT_ENV`] —
//!   `PATH` only — for built binaries);
//! - a **wall-clock timeout**: `spawn` + `try_wait` polling + `kill`;
//! - **deadlock-free output capture**: stdout and stderr are drained on
//!   dedicated threads while the parent polls, so a child that prints more
//!   than a pipe buffer can never block against a parent that is waiting;
//! - a **bounded capture**: output past a cap kills the child (a candidate
//!   that prints forever must not exhaust the harness's memory);
//! - optional wrapping in the sandbox ([`crate::sandbox`]).

use crate::sandbox;
use harness_core::error::Error;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Variables copied from the parent (when set) into tool children. `PATH`
/// is filtered ([`child_path`]); `RUSTUP_TOOLCHAIN` is never copied as it
/// is but pinned ([`pinned_toolchain`]).
pub(crate) const TOOL_ENV: &[&str] = &["PATH", "HOME", "TMPDIR", "CARGO_HOME", "RUSTUP_HOME"];

/// Variables every tool child gets with a fixed value: `SOURCE_DATE_EPOCH`
/// pins `__DATE__`, `__TIME__` and `__TIMESTAMP__`, so two builds of the same
/// C print the same whenever they ran (review M5: the all-C and the mixed
/// whole programs, and the map's plain and probed copies, are separate
/// compiles); `RUSTUP_AUTO_INSTALL=0` makes rustup refuse a toolchain that
/// is not installed instead of downloading it (the sandbox never needs the
/// network, and the harness never installs anything).
pub(crate) const TOOL_FIXED_ENV: &[(&str, &str)] =
    &[("SOURCE_DATE_EPOCH", "0"), ("RUSTUP_AUTO_INSTALL", "0")];

/// Variables copied from the parent (when set) into built binaries (`PATH`
/// filtered by [`child_path`]).
pub(crate) const BUILT_ENV: &[&str] = &["PATH"];

/// The toolchain every tool child is pinned to through `RUSTUP_TOOLCHAIN`
/// (docs/PROJECT-MAP-DESIGN.md §3.7): the value rustup gave the harness when
/// there is one, else `stable` (RuHarness's own `rust-toolchain.toml`'s
/// channel). Set in the child's environment, it outranks every
/// `rust-toolchain.toml`, so a project's own (which could name a `path` to
/// its own `cargo`) is never read.
pub(crate) fn pinned_toolchain() -> std::ffi::OsString {
    std::env::var_os("RUSTUP_TOOLCHAIN")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "stable".into())
}

/// [`TOOL_FIXED_ENV`] and `RUSTUP_TOOLCHAIN=pinned`, then `extra` (which may
/// override them).
fn tool_env<'a>(
    pinned: &'a std::ffi::OsStr,
    extra: &[(&'a str, &'a std::ffi::OsStr)],
) -> Vec<(&'a str, &'a std::ffi::OsStr)> {
    TOOL_FIXED_ENV
        .iter()
        .map(|(k, v)| (*k, std::ffi::OsStr::new(*v)))
        .chain(std::iter::once(("RUSTUP_TOOLCHAIN", pinned)))
        .chain(extra.iter().copied())
        .collect()
}

/// The `PATH` a child gets: the parent's `raw` with only absolute entries
/// outside the project `root` (lexically and once resolved) kept — a
/// relative or empty entry would find the project's own `cc` from a working
/// folder, an entry under the root would find it anywhere. `None` when the
/// parent has no `PATH`; `/usr/bin:/bin` when no entry is left (an empty
/// `PATH` means the working folder to some `execvp`s).
pub(crate) fn child_path(
    raw: Option<std::ffi::OsString>,
    root: &Path,
) -> Option<std::ffi::OsString> {
    let raw = raw?;
    let kept: Vec<PathBuf> = std::env::split_paths(&raw)
        .filter(|p| {
            p.is_absolute()
                && !p.starts_with(root)
                && !p.canonicalize().is_ok_and(|c| c.starts_with(root))
        })
        .collect();
    if kept.is_empty() {
        return Some("/usr/bin:/bin".into());
    }
    // split_paths never yields an entry holding the separator.
    std::env::join_paths(kept).ok()
}

/// The harness's work folder (docs/PROJECT-MAP-DESIGN.md §3.7), made now:
/// [`sandbox::work_root`] of `$HOME`. Every cargo, rustc and compiler child
/// starts there.
pub(crate) fn work_dir() -> Result<PathBuf, Error> {
    let home_raw = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            Error::Invariant("HOME is not set, so the harness has no work folder".into())
        })?;
    let home = home_raw
        .canonicalize()
        .map_err(|e| Error::io(&home_raw, e))?;
    let tmpdir = std::env::var_os("TMPDIR")
        .filter(|v| !v.is_empty())
        .and_then(|v| PathBuf::from(v).canonicalize().ok());
    work_dir_at(&sandbox::work_root(&home), &home, tmpdir.as_deref())
}

/// Make the work folder `dir` (absolute, built from canonical parts) and
/// return it, refusing it when it, or a folder made for it, is a link, and
/// when it lies in a temporary folder (`/tmp`, `/private/var/folders`, or
/// `tmpdir` unless that holds `home`): anyone may make a folder there first.
pub(crate) fn work_dir_at(
    dir: &Path,
    home: &Path,
    tmpdir: Option<&Path>,
) -> Result<PathBuf, Error> {
    let temporary = [
        "/tmp",
        "/private/tmp",
        "/var/folders",
        "/private/var/folders",
    ]
    .iter()
    .map(Path::new)
    .any(|t| dir.starts_with(t))
        || tmpdir.is_some_and(|t| dir.starts_with(t) && !home.starts_with(t));
    if temporary {
        return Err(Error::Invariant(format!(
            "the harness's work folder {} lies in a temporary folder, where anyone may make it \
             first; set HOME to your own home folder",
            dir.display()
        )));
    }
    make_work_dir(dir)
}

/// Make `dir` (absolute, from canonical parts) and return it, refusing it
/// when it, or a folder above it that was made for it, is a link.
fn make_work_dir(dir: &Path) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let meta = std::fs::symlink_metadata(dir).map_err(|e| Error::io(dir, e))?;
    let canonical = dir.canonicalize().map_err(|e| Error::io(dir, e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() || canonical != dir {
        return Err(Error::Invariant(format!(
            "the harness's work folder {} is a link or passes through one; remove it and the \
             harness makes a real folder there again",
            dir.display()
        )));
    }
    Ok(canonical)
}

/// The one sentence a tool run gives when rustup has no such toolchain
/// (`RUSTUP_AUTO_INSTALL=0` makes it say so instead of downloading one).
pub(crate) fn missing_toolchain_sentence(toolchain: &str) -> String {
    format!(
        "the Rust toolchain `{toolchain}` the harness pins its builds to is not installed, and \
         the harness never installs one: install it yourself with `rustup toolchain install \
         {toolchain}`, or run the harness under a toolchain you have"
    )
}

/// The toolchain rustup's `stderr` says is not installed, if it says so.
fn missing_toolchain(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    text.lines().find_map(|line| {
        let rest = line.split("toolchain '").nth(1)?;
        let (name, tail) = rest.split_once('\'')?;
        tail.trim_start()
            .starts_with("is not installed")
            .then(|| name.to_string())
    })
}

/// Default `[oracle] timeout_secs`.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default cap on captured bytes per stream (the M0 driver prints ~180KB).
pub(crate) const DEFAULT_MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// How much of a failed child's stderr is quoted in errors and check details.
pub(crate) const STDERR_EXCERPT: usize = 8 * 1024;

/// `try_wait` polling interval of a built program or a scenario run. A
/// fixed interval narrows, but does not remove, the chance in whether a
/// forked child that outlives its leader by a few ms is kept: the group is
/// killed at the first poll after the leader exits, and where that exit
/// falls against the 50 ms grid varies run to run (review). Only an
/// unsandboxed run can fork (macOS's scenario profile denies it); a
/// blocking wait on a helper thread would remove the chance.
const POLL: Duration = Duration::from_millis(50);

/// How a run waits for its child: a tool run (the compiler, whose listings
/// end in milliseconds) polls from 1 ms doubling to 8 ms; a built program or
/// a scenario polls every [`POLL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// 1 ms, doubling to at most 8 ms.
    Tool,
    /// Every [`POLL`].
    Built,
}

impl Wait {
    fn first(self) -> Duration {
        match self {
            Wait::Tool => Duration::from_millis(1),
            Wait::Built => POLL,
        }
    }

    fn cap(self) -> Duration {
        match self {
            Wait::Tool => Duration::from_millis(8),
            Wait::Built => POLL,
        }
    }
}

/// How long to wait for the reader threads after the child is gone. They
/// normally finish at once (EOF when the child exits); the bound only
/// matters when an orphaned grandchild still holds the pipe open.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Set once the harness is being cancelled (docs/CLI-HARDENING.md §3).
/// Observed at the spawn choke point: no child is spawned after it, and a
/// child that ends after it is reported as [`Error::Interrupted`], never as
/// a [`ChildEnd`] — a SIGKILL the harness sent itself is not evidence.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// The process groups of every live child (each child leads its own group,
/// so its pid is its pgid), each with its kill order: a perf program's group
/// ([`PROGRAM_FIRST`]) before any other (its launcher's among them;
/// docs/PERF-DESIGN.md build note 7). Spawning happens under this lock, so
/// [`kill_live_process_groups`] — which keeps the lock until the process is
/// gone — can never miss a child that is about to be spawned.
static LIVE: Mutex<std::collections::BTreeSet<(u8, u32)>> =
    Mutex::new(std::collections::BTreeSet::new());

/// The kill order of a perf program's group: before every other.
pub(crate) const PROGRAM_FIRST: u8 = 0;
/// The kill order of every other child's group.
pub(crate) const OTHER: u8 = 1;

/// Cancel the harness's children: mark the harness cancelled, then SIGKILL
/// every live process group, returning how many were signalled. The
/// registry lock is deliberately NOT released (the caller terminates the
/// process next), so a spawner arriving later blocks and dies with the
/// process instead of starting a child that would outlive it.
pub fn kill_live_process_groups() -> usize {
    CANCELLED.store(true, Ordering::SeqCst);
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(unix)]
    for (_, pgid) in live.iter() {
        kill_process_group(*pgid);
    }
    let n = live.len();
    std::mem::forget(live);
    n
}

/// Whether [`kill_live_process_groups`] has run in this process.
pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// A live child's registry entry, removed on drop. Never dropped while
/// the caller holds a [`LiveLock`] (the drop takes the lock).
pub(crate) struct Registered((u8, u32));

impl Drop for Registered {
    fn drop(&mut self) {
        // After a cancellation the registry stays locked for good (see
        // `kill_live_process_groups`) and the process is on its way out:
        // nothing to unregister.
        if cancelled() {
            return;
        }
        LIVE.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// The registry, locked: spawn under it and register the child's group, so
/// a cancellation either happened before (nothing is spawned) or finds the
/// group registered.
pub(crate) struct LiveLock(std::sync::MutexGuard<'static, std::collections::BTreeSet<(u8, u32)>>);

/// Take the registry's lock; [`Error::Interrupted`] once cancelled.
pub(crate) fn live_lock() -> Result<LiveLock, Error> {
    let guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if cancelled() {
        return Err(Error::Interrupted);
    }
    Ok(LiveLock(guard))
}

impl LiveLock {
    /// Register `pgid` with its kill `order` ([`PROGRAM_FIRST`] or
    /// [`OTHER`]).
    pub(crate) fn register(&mut self, pgid: u32, order: u8) -> Registered {
        self.0.insert((order, pgid));
        Registered((order, pgid))
    }
}

/// How a child ended.
#[derive(Debug)]
pub(crate) enum ChildEnd {
    /// Exited (or was killed by a signal) on its own.
    Exited(ExitStatus),
    /// Killed by the harness at the wall-clock deadline.
    TimedOut,
    /// Killed by the harness because a stream exceeded the capture cap.
    OutputOverflow,
}

/// A tool's stdout and stderr.
pub(crate) type Streams = (Vec<u8>, Vec<u8>);

/// A finished child: how it ended plus everything captured.
#[derive(Debug)]
pub(crate) struct ChildOutput {
    /// How the child ended.
    pub end: ChildEnd,
    /// Captured stdout (complete unless the child was killed).
    pub stdout: Vec<u8>,
    /// Captured stderr (complete unless the child was killed).
    pub stderr: Vec<u8>,
}

/// What a built binary that exited cleanly printed. Both streams are the
/// observable behavior of a run: the differential oracle and driver
/// validation compare them together (M4 found a unit reporting on stderr
/// verified while wrong when only stdout was compared).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunOutput {
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
}

/// Why a built binary's run is not usable as evidence of success. Always
/// mapped to a failed [`harness_core::verdict::Check`], never a harness error.
#[derive(Debug)]
pub(crate) enum RunFailure {
    /// The wall-clock timeout expired and the binary was killed.
    TimedOut {
        /// The configured timeout.
        secs: u64,
    },
    /// Spawn failure, non-zero exit, signal, or output overflow.
    Failed(String),
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunFailure::TimedOut { secs } => write!(f, "timed out after {secs}s"),
            RunFailure::Failed(msg) => f.write_str(msg),
        }
    }
}

/// Spawns every oracle child with the guarantees in the module docs.
#[derive(Debug, Clone)]
pub(crate) struct Runner {
    /// Working directory of every built program (the target root, or the
    /// folder its run asks for).
    pub cwd: PathBuf,
    /// Working directory of every tool child (`cc`, `cargo`, `rustc`,
    /// `nm`): the harness's work folder ([`work_dir`]), never the project,
    /// so cargo never reads a project's `.cargo/config.toml`. Tools are
    /// given absolute paths only.
    pub work: PathBuf,
    /// The project root: no `PATH` entry under it reaches a child
    /// ([`child_path`]).
    pub root: PathBuf,
    /// The core-owned `[oracle] allowlist` of tool names.
    pub allowlist: Vec<String>,
    /// Wall-clock limit per child.
    pub timeout: Duration,
    /// Capture cap per stream, in bytes.
    pub max_output: usize,
    /// Sandbox profile for tool invocations (`None` = unsandboxed).
    pub tool_profile: Option<String>,
    /// `TMPDIR` for tool children, over the person's (the features map's:
    /// inside its random folder, so a compiler driver's temporaries go with
    /// it on every way out; fix pass 2's check).
    pub tool_tmpdir: Option<PathBuf>,
}

impl Runner {
    /// A runner for the project at `root` (canonical): built programs start
    /// in `root`, tools in the work folder (made now), `PATH` filtered
    /// against `root`, the default output cap, no `TMPDIR` of its own.
    pub(crate) fn new(
        root: &Path,
        allowlist: Vec<String>,
        timeout: Duration,
        tool_profile: Option<String>,
    ) -> Result<Runner, Error> {
        Ok(Runner {
            cwd: root.to_path_buf(),
            work: work_dir()?,
            root: root.to_path_buf(),
            allowlist,
            timeout,
            max_output: DEFAULT_MAX_OUTPUT,
            tool_profile,
            tool_tmpdir: None,
        })
    }

    /// A runner outside any target — the benchmark scorer's builds, the
    /// project map's compiles: one allowlist and one timeout, `read_root`
    /// the sandbox's read root (and the root `PATH` is filtered against),
    /// each call's profile given ([`Runner::tool_with_env`]) or `profile`.
    pub(crate) fn targetless(
        read_root: &Path,
        allowlist: &[&str],
        timeout: Duration,
        profile: Option<String>,
    ) -> Result<Runner, Error> {
        Runner::new(
            read_root,
            allowlist.iter().map(|a| (*a).to_string()).collect(),
            timeout,
            profile,
        )
    }

    /// The project map's runner (docs/PROJECT-MAP-DESIGN.md §3.9): a
    /// [`Runner::targetless`] over `project_root` under the map profile
    /// (when this computer has a sandbox), writing only into `fresh`, which
    /// is also its tools' `TMPDIR`. Both paths canonical.
    pub(crate) fn map(
        project_root: &Path,
        fresh: &Path,
        allowlist: &[&str],
        timeout: Duration,
    ) -> Result<Runner, Error> {
        let profile = match sandbox::sandbox_mode() {
            "sandbox-exec" => Some(sandbox::render_map_profile(&sandbox::MapSpec {
                host: &sandbox::HostDirs::from_env()?,
                project_root,
                fresh,
            })?),
            _ => None,
        };
        let mut runner = Runner::targetless(project_root, allowlist, timeout, profile)?;
        runner.tool_tmpdir = Some(fresh.to_path_buf());
        Ok(runner)
    }

    /// Run an allowlisted tool under the tool sandbox profile. Returns
    /// stdout; a non-zero exit, a timeout, or an overflow is an `Err`.
    pub(crate) fn tool(&self, argv: &[String]) -> Result<Vec<u8>, Error> {
        self.tool_with_profile(argv, self.tool_profile.as_deref())
    }

    /// [`Runner::tool`] with an explicit sandbox profile (the symbol baseline
    /// build uses one whose write set is the baseline dir, not the unit's).
    pub(crate) fn tool_with_profile(
        &self,
        argv: &[String],
        profile: Option<&str>,
    ) -> Result<Vec<u8>, Error> {
        self.tool_with_env(argv, profile, &[])
    }

    /// [`Runner::tool_with_profile`] with `extra_env` set on top of
    /// [`TOOL_ENV`] (the benchmark scorer build pins an empty, harness-owned
    /// `CARGO_HOME`).
    pub(crate) fn tool_with_env(
        &self,
        argv: &[String],
        profile: Option<&str>,
        extra_env: &[(&str, &std::ffi::OsStr)],
    ) -> Result<Vec<u8>, Error> {
        let exe = argv
            .first()
            .ok_or_else(|| Error::Invariant("oracle: empty argv".into()))?;
        if !self.allowlist.iter().any(|a| a == exe) {
            return Err(Error::Invariant(format!(
                "executable `{exe}` is not on the [oracle] allowlist in harness.toml"
            )));
        }
        let shown = argv.join(" ");
        let pinned = pinned_toolchain();
        let mut env = tool_env(&pinned, extra_env);
        if let Some(tmp) = &self.tool_tmpdir {
            env.push(("TMPDIR", tmp.as_os_str()));
        }
        let out = self.spawn_in(
            argv,
            profile,
            TOOL_ENV,
            &env,
            &shown,
            &self.work,
            false,
            Wait::Tool,
        )?;
        match out.end {
            ChildEnd::Exited(status) if status.success() => Ok(out.stdout),
            ChildEnd::Exited(_)
                if matches!(exe.as_str(), "cargo" | "rustc")
                    && missing_toolchain(&out.stderr).is_some() =>
            {
                let name = missing_toolchain(&out.stderr).unwrap_or_default();
                Err(Error::Invariant(missing_toolchain_sentence(&name)))
            }
            ChildEnd::Exited(status) => Err(Error::Invariant(format!(
                "`{shown}` failed ({status}):\n{}",
                stderr_excerpt(&out.stderr)
            ))),
            ChildEnd::TimedOut => Err(Error::Invariant(format!(
                "`{shown}` timed out after {}s",
                self.timeout.as_secs()
            ))),
            ChildEnd::OutputOverflow => Err(Error::Invariant(format!(
                "`{shown}` produced more than {} bytes of output",
                self.max_output
            ))),
        }
    }

    /// Run an allowlisted tool under the tool profile, keeping a tool
    /// FAILURE apart from a harness error: `Ok(Ok(stdout))` on success,
    /// `Ok(Err(text))` when the tool ran and failed (the bounded stderr
    /// excerpt, or the timeout/overflow note — never the command line), and
    /// `Err` only when it could not be run at all (allowlist, spawn). Used
    /// where a compile failure is evidence (a failed check), not an error.
    pub(crate) fn tool_outcome(&self, argv: &[String]) -> Result<Result<Vec<u8>, String>, Error> {
        Ok(self.tool_outcome_both(argv)?.map(|(stdout, _)| stdout))
    }

    /// [`Runner::tool_outcome`] with the stderr of a run that succeeded
    /// (`cc -H` lists the headers there).
    pub(crate) fn tool_outcome_both(
        &self,
        argv: &[String],
    ) -> Result<Result<Streams, String>, Error> {
        let exe = argv.first().cloned().unwrap_or_default();
        let out = self.tool_run(argv)?;
        Ok(match out.end {
            ChildEnd::Exited(status) if status.success() => Ok((out.stdout, out.stderr)),
            ChildEnd::Exited(status) => Err(format!(
                "{exe} failed ({status}):\n{}",
                stderr_excerpt(&out.stderr)
            )),
            ChildEnd::TimedOut => Err(format!("{exe} timed out after {}s", self.timeout.as_secs())),
            ChildEnd::OutputOverflow => Err(format!(
                "{exe} produced more than {} bytes of output",
                self.max_output
            )),
        })
    }

    /// Run an allowlisted tool under the tool profile and hand back how it
    /// ended with both streams whole (up to the output cap) — a failed
    /// compile's every error line, where [`Runner::tool_outcome`] keeps an
    /// excerpt (docs/FEATURES-PROBE-REDESIGN.md §3.4 step 2). `Err` only when
    /// it could not be run at all (allowlist, spawn, interrupt).
    pub(crate) fn tool_run(&self, argv: &[String]) -> Result<ChildOutput, Error> {
        let exe = argv
            .first()
            .ok_or_else(|| Error::Invariant("oracle: empty argv".into()))?;
        if !self.allowlist.iter().any(|a| a == exe) {
            return Err(Error::Invariant(format!(
                "executable `{exe}` is not on the [oracle] allowlist in harness.toml"
            )));
        }
        let shown = argv.join(" ");
        let pinned = pinned_toolchain();
        let mut env = tool_env(&pinned, &[]);
        if let Some(tmp) = &self.tool_tmpdir {
            env.push(("TMPDIR", tmp.as_os_str()));
        }
        self.spawn_in(
            argv,
            self.tool_profile.as_deref(),
            TOOL_ENV,
            &env,
            &shown,
            &self.work,
            false,
            Wait::Tool,
        )
    }

    /// Run a binary the oracle just built — by path, exempt from the name
    /// allowlist, with only `PATH` in its environment, under the given run
    /// sandbox profile (`None` = unsandboxed). Production runs go through
    /// [`crate::confine::Confinement`], which renders the per-run profile and
    /// temp dir; this raw form is for tests. Returns stdout only; every failure
    /// (including a timeout) is a [`RunFailure`] the caller turns into a
    /// failed check.
    #[cfg(test)]
    pub(crate) fn built_with_profile(
        &self,
        bin: &Path,
        args: &[&str],
        profile: Option<&str>,
    ) -> Result<Vec<u8>, RunFailure> {
        match self.built_with_env(bin, args, profile, &[]) {
            Ok(Ok(out)) => Ok(out.stdout),
            Ok(Err(failure)) => Err(failure),
            Err(e) => Err(RunFailure::Failed(e.to_string())),
        }
    }

    /// [`Runner::built_with_profile`] plus `extra_env` set explicitly on top
    /// of [`BUILT_ENV`] (the confinement's per-run `TMPDIR`), returning
    /// both captured streams. `Ok(Err(_))` is evidence — the run failed,
    /// timed out, overflowed, or could not be spawned; the outer `Err` is
    /// only [`Error::Interrupted`]: the harness was cancelled, and what the
    /// killed child did is not evidence of anything.
    pub(crate) fn built_with_env(
        &self,
        bin: &Path,
        args: &[&str],
        profile: Option<&str>,
        extra_env: &[(&str, &std::ffi::OsStr)],
    ) -> Result<Result<RunOutput, RunFailure>, Error> {
        let Some(bin_str) = bin.to_str() else {
            return Ok(Err(RunFailure::Failed(format!(
                "non-UTF-8 path: {}",
                bin.display()
            ))));
        };
        let mut argv: Vec<String> = vec![bin_str.to_string()];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        let shown = argv.join(" ");
        let out = match self.spawn(&argv, profile, BUILT_ENV, extra_env, &shown, Wait::Built) {
            Ok(out) => out,
            Err(Error::Interrupted) => return Err(Error::Interrupted),
            Err(e) => return Ok(Err(RunFailure::Failed(e.to_string()))),
        };
        Ok(match out.end {
            ChildEnd::Exited(status) if status.success() => Ok(RunOutput {
                stdout: out.stdout,
                stderr: out.stderr,
            }),
            ChildEnd::Exited(status) => Err(RunFailure::Failed(format!(
                "`{shown}` failed ({status}):\n{}",
                stderr_excerpt(&out.stderr)
            ))),
            ChildEnd::TimedOut => Err(RunFailure::TimedOut {
                secs: self.timeout.as_secs(),
            }),
            ChildEnd::OutputOverflow => Err(RunFailure::Failed(format!(
                "`{shown}` produced more than {} bytes of output",
                self.max_output
            ))),
        })
    }

    /// Run a built binary like [`Runner::built_with_env`], but report HOW it
    /// ended instead of treating a non-zero exit as a failure: the benchmark
    /// scorer's runner exits 1 for a failed vector, a normal outcome.
    pub(crate) fn built_status(
        &self,
        bin: &Path,
        args: &[&str],
        profile: Option<&str>,
        extra_env: &[(&str, &std::ffi::OsStr)],
    ) -> Result<crate::bench::RunStatus, Error> {
        let bin_str = bin
            .to_str()
            .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", bin.display())))?;
        let mut argv: Vec<String> = vec![bin_str.to_string()];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        let shown = argv.join(" ");
        let out = self.spawn(&argv, profile, BUILT_ENV, extra_env, &shown, Wait::Built)?;
        Ok(match out.end {
            ChildEnd::Exited(status) => match status.code() {
                Some(code) => crate::bench::RunStatus::Exited(code),
                None => crate::bench::RunStatus::Killed,
            },
            ChildEnd::TimedOut => crate::bench::RunStatus::TimedOut,
            ChildEnd::OutputOverflow => crate::bench::RunStatus::Killed,
        })
    }

    /// Spawn a built program in [`Runner::cwd`].
    fn spawn(
        &self,
        argv: &[String],
        profile: Option<&str>,
        env_keys: &[&str],
        extra_env: &[(&str, &std::ffi::OsStr)],
        shown: &str,
        wait: Wait,
    ) -> Result<ChildOutput, Error> {
        self.spawn_in(
            argv, profile, env_keys, extra_env, shown, &self.cwd, false, wait,
        )
    }

    /// [`Runner::spawn`] with its working directory given, and — for a
    /// scenario run — the child's process group killed as soon as the child
    /// itself has exited (docs/FEATURES-DESIGN.md §4.1 step 5).
    #[allow(clippy::too_many_arguments)]
    fn spawn_in(
        &self,
        argv: &[String],
        profile: Option<&str>,
        env_keys: &[&str],
        extra_env: &[(&str, &std::ffi::OsStr)],
        shown: &str,
        cwd: &Path,
        kill_group_on_exit: bool,
        wait: Wait,
    ) -> Result<ChildOutput, Error> {
        let full: Vec<String> = match profile {
            Some(p) => sandbox::wrap(p, argv),
            None => argv.to_vec(),
        };
        let mut cmd = scrubbed_command(&full, env_keys, cwd)?;
        if env_keys.contains(&"PATH") {
            if let Some(path) = child_path(std::env::var_os("PATH"), &self.root) {
                cmd.env("PATH", path);
            }
        }
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        run_with_timeout(
            cmd,
            shown,
            self.timeout,
            self.max_output,
            kill_group_on_exit,
            wait,
        )
    }

    /// Run a scenario (docs/FEATURES-DESIGN.md §4): the built binary `bin`
    /// (absolute) with `args`, in `cwd`, with only `PATH` and `extra_env`,
    /// under `profile`; its process group killed once it exits. How it ended
    /// is data, whatever the exit code; the outer `Err` is only
    /// [`Error::Interrupted`] or a spawn that could not happen at all.
    pub(crate) fn scenario(
        &self,
        bin: &Path,
        args: &[&str],
        profile: Option<&str>,
        extra_env: &[(&str, &std::ffi::OsStr)],
        cwd: &Path,
    ) -> Result<ChildOutput, Error> {
        let bin_str = bin
            .to_str()
            .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", bin.display())))?;
        let mut argv: Vec<String> = vec![bin_str.to_string()];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        let shown = argv.join(" ");
        self.spawn_in(
            &argv,
            profile,
            BUILT_ENV,
            extra_env,
            &shown,
            cwd,
            true,
            Wait::Built,
        )
    }
}

/// Stderr as it appears in errors and check details: lossy UTF-8, capped at
/// [`STDERR_EXCERPT`] bytes (keeping the head, where compilers and sanitizers
/// put the primary diagnostic) — a child controls its stderr, and verdict
/// details are committed evidence that must stay small.
pub(crate) fn stderr_excerpt(stderr: &[u8]) -> String {
    if stderr.len() <= STDERR_EXCERPT {
        return String::from_utf8_lossy(stderr).into_owned();
    }
    format!(
        "{}\n… [{} more bytes of stderr omitted]",
        String::from_utf8_lossy(&stderr[..STDERR_EXCERPT]),
        stderr.len() - STDERR_EXCERPT
    )
}

/// Build a [`Command`] for `argv` with the environment cleared and only
/// `env_keys` copied from the parent (when set), stdin closed, and the
/// working directory pinned to `cwd`.
pub(crate) fn scrubbed_command(
    argv: &[String],
    env_keys: &[&str],
    cwd: &Path,
) -> Result<Command, Error> {
    let (exe, rest) = argv
        .split_first()
        .ok_or_else(|| Error::Invariant("oracle: empty argv".into()))?;
    let mut cmd = Command::new(exe);
    cmd.args(rest).current_dir(cwd).env_clear();
    for key in env_keys {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.stdin(Stdio::null());
    // Each child leads its own process group (its pgid == its pid). A child
    // that spawns grandchildren cannot then outlive a kill: at the timeout we
    // signal the whole group, so nothing model- or target-derived keeps
    // running after the oracle decides the run is over.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    Ok(cmd)
}

/// Best-effort SIGKILL of the whole process group led by `pgid` (a child
/// spawned with `process_group(0)`), so grandchildren die with the child.
/// Spawned as `/bin/kill -KILL -- -<pgid>`; any failure is ignored (the
/// group may already be gone).
#[cfg(unix)]
pub(crate) fn kill_process_group(pgid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Best-effort SIGTERM of one process (`/bin/kill -TERM <pid>`; no
/// `unsafe`): perfrun's own end (docs/PERF-DESIGN.md §3.3 step 6).
#[cfg(unix)]
pub(crate) fn terminate(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Spawn `cmd`, capture stdout/stderr on reader threads, and poll `try_wait`
/// as `wait` says ([`Wait`]) until exit or `timeout`, killing the child at the deadline
/// (or when a stream exceeds `max_output`). `Err` only for spawn/wait
/// failures; timeouts and overflows are reported in [`ChildOutput::end`].
pub(crate) fn run_with_timeout(
    mut cmd: Command,
    shown: &str,
    timeout: Duration,
    max_output: usize,
    kill_group_on_exit: bool,
    wait: Wait,
) -> Result<ChildOutput, Error> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Spawn under the registry lock: a cancellation either happened before
    // (nothing is spawned) or will find this child registered.
    let (mut child, registered) = {
        let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        if cancelled() {
            return Err(Error::Interrupted);
        }
        let child = cmd
            .spawn()
            .map_err(|e| Error::Invariant(format!("spawning `{shown}`: {e}")))?;
        let id = child.id();
        live.insert((OTHER, id));
        (child, Registered((OTHER, id)))
    };
    // The child leads its own group (see `scrubbed_command`), so its pid is
    // also its pgid — the group to kill on timeout/overflow.
    #[cfg(unix)]
    let pgid = child.id();

    let overflow = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let mut readers = 0usize;
    if let Some(pipe) = child.stdout.take() {
        drain(pipe, &stdout_buf, max_output, &overflow, done_tx.clone());
        readers += 1;
    }
    if let Some(pipe) = child.stderr.take() {
        drain(pipe, &stderr_buf, max_output, &overflow, done_tx.clone());
        readers += 1;
    }
    drop(done_tx);

    let deadline = Instant::now() + timeout;
    let mut poll = wait.first();
    let end = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // A scenario run leaves nothing behind: whatever the child
                // forked dies now, before the output is drained and before
                // anything it wrote is read back.
                #[cfg(unix)]
                if kill_group_on_exit {
                    kill_process_group(pgid);
                }
                break ChildEnd::Exited(status);
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Invariant(format!("waiting for `{shown}`: {e}")));
            }
        }
        let overflowed = overflow.load(Ordering::SeqCst);
        if overflowed || Instant::now() >= deadline {
            // kill() can only fail if the child is already gone; the group
            // kill then reaches any grandchildren it spawned; wait() reaps
            // the child itself so no zombie outlives the oracle.
            let _ = child.kill();
            #[cfg(unix)]
            kill_process_group(pgid);
            let _ = child.wait();
            break if overflowed {
                ChildEnd::OutputOverflow
            } else {
                ChildEnd::TimedOut
            };
        }
        std::thread::sleep(poll);
        poll = (poll * 2).min(wait.cap());
    };
    // A child that ended after the cancellation may have been killed by it:
    // its end is not evidence. Reap anything left and report the interrupt.
    if cancelled() {
        let _ = child.kill();
        #[cfg(unix)]
        kill_process_group(pgid);
        let _ = child.wait();
        drop(registered);
        return Err(Error::Interrupted);
    }
    drop(registered);

    // The readers hit EOF as soon as every write end of the pipes is closed —
    // immediately, unless an orphaned grandchild inherited them. Bound the
    // wait so such a grandchild can never hang the harness; whatever was
    // captured so far is still returned.
    let drain_deadline = Instant::now() + DRAIN_GRACE;
    for _ in 0..readers {
        let left = drain_deadline.saturating_duration_since(Instant::now());
        if done_rx.recv_timeout(left).is_err() {
            break;
        }
    }
    // A reader may flag overflow in the same instant the child exits.
    let end = match end {
        ChildEnd::Exited(_) if overflow.load(Ordering::SeqCst) => ChildEnd::OutputOverflow,
        other => other,
    };
    Ok(ChildOutput {
        end,
        stdout: take(&stdout_buf),
        stderr: take(&stderr_buf),
    })
}

/// Read `pipe` to EOF on a new thread, appending into `buf` (shared so a
/// partial capture survives if the thread is abandoned). Past `cap` bytes the
/// thread raises `overflow` and stops reading.
pub(crate) fn drain<R: Read + Send + 'static>(
    mut pipe: R,
    buf: &Arc<Mutex<Vec<u8>>>,
    cap: usize,
    overflow: &Arc<AtomicBool>,
    done: mpsc::Sender<()>,
) {
    let buf = Arc::clone(buf);
    let overflow = Arc::clone(overflow);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let Ok(mut guard) = buf.lock() else { break };
                    if guard.len() + n > cap {
                        overflow.store(true, Ordering::SeqCst);
                        break;
                    }
                    guard.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = done.send(());
    });
}

pub(crate) fn take(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
    match buf.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(timeout: Duration) -> Runner {
        Runner {
            cwd: std::env::temp_dir(),
            work: work_dir().expect("work folder"),
            root: std::env::temp_dir(),
            allowlist: vec!["env".into(), "sh".into(), "sleep".into()],
            timeout,
            max_output: DEFAULT_MAX_OUTPUT,
            tool_profile: None,
            tool_tmpdir: None,
        }
    }

    fn sv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    /// Test shorthand for an unsandboxed built-binary run.
    fn built(r: &Runner, bin: &str, args: &[&str]) -> Result<Vec<u8>, RunFailure> {
        r.built_with_profile(Path::new(bin), args, None)
    }

    fn env_keys(stdout: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(stdout)
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.to_string()))
            .collect()
    }

    #[test]
    fn tools_outside_the_allowlist_never_spawn() {
        let err = runner(Duration::from_secs(5))
            .tool(&sv(&["curl", "https://example.com"]))
            .expect_err("curl is not allowlisted");
        assert!(err.to_string().contains("not on the [oracle] allowlist"));
    }

    #[test]
    fn tool_environment_is_scrubbed_to_the_fixed_list() {
        // cargo exports CARGO_MANIFEST_DIR & co. into every test process, so
        // the parent environment provably holds variables outside the list.
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        let out = runner(Duration::from_secs(10))
            .tool(&sv(&["env"]))
            .expect("env runs");
        let keys = env_keys(&out);
        assert!(keys.iter().any(|k| k == "PATH"), "{keys:?}");
        for k in &keys {
            assert!(
                TOOL_ENV.contains(&k.as_str())
                    || TOOL_FIXED_ENV.iter().any(|(f, _)| f == k)
                    || k == "RUSTUP_TOOLCHAIN",
                "leaked variable {k}"
            );
        }
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.lines().any(|l| l == "SOURCE_DATE_EPOCH=0"),
            "builds are dated alike: {text}"
        );
        // The toolchain is pinned, and never installed.
        let pinned = format!("RUSTUP_TOOLCHAIN={}", pinned_toolchain().to_string_lossy());
        assert!(text.lines().any(|l| l == pinned), "{text}");
        assert!(text.lines().any(|l| l == "RUSTUP_AUTO_INSTALL=0"), "{text}");
        assert!(!keys.iter().any(|k| k == "CARGO_MANIFEST_DIR"));
    }

    /// Fix pass 2's check: a runner's own TMPDIR reaches its tools over the
    /// person's.
    #[test]
    fn a_runners_tmpdir_reaches_its_tools() {
        let mut r = runner(Duration::from_secs(10));
        r.tool_tmpdir = Some(PathBuf::from("/nowhere/tmp"));
        let out = r.tool(&sv(&["env"])).expect("env runs");
        let text = String::from_utf8_lossy(&out);
        let tmp: Vec<&str> = text.lines().filter(|l| l.starts_with("TMPDIR=")).collect();
        assert_eq!(tmp, ["TMPDIR=/nowhere/tmp"], "{text}");
    }

    #[test]
    fn built_binaries_get_only_path() {
        assert!(std::env::var_os("HOME").is_some());
        let out = built(&runner(Duration::from_secs(10)), "/usr/bin/env", &[]).expect("env runs");
        assert_eq!(env_keys(&out), vec!["PATH".to_string()]);
    }

    #[test]
    fn sandboxed_children_are_scrubbed_too() {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let r = runner(Duration::from_secs(10));
        let out = r
            .built_with_profile(
                Path::new("/usr/bin/env"),
                &[],
                Some("(version 1)(allow default)"),
            )
            .expect("env runs");
        assert_eq!(env_keys(&out), vec!["PATH".to_string()]);
    }

    #[test]
    fn large_output_on_both_streams_cannot_deadlock() {
        // ~1.3MB on stdout and on stderr — far beyond a 64KB pipe buffer.
        let script = "i=0; while [ $i -lt 20000 ]; do \
                      echo 'oooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooo'; \
                      echo 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' >&2; \
                      i=$((i+1)); done";
        let cmd = scrubbed_command(&sv(&["sh", "-c", script]), TOOL_ENV, &std::env::temp_dir())
            .expect("command");
        let out = run_with_timeout(
            cmd,
            "sh",
            Duration::from_secs(60),
            DEFAULT_MAX_OUTPUT,
            false,
            Wait::Tool,
        )
        .expect("runs");
        assert!(matches!(out.end, ChildEnd::Exited(s) if s.success()));
        assert_eq!(out.stdout.len(), 20000 * 65);
        assert_eq!(out.stderr.len(), 20000 * 65);
    }

    #[test]
    fn a_tool_timeout_is_an_error() {
        let started = Instant::now();
        let err = runner(Duration::from_secs(1))
            .tool(&sv(&["sleep", "30"]))
            .expect_err("must time out");
        assert!(err.to_string().contains("timed out after 1s"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_built_binary_timeout_is_a_run_failure_with_the_normative_detail() {
        let started = Instant::now();
        let failure = built(&runner(Duration::from_secs(1)), "/bin/sleep", &["30"])
            .expect_err("must time out");
        assert!(matches!(failure, RunFailure::TimedOut { secs: 1 }));
        assert_eq!(failure.to_string(), "timed out after 1s");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn timeouts_kill_sandboxed_children_as_well() {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let r = runner(Duration::from_secs(1));
        let started = Instant::now();
        let failure = r
            .built_with_profile(
                Path::new("/bin/sleep"),
                &["30"],
                Some("(version 1)(allow default)"),
            )
            .expect_err("must time out");
        assert_eq!(failure.to_string(), "timed out after 1s");
        // sandbox-exec execs in place, so kill() reaches the real child and
        // the pipes close at once — well inside the drain grace.
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn an_orphaned_grandchild_holding_the_pipe_cannot_hang_the_harness() {
        // The shell exits at once; its backgrounded sleep inherits stdout.
        let started = Instant::now();
        let out = runner(Duration::from_secs(20))
            .tool(&sv(&["sh", "-c", "sleep 15 & echo started"]))
            .expect("the shell itself succeeds");
        assert_eq!(String::from_utf8_lossy(&out).trim(), "started");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn non_zero_exit_reports_status_and_stderr() {
        let err = runner(Duration::from_secs(10))
            .tool(&sv(&["sh", "-c", "echo boom >&2; exit 3"]))
            .expect_err("exit 3");
        let text = err.to_string();
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains('3'), "{text}");

        let failure = built(
            &runner(Duration::from_secs(10)),
            "/bin/sh",
            &["-c", "exit 4"],
        )
        .expect_err("exit 4");
        assert!(matches!(failure, RunFailure::Failed(_)));
    }

    #[test]
    fn quoted_stderr_is_capped() {
        assert_eq!(stderr_excerpt(b"short"), "short");
        let long = vec![b'x'; STDERR_EXCERPT + 100];
        let text = stderr_excerpt(&long);
        assert!(text.starts_with("xxxx"));
        assert!(
            text.ends_with("[100 more bytes of stderr omitted]"),
            "{text}"
        );
        assert!(text.len() < STDERR_EXCERPT + 100);

        // End to end: a chatty failing child cannot bloat the failure text.
        let script =
            "i=0; while [ $i -lt 2000 ]; do echo 0123456789012345678901234567890123456789 >&2; \
                      i=$((i+1)); done; exit 1";
        let failure = built(&runner(Duration::from_secs(30)), "/bin/sh", &["-c", script])
            .expect_err("exit 1");
        assert!(failure.to_string().len() < STDERR_EXCERPT + 4096);
        assert!(failure.to_string().contains("more bytes of stderr omitted"));
    }

    /// A child that writes past the cap and then goes quiet — sleeping, or
    /// ignoring SIGPIPE and writing on — ends as an overflow at once, never
    /// at the deadline (docs/FEATURES-PROBE-REDESIGN.md §4, the runner).
    #[test]
    fn an_overflow_ends_the_run_even_when_the_child_goes_quiet() {
        for (script, wait) in [
            ("head -c 3000000 /dev/zero; sleep 60", Wait::Tool),
            ("head -c 3000000 /dev/zero; sleep 60", Wait::Built),
            (
                "trap '' PIPE; while :; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx 2>/dev/null; done",
                Wait::Built,
            ),
        ] {
            let cmd = scrubbed_command(&sv(&["sh", "-c", script]), TOOL_ENV, &std::env::temp_dir())
                .expect("command");
            let started = Instant::now();
            let out =
                run_with_timeout(cmd, "sh", Duration::from_secs(60), 1024 * 1024, false, wait)
                    .expect("runs");
            assert!(
                matches!(out.end, ChildEnd::OutputOverflow),
                "{script}: {:?}",
                out.end
            );
            assert!(started.elapsed() < Duration::from_secs(20), "{script}");
        }
    }

    #[test]
    fn runaway_output_is_cut_off() {
        let mut r = runner(Duration::from_secs(30));
        r.max_output = 1024 * 1024;
        let started = Instant::now();
        let failure = built(&r, "/usr/bin/yes", &[]).expect_err("yes never stops");
        assert!(
            failure.to_string().contains("more than 1048576 bytes"),
            "{failure}"
        );
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    /// A timeout kills the child's whole process group, not just the child:
    /// a shell-free C fixture forks a grandchild, both sleep well past the
    /// 1s deadline, and both pids are gone shortly after — proof the group
    /// kill reached the grandchild the plain `kill()` would have orphaned.
    #[cfg(unix)]
    #[test]
    fn a_timeout_kills_the_whole_process_group() {
        let tmp = crate::testutil::TempDir::new("pgkill");
        let src = tmp.path().join("forker.c");
        std::fs::write(
            &src,
            "#include <stdio.h>\n#include <unistd.h>\n\
             int main(int argc, char **argv) {\n\
               (void)argv;\n\
               if (argc > 1) return 0;\n\
               pid_t c = fork();\n\
               if (c == 0) { sleep(30); return 0; }\n\
               printf(\"%d %d\\n\", (int)getpid(), (int)c);\n\
               fflush(stdout);\n\
               sleep(30);\n\
               return 0;\n\
             }\n",
        )
        .expect("write fixture source");
        let bin = tmp.path().join("forker");
        let built = Command::new("cc")
            .arg("-o")
            .arg(&bin)
            .arg(&src)
            .status()
            .expect("cc runs");
        assert!(built.success(), "fixture must compile");
        // The first exec of a freshly linked binary can take seconds under
        // load (the system checks it): warm it once, so the timed run's
        // second is the program's own (check 7: this test's flake).
        let warm = Command::new(&bin).arg("warm").status().expect("warm run");
        assert!(warm.success());

        let bin_str = bin.to_str().expect("utf-8 bin path");
        let cmd = scrubbed_command(&sv(&[bin_str]), BUILT_ENV, tmp.path()).expect("command");
        let started = Instant::now();
        let out = run_with_timeout(
            cmd,
            "forker",
            Duration::from_secs(1),
            DEFAULT_MAX_OUTPUT,
            false,
            Wait::Built,
        )
        .expect("runs");
        assert!(matches!(out.end, ChildEnd::TimedOut), "{:?}", out.end);
        assert!(started.elapsed() < Duration::from_secs(10));

        let text = String::from_utf8_lossy(&out.stdout);
        let mut ids = text.split_whitespace();
        let parent: i32 = ids.next().and_then(|s| s.parse().ok()).expect("parent pid");
        let child: i32 = ids.next().and_then(|s| s.parse().ok()).expect("child pid");
        assert!(wait_until_gone(parent), "parent {parent} still alive");
        assert!(wait_until_gone(child), "grandchild {child} still alive");
    }

    /// `kill -0 <pid>` succeeds only while the process exists (and is not yet
    /// reaped); poll briefly so a just-signalled grandchild has time to go.
    #[cfg(unix)]
    /// The body of the cancellation scenario. The cancellation state is
    /// process-global, so this runs in a CHILD test process (re-exec'd by
    /// the test below with `RUHARNESS_CANCEL_TEST` set) and is a no-op
    /// otherwise.
    #[test]
    fn cancel_scenario_child_body() {
        if std::env::var_os("RUHARNESS_CANCEL_TEST").is_none() {
            return;
        }
        let worker = std::thread::spawn(|| {
            runner(Duration::from_secs(60)).tool(&sv(&["sh", "-c", "sleep 30"]))
        });
        let pgid = loop {
            if let Some((_, p)) = LIVE.lock().unwrap().iter().next().copied() {
                break p;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        println!("pgid={pgid}");
        assert_eq!(kill_live_process_groups(), 1);
        assert!(cancelled());
        // The run reports the interrupt — never a ChildEnd of a child the
        // harness itself killed.
        let result = worker.join().unwrap();
        assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
        // A spawner arriving after the cancellation never spawns: it blocks
        // on the registry, which stays locked until the process dies.
        let late = std::thread::spawn(|| {
            runner(Duration::from_secs(60)).tool(&sv(&["sh", "-c", "sleep 30"]))
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(!late.is_finished(), "a late spawner must block, not run");
        println!("cancel-ok");
        std::process::exit(0);
    }

    #[test]
    fn a_cancellation_interrupts_the_run_kills_the_group_and_blocks_later_spawns() {
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "exec::tests::cancel_scenario_child_body",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RUHARNESS_CANCEL_TEST", "1")
            .output()
            .expect("re-exec the test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "child test process failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("cancel-ok"), "{stdout}");
        // libtest prints its own "test … ..." prefix on the same line.
        let at = stdout.find("pgid=").expect("the child printed its pgid");
        let pgid: i32 = stdout[at + 5..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            wait_until_gone(pgid),
            "the cancelled child's group survived"
        );
    }

    fn wait_until_gone(pid: i32) -> bool {
        for _ in 0..100 {
            let alive = Command::new("/bin/kill")
                .arg("-0")
                .arg(pid.to_string())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// The tool profile a test runs cargo under: the production one, when
    /// this computer has the sandbox.
    fn toolchain_profile(root: &Path) -> Option<String> {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return None;
        }
        let host = sandbox::HostDirs::from_env().expect("HOME set");
        Some(
            sandbox::render_profile(&sandbox::ProfileSpec {
                host: &host,
                target_root: root,
                toolchain: true,
                write_dirs: &[],
                write_files: &[],
            })
            .expect("profile renders"),
        )
    }

    /// docs/PROJECT-MAP-DESIGN.md §3.7 and §4: a project's
    /// `.cargo/config.toml` (here a `target-dir` elsewhere and a
    /// `rustc-wrapper` that leaves a mark) and its `rust-toolchain.toml` (a
    /// `path` to nowhere) are never read: cargo starts in the work folder,
    /// with the manifest given absolute and the toolchain pinned.
    #[test]
    fn cargo_never_reads_a_projects_config_or_toolchain_file() {
        let tmp = crate::testutil::TempDir::new("project-config");
        let root = tmp.path();
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let elsewhere = root.join("elsewhere");
        let marker = root.join("wrapper-ran");
        put(
            "Cargo.toml",
            "[package]\nname = \"scratch\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        );
        put("src/lib.rs", "\n");
        put(
            ".cargo/config.toml",
            &format!(
                "[build]\ntarget-dir = \"{}\"\nrustc-wrapper = \"{}\"\n",
                elsewhere.display(),
                root.join("wrap.sh").display()
            ),
        );
        put(
            "wrap.sh",
            &format!("#!/bin/sh\ntouch '{}'\nexec \"$@\"\n", marker.display()),
        );
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("wrap.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        put(
            "rust-toolchain.toml",
            &format!(
                "[toolchain]\npath = \"{}\"\n",
                root.join("nowhere").display()
            ),
        );
        let runner = Runner::new(
            root,
            vec!["cargo".into()],
            Duration::from_secs(120),
            toolchain_profile(root),
        )
        .expect("runner");
        let manifest = root.join("Cargo.toml");
        let out = runner
            .tool(&sv(&[
                "cargo",
                "metadata",
                "--offline",
                "--no-deps",
                "--format-version",
                "1",
                "--manifest-path",
                manifest.to_str().unwrap(),
            ]))
            .expect("cargo metadata runs with the pinned toolchain");
        let meta: serde_json::Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(
            meta["target_directory"].as_str(),
            Some(root.join("target").to_str().unwrap()),
            "cargo's own target dir, not the project config's"
        );
        assert!(!marker.exists(), "the project's rustc-wrapper ran");
        assert!(!elsewhere.exists());
    }

    /// `RUSTUP_AUTO_INSTALL=0` is in every tool child's environment (the
    /// scrub test), so a toolchain that is not installed is refused in one
    /// sentence, never installed. Simulated with a name no channel has.
    #[test]
    fn a_missing_toolchain_is_refused_in_one_sentence() {
        let rustup = Command::new("rustup")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !rustup.is_ok_and(|s| s.success()) {
            return; // rustc is not rustup's here: there is nothing to pin.
        }
        let tmp = crate::testutil::TempDir::new("no-toolchain");
        let runner = Runner::new(
            tmp.path(),
            vec!["rustc".into()],
            Duration::from_secs(60),
            toolchain_profile(tmp.path()),
        )
        .expect("runner");
        let err = runner
            .tool_with_env(
                &sv(&["rustc", "-V"]),
                runner.tool_profile.as_deref(),
                &[(
                    "RUSTUP_TOOLCHAIN",
                    std::ffi::OsStr::new("ruharness-no-such-toolchain"),
                )],
            )
            .expect_err("no such toolchain");
        assert_eq!(
            err.to_string(),
            "the Rust toolchain `ruharness-no-such-toolchain` the harness pins its builds to is \
             not installed, and the harness never installs one: install it yourself with \
             `rustup toolchain install ruharness-no-such-toolchain`, or run the harness under a \
             toolchain you have"
        );
    }

    #[test]
    fn a_childs_path_keeps_absolute_entries_outside_the_project_only() {
        let tmp = crate::testutil::TempDir::new("child-path");
        let root = tmp.path().join("project");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let link = tmp.path().join("into-project");
        std::os::unix::fs::symlink(root.join("bin"), &link).unwrap();
        let raw = format!(
            "rel/bin::/usr/bin:.:{}:{}:/bin",
            root.join("bin").display(),
            link.display()
        );
        assert_eq!(
            child_path(Some(raw.into()), &root),
            Some("/usr/bin:/bin".into())
        );
        assert_eq!(child_path(None, &root), None);
        assert_eq!(
            child_path(Some(root.join("bin").into_os_string()), &root),
            Some("/usr/bin:/bin".into()),
            "nothing left: the system's folders, never an empty PATH"
        );
    }

    /// Live: a project `cc` on the harness's `PATH` (inside the root, and
    /// reachable by an empty and a `.` entry) never runs; the system's does.
    /// `PATH` is the process's, so the body runs in a child test process.
    #[test]
    fn project_cc_child_body() {
        let Some(root) = std::env::var_os("RUHARNESS_PATH_TEST") else {
            return;
        };
        let root = PathBuf::from(root);
        let runner =
            Runner::new(&root, vec!["cc".into()], Duration::from_secs(60), None).expect("runner");
        let out = runner
            .tool(&sv(&["cc", "--version"]))
            .expect("the system's cc runs");
        println!(
            "cc-said={}",
            String::from_utf8_lossy(&out).lines().next().unwrap_or("")
        );
        let built = runner.built_with_env(Path::new("/usr/bin/env"), &[], None, &[]);
        let env = built.expect("not interrupted").expect("env runs");
        let text = String::from_utf8_lossy(&env.stdout).into_owned();
        println!(
            "built-path={}",
            text.lines().find(|l| l.starts_with("PATH=")).unwrap_or("")
        );
        std::process::exit(0);
    }

    #[test]
    fn a_project_cc_on_the_path_never_runs() {
        let tmp = crate::testutil::TempDir::new("project-cc");
        let root = tmp.path().to_path_buf();
        let marker = root.join("project-cc-ran");
        let fake = format!("#!/bin/sh\ntouch '{}'\necho fake-cc\n", marker.display());
        for dir in [root.join("bin"), root.clone()] {
            std::fs::create_dir_all(&dir).unwrap();
            let cc = dir.join("cc");
            std::fs::write(&cc, &fake).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cc, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}::.:/usr/bin:/bin", root.join("bin").display());
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "exec::tests::project_cc_child_body",
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(&root)
            .env("PATH", &path)
            .env("RUHARNESS_PATH_TEST", &root)
            .output()
            .expect("re-exec the test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!marker.exists(), "the project's cc ran: {stdout}");
        assert!(!stdout.contains("cc-said=fake-cc"), "{stdout}");
        assert!(stdout.contains("cc-said="), "{stdout}");
        assert!(stdout.contains("built-path=PATH=/usr/bin:/bin"), "{stdout}");
    }

    /// The work folder is made at each run and refused when it is a link
    /// (or passes through one), or lies in a temporary folder.
    #[test]
    fn the_work_folder_is_refused_when_it_is_a_link() {
        let tmp = crate::testutil::TempDir::new("work-link");
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = tmp.path().join("work");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = make_work_dir(&link).expect_err("a link").to_string();
        assert!(err.contains("is a link or passes through one"), "{err}");
        let through = tmp.path().join("cache-link");
        std::os::unix::fs::symlink(&real, &through).unwrap();
        let err = make_work_dir(&through.join("ruharness/work"))
            .expect_err("through a link")
            .to_string();
        assert!(err.contains("is a link or passes through one"), "{err}");
        // A real folder is made, and made again when it is gone.
        let fresh = tmp.path().join("caches/ruharness/work");
        assert_eq!(make_work_dir(&fresh).unwrap(), fresh);
        std::fs::remove_dir(&fresh).unwrap();
        assert_eq!(make_work_dir(&fresh).unwrap(), fresh);
        // Never in a temporary folder.
        let home = Path::new("/Users/u");
        for dir in [
            "/private/tmp/u/Library/Caches/ruharness/work",
            "/private/var/folders/xy/T/ruharness/work",
        ] {
            let err = work_dir_at(Path::new(dir), home, None)
                .expect_err(dir)
                .to_string();
            assert!(err.contains("lies in a temporary folder"), "{err}");
        }
        let err = work_dir_at(
            Path::new("/Volumes/scratch/ruharness/work"),
            home,
            Some(Path::new("/Volumes/scratch")),
        )
        .expect_err("under TMPDIR")
        .to_string();
        assert!(err.contains("lies in a temporary folder"), "{err}");
        // Production's: under the home folder, a real folder, every run.
        let work = work_dir().expect("the work folder");
        assert!(work.ends_with("ruharness/work"), "{}", work.display());
        assert!(std::fs::symlink_metadata(&work).unwrap().is_dir());
    }

    /// Every tool child starts in the work folder; a built program keeps
    /// the folder its run asks for (the runner's `cwd`).
    #[test]
    fn tools_start_in_the_work_folder_and_built_programs_where_asked() {
        let tmp = crate::testutil::TempDir::new("cwds");
        let r = Runner::new(tmp.path(), vec!["sh".into()], Duration::from_secs(20), None)
            .expect("runner");
        let out = r.tool(&sv(&["sh", "-c", "pwd -P"])).expect("sh runs");
        assert_eq!(
            String::from_utf8_lossy(&out).trim(),
            r.work.to_str().unwrap()
        );
        assert_eq!(r.work, work_dir().unwrap());
        let out = built(&r, "/bin/pwd", &["-P"]).expect("pwd runs");
        assert_eq!(
            String::from_utf8_lossy(&out).trim(),
            tmp.path().to_str().unwrap()
        );
    }

    #[test]
    fn spawn_failure_of_a_built_binary_is_a_run_failure() {
        let failure = built(
            &runner(Duration::from_secs(5)),
            "/nonexistent/ruharness-bin",
            &[],
        )
        .expect_err("cannot spawn");
        assert!(failure.to_string().contains("spawning"), "{failure}");
    }

    /// The tool profile lets the toolchain read CARGO_HOME — except the
    /// registry credentials stored there.
    #[test]
    fn sandboxed_tools_cannot_read_cargo_credentials() {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = crate::testutil::TempDir::new("creds");
        let fake_cargo_home = tmp.path().join("cargo-home");
        std::fs::create_dir_all(&fake_cargo_home).expect("fake CARGO_HOME");
        std::fs::write(
            fake_cargo_home.join("credentials.toml"),
            "token = \"s3cret\"\n",
        )
        .expect("credentials");
        std::fs::write(fake_cargo_home.join("config.toml"), "# plain config\n").expect("config");
        let mut host = sandbox::HostDirs::from_env().expect("HOME set");
        host.cargo_home = Some(fake_cargo_home.clone());
        let profile = sandbox::render_profile(&sandbox::ProfileSpec {
            host: &host,
            target_root: tmp.path(),
            toolchain: true,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("profile renders");
        let mut r = runner(Duration::from_secs(20));
        r.allowlist.push("cat".into());
        r.tool_profile = Some(profile);

        let creds = fake_cargo_home.join("credentials.toml");
        let err = r
            .tool(&sv(&["cat", creds.to_str().expect("utf-8")]))
            .expect_err("credentials must be unreadable");
        assert!(err.to_string().contains("Operation not permitted"), "{err}");
        let config = fake_cargo_home.join("config.toml");
        let out = r
            .tool(&sv(&["cat", config.to_str().expect("utf-8")]))
            .expect("the rest of CARGO_HOME stays readable");
        assert_eq!(String::from_utf8_lossy(&out), "# plain config\n");
    }

    /// Behavioural proof of the run profile, hermetic: the network probe
    /// targets a listener this test owns on loopback, and every denial is
    /// paired with the same action succeeding unsandboxed, so a pass can
    /// never be vacuous.
    #[test]
    fn sandbox_denies_network_home_reads_and_stray_writes() {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let host = sandbox::HostDirs::from_env().expect("HOME set");
        let tmp = crate::testutil::TempDir::new("sbx");
        let profile = sandbox::render_profile(&sandbox::ProfileSpec {
            host: &host,
            target_root: tmp.path(),
            toolchain: false,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("profile renders");
        let open = runner(Duration::from_secs(20));
        let boxed = runner(Duration::from_secs(20));
        let boxed_built = |bin: &str, args: &[&str]| {
            boxed.built_with_profile(Path::new(bin), args, Some(profile.as_str()))
        };

        // Reads under the home directory are denied. The unsandboxed control
        // lists a directory under HOME with nothing privacy-protected in it
        // (the toolchain's), never HOME itself: `ls` stats every entry, and
        // stat-ing ~/Music or a network share mounted in HOME makes macOS
        // prompt for Apple Music / network-volume access on the user's
        // machine. Without one, `ls -d` stats HOME alone.
        let probe_dir = [host.cargo_home.as_ref(), host.rustup_home.as_ref()]
            .into_iter()
            .flatten()
            .find(|dir| dir.starts_with(&host.home) && dir.is_dir())
            .cloned();
        let (flag, target) = match &probe_dir {
            Some(dir) => ("-1", dir.to_str().expect("utf-8 probe dir").to_string()),
            None => ("-d", host.home.to_str().expect("utf-8 home").to_string()),
        };
        built(&open, "/bin/ls", &[flag, &target]).expect("readable unsandboxed");
        let failure = boxed_built("/bin/ls", &[flag, &target])
            .expect_err("reads under HOME must be denied in the sandbox");
        assert!(
            failure.to_string().contains("Operation not permitted"),
            "{failure}"
        );

        // Writes are confined to temp. The probe dir is the test binary's
        // own directory — writable, and not a temp dir on a normal checkout.
        let exe_dir = std::env::current_exe()
            .expect("test exe")
            .parent()
            .expect("exe has a parent")
            .canonicalize()
            .expect("exe dir resolves");
        let in_temp = exe_dir.starts_with("/private/tmp")
            || exe_dir.starts_with("/private/var/folders")
            || host.tmpdir.as_ref().is_some_and(|t| exe_dir.starts_with(t));
        if !in_temp {
            let probe = exe_dir.join(format!("ruharness-sbx-probe-{}", std::process::id()));
            let probe_str = probe.to_str().expect("utf-8 probe");
            let denied = boxed_built("/usr/bin/touch", &[probe_str]);
            let created = probe.exists();
            let _ = std::fs::remove_file(&probe);
            assert!(denied.is_err() && !created, "stray write was not denied");
            built(&open, "/usr/bin/touch", &[probe_str]).expect("the same write works unsandboxed");
            let _ = std::fs::remove_file(&probe);
        }
        let ok = tmp.path().join("probe");
        boxed_built("/usr/bin/touch", &[ok.to_str().expect("utf-8 temp")])
            .expect("temp stays writable in the sandbox");
        assert!(ok.exists());

        // Network is denied — even loopback, to a port that is listening.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("addr").port().to_string();
        let nc = ["-z", "-w", "5", "127.0.0.1", port.as_str()];
        built(&open, "/usr/bin/nc", &nc).expect("loopback connect works unsandboxed");
        let failure =
            boxed_built("/usr/bin/nc", &nc).expect_err("network must be denied in the sandbox");
        assert!(matches!(failure, RunFailure::Failed(_)), "{failure}");
    }
}
