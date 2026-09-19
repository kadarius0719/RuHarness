//! Machine-path scrubbing (docs/SCHEMAS.md "Verdicts" — committed evidence).
//!
//! Verdict check details and any error text this crate emits can embed
//! absolute paths lifted from command lines and tool stderr: the canonical
//! target root, the user's home, the rustup/cargo/temp directories. Those are
//! machine-specific, but a verdict is committed evidence that must be
//! byte-identical across the developers who share a target. This module
//! rewrites every such prefix to a stable placeholder BEFORE a check or error
//! leaves the crate, so the surviving text (a rustc diagnostic, a
//! relative `src/…` path) is reproducible.
//!
//! Prefixes are replaced longest-first, so the most specific location wins: a
//! target root nested under the home directory becomes `<target>`, not
//! `<home>/…`, and `<home>/.cargo` becomes `<cargo>`, not `<home>/.cargo`.

use harness_core::error::Error;
use std::path::{Path, PathBuf};

/// A set of machine-specific path prefixes and the placeholders they map to.
#[derive(Debug, Clone)]
pub(crate) struct Scrubber {
    /// `(needle, placeholder)` pairs, sorted so the longest needle is applied
    /// first (a nested prefix wins over the directory that contains it).
    replacements: Vec<(String, &'static str)>,
}

impl Scrubber {
    /// Build a scrubber from `target_root` and the process environment. Never
    /// fails: a path that will not canonicalize is used as-is, and a variable
    /// that is unset simply contributes no rule, so scrubbing works even
    /// before (or without) a sandbox.
    pub(crate) fn from_env(target_root: &Path) -> Scrubber {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let env_dir = |key: &str| {
            std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(|v| canon(Path::new(&v)))
        };
        let home = env_dir("HOME");

        // (needle, placeholder) — collected unsorted; ordering by length is
        // applied once at the end so `apply` can walk them in place.
        let mut raw: Vec<(PathBuf, &'static str)> = Vec::new();
        raw.push((canon(target_root), "<target>"));

        let home_join = |sub: &str| home.as_ref().map(|h| h.join(sub));
        for dir in env_dir("RUSTUP_HOME")
            .into_iter()
            .chain(home_join(".rustup"))
        {
            raw.push((dir, "<rustup>"));
        }
        for dir in env_dir("CARGO_HOME").into_iter().chain(home_join(".cargo")) {
            raw.push((dir, "<cargo>"));
        }

        // Temp: the canonical TMPDIR plus its `/private` twin (macOS routes
        // /tmp and /var through /private), and the shared temp roots.
        for tmp in temp_dirs() {
            raw.push((tmp, "<tmp>"));
        }

        if let Some(home) = home {
            raw.push((home, "<home>"));
        }

        // Longest needle first; drop empties and non-UTF-8 (they cannot be
        // substring-matched anyway).
        let mut replacements: Vec<(String, &'static str)> = raw
            .into_iter()
            .filter_map(|(p, tok)| p.to_str().map(|s| (s.to_string(), tok)))
            .filter(|(s, _)| !s.is_empty() && s != "/")
            .collect();
        replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        replacements.dedup_by(|a, b| a.0 == b.0);
        Scrubber { replacements }
    }

    /// Replace every known machine path prefix in `text` with its placeholder.
    pub(crate) fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (needle, token) in &self.replacements {
            if out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), token);
            }
        }
        out
    }

    /// Scrub a [`Check`]'s detail in place, right before it enters a verdict.
    pub(crate) fn scrub_check(&self, check: &mut harness_core::verdict::Check) {
        check.detail = self.apply(&check.detail);
    }

    /// Scrub the machine paths out of an error whose message embeds a command
    /// line or tool stderr before it leaves this crate. Only the string-body
    /// variants can carry such text; structured variants pass through.
    pub(crate) fn scrub_error(&self, err: Error) -> Error {
        match err {
            Error::Invariant(m) => Error::Invariant(self.apply(&m)),
            Error::InvalidPlan(m) => Error::InvalidPlan(self.apply(&m)),
            Error::Parse { path, message } => Error::parse(path, self.apply(&message)),
            other => other,
        }
    }
}

/// The temp directories to fold to `<tmp>`: the canonical `TMPDIR` and its
/// `/private`-prefixed twin, plus the shared macOS temp roots.
fn temp_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    if let Some(raw) = std::env::var_os("TMPDIR").filter(|v| !v.is_empty()) {
        let raw = PathBuf::from(raw);
        if let Ok(canon) = raw.canonicalize() {
            // The `/private` twin, so a path printed either way is caught.
            if let Ok(stripped) = canon.strip_prefix("/private") {
                push(Path::new("/").join(stripped));
            } else {
                push(Path::new("/private").join(canon.strip_prefix("/").unwrap_or(&canon)));
            }
            push(canon);
        }
        push(raw);
    }
    push(PathBuf::from("/private/var/folders"));
    push(PathBuf::from("/private/tmp"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins_over_the_directory_that_contains_it() {
        // A synthetic environment: target nested under home, cargo/rustup too.
        let home = "/Users/dev";
        let scrub = Scrubber {
            replacements: {
                let mut r = vec![
                    ("/Users/dev/code/repo/targets/z".to_string(), "<target>"),
                    ("/Users/dev/.rustup".to_string(), "<rustup>"),
                    ("/Users/dev/.cargo".to_string(), "<cargo>"),
                    ("/private/var/folders/xy/T".to_string(), "<tmp>"),
                    (home.to_string(), "<home>"),
                ];
                r.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
                r
            },
        };
        let text = "note: /Users/dev/code/repo/targets/z/src/lib.rs and \
                    /Users/dev/.cargo/registry and /Users/dev/.rustup/toolchains and \
                    /Users/dev/other and /private/var/folders/xy/T/x";
        assert_eq!(
            scrub.apply(text),
            "note: <target>/src/lib.rs and <cargo>/registry and <rustup>/toolchains and \
             <home>/other and <tmp>/x"
        );
    }

    #[test]
    fn from_env_folds_the_real_target_root_and_home() {
        let tmp = crate::testutil::TempDir::new("scrub-env");
        let scrub = Scrubber::from_env(tmp.path());
        let sample = format!(
            "built at {}/migration/build/u1/drv_rs",
            tmp.path().display()
        );
        let out = scrub.apply(&sample);
        assert_eq!(out, "built at <target>/migration/build/u1/drv_rs");
        // The temp dir the bench lives in is under the system temp root, so a
        // scrubbed detail can never leak an absolute machine path.
        assert!(!out.contains("/Users/"), "{out}");
        assert!(!out.contains("/home/"), "{out}");

        if let Some(home) = std::env::var_os("HOME") {
            let home = Path::new(&home).canonicalize().unwrap_or_default();
            let msg = format!("stray {}/secret", home.display());
            assert_eq!(scrub.apply(&msg), "stray <home>/secret");
        }
    }

    #[test]
    fn errors_are_scrubbed_only_where_they_carry_free_text() {
        let scrub = Scrubber {
            replacements: vec![("/Users/dev/t".to_string(), "<target>")],
        };
        let inv = scrub.scrub_error(Error::Invariant("cc at /Users/dev/t/x failed".into()));
        assert_eq!(inv.to_string(), "cc at <target>/x failed");
        let plan = scrub.scrub_error(Error::InvalidPlan("/Users/dev/t/p is bad".into()));
        assert!(plan.to_string().contains("<target>/p is bad"), "{plan}");
        // A structured I/O error keeps its path (no command line or stderr).
        let io = scrub.scrub_error(Error::io(
            Path::new("/Users/dev/t/x"),
            std::io::Error::new(std::io::ErrorKind::NotFound, "nope"),
        ));
        assert!(io.to_string().contains("/Users/dev/t/x"), "{io}");
    }
}
