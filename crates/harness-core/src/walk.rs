//! The confined source walk: which files under a directory a frontend reads
//! and the cockpit lists (docs/COCKPIT-WRAPPER-DESIGN.md §2.1). Language
//! neutral — the caller names the extensions.
//!
//! Every entry is canonicalized only to check containment and cycles: one
//! that resolves outside the walked directory (a symlink out of it) or does
//! not resolve at all (a dangling link) is left out, and a directory already
//! walked (a symlink cycle) is not walked again. The paths returned are
//! joined under the directory as given, never canonical, in the walk's
//! order: each directory's entries sorted, depth first. A matching entry
//! that is not a regular file once followed (a FIFO, a device, a socket)
//! is never returned — reading it could block forever — but reported in
//! [`Walk::skipped`]. Errors are returned as data; the walk reads directory
//! entries and metadata only, never a file's contents.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Bounds on a walk. The default bounds nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Most files returned; the walk stops at the limit and says so.
    pub max_files: Option<usize>,
    /// Deepest directory walked (the walked directory is depth 0); deeper
    /// ones are not entered, and the walk says so.
    pub max_depth: Option<usize>,
}

/// Why a matching entry was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Not a regular file once followed (a FIFO, a device, a socket).
    NotRegular,
}

/// What a walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walk {
    /// The matching regular files, joined under the walked directory.
    pub files: Vec<PathBuf>,
    /// Matching entries left out, and why.
    pub skipped: Vec<(PathBuf, Skip)>,
    /// Entries that could not be read: the path and the error, in words.
    pub errors: Vec<(PathBuf, String)>,
    /// A limit cut the walk short: more files may exist than it returned.
    pub truncated: bool,
}

/// The `exts` that matches every regular file, whatever its name (the
/// features probe's mirror, docs/FEATURES-DESIGN.md §5.3).
pub const ALL_FILES: &[&str] = &["*"];

/// Walk `dir` for files whose extension is one of `exts` (without the dot;
/// [`ALL_FILES`] for every file), within `limits`. See the module docs.
pub fn confined(dir: &Path, exts: &[&str], limits: Limits) -> Walk {
    confined_except(dir, exts, limits, &[])
}

/// [`confined`], never entering the directories `prune` names (paths as
/// the walk joins them: under `dir` as given) — they count toward no limit
/// and report no error (the features mirror leaves out `migration/` and
/// `.git/`; review M6).
pub fn confined_except(dir: &Path, exts: &[&str], limits: Limits, prune: &[PathBuf]) -> Walk {
    let mut walk = Walk::default();
    let canon = match dir.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            walk.errors.push((dir.to_path_buf(), e.to_string()));
            return walk;
        }
    };
    let mut visited = BTreeSet::new();
    let ctx = Ctx {
        root: &canon,
        exts,
        limits,
        prune,
    };
    visit(dir, &ctx, 0, &mut visited, &mut walk);
    walk
}

/// What every level of a walk shares.
struct Ctx<'a> {
    root: &'a Path,
    exts: &'a [&'a str],
    limits: Limits,
    prune: &'a [PathBuf],
}

fn full(walk: &Walk, limits: Limits) -> bool {
    limits.max_files.is_some_and(|max| walk.files.len() >= max)
}

fn visit(
    dir: &Path,
    ctx: &Ctx<'_>,
    depth: usize,
    visited: &mut BTreeSet<PathBuf>,
    walk: &mut Walk,
) {
    let (root, exts, limits) = (ctx.root, ctx.exts, ctx.limits);
    let canon_dir = match dir.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            walk.errors.push((dir.to_path_buf(), e.to_string()));
            return;
        }
    };
    if !visited.insert(canon_dir) {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            walk.errors.push((dir.to_path_buf(), e.to_string()));
            return;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(e) => walk.errors.push((dir.to_path_buf(), e.to_string())),
        }
    }
    paths.sort();
    for path in paths {
        // Full, and more files found: the walk stops.
        if walk.truncated && full(walk, limits) {
            return;
        }
        // A dangling symlink, or one leading out of the walked directory,
        // is not part of it.
        let Ok(canon) = path.canonicalize() else {
            continue;
        };
        if !canon.starts_with(root) {
            continue;
        }
        // Followed, as the canonical path was.
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) => {
                walk.errors.push((path, e.to_string()));
                continue;
            }
        };
        if meta.is_dir() {
            if ctx.prune.contains(&path) {
                continue;
            }
            if limits.max_depth.is_some_and(|max| depth >= max) {
                // Not entered; its siblings still are.
                walk.truncated = true;
                continue;
            }
            visit(&path, ctx, depth + 1, visited, walk);
            continue;
        }
        let matches = exts.contains(&ALL_FILES[0])
            || path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e));
        if !matches {
            continue;
        }
        if !meta.is_file() {
            walk.skipped.push((path, Skip::NotRegular));
            continue;
        }
        if full(walk, limits) {
            walk.truncated = true;
            return;
        }
        walk.files.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let dir = std::env::temp_dir()
                .join(format!("harness-core-walk-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }

        fn file(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "int x;\n").unwrap();
            path
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rel(walk: &Walk, root: &Path) -> Vec<String> {
        walk.files
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .collect()
    }

    /// The path form matches the scanner's: joined under the directory as
    /// given (not canonical), each directory sorted, depth first.
    #[test]
    fn paths_are_joined_under_the_directory_in_walk_order() {
        let t = Tmp::new("order");
        for f in ["src/b.c", "src/a/z.h", "src/a.c", "src/c.txt", "src/a/y.c"] {
            t.file(f);
        }
        let dir = t.0.join("src");
        let walk = confined(&dir, &["c", "h"], Limits::default());
        assert_eq!(rel(&walk, &dir), ["a/y.c", "a/z.h", "a.c", "b.c"]);
        assert!(walk.files.iter().all(|p| p.starts_with(&dir)));
        assert_eq!(walk.errors, []);
        assert!(!walk.truncated);
    }

    /// A symlink out of the directory is left out; a cycle is walked once;
    /// a link inside is followed and returned at its own path.
    #[test]
    fn links_out_are_left_out_and_cycles_walked_once() {
        let t = Tmp::new("links");
        t.file("outside/secret.c");
        t.file("src/real/a.c");
        let src = t.0.join("src");
        std::os::unix::fs::symlink(t.0.join("outside"), src.join("out")).unwrap();
        std::os::unix::fs::symlink(t.0.join("outside/secret.c"), src.join("s.c")).unwrap();
        std::os::unix::fs::symlink(&src, src.join("real/loop")).unwrap();
        std::os::unix::fs::symlink(src.join("real/a.c"), src.join("inside.c")).unwrap();
        std::os::unix::fs::symlink(src.join("gone.c"), src.join("dangling.c")).unwrap();
        let walk = confined(&src, &["c"], Limits::default());
        assert_eq!(rel(&walk, &src), ["inside.c", "real/a.c"]);
        assert_eq!(walk.skipped, []);
        assert_eq!(walk.errors, []);
    }

    /// A non-regular file with a matching name (here a socket; a FIFO is
    /// covered by harness-scan's test — making one needs a subprocess, and a
    /// fork here would hold the ledger tests' lock files open) is skipped,
    /// never returned. A link to a device resolves outside the tree and is
    /// left out before its type is looked at.
    #[test]
    fn non_regular_files_are_skipped() {
        let t = Tmp::new("sock");
        t.file("src/ok.c");
        let src = t.0.join("src");
        let _socket = std::os::unix::net::UnixListener::bind(src.join("sock.c")).unwrap();
        std::os::unix::fs::symlink("/dev/zero", src.join("zero.c")).unwrap();
        let walk = confined(&src, &["c"], Limits::default());
        assert_eq!(rel(&walk, &src), ["ok.c"]);
        assert_eq!(walk.skipped, [(src.join("sock.c"), Skip::NotRegular)]);
    }

    /// The limits bound the listing and say so.
    #[test]
    fn the_limits_truncate_and_say_so() {
        let t = Tmp::new("limits");
        for i in 0..10 {
            t.file(&format!("src/f{i}.c"));
        }
        t.file("src/d1/d2/d3/deep.c");
        let src = t.0.join("src");
        let walk = confined(
            &src,
            &["c"],
            Limits {
                max_files: Some(4),
                max_depth: None,
            },
        );
        assert_eq!(walk.files.len(), 4);
        assert!(walk.truncated);
        let walk = confined(
            &src,
            &["c"],
            Limits {
                max_files: None,
                max_depth: Some(2),
            },
        );
        assert!(walk.truncated);
        assert!(!walk.files.iter().any(|p| p.ends_with("deep.c")));
        let walk = confined(&src, &["c"], Limits::default());
        assert_eq!(walk.files.len(), 11);
        assert!(!walk.truncated);
    }

    /// Review M6: a pruned directory is never entered — its files count
    /// toward no limit.
    #[test]
    fn pruned_directories_are_not_walked() {
        let t = Tmp::new("prune");
        t.file("main.c");
        for i in 0..10 {
            t.file(&format!("migration/units/u/target/f{i}.o"));
        }
        t.file(".git/objects/aa/x");
        let root = t.0.clone();
        let limits = Limits {
            max_files: Some(3),
            max_depth: None,
        };
        let walk = confined_except(
            &root,
            ALL_FILES,
            limits,
            &[root.join("migration"), root.join(".git")],
        );
        assert_eq!(rel(&walk, &root), ["main.c"]);
        assert!(!walk.truncated);
        assert!(confined(&root, ALL_FILES, limits).truncated);
    }

    /// Errors are data: an unreadable directory is reported, the rest walked.
    #[test]
    fn errors_are_returned_as_data() {
        use std::os::unix::fs::PermissionsExt;
        let t = Tmp::new("errors");
        t.file("src/ok.c");
        t.file("src/locked/x.c");
        let src = t.0.join("src");
        let locked = src.join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let walk = confined(&src, &["c"], Limits::default());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(rel(&walk, &src), ["ok.c"]);
        assert_eq!(walk.errors.len(), 1, "{:?}", walk.errors);
        assert_eq!(walk.errors[0].0, locked);
        let missing = confined(&t.0.join("nope"), &["c"], Limits::default());
        assert_eq!(missing.errors.len(), 1);
        assert!(missing.files.is_empty());
    }
}
