//! The file tree's states (docs/COCKPIT-WRAPPER-DESIGN.md §2.1, §2.3): which
//! files the scanner reads, and what state each file, function and unit is
//! in — with a glyph AND a word, colour never the only signal.
//!
//! [`walk_tree`] runs on the loader thread (directory entries and metadata
//! only, within [`TREE_LIMITS`]); [`build`] is pure over the snapshot and
//! that walk, and hashes nothing: the snapshot already carries the stale
//! paths and each unit crate's content hash.

use crate::model::{ProvenanceView, Snapshot, UnitView};
use harness_core::facts::Facts;
use harness_core::status::VerdictState;
use harness_core::walk::{self, Limits, Skip};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The C frontend's extensions (harness-scan reads `*.c` and `*.h`).
pub const C_EXTENSIONS: [&str; 2] = ["c", "h"];

/// The tree's bounds: they bound the listing, not a full count.
pub const TREE_LIMITS: Limits = Limits {
    max_files: Some(20_000),
    max_depth: Some(32),
};

/// What the loader's walk of the source directory found, repo-relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeWalk {
    /// The target's `source_dir`, repo-relative (`src/zopfli`).
    pub source_dir: String,
    /// The files listed, repo-relative with `/`, in walk order.
    pub listed: Vec<String>,
    /// Files the facts record that the walk did not return and that are
    /// absent (`lstat` says so) — "missing". One present but beyond the
    /// limits is simply not listed.
    pub absent: BTreeSet<String>,
    /// Matching entries left out (a FIFO named `a.c`).
    pub skipped: Vec<(String, Skip)>,
    /// Entries that could not be read, and why.
    pub errors: Vec<(String, String)>,
    /// A limit cut the listing short.
    pub truncated: bool,
}

/// `path` relative to `root`, with `/` (the scanner's form); the path
/// itself when it is not under `root`.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Walk the target's source directory for the tree (on the loader thread).
pub fn walk_tree(root: &Path, source_dir: &str, facts: Option<&Facts>) -> TreeWalk {
    let walked = walk::confined(&root.join(source_dir), &C_EXTENSIONS, TREE_LIMITS);
    let listed: Vec<String> = walked.files.iter().map(|p| relative(root, p)).collect();
    let seen: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
    let absent = facts
        .map(|f| {
            f.files
                .iter()
                .filter(|r| !seen.contains(r.path.as_str()))
                .filter(|r| {
                    matches!(
                        std::fs::symlink_metadata(root.join(&r.path)),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound
                    )
                })
                .map(|r| r.path.clone())
                .collect()
        })
        .unwrap_or_default();
    TreeWalk {
        source_dir: relative(root, &root.join(source_dir)),
        absent,
        skipped: walked
            .skipped
            .iter()
            .map(|(p, why)| (relative(root, p), *why))
            .collect(),
        errors: walked
            .errors
            .iter()
            .map(|(p, e)| (relative(root, p), e.clone()))
            .collect(),
        truncated: walked.truncated,
        listed,
    }
}

/// Why a unit needs attention — each a cause the user can fix, named in the
/// View with its next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// Status and verdict evidence disagree.
    Contradiction,
    /// A promotion was interrupted ("the next command recovers it").
    PromotionInterrupted(String),
    /// The unit's C changed since it was planned (or since the verdict).
    SourceChanged,
    /// The verdict no longer matches the crate or driver ("Re-check").
    StaleVerdict,
    /// The crate matches neither a recorded attempt's candidate nor what the
    /// oracle last judged.
    ChangedOutside,
    /// The crate matches no recorded attempt, and there is no verdict to
    /// compare it with (deleted or unreadable): whose code it is cannot be
    /// told (review ENG-11).
    NoEvidence,
}

impl Cause {
    /// The cause and its next step, in words.
    pub fn words(&self) -> String {
        match self {
            Cause::Contradiction => {
                "its status and its verdict disagree — Re-check it (`harness state status` \
                 explains)"
                    .into()
            }
            Cause::PromotionInterrupted(id) => {
                format!("the promotion of {id} was interrupted — the next command recovers it")
            }
            Cause::SourceChanged => {
                "its C changed since it was planned — scan, refresh the plan, then review its \
                 diff"
                    .into()
            }
            Cause::StaleVerdict => "its verdict is out of date — Re-check it".into(),
            Cause::ChangedOutside => {
                "changed outside the harness — restore it, or record it with `harness \
                 override` (see Help)"
                    .into()
            }
            Cause::NoEvidence => {
                "its verdict is missing, so the harness cannot tell whose code the crate is — \
                 restore oracle-latest.json (git), or record the crate with `harness override` \
                 (see Help)"
                    .into()
            }
        }
    }
}

/// Who a migrated unit's crate is attributed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Unassisted pipeline output.
    Pipeline,
    /// A steer attempt (a reviewer's note guided it).
    Steered,
    /// A hand edit.
    Human,
}

/// A unit's state (§2.3, first match wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitState {
    /// `⊘ blocked` (unit rows only).
    Blocked,
    /// 7 `⚠ needs attention`.
    Attention(Cause),
    /// 8 `✗ failing`: the current verdict is red.
    Failing,
    /// 9 `✓ migrated`: verified or merged, green, fresh, attributed.
    Migrated(Origin),
    /// 10 `✓? verified, origin not recorded`.
    OriginUnknown,
    /// 11 `◐ tried`: pending, with an attempt.
    Tried,
    /// 12 `◇ planned`: pending, no attempt.
    Planned,
    /// Any other combination: the status word.
    Other(String),
}

impl UnitState {
    /// Its glyph (`""` for the fallback, which shows its word alone).
    pub fn glyph(&self) -> &'static str {
        match self {
            UnitState::Blocked => "⊘",
            UnitState::Attention(_) => "⚠",
            UnitState::Failing => "✗",
            UnitState::Migrated(_) => "✓",
            UnitState::OriginUnknown => "✓?",
            UnitState::Tried => "◐",
            UnitState::Planned => "◇",
            // The fallback has no glyph (§2.3): only its word.
            UnitState::Other(_) => "",
        }
    }

    /// Its word.
    pub fn word(&self) -> String {
        match self {
            UnitState::Blocked => "blocked".into(),
            UnitState::Attention(_) => "needs attention".into(),
            UnitState::Failing => "failing".into(),
            UnitState::Migrated(Origin::Pipeline) => "migrated".into(),
            UnitState::Migrated(Origin::Steered) => "migrated (steered)".into(),
            UnitState::Migrated(Origin::Human) => "migrated (by hand)".into(),
            UnitState::OriginUnknown => "verified, origin not recorded".into(),
            UnitState::Tried => "tried".into(),
            UnitState::Planned => "planned".into(),
            UnitState::Other(status) => status.replace('-', " "),
        }
    }
}

/// A file's state (§2.3): its own, or its owning unit's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// 1 `? missing`: the facts record it, the tree does not have it.
    Missing,
    /// 2 `! changed since scan`.
    Changed,
    /// 3 `+ not scanned yet`.
    New,
    /// 4 `· header`.
    Header,
    /// Owned by a unit (index into the snapshot's units): the unit's state.
    Owned(usize),
    /// 5 `– no exported functions`.
    NoExports,
    /// 6 `○ not in the plan`.
    NotInPlan,
}

/// One function of a file, from the facts' symbol records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Its name.
    pub name: String,
    /// 1-based first line.
    pub line: u32,
    /// Named in its owning unit's `symbols`: it takes the unit's state.
    /// Otherwise it is internal (dim, no glyph).
    pub in_unit: bool,
}

/// One file of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Repo-relative path, `/`-separated (untrusted: display-filtered).
    pub path: String,
    /// Its state.
    pub state: FileState,
    /// The non-blocked unit whose `files` hold it, when one does.
    pub owner: Option<usize>,
    /// Its functions in span order.
    pub functions: Vec<FunctionInfo>,
}

/// One plan unit's derived facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitInfo {
    /// Its state.
    pub state: UnitState,
    /// Its crate is code the harness knows: its content hash equals a
    /// recorded attempt's candidate digest, or its file-set hash equals the
    /// stored verdict's `rust_crate` (what the oracle last judged). Only
    /// then is Re-check offered.
    pub known_code: bool,
}

/// Non-zero counts of the action states and the migrated share of a subtree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rollup {
    /// Files failing.
    pub failing: usize,
    /// Files needing attention.
    pub attention: usize,
    /// Files changed since the scan.
    pub changed: usize,
    /// Files not scanned yet.
    pub new: usize,
    /// Files missing.
    pub missing: usize,
    /// Files migrated.
    pub migrated: usize,
    /// Files verified with no recorded origin.
    pub origin_unknown: usize,
    /// Files that can be migrated (owned, or with exports but not planned).
    pub migratable: usize,
}

impl Rollup {
    /// `✗2 ⚠1 ✓3/11 (+1 ✓?)`: the non-zero counts; empty when all are zero.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        for (glyph, n) in [
            ("✗", self.failing),
            ("⚠", self.attention),
            ("!", self.changed),
            ("+", self.new),
            ("?", self.missing),
        ] {
            if n > 0 {
                parts.push(format!("{glyph}{n}"));
            }
        }
        if self.migratable > 0 {
            parts.push(format!("✓{}/{}", self.migrated, self.migratable));
        }
        if self.origin_unknown > 0 {
            parts.push(format!("(+{} ✓?)", self.origin_unknown));
        }
        parts.join(" ")
    }
}

/// The tree's facts: files sorted by path, and per-unit states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    /// Every file of the tree: the listed ones and the missing ones, by path.
    pub files: Vec<FileInfo>,
    /// Per snapshot unit (same index).
    pub units: Vec<UnitInfo>,
}

impl Files {
    /// The file at `path`.
    pub fn file(&self, path: &str) -> Option<&FileInfo> {
        self.files
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.files[i])
    }

    /// The rollup of the files under `dir` (`""`: every file).
    pub fn rollup(&self, dir: &str) -> Rollup {
        let mut r = Rollup::default();
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        for f in self.files.iter().filter(|f| f.path.starts_with(&prefix)) {
            match &f.state {
                FileState::Missing => r.missing += 1,
                FileState::Changed => r.changed += 1,
                FileState::New => r.new += 1,
                FileState::NotInPlan => r.migratable += 1,
                FileState::Owned(u) => {
                    r.migratable += 1;
                    match self.units.get(*u).map(|i| &i.state) {
                        Some(UnitState::Failing) => r.failing += 1,
                        Some(UnitState::Attention(_)) => r.attention += 1,
                        Some(UnitState::Migrated(_)) => r.migrated += 1,
                        Some(UnitState::OriginUnknown) => r.origin_unknown += 1,
                        _ => {}
                    }
                }
                FileState::Header | FileState::NoExports => {}
            }
        }
        r
    }
}

/// A unit's crate is code the harness knows (see [`UnitInfo::known_code`]).
pub fn known_code(unit: &UnitView) -> bool {
    let Some(digest) = unit.crate_digest.as_deref() else {
        return false;
    };
    let recorded = unit
        .attempts
        .iter()
        .any(|a| !a.record.candidate_digest.is_empty() && a.record.candidate_digest == digest);
    let judged = unit.report.verdict.state == VerdictState::Present
        && unit
            .verdict
            .as_ref()
            .is_some_and(|v| !v.inputs.rust_crate.is_empty())
        && !unit.report.verdict.stale.iter().any(|s| s == "rust-crate");
    recorded || judged
}

/// A unit's state (§2.3, first match wins).
pub fn unit_state(unit: &UnitView) -> UnitState {
    let r = &unit.report;
    let status = r.status.as_str();
    if status == "blocked" {
        return UnitState::Blocked;
    }
    if let Some(id) = &r.promotion_interrupted {
        return UnitState::Attention(Cause::PromotionInterrupted(id.clone()));
    }
    // The specific causes first: harness-core also calls a verified unit
    // whose verdict went stale a contradiction. The C changed since planning
    // only while the plan says so: once `plan` re-approved it, a verdict
    // still stale on its source needs a Re-check (review ENG-2).
    let present = r.verdict.state == VerdictState::Present;
    if !r.source_fresh {
        return UnitState::Attention(Cause::SourceChanged);
    }
    if unit.crate_digest.is_some() && !known_code(unit) {
        return UnitState::Attention(if present {
            Cause::ChangedOutside
        } else {
            Cause::NoEvidence
        });
    }
    if present && !r.verdict.stale.is_empty() {
        return UnitState::Attention(Cause::StaleVerdict);
    }
    if r.contradiction {
        return UnitState::Attention(Cause::Contradiction);
    }
    if present && r.verdict.green == Some(false) {
        return UnitState::Failing;
    }
    let done = matches!(status, "verified" | "merged");
    if done && r.fresh_green() {
        return match &unit.provenance {
            ProvenanceView::Pipeline(_) => UnitState::Migrated(Origin::Pipeline),
            ProvenanceView::Steered(_) => UnitState::Migrated(Origin::Steered),
            ProvenanceView::Human { .. } => UnitState::Migrated(Origin::Human),
            ProvenanceView::None | ProvenanceView::Ambiguous(_) => UnitState::OriginUnknown,
        };
    }
    if status == "pending" {
        return if unit.attempts.is_empty() {
            UnitState::Planned
        } else {
            UnitState::Tried
        };
    }
    UnitState::Other(status.to_string())
}

/// The states of every file, function and unit (see the module docs).
pub fn build(snapshot: &Snapshot, walk: &TreeWalk) -> Files {
    let units: Vec<UnitInfo> = snapshot
        .units
        .iter()
        .map(|u| UnitInfo {
            state: unit_state(u),
            known_code: known_code(u),
        })
        .collect();
    // file → its non-blocked owner (the first in plan order).
    let mut owners: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, u) in snapshot.units.iter().enumerate() {
        if u.report.status == "blocked" {
            continue;
        }
        for f in &u.unit.files {
            owners.entry(f.as_str()).or_insert(i);
        }
    }
    let facts = snapshot.facts.as_ref();
    let recorded: BTreeSet<&str> = facts
        .map(|f| f.files.iter().map(|r| r.path.as_str()).collect())
        .unwrap_or_default();
    let stale: BTreeSet<&str> = snapshot
        .facts_state
        .as_ref()
        .map(|s| s.stale_paths.iter().map(String::as_str).collect())
        .unwrap_or_default();
    // Functions per file, in span order.
    let mut functions: BTreeMap<&str, Vec<(u32, &str, bool)>> = BTreeMap::new();
    if let Some(facts) = facts {
        for s in facts.symbols.iter().filter(|s| s.kind == "function") {
            functions.entry(s.file.as_str()).or_default().push((
                s.span.0,
                s.name.as_str(),
                s.visibility == "public",
            ));
        }
    }
    let mut paths: BTreeSet<&str> = walk.listed.iter().map(String::as_str).collect();
    paths.extend(walk.absent.iter().map(String::as_str));
    let files = paths
        .into_iter()
        .map(|path| {
            let owner = owners.get(path).copied();
            let mut fns = functions.get(path).cloned().unwrap_or_default();
            fns.sort();
            // Before the one-row-per-name cut: a public definition in any
            // `#if` branch makes the file export (review NEW-11).
            let public = fns.iter().any(|(_, _, public)| *public);
            // One row per name: the scanner records each `#if` branch's
            // definition (review ENG-7); the first span stands for them.
            let mut named = BTreeSet::new();
            fns.retain(|(_, name, _)| named.insert(*name));
            let state = if walk.absent.contains(path) {
                FileState::Missing
            } else if stale.contains(path) {
                FileState::Changed
            } else if !recorded.contains(path) {
                FileState::New
            } else if let Some(u) = owner {
                // A file owned by a unit takes its owner's state — a header
                // in a unit's files too (review ENG-12).
                FileState::Owned(u)
            } else if path.ends_with(".h") {
                FileState::Header
            } else if !public {
                FileState::NoExports
            } else {
                FileState::NotInPlan
            };
            let unit_symbols: BTreeSet<&str> = owner
                .and_then(|u| snapshot.units.get(u))
                .map(|u| u.unit.symbols.iter().map(String::as_str).collect())
                .unwrap_or_default();
            FileInfo {
                path: path.to_string(),
                state,
                owner,
                functions: fns
                    .into_iter()
                    .map(|(line, name, _)| FunctionInfo {
                        name: name.to_string(),
                        line,
                        in_unit: unit_symbols.contains(name),
                    })
                    .collect(),
            }
        })
        .collect();
    Files { files, units }
}

/// A file state's glyph and word, given the tree's unit states.
pub fn file_label(files: &Files, state: &FileState) -> (&'static str, String) {
    match state {
        FileState::Missing => ("?", "missing".into()),
        FileState::Changed => ("!", "changed since scan".into()),
        FileState::New => ("+", "not scanned yet".into()),
        FileState::Header => ("·", "header".into()),
        FileState::NoExports => ("–", "no exported functions".into()),
        FileState::NotInPlan => ("○", "not in the plan".into()),
        FileState::Owned(u) => match files.units.get(*u) {
            Some(info) => (info.state.glyph(), info.state.word()),
            None => ("·", "owned".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::attempts::{attempt_dir, AttemptRecord};
    use harness_core::facts::{FileRecord, SymbolRecord};
    use harness_core::ledger::Ledger;
    use std::path::PathBuf;

    const CASE: &str = "targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib";
    const LIB_C: &str = "test_case/src/lib.c";
    const LIB_H: &str = "test_case/include/lib.h";

    /// A scratch copy of a committed target (build products left out),
    /// removed on drop.
    struct Copy(PathBuf);

    impl Copy {
        fn of(rel: &str, tag: &str) -> Copy {
            fn copy(src: &Path, dst: &Path) {
                std::fs::create_dir_all(dst).unwrap();
                for entry in std::fs::read_dir(src).unwrap() {
                    let entry = entry.unwrap();
                    let name = entry.file_name();
                    if name == "target" || name == "build" || name == ".lock" {
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
            let dst = std::env::temp_dir()
                .join(format!("harness-tui-files-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dst);
            copy(&repo.join(rel), &dst);
            Copy(dst.canonicalize().unwrap())
        }

        fn read(&self) -> (Snapshot, Files) {
            let read = crate::load::read(&self.0).unwrap_or_else(|e| panic!("{e}"));
            let files = build(&read.snapshot, &read.walk);
            (read.snapshot, files)
        }

        fn state(&self, path: &str) -> FileState {
            self.read()
                .1
                .file(path)
                .unwrap_or_else(|| panic!("{path} not in the tree"))
                .state
                .clone()
        }

        fn unit(&self, id: &str) -> UnitInfo {
            let (snap, files) = self.read();
            let i = snap.units.iter().position(|u| u.unit.id == id).unwrap();
            files.units[i].clone()
        }

        fn write(&self, rel: &str, text: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }

        fn edit(&self, rel: &str, f: impl FnOnce(String) -> String) {
            let p = self.0.join(rel);
            let text = std::fs::read_to_string(&p).unwrap();
            std::fs::write(p, f(text)).unwrap();
        }

        fn record(&self, unit: &str, id: &str, f: impl FnOnce(&mut AttemptRecord)) {
            let dir = attempt_dir(&Ledger::new(&self.0), unit, id);
            let mut r = AttemptRecord::load(&dir).unwrap();
            f(&mut r);
            r.store(&dir).unwrap();
        }

        /// Record `rel` in the facts as the scanner would (its current hash),
        /// with `symbols` as (name, visibility, first line).
        fn scanned(&self, rel: &str, symbols: &[(&str, &str, u32)]) {
            let path = Ledger::new(&self.0).facts_path();
            let mut facts = Facts::load(&path).unwrap();
            facts.files.push(FileRecord {
                path: rel.into(),
                hash: harness_core::hash::file_hash(&self.0.join(rel)).unwrap(),
                includes: vec![],
            });
            facts.files.sort_by(|a, b| a.path.cmp(&b.path));
            for (name, visibility, line) in symbols {
                facts.symbols.push(SymbolRecord {
                    name: (*name).into(),
                    kind: "function".into(),
                    file: rel.into(),
                    visibility: (*visibility).into(),
                    signature: format!("int {name}(void)"),
                    span: (*line, *line + 1),
                });
            }
            facts.store(&path).unwrap();
        }
    }

    impl Drop for Copy {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The tractor case: its unit is migrated by the pipeline; its file takes
    /// the unit's state; the header is a header; the exported function is
    /// in the unit, the static one internal.
    #[test]
    fn the_tractor_case_is_migrated() {
        let t = Copy::of(CASE, "tractor");
        let (_, files) = t.read();
        assert_eq!(files.units[0].state, UnitState::Migrated(Origin::Pipeline));
        assert!(files.units[0].known_code);
        let lib = files.file(LIB_C).unwrap();
        assert_eq!(lib.state, FileState::Owned(0));
        assert_eq!(file_label(&files, &lib.state), ("✓", "migrated".into()));
        let fns: Vec<(&str, bool)> = lib
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.in_unit))
            .collect();
        assert_eq!(
            fns,
            [
                ("test_case/src/lib.c::get_bits", false),
                ("read_scalefactors", true)
            ]
        );
        assert_eq!(files.file(LIB_H).unwrap().state, FileState::Header);
        assert_eq!(files.rollup("").text(), "✓1/1");
    }

    /// zopfli: u001 (the M0 layout; the source moved on) is verified with no
    /// recorded origin — and its crate is known through the verdict's
    /// `rust_crate`, so Re-check stays available; 10 units are planned; one
    /// unit owns 3 files; headers are headers.
    #[test]
    fn zopfli_reads_as_its_ledger_says() {
        let t = Copy::of("targets/zopfli", "zopfli");
        let (snap, files) = t.read();
        let u001 = snap
            .units
            .iter()
            .position(|u| u.unit.id == "u001-katajainen")
            .unwrap();
        assert_eq!(files.units[u001].state, UnitState::OriginUnknown);
        assert!(
            files.units[u001].known_code,
            "Re-check through the verdict's rust_crate"
        );
        let planned = files
            .units
            .iter()
            .filter(|u| u.state == UnitState::Planned)
            .count();
        assert_eq!(planned, 10);
        let three = snap
            .units
            .iter()
            .position(|u| u.unit.files.len() == 3)
            .expect("a 3-file unit");
        for f in &snap.units[three].unit.files {
            assert_eq!(files.file(f).unwrap().state, FileState::Owned(three), "{f}");
        }
        assert_eq!(
            files.file("src/zopfli/cache.h").unwrap().state,
            FileState::Header
        );
        assert_eq!(files.rollup("").text(), "✓0/13 (+1 ✓?)");
        assert_eq!(files.rollup("src/zopfli").text(), "✓0/13 (+1 ✓?)");
    }

    /// Rules 1–3: a deleted file is missing, an edited `.c` and an edited
    /// header are changed since the scan, a new file is not scanned yet. The
    /// conflict: a header edit also makes the unit's verdict stale — the
    /// file says changed, the unit needs attention (its C changed).
    #[test]
    fn missing_changed_and_new_files() {
        let t = Copy::of(CASE, "fresh");
        t.write("test_case/src/new.c", "int fresh(void) { return 1; }\n");
        assert_eq!(t.state("test_case/src/new.c"), FileState::New);
        t.edit(LIB_H, |s| format!("{s}\n/* edited */\n"));
        assert_eq!(t.state(LIB_H), FileState::Changed);
        assert_eq!(
            t.unit("u-lib").state,
            UnitState::Attention(Cause::SourceChanged)
        );
        t.edit(LIB_C, |s| format!("/* edited */\n{s}"));
        assert_eq!(t.state(LIB_C), FileState::Changed);
        std::fs::remove_file(t.0.join(LIB_H)).unwrap();
        assert_eq!(t.state(LIB_H), FileState::Missing);
        let (_, files) = t.read();
        assert_eq!(files.rollup("").text(), "!1 +1 ?1");
    }

    /// Rules 5 and 6: a scanned `.c` with no public function is never
    /// planned; one with a public function that no unit holds is not in the
    /// plan.
    #[test]
    fn static_only_and_unowned_files() {
        let t = Copy::of(CASE, "unowned");
        t.write(
            "test_case/src/helpers.c",
            "static int h(void) { return 0; }\n",
        );
        t.write("test_case/src/extra.c", "int extra(void) { return 0; }\n");
        t.scanned(
            "test_case/src/helpers.c",
            &[("test_case/src/helpers.c::h", "internal", 1)],
        );
        t.scanned("test_case/src/extra.c", &[("extra", "public", 1)]);
        let (_, files) = t.read();
        assert_eq!(
            files.file("test_case/src/helpers.c").unwrap().state,
            FileState::NoExports
        );
        assert_eq!(
            files.file("test_case/src/extra.c").unwrap().state,
            FileState::NotInPlan
        );
        assert_eq!(files.rollup("").text(), "✓1/2");
    }

    /// No facts: every listed file is not scanned yet and there are no
    /// units. No plan: a public `.c` is not in the plan.
    #[test]
    fn no_facts_and_no_plan() {
        let t = Copy::of(CASE, "noplan");
        std::fs::remove_file(t.0.join("migration/plan.toml")).unwrap();
        assert_eq!(t.state(LIB_C), FileState::NotInPlan);
        assert_eq!(t.state(LIB_H), FileState::Header);
        std::fs::remove_file(t.0.join("migration/facts.jsonl")).unwrap();
        let (snap, files) = t.read();
        assert!(snap.units.is_empty());
        assert_eq!(files.file(LIB_C).unwrap().state, FileState::New);
        assert_eq!(files.file(LIB_H).unwrap().state, FileState::New);
    }

    /// Rule 8: a red verdict on a unit `verify` demoted to in-progress is
    /// failing.
    #[test]
    fn a_red_verdict_is_failing() {
        let t = Copy::of(CASE, "red");
        let v = t.0.join("migration/units/u-lib/oracle-latest.json");
        let mut verdict = harness_core::verdict::Verdict::load(&v).unwrap();
        verdict.green = false;
        verdict.checks[0].passed = false;
        std::fs::write(&v, serde_json::to_vec_pretty(&verdict).unwrap()).unwrap();
        t.edit("migration/plan.toml", |s| {
            s.replace("status = \"verified\"", "status = \"in-progress\"")
        });
        assert_eq!(t.unit("u-lib").state, UnitState::Failing);
        assert_eq!(t.state(LIB_C), FileState::Owned(0));
        assert_eq!(t.read().1.rollup("").text(), "✗1 ✓0/1");
    }

    /// Rules 11 and 12: a pending unit with attempts is tried; without, it
    /// is planned.
    #[test]
    fn pending_units_are_tried_or_planned() {
        let t = Copy::of(CASE, "tried");
        std::fs::remove_file(t.0.join("migration/units/u-lib/oracle-latest.json")).unwrap();
        t.edit("migration/plan.toml", |s| {
            s.replace("status = \"verified\"", "status = \"pending\"")
        });
        std::fs::remove_dir_all(t.0.join("migration/units/u-lib/u_lib_rs")).unwrap();
        assert_eq!(t.unit("u-lib").state, UnitState::Tried);
        std::fs::remove_dir_all(t.0.join("migration/units/u-lib/attempts")).unwrap();
        assert_eq!(t.unit("u-lib").state, UnitState::Planned);
    }

    /// CHK-11: a blocked unit shows ⊘ on its row; a file it held that a new
    /// unit also holds belongs to the non-blocked one.
    #[test]
    fn a_blocked_units_file_belongs_to_the_non_blocked_owner() {
        let t = Copy::of(CASE, "blocked");
        t.edit("migration/plan.toml", |s| {
            let s = s.replace("status = \"verified\"", "status = \"blocked\"");
            format!(
                "{s}\n[[unit]]\nid = \"u-new\"\nstatus = \"pending\"\nfiles = [\"{LIB_C}\"]\n\
                 source_hash = \"blake3:00\"\nsymbols = [\"read_scalefactors\"]\ninterface = []\n\
                 depends_on = []\ntest_strategy = \"\"\ndone_criteria = \"\"\n"
            )
        });
        let (snap, files) = t.read();
        assert_eq!(files.units[0].state, UnitState::Blocked);
        let new = snap
            .units
            .iter()
            .position(|u| u.unit.id == "u-new")
            .unwrap();
        assert_eq!(files.file(LIB_C).unwrap().owner, Some(new));
    }

    /// Rule 10 and CHK-2: a merged unit whose crate no attempt bound to the
    /// current inputs produced is verified with no recorded origin — and not
    /// counted as migrated; its crate is still code the oracle judged.
    #[test]
    fn a_merged_unit_with_no_attributable_attempt_is_origin_unknown() {
        let t = Copy::of(CASE, "merged");
        t.edit("migration/plan.toml", |s| {
            s.replace("status = \"verified\"", "status = \"merged\"")
        });
        for id in ["a-13c941dfff95", "a-28d8ddc411f9", "a-d2e5513cdfa6"] {
            t.record("u-lib", id, |r| {
                r.candidate_digest = "blake3:elsewhere".into()
            });
        }
        let u = t.unit("u-lib");
        assert_eq!(u.state, UnitState::OriginUnknown);
        assert!(
            u.known_code,
            "judged by the oracle (the verdict's rust_crate)"
        );
        assert_eq!(t.read().1.rollup("").text(), "✓0/1 (+1 ✓?)");
    }

    /// Rule 7, SAFE-4: a crate edited outside the harness matches neither a
    /// recorded candidate nor what the oracle last judged — it needs
    /// attention, and it is not code the harness knows.
    #[test]
    fn a_crate_edited_outside_the_harness_needs_attention() {
        let t = Copy::of(CASE, "outside");
        t.edit("migration/units/u-lib/u_lib_rs/src/logic.rs", |s| {
            format!("{s}\n// by someone\n")
        });
        let u = t.unit("u-lib");
        assert_eq!(u.state, UnitState::Attention(Cause::ChangedOutside));
        assert!(!u.known_code);
        // Put back as it was: known again.
        t.edit("migration/units/u-lib/u_lib_rs/src/logic.rs", |s| {
            s.replace("\n// by someone\n", "")
        });
        assert!(t.unit("u-lib").known_code);
    }

    /// Review ENG-2: once `plan` re-approved a unit's changed C, a verdict
    /// still stale on its source asks for a Re-check, not a scan.
    #[test]
    fn a_reapproved_source_asks_for_a_recheck() {
        let t = Copy::of(CASE, "reapproved");
        t.edit(LIB_C, |s| format!("{s}/* edited */\n"));
        assert_eq!(
            t.unit("u-lib").state,
            UnitState::Attention(Cause::SourceChanged)
        );
        // What `scan` then `plan` do: the facts and the plan take the new
        // hashes; the verdict stays stale on its source.
        let path = Ledger::new(&t.0).facts_path();
        let mut facts = Facts::load(&path).unwrap();
        for f in facts.files.iter_mut() {
            f.hash = harness_core::hash::file_hash(&t.0.join(&f.path)).unwrap();
        }
        facts.store(&path).unwrap();
        let closure = facts.include_closure(&[LIB_C.to_string()]);
        let now = harness_core::hash::file_set_hash_on_disk(&t.0, &closure).unwrap();
        t.edit("migration/plan.toml", |s| {
            let start = s.find("source_hash = \"").unwrap() + "source_hash = \"".len();
            let end = start + s[start..].find('"').unwrap();
            format!("{}{now}{}", &s[..start], &s[end..])
        });
        assert_eq!(
            t.unit("u-lib").state,
            UnitState::Attention(Cause::StaleVerdict)
        );
    }

    /// Review ENG-7: a function defined in two `#if` branches is one row.
    #[test]
    fn a_function_defined_twice_is_one_row() {
        let t = Copy::of(CASE, "dupfn");
        t.write("test_case/src/dup.c", "static int g(void) { return 0; }\n");
        t.scanned(
            "test_case/src/dup.c",
            &[
                ("test_case/src/dup.c::g", "internal", 1),
                ("test_case/src/dup.c::g", "internal", 3),
            ],
        );
        let (_, files) = t.read();
        assert_eq!(
            files.file("test_case/src/dup.c").unwrap().functions.len(),
            1
        );
    }

    /// Review ENG-11: a missing verdict is not "changed outside the harness"
    /// — the cause says what is missing.
    #[test]
    fn a_missing_verdict_is_named_as_such() {
        let t = Copy::of("targets/zopfli", "noverdict");
        std::fs::remove_file(t.0.join("migration/units/u001-katajainen/oracle-latest.json"))
            .unwrap();
        assert_eq!(
            t.unit("u001-katajainen").state,
            UnitState::Attention(Cause::NoEvidence)
        );
    }

    /// Review ENG-12: a header in a unit's files takes its owner's state.
    #[test]
    fn an_owned_header_takes_its_owners_state() {
        let t = Copy::of(CASE, "ownedh");
        t.edit("migration/plan.toml", |s| {
            s.replace(
                "files = [\"test_case/src/lib.c\"]",
                "files = [\"test_case/src/lib.c\", \"test_case/include/lib.h\"]",
            )
        });
        assert_eq!(t.state(LIB_H), FileState::Owned(0));
    }

    /// Second fix pass, NEW-11: a public definition in any `#if` branch
    /// makes the file export, whichever branch comes first.
    #[test]
    fn a_later_public_branch_still_exports() {
        let t = Copy::of(CASE, "laterpublic");
        t.write("test_case/src/br.c", "static int f(void) { return 0; }\n");
        t.scanned(
            "test_case/src/br.c",
            &[("f", "internal", 1), ("f", "public", 3)],
        );
        assert_eq!(t.state("test_case/src/br.c"), FileState::NotInPlan);
    }

    /// The tree's limits bound the listing and say so; the facts' files
    /// beyond the limit are not called missing.
    #[test]
    fn more_than_the_limit_is_truncated_not_missing() {
        let t = Copy::of(CASE, "limit");
        let gen = t.0.join("test_case/a_gen");
        std::fs::create_dir_all(&gen).unwrap();
        for i in 0..20_001 {
            std::fs::File::create(gen.join(format!("f{i:05}.c"))).unwrap();
        }
        let read = crate::load::read(&t.0).unwrap();
        assert!(read.walk.truncated);
        assert_eq!(read.walk.listed.len(), 20_000);
        // lib.c sorts after a_gen/: beyond the limit, present — not missing.
        assert!(!read.walk.absent.contains(LIB_C));
        let files = build(&read.snapshot, &read.walk);
        assert!(files.file(LIB_C).is_none());
    }
}
