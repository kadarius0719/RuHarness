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

/// Variables copied from the parent (when set) into tool children.
pub(crate) const TOOL_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
];

/// Variables copied from the parent (when set) into built binaries.
pub(crate) const BUILT_ENV: &[&str] = &["PATH"];

/// Default `[oracle] timeout_secs`.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default cap on captured bytes per stream (the M0 driver prints ~180KB).
pub(crate) const DEFAULT_MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// How much of a failed child's stderr is quoted in errors and check details.
pub(crate) const STDERR_EXCERPT: usize = 8 * 1024;

/// `try_wait` polling interval.
const POLL: Duration = Duration::from_millis(50);

/// How long to wait for the reader threads after the child is gone. They
/// normally finish at once (EOF when the child exits); the bound only
/// matters when an orphaned grandchild still holds the pipe open.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

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
    /// Working directory of every child (the target root).
    pub cwd: PathBuf,
    /// The core-owned `[oracle] allowlist` of tool names.
    pub allowlist: Vec<String>,
    /// Wall-clock limit per child.
    pub timeout: Duration,
    /// Capture cap per stream, in bytes.
    pub max_output: usize,
    /// Sandbox profile for tool invocations (`None` = unsandboxed).
    pub tool_profile: Option<String>,
}

impl Runner {
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
        let out = self.spawn(argv, profile, TOOL_ENV, extra_env, &shown)?;
        match out.end {
            ChildEnd::Exited(status) if status.success() => Ok(out.stdout),
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
        let exe = argv
            .first()
            .ok_or_else(|| Error::Invariant("oracle: empty argv".into()))?;
        if !self.allowlist.iter().any(|a| a == exe) {
            return Err(Error::Invariant(format!(
                "executable `{exe}` is not on the [oracle] allowlist in harness.toml"
            )));
        }
        let shown = argv.join(" ");
        let out = self.spawn(argv, self.tool_profile.as_deref(), TOOL_ENV, &[], &shown)?;
        Ok(match out.end {
            ChildEnd::Exited(status) if status.success() => Ok(out.stdout),
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
        self.built_with_env(bin, args, profile, &[])
            .map(|out| out.stdout)
    }

    /// [`Runner::built_with_profile`] plus `extra_env` set explicitly on top
    /// of [`BUILT_ENV`] (the confinement's per-run `TMPDIR`), returning
    /// both captured streams.
    pub(crate) fn built_with_env(
        &self,
        bin: &Path,
        args: &[&str],
        profile: Option<&str>,
        extra_env: &[(&str, &std::ffi::OsStr)],
    ) -> Result<RunOutput, RunFailure> {
        let bin_str = bin
            .to_str()
            .ok_or_else(|| RunFailure::Failed(format!("non-UTF-8 path: {}", bin.display())))?;
        let mut argv: Vec<String> = vec![bin_str.to_string()];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        let shown = argv.join(" ");
        let out = self
            .spawn(&argv, profile, BUILT_ENV, extra_env, &shown)
            .map_err(|e| RunFailure::Failed(e.to_string()))?;
        match out.end {
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
        }
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
        let out = self.spawn(&argv, profile, BUILT_ENV, extra_env, &shown)?;
        Ok(match out.end {
            ChildEnd::Exited(status) => match status.code() {
                Some(code) => crate::bench::RunStatus::Exited(code),
                None => crate::bench::RunStatus::Killed,
            },
            ChildEnd::TimedOut => crate::bench::RunStatus::TimedOut,
            ChildEnd::OutputOverflow => crate::bench::RunStatus::Killed,
        })
    }

    fn spawn(
        &self,
        argv: &[String],
        profile: Option<&str>,
        env_keys: &[&str],
        extra_env: &[(&str, &std::ffi::OsStr)],
        shown: &str,
    ) -> Result<ChildOutput, Error> {
        let full: Vec<String> = match profile {
            Some(p) => sandbox::wrap(p, argv),
            None => argv.to_vec(),
        };
        let mut cmd = scrubbed_command(&full, env_keys, &self.cwd)?;
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        run_with_timeout(cmd, shown, self.timeout, self.max_output)
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
fn kill_process_group(pgid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Spawn `cmd`, capture stdout/stderr on reader threads, and poll `try_wait`
/// every [`POLL`] until exit or `timeout`, killing the child at the deadline
/// (or when a stream exceeds `max_output`). `Err` only for spawn/wait
/// failures; timeouts and overflows are reported in [`ChildOutput::end`].
pub(crate) fn run_with_timeout(
    mut cmd: Command,
    shown: &str,
    timeout: Duration,
    max_output: usize,
) -> Result<ChildOutput, Error> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Invariant(format!("spawning `{shown}`: {e}")))?;
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
    let end = loop {
        match child.try_wait() {
            Ok(Some(status)) => break ChildEnd::Exited(status),
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
        std::thread::sleep(POLL);
    };

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
fn drain<R: Read + Send + 'static>(
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

fn take(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
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
            allowlist: vec!["env".into(), "sh".into(), "sleep".into()],
            timeout,
            max_output: DEFAULT_MAX_OUTPUT,
            tool_profile: None,
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
            assert!(TOOL_ENV.contains(&k.as_str()), "leaked variable {k}");
        }
        assert!(!keys.iter().any(|k| k == "CARGO_MANIFEST_DIR"));
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
        let out =
            run_with_timeout(cmd, "sh", Duration::from_secs(60), DEFAULT_MAX_OUTPUT).expect("runs");
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
             int main(void) {\n\
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

        let bin_str = bin.to_str().expect("utf-8 bin path");
        let cmd = scrubbed_command(&sv(&[bin_str]), BUILT_ENV, tmp.path()).expect("command");
        let started = Instant::now();
        let out = run_with_timeout(cmd, "forker", Duration::from_secs(1), DEFAULT_MAX_OUTPUT)
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

        // Reads under the home directory are denied.
        let home = host.home.to_str().expect("utf-8 home").to_string();
        built(&open, "/bin/ls", &[&home]).expect("home is listable unsandboxed");
        let failure =
            boxed_built("/bin/ls", &[&home]).expect_err("home must be unreadable in the sandbox");
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
