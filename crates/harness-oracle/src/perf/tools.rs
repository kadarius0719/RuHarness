//! `harness perf show`'s compiler check (docs/PERF-DESIGN.md §3.9): `cc
//! --version` and `rustc -V` as tool runs — the target's allowlist, the
//! tool environment, the tool sandbox profile and `[oracle] timeout_secs` —
//! the way `perf run` reads them (the same environment, the same first
//! line), so unchanged compilers compare equal. Never a plain process: the
//! target picks which `rustc` runs (a `rust-toolchain.toml`), so its
//! compilers are target code.

use crate::exec::Runner;
use crate::sandbox::{self, HostDirs, ProfileSpec};
use crate::Base;
use harness_core::TargetContext;

/// Most bytes a version check may print before it is stopped: a version is
/// a few short lines.
pub(crate) const VERSION_OUTPUT_CAP: usize = 64 * 1024;

/// The first line a tool run printed, trimmed, at most 160 characters;
/// `None` when the tool failed, ran past its time, printed too much or
/// printed nothing.
pub(crate) fn first_line(runner: &Runner, argv: &[&str]) -> Option<String> {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let out = runner.tool(&argv).ok()?;
    let line: String = String::from_utf8_lossy(&out)
        .lines()
        .next()?
        .trim()
        .chars()
        .take(160)
        .collect();
    (!line.is_empty()).then_some(line)
}

/// `cc --version` and `rustc -V`'s first lines on `runner`, both read
/// whatever the first gave — what `perf run` stores; `perf show` checks
/// with the same [`first_line`] on the same runs (§3.9), stopping at the
/// first failure, so unchanged compilers compare equal by construction.
/// Each `None` as [`first_line`] says.
pub(crate) fn compiler_lines(runner: &Runner) -> (Option<String>, Option<String>) {
    (
        first_line(runner, &["cc", "--version"]),
        first_line(runner, &["rustc", "-V"]),
    )
}

/// `cc --version` and `rustc -V`'s first lines for `harness perf show`, each
/// run as a tool run in the target's root. `None` — "compilers not checked"
/// — when the allowlist lacks either tool, the target's settings cannot be
/// used, either run fails, or there is no sandbox and `allow_unsandboxed`
/// is false (without a sandbox and with it, the runs keep the tool
/// environment and the timeout).
pub fn perf_compilers(target: &TargetContext, allow_unsandboxed: bool) -> Option<(String, String)> {
    let sandboxed = sandbox::sandbox_mode() == "sandbox-exec";
    if !sandboxed && !allow_unsandboxed {
        return None;
    }
    let base = Base::resolve(target, "perf", &["cc", "rustc"]).ok()?;
    let tool_profile = if sandboxed {
        let host = HostDirs::from_env().ok()?;
        // Writes nowhere but the temp folders every tool profile allows.
        Some(
            sandbox::render_profile(&ProfileSpec {
                host: &host,
                target_root: &base.root,
                toolchain: true,
                write_dirs: &[],
                write_files: &[],
            })
            .ok()?,
        )
    } else {
        None
    };
    let runner = Runner {
        cwd: base.root.clone(),
        allowlist: base.allowlist.clone(),
        timeout: base.timeout,
        max_output: VERSION_OUTPUT_CAP,
        tool_profile,
        tool_tmpdir: None,
    };
    // One at a time, stopping at the first failure: when `cc` fails the
    // answer is already "not checked", so `rustc` is never started (a hung
    // one would cost a whole timeout more). `perf run` needs both lines
    // whatever happens, so it reads them with [`compiler_lines`]; each read
    // here is the same [`first_line`] on the same runs.
    let cc = first_line(&runner, &["cc", "--version"])?;
    let rustc = first_line(&runner, &["rustc", "-V"])?;
    Some((cc, rustc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// A target whose `[oracle]` table is `oracle`.
    fn target(tmp: &TempDir, oracle: &str) -> TargetContext {
        let root = tmp.path();
        std::fs::write(
            root.join("harness.toml"),
            format!(
                "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n\
                 [oracle]\n{oracle}\n"
            ),
        )
        .expect("harness.toml");
        std::fs::create_dir_all(root.join("src")).expect("src");
        TargetContext::load(root).expect("target loads")
    }

    /// The compilers' lines come from tool runs, as perf run reads them;
    /// without both tools on the allowlist, or without a sandbox and the
    /// person's leave, nothing runs.
    #[test]
    fn the_compilers_are_read_as_tool_runs() {
        let tmp = TempDir::new("perf-compilers");
        let t = target(&tmp, "allowlist = [\"cc\", \"rustc\"]");
        if sandbox::sandbox_mode() == "sandbox-exec" {
            let (cc, rustc) = perf_compilers(&t, false).expect("both ran");
            assert!(!cc.is_empty() && cc.chars().count() <= 160, "{cc}");
            assert!(rustc.starts_with("rustc "), "{rustc}");
            // The same line an unsandboxed tool run gives: the sandbox
            // changes nothing in a version line. This is not perf run's
            // read (its Runner is steps.writing's, under the tool profile,
            // with VERSION_OUTPUT_CAP): that perf run stores the very lines
            // perf show reads is guarded by measure.rs's
            // a_run_over_verified_units_end_to_end.
            let runner = Runner {
                cwd: t.root.canonicalize().expect("root"),
                allowlist: vec!["rustc".into()],
                timeout: std::time::Duration::from_secs(120),
                max_output: VERSION_OUTPUT_CAP,
                tool_profile: None,
                tool_tmpdir: None,
            };
            assert_eq!(first_line(&runner, &["rustc", "-V"]), Some(rustc));
        } else {
            assert_eq!(perf_compilers(&t, false), None, "no sandbox, no leave");
            assert!(perf_compilers(&t, true).is_some());
        }
        let only_cc = TempDir::new("perf-compilers-cc");
        let t = target(&only_cc, "allowlist = [\"cc\"]");
        assert_eq!(perf_compilers(&t, true), None, "rustc is not allowlisted");
        let bad = TempDir::new("perf-compilers-timeout");
        let t = target(&bad, "allowlist = [\"cc\", \"rustc\"]\ntimeout_secs = 0");
        assert_eq!(perf_compilers(&t, true), None, "an unusable timeout");
    }

    /// A tool that fails, prints nothing, or is not on the allowlist gives
    /// no line; a long first line is cut at 160 characters.
    #[test]
    fn a_first_line_or_none() {
        let tmp = TempDir::new("perf-first-line");
        let runner = Runner {
            cwd: tmp.path().to_path_buf(),
            allowlist: vec!["sh".into()],
            timeout: std::time::Duration::from_secs(30),
            max_output: VERSION_OUTPUT_CAP,
            tool_profile: None,
            tool_tmpdir: None,
        };
        let long = "x".repeat(300);
        let script = format!("printf '  {long}  \\nsecond\\n'");
        let line = first_line(&runner, &["sh", "-c", &script]).expect("a line");
        assert_eq!(line, "x".repeat(160));
        assert_eq!(first_line(&runner, &["sh", "-c", "exit 0"]), None);
        assert_eq!(first_line(&runner, &["sh", "-c", "echo v; exit 3"]), None);
        assert_eq!(first_line(&runner, &["cc", "--version"]), None);
    }
}
