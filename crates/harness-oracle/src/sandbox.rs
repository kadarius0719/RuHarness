//! The oracle sandbox (docs/SCHEMAS.md "Trust boundaries"): on macOS every
//! build and every run of target- or model-derived code is wrapped in
//! `/usr/bin/sandbox-exec` with a generated SBPL profile — network denied,
//! reads under the user's home denied except the target root and the Rust
//! toolchain dirs, writes confined to explicitly listed locations and temp.
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
    /// When set (built-binary runs only), the ONLY program the child may
    /// `exec`: the profile denies `process-exec*` and re-allows exactly this
    /// canonical path, so a built binary cannot spawn helper processes and
    /// `sandbox-exec` can still start the binary itself. `None` (the tool
    /// profile, or an unsandboxed run) leaves `(allow default)` in force.
    pub exec_only: Option<&'a Path>,
}

/// Render the SBPL profile for `spec`. SBPL is last-match-wins, so each
/// `deny` is followed by the narrower `allow`s that carve exceptions from it.
pub(crate) fn render_profile(spec: &ProfileSpec<'_>) -> Result<String, Error> {
    let home = spec.host.home.as_path();
    let mut out = String::new();
    out.push_str("(version 1)\n(allow default)\n(deny network*)\n");
    // Built-binary runs may exec nothing but themselves: deny every exec,
    // then re-allow the one binary (SBPL is last-match-wins, so the deny
    // comes first). The allow is also what lets `sandbox-exec` start it.
    if let Some(bin) = spec.exec_only {
        out.push_str("(deny process-exec*)\n");
        out.push_str(&format!(
            "(allow process-exec (literal {}))\n",
            sbpl_string(bin)?
        ));
    }
    out.push_str(&format!(
        "(deny file-read* (subpath {}))\n",
        sbpl_string(home)?
    ));

    let mut read_roots: Vec<&Path> = vec![spec.target_root];
    if spec.toolchain {
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

        // Narrow exception 2: rustup picks the toolchain from the nearest
        // `rust-toolchain(.toml)` walking up from the cwd (the target root).
        // Those ancestor files are user-owned, not target-owned; leaving them
        // unreadable would make rustup silently fall back to the default
        // toolchain, so sandboxed and unsandboxed runs could disagree about
        // which rustc they test with. Allow exactly those two file names.
        let root_ancestors: Vec<&Path> = spec
            .target_root
            .ancestors()
            .skip(1)
            .filter(|a| a.starts_with(home))
            .collect();
        if !root_ancestors.is_empty() {
            out.push_str("(allow file-read*");
            for anc in root_ancestors.iter().rev() {
                for name in ["rust-toolchain", "rust-toolchain.toml"] {
                    out.push_str(&format!(" (literal {})", sbpl_string(&anc.join(name))?));
                }
            }
            out.push_str(")\n");
        }
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
    Ok(out)
}

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
        }
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
            exec_only: None,
        })
        .expect("renders");
        let expected = "\
(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath \"/Users/u\"))
(allow file-read-metadata (literal \"/Users/u\"))
(allow file-read* (literal \"/Users/u/rust-toolchain\") (literal \"/Users/u/rust-toolchain.toml\"))
(allow file-read* (subpath \"/Users/u/t\") (subpath \"/Users/u/.cargo\") (subpath \"/Users/u/.rustup\"))
(deny file-read* (literal \"/Users/u/.cargo/credentials.toml\") (literal \"/Users/u/.cargo/credentials\"))
(deny file-write* (subpath \"/\"))
(allow file-write* (subpath \"/Users/u/t/migration/build/u1\") (subpath \"/Users/u/t/migration/units/u1/c/target\") (literal \"/Users/u/t/migration/units/u1/c/Cargo.lock\") (subpath \"/private/tmp\") (subpath \"/private/var/folders\") (subpath \"/private/var/folders/xy/T\") (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))
";
        assert_eq!(text, expected);
    }

    #[test]
    fn run_profile_reads_only_the_target_root_and_writes_only_temp() {
        let host = host();
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/code/t"),
            toolchain: false,
            write_dirs: &[],
            write_files: &[],
            exec_only: None,
        })
        .expect("renders");
        assert!(text.contains("(deny network*)"), "{text}");
        // With no exec restriction the default allow governs process-exec.
        assert!(!text.contains("process-exec"), "{text}");
        assert!(
            text.contains("(allow file-read* (subpath \"/Users/u/code/t\"))\n"),
            "{text}"
        );
        assert!(!text.contains(".cargo"), "{text}");
        assert!(!text.contains(".rustup"), "{text}");
        assert!(!text.contains("file-read-metadata"), "{text}");
        assert!(!text.contains("rust-toolchain"), "{text}");
        assert!(
            text.contains(
                "(allow file-write* (subpath \"/private/tmp\") (subpath \"/private/var/folders\")"
            ),
            "{text}"
        );
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
            exec_only: None,
        })
        .expect("renders");
        // Only the toolchain dirs' ancestor (the home itself) needs metadata.
        assert!(
            text.contains("(allow file-read-metadata (literal \"/Users/u\"))\n"),
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
            exec_only: None,
        })
        .expect("renders");
        assert!(
            text.contains(
                "(allow file-read-metadata (literal \"/Users/u\") (literal \"/Users/u/code\") \
                 (literal \"/Users/u/code/repo\") (literal \"/Users/u/code/repo/targets\"))\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("(literal \"/Users/u/code/repo/rust-toolchain.toml\")"),
            "{text}"
        );
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
            exec_only: None,
        })
        .expect_err("injection attempt must fail");
        assert!(err.to_string().contains("sandbox"), "{err}");
    }

    #[test]
    fn a_run_profile_denies_every_exec_but_the_binary_itself() {
        let host = host();
        let bin = PathBuf::from("/Users/u/t/migration/build/u1/drv_rs");
        let text = render_profile(&ProfileSpec {
            host: &host,
            target_root: Path::new("/Users/u/t"),
            toolchain: false,
            write_dirs: &[],
            write_files: &[],
            exec_only: Some(&bin),
        })
        .expect("renders");
        // Deny comes before the allow so last-match-wins leaves only the
        // binary itself executable.
        let deny = text.find("(deny process-exec*)").expect("deny present");
        let allow = text
            .find("(allow process-exec (literal \"/Users/u/t/migration/build/u1/drv_rs\"))")
            .expect("allow present");
        assert!(deny < allow, "{text}");
        // The exec rules sit above the file rules, right after the network deny.
        assert!(text.starts_with(
            "(version 1)\n(allow default)\n(deny network*)\n\
             (deny process-exec*)\n\
             (allow process-exec (literal \"/Users/u/t/migration/build/u1/drv_rs\"))\n"
        ));
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
