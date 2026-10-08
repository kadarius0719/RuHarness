//! The project map's per-file facts over one folder (docs/PROJECT-MAP-DESIGN.md
//! §3.1 steps 1–5, §5 step a): the walk, the scanner's facts, each file's
//! include folders, and one compile per `.c` under the map's sandbox profile
//! whose object gives the symbols the file defines and needs.
//!
//! Step (a) maps one folder (a target's `source_dir`, as `harness scan`
//! reads it) with no configuration: the compile takes the judge's base
//! flags and the file's include folders only. Programs, closures,
//! duplicates, libraries, the configuration and the map file are step (b).
//!
//! What a compile read is known from the compiler's own dependency list
//! (`-MD -MF`): a file inside the root the walk did not record is an
//! `included_other`; anything outside the root and the toolchain's own
//! folders makes the file `outside_includes`, and its symbol names are then
//! counted, never kept (a readable file's bytes can become a symbol name).
//! Every object and dependency list is deleted as soon as it is read; the
//! fresh folder they live in goes with the run.
//!
//! Every string kept from the project (a path, a header name, a symbol name)
//! is raw: the caller filters it for a terminal.

use crate::exec::{ChildEnd, Runner};
use crate::objsyms;
use crate::sandbox::HostDirs;
use crate::scrub::Scrubber;
use harness_core::error::Error;
use harness_core::walk;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Most `.c`/`.h` files walked (§3.10).
pub const MAX_FILES: usize = 20_000;
/// Deepest folder walked below the mapped folder (§3.10).
pub const MAX_DEPTH: usize = 32;
/// A source file larger than this is recorded `too_large`: neither parsed
/// nor compiled, still hashed (§3.1 step 1).
pub const MAX_SOURCE_BYTES: u64 = 8 << 20;
/// An object larger than this is not read (§3.1 step 5).
pub const MAX_OBJECT_BYTES: u64 = 64 << 20;
/// The wall-clock limit of one compile (§3.10).
pub const COMPILE_TIMEOUT_SECS: u64 = 120;
/// The judge's own base flags, first on every map compile (§3.1 step 5).
pub const MAP_CFLAGS: [&str; 2] = ["-O2", crate::FP_CONTRACT_OFF];

/// A walked file's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A `.c` file: parsed and compiled.
    C,
    /// A `.h` file: parsed, never compiled on its own.
    H,
}

impl FileKind {
    /// `c` or `h`.
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::C => "c",
            FileKind::H => "h",
        }
    }
}

/// Why a `.c` did not compile: a closed set, never the compiler's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// An include was not found (the header's name is in [`Compiled::Failed`]).
    MissingHeader,
    /// The compiler reported an error at a place in a file.
    Syntax,
    /// Anything else: a timeout, a crash, an object too large or unreadable.
    Other,
}

impl Reason {
    /// `missing-header`, `syntax` or `other`.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::MissingHeader => "missing-header",
            Reason::Syntax => "syntax",
            Reason::Other => "other",
        }
    }
}

/// How a `.c` file's compile ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compiled {
    /// An object was made and read.
    Ok,
    /// No object was read.
    Failed {
        /// The closed reason.
        reason: Reason,
        /// The missing header's name as the include wrote it
        /// ([`Reason::MissingHeader`] only).
        header: Option<String>,
        /// A fixed word for [`Reason::Other`]: `timeout`, `output-overflow`,
        /// `too-large-object`, `unreadable-object`.
        detail: Option<&'static str>,
        /// Where in the project the compiler stopped: `<relative path>:<line>`.
        at: Option<String>,
    },
}

/// An include the map would add a folder to reach while another project
/// folder or the system also holds the name: every candidate, none picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    /// The include's name as written.
    pub header: String,
    /// Each project candidate's path relative to the root, sorted, then
    /// `system` when the toolchain's folders hold the name.
    pub candidates: Vec<String>,
    /// The candidate the compile actually read (from its dependency list),
    /// `system` for the system's; `None` when it read none of them (a `.h`,
    /// or a compile that stopped first).
    pub used: Option<String>,
}

/// An external symbol a file's object defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedSymbol {
    /// The C name (a `$`-suffixed libc name by its base name).
    pub name: String,
    /// `function`, `data`, `read-only`, `bss` or `common`.
    pub kind: &'static str,
    /// A weak definition.
    pub weak: bool,
}

/// An external symbol a file's object needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeededSymbol {
    /// The C name (a `$`-suffixed libc name by its base name).
    pub name: String,
    /// A weak reference.
    pub weak: bool,
}

/// The facts of one walked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// Relative to the project root, `/`-separated.
    pub path: String,
    /// Its other paths through links inside the mapped folder.
    pub aliases: Vec<String>,
    /// `.c` or `.h`.
    pub kind: FileKind,
    /// Its size.
    pub bytes: u64,
    /// Its blake3 hash (`blake3:<hex>`), read by streaming.
    pub blake3: String,
    /// The scanner read it.
    pub parsed: bool,
    /// Over [`MAX_SOURCE_BYTES`]: neither parsed nor compiled.
    pub too_large: bool,
    /// Not UTF-8 (parsed from its bytes all the same).
    pub not_utf8: bool,
    /// Functions the scanner found defined.
    pub functions: usize,
    /// The project files its includes reach directly, relative to the root,
    /// sorted.
    pub includes: Vec<String>,
    /// The folders its includes (and its headers' includes, to closure)
    /// need, relative to the root (`.` for the root), in the order first
    /// needed: each is passed to its compile as `-I`.
    pub include_dirs: Vec<String>,
    /// Its ambiguous includes and its headers', sorted by name.
    pub ambiguous: Vec<Ambiguous>,
    /// A `.c`'s compile; `None` for a `.h` or a `.c` too large to read.
    pub compiled: Option<Compiled>,
    /// The compile read something outside the root and the toolchain's
    /// folders: its symbol names are withheld.
    pub outside_includes: bool,
    /// Files inside the root the compile read that the walk did not record
    /// (a `.inc`, a `.def`, a header outside the mapped folder), sorted.
    pub included_other: Vec<String>,
    /// External symbols defined, sorted by name.
    pub defined: Vec<DefinedSymbol>,
    /// External symbols needed, sorted by name.
    pub needed: Vec<NeededSymbol>,
    /// Symbol names not shaped like a C identifier (an `asm` label can hold
    /// any text): counted, never kept.
    pub odd_names: usize,
    /// Identifier-shaped names withheld because of `outside_includes`.
    pub withheld_names: usize,
    /// The compiler's first error line, machine paths scrubbed: for the
    /// terminal once, never to be stored.
    pub message: Option<String>,
}

/// The toolchain the compiles ran under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// The first line of `cc --version`.
    pub cc: String,
    /// `cc -dumpmachine`.
    pub target: String,
    /// The base flags of every compile ([`MAP_CFLAGS`]).
    pub cflags: Vec<String>,
    /// The compiler's own `#include <...>` search folders, from `cc -E -v`,
    /// a framework folder's ` (framework directory)` dropped, in order.
    pub system_include_dirs: Vec<PathBuf>,
}

/// An entry the walk did not walk, and why (in words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkIssue {
    /// Relative to the root when inside it, else as found.
    pub path: String,
    /// Why, in words.
    pub why: String,
}

/// A folder the walk skipped (a dot-folder, `migration/`) and its count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFolder {
    /// Relative to the root.
    pub path: String,
    /// Its `.c`/`.h` files.
    pub files: usize,
    /// False when `files` is a lower bound.
    pub complete: bool,
}

/// A cap the run reached (§3.10): what follows it was not visited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitHit {
    /// `files` or `depth`.
    pub limit: &'static str,
    /// The cap, in words.
    pub at: String,
}

/// What mapping one folder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMap {
    /// The canonical project root.
    pub root: PathBuf,
    /// The mapped folder, relative to the root (`.` for the root).
    pub folder: String,
    /// The toolchain.
    pub toolchain: Toolchain,
    /// Every walked `.c` and `.h`, sorted by path.
    pub files: Vec<FileFacts>,
    /// The walk's issues, in its order.
    pub walk_issues: Vec<WalkIssue>,
    /// The folders it skipped.
    pub skipped_folders: Vec<SkippedFolder>,
    /// The caps reached.
    pub limits_hit: Vec<LimitHit>,
}

/// Refuse a root that is the home folder, holds it, or holds the cargo or
/// rustup home (§3.9): one sentence saying what to do instead.
pub fn refuse_root(root: &Path) -> Result<(), Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    let host = HostDirs::from_env()?;
    let refuse = |what: &str| {
        let is = if host.home == root && what == "your home folder" {
            "is"
        } else {
            "holds"
        };
        Err(Error::Invariant(format!(
            "{} {is} {what}, which the map never reads: point --target at the project's own \
             folder instead",
            root.display()
        )))
    };
    if host.home.starts_with(&root) {
        return refuse("your home folder");
    }
    for (home, what) in [
        (&host.cargo_home, "the cargo home"),
        (&host.rustup_home, "the rustup home"),
    ] {
        if home.as_ref().is_some_and(|h| h.starts_with(&root)) {
            return refuse(what);
        }
    }
    Ok(())
}

/// Map `folder` (relative to `root`, inside it; `.` for the root itself):
/// see the module docs. Refuses a root [`refuse_root`] refuses and a folder
/// outside the root; a file that does not compile, cannot be read or is too
/// large is a fact of that file, never a stop.
pub fn map_folder(root: &Path, folder: &Path) -> Result<FolderMap, Error> {
    map_folder_in(root, folder, &std::env::temp_dir())
}

/// [`map_folder`] with the fresh folder made under `fresh_parent`.
pub(crate) fn map_folder_in(
    root: &Path,
    folder: &Path,
    fresh_parent: &Path,
) -> Result<FolderMap, Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    refuse_root(&root)?;
    let walked_dir = root.join(folder);
    let inside = walked_dir
        .canonicalize()
        .map_err(|e| Error::io(&walked_dir, e))?;
    if !inside.starts_with(&root) || folder.is_absolute() {
        return Err(Error::Invariant(format!(
            "the folder to map, {}, lies outside {}: name a folder inside the project",
            folder.display(),
            root.display()
        )));
    }
    let folder_rel = rel_of(&root, &inside).unwrap_or_else(|| ".".into());

    // Step 1: the walk.
    // Walked under its canonical path, so every file is a plain path under
    // the root.
    let walked = walk::confined_except(
        &inside,
        &harness_scan::C_EXTENSIONS,
        walk::Limits {
            max_files: Some(MAX_FILES),
            max_depth: Some(MAX_DEPTH),
        },
        &[root.join(harness_core::ledger::MIGRATION_DIR)],
    );
    let mut limits_hit = Vec::new();
    if walked.truncated {
        limits_hit.push(if walked.files.len() >= MAX_FILES {
            LimitHit {
                limit: "files",
                at: format!("{MAX_FILES} .c and .h files"),
            }
        } else {
            LimitHit {
                limit: "depth",
                at: format!("{MAX_DEPTH} folders deep"),
            }
        });
    }
    let shown = |p: &Path| rel_of(&root, p).unwrap_or_else(|| p.to_string_lossy().into_owned());
    let walk_issues = walked
        .issues
        .iter()
        .map(|i| WalkIssue {
            path: shown(&i.path),
            why: i.why.words(),
        })
        .collect();
    let skipped_folders = walked
        .skipped_folders
        .iter()
        .map(|s| SkippedFolder {
            path: shown(&s.path),
            files: s.files,
            complete: s.complete,
        })
        .collect();
    let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for a in &walked.aliases {
        aliases
            .entry(shown(&a.file))
            .or_default()
            .push(shown(&a.path));
    }
    let mut paths: Vec<String> = walked
        .files
        .iter()
        .filter_map(|f| rel_of(&root, f))
        .collect();
    paths.sort();

    // Step 2: the scanner's facts.
    let mut files: Vec<FileFacts> = paths
        .iter()
        .map(|rel| read_file(&root, rel, aliases.remove(rel).unwrap_or_default()))
        .collect();
    let mut scanned: BTreeMap<String, Vec<harness_scan::Include>> = BTreeMap::new();
    for (facts, rel) in files.iter_mut().zip(&paths) {
        let (count, includes) = scan(&root, facts);
        facts.functions = count;
        scanned.insert(rel.clone(), includes);
    }

    // The fresh folder and the runner (§3.9), the toolchain once.
    let fresh = Fresh::make(fresh_parent)?;
    let runner = Runner::map(
        &root,
        &fresh.0,
        &["cc"],
        Duration::from_secs(COMPILE_TIMEOUT_SECS),
    )?;
    let toolchain = toolchain(&runner)?;
    let sys_dirs: Vec<PathBuf> = toolchain
        .system_include_dirs
        .iter()
        .map(|d| d.canonicalize().unwrap_or_else(|_| d.clone()))
        .collect();

    // Step 3: include folders, per file.
    let resolver = Resolver::new(&root, &paths, &scanned, &sys_dirs);
    for facts in &mut files {
        let (direct, dirs, ambiguous) = resolver.closure(&facts.path);
        facts.includes = direct;
        facts.include_dirs = dirs;
        facts.ambiguous = ambiguous;
    }

    // Step 5: the compiles.
    let walked_set: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    let scrubber = Scrubber::from_env(&root);
    let ctx = CompileCtx {
        root: &root,
        fresh: &fresh.0,
        runner: &runner,
        sys_dirs: &sys_dirs,
        walked: &walked_set,
        scrubber: &scrubber,
    };
    for (index, facts) in files.iter_mut().enumerate() {
        if facts.kind == FileKind::C && !facts.too_large {
            compile(&ctx, index, facts)?;
        }
    }
    drop(fresh);
    Ok(FolderMap {
        root,
        folder: folder_rel,
        toolchain,
        files,
        walk_issues,
        skipped_folders,
        limits_hit,
    })
}

/// `path` relative to `root`, `/`-separated; `.` for the root itself;
/// `None` when outside it.
fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

/// The file's size, hash and size cap (step 1).
fn read_file(root: &Path, rel: &str, aliases: Vec<String>) -> FileFacts {
    let abs = root.join(rel);
    let bytes = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
    FileFacts {
        path: rel.to_string(),
        aliases,
        kind: if rel.ends_with(".c") {
            FileKind::C
        } else {
            FileKind::H
        },
        bytes,
        blake3: harness_core::hash::file_hash(&abs).unwrap_or_default(),
        parsed: false,
        too_large: bytes > MAX_SOURCE_BYTES,
        not_utf8: false,
        functions: 0,
        includes: Vec::new(),
        include_dirs: Vec::new(),
        ambiguous: Vec::new(),
        compiled: None,
        outside_includes: false,
        included_other: Vec::new(),
        defined: Vec::new(),
        needed: Vec::new(),
        odd_names: 0,
        withheld_names: 0,
        message: None,
    }
}

/// Step 2 for one file: its function count and includes (none when too
/// large or unreadable), marking it parsed and UTF-8 or not.
fn scan(root: &Path, facts: &mut FileFacts) -> (usize, Vec<harness_scan::Include>) {
    if facts.too_large {
        return (0, Vec::new());
    }
    let Ok(source) = std::fs::read(root.join(&facts.path)) else {
        return (0, Vec::new());
    };
    facts.not_utf8 = std::str::from_utf8(&source).is_err();
    match harness_scan::file_facts(&source) {
        Ok(read) => {
            facts.parsed = true;
            (read.functions.len(), read.includes)
        }
        Err(_) => (0, Vec::new()),
    }
}

/// The clean parts of an include name: `None` when it is absolute or
/// climbs (`..`), which only the including file's own folder can resolve.
fn clean_parts(name: &str) -> Option<Vec<&str>> {
    if name.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = name
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    (!parts.is_empty() && !parts.contains(&"..")).then_some(parts)
}

/// `name` resolved lexically against the folder of `including` (both
/// relative to the root): `None` when it climbs out of the root or is
/// absolute.
fn beside(including: &str, name: &str) -> Option<String> {
    if name.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = including.split('/').collect();
    parts.pop();
    for seg in name.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// How one include of one file resolves (§3.1 step 3).
enum Resolution {
    /// A project file reached without a folder (beside the including file,
    /// quoted) or through one folder no one else holds the name in.
    Project {
        file: String,
        folder: Option<String>,
    },
    /// Several holders: every candidate, none picked.
    Ambiguous(Ambiguous),
    /// The system's, a file the walk did not record, or nowhere.
    Elsewhere,
}

/// Step 3: include folders by whole path parts.
struct Resolver<'a> {
    root: &'a Path,
    walked: BTreeSet<&'a str>,
    /// Walked files by their last path part.
    by_name: BTreeMap<&'a str, Vec<&'a str>>,
    includes: &'a BTreeMap<String, Vec<harness_scan::Include>>,
    sys_dirs: &'a [PathBuf],
}

impl<'a> Resolver<'a> {
    fn new(
        root: &'a Path,
        paths: &'a [String],
        includes: &'a BTreeMap<String, Vec<harness_scan::Include>>,
        sys_dirs: &'a [PathBuf],
    ) -> Resolver<'a> {
        let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for p in paths {
            let last = p.rsplit('/').next().unwrap_or(p);
            by_name.entry(last).or_default().push(p);
        }
        Resolver {
            root,
            walked: paths.iter().map(String::as_str).collect(),
            by_name,
            includes,
            sys_dirs,
        }
    }

    fn system_holds(&self, name: &str) -> bool {
        self.sys_dirs.iter().any(|d| d.join(name).is_file())
    }

    fn resolve(&self, including: &str, inc: &harness_scan::Include) -> Resolution {
        // A quoted include beside the including file is that file, whatever
        // the flags: never ambiguous.
        if inc.quoted {
            if let Some(near) = beside(including, &inc.name) {
                if self.walked.contains(near.as_str()) {
                    return Resolution::Project {
                        file: near,
                        folder: None,
                    };
                }
                if self.root.join(&near).is_file() {
                    return Resolution::Elsewhere;
                }
            }
        }
        let Some(parts) = clean_parts(&inc.name) else {
            return Resolution::Elsewhere;
        };
        let last = parts[parts.len() - 1];
        let mut holders: Vec<(String, &str)> = Vec::new();
        for path in self.by_name.get(last).into_iter().flatten() {
            if *path == including {
                continue;
            }
            let segs: Vec<&str> = path.split('/').collect();
            if segs.len() >= parts.len() && segs[segs.len() - parts.len()..] == parts[..] {
                let folder = segs[..segs.len() - parts.len()].join("/");
                let folder = if folder.is_empty() {
                    ".".into()
                } else {
                    folder
                };
                holders.push((folder, path));
            }
        }
        let system = self.system_holds(&inc.name);
        match holders.len() {
            0 => Resolution::Elsewhere,
            1 if !system => {
                let (folder, path) = holders.remove(0);
                Resolution::Project {
                    file: path.to_string(),
                    folder: Some(folder),
                }
            }
            _ => {
                let mut candidates: Vec<String> =
                    holders.into_iter().map(|(_, p)| p.to_string()).collect();
                candidates.sort();
                if system {
                    candidates.push("system".into());
                }
                Resolution::Ambiguous(Ambiguous {
                    header: inc.name.clone(),
                    candidates,
                    used: None,
                })
            }
        }
    }

    /// For `file`: the project files its own includes reach, the folders it
    /// and the headers it reaches need (in the order first needed), and the
    /// ambiguous includes among them.
    fn closure(&self, file: &str) -> (Vec<String>, Vec<String>, Vec<Ambiguous>) {
        let mut direct = BTreeSet::new();
        let mut dirs: Vec<String> = Vec::new();
        let mut ambiguous: BTreeMap<String, Ambiguous> = BTreeMap::new();
        let mut seen: BTreeSet<String> = BTreeSet::from([file.to_string()]);
        let mut queue: VecDeque<String> = VecDeque::from([file.to_string()]);
        while let Some(at) = queue.pop_front() {
            for inc in self.includes.get(&at).into_iter().flatten() {
                match self.resolve(&at, inc) {
                    Resolution::Project { file: hit, folder } => {
                        if at == file {
                            direct.insert(hit.clone());
                        }
                        if let Some(folder) = folder {
                            if !dirs.contains(&folder) {
                                dirs.push(folder);
                            }
                        }
                        if seen.insert(hit.clone()) {
                            queue.push_back(hit);
                        }
                    }
                    Resolution::Ambiguous(a) => {
                        ambiguous.entry(a.header.clone()).or_insert(a);
                    }
                    Resolution::Elsewhere => {}
                }
            }
        }
        (
            direct.into_iter().collect(),
            dirs,
            ambiguous.into_values().collect(),
        )
    }
}

/// The map's fresh folder: made private, removed with everything in it when
/// dropped (on every way out of the run).
struct Fresh(PathBuf);

impl Fresh {
    fn make(parent: &Path) -> Result<Fresh, Error> {
        let dir = parent.join(format!(
            "ruharness-map-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(6)
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir).map_err(|e| Error::io(&dir, e))?;
        let canonical = dir.canonicalize().map_err(|e| Error::io(&dir, e))?;
        Ok(Fresh(canonical))
    }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The toolchain's identity and its own include folders, read once.
fn toolchain(runner: &Runner) -> Result<Toolchain, Error> {
    let text = |argv: &[&str]| -> Result<String, Error> {
        let argv: Vec<String> = argv.iter().map(|a| (*a).to_string()).collect();
        Ok(String::from_utf8_lossy(&runner.tool(&argv)?).into_owned())
    };
    let cc = text(&["cc", "--version"])?
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let target = text(&["cc", "-dumpmachine"])?.trim().to_string();
    let argv: Vec<String> = ["cc", "-E", "-v", "-x", "c", "/dev/null"]
        .iter()
        .map(|a| (*a).to_string())
        .collect();
    let (_, stderr) = runner
        .tool_outcome_both(&argv)?
        .map_err(|e| Error::Invariant(format!("`cc -E -v` failed: {e}")))?;
    Ok(Toolchain {
        cc,
        target,
        cflags: MAP_CFLAGS.iter().map(|f| (*f).to_string()).collect(),
        system_include_dirs: system_include_dirs(&String::from_utf8_lossy(&stderr)),
    })
}

/// The `#include <...>` search folders `cc -E -v` lists, in order, a
/// framework folder's ` (framework directory)` dropped.
fn system_include_dirs(verbose: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut inside = false;
    for line in verbose.lines() {
        if line.starts_with("#include <...> search starts here:") {
            inside = true;
        } else if line.starts_with("End of search list.") {
            break;
        } else if inside {
            let dir = line.trim();
            let dir = dir.strip_suffix(" (framework directory)").unwrap_or(dir);
            if dir.starts_with('/') {
                dirs.push(PathBuf::from(dir));
            }
        }
    }
    dirs
}

/// What every compile shares.
struct CompileCtx<'a> {
    root: &'a Path,
    fresh: &'a Path,
    runner: &'a Runner,
    sys_dirs: &'a [PathBuf],
    walked: &'a BTreeSet<&'a str>,
    scrubber: &'a Scrubber,
}

fn path_arg(p: &Path) -> Result<String, Error> {
    crate::path_str(p).map(str::to_string)
}

/// Step 5 for one `.c`: compile, read the dependency list and the object,
/// delete both.
fn compile(ctx: &CompileCtx<'_>, index: usize, facts: &mut FileFacts) -> Result<(), Error> {
    let object = ctx.fresh.join(format!("{index}.o"));
    let deps = ctx.fresh.join(format!("{index}.d"));
    let mut argv: Vec<String> = vec!["cc".into(), "-c".into(), "-w".into()];
    argv.extend(MAP_CFLAGS.iter().map(|f| (*f).to_string()));
    for dir in &facts.include_dirs {
        argv.push(format!("-I{}", path_arg(&ctx.root.join(dir))?));
    }
    argv.push(path_arg(&ctx.root.join(&facts.path))?);
    argv.extend([
        "-o".into(),
        path_arg(&object)?,
        "-MD".into(),
        "-MF".into(),
        path_arg(&deps)?,
    ]);
    let run = ctx.runner.tool_run(&argv);
    let listed = std::fs::read(&deps).ok();
    let _ = std::fs::remove_file(&deps);
    let out = match run {
        Ok(out) => out,
        Err(e) => {
            let _ = std::fs::remove_file(&object);
            return Err(e);
        }
    };
    let other = |detail: &'static str| Compiled::Failed {
        reason: Reason::Other,
        header: None,
        detail: Some(detail),
        at: None,
    };
    match out.end {
        ChildEnd::Exited(status) if status.success() => {}
        ChildEnd::Exited(_) => {
            let text = String::from_utf8_lossy(&out.stderr);
            let (compiled, line) = classify(ctx.root, &text);
            facts.compiled = Some(compiled);
            facts.message = line.map(|l| ctx.scrubber.apply(&l));
            let _ = std::fs::remove_file(&object);
            return Ok(());
        }
        ChildEnd::TimedOut => {
            facts.compiled = Some(other("timeout"));
            let _ = std::fs::remove_file(&object);
            return Ok(());
        }
        ChildEnd::OutputOverflow => {
            facts.compiled = Some(other("output-overflow"));
            let _ = std::fs::remove_file(&object);
            return Ok(());
        }
    }
    if let Some(listed) = listed {
        read_dependencies(ctx, facts, &String::from_utf8_lossy(&listed));
    }
    let size = std::fs::metadata(&object).map(|m| m.len()).unwrap_or(0);
    let read = if size > MAX_OBJECT_BYTES {
        None
    } else {
        std::fs::read(&object).ok()
    };
    let _ = std::fs::remove_file(&object);
    let symbols = match read {
        None if size > MAX_OBJECT_BYTES => {
            facts.compiled = Some(other("too-large-object"));
            return Ok(());
        }
        None => Err(String::new()),
        Some(bytes) => objsyms::external(&bytes),
    };
    let Ok(symbols) = symbols else {
        facts.compiled = Some(other("unreadable-object"));
        return Ok(());
    };
    facts.compiled = Some(Compiled::Ok);
    keep_symbols(facts, symbols);
    Ok(())
}

/// The names worth keeping (§3.1 step 5): identifier-shaped ones by their
/// base name, the rest counted; every name withheld (counted) when the
/// compile read outside the root and the toolchain.
fn keep_symbols(facts: &mut FileFacts, symbols: objsyms::External) {
    let mut keep = |name: &str| -> Option<String> {
        if !objsyms::identifier_shaped(name) {
            facts.odd_names += 1;
            return None;
        }
        if facts.outside_includes {
            facts.withheld_names += 1;
            return None;
        }
        Some(objsyms::dollar_base(name).unwrap_or(name).to_string())
    };
    let mut defined: Vec<DefinedSymbol> = symbols
        .defines
        .iter()
        .filter_map(|d| {
            keep(&d.name).map(|name| DefinedSymbol {
                name,
                kind: d.kind.as_str(),
                weak: d.weak,
            })
        })
        .collect();
    let mut needed: Vec<NeededSymbol> = symbols
        .needs
        .iter()
        .filter_map(|n| keep(&n.name).map(|name| NeededSymbol { name, weak: n.weak }))
        .collect();
    defined.sort_by(|a, b| a.name.cmp(&b.name));
    defined.dedup_by(|a, b| a.name == b.name);
    needed.sort_by(|a, b| a.name.cmp(&b.name));
    needed.dedup_by(|a, b| a.name == b.name);
    facts.defined = defined;
    facts.needed = needed;
}

/// Read a compile's dependency list: the files inside the root the walk did
/// not record, whether it read outside the root and the toolchain, and which
/// candidate each ambiguous include used.
fn read_dependencies(ctx: &CompileCtx<'_>, facts: &mut FileFacts, listed: &str) {
    // The compiled file itself is taken out first, as the compiler wrote it:
    // make's syntax cannot escape a newline, which a file name may hold.
    let own = ctx
        .root
        .join(&facts.path)
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace('#', "\\#")
        .replace('$', "$$");
    let listed = listed.replacen(&format!(" {own}"), " ", 1);
    let read: Vec<PathBuf> = dependency_paths(&listed)
        .into_iter()
        .map(|p| {
            let p = PathBuf::from(p);
            p.canonicalize().unwrap_or(p)
        })
        .collect();
    let mut other = BTreeSet::new();
    for path in &read {
        if let Some(rel) = rel_of(ctx.root, path) {
            if rel != facts.path && !ctx.walked.contains(rel.as_str()) {
                other.insert(rel);
            }
        } else if !ctx.sys_dirs.iter().any(|d| path.starts_with(d)) {
            facts.outside_includes = true;
        }
    }
    facts.included_other = other.into_iter().collect();
    let read: BTreeSet<&PathBuf> = read.iter().collect();
    for amb in &mut facts.ambiguous {
        amb.used = amb
            .candidates
            .iter()
            .find(|c| {
                if c.as_str() == "system" {
                    ctx.sys_dirs.iter().any(|d| {
                        let p = d.join(&amb.header);
                        read.contains(&p.canonicalize().unwrap_or(p))
                    })
                } else {
                    let p = ctx.root.join(c.as_str());
                    read.contains(&p.canonicalize().unwrap_or(p))
                }
            })
            .cloned();
    }
}

/// The prerequisites of a make rule as `-MD` writes it: everything after the
/// first `: `, split on unescaped blanks, with `\ ` a space, `\#` a `#`,
/// `$$` a `$` and a backslash-newline a blank.
fn dependency_paths(rule: &str) -> Vec<String> {
    let Some((_, rest)) = rule.split_once(": ") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some(' ') | Some('#') | Some('\\') => {
                    cur.extend(chars.next());
                }
                Some('\n') | Some('\r') => {
                    chars.next();
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push('\\'),
            },
            '$' if chars.peek() == Some(&'$') => {
                chars.next();
                cur.push('$');
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A failed compile's closed reason and its first error line (raw: the
/// caller scrubs it). clang: `<path>:<line>:<col>: fatal error: '<name>'
/// file not found`; gcc: `…: fatal error: <name>: No such file or
/// directory`.
fn classify(root: &Path, stderr: &str) -> (Compiled, Option<String>) {
    let line = stderr
        .lines()
        .find(|l| l.contains("error:"))
        .or_else(|| stderr.lines().find(|l| !l.trim().is_empty()));
    let Some(line) = line else {
        return (
            Compiled::Failed {
                reason: Reason::Other,
                header: None,
                detail: None,
                at: None,
            },
            None,
        );
    };
    let (place, said) = match line
        .find(": fatal error: ")
        .or_else(|| line.find(": error: "))
    {
        Some(i) => (
            Some(&line[..i]),
            line[i..].split_once("error: ").map(|x| x.1),
        ),
        None => (None, None),
    };
    // `<path>:<line>[:<col>]`, the path inside the root.
    let at = place.and_then(|p| {
        let mut parts = p.rsplitn(3, ':');
        let (a, b, c) = (parts.next()?, parts.next(), parts.next());
        let (path, line_no) = match (b, c) {
            (Some(l), Some(path)) if l.bytes().all(|x| x.is_ascii_digit()) => (path, l),
            (Some(path), None) if a.bytes().all(|x| x.is_ascii_digit()) => (path, a),
            _ => return None,
        };
        let abs = PathBuf::from(path);
        let abs = abs.canonicalize().unwrap_or(abs);
        rel_of(root, &abs).map(|rel| format!("{rel}:{line_no}"))
    });
    let missing = said.and_then(|s| {
        s.strip_prefix('\'')
            .and_then(|r| r.strip_suffix("' file not found"))
            .or_else(|| s.strip_suffix(": No such file or directory"))
    });
    let compiled = match (missing, place) {
        (Some(name), _) => Compiled::Failed {
            reason: Reason::MissingHeader,
            header: Some(name.to_string()),
            detail: None,
            at,
        },
        (None, Some(_)) => Compiled::Failed {
            reason: Reason::Syntax,
            header: None,
            detail: None,
            at,
        },
        (None, None) => Compiled::Failed {
            reason: Reason::Other,
            header: None,
            detail: None,
            at: None,
        },
    };
    (compiled, Some(line.to_string()))
}

pub mod closure;
pub mod ids;
pub mod link;
pub use closure::{analyze, Analysis, Input, Linked, Linker, ParserFacts};
pub use link::{analyze_linked, CcLinker};

#[cfg(test)]
mod tests;
