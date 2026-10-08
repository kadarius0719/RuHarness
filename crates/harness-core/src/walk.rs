//! The confined source walk: which files under a directory a frontend reads
//! and the cockpit lists (docs/COCKPIT-WRAPPER-DESIGN.md §2.1), and the
//! project map's walk (docs/PROJECT-MAP-DESIGN.md §3.1 step 1). Language
//! neutral — the caller names the extensions.
//!
//! The walk descends real folders only, never a link. Each file it returns
//! is recorded **once**, under its path with no link in it, joined under the
//! directory as given (never canonical), in the walk's order: each folder's
//! entries sorted, depth first.
//!
//! - **Pruned by name:** a folder whose name starts with a dot (`.git`,
//!   `.cache`), at any depth below the walked directory. **Pruned by path:**
//!   the folders the caller names (`migration/`), compared canonically. A
//!   pruned folder is not walked; a count-only pass gives the number of its
//!   matching files in [`Walk::skipped_folders`], and they count toward no
//!   limit.
//! - **A link inside the walked directory** is not followed: a link to a
//!   folder makes the link's path an alias of each file walked under the
//!   real folder; a link to a file makes the link's path an alias of that
//!   file ([`Walk::aliases`]).
//! - **Recorded in [`Walk::issues`], never walked or read:** a link into a
//!   pruned folder, a link that leaves the walked directory, a link that
//!   points nowhere, a matching entry that is not a regular file (a FIFO, a
//!   device, a socket — reading one could block forever), and an entry or a
//!   folder that cannot be read. A link out of the directory or into a
//!   pruned folder is recorded when it leads to a folder or its name (or its
//!   target's) matches; a dangling link always.
//!
//! Errors are returned as data; the walk reads directory entries and
//! metadata only, never a file's contents.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Bounds on a walk. The default bounds nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Most files returned — matching files only: a pruned folder's count,
    /// an alias or a file of another extension counts toward nothing. The
    /// walk stops at the limit and says so.
    pub max_files: Option<usize>,
    /// Deepest directory walked (the walked directory is depth 0); deeper
    /// ones are not entered, and the walk says so.
    pub max_depth: Option<usize>,
}

/// Why an entry is in [`Walk::issues`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// A link that resolves outside the walked directory.
    LeavesRoot,
    /// A link that resolves to nothing (its target is gone, or a loop of
    /// links).
    Dangling,
    /// A link into a pruned folder (by name or by path).
    IntoPruned,
    /// Not a regular file (a FIFO, a device, a socket).
    NotRegular,
    /// The entry or folder could not be read: the error, in words.
    Unreadable(String),
}

impl Why {
    /// The reason, in words.
    pub fn words(&self) -> String {
        match self {
            Why::LeavesRoot => "a link that leaves the walked folder".into(),
            Why::Dangling => "a link that points nowhere".into(),
            Why::IntoPruned => "a link into a skipped folder".into(),
            Why::NotRegular => "not a regular file".into(),
            Why::Unreadable(e) => format!("cannot be read: {e}"),
        }
    }
}

/// An entry the walk did not walk or return, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The entry, joined under the walked directory.
    pub path: PathBuf,
    /// Why.
    pub why: Why,
}

/// A second path of a returned file, through a link inside the walked
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Alias {
    /// The path through the link, joined under the walked directory.
    pub path: PathBuf,
    /// The file, as [`Walk::files`] holds it.
    pub file: PathBuf,
}

/// A pruned folder, and what the count-only pass found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFolder {
    /// The folder, joined under the walked directory.
    pub path: PathBuf,
    /// Its matching regular files, links not followed.
    pub files: usize,
    /// False when the count stopped short (a folder inside it could not be
    /// read, or the pass's own bounds were reached): `files` is then a
    /// lower bound.
    pub complete: bool,
}

/// What a walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walk {
    /// The matching regular files, joined under the walked directory, each
    /// once.
    pub files: Vec<PathBuf>,
    /// The files' second paths through links, sorted by path.
    pub aliases: Vec<Alias>,
    /// Entries not walked or returned, and why, in the walk's order.
    pub issues: Vec<Issue>,
    /// The pruned folders the walk met, in the walk's order.
    pub skipped_folders: Vec<SkippedFolder>,
    /// A limit cut the walk short: more files may exist than it returned.
    pub truncated: bool,
}

impl Walk {
    /// The first entry that could not be read, with the error in words —
    /// for callers to which an unreadable folder is fatal.
    pub fn first_unreadable(&self) -> Option<(&Path, &str)> {
        self.issues.iter().find_map(|i| match &i.why {
            Why::Unreadable(e) => Some((i.path.as_path(), e.as_str())),
            _ => None,
        })
    }

    /// The matching entries left out because they are not regular files.
    pub fn not_regular(&self) -> impl Iterator<Item = &Path> {
        self.issues
            .iter()
            .filter(|i| i.why == Why::NotRegular)
            .map(|i| i.path.as_path())
    }
}

/// The `exts` that matches every regular file, whatever its name (the
/// features probe's mirror, docs/FEATURES-DESIGN.md §5.3).
pub const ALL_FILES: &[&str] = &["*"];

/// Entries the count-only pass looks at in one pruned folder, at most.
const COUNT_MAX_ENTRIES: usize = 200_000;

/// Deepest folder the count-only pass enters below a pruned folder.
const COUNT_MAX_DEPTH: usize = 64;

/// `ELOOP`, too many levels of links: a link loop points nowhere.
#[cfg(target_os = "linux")]
const LOOP_ERRNO: i32 = 40;
/// `ELOOP`, too many levels of links: a link loop points nowhere.
#[cfg(not(target_os = "linux"))]
const LOOP_ERRNO: i32 = 62;

/// Walk `dir` for files whose extension is one of `exts` (without the dot;
/// [`ALL_FILES`] for every file), within `limits`. See the module docs.
pub fn confined(dir: &Path, exts: &[&str], limits: Limits) -> Walk {
    confined_except(dir, exts, limits, &[])
}

/// [`confined`], also pruning the folders `prune` names, compared by
/// canonical path, so a link into one is an issue, never walked (the
/// features mirror and the program digest leave out `migration/`).
pub fn confined_except(dir: &Path, exts: &[&str], limits: Limits, prune: &[PathBuf]) -> Walk {
    let canon = match dir.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            let mut walk = Walk::default();
            issue(&mut walk, dir.to_path_buf(), Why::Unreadable(e.to_string()));
            return walk;
        }
    };
    let prune: Vec<PathBuf> = prune
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .collect();
    let ctx = Ctx {
        root: &canon,
        exts,
        limits,
        prune: &prune,
    };
    let mut st = State::default();
    visit(dir, &canon, &ctx, 0, &mut st);
    let State {
        mut walk,
        canon_files,
        links,
    } = st;
    for (link, target, is_dir) in links {
        if is_dir {
            // Every file walked under the real folder, through the link.
            for (canon, file) in canon_files
                .range(target.clone()..)
                .take_while(|(c, _)| c.starts_with(&target))
            {
                if let Ok(rest) = canon.strip_prefix(&target) {
                    walk.aliases.push(Alias {
                        path: link.join(rest),
                        file: file.clone(),
                    });
                }
            }
        } else if let Some(file) = canon_files.get(&target) {
            walk.aliases.push(Alias {
                path: link,
                file: file.clone(),
            });
        }
    }
    walk.aliases.sort();
    walk
}

/// What every level of a walk shares.
struct Ctx<'a> {
    /// The walked directory, canonical.
    root: &'a Path,
    exts: &'a [&'a str],
    limits: Limits,
    /// The pruned folders, canonical.
    prune: &'a [PathBuf],
}

impl Ctx<'_> {
    fn matches(&self, path: &Path) -> bool {
        self.exts.contains(&ALL_FILES[0])
            || path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| self.exts.contains(&e))
    }

    /// `canon`, inside the root, is a pruned folder or lies in one: a
    /// pruned path holds it, or one of its folders below the root has a
    /// dotted name. `is_dir` says whether `canon`'s last part is a folder.
    fn pruned(&self, canon: &Path, is_dir: bool) -> bool {
        if self.prune.iter().any(|p| canon.starts_with(p)) {
            return true;
        }
        let Ok(rel) = canon.strip_prefix(self.root) else {
            return false;
        };
        let parts: Vec<&OsStr> = rel.iter().collect();
        let folders = if is_dir {
            &parts[..]
        } else {
            &parts[..parts.len().saturating_sub(1)]
        };
        folders.iter().any(|p| dotted(p))
    }
}

/// A folder name the walk prunes: it starts with a dot.
fn dotted(name: &OsStr) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

/// The walk under way.
#[derive(Default)]
struct State {
    walk: Walk,
    /// Each returned file's canonical path, and the path returned.
    canon_files: BTreeMap<PathBuf, PathBuf>,
    /// Each link inside the root: its path, its canonical target, and
    /// whether the target is a folder.
    links: Vec<(PathBuf, PathBuf, bool)>,
}

fn full(walk: &Walk, limits: Limits) -> bool {
    limits.max_files.is_some_and(|max| walk.files.len() >= max)
}

fn issue(walk: &mut Walk, path: PathBuf, why: Why) {
    walk.issues.push(Issue { path, why });
}

/// Walk the real folder `dir`, whose canonical path is `canon_dir` (exact:
/// the walk never descends a link, so no part of it is one below the root).
fn visit(dir: &Path, canon_dir: &Path, ctx: &Ctx<'_>, depth: usize, st: &mut State) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            issue(
                &mut st.walk,
                dir.to_path_buf(),
                Why::Unreadable(e.to_string()),
            );
            return;
        }
    };
    let mut found = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => found.push((entry.path(), entry.file_name(), entry.file_type())),
            Err(e) => issue(
                &mut st.walk,
                dir.to_path_buf(),
                Why::Unreadable(e.to_string()),
            ),
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, name, file_type) in found {
        // Full, and more files found: the walk stops.
        if st.walk.truncated && full(&st.walk, ctx.limits) {
            return;
        }
        // The entry itself, not followed.
        let file_type = match file_type {
            Ok(t) => t,
            Err(e) => {
                issue(&mut st.walk, path, Why::Unreadable(e.to_string()));
                continue;
            }
        };
        if file_type.is_symlink() {
            link(path, ctx, st);
            continue;
        }
        let canon = canon_dir.join(&name);
        if file_type.is_dir() {
            if dotted(&name) || ctx.prune.contains(&canon) {
                skip_folder(path, ctx, st);
                continue;
            }
            if ctx.limits.max_depth.is_some_and(|max| depth >= max) {
                // Not entered; its siblings still are.
                st.walk.truncated = true;
                continue;
            }
            visit(&path, &canon, ctx, depth + 1, st);
            continue;
        }
        if !ctx.matches(&path) {
            continue;
        }
        if !file_type.is_file() {
            issue(&mut st.walk, path, Why::NotRegular);
            continue;
        }
        if full(&st.walk, ctx.limits) {
            st.walk.truncated = true;
            return;
        }
        st.canon_files.insert(canon, path.clone());
        st.walk.files.push(path);
    }
}

/// A link the walk met: an issue, or kept to become an alias once the walk
/// is done.
fn link(path: PathBuf, ctx: &Ctx<'_>, st: &mut State) {
    // Followed, as the canonical path is.
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) => {
            let why = if e.kind() == std::io::ErrorKind::NotFound
                || e.raw_os_error() == Some(LOOP_ERRNO)
            {
                Why::Dangling
            } else {
                Why::Unreadable(e.to_string())
            };
            issue(&mut st.walk, path, why);
            return;
        }
    };
    let target = match path.canonicalize() {
        Ok(t) => t,
        Err(e) => {
            issue(&mut st.walk, path, Why::Unreadable(e.to_string()));
            return;
        }
    };
    let is_dir = meta.is_dir();
    let counts = is_dir || ctx.matches(&path) || ctx.matches(&target);
    if !target.starts_with(ctx.root) {
        if counts {
            issue(&mut st.walk, path, Why::LeavesRoot);
        }
        return;
    }
    // A dotted name on a link counts nothing through it: the target decides
    // (a link into a pruned folder is an issue, one to a walked folder an
    // alias), so `.alias -> src` never lists `src`'s files as skipped and
    // `.alias -> .` never counts the whole root. Only real dot-folders are
    // counted.
    if ctx.pruned(&target, is_dir) {
        if counts {
            issue(&mut st.walk, path, Why::IntoPruned);
        }
        return;
    }
    if is_dir || meta.is_file() {
        st.links.push((path, target, is_dir));
    }
    // A link to a FIFO or a device inside the root: the entry itself is
    // recorded where the walk meets it.
}

/// Record the pruned folder `path` with its count.
fn skip_folder(path: PathBuf, ctx: &Ctx<'_>, st: &mut State) {
    let (files, complete) = count(&path, ctx);
    st.walk.skipped_folders.push(SkippedFolder {
        path,
        files,
        complete,
    });
}

/// The count-only pass over a pruned folder: its matching regular files,
/// links not followed, within the pass's own bounds; and whether the count
/// is complete.
fn count(dir: &Path, ctx: &Ctx<'_>) -> (usize, bool) {
    let mut files = 0;
    let mut seen = 0;
    let mut complete = true;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            complete = false;
            continue;
        };
        for entry in entries {
            seen += 1;
            if seen > COUNT_MAX_ENTRIES {
                return (files, false);
            }
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                complete = false;
                continue;
            };
            if file_type.is_dir() {
                if depth < COUNT_MAX_DEPTH {
                    stack.push((entry.path(), depth + 1));
                } else {
                    complete = false;
                }
            } else if file_type.is_file() && ctx.matches(&entry.path()) {
                files += 1;
            }
        }
    }
    (files, complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

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

    /// The issues as `(relative path, why)`.
    fn issues(walk: &Walk, root: &Path) -> Vec<(String, Why)> {
        walk.issues
            .iter()
            .map(|i| {
                (
                    i.path.strip_prefix(root).unwrap().display().to_string(),
                    i.why.clone(),
                )
            })
            .collect()
    }

    /// The aliases as `(alias, file)`, relative.
    fn aliases(walk: &Walk, root: &Path) -> Vec<(String, String)> {
        walk.aliases
            .iter()
            .map(|a| {
                (
                    a.path.strip_prefix(root).unwrap().display().to_string(),
                    a.file.strip_prefix(root).unwrap().display().to_string(),
                )
            })
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
        assert_eq!(walk.issues, []);
        assert_eq!(walk.aliases, []);
        assert!(!walk.truncated);
    }

    /// A folder whose name starts with a dot is pruned at any depth, never
    /// walked, and listed with its count of matching files.
    #[test]
    fn a_dot_folder_at_depth_two_is_skipped_and_counted() {
        let t = Tmp::new("dot");
        t.file("src/a/b.c");
        t.file("src/a/.cache/x.c");
        t.file("src/a/.cache/deep/y.h");
        t.file("src/a/.cache/notes.txt");
        t.file("src/.git/objects/z.c");
        let dir = t.0.join("src");
        let walk = confined(&dir, &["c", "h"], Limits::default());
        assert_eq!(rel(&walk, &dir), ["a/b.c"]);
        assert_eq!(
            walk.skipped_folders,
            [
                SkippedFolder {
                    path: dir.join(".git"),
                    files: 1,
                    complete: true,
                },
                SkippedFolder {
                    path: dir.join("a/.cache"),
                    files: 2,
                    complete: true,
                },
            ]
        );
        assert_eq!(walk.issues, []);
    }

    /// A link into a pruned folder — by path or by name, to the folder or
    /// to a file in it — is an issue, never walked.
    #[test]
    fn a_link_into_a_pruned_folder_is_an_issue() {
        let t = Tmp::new("intopruned");
        t.file("main.c");
        t.file("migration/m.c");
        t.file(".hidden/h.c");
        let root = t.0.clone();
        symlink(root.join("migration"), root.join("m")).unwrap();
        symlink(root.join("migration/m.c"), root.join("mm.c")).unwrap();
        symlink(root.join(".hidden"), root.join("h")).unwrap();
        symlink(root.join(".hidden/h.c"), root.join("hh.c")).unwrap();
        let walk = confined_except(&root, &["c"], Limits::default(), &[root.join("migration")]);
        assert_eq!(rel(&walk, &root), ["main.c"]);
        assert_eq!(
            issues(&walk, &root),
            [
                ("h".to_string(), Why::IntoPruned),
                ("hh.c".to_string(), Why::IntoPruned),
                ("m".to_string(), Why::IntoPruned),
                ("mm.c".to_string(), Why::IntoPruned),
            ]
        );
        assert_eq!(walk.aliases, []);
    }

    /// The readers review, finding 8: a dot-named link counts nothing
    /// through the link. To a walked folder it is an alias (never "skipped
    /// files" that are walked); to the root itself, no count of the whole
    /// root; to a real dot-folder, a link into a pruned folder. Only a real
    /// dot-folder is counted.
    #[test]
    fn a_dot_named_link_is_judged_by_its_target_and_counts_nothing() {
        let t = Tmp::new("dotlink");
        t.file("src/x.c");
        t.file(".cache/c.c");
        let root = t.0.clone();
        symlink(root.join("src"), root.join(".alias")).unwrap();
        symlink(&root, root.join(".self")).unwrap();
        symlink(root.join(".cache"), root.join(".tocache")).unwrap();
        let walk = confined(&root, &["c"], Limits::default());
        assert_eq!(rel(&walk, &root), ["src/x.c"]);
        assert_eq!(
            walk.skipped_folders,
            [SkippedFolder {
                path: root.join(".cache"),
                files: 1,
                complete: true,
            }],
            "only the real dot-folder is counted"
        );
        assert_eq!(
            issues(&walk, &root),
            [(".tocache".to_string(), Why::IntoPruned)]
        );
        let aliases = aliases(&walk, &root);
        assert!(
            aliases.contains(&(".alias/x.c".to_string(), "src/x.c".to_string())),
            "{aliases:?}"
        );
        assert!(
            aliases.contains(&(".self/src/x.c".to_string(), "src/x.c".to_string())),
            "{aliases:?}"
        );
    }

    /// A link to a folder inside the root is not descended: the real folder
    /// is walked — whichever path sorts first — and the link's path is an
    /// alias of each of its files. A cycle ends.
    #[test]
    fn a_folder_link_is_an_alias_and_never_hides_the_real_folder() {
        let t = Tmp::new("dirlink");
        t.file("src/x.c");
        t.file("src/sub/y.c");
        let root = t.0.clone();
        symlink(root.join("src"), root.join("aaa")).unwrap();
        symlink(&root, root.join("src/loop")).unwrap();
        let walk = confined(&root, &["c"], Limits::default());
        assert_eq!(rel(&walk, &root), ["src/sub/y.c", "src/x.c"]);
        assert_eq!(
            aliases(&walk, &root),
            [
                ("aaa/sub/y.c".to_string(), "src/sub/y.c".to_string()),
                ("aaa/x.c".to_string(), "src/x.c".to_string()),
                (
                    "src/loop/src/sub/y.c".to_string(),
                    "src/sub/y.c".to_string()
                ),
                ("src/loop/src/x.c".to_string(), "src/x.c".to_string()),
            ]
        );
        assert_eq!(walk.issues, []);
    }

    /// A link to a file inside the root: the file once, under its path with
    /// no link in it, and the link's path its alias.
    #[test]
    fn a_file_link_is_an_alias_of_the_file() {
        let t = Tmp::new("filelink");
        t.file("src/x.c");
        let root = t.0.clone();
        symlink(root.join("src/x.c"), root.join("b.c")).unwrap();
        let walk = confined(&root, &["c"], Limits::default());
        assert_eq!(rel(&walk, &root), ["src/x.c"]);
        assert_eq!(
            aliases(&walk, &root),
            [("b.c".to_string(), "src/x.c".to_string())]
        );
    }

    /// A link out of the root (to a folder or a matching file) and a
    /// dangling link are issues; a link out to a file of another extension
    /// is no concern of this walk.
    #[test]
    fn links_out_and_dangling_links_are_issues() {
        let t = Tmp::new("links");
        t.file("outside/secret.c");
        t.file("outside/notes.txt");
        t.file("src/real/a.c");
        let src = t.0.join("src");
        symlink(t.0.join("outside"), src.join("out")).unwrap();
        symlink(t.0.join("outside/secret.c"), src.join("s.c")).unwrap();
        symlink(t.0.join("outside/notes.txt"), src.join("notes")).unwrap();
        symlink(src.join("gone.c"), src.join("dangling.c")).unwrap();
        symlink(src.join("self"), src.join("self")).unwrap();
        let walk = confined(&src, &["c"], Limits::default());
        assert_eq!(rel(&walk, &src), ["real/a.c"]);
        assert_eq!(
            issues(&walk, &src),
            [
                ("dangling.c".to_string(), Why::Dangling),
                ("out".to_string(), Why::LeavesRoot),
                ("s.c".to_string(), Why::LeavesRoot),
                ("self".to_string(), Why::Dangling),
            ]
        );
    }

    /// A non-regular file with a matching name (here a socket; a FIFO is
    /// covered by harness-scan's test — making one needs a subprocess, and a
    /// fork here would hold the ledger tests' lock files open) is an issue,
    /// never returned. A link to a device leaves the tree.
    #[test]
    fn non_regular_files_are_issues() {
        let t = Tmp::new("sock");
        t.file("src/ok.c");
        let src = t.0.join("src");
        let _socket = std::os::unix::net::UnixListener::bind(src.join("sock.c")).unwrap();
        symlink("/dev/zero", src.join("zero.c")).unwrap();
        let walk = confined(&src, &["c"], Limits::default());
        assert_eq!(rel(&walk, &src), ["ok.c"]);
        assert_eq!(
            issues(&walk, &src),
            [
                ("sock.c".to_string(), Why::NotRegular),
                ("zero.c".to_string(), Why::LeavesRoot),
            ]
        );
        assert_eq!(walk.not_regular().collect::<Vec<_>>(), [src.join("sock.c")]);
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

    /// The file cap counts matching files only: neither files of another
    /// extension, nor a pruned folder's files (counted apart), nor aliases.
    #[test]
    fn the_cap_counts_only_matched_files() {
        let t = Tmp::new("cap");
        t.file("a.c");
        t.file("b.c");
        for i in 0..10 {
            t.file(&format!("doc{i}.txt"));
            t.file(&format!(".cache/c{i}.c"));
        }
        let root = t.0.clone();
        symlink(root.join("a.c"), root.join("l1.c")).unwrap();
        symlink(root.join("a.c"), root.join("l2.c")).unwrap();
        let limits = Limits {
            max_files: Some(2),
            max_depth: None,
        };
        let walk = confined(&root, &["c"], limits);
        assert_eq!(rel(&walk, &root), ["a.c", "b.c"]);
        assert!(!walk.truncated);
        assert_eq!(walk.skipped_folders.len(), 1);
        assert_eq!(walk.skipped_folders[0].files, 10);
        assert_eq!(walk.aliases.len(), 2);
    }

    /// A folder pruned by path is never entered — its files count toward no
    /// limit — and is listed with its count.
    #[test]
    fn pruned_directories_are_not_walked() {
        let t = Tmp::new("prune");
        t.file("main.c");
        for i in 0..10 {
            t.file(&format!("migration/units/u/target/f{i}.o"));
        }
        let root = t.0.clone();
        let limits = Limits {
            max_files: Some(3),
            max_depth: None,
        };
        let walk = confined_except(&root, ALL_FILES, limits, &[root.join("migration")]);
        assert_eq!(rel(&walk, &root), ["main.c"]);
        assert!(!walk.truncated);
        assert_eq!(
            walk.skipped_folders,
            [SkippedFolder {
                path: root.join("migration"),
                files: 10,
                complete: true,
            }]
        );
        assert!(confined(&root, ALL_FILES, limits).truncated);
    }

    /// Errors are data: an unreadable directory is an issue, the rest
    /// walked.
    #[test]
    fn an_unreadable_folder_is_an_issue() {
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
        assert_eq!(walk.issues.len(), 1, "{:?}", walk.issues);
        assert_eq!(walk.issues[0].path, locked);
        assert!(matches!(walk.issues[0].why, Why::Unreadable(_)));
        assert_eq!(
            walk.first_unreadable().map(|(p, _)| p),
            Some(locked.as_path())
        );
        let missing = confined(&t.0.join("nope"), &["c"], Limits::default());
        assert_eq!(missing.issues.len(), 1);
        assert!(missing.first_unreadable().is_some());
        assert!(missing.files.is_empty());
    }
}
