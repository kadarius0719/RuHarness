//! The oracle sandbox (docs/SCHEMAS.md "Trust boundaries"): on macOS every
//! build and every run of target- or model-derived code is wrapped in
//! `/usr/bin/sandbox-exec` with a generated SBPL profile.
//!
//! Two profile shapes:
//! - the **tool profile** ([`render_profile`]) for `cc`/`cargo`/`rustc`/`nm`:
//!   network denied, reads under the user's home denied except the target
//!   root, the Rust toolchain dirs and the harness's work folder (every tool
//!   child's working folder), writes confined to explicitly listed
//!   locations and temp;
//! - the **map profile** ([`render_map_profile`], docs/PROJECT-MAP-DESIGN.md
//!   §3.9) for the project map's compiler: network denied, reads of
//!   `/Users`, `/Volumes`, `/private/tmp` and `/private/var/tmp` denied
//!   except the project root and one fresh folder, the cargo and rustup
//!   homes denied wherever they are, and writes only to that fresh folder;
//! - the **run profile** ([`render_run_profile`], M4 R1 run confinement) for
//!   every binary the oracle built: network denied, `exec` of nothing but
//!   the binary itself, reads denied under the user's home AND under the
//!   target root except the binary and explicitly listed input files, and
//!   writes confined to one fresh per-run temp dir. A candidate can thereby
//!   no longer read an earlier run's output (e.g. the build dir's
//!   `drv_c.out`) and replay it.
//!
//! Profiles are rendered from canonical paths only; a path containing a
//! double quote or a backslash is rejected rather than escaped, so a hostile
//! directory name can never break out of an SBPL string literal.

use harness_core::error::Error;
use std::path::{Path, PathBuf};

/// Absolute path of the macOS sandbox wrapper.
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The sandbox mode this build of the oracle applies to child processes:
/// `"sandbox-exec"` when `/usr/bin/sandbox-exec` exists (macOS), otherwise
/// `"none"`. Recorded in every verdict as `sandbox: <mode>` in
/// `inputs.toolchain`; the CLI decides policy for `none`.
pub fn sandbox_mode() -> &'static str {
    if Path::new(SANDBOX_EXEC).exists() {
        "sandbox-exec"
    } else {
        "none"
    }
}

/// Host facts a profile is rendered against, all canonicalized.
#[derive(Debug, Clone)]
pub(crate) struct HostDirs {
    /// The user's home directory (reads beneath it are denied by default).
    pub home: PathBuf,
    /// `CARGO_HOME`, else `<home>/.cargo` — `None` when it does not exist.
    pub cargo_home: Option<PathBuf>,
    /// `RUSTUP_HOME`, else `<home>/.rustup` — `None` when it does not exist.
    pub rustup_home: Option<PathBuf>,
    /// `TMPDIR`, when set and existing.
    pub tmpdir: Option<PathBuf>,
    /// perf's launcher cache (docs/PERF-DESIGN.md §3.2): no profile may
    /// write under it (build note 5).
    pub perf_cache: PathBuf,
    /// The harness's work folder ([`work_root`]): every cargo, rustc and
    /// compiler child starts there. A read root of the toolchain profile;
    /// no profile may write it or any folder above it.
    pub work: PathBuf,
}

/// The harness's work folder under `home` (docs/PROJECT-MAP-DESIGN.md §3.7):
/// `~/Library/Caches/ruharness/work` on macOS (with `Library/Caches`
/// canonical when it exists), `~/.cache/ruharness/work` elsewhere. Tool
/// children start there, so a project's `.cargo/config.toml` and
/// `rust-toolchain.toml` (read from the working folder upward) are never
/// read.
pub(crate) fn work_root(home: &Path) -> PathBuf {
    let caches = if cfg!(target_os = "macos") {
        home.join("Library").join("Caches")
    } else {
        home.join(".cache")
    };
    caches
        .canonicalize()
        .unwrap_or(caches)
        .join("ruharness")
        .join("work")
}

/// perf's launcher cache root under `home`: `~/Library/Caches/ruharness/
/// perf`, with `Library/Caches` canonical when it exists.
pub(crate) fn perf_cache_root(home: &Path) -> PathBuf {
    let caches = home.join("Library").join("Caches");
    caches
        .canonicalize()
        .unwrap_or(caches)
        .join("ruharness")
        .join("perf")
}

/// The line every rendered profile ends its write rules with: nothing a
/// sandboxed child runs may write perf's launcher cache (build note 5), the
/// harness's work folder, or a folder between the work folder and the home
/// folder (a child that could move one could put a `.cargo/config.toml`
/// where cargo looks).
fn harness_dirs_tail(host: &HostDirs) -> Result<String, Error> {
    let mut out = format!(
        "(deny file-write* (subpath {}) (subpath {})",
        sbpl_string(&host.perf_cache)?,
        sbpl_string(&host.work)?
    );
    // Up to the home folder: no write rule reaches above it.
    for anc in host.work.ancestors().skip(1) {
        if anc.starts_with(&host.home) {
            out.push_str(&format!(" (literal {})", sbpl_string(anc)?));
        }
    }
    out.push_str(")\n");
    Ok(out)
}

impl HostDirs {
    /// Read `HOME`, `CARGO_HOME`, `RUSTUP_HOME` and `TMPDIR` from the process
    /// environment. Fails closed when `HOME` is unset or does not resolve: a
    /// profile that cannot name the home directory cannot protect it.
    pub(crate) fn from_env() -> Result<HostDirs, Error> {
        let home_raw = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error::Invariant("sandbox: HOME is not set; cannot build a sandbox profile".into())
            })?;
        let home = home_raw
            .canonicalize()
            .map_err(|e| Error::io(&home_raw, e))?;
        let or_default = |key: &str, default: &str| -> Option<PathBuf> {
            let p = std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(default));
            p.canonicalize().ok()
        };
        Ok(HostDirs {
            perf_cache: perf_cache_root(&home),
            work: work_root(&home),
            cargo_home: or_default("CARGO_HOME", ".cargo"),
            rustup_home: or_default("RUSTUP_HOME", ".rustup"),
            tmpdir: std::env::var_os("TMPDIR")
                .filter(|v| !v.is_empty())
                .and_then(|v| PathBuf::from(v).canonicalize().ok()),
            home,
        })
    }
}

/// What one class of child process may touch.
#[derive(Debug, Clone)]
pub(crate) struct ProfileSpec<'a> {
    /// Host directories.
    pub host: &'a HostDirs,
    /// Canonical target root (always readable).
    pub target_root: &'a Path,
    /// True for toolchain invocations (`cargo`, `rustc`, `cc`, `nm`): also
    /// allows reading the Rust toolchain dirs and the narrow rustup/cargo
    /// path-walk exceptions. False for built binaries, which need neither.
    pub toolchain: bool,
    /// Canonical directories the child may write beneath.
    pub write_dirs: &'a [PathBuf],
    /// Canonical single files the child may create or write.
    pub write_files: &'a [PathBuf],
}

/// Render the SBPL profile for `spec`. SBPL is last-match-wins, so each
/// `deny` is followed by the narrower `allow`s that carve exceptions from it.
pub(crate) fn render_profile(spec: &ProfileSpec<'_>) -> Result<String, Error> {
    let home = spec.host.home.as_path();
    let mut out = String::new();
    out.push_str("(version 1)\n(allow default)\n(deny network*)\n");
    out.push_str(&format!(
        "(deny file-read* (subpath {}))\n",
        sbpl_string(home)?
    ));

    let mut read_roots: Vec<&Path> = vec![spec.target_root];
    // What a child may write it may read back: the harness's own folders
    // (a features map's random folder holds the probe header every compile
    // `-include`s) and the temp dir its compiler's own temp files go to —
    // under the home folder too (review: a TMPDIR there broke every build).
    // Only one the home denial hides and no read root holds already; never
    // a TMPDIR that is the home folder or holds it.
    let tmp = spec.host.tmpdir.as_deref().filter(|t| !home.starts_with(t));
    for dir in spec.write_dirs.iter().map(PathBuf::as_path).chain(tmp) {
        if dir.starts_with(home) && !read_roots.iter().any(|r| dir.starts_with(r)) {
            read_roots.push(dir);
        }
    }
    if spec.toolchain {
        // The work folder is every tool child's working folder (cargo
        // canonicalizes it), so it is read like the toolchain's own.
        if !read_roots.iter().any(|r| spec.host.work.starts_with(r)) {
            read_roots.push(&spec.host.work);
        }
        read_roots.extend(spec.host.cargo_home.as_deref());
        read_roots.extend(spec.host.rustup_home.as_deref());

        // Narrow exception 1 (found empirically): cargo and the rustup proxy
        // canonicalize the manifest/cwd paths, which lstat()s every ancestor
        // directory. With the home subtree denied that fails with EPERM
        // ("could not parse/generate dep info"), so allow METADATA ONLY — no
        // directory listing, no file contents — on the ancestors of the
        // readable roots that lie inside the home directory.
        let mut ancestors: Vec<PathBuf> = Vec::new();
        for root in &read_roots {
            for anc in root.ancestors().skip(1) {
                if anc.starts_with(home) && !ancestors.iter().any(|a| a == anc) {
                    ancestors.push(anc.to_path_buf());
                }
            }
        }
        ancestors.sort();
        if !ancestors.is_empty() {
            out.push_str("(allow file-read-metadata");
            for anc in &ancestors {
                out.push_str(&format!(" (literal {})", sbpl_string(anc)?));
            }
            out.push_str(")\n");
        }

        // No exception for an ancestor `rust-toolchain(.toml)`: every tool
        // child starts in the work folder with `RUSTUP_TOOLCHAIN` pinned
        // (exec.rs, `tool_env`), so rustup never looks for one.
    }

    out.push_str("(allow file-read*");
    for root in &read_roots {
        out.push_str(&format!(" (subpath {})", sbpl_string(root)?));
    }
    out.push_str(")\n");
    if spec.toolchain {
        if let Some(cargo_home) = &spec.host.cargo_home {
            // CARGO_HOME is readable for the toolchain's sake, but registry
            // tokens live there too. An offline, dependency-free build never
            // needs them, and target-controlled build steps (a hostile
            // `.cargo/config.toml` wrapper, a build script) must not see them.
            out.push_str("(deny file-read*");
            for name in ["credentials.toml", "credentials"] {
                out.push_str(&format!(
                    " (literal {})",
                    sbpl_string(&cargo_home.join(name))?
                ));
            }
            out.push_str(")\n");
        }
    }

    out.push_str("(deny file-write* (subpath \"/\"))\n(allow file-write*");
    for dir in spec.write_dirs {
        out.push_str(&format!(" (subpath {})", sbpl_string(dir)?));
    }
    for file in spec.write_files {
        out.push_str(&format!(" (literal {})", sbpl_string(file)?));
    }
    // Temp dirs (canonical spellings: /tmp and /var are symlinks into
    // /private on macOS) and the three device nodes toolchains write to.
    out.push_str(" (subpath \"/private/tmp\") (subpath \"/private/var/folders\")");
    if let Some(tmp) = &spec.host.tmpdir {
        out.push_str(&format!(" (subpath {})", sbpl_string(tmp)?));
    }
    out.push_str(
        " (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n",
    );
    out.push_str(&harness_dirs_tail(spec.host)?);
    out.push_str(NO_STARTS_THROUGH_THE_SYSTEM);
    Ok(out)
}

/// The places the map profile denies reading, whole, wherever the project
/// lies: every person's home folder and `/Users/Shared`, mounted volumes,
/// and the shared temporary folders (`/tmp` is a link to the first;
/// `/private/var/tmp` is world-writable and kept across restarts).
const MAP_DENIED_READS: [&str; 4] = ["/Users", "/Volumes", "/private/tmp", "/private/var/tmp"];

/// What the project map's compiler may touch (docs/PROJECT-MAP-DESIGN.md
/// §3.9).
#[derive(Debug, Clone)]
pub(crate) struct MapSpec<'a> {
    /// Host directories.
    pub host: &'a HostDirs,
    /// Canonical project root: readable, never writable.
    pub project_root: &'a Path,
    /// The map's fresh, canonical folder: the only writable place (its
    /// objects and dependency lists; `TMPDIR` points at it).
    pub fresh: &'a Path,
}

/// The map profile, its own renderer (the tool profile always adds the
/// temporary folders to its writable list): network denied; reads of
/// `/Users`, `/Volumes`, `/private/tmp`, `/private/var/tmp` and the home
/// folder denied except the project root, the fresh folder and the work
/// folder (the compiler's working folder, empty), with metadata only on
/// the folders above them;
/// `/private/var/folders` stays readable (the system's per-user caches);
/// the cargo and rustup homes denied wherever they are (a custom
/// `CARGO_HOME` holds credentials; `cc` needs neither); writes only to the
/// fresh folder and three device nodes; nothing started through the system.
pub(crate) fn render_map_profile(spec: &MapSpec<'_>) -> Result<String, Error> {
    let home = spec.host.home.as_path();
    let mut denied: Vec<&Path> = MAP_DENIED_READS.iter().map(Path::new).collect();
    if !denied.iter().any(|d| home.starts_with(d)) {
        denied.push(home);
    }
    let mut out = String::new();
    out.push_str("(version 1)\n(allow default)\n(deny network*)\n(deny file-read*");
    for dir in &denied {
        out.push_str(&format!(" (subpath {})", sbpl_string(dir)?));
    }
    out.push_str(")\n");

    let read_roots: [&Path; 3] = [spec.project_root, spec.fresh, &spec.host.work];
    let mut ancestors: Vec<&Path> = Vec::new();
    for root in read_roots {
        for anc in root.ancestors().skip(1) {
            if denied.iter().any(|d| anc.starts_with(d)) && !ancestors.contains(&anc) {
                ancestors.push(anc);
            }
        }
    }
    ancestors.sort();
    if !ancestors.is_empty() {
        out.push_str("(allow file-read-metadata");
        for anc in &ancestors {
            out.push_str(&format!(" (literal {})", sbpl_string(anc)?));
        }
        out.push_str(")\n");
    }
    out.push_str("(allow file-read*");
    for root in read_roots {
        out.push_str(&format!(" (subpath {})", sbpl_string(root)?));
    }
    out.push_str(")\n");
    let homes: Vec<&Path> = [&spec.host.cargo_home, &spec.host.rustup_home]
        .into_iter()
        .flatten()
        .map(PathBuf::as_path)
        .collect();
    if !homes.is_empty() {
        out.push_str("(deny file-read*");
        for dir in homes {
            out.push_str(&format!(" (subpath {})", sbpl_string(dir)?));
        }
        out.push_str(")\n");
    }
    out.push_str(&format!(
        "(deny file-write* (subpath \"/\"))\n(allow file-write* (subpath {}) \
         (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n",
        sbpl_string(spec.fresh)?
    ));
    out.push_str(&harness_dirs_tail(spec.host)?);
    out.push_str(NO_STARTS_THROUGH_THE_SYSTEM);
    Ok(out)
}

/// What one run of a built binary may touch (R1 run confinement).
#[derive(Debug, Clone)]
pub(crate) struct RunSpec<'a> {
    /// Host directories (the home directory is denied).
    pub host: &'a HostDirs,
    /// Canonical target root (denied, wherever it lives).
    pub target_root: &'a Path,
    /// Canonical path of the binary: the only program the run may `exec`,
    /// and readable (the loader maps it).
    pub bin: &'a Path,
    /// Canonical input files the run may read (a whole-program sample).
    pub read_files: &'a [PathBuf],
    /// The fresh, canonical per-run temp dir: the only writable location.
    pub tmpdir: &'a Path,
}

/// Render the run profile for `spec`. SBPL is last-match-wins, so each deny
/// precedes the narrower allows that carve its exceptions:
/// - `exec` of anything but the binary is denied (the allow is also what
///   lets `sandbox-exec` start it);
/// - reads under the home directory and under the target root are denied,
///   except the binary, the listed inputs and the run's own temp dir;
/// - writes are denied everywhere but the run's temp dir and three device
///   nodes (`/dev/null`, `/dev/tty`, `/dev/dtracehelper`).
pub(crate) fn render_run_profile(spec: &RunSpec<'_>) -> Result<String, Error> {
    let bin = sbpl_string(spec.bin)?;
    let tmp = sbpl_string(spec.tmpdir)?;
    let mut out = String::new();
    out.push_str("(version 1)\n(allow default)\n(deny network*)\n");
    out.push_str("(deny process-exec*)\n");
    out.push_str(&format!("(allow process-exec (literal {bin}))\n"));
    out.push_str(&format!(
        "(deny file-read* (subpath {}) (subpath {}))\n",
        sbpl_string(&spec.host.home)?,
        sbpl_string(spec.target_root)?
    ));
    out.push_str(&format!("(allow file-read* (literal {bin})"));
    for file in spec.read_files {
        out.push_str(&format!(" (literal {})", sbpl_string(file)?));
    }
    out.push_str(&format!(" (subpath {tmp}))\n"));
    out.push_str(&format!(
        "(deny file-write* (subpath \"/\"))\n(allow file-write* (subpath {tmp}) \
         (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n"
    ));
    out.push_str(&harness_dirs_tail(spec.host)?);
    out.push_str(NO_STARTS_THROUGH_THE_SYSTEM);
    Ok(out)
}

/// What one perf run may touch (docs/PERF-DESIGN.md §3.4).
#[derive(Debug, Clone)]
pub(crate) struct PerfSpec<'a> {
    /// Host directories (the home directory is denied).
    pub host: &'a HostDirs,
    /// Canonical target root (denied, wherever it lives).
    pub target_root: &'a Path,
    /// Canonical path of the side's program.
    pub bin: &'a Path,
    /// Canonical path of perfgo, in the launcher cache.
    pub perfgo: &'a Path,
    /// The run's fresh, canonical temp dir: the only writable location.
    pub tmpdir: &'a Path,
}

/// The perf profile (§3.4): the scenario profile with `exec` allowed for
/// exactly perfgo and the side's program, reads of exactly those two and
/// the run's temp dir under the home folder and the target, no signal but
/// to itself, a fork killed on trying, and no program started for it by the
/// system ([`NO_STARTS_THROUGH_THE_SYSTEM`]).
pub(crate) fn render_perf_profile(spec: &PerfSpec<'_>) -> Result<String, Error> {
    let bin = sbpl_string(spec.bin)?;
    let perfgo = sbpl_string(spec.perfgo)?;
    let tmp = sbpl_string(spec.tmpdir)?;
    let mut out = String::new();
    out.push_str("(version 1)\n(allow default)\n(deny network*)\n");
    out.push_str("(deny process-exec*)\n");
    out.push_str(&format!(
        "(allow process-exec (literal {perfgo}) (literal {bin}))\n"
    ));
    out.push_str(&format!(
        "(deny file-read* (subpath {}) (subpath {}))\n",
        sbpl_string(&spec.host.home)?,
        sbpl_string(spec.target_root)?
    ));
    out.push_str(&format!(
        "(allow file-read* (literal {bin}) (literal {perfgo}) (subpath {tmp}))\n"
    ));
    out.push_str(&format!(
        "(deny file-write* (subpath \"/\"))\n(allow file-write* (subpath {tmp}) \
         (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n"
    ));
    out.push_str(&harness_dirs_tail(spec.host)?);
    out.push_str(
        "(deny signal)\n(allow signal (target self))\n(deny process-fork (with send-signal SIGKILL))\n",
    );
    out.push_str(NO_STARTS_THROUGH_THE_SYSTEM);
    Ok(out)
}

/// What stops a sandboxed program from having the system start a program
/// for it — started by launchd, outside the sandbox and the program's group,
/// so it would outlive the run with the person's own rights: opening an app,
/// a document or a web address (LaunchServices), sending Apple events, and
/// submitting a launchd job are denied, and so is reaching the services
/// that do them (LaunchServices and Core Services, the Apple event server,
/// login items and helper registration). A plain C or Rust program uses
/// none of them; other same-user services stay reachable (named in
/// SCHEMAS' perf trust boundaries). The perf profile ends with it.
pub(crate) const NO_STARTS_THROUGH_THE_SYSTEM: &str = "\
(deny lsopen appleevent-send job-creation)
(deny mach-lookup (global-name-prefix \"com.apple.coreservices.\") \
(global-name-prefix \"com.apple.CoreServices.\") (global-name \"com.apple.coreservicesd\") \
(global-name-prefix \"com.apple.lsd.\") (global-name \"com.apple.xpc.smd\") \
(global-name \"com.apple.xpc.loginitemregisterd\"))
";

/// The scenario run's profile (docs/FEATURES-DESIGN.md §4.1 step 4): the run
/// profile, plus no signal to any process but itself (`(target others)`
/// alone would mean "outside the run's process group", which the run can
/// change) and no fork — applied to the C side and the mixed side alike.
pub(crate) fn render_scenario_profile(spec: &RunSpec<'_>) -> Result<String, Error> {
    let mut out = render_run_profile(spec)?;
    out.push_str(SCENARIO_PROFILE_TAIL);
    Ok(out)
}

/// What a scenario run's profile adds to the run profile.
pub(crate) const SCENARIO_PROFILE_TAIL: &str =
    "(deny signal)\n(allow signal (target self))\n(deny process-fork)\n";

/// Quote a path as an SBPL string literal. Paths must be absolute UTF-8 and
/// free of `"`, `\` and control characters — such a path is refused outright
/// (never escaped), so profile text cannot be injected through a directory
/// name.
pub(crate) fn sbpl_string(path: &Path) -> Result<String, Error> {
    let s = path.to_str().ok_or_else(|| {
        Error::Invariant(format!(
            "sandbox: non-UTF-8 path cannot appear in a profile: {}",
            path.display()
        ))
    })?;
    if !path.is_absolute() {
        return Err(Error::Invariant(format!(
            "sandbox: profile paths must be absolute, got {s:?}"
        )));
    }
    if let Some(bad) = s
        .chars()
        .find(|c| *c == '"' || *c == '\\' || c.is_control())
    {
        return Err(Error::Invariant(format!(
            "sandbox: path {s:?} contains {bad:?}, which cannot be represented safely in a \
             sandbox profile; move the target to a path without quotes, backslashes or \
             control characters"
        )));
    }
    Ok(format!("\"{s}\""))
}

/// Wrap `argv` for execution under `profile`:
/// `["/usr/bin/sandbox-exec", "-p", <profile>, <argv…>]`. `sandbox-exec`
/// applies the profile and then `exec`s the command in place, so the child's
/// pid, exit status and signals are the wrapped command's own.
pub(crate) fn wrap(profile: &str, argv: &[String]) -> Vec<String> {
    let mut wrapped = Vec::with_capacity(argv.len() + 3);
    wrapped.push(SANDBOX_EXEC.to_string());
    wrapped.push("-p".to_string());
    wrapped.push(profile.to_string());
    wrapped.extend(argv.iter().cloned());
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostDirs {
        HostDirs {
            home: PathBuf::from("/Users/u"),
            cargo_home: Some(PathBuf::from("/Users/u/.cargo")),
            rustup_home: Some(PathBuf::from("/Users/u/.rustup")),
            tmpdir: Some(PathBuf::from("/private/var/folders/xy/T")),
            perf_cache: PathBuf::from("/Users/u/Library/Caches/ruharness/perf"),
            work: PathBuf::from("/Users/u/Library/Caches/ruharness/work"),
        }
    }

    /// The perf profile (docs/PERF-DESIGN.md §3.4): exec of exactly perfgo
    /// and the side's program, reads of exactly those and the temp dir,
    /// writes only the temp dir and never the launcher cache, no signal out,
    /// a fork killed, nothing opened or started through the system (what
    /// macOS does with these rules is tested live in `perf::launcher`).
    #[test]
    fn the_perf_profile() {
        let host = host();
        let text = render_perf_profile(&PerfSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            bin: Path::new("/Users/u/t/migration/build/.perf/bin/p001/tool"),
            perfgo: Path::new("/Users/u/Library/Caches/ruharness/perf/perf-launcher-1-ab/perfgo"),
            tmpdir: Path::new("/private/var/folders/xy/T/ruharness-perf-1"),
        })
        .expect("renders");
        let expected = "\
(version 1)
(allow default)
(deny network*)
(deny process-exec*)
(allow process-exec (literal \"/Users/u/Library/Caches/ruharness/perf/perf-launcher-1-ab/perfgo\") (literal \"/Users/u/t/migration/build/.perf/bin/p001/tool\"))
(deny file-read* (subpath \"/Users/u\") (subpath \"/Users/u/t\"))
(allow file-read* (literal \"/Users/u/t/migration/build/.perf/bin/p001/tool\") (literal \"/Users/u/Library/Caches/ruharness/perf/perf-launcher-1-ab/perfgo\") (subpath \"/private/var/folders/xy/T/ruharness-perf-1\"))
(deny file-write* (subpath \"/\"))
(allow file-write* (subpath \"/private/var/folders/xy/T/ruharness-perf-1\") (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))
(deny file-write* (subpath \"/Users/u/Library/Caches/ruharness/perf\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library\") (literal \"/Users/u\"))
(deny signal)
(allow signal (target self))
(deny process-fork (with send-signal SIGKILL))
(deny lsopen appleevent-send job-creation)
(deny mach-lookup (global-name-prefix \"com.apple.coreservices.\") (global-name-prefix \"com.apple.CoreServices.\") (global-name \"com.apple.coreservicesd\") (global-name-prefix \"com.apple.lsd.\") (global-name \"com.apple.xpc.smd\") (global-name \"com.apple.xpc.loginitemregisterd\"))
";
        assert_eq!(text, expected);
    }

    #[test]
    fn mode_matches_wrapper_presence() {
        let expected = if Path::new(SANDBOX_EXEC).exists() {
            "sandbox-exec"
        } else {
            "none"
        };
        assert_eq!(sandbox_mode(), expected);
    }

    #[test]
    fn tool_profile_has_the_normative_rules_in_last_match_wins_order() {
        let host = host();
        let write_dirs = vec![
            PathBuf::from("/Users/u/t/migration/build/u1"),
            PathBuf::from("/Users/u/t/migration/units/u1/c/target"),
        ];
        let write_files = vec![PathBuf::from("/Users/u/t/migration/units/u1/c/Cargo.lock")];
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            toolchain: true,
            write_dirs: &write_dirs,
            write_files: &write_files,
        })
        .expect("renders");
        let expected = "\
(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath \"/Users/u\"))
(allow file-read-metadata (literal \"/Users/u\") (literal \"/Users/u/Library\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library/Caches/ruharness\"))
(allow file-read* (subpath \"/Users/u/t\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (subpath \"/Users/u/.cargo\") (subpath \"/Users/u/.rustup\"))
(deny file-read* (literal \"/Users/u/.cargo/credentials.toml\") (literal \"/Users/u/.cargo/credentials\"))
(deny file-write* (subpath \"/\"))
(allow file-write* (subpath \"/Users/u/t/migration/build/u1\") (subpath \"/Users/u/t/migration/units/u1/c/target\") (literal \"/Users/u/t/migration/units/u1/c/Cargo.lock\") (subpath \"/private/tmp\") (subpath \"/private/var/folders\") (subpath \"/private/var/folders/xy/T\") (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))
(deny file-write* (subpath \"/Users/u/Library/Caches/ruharness/perf\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library\") (literal \"/Users/u\"))
(deny lsopen appleevent-send job-creation)
(deny mach-lookup (global-name-prefix \"com.apple.coreservices.\") (global-name-prefix \"com.apple.CoreServices.\") (global-name \"com.apple.coreservicesd\") (global-name-prefix \"com.apple.lsd.\") (global-name \"com.apple.xpc.smd\") (global-name \"com.apple.xpc.loginitemregisterd\"))
";
        assert_eq!(text, expected);
    }

    /// Review: a TMPDIR under the home folder, and a write dir outside the
    /// target there (the features map's random folder), are readable — the
    /// compiler reads back its own temp files and the probe header.
    #[test]
    fn a_tool_reads_back_what_it_writes_under_home() {
        let mut host = host();
        host.tmpdir = Some(PathBuf::from("/Users/u/tmp"));
        let write_dirs = vec![PathBuf::from("/Users/u/tmp/ruharness-map-0123456789abcdef")];
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            toolchain: true,
            write_dirs: &write_dirs,
            write_files: &[],
        })
        .expect("renders");
        assert!(
            text.contains(
                "(allow file-read* (subpath \"/Users/u/t\") (subpath \"/Users/u/tmp/ruharness-map-0123456789abcdef\") (subpath \"/Users/u/tmp\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (subpath \"/Users/u/.cargo\")"
            ),
            "{text}"
        );
        // A TMPDIR that is the home folder opens nothing.
        host.tmpdir = Some(PathBuf::from("/Users/u"));
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            toolchain: false,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("renders");
        assert!(
            text.contains("(allow file-read* (subpath \"/Users/u/t\"))\n"),
            "{text}"
        );
    }

    #[test]
    fn a_non_toolchain_profile_reads_only_the_target_root() {
        let host = host();
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/code/t"),
            toolchain: false,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("renders");
        assert!(text.contains("(deny network*)"), "{text}");
        assert!(!text.contains("process-exec"), "{text}");
        assert!(
            text.contains("(allow file-read* (subpath \"/Users/u/code/t\"))\n"),
            "{text}"
        );
        assert!(!text.contains(".cargo"), "{text}");
        assert!(!text.contains(".rustup"), "{text}");
        assert!(!text.contains("file-read-metadata"), "{text}");
        assert!(!text.contains("rust-toolchain"), "{text}");
    }

    #[test]
    fn ancestors_outside_home_get_no_rules() {
        let host = host();
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/private/tmp/t"),
            toolchain: true,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("renders");
        // Only the toolchain dirs' and the work folder's ancestors need
        // metadata.
        assert!(
            text.contains(
                "(allow file-read-metadata (literal \"/Users/u\") (literal \"/Users/u/Library\") \
                 (literal \"/Users/u/Library/Caches\") \
                 (literal \"/Users/u/Library/Caches/ruharness\"))\n"
            ),
            "{text}"
        );
        assert!(!text.contains("rust-toolchain"), "{text}");
    }

    #[test]
    fn nested_target_lists_every_home_ancestor_once() {
        let host = host();
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/code/repo/targets/z"),
            toolchain: true,
            write_dirs: &[],
            write_files: &[],
        })
        .expect("renders");
        assert!(
            text.contains(
                "(allow file-read-metadata (literal \"/Users/u\") (literal \"/Users/u/Library\") \
                 (literal \"/Users/u/Library/Caches\") \
                 (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/code\") \
                 (literal \"/Users/u/code/repo\") (literal \"/Users/u/code/repo/targets\"))\n"
            ),
            "{text}"
        );
        // No ancestor's rust-toolchain file is readable any more: tools
        // start in the work folder with the toolchain pinned.
        assert!(!text.contains("rust-toolchain"), "{text}");
        // Nothing above the home directory is mentioned.
        assert!(!text.contains("(literal \"/Users\")"), "{text}");
    }

    #[test]
    fn hostile_path_characters_are_rejected_not_escaped() {
        for bad in ["/t/a\"b", "/t/a\\b", "/t/a\nb"] {
            let err = sbpl_string(Path::new(bad)).expect_err("must be rejected");
            assert!(err.to_string().contains("sandbox"), "{err}");
        }
        let err = sbpl_string(Path::new("relative/p")).expect_err("relative rejected");
        assert!(err.to_string().contains("absolute"), "{err}");
        assert_eq!(
            sbpl_string(Path::new("/ok/with space")).expect("plain path"),
            "\"/ok/with space\""
        );

        let host = host();
        let write_dirs = vec![PathBuf::from("/t/x\") (subpath \"/")];
        let err = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/t"),
            toolchain: false,
            write_dirs: &write_dirs,
            write_files: &[],
        })
        .expect_err("injection attempt must fail");
        assert!(err.to_string().contains("sandbox"), "{err}");
    }

    /// The whole run profile, byte for byte: exec only the binary, no reads
    /// under home or the target root beyond the binary, the listed inputs and
    /// the run's temp dir, writes only to that temp dir.
    #[test]
    fn the_run_profile_confines_a_built_binary() {
        let host = host();
        let bin = PathBuf::from("/Users/u/t/migration/build/u1/drv_rs");
        let inputs = vec![PathBuf::from(
            "/Users/u/t/migration/build/u1/sample_text.txt",
        )];
        let text = render_run_profile(&RunSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            bin: &bin,
            read_files: &inputs,
            tmpdir: Path::new("/private/var/folders/xy/T/ruharness-run-1-0"),
        })
        .expect("renders");
        let expected = "\
(version 1)
(allow default)
(deny network*)
(deny process-exec*)
(allow process-exec (literal \"/Users/u/t/migration/build/u1/drv_rs\"))
(deny file-read* (subpath \"/Users/u\") (subpath \"/Users/u/t\"))
(allow file-read* (literal \"/Users/u/t/migration/build/u1/drv_rs\") (literal \"/Users/u/t/migration/build/u1/sample_text.txt\") (subpath \"/private/var/folders/xy/T/ruharness-run-1-0\"))
(deny file-write* (subpath \"/\"))
(allow file-write* (subpath \"/private/var/folders/xy/T/ruharness-run-1-0\") (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))
(deny file-write* (subpath \"/Users/u/Library/Caches/ruharness/perf\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library\") (literal \"/Users/u\"))
(deny lsopen appleevent-send job-creation)
(deny mach-lookup (global-name-prefix \"com.apple.coreservices.\") (global-name-prefix \"com.apple.CoreServices.\") (global-name \"com.apple.coreservicesd\") (global-name-prefix \"com.apple.lsd.\") (global-name \"com.apple.xpc.smd\") (global-name \"com.apple.xpc.loginitemregisterd\"))
";
        assert_eq!(text, expected);

        // A target root outside the home directory is denied just the same.
        let text = render_run_profile(&RunSpec {
            host: &host,
            target_root: Path::new("/private/var/folders/xy/T/target"),
            bin: Path::new("/private/var/folders/xy/T/target/b"),
            read_files: &[],
            tmpdir: Path::new("/private/var/folders/xy/T/run"),
        })
        .expect("renders");
        assert!(
            text.contains("(subpath \"/private/var/folders/xy/T/target\"))\n"),
            "{text}"
        );
        assert!(!text.contains("/private/tmp"), "{text}");

        // Hostile characters are refused here too.
        let bad = PathBuf::from("/t/x\") (subpath \"/");
        assert!(render_run_profile(&RunSpec {
            host: &host,
            target_root: Path::new("/t"),
            bin: Path::new("/t/b"),
            read_files: std::slice::from_ref(&bad),
            tmpdir: Path::new("/tmp/r"),
        })
        .is_err());
    }

    /// Live (macOS, sandbox-exec): a program under any of verify's profiles
    /// — the run profile, the scenario profile built on it, and the tool
    /// profile — can have nothing started for it by the system either: the
    /// gap the perf review found in these profiles, closed by the same rule.
    /// Each profile is asked on its own: the scenario profile's text has no
    /// golden of its own, so only this run shows it keeps the rule.
    #[test]
    fn a_run_cannot_open_or_start_anything_through_the_system() {
        if !cfg!(target_os = "macos") || !Path::new(SANDBOX_EXEC).exists() {
            return;
        }
        let tmp = crate::testutil::TempDir::new("run-no-open");
        let dir = tmp.path().canonicalize().expect("tmp");
        let src = dir.join("opener.c");
        std::fs::write(
            &src,
            "#include <servers/bootstrap.h>\n#include <stdio.h>\n#include <unistd.h>\n\
             int sandbox_check(pid_t pid, const char *operation, int type, ...);\n\
             int main(void) {\n\
             const char *ops[] = { \"lsopen\", \"appleevent-send\", \"job-creation\" };\n\
             for (int i = 0; i < 3; i++) printf(\"%s %d\\n\", ops[i], sandbox_check(getpid(), ops[i], 0));\n\
             mach_port_t p = MACH_PORT_NULL;\n\
             printf(\"lsd %d\\n\", bootstrap_look_up(bootstrap_port, \"com.apple.lsd.open\", &p) == BOOTSTRAP_NOT_PRIVILEGED);\n\
             return 0; }\n",
        )
        .expect("source");
        let bin = dir.join("opener");
        let cc = std::process::Command::new("/usr/bin/cc")
            .args(["-w", "-o"])
            .arg(&bin)
            .arg(&src)
            .status()
            .expect("cc");
        assert!(cc.success());
        let host = HostDirs::from_env().expect("host");
        let run_tmp = dir.join("run");
        std::fs::create_dir(&run_tmp).expect("run dir");
        let run = RunSpec {
            host: &host,
            target_root: &dir,
            bin: &bin,
            read_files: &[],
            tmpdir: &run_tmp,
        };
        let tool = ProfileSpec {
            host: &host,
            target_root: &dir,
            toolchain: false,
            write_dirs: std::slice::from_ref(&run_tmp),
            write_files: &[],
        };
        for (name, profile) in [
            ("run", render_run_profile(&run)),
            ("scenario", render_scenario_profile(&run)),
            ("tool", render_profile(&tool)),
        ] {
            let profile = profile.expect("renders");
            let argv = wrap(&profile, &[bin.to_string_lossy().into_owned()]);
            let out = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .output()
                .expect("sandbox-exec");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                "lsopen 1\nappleevent-send 1\njob-creation 1\nlsd 1\n",
                "the {name} profile: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// The map profile, byte for byte (docs/PROJECT-MAP-DESIGN.md §3.9).
    #[test]
    fn the_map_profile() {
        let host = host();
        let text = render_map_profile(&MapSpec {
            host: &host,
            project_root: Path::new("/Users/u/code/lz4"),
            fresh: Path::new("/private/var/folders/xy/T/ruharness-map-1"),
        })
        .expect("renders");
        let expected = "\
(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath \"/Users\") (subpath \"/Volumes\") (subpath \"/private/tmp\") (subpath \"/private/var/tmp\"))
(allow file-read-metadata (literal \"/Users\") (literal \"/Users/u\") (literal \"/Users/u/Library\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/code\"))
(allow file-read* (subpath \"/Users/u/code/lz4\") (subpath \"/private/var/folders/xy/T/ruharness-map-1\") (subpath \"/Users/u/Library/Caches/ruharness/work\"))
(deny file-read* (subpath \"/Users/u/.cargo\") (subpath \"/Users/u/.rustup\"))
(deny file-write* (subpath \"/\"))
(allow file-write* (subpath \"/private/var/folders/xy/T/ruharness-map-1\") (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))
(deny file-write* (subpath \"/Users/u/Library/Caches/ruharness/perf\") (subpath \"/Users/u/Library/Caches/ruharness/work\") (literal \"/Users/u/Library/Caches/ruharness\") (literal \"/Users/u/Library/Caches\") (literal \"/Users/u/Library\") (literal \"/Users/u\"))
(deny lsopen appleevent-send job-creation)
(deny mach-lookup (global-name-prefix \"com.apple.coreservices.\") (global-name-prefix \"com.apple.CoreServices.\") (global-name \"com.apple.coreservicesd\") (global-name-prefix \"com.apple.lsd.\") (global-name \"com.apple.xpc.smd\") (global-name \"com.apple.xpc.loginitemregisterd\"))
";
        assert_eq!(text, expected);

        // A home folder outside /Users is denied by name; a cargo home
        // outside it too, wherever it is.
        let mut host = host;
        host.home = PathBuf::from("/home/u");
        host.cargo_home = Some(PathBuf::from("/opt/cargo"));
        host.rustup_home = None;
        host.perf_cache = PathBuf::from("/home/u/.cache/ruharness/perf");
        host.work = PathBuf::from("/home/u/.cache/ruharness/work");
        let text = render_map_profile(&MapSpec {
            host: &host,
            project_root: Path::new("/srv/p"),
            fresh: Path::new("/srv/fresh"),
        })
        .expect("renders");
        assert!(
            text.contains(
                "(deny file-read* (subpath \"/Users\") (subpath \"/Volumes\") \
                 (subpath \"/private/tmp\") (subpath \"/private/var/tmp\") (subpath \"/home/u\"))\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("(deny file-read* (subpath \"/opt/cargo\"))\n"),
            "{text}"
        );
    }

    /// Live (macOS): under the map profile `cc -c` and the link of a
    /// two-file program succeed, the compiler starting in the work folder
    /// with `TMPDIR` in the fresh folder, while a header under the home
    /// folder outside the project (the stand-in for `/Users/Shared`) and the
    /// cargo home cannot be read — and the same compile reads it unsandboxed.
    #[test]
    fn cc_compiles_and_links_under_the_map_profile() {
        if !cfg!(target_os = "macos") || !Path::new(SANDBOX_EXEC).exists() {
            return;
        }
        // Under the home folder, as a real project is: beside the test
        // binary (never in a temporary folder, which the profile reads).
        let exe = std::env::current_exe().expect("test exe");
        let base = exe.parent().expect("exe dir").join(format!(
            "ruharness-map-profile-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        std::fs::create_dir_all(&base).expect("base");
        let base = Gone(base.canonicalize().expect("canonical base"));
        let host = HostDirs::from_env().expect("HOME");
        if !base.0.starts_with(&host.home) {
            eprintln!("skipped: the test binary does not lie under the home folder");
            return;
        }
        let root = base.0.join("project");
        let outside = base.0.join("outside");
        for dir in [&root, &outside] {
            std::fs::create_dir_all(dir).expect("dir");
        }
        let tmp = crate::testutil::TempDir::new("map-fresh");
        let fresh = tmp.path().to_path_buf();
        std::fs::write(outside.join("secret.h"), "#define SECRET 1\n").expect("secret");
        std::fs::write(
            root.join("a.c"),
            "int b(void);\nint main(void) { return b(); }\n",
        )
        .expect("a.c");
        std::fs::write(root.join("b.c"), "int b(void) { return 0; }\n").expect("b.c");
        std::fs::write(
            root.join("leak.c"),
            format!(
                "#include \"{}\"\nint x = SECRET;\n",
                outside.join("secret.h").display()
            ),
        )
        .expect("leak.c");
        let runner = crate::exec::Runner::map(
            &root,
            &fresh,
            &["cc", "sh"],
            std::time::Duration::from_secs(120),
        )
        .expect("map runner");
        assert!(runner.tool_profile.is_some());
        let s = |p: &Path| p.to_str().expect("utf-8").to_string();
        for name in ["a", "b"] {
            runner
                .tool(&[
                    "cc".into(),
                    "-c".into(),
                    "-o".into(),
                    s(&fresh.join(format!("{name}.o"))),
                    s(&root.join(format!("{name}.c"))),
                ])
                .expect("cc -c under the map profile");
        }
        runner
            .tool(&[
                "cc".into(),
                "-o".into(),
                s(&fresh.join("prog")),
                s(&fresh.join("a.o")),
                s(&fresh.join("b.o")),
            ])
            .expect("the link under the map profile");
        assert!(fresh.join("prog").is_file());

        // Where it ran, and its TMPDIR.
        let out = runner
            .tool(&["sh".into(), "-c".into(), "pwd -P; echo \"$TMPDIR\"".into()])
            .expect("sh");
        let text = String::from_utf8_lossy(&out);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [s(&host.work).as_str(), s(&fresh).as_str()],
            "{text}"
        );

        // A header outside the project under the home folder is not read.
        let leak = [
            "cc".to_string(),
            "-c".into(),
            "-o".into(),
            s(&fresh.join("leak.o")),
            s(&root.join("leak.c")),
        ];
        let err = runner.tool(&leak).expect_err("the secret must not be read");
        assert!(err.to_string().contains("secret.h"), "{err}");
        let open = crate::exec::Runner {
            tool_profile: None,
            ..runner.clone()
        };
        open.tool(&leak)
            .expect("the same compile reads it unsandboxed");

        // Nor the cargo home, nor anything else under the home folder.
        if let Some(cargo_home) = host.cargo_home.as_ref().filter(|c| c.is_dir()) {
            runner
                .tool(&["sh".into(), "-c".into(), format!("ls '{}'", s(cargo_home))])
                .expect_err("the cargo home is denied");
        }
        runner
            .tool(&[
                "sh".into(),
                "-c".into(),
                format!("cat '{}'", s(&outside.join("secret.h"))),
            ])
            .expect_err("reads under the home folder are denied");
        // Writes go to the fresh folder only.
        runner
            .tool(&[
                "sh".into(),
                "-c".into(),
                format!("touch '{}'", s(&root.join("w"))),
            ])
            .expect_err("the project is not writable");
        assert!(!root.join("w").exists());
    }

    #[test]
    fn wrap_prefixes_the_exact_argv() {
        let argv = vec!["cc".to_string(), "--version".to_string()];
        assert_eq!(
            wrap("(version 1)", &argv),
            vec![
                "/usr/bin/sandbox-exec".to_string(),
                "-p".to_string(),
                "(version 1)".to_string(),
                "cc".to_string(),
                "--version".to_string()
            ]
        );
    }

    #[test]
    fn host_dirs_come_from_the_environment() {
        // HOME is set in every environment these tests run in (cargo needs it).
        let host = HostDirs::from_env().expect("HOME is set");
        assert!(host.home.is_absolute());
        if let Some(c) = &host.cargo_home {
            assert!(c.is_absolute());
        }
    }
}
