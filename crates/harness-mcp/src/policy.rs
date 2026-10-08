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
    /// `--cockpit`: the server of the cockpit's chat (docs/CHAT-PANE-DESIGN.md
    /// §4.4) — no harness binary at all (none is looked for), its one
    /// target, the reads, and act tools that only ASK: the cockpit, which
    /// holds every tool call as a permission request, runs the act itself;
    /// this server refuses every act that reaches it.
    pub cockpit: bool,
    /// The default target's mapped tool (`--tool`, or the project's only
    /// tool; docs/PROJECT-MAP-DESIGN.md §3.7); `None` for a folder-form
    /// target. A call's other target (under a `--target-root`) has none.
    pub tool: Option<String>,
}

const USAGE: &str = "usage: harness-mcp --target DIR [--tool ID] [--target-root DIR]... \
                     [--harness PATH] [--provider NAME]... [--allow-unsandboxed]\n       \
                     harness-mcp --cockpit --target DIR [--tool ID]";

/// The usage line.
pub fn usage() -> &'static str {
    USAGE
}

/// Why the command line was refused, and the exit code: 2 for a usage
/// error, 1 for a folder that names no target (as the command line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgError {
    /// The exit code.
    pub code: i32,
    /// The sentence.
    pub message: String,
}

impl From<String> for ArgError {
    fn from(message: String) -> ArgError {
        ArgError { code: 2, message }
    }
}

/// Parse the command line (after the program name). `Ok(None)` for
/// `--help`/`--version` (already answered on stderr).
pub fn parse_args(args: &[String]) -> Result<Option<Config>, ArgError> {
    let mut target = None;
    let mut tool = None;
    let mut roots = Vec::new();
    let mut harness = None;
    let mut providers: Vec<String> = Vec::new();
    let mut allow_unsandboxed = false;
    let mut cockpit = false;
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
            "--tool" => {
                let id = value()?;
                harness_core::config::check_tool_id(&id).map_err(|why| format!("--tool: {why}"))?;
                tool = Some(id);
            }
            "--target-root" => roots.push(PathBuf::from(value()?)),
            "--harness" => harness = Some(PathBuf::from(value()?)),
            "--provider" => providers.push(value()?),
            "--allow-unsandboxed" if attached.is_none() => allow_unsandboxed = true,
            "--cockpit" if attached.is_none() => cockpit = true,
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
            other => return Err(format!("unknown argument {other:?}\n{USAGE}").into()),
        }
        i += 1;
    }
    // The cockpit's server spawns nothing and serves one target: a flag
    // that would give it a binary, a provider, another target or the
    // sandbox switch is a mistake, never ignored (§4.4).
    if cockpit {
        let given = [
            ("--harness", harness.is_some()),
            ("--provider", !providers.is_empty()),
            ("--target-root", !roots.is_empty()),
            ("--allow-unsandboxed", allow_unsandboxed),
        ];
        if let Some((flag, _)) = given.iter().find(|(_, set)| *set) {
            return Err(format!(
                "--cockpit takes no {flag}: the cockpit runs every act itself\n{USAGE}"
            )
            .into());
        }
    }
    let target = target.ok_or_else(|| format!("--target is required\n{USAGE}"))?;
    let target = canonical_dir(&target, "--target")?;
    // The lookup order of the command line: `--tool`, else the root's
    // harness.toml, else the project's only mapped tool.
    // A folder that names no target is the command line's refusal, in its
    // words and with its exit code (1), not a usage error.
    let refused = |e: harness_core::Error| ArgError {
        code: 1,
        message: e.to_string(),
    };
    let tool = match harness_core::config::find_target(&target, tool.as_deref())
        .map_err(|e| refused(e.opening(&target, tool.as_deref())))?
    {
        harness_core::config::Found::Tool(id) => Some(id),
        harness_core::config::Found::Root => {
            if !target.join("harness.toml").is_file() {
                return Err(refused(harness_core::Error::no_target_here(&target)));
            }
            None
        }
    };
    let target_roots = roots
        .iter()
        .map(|r| canonical_dir(r, "--target-root"))
        .collect::<Result<Vec<_>, _>>()?;
    if providers.is_empty() {
        providers.push("external".into());
    }
    for p in &providers {
        if !harness_core::plan::is_clean_segment(p) || p.len() > 64 {
            return Err(format!("--provider {p:?} is not a provider profile name").into());
        }
    }
    // Keep the first of each, in order; drop repeats.
    let mut seen = std::collections::BTreeSet::new();
    providers.retain(|p| seen.insert(p.clone()));
    let harness = match harness {
        // No binary at all — not "none given, look for one" (§R SAFE-9).
        None if cockpit => None,
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
        cockpit,
        tool,
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
    /// The tool of `target`: the server's own `--tool` for its own target,
    /// none for any other.
    pub fn tool_for(&self, target: &Path) -> Option<&str> {
        (target == self.target)
            .then_some(self.tool.as_deref())
            .flatten()
    }

    /// The ledger of `target` (see [`Config::tool_for`]).
    pub fn ledger_for(&self, target: &Path) -> harness_core::ledger::Ledger {
        match self.tool_for(target) {
            None => harness_core::ledger::Ledger::new(target),
            Some(id) => {
                harness_core::ledger::Ledger::at(target, harness_core::config::tool_dir(target, id))
            }
        }
    }

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
pub use harness_tui::preflight::preflight_tool;

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
            tool: None,
            target_roots: roots,
            harness: None,
            providers: vec!["external".into()],
            allow_unsandboxed: false,
            home,
            cockpit: false,
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

    /// `--tool` (docs/PROJECT-MAP-DESIGN.md §3.7): checked by the id rule,
    /// found by the command line's lookup order, and carried only by the
    /// server's own target.
    #[test]
    fn a_tool_is_found_as_the_command_line_finds_it() {
        let base = tmp("tools");
        let project = base.join("p");
        for id in ["t-a", "t-b"] {
            let dir = harness_core::config::tool_dir(&project, id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("harness.toml"), "schema_version = 1\n").unwrap();
        }
        let project = project.canonicalize().unwrap();
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let p = project.to_str().unwrap();
        // Two tools and no --tool: the command line's refusal and exit code.
        let err = parse_args(&args(&["--target", p])).unwrap_err();
        assert!(
            err.message.contains("2 mapped tools") && err.message.contains("t-a, t-b"),
            "{err:?}"
        );
        assert_eq!(err.code, 1);
        let err = parse_args(&args(&["--target", p, "--tool", "T-A"])).unwrap_err();
        assert!(err.message.contains("is not a tool id"), "{err:?}");
        assert_eq!(err.code, 2, "a bad id is a usage error");
        // A folder with neither: the one "no target here" sentence.
        let bare = base.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let bare = bare.canonicalize().unwrap();
        let err = parse_args(&args(&["--target", bare.to_str().unwrap()])).unwrap_err();
        assert_eq!(
            err,
            ArgError {
                code: 1,
                message: harness_core::Error::no_target_here(&bare).to_string()
            }
        );
        let cfg = parse_args(&args(&["--cockpit", "--target", p, "--tool=t-b"]))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.tool.as_deref(), Some("t-b"));
        assert_eq!(cfg.tool_for(&project), Some("t-b"));
        assert_eq!(cfg.tool_for(&base), None);
        assert_eq!(
            cfg.ledger_for(&project).dir(),
            project.join("migration/tools/t-b")
        );
        assert_eq!(cfg.ledger_for(&base).dir(), base.join("migration"));
        // One tool left: found without --tool.
        std::fs::remove_dir_all(harness_core::config::tool_dir(&project, "t-b")).unwrap();
        let cfg = parse_args(&args(&["--target", p])).unwrap().unwrap();
        assert_eq!(cfg.tool.as_deref(), Some("t-a"));
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
            .message
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

    /// `--cockpit` (docs/CHAT-PANE-DESIGN.md §4.4): no harness binary at
    /// all — none is looked for — and every flag that would give it one, a
    /// provider, another target or the sandbox switch is refused.
    #[test]
    fn cockpit_mode_has_no_binary_and_refuses_the_act_flags() {
        let base = tmp("cockpit-args");
        let t = target(&base.join("t"));
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let target = format!("--target={}", t.display());
        let cfg = parse_args(&args(&["--cockpit", &target])).unwrap().unwrap();
        assert!(cfg.cockpit);
        assert_eq!(cfg.harness, None, "no binary, not even one found");
        assert!(cfg.target_roots.is_empty());
        assert!(!cfg.allow_unsandboxed);
        for extra in [
            vec!["--harness", "/bin/sh"],
            vec!["--harness=/bin/sh"],
            vec!["--provider", "external"],
            vec!["--target-root", base.to_str().unwrap()],
            vec!["--allow-unsandboxed"],
        ] {
            let mut v = vec!["--cockpit", target.as_str()];
            v.extend(extra.iter().copied());
            let err = parse_args(&args(&v)).unwrap_err();
            assert!(err.message.contains("--cockpit takes no"), "{v:?}: {err:?}");
        }
        // The flag takes no value.
        assert!(parse_args(&args(&["--cockpit=yes", &target])).is_err());
        assert!(!parse_args(&args(&[&target])).unwrap().unwrap().cockpit);
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
        harness_core::adopt::testing::adopt(dst);
        dst.canonicalize().unwrap()
    }
}
