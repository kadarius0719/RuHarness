//! Consent is the server's, never the caller's (docs/MCP-DESIGN.md §1, §4):
//! the targets, the provider list, the harness binary and the sandbox switch
//! are server flags a human wrote. A tool argument can only NAME a target,
//! and only one strictly inside a `--target-root`.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The server's configuration, from its command line.
#[derive(Debug, Clone)]
pub struct Config {
    /// The default target (canonical).
    pub target: PathBuf,
    /// Directories a call's `target` may lie strictly inside (canonical).
    pub target_roots: Vec<PathBuf>,
    /// The `harness` binary acts spawn; `None` = read-only (none found).
    pub harness: Option<PathBuf>,
    /// Provider profiles a steer attempt may use (`external`, when listed,
    /// is the default).
    pub providers: Vec<String>,
    /// Pass `--allow-unsandboxed` to the acts.
    pub allow_unsandboxed: bool,
    /// `$HOME`, canonical, when set: it and its ancestors are never targets.
    pub home: Option<PathBuf>,
}

const USAGE: &str = "usage: harness-mcp --target DIR [--target-root DIR]... [--harness PATH] \
                     [--provider NAME]... [--allow-unsandboxed]";

/// The usage line.
pub fn usage() -> &'static str {
    USAGE
}

/// Parse the command line (after the program name). `Ok(None)` for
/// `--help`/`--version` (already answered on stderr).
pub fn parse_args(args: &[String]) -> Result<Option<Config>, String> {
    let mut target = None;
    let mut roots = Vec::new();
    let mut harness = None;
    let mut providers: Vec<String> = Vec::new();
    let mut allow_unsandboxed = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let (flag, attached) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(v) = attached.clone() {
                return Ok(v);
            }
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
        };
        match flag {
            "--target" => target = Some(PathBuf::from(value()?)),
            "--target-root" => roots.push(PathBuf::from(value()?)),
            "--harness" => harness = Some(PathBuf::from(value()?)),
            "--provider" => providers.push(value()?),
            "--allow-unsandboxed" if attached.is_none() => allow_unsandboxed = true,
            "--help" | "-h" => {
                let _ = writeln!(std::io::stderr(), "{USAGE}");
                return Ok(None);
            }
            "--version" => {
                let _ = writeln!(
                    std::io::stderr(),
                    "harness-mcp {}",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(None);
            }
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
        i += 1;
    }
    let target = target.ok_or_else(|| format!("--target is required\n{USAGE}"))?;
    let target = canonical_dir(&target, "--target")?;
    if !target.join("harness.toml").is_file() {
        return Err(format!(
            "--target {}: no harness.toml there",
            target.display()
        ));
    }
    let target_roots = roots
        .iter()
        .map(|r| canonical_dir(r, "--target-root"))
        .collect::<Result<Vec<_>, _>>()?;
    if providers.is_empty() {
        providers.push("external".into());
    }
    for p in &providers {
        if !harness_core::plan::is_clean_segment(p) || p.len() > 64 {
            return Err(format!("--provider {p:?} is not a provider profile name"));
        }
    }
    // Keep the first of each, in order; drop repeats.
    let mut seen = std::collections::BTreeSet::new();
    providers.retain(|p| seen.insert(p.clone()));
    let harness = match harness {
        Some(path) => Some(
            path.canonicalize()
                .ok()
                .filter(|p| is_executable(p))
                .ok_or_else(|| format!("--harness {}: not an executable file", path.display()))?,
        ),
        None => find_harness(),
    };
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .and_then(|h| PathBuf::from(h).canonicalize().ok());
    Ok(Some(Config {
        target,
        target_roots,
        harness,
        providers,
        allow_unsandboxed,
        home,
    }))
}

fn canonical_dir(path: &Path, flag: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("{flag} {}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{flag} {}: not a directory", path.display()));
    }
    Ok(canonical)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `harness` on `PATH` (absolute entries only: a relative entry would
/// resolve inside whatever directory the server runs in), else next to
/// this binary.
fn find_harness() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("harness"))
            .find(|p| is_executable(p))
    });
    on_path
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|exe| exe.with_file_name("harness"))
                .filter(|p| is_executable(p))
        })
        .and_then(|p| p.canonicalize().ok())
}

impl Config {
    /// The target a call names (`None` = the default), canonical — or why
    /// it is refused. Nothing is read or spawned for a refused target.
    pub fn resolve_target(&self, requested: Option<&str>) -> Result<PathBuf, String> {
        let Some(requested) = requested else {
            return Ok(self.target.clone());
        };
        let canonical = Path::new(requested)
            .canonicalize()
            .map_err(|_| "the target does not exist".to_string())?;
        if canonical == self.target {
            return Ok(canonical);
        }
        if canonical.parent().is_none() {
            return Err("the target is the filesystem root".into());
        }
        if let Some(home) = &self.home {
            if home.starts_with(&canonical) {
                return Err("the target is $HOME or an ancestor of it".into());
            }
        }
        if !self
            .target_roots
            .iter()
            .any(|root| canonical != *root && canonical.starts_with(root))
        {
            return Err(if self.target_roots.is_empty() {
                "this server serves only its --target (no --target-root configured)".into()
            } else {
                "the target is not strictly inside a configured --target-root".into()
            });
        }
        if !canonical.join("harness.toml").is_file() {
            return Err("the target holds no harness.toml".into());
        }
        Ok(canonical)
    }
}

/// The read preflight: one implementation, in the read model's crate.
pub use harness_tui::preflight::preflight;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("harness-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn target(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("harness.toml"), "schema_version = 1\n").unwrap();
        dir.canonicalize().unwrap()
    }

    fn config(default: PathBuf, roots: Vec<PathBuf>, home: Option<PathBuf>) -> Config {
        Config {
            target: default,
            target_roots: roots,
            harness: None,
            providers: vec!["external".into()],
            allow_unsandboxed: false,
            home,
        }
    }

    #[test]
    fn a_target_must_lie_strictly_inside_a_root() {
        let base = tmp("targets");
        let default = target(&base.join("default"));
        let root = base.join("root");
        let inside = target(&root.join("case"));
        let outside = target(&base.join("elsewhere"));
        let cfg = config(default.clone(), vec![root.canonicalize().unwrap()], None);
        assert_eq!(cfg.resolve_target(None).unwrap(), default);
        assert_eq!(
            cfg.resolve_target(Some(default.to_str().unwrap())).unwrap(),
            default
        );
        // The caller's spelling never reaches the argv: canonical only.
        let spelled = format!("{}/../root/./case", default.display());
        assert_eq!(cfg.resolve_target(Some(&spelled)).unwrap(), inside);
        assert!(cfg.resolve_target(Some(outside.to_str().unwrap())).is_err());
        // The root itself is not strictly inside itself.
        let root_target = target(&root);
        assert!(cfg
            .resolve_target(Some(root_target.to_str().unwrap()))
            .unwrap_err()
            .contains("strictly inside"));
        // Inside a root but no harness.toml.
        std::fs::create_dir_all(root.join("bare")).unwrap();
        assert!(cfg
            .resolve_target(Some(root.join("bare").to_str().unwrap()))
            .unwrap_err()
            .contains("harness.toml"));
        assert!(cfg.resolve_target(Some("/no/such/dir")).is_err());
        // A symlink inside a root pointing outside resolves outside.
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(cfg
            .resolve_target(Some(root.join("link").to_str().unwrap()))
            .is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn root_home_and_its_ancestors_are_never_targets() {
        let base = tmp("home");
        let home = target(&base.join("home/me"));
        let cfg = config(
            target(&base.join("default")),
            vec![PathBuf::from("/"), base.clone()],
            Some(home.clone()),
        );
        assert!(cfg.resolve_target(Some("/")).unwrap_err().contains("root"));
        assert!(cfg
            .resolve_target(Some(home.to_str().unwrap()))
            .unwrap_err()
            .contains("$HOME"));
        let parent = target(&base.join("home"));
        assert!(cfg
            .resolve_target(Some(parent.to_str().unwrap()))
            .unwrap_err()
            .contains("$HOME"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_command_line_is_parsed_and_checked() {
        let base = tmp("args");
        let t = target(&base.join("t"));
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cfg = parse_args(&args(&["--target", t.to_str().unwrap()]))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.providers, vec!["external"]);
        assert!(!cfg.allow_unsandboxed);
        let cfg = parse_args(&args(&[
            &format!("--target={}", t.display()),
            "--provider=local",
            "--provider",
            "external",
            "--allow-unsandboxed",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(cfg.providers, vec!["local", "external"]);
        assert!(cfg.allow_unsandboxed);
        // Repeats are dropped wherever they are; the order is kept.
        let cfg = parse_args(&args(&[
            "--target",
            t.to_str().unwrap(),
            "--provider=external",
            "--provider=local",
            "--provider=external",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(cfg.providers, vec!["external", "local"]);
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["--target", base.to_str().unwrap()]))
            .unwrap_err()
            .contains("harness.toml"));
        assert!(parse_args(&args(&["--target", t.to_str().unwrap(), "--bogus"])).is_err());
        assert!(parse_args(&args(&[
            "--target",
            t.to_str().unwrap(),
            "--provider",
            "../x"
        ]))
        .is_err());
        assert!(parse_args(&args(&[
            "--target",
            t.to_str().unwrap(),
            "--allow-unsandboxed=no"
        ]))
        .is_err());
        assert!(parse_args(&args(&[
            "--target",
            t.to_str().unwrap(),
            "--harness",
            "/no/such/harness"
        ]))
        .is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A temp directory removed on drop — also when a test fails (a
    /// mutation run fails many on purpose).
    pub(crate) struct TmpDir(pub PathBuf);

    impl TmpDir {
        pub(crate) fn new(tag: &str) -> TmpDir {
            let dir =
                std::env::temp_dir().join(format!("harness-mcp-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TmpDir(dir)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A copy of the zopfli target (its ledger, config and sources).
    pub(crate) fn zopfli_copy(dst: &Path) -> PathBuf {
        fn copy(src: &Path, dst: &Path) {
            std::fs::create_dir_all(dst).unwrap();
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                if name == "build" || name == "target" || name == ".lock" {
                    continue;
                }
                let (from, to) = (entry.path(), dst.join(&name));
                if from.is_dir() {
                    copy(&from, &to);
                } else {
                    std::fs::copy(&from, &to).unwrap();
                }
            }
        }
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy(&repo.join("targets/zopfli"), dst);
        dst.canonicalize().unwrap()
    }
}
