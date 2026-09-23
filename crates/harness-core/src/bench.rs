//! Benchmark suites (docs/SCHEMAS.md "M4 additions"): the suite manifest,
//! the corpus lock that pins and checksums every vendored upstream file, the
//! vendoring step, and the canonical scores record with its regression
//! comparison.
//!
//! Layout of a suite dir (e.g. `targets/tractor/`):
//! - `suite.toml` — upstream pin + batteries + the derived case list;
//! - `corpus.lock` — canonical JSONL: one line per vendored file;
//! - `cases/<upstream-case-path>/` — one harness TARGET per case: the
//!   upstream `test_case/` plus harness-owned files (`harness.toml`,
//!   `migration/`, runtime views);
//! - `heldout/<upstream-case-path>/{test_vectors,runner}` and
//!   `heldout/tools/cando2/` — held-out scoring material, never inside any
//!   target root, never read by a prompt-building code path;
//! - `scores.json` — the committed scores (the regression baseline).
//!
//! Filesystem only (no processes): building and running the scorer is the
//! oracle crate's job.

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Version of `suite.toml` this build understands.
pub const SUITE_SCHEMA_VERSION: u64 = 1;
/// `schema` of `corpus.lock`'s header line.
pub const CORPUS_LOCK_SCHEMA_NAME: &str = "ruharness-corpus-lock";
/// Version of `corpus.lock`.
pub const CORPUS_LOCK_SCHEMA_VERSION: u64 = 1;
/// `schema` of `scores.json`.
pub const SCORES_SCHEMA_NAME: &str = "ruharness-bench-scores";
/// Version of `scores.json`.
pub const SCORES_SCHEMA_VERSION: u64 = 1;
/// Dir (under the suite dir) holding one harness target per case.
pub const CASES_DIR: &str = "cases";
/// Dir (under the suite dir) holding held-out scoring material.
pub const HELDOUT_DIR: &str = "heldout";
/// Harness-authored files inside `heldout/` that are hash-locked (never
/// exempt): the scorer workspace manifest and its dependency lock.
pub const LOCAL_LOCKED: [&str; 2] = ["heldout/Cargo.toml", "heldout/Cargo.lock"];
/// Harness-authored dir inside `heldout/` whose every file is hash-locked:
/// minimal, documented portability patches to scorer DEPENDENCIES (applied
/// via `[patch.crates-io]`; never to the corpus's own sources).
pub const LOCAL_LOCKED_DIR: &str = "heldout/patches";
/// Harness-owned file names allowed directly in a case target root.
pub const CASE_LOCAL_FILES: [&str; 3] = ["harness.toml", "AGENTS.md", "CLAUDE.md"];
/// Upstream tool dirs vendored into `heldout/` (scorer library). Their
/// `tests/` subdirs are not vendored (never built by the scorer).
pub const SCORER_TOOL_DIRS: [&str; 1] = ["tools/cando2"];

// ---------------------------------------------------------------- suite.toml

/// `suite.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suite {
    /// Schema version.
    pub schema_version: u64,
    /// Suite name (e.g. `tractor-b01-lib`).
    pub name: String,
    /// The pinned upstream.
    pub upstream: Upstream,
    /// Battery dirs the case list is derived from.
    #[serde(default, rename = "battery")]
    pub batteries: Vec<Battery>,
    /// The derived case list (written by vendoring; reviewable).
    #[serde(default, rename = "case")]
    pub cases: Vec<SuiteCase>,
    /// Cases found in a battery but excluded, with the reason.
    #[serde(default, rename = "excluded")]
    pub excluded: Vec<Excluded>,
}

/// The pinned upstream corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Upstream {
    /// Repository URL (provenance only; never fetched by the harness).
    pub repo: String,
    /// Tag at the pinned commit.
    pub tag: String,
    /// Full commit SHA the vendored files were taken from.
    pub commit: String,
}

/// One upstream battery dir and the split its cases belong to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Battery {
    /// Upstream-relative dir, e.g. `Public-Tests/B01_organic`.
    pub dir: String,
    /// `public` or `hidden`.
    pub split: String,
}

/// One scored case.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuiteCase {
    /// Upstream-relative case dir, e.g. `Public-Tests/B01_organic/rev16_lib`.
    pub path: String,
    /// `public` or `hidden`.
    pub split: String,
    /// Shared-library stem the runner `dlopen`s (`lib<library>.dylib`).
    pub library: String,
    /// Symbol the runner calls.
    pub symbol: String,
    /// Cargo package name of the case runner.
    pub runner: String,
}

/// A battery case left out of the suite, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Excluded {
    /// Upstream-relative case dir.
    pub path: String,
    /// Reason (harness-generated).
    pub reason: String,
}

impl SuiteCase {
    /// The case's harness target root.
    pub fn target_root(&self, suite_dir: &Path) -> PathBuf {
        suite_dir.join(CASES_DIR).join(&self.path)
    }
    /// The case's held-out dir (`test_vectors/`, `runner/`).
    pub fn heldout_dir(&self, suite_dir: &Path) -> PathBuf {
        suite_dir.join(HELDOUT_DIR).join(&self.path)
    }
    /// Last path component (the upstream case dir name).
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

impl Suite {
    /// Load and validate `suite.toml` (hostile-input rules: clean relative
    /// paths, clean segment names, closed split enum, no duplicates).
    pub fn load(path: &Path) -> Result<Suite, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let suite: Suite = toml::from_str(&text).map_err(|e| Error::parse(path, e.to_string()))?;
        if suite.schema_version > SUITE_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: suite.schema_version,
                supported: SUITE_SCHEMA_VERSION,
            });
        }
        suite.validate().map_err(|m| Error::parse(path, m))?;
        Ok(suite)
    }

    fn validate(&self) -> Result<(), String> {
        let clean = crate::plan::is_clean_relative_path;
        let seg = crate::plan::is_clean_segment;
        if !(self.upstream.commit.len() == 40
            && self.upstream.commit.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("upstream.commit must be a full 40-hex commit id".into());
        }
        for b in &self.batteries {
            if !clean(&b.dir) || !matches!(b.split.as_str(), "public" | "hidden") {
                return Err(format!("bad battery {:?}/{:?}", b.dir, b.split));
            }
        }
        let mut seen = BTreeSet::new();
        for c in &self.cases {
            // `_` leads cargo package names of runners (`_rev16_cando_librunner`).
            let runner_ok = !c.runner.is_empty()
                && c.runner
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
            if !clean(&c.path)
                || !matches!(c.split.as_str(), "public" | "hidden")
                || !seg(&c.library)
                || !seg(&c.symbol)
                || !runner_ok
            {
                return Err(format!("bad case entry {:?}", c.path));
            }
            if !seen.insert(c.path.to_ascii_lowercase()) {
                return Err(format!("duplicate case {:?}", c.path));
            }
        }
        Ok(())
    }

    /// Write `suite.toml` (cases and exclusions sorted by path).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let mut s = self.clone();
        s.cases.sort_by(|a, b| a.path.cmp(&b.path));
        s.excluded.sort_by(|a, b| a.path.cmp(&b.path));
        let body = toml::to_string_pretty(&s)
            .map_err(|e| Error::Invariant(format!("serialize suite: {e}")))?;
        let text = format!(
            "# RuHarness benchmark suite (docs/SCHEMAS.md \"M4 additions\").\n\
             # [[case]] and [[excluded]] are DERIVED by `harness bench vendor`.\n{body}"
        );
        crate::ledger::write_atomic(path, text.as_bytes())
    }

    /// A case by upstream path or by case dir name.
    pub fn case(&self, name: &str) -> Result<&SuiteCase, Error> {
        self.cases
            .iter()
            .find(|c| c.path == name || c.name() == name)
            .ok_or_else(|| Error::Invariant(format!("no case {name:?} in suite {}", self.name)))
    }
}

/// Extract `(library, symbol)` from a runner's `harness!` invocation: the
/// long form's `library: "…"` / `symbol: "…"` string literals, else the
/// short-form defaults (case dir name; case dir name minus `_lib`).
pub fn runner_names(main_rs: &str, case_dir_name: &str) -> (String, String) {
    let literal_after = |key: &str| -> Option<String> {
        let mut rest = main_rs;
        while let Some(pos) = rest.find(key) {
            let before_ok = rest[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            let after = rest[pos + key.len()..].trim_start();
            if before_ok {
                if let Some(after) = after.strip_prefix(':') {
                    let after = after.trim_start();
                    if let Some(body) = after.strip_prefix('"') {
                        if let Some(end) = body.find('"') {
                            return Some(body[..end].to_string());
                        }
                    }
                }
            }
            rest = &rest[pos + key.len()..];
        }
        None
    };
    let library = literal_after("library").unwrap_or_else(|| case_dir_name.to_string());
    let symbol = literal_after("symbol").unwrap_or_else(|| {
        case_dir_name
            .strip_suffix("_lib")
            .unwrap_or(case_dir_name)
            .to_string()
    });
    (library, symbol)
}

// ---------------------------------------------------------------- corpus.lock

/// One locked file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedFile {
    /// Always `file`.
    pub k: String,
    /// Suite-dir-relative path (`cases/…` or `heldout/…`).
    pub path: String,
    /// Upstream-relative path, `""` for a harness-authored locked file.
    pub upstream: String,
    /// `blake3:<hex>` of the bytes.
    pub hash: String,
}

/// `corpus.lock`: header + sorted file lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusLock {
    /// The upstream the `upstream` paths refer to.
    pub upstream: Upstream,
    /// Locked files, sorted by `path`.
    pub files: Vec<LockedFile>,
}

#[derive(Serialize, Deserialize)]
struct LockHeader {
    k: String,
    schema: String,
    schema_version: u64,
    repo: String,
    tag: String,
    commit: String,
}

impl CorpusLock {
    /// Canonical bytes: header line, then one line per file sorted by path.
    pub fn to_canonical(&self) -> Result<String, Error> {
        let header = LockHeader {
            k: "header".into(),
            schema: CORPUS_LOCK_SCHEMA_NAME.into(),
            schema_version: CORPUS_LOCK_SCHEMA_VERSION,
            repo: self.upstream.repo.clone(),
            tag: self.upstream.tag.clone(),
            commit: self.upstream.commit.clone(),
        };
        let mut out = line(&header)?;
        let mut files = self.files.clone();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        for f in &files {
            out.push_str(&line(f)?);
        }
        Ok(out)
    }

    /// Write canonically (atomic).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        crate::ledger::write_atomic(path, self.to_canonical()?.as_bytes())
    }

    /// Load; refuses a foreign/newer header, unsorted or duplicate lines, and
    /// case-folded path collisions.
    pub fn load(path: &Path) -> Result<CorpusLock, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let mut lines = text.lines();
        let header: LockHeader = serde_json::from_str(lines.next().unwrap_or(""))
            .map_err(|e| Error::parse(path, format!("header: {e}")))?;
        if header.schema != CORPUS_LOCK_SCHEMA_NAME || header.k != "header" {
            return Err(Error::parse(path, "not a ruharness-corpus-lock file"));
        }
        if header.schema_version > CORPUS_LOCK_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: header.schema_version,
                supported: CORPUS_LOCK_SCHEMA_VERSION,
            });
        }
        let mut files: Vec<LockedFile> = Vec::new();
        for (i, line) in lines.enumerate() {
            let f: LockedFile = serde_json::from_str(line)
                .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 2)))?;
            if f.k != "file" || !crate::plan::is_clean_relative_path(&f.path) {
                return Err(Error::parse(path, format!("line {}: bad entry", i + 2)));
            }
            files.push(f);
        }
        if files.windows(2).any(|w| w[0].path >= w[1].path) {
            return Err(Error::parse(
                path,
                "entries must be strictly sorted by path",
            ));
        }
        let mut folded = BTreeSet::new();
        for f in &files {
            if !folded.insert(f.path.to_lowercase()) {
                return Err(Error::parse(
                    path,
                    format!("case-folded path collision at {:?}", f.path),
                ));
            }
        }
        Ok(CorpusLock {
            upstream: Upstream {
                repo: header.repo,
                tag: header.tag,
                commit: header.commit,
            },
            files,
        })
    }

    /// Verify the suite dir against the lock (R9): every locked file is a
    /// regular file (not a symlink, `nlink == 1`) with the locked hash, under
    /// its exact byte-equal name; every regular file under `cases/` and
    /// `heldout/` is locked or harness-owned (anchored to suite case dirs);
    /// nothing else (symlinks, fifos, …) exists there. Returns every
    /// violation, sorted; empty = verified.
    pub fn verify(&self, suite_dir: &Path, suite: &Suite) -> Result<Vec<String>, Error> {
        let mut violations = Vec::new();
        if self.upstream != suite.upstream {
            violations.push("corpus.lock upstream differs from suite.toml upstream".into());
        }
        let locked: BTreeMap<&str, &LockedFile> =
            self.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let case_roots: BTreeSet<String> = suite
            .cases
            .iter()
            .map(|c| format!("{CASES_DIR}/{}", c.path))
            .collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for top in [CASES_DIR, HELDOUT_DIR] {
            let dir = suite_dir.join(top);
            if dir.exists() {
                walk(&dir, top, &case_roots, &mut seen, &mut violations)?;
            }
        }
        for (path, f) in &locked {
            if !seen.contains(*path) {
                violations.push(format!("missing or not a regular file: {path}"));
                continue;
            }
            let actual = crate::hash::file_hash(&suite_dir.join(path))?;
            if actual != f.hash {
                violations.push(format!("hash mismatch: {path}"));
            }
        }
        for path in &seen {
            if !locked.contains_key(path.as_str()) {
                violations.push(format!("unlocked file: {path}"));
            }
        }
        violations.sort();
        Ok(violations)
    }

    /// `verify` as a gate: `Err` listing the violations (at most 20 shown).
    pub fn require_verified(&self, suite_dir: &Path, suite: &Suite) -> Result<(), Error> {
        let v = self.verify(suite_dir, suite)?;
        if v.is_empty() {
            return Ok(());
        }
        let shown: Vec<&str> = v.iter().take(20).map(String::as_str).collect();
        Err(Error::Invariant(format!(
            "corpus does not match corpus.lock ({} violation(s)): {}",
            v.len(),
            shown.join("; ")
        )))
    }
}

/// Walk `dir` (suite-relative `rel`), recording regular non-harness-owned
/// files in `seen` and every non-regular entry as a violation. Harness-owned
/// paths — `CASE_LOCAL_FILES` directly in a case root and everything under a
/// case root's `migration/` — are skipped without descending.
fn walk(
    dir: &Path,
    rel: &str,
    case_roots: &BTreeSet<String>,
    seen: &mut BTreeSet<String>,
    violations: &mut Vec<String>,
) -> Result<(), Error> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .collect::<Result<_, _>>()
        .map_err(|e| Error::io(dir, e))?;
    entries.sort_by_key(|e| e.file_name());
    let in_case_root = case_roots.contains(rel);
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            violations.push(format!("non-UTF-8 name under {rel}"));
            continue;
        };
        let child_rel = format!("{rel}/{name}");
        let meta =
            std::fs::symlink_metadata(entry.path()).map_err(|e| Error::io(entry.path(), e))?;
        let ft = meta.file_type();
        if in_case_root && (name == "migration" && ft.is_dir()) {
            continue;
        }
        if in_case_root && CASE_LOCAL_FILES.contains(&name) && ft.is_file() {
            continue;
        }
        if ft.is_dir() {
            walk(&entry.path(), &child_rel, case_roots, seen, violations)?;
        } else if ft.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    violations.push(format!("hard-linked file: {child_rel}"));
                    continue;
                }
            }
            seen.insert(child_rel);
        } else {
            violations.push(format!("not a regular file or dir: {child_rel}"));
        }
    }
    Ok(())
}

/// One canonical JSON line (compact, `\n`-terminated).
fn line<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut s = serde_json::to_string(value)
        .map_err(|e| Error::Invariant(format!("serialize lock line: {e}")))?;
    s.push('\n');
    Ok(s)
}

// ---------------------------------------------------------------- vendoring

/// Vendor the suite from an upstream checkout (`from`): verifies the
/// checkout's detached `HEAD` equals the pinned commit, derives the case list
/// from the batteries (single-`.c` library cases with a runner; others are
/// recorded as excluded), copies `test_case/` into `cases/`, `test_vectors/`
/// (regular `*.json` files only) and `runner/{Cargo.toml,src/**}` into
/// `heldout/`, plus the scorer tool dirs (minus `tests/`; dotfiles are never
/// vendored), and returns the
/// updated suite and the lock of every copied file. Harness-authored
/// `LOCAL_LOCKED` files that already exist are locked too.
///
/// Refuses to overwrite an existing vendored file with different bytes.
pub fn vendor(suite_dir: &Path, suite: &Suite, from: &Path) -> Result<(Suite, CorpusLock), Error> {
    let head_path = from.join(".git").join("HEAD");
    let head = std::fs::read_to_string(&head_path).map_err(|e| Error::io(&head_path, e))?;
    if head.trim() != suite.upstream.commit {
        return Err(Error::Invariant(format!(
            "checkout HEAD is {:?}, not the pinned commit {} (check out the pin, detached)",
            head.trim(),
            suite.upstream.commit
        )));
    }
    let mut out = suite.clone();
    out.cases.clear();
    out.excluded.clear();
    let mut files: Vec<LockedFile> = Vec::new();
    for battery in &suite.batteries {
        let bdir = from.join(&battery.dir);
        let mut names: Vec<String> = std::fs::read_dir(&bdir)
            .map_err(|e| Error::io(&bdir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.ends_with("_lib"))
            .collect();
        names.sort();
        for name in names {
            let path = format!("{}/{name}", battery.dir);
            let case_dir = from.join(&path);
            let c_files = list_files(&case_dir.join("test_case/src"), |n| n.ends_with(".c"))?;
            let main_rs = case_dir.join("runner/src/main.rs");
            let reason = if c_files.len() != 1 {
                Some(format!(
                    "{} .c files (M4 scores single-unit cases only)",
                    c_files.len()
                ))
            } else if !main_rs.is_file() {
                Some("no runner/src/main.rs".to_string())
            } else {
                None
            };
            if let Some(reason) = reason {
                out.excluded.push(Excluded { path, reason });
                continue;
            }
            let main_text =
                std::fs::read_to_string(&main_rs).map_err(|e| Error::io(&main_rs, e))?;
            let (library, symbol) = runner_names(&main_text, &name);
            let cargo_path = case_dir.join("runner/Cargo.toml");
            let cargo_text =
                std::fs::read_to_string(&cargo_path).map_err(|e| Error::io(&cargo_path, e))?;
            let runner = cargo_text
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| t.get("package")?.get("name")?.as_str().map(str::to_string))
                .ok_or_else(|| Error::parse(&cargo_path, "no [package] name"))?;
            // Copy: test_case → cases/, vectors + runner → heldout/.
            copy_tree(
                from,
                &format!("{path}/test_case"),
                suite_dir,
                CASES_DIR,
                &|_| true,
                &mut files,
            )?;
            copy_tree(
                from,
                &format!("{path}/test_vectors"),
                suite_dir,
                HELDOUT_DIR,
                &|rel: &str| rel.ends_with(".json") && !rel.contains('/'),
                &mut files,
            )?;
            copy_tree(
                from,
                &format!("{path}/runner/src"),
                suite_dir,
                HELDOUT_DIR,
                &|_| true,
                &mut files,
            )?;
            copy_file(
                from,
                &format!("{path}/runner/Cargo.toml"),
                suite_dir,
                HELDOUT_DIR,
                &mut files,
            )?;
            out.cases.push(SuiteCase {
                path,
                split: battery.split.clone(),
                library,
                symbol,
                runner,
            });
        }
    }
    for tool in SCORER_TOOL_DIRS {
        copy_tree(
            from,
            tool,
            suite_dir,
            HELDOUT_DIR,
            &|rel: &str| !(rel == "tests" || rel.starts_with("tests/")),
            &mut files,
        )?;
    }
    for local in LOCAL_LOCKED {
        let p = suite_dir.join(local);
        if p.is_file() {
            files.push(LockedFile {
                k: "file".into(),
                path: local.to_string(),
                upstream: String::new(),
                hash: crate::hash::file_hash(&p)?,
            });
        }
    }
    let patches = suite_dir.join(LOCAL_LOCKED_DIR);
    if patches.is_dir() {
        let mut local: Vec<LockedFile> = Vec::new();
        // Reuse the tree copier's walk by "copying" the dir onto itself: the
        // byte-equality guard makes that a pure hashing pass.
        copy_tree(
            suite_dir,
            LOCAL_LOCKED_DIR,
            suite_dir,
            ".",
            &|_| true,
            &mut local,
        )?;
        for mut f in local {
            f.path = f.upstream.clone();
            f.upstream = String::new();
            files.push(f);
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    out.validate().map_err(Error::Invariant)?;
    Ok((
        out,
        CorpusLock {
            upstream: suite.upstream.clone(),
            files,
        },
    ))
}

fn list_files(dir: &Path, keep: impl Fn(&str) -> bool) -> Result<Vec<String>, Error> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| keep(n))
        .collect();
    out.sort();
    Ok(out)
}

/// Copy every regular file under `from/<upstream_rel>` whose path relative
/// to that dir passes `keep`, to `suite_dir/<top>/<upstream_rel>/…`.
fn copy_tree(
    from: &Path,
    upstream_rel: &str,
    suite_dir: &Path,
    top: &str,
    keep: &dyn Fn(&str) -> bool,
    files: &mut Vec<LockedFile>,
) -> Result<(), Error> {
    let mut stack = vec![String::new()];
    while let Some(sub) = stack.pop() {
        let dir = if sub.is_empty() {
            from.join(upstream_rel)
        } else {
            from.join(upstream_rel).join(&sub)
        };
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| Error::io(&dir, e))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                return Err(Error::Invariant(format!(
                    "non-UTF-8 name under {}",
                    dir.display()
                )));
            };
            // Dotfiles (`.gitignore`, …) are upstream VCS metadata, never
            // built or scored — and never clean paths under plan rules.
            if name.starts_with('.') {
                continue;
            }
            let rel = if sub.is_empty() {
                name.clone()
            } else {
                format!("{sub}/{name}")
            };
            let ft = std::fs::symlink_metadata(entry.path())
                .map_err(|e| Error::io(entry.path(), e))?
                .file_type();
            if !keep(&rel) {
                continue;
            }
            if ft.is_dir() {
                stack.push(rel);
            } else if ft.is_file() {
                copy_file(
                    from,
                    &format!("{upstream_rel}/{rel}"),
                    suite_dir,
                    top,
                    files,
                )?;
            } else {
                return Err(Error::Invariant(format!(
                    "upstream {upstream_rel}/{rel} is not a regular file or dir"
                )));
            }
        }
    }
    Ok(())
}

fn copy_file(
    from: &Path,
    upstream_path: &str,
    suite_dir: &Path,
    top: &str,
    files: &mut Vec<LockedFile>,
) -> Result<(), Error> {
    if !crate::plan::is_clean_relative_path(upstream_path) {
        return Err(Error::Invariant(format!(
            "upstream path {upstream_path:?} is not a clean relative path"
        )));
    }
    let src = from.join(upstream_path);
    let bytes = std::fs::read(&src).map_err(|e| Error::io(&src, e))?;
    let local = format!("{top}/{upstream_path}");
    let dst = suite_dir.join(&local);
    if dst.exists() {
        let existing = std::fs::read(&dst).map_err(|e| Error::io(&dst, e))?;
        if existing != bytes {
            return Err(Error::Invariant(format!(
                "refusing to overwrite vendored {local} with different bytes"
            )));
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&dst, &bytes).map_err(|e| Error::io(&dst, e))?;
    }
    files.push(LockedFile {
        k: "file".into(),
        path: local,
        upstream: upstream_path.to_string(),
        hash: crate::hash::bytes_hash(&bytes),
    });
    Ok(())
}

// ---------------------------------------------------------------- scores.json

/// Pass/fail counts of one side over a case's vectors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Vectors that passed.
    pub pass: u32,
    /// Vectors that failed (any non-pass cando result, timeouts included).
    pub fail: u32,
    /// Vectors skipped (`has_ub`).
    pub skip: u32,
    /// Vectors not run (no artifact to run).
    pub not_run: u32,
    /// Vectors excused as `unmarked-ub` (the sanitized C pass proved the C
    /// memory-unsafe on them — [`is_unmarked_ub`]); counted here INSTEAD of
    /// pass/fail on every side, so totals and classes agree.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unmarked_ub: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// One vector's results. Result strings (closed): `pass`, `skip`,
/// `not-run`, `timeout`, `fail:<cando ResultType>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorScore {
    /// Vector file name (e.g. `1.json`).
    pub name: String,
    /// C baseline result.
    pub c: String,
    /// Verified Rust result (`not-run` when the case has no verified unit).
    pub rust: String,
    /// Latest UNverified candidate's result, when one was scored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    /// The SANITIZED C pass (docs/ORACLE-HARDENING.md §A), run only on a
    /// vector the plain C passed and the Rust or candidate did not. Closed:
    /// `clean` | `ub:<allow-listed ASan kind>` (excused) |
    /// `sanitizer:<other kind>` (recorded, not excused) | `fail:<cando
    /// result>` (abnormal end without a report: not excused) | an infra
    /// result ([`is_infra_result`]; a PROBLEM). Absent = not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_sanitized: Option<String>,
}

/// Whether a vector is excused as `unmarked-ub`: the sanitized C pass
/// reported an allow-listed memory error on it. Excluded from a case's
/// non-UB set exactly like `has_ub`, and counted separately.
pub fn is_unmarked_ub(v: &VectorScore) -> bool {
    v.c_sanitized
        .as_deref()
        .is_some_and(|s| s.starts_with("ub:"))
}

/// Digests a case's score is bound to (any change ⇒ re-score required).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseInputs {
    /// Unit source file-set hash (`""` before the case is planned).
    pub unit_source: String,
    /// Driver digest (`""` when none).
    pub driver: String,
    /// Driver-validation record digest (`""` when none).
    pub validation: String,
    /// Verified crate digest (`""` when not verified).
    pub rust_crate: String,
    /// Scored unverified candidate digest (`""` when none).
    pub candidate: String,
}

/// Pipeline facts about a case (from its ledger).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasePipeline {
    /// Unit id (`""` before planning).
    pub unit: String,
    /// Plan status of the unit (`""` before planning).
    pub status: String,
    /// `validated | stale | failed | missing`.
    pub driver: String,
    /// Driver mutation kills / compiled, when validated.
    pub mutation: Option<(u32, u32)>,
    /// Turns the green driver attempt took.
    pub driver_turns: Option<u32>,
    /// Turns the promoted (green) migrate attempt took.
    pub migrate_turns: Option<u32>,
    /// Latest migrate attempt outcome (`""` when none).
    pub migrate_outcome: String,
    /// The unverified candidate scored for this case (`""` when none): only a
    /// FINISHED red attempt whose last turn failed on behavior (`oracle`) —
    /// a candidate that built, ran, and differed from the C on the driver.
    #[serde(default)]
    pub candidate_attempt: String,
}

/// One case's score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseScore {
    /// Upstream case path.
    pub case: String,
    /// `public | hidden`.
    pub split: String,
    /// Closed: `strict-pass` (verified; every non-UB vector passes) |
    /// `blind-spot` (verified; some non-UB vector fails on a real cando
    /// outcome) | `unverified` | `stale-verified` (plan says verified, but the
    /// latest verdict is not green over the current digests, or the driver is
    /// not freshly validated — never scored as verified) | `unscorable` (no
    /// non-UB vector) | `c-baseline-invalid` (the C itself fails a non-UB
    /// vector on a real cando outcome on this platform) | `infra-error` (a
    /// build/runner/report failure on either side: a harness problem, never a
    /// measurement).
    pub class: String,
    /// Pipeline facts.
    pub pipeline: CasePipeline,
    /// What the score is bound to.
    pub inputs: CaseInputs,
    /// C baseline counts.
    pub c_baseline: Counts,
    /// Verified-Rust counts.
    pub rust: Counts,
    /// Unverified-candidate counts, when one was scored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<Counts>,
    /// Which sanitized C build ran for this case (docs/ORACLE-HARDENING.md
    /// §A.2): `asan+bounds-safety` | `asan` (the C does not compile with
    /// `-fbounds-safety`) | `none` (neither built: a PROBLEM). Absent = the
    /// pass did not run for this case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sanitized_build: Option<String>,
    /// Per-vector results, sorted by name.
    pub vectors: Vec<VectorScore>,
}

/// Aggregates of one split (all integers; rates are derived in renderings).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitTotals {
    /// `public | hidden`.
    pub split: String,
    /// Cases in the split.
    pub cases: u32,
    /// Cases with ≥ 1 non-UB vector and a valid C baseline (the denominator).
    pub scorable: u32,
    /// Scorable cases whose unit is verified.
    pub verified: u32,
    /// `strict-pass` cases (the headline numerator).
    pub strict_pass: u32,
    /// `blind-spot` cases.
    pub blind_spots: u32,
    /// Unverified cases whose scored candidate (a finished attempt that the
    /// oracle rejected on behavior) nevertheless passes every non-UB vector:
    /// "vector-pass / oracle-red" — a lead to triage by hand (the oracle may
    /// have caught a real bug the vectors cannot see), NOT a false negative
    /// by itself.
    pub vector_pass_oracle_red: u32,
    /// `stale-verified` cases (counted in `scorable`, not in `verified`).
    #[serde(default)]
    pub stale_verified: u32,
    /// `infra-error` cases (excluded from `scorable`; any is a harness error).
    #[serde(default)]
    pub infra_errors: u32,
    /// `unscorable` cases.
    pub unscorable: u32,
    /// `c-baseline-invalid` cases.
    pub c_baseline_invalid: u32,
    /// Non-UB vectors in scorable cases.
    pub vectors: u32,
    /// Of those, passed by verified Rust.
    pub vectors_passed: u32,
    /// UB vectors skipped (all cases).
    pub vectors_skipped: u32,
    /// Vectors excused as `unmarked-ub` (all cases; [`is_unmarked_ub`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vectors_unmarked_ub: u32,
}

/// `scores.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scores {
    /// Always [`SCORES_SCHEMA_NAME`].
    pub schema: String,
    /// Schema version.
    pub schema_version: u64,
    /// Suite name.
    pub suite: String,
    /// Digest of `corpus.lock`.
    pub corpus_lock: String,
    /// Digest of `heldout/Cargo.lock` (the scorer's dependency pin).
    pub scorer_lock: String,
    /// Environment fingerprint (rustc, cc, OS, sandbox mode, cflags).
    pub environment: Vec<String>,
    /// Per split, sorted by split name.
    pub totals: Vec<SplitTotals>,
    /// Per case, sorted by case path.
    pub cases: Vec<CaseScore>,
}

impl Scores {
    /// Sort canonically and derive `totals` from `cases`.
    pub fn finalize(&mut self) {
        self.cases.sort_by(|a, b| a.case.cmp(&b.case));
        for c in &mut self.cases {
            c.vectors.sort_by(|a, b| a.name.cmp(&b.name));
        }
        let mut by_split: BTreeMap<String, SplitTotals> = BTreeMap::new();
        for c in &self.cases {
            let t = by_split
                .entry(c.split.clone())
                .or_insert_with(|| SplitTotals {
                    split: c.split.clone(),
                    ..SplitTotals::default()
                });
            t.cases += 1;
            t.vectors_skipped += c.c_baseline.skip;
            t.vectors_unmarked_ub += c.c_baseline.unmarked_ub;
            match c.class.as_str() {
                "unscorable" => t.unscorable += 1,
                "c-baseline-invalid" => t.c_baseline_invalid += 1,
                "infra-error" => t.infra_errors += 1,
                _ => {
                    t.scorable += 1;
                    t.vectors += c.c_baseline.pass + c.c_baseline.fail;
                    t.vectors_passed += c.rust.pass;
                    match c.class.as_str() {
                        "strict-pass" => {
                            t.verified += 1;
                            t.strict_pass += 1;
                        }
                        "blind-spot" => {
                            t.verified += 1;
                            t.blind_spots += 1;
                        }
                        other => {
                            if other == "stale-verified" {
                                t.stale_verified += 1;
                            }
                            if c.candidate
                                .as_ref()
                                .is_some_and(|k| k.fail == 0 && k.not_run == 0 && k.pass > 0)
                            {
                                t.vector_pass_oracle_red += 1;
                            }
                        }
                    }
                }
            }
        }
        self.totals = by_split.into_values().collect();
    }

    /// Atomic pretty-JSON write (call [`Scores::finalize`] first).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Invariant(format!("serialize scores: {e}")))?;
        text.push('\n');
        crate::ledger::write_atomic(path, text.as_bytes())
    }

    /// Load, refusing a foreign/newer schema.
    pub fn load(path: &Path) -> Result<Scores, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let s: Scores =
            serde_json::from_str(&text).map_err(|e| Error::parse(path, e.to_string()))?;
        if s.schema != SCORES_SCHEMA_NAME {
            return Err(Error::parse(path, "not a ruharness-bench-scores file"));
        }
        if s.schema_version > SCORES_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: s.schema_version,
                supported: SCORES_SCHEMA_VERSION,
            });
        }
        Ok(s)
    }
}

/// Whether a vector result is a harness/infrastructure failure rather than
/// a measurement: the side did not build or link, the runner did not exit
/// normally, or its report was missing/malformed. Real cando outcomes
/// (`fail:VectorComparisonFailed`, `fail:Panic`, `fail:SegmentationFault`,
/// `fail:UnknownFailure`, …) and `timeout` are measurements.
pub fn is_infra_result(result: &str) -> bool {
    matches!(
        result,
        "fail:dylib-build"
            | "fail:build"
            | "fail:no-report"
            | "fail:bad-report"
            | "fail:runner-killed"
    ) || result.starts_with("fail:runner-exit-")
}

/// How a case's unit stands for scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    /// Latest verdict green over the current digests, driver validated.
    Verified,
    /// Plan says verified/merged, but the evidence is not fresh and green.
    Stale,
    /// Not verified.
    No,
}

/// Classify a case from its per-vector results (see [`CaseScore::class`]).
pub fn classify_case(vectors: &[VectorScore], verification: Verification) -> &'static str {
    let non_ub: Vec<&VectorScore> = vectors
        .iter()
        .filter(|v| v.c != "skip" && !is_unmarked_ub(v))
        .collect();
    if non_ub.is_empty() {
        return "unscorable";
    }
    let verified = verification == Verification::Verified;
    if non_ub.iter().any(|v| is_infra_result(&v.c))
        || (verified && non_ub.iter().any(|v| is_infra_result(&v.rust)))
    {
        return "infra-error";
    }
    if non_ub.iter().any(|v| v.c != "pass") {
        return "c-baseline-invalid";
    }
    match verification {
        Verification::No => "unverified",
        Verification::Stale => "stale-verified",
        Verification::Verified if non_ub.iter().all(|v| v.rust == "pass") => "strict-pass",
        Verification::Verified => "blind-spot",
    }
}

/// Outcome of comparing a fresh score run against the committed baseline
/// (R10). Every list is sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Environment fingerprint or scorer/corpus lock differs: nothing below
    /// is meaningful (exit 1, "incomparable").
    pub incomparable: Vec<String>,
    /// Cases whose recorded inputs changed (re-score required, exit 1).
    pub input_changes: Vec<String>,
    /// C-side vector flips with unchanged inputs (environment drift,
    /// reported, not a regression).
    pub drift: Vec<String>,
    /// Rust `pass` → anything else with unchanged inputs (exit 10).
    pub regressions: Vec<String>,
    /// Rust non-pass → `pass` (reported; record with `bench score --write`).
    pub improvements: Vec<String>,
    /// Cases present on one side only.
    pub membership: Vec<String>,
    /// Split totals that differ (reported; the per-vector lists say why).
    pub totals_changes: Vec<String>,
}

/// Compare `now` against the committed `baseline`, per vector.
pub fn compare(baseline: &Scores, now: &Scores) -> Comparison {
    let mut out = Comparison::default();
    if baseline.environment != now.environment {
        out.incomparable.push(format!(
            "environment fingerprint differs: recorded {:?}, now {:?}",
            baseline.environment, now.environment
        ));
    }
    for (what, a, b) in [
        ("corpus_lock", &baseline.corpus_lock, &now.corpus_lock),
        ("scorer_lock", &baseline.scorer_lock, &now.scorer_lock),
    ] {
        if a != b {
            out.incomparable.push(format!("{what} differs"));
        }
    }
    let old: BTreeMap<&str, &CaseScore> = baseline
        .cases
        .iter()
        .map(|c| (c.case.as_str(), c))
        .collect();
    let new: BTreeMap<&str, &CaseScore> = now.cases.iter().map(|c| (c.case.as_str(), c)).collect();
    for name in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        let (Some(o), Some(n)) = (old.get(name), new.get(name)) else {
            out.membership.push((*name).to_string());
            continue;
        };
        let was_verified = matches!(o.class.as_str(), "strict-pass" | "blind-spot");
        let lost = matches!(
            n.class.as_str(),
            "unverified" | "stale-verified" | "infra-error"
        );
        if was_verified && lost {
            // Losing verified status is a regression whatever else changed
            // (a move to c-baseline-invalid/unscorable is the C side or the
            // corpus: drift, reported via the vector lists and totals).
            out.regressions
                .push(format!("{name}: {} -> {}", o.class, n.class));
        }
        if o.inputs != n.inputs {
            out.input_changes.push((*name).to_string());
            continue;
        }
        let ov: BTreeMap<&str, &VectorScore> =
            o.vectors.iter().map(|v| (v.name.as_str(), v)).collect();
        for v in &n.vectors {
            let Some(prev) = ov.get(v.name.as_str()) else {
                out.membership.push(format!("{name}/{}", v.name));
                continue;
            };
            if prev.c != v.c {
                out.drift
                    .push(format!("{name}/{}: C {} -> {}", v.name, prev.c, v.c));
            }
            if prev.c_sanitized != v.c_sanitized {
                let show = |s: &Option<String>| s.as_deref().unwrap_or("not-run").to_string();
                let change = format!(
                    "{name}/{}: C sanitized {} -> {}",
                    v.name,
                    show(&prev.c_sanitized),
                    show(&v.c_sanitized)
                );
                // An excusal that no longer holds while a MEASURED Rust still
                // does not pass is a verified vector newly failing (R-A5);
                // `not-run` (no verified unit: a candidate triggered the
                // pass) is never a Rust regression.
                if is_unmarked_ub(prev)
                    && !is_unmarked_ub(v)
                    && v.rust != "pass"
                    && v.rust != "not-run"
                {
                    out.regressions
                        .push(format!("{change} (excusal lost; Rust {})", v.rust));
                } else {
                    out.drift.push(change);
                }
            }
            if prev.rust == "pass" && v.rust != "pass" {
                out.regressions
                    .push(format!("{name}/{}: Rust pass -> {}", v.name, v.rust));
            } else if prev.rust != "pass" && v.rust == "pass" {
                out.improvements
                    .push(format!("{name}/{}: Rust {} -> pass", v.name, prev.rust));
            }
        }
    }
    if baseline.totals != now.totals {
        let show = |t: &[SplitTotals]| {
            t.iter()
                .map(|t| {
                    format!(
                        "{}: strict {}/{} verified {} stale {} blind {} infra {} vectors {}/{}",
                        t.split,
                        t.strict_pass,
                        t.scorable,
                        t.verified,
                        t.stale_verified,
                        t.blind_spots,
                        t.infra_errors,
                        t.vectors_passed,
                        t.vectors
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        out.totals_changes.push(format!(
            "recorded [{}] now [{}]",
            show(&baseline.totals),
            show(&now.totals)
        ));
    }
    out.regressions.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_names_long_and_short_form() {
        let long =
            "harness! {\n    library: \"driver\",\n    symbol: \"my_pow\",\n    state: {}\n}";
        assert_eq!(
            runner_names(long, "007_errno_pow_lib"),
            ("driver".into(), "my_pow".into())
        );
        let short = "harness! {\n    state: { x: u32 },\n    signature: unsafe extern \"C\" fn(u32) -> u32,\n}";
        assert_eq!(
            runner_names(short, "rev16_lib"),
            ("rev16_lib".into(), "rev16".into())
        );
        // `signature:` must not be mistaken for a `library`/`symbol` key.
        let tricky = "let xsymbol: u8 = 0; harness! { symbol: \"s\" }";
        assert_eq!(runner_names(tricky, "a_lib").1, "s");
    }

    fn vs(name: &str, c: &str, rust: &str) -> VectorScore {
        VectorScore {
            name: name.into(),
            c: c.into(),
            rust: rust.into(),
            candidate: None,
            c_sanitized: None,
        }
    }

    fn excused(name: &str, rust: &str) -> VectorScore {
        VectorScore {
            c_sanitized: Some("ub:stack-buffer-overflow".into()),
            ..vs(name, "pass", rust)
        }
    }

    /// docs/ORACLE-HARDENING.md §A: a vector the sanitized C proves
    /// memory-unsafe is excluded like `has_ub`; any other sanitized result is
    /// recorded and changes nothing.
    #[test]
    fn unmarked_ub_vectors_are_excluded_like_has_ub() {
        // decorrelate's shape: correct Rust fails only the UB vector.
        assert_eq!(
            classify_case(
                &[
                    vs("1", "pass", "pass"),
                    excused("2", "fail:VectorComparisonFailed")
                ],
                Verification::Verified
            ),
            "strict-pass"
        );
        // Every non-UB vector excused: nothing left to score.
        assert_eq!(
            classify_case(&[excused("1", "fail:Panic")], Verification::Verified),
            "unscorable"
        );
        // Not excused: another kind, an abnormal end without a report, clean.
        for other in [
            "sanitizer:alloc-dealloc-mismatch",
            "fail:SegmentationFault",
            "clean",
        ] {
            let v = VectorScore {
                c_sanitized: Some(other.into()),
                ..vs("1", "pass", "fail:Panic")
            };
            assert!(!is_unmarked_ub(&v), "{other}");
            assert_eq!(
                classify_case(&[v], Verification::Verified),
                "blind-spot",
                "{other}"
            );
        }
    }

    #[test]
    fn totals_count_unmarked_ub_apart() {
        let mut s = scores("pass", "pass", "e");
        s.cases[0].vectors.push(excused("2.json", "fail:Panic"));
        s.cases[0].c_baseline.unmarked_ub = 1;
        s.cases[0].rust.unmarked_ub = 1;
        s.finalize();
        let t = &s.totals[0];
        assert_eq!((t.strict_pass, t.vectors, t.vectors_passed), (1, 1, 1));
        assert_eq!(t.vectors_unmarked_ub, 1);
        // Absent when zero: M4 records stay byte-identical.
        let json = serde_json::to_string(&scores("pass", "pass", "e")).unwrap();
        assert!(
            !json.contains("unmarked_ub") && !json.contains("c_sanitized"),
            "{json}"
        );
    }

    /// R-A5: an excusal that stops holding while the Rust still fails is a
    /// regression; other sanitized changes are drift.
    #[test]
    fn a_lost_excusal_with_a_failing_rust_is_a_regression() {
        let mut base = scores("fail:Panic", "pass", "e1");
        base.cases[0].vectors[0].c_sanitized = Some("ub:heap-buffer-overflow".into());
        let mut now = base.clone();
        now.cases[0].vectors[0].c_sanitized = Some("clean".into());
        let r = compare(&base, &now);
        assert_eq!(r.regressions.len(), 1, "{r:?}");
        assert!(r.regressions[0].contains("excusal lost"), "{r:?}");

        let mut drifted = base.clone();
        drifted.cases[0].vectors[0].c_sanitized = Some("ub:stack-buffer-overflow".into());
        let r = compare(&base, &drifted);
        assert!(r.regressions.is_empty(), "{r:?}");
        assert_eq!(r.drift.len(), 1);

        // Code review: an unverified case (Rust `not-run`, the pass
        // triggered by a candidate) never yields a Rust regression.
        let mut unverified = base.clone();
        unverified.cases[0].vectors[0].rust = "not-run".into();
        let mut lost = unverified.clone();
        lost.cases[0].vectors[0].c_sanitized = Some("clean".into());
        let r = compare(&unverified, &lost);
        assert!(r.regressions.is_empty(), "{r:?}");
        assert_eq!(r.drift.len(), 1);
    }

    #[test]
    fn classification() {
        assert_eq!(
            classify_case(&[vs("1", "skip", "skip")], Verification::Verified),
            "unscorable"
        );
        assert_eq!(classify_case(&[], Verification::Verified), "unscorable");
        assert_eq!(
            classify_case(
                &[vs("1", "fail:VectorComparisonFailed", "pass")],
                Verification::Verified
            ),
            "c-baseline-invalid"
        );
        assert_eq!(
            classify_case(&[vs("1", "pass", "not-run")], Verification::No),
            "unverified"
        );
        assert_eq!(
            classify_case(
                &[vs("1", "pass", "pass"), vs("2", "skip", "skip")],
                Verification::Verified
            ),
            "strict-pass"
        );
        assert_eq!(
            classify_case(
                &[vs("1", "pass", "pass"), vs("2", "pass", "timeout")],
                Verification::Verified
            ),
            "blind-spot"
        );
    }

    #[test]
    fn infra_and_stale_are_never_measurements() {
        // Review (M4): a harness failure must not read as a blind spot or as
        // an invalid C baseline, and a stale "verified" must not score.
        assert_eq!(
            classify_case(
                &[vs("1", "pass", "fail:dylib-build")],
                Verification::Verified
            ),
            "infra-error"
        );
        assert_eq!(
            classify_case(
                &[vs("1", "fail:runner-exit-2", "not-run")],
                Verification::No
            ),
            "infra-error"
        );
        assert_eq!(
            classify_case(&[vs("1", "pass", "not-run")], Verification::Stale),
            "stale-verified"
        );
        assert_eq!(
            classify_case(&[vs("1", "pass", "fail:build")], Verification::No),
            "unverified",
            "an unverified side is never scored, so its build failure is moot"
        );
        assert_eq!(
            classify_case(&[vs("1", "pass", "fail:Panic")], Verification::Verified),
            "blind-spot",
            "a real cando outcome is a measurement"
        );
    }

    #[test]
    fn losing_verified_status_is_a_regression_even_with_changed_inputs() {
        let mut base = scores("pass", "pass", "e1");
        base.cases[0].inputs.rust_crate = "blake3:verified-crate".into();
        let mut now = scores("pass", "pass", "e1");
        now.cases[0].class = "stale-verified".into();
        now.cases[0].inputs.rust_crate = String::new();
        now.finalize();
        let r = compare(&base, &now);
        assert_eq!(r.regressions.len(), 1, "{r:?}");
        assert_eq!(r.input_changes.len(), 1);
        assert_eq!(r.totals_changes.len(), 1);
    }

    fn scores(rust: &str, c: &str, env: &str) -> Scores {
        let vectors = vec![vs("1.json", c, rust)];
        let class = classify_case(&vectors, Verification::Verified).to_string();
        let mut s = Scores {
            schema: SCORES_SCHEMA_NAME.into(),
            schema_version: 1,
            suite: "s".into(),
            corpus_lock: "blake3:a".into(),
            scorer_lock: "blake3:b".into(),
            environment: vec![env.into()],
            totals: vec![],
            cases: vec![CaseScore {
                case: "P/B/x_lib".into(),
                split: "public".into(),
                class,
                pipeline: CasePipeline::default(),
                inputs: CaseInputs::default(),
                c_baseline: Counts {
                    pass: 1,
                    ..Counts::default()
                },
                rust: Counts {
                    pass: u32::from(rust == "pass"),
                    fail: u32::from(rust != "pass"),
                    ..Counts::default()
                },
                candidate: None,
                sanitized_build: None,
                vectors,
            }],
        };
        s.finalize();
        s
    }

    #[test]
    fn compare_flags_regressions_drift_and_incomparable() {
        let base = scores("pass", "pass", "e1");
        assert_eq!(compare(&base, &base), Comparison::default());
        let r = compare(&base, &scores("fail:Panic", "pass", "e1"));
        assert_eq!(r.regressions.len(), 1);
        let r = compare(&scores("fail:Panic", "pass", "e1"), &base);
        assert_eq!(r.improvements.len(), 1);
        let r = compare(&base, &scores("pass", "fail:Timeout", "e1"));
        assert_eq!(r.drift.len(), 1);
        assert!(r.regressions.is_empty());
        let r = compare(&base, &scores("pass", "pass", "e2"));
        assert_eq!(r.incomparable.len(), 1);
        let mut changed = scores("pass", "pass", "e1");
        changed.cases[0].inputs.rust_crate = "blake3:new".into();
        let r = compare(&base, &changed);
        assert_eq!(r.input_changes, vec!["P/B/x_lib".to_string()]);
    }

    #[test]
    fn totals_use_scorable_denominator() {
        let s = scores("pass", "pass", "e");
        let t = &s.totals[0];
        assert_eq!(
            (
                t.cases,
                t.scorable,
                t.strict_pass,
                t.vectors,
                t.vectors_passed
            ),
            (1, 1, 1, 1, 1)
        );
    }

    #[test]
    fn lock_round_trip_and_verify() {
        let dir = std::env::temp_dir().join(format!("ruharness-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let case = "Public-Tests/B/x_lib";
        std::fs::create_dir_all(dir.join(format!("cases/{case}/test_case/src"))).unwrap();
        std::fs::create_dir_all(dir.join(format!("cases/{case}/migration/units"))).unwrap();
        std::fs::create_dir_all(dir.join(format!("heldout/{case}/test_vectors"))).unwrap();
        std::fs::write(
            dir.join(format!("cases/{case}/test_case/src/lib.c")),
            "int x;\n",
        )
        .unwrap();
        std::fs::write(dir.join(format!("cases/{case}/harness.toml")), "x\n").unwrap();
        std::fs::write(
            dir.join(format!("cases/{case}/migration/units/free")),
            "x\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("heldout/{case}/test_vectors/1.json")),
            "{}\n",
        )
        .unwrap();
        let upstream = Upstream {
            repo: "r".into(),
            tag: "v2".into(),
            commit: "0".repeat(40),
        };
        let suite = Suite {
            schema_version: 1,
            name: "s".into(),
            upstream: upstream.clone(),
            batteries: vec![],
            cases: vec![SuiteCase {
                path: case.into(),
                split: "public".into(),
                library: "x_lib".into(),
                symbol: "x".into(),
                runner: "_x_cando_librunner".into(),
            }],
            excluded: vec![],
        };
        let mut files = Vec::new();
        for rel in [
            format!("cases/{case}/test_case/src/lib.c"),
            format!("heldout/{case}/test_vectors/1.json"),
        ] {
            files.push(LockedFile {
                k: "file".into(),
                upstream: rel.split_once('/').unwrap().1.to_string(),
                hash: crate::hash::file_hash(&dir.join(&rel)).unwrap(),
                path: rel,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let lock = CorpusLock { upstream, files };
        lock.store(&dir.join("corpus.lock")).unwrap();
        let loaded = CorpusLock::load(&dir.join("corpus.lock")).unwrap();
        assert_eq!(loaded, lock);
        assert_eq!(lock.verify(&dir, &suite).unwrap(), Vec::<String>::new());

        // Tamper, add a stray file, add a symlink: all reported.
        std::fs::write(
            dir.join(format!("heldout/{case}/test_vectors/1.json")),
            "{\"x\":1}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("heldout/{case}/test_vectors/2.json")),
            "{}\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "/etc/hosts",
            dir.join(format!("cases/{case}/test_case/link")),
        )
        .unwrap();
        let v = lock.verify(&dir, &suite).unwrap();
        assert!(v.iter().any(|m| m.starts_with("hash mismatch")), "{v:?}");
        assert!(v.iter().any(|m| m.starts_with("unlocked file")), "{v:?}");
        #[cfg(unix)]
        assert!(v.iter().any(|m| m.starts_with("not a regular")), "{v:?}");
        assert!(lock.require_verified(&dir, &suite).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
