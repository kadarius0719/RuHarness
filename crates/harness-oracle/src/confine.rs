//! R1 run confinement (docs/M4-DESIGN.md §R1): how every binary the oracle
//! built — differential drivers, whole programs, sanitizer builds, mutants —
//! is run.
//!
//! Each run gets a FRESH temp dir, created here, passed as `TMPDIR`, the only
//! place the run may write, and removed afterwards. Under the sandbox the run
//! may read nothing under the user's home or the target root except the
//! binary itself and the explicitly listed input files (a whole-program
//! sample), and may `exec` nothing but itself ([`crate::sandbox::render_run_profile`]).
//!
//! This closes a real hole: before M4 a run could read the target root, so a
//! Rust candidate could open the previous turn's `migration/build/<unit>/
//! drv_c.out` and print it back. Nothing a C-side run writes survives it
//! either — its temp dir is gone before the candidate side starts.

use crate::exec::{RunFailure, Runner};
use crate::sandbox::{self, HostDirs, RunSpec};
use harness_core::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Per-process counter making run temp dir names unique.
static RUN_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Everything a confined run needs from the verification in progress.
#[derive(Clone, Copy)]
pub(crate) struct Confinement<'a> {
    /// Spawns the child (timeout, output cap, scrubbed environment).
    pub runner: &'a Runner,
    /// Host dirs for the run profile; `None` = no sandbox on this platform
    /// (the fresh `TMPDIR` is still applied).
    pub host: Option<&'a HostDirs>,
    /// Canonical target root (denied to the run).
    pub target_root: &'a Path,
}

impl Confinement<'_> {
    /// Run the built binary `bin` (canonical) with `args`, allowed to read the
    /// canonical `inputs` besides itself. Every failure — including a profile
    /// that cannot be rendered or a temp dir that cannot be created — is a
    /// [`RunFailure`], i.e. a failed check rather than a harness abort.
    pub(crate) fn run(
        &self,
        bin: &Path,
        args: &[&str],
        inputs: &[PathBuf],
    ) -> Result<Vec<u8>, RunFailure> {
        let tmp = RunTmp::create().map_err(|e| RunFailure::Failed(e.to_string()))?;
        let profile = match self.host {
            Some(host) => Some(
                sandbox::render_run_profile(&RunSpec {
                    host,
                    target_root: self.target_root,
                    bin,
                    read_files: inputs,
                    tmpdir: tmp.path(),
                })
                .map_err(|e| RunFailure::Failed(e.to_string()))?,
            ),
            None => None,
        };
        let env = [("TMPDIR", tmp.path().as_os_str())];
        self.runner
            .built_with_env(bin, args, profile.as_deref(), &env)
        // `tmp` is dropped (removed) here, after the child is gone.
    }
}

/// A fresh, canonical, per-run temp dir under the system temp dir, removed
/// on drop.
struct RunTmp(PathBuf);

impl RunTmp {
    fn create() -> Result<RunTmp, Error> {
        let base_raw = std::env::temp_dir();
        let base = base_raw
            .canonicalize()
            .map_err(|e| Error::io(&base_raw, e))?;
        for _ in 0..1000 {
            let dir = base.join(format!(
                "ruharness-run-{}-{}",
                std::process::id(),
                RUN_COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(RunTmp(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io(&dir, e)),
            }
        }
        Err(Error::Invariant(format!(
            "could not create a fresh run temp dir under {}",
            base.display()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for RunTmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::DEFAULT_MAX_OUTPUT;
    use crate::testutil::TempDir;
    use std::process::Command;
    use std::time::Duration;

    /// A probe that reports what it could do, one line per argument:
    /// `r:<path>` read, `w:<path>` create, `t` write into `$TMPDIR` (and print
    /// it), `x` exec `/usr/bin/true`.
    const PROBE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) {
    const char *a = argv[i];
    if (a[0] == 'r') {
      FILE *f = fopen(a + 2, "rb");
      printf("read %s\n", f ? "ALLOWED" : "denied");
      if (f) fclose(f);
    } else if (a[0] == 'w') {
      FILE *f = fopen(a + 2, "wb");
      printf("write %s\n", f ? "ALLOWED" : "denied");
      if (f) fclose(f);
    } else if (a[0] == 't') {
      const char *t = getenv("TMPDIR");
      char p[4096];
      snprintf(p, sizeof p, "%s/probe.tmp", t ? t : "/nonexistent");
      FILE *f = fopen(p, "wb");
      printf("tmp %s %s\n", t ? t : "(unset)", f ? "writable" : "denied");
      if (f) fclose(f);
    } else if (a[0] == 'x') {
      pid_t pid = fork();
      if (pid == 0) { execl("/usr/bin/true", "true", (char *)0); _exit(127); }
      int st = 0;
      if (pid > 0) waitpid(pid, &st, 0);
      printf("exec %s\n", (pid > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0) ? "ALLOWED" : "denied");
    }
  }
  return 0;
}
"#;

    fn build_probe(dir: &Path) -> PathBuf {
        let src = dir.join("probe.c");
        std::fs::write(&src, format!("#include <sys/wait.h>\n{PROBE}")).expect("probe source");
        let bin = dir.join("probe");
        let status = Command::new("cc")
            .arg("-o")
            .arg(&bin)
            .arg(&src)
            .status()
            .expect("cc runs");
        assert!(status.success(), "probe compiles");
        bin.canonicalize().expect("canonical probe")
    }

    fn runner(root: &Path) -> Runner {
        Runner {
            cwd: root.to_path_buf(),
            allowlist: Vec::new(),
            timeout: Duration::from_secs(30),
            max_output: DEFAULT_MAX_OUTPUT,
            tool_profile: None,
        }
    }

    /// Regression for the M3 hole R1 closes: a built binary could read the
    /// build dir's `drv_c.out` (inside the target root) and replay it. Under
    /// the run profile it cannot — while its listed input, its own temp dir
    /// and nothing else stay usable. Every denial is paired with the same
    /// action succeeding unconfined, so the test can never pass vacuously.
    #[test]
    fn a_built_binary_cannot_read_the_target_root() {
        if sandbox::sandbox_mode() != "sandbox-exec" {
            return;
        }
        let tmp = TempDir::new("confine");
        let root = tmp.path();
        let build = root.join("migration/build/u1");
        std::fs::create_dir_all(&build).expect("build dir");
        let secret = build.join("drv_c.out");
        std::fs::write(&secret, "pinned C output\n").expect("drv_c.out");
        let config = root.join("harness.toml");
        std::fs::write(&config, "schema_version = 1\n").expect("harness.toml");
        let sample = build.join("sample_text.txt");
        std::fs::write(&sample, "sample\n").expect("sample");
        let stray = build.join("stray.out");
        let bin = build_probe(&build);

        let args_owned = [
            format!("r:{}", secret.display()),
            format!("r:{}", config.display()),
            format!("r:{}", sample.display()),
            format!("w:{}", stray.display()),
            "t".to_string(),
            "x".to_string(),
        ];
        let args: Vec<&str> = args_owned.iter().map(String::as_str).collect();

        // Unconfined, everything works (so the denials below are real).
        let r = runner(root);
        let open = r
            .built_with_env(&bin, &args, None, &[])
            .expect("unconfined probe runs");
        let open = String::from_utf8_lossy(&open);
        assert!(
            open.starts_with("read ALLOWED\nread ALLOWED\nread ALLOWED\nwrite ALLOWED\n"),
            "{open}"
        );
        assert!(open.ends_with("exec ALLOWED\n"), "{open}");
        std::fs::remove_file(&stray).expect("stray created unconfined");

        let host = HostDirs::from_env().expect("HOME set");
        let confined = Confinement {
            runner: &r,
            host: Some(&host),
            target_root: root,
        };
        let out = confined
            .run(&bin, &args, std::slice::from_ref(&sample))
            .expect("confined probe runs");
        let out = String::from_utf8_lossy(&out).into_owned();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 6, "{out}");
        assert_eq!(lines[0], "read denied", "drv_c.out must be unreadable");
        assert_eq!(
            lines[1], "read denied",
            "the target root must be unreadable"
        );
        assert_eq!(lines[2], "read ALLOWED", "a listed input stays readable");
        assert_eq!(lines[3], "write denied");
        assert!(!stray.exists(), "a confined run wrote into the build dir");
        assert_eq!(lines[5], "exec denied");

        // TMPDIR is a fresh dir of this run, writable during it, gone after.
        let tmp_line: Vec<&str> = lines[4].split(' ').collect();
        assert_eq!(tmp_line.len(), 3, "{out}");
        assert_eq!(tmp_line[2], "writable", "{out}");
        let run_tmp = PathBuf::from(tmp_line[1]);
        assert!(
            run_tmp
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ruharness-run-")),
            "{out}"
        );
        assert!(!run_tmp.exists(), "the run temp dir must be removed");

        // Two runs never share a temp dir.
        let again = confined.run(&bin, &["t"], &[]).expect("second run");
        let again = String::from_utf8_lossy(&again).into_owned();
        assert!(!again.contains(tmp_line[1]), "{again} vs {out}");
    }

    /// Without a sandbox the fresh TMPDIR is still applied (and removed).
    #[test]
    fn the_fresh_tmpdir_applies_without_a_sandbox_too() {
        let tmp = TempDir::new("confine-nosbx");
        let bin = build_probe(tmp.path());
        let r = runner(tmp.path());
        let confined = Confinement {
            runner: &r,
            host: None,
            target_root: tmp.path(),
        };
        let out = confined.run(&bin, &["t"], &[]).expect("runs");
        let out = String::from_utf8_lossy(&out).into_owned();
        let fields: Vec<&str> = out.trim().split(' ').collect();
        assert_eq!(fields.len(), 3, "{out}");
        assert_eq!(fields[2], "writable", "{out}");
        assert!(!Path::new(fields[1]).exists(), "{out}");
    }
}
