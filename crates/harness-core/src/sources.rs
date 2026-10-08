//! A target's C sources, confined (docs/PROJECT-MAP-DESIGN.md §3.7 "The
//! scanner" and "Confinement, restated"): the folders no scan, prompt or
//! compile reads as project C (`<root>/migration/`, every tool's ledger and
//! the map), a file-list target's files checked against the root and the
//! ledger, **the include rule** of a file-list target ([`Resolver`]), and
//! **the include reader** ([`include_names`]) every reader takes names from
//! — the scanner, the staleness rule, the prompt scope and detect alike.
//!
//! # The include rule (one rule, one place)
//!
//! A file-list target's files are compiled with the configuration's path
//! flags before each file's own folders (the oracle passes the judge's
//! flags, the configuration's flags, then the file's `-I`), so a reader
//! that wants the file the compiler reads searches in the compiler's own
//! order. For an include name N written in file F, compiled as part of the
//! listed file U (U itself, a header U reaches, or a `-include` file):
//!
//! 1. a quoted include: F's own folder; the configuration's `-iquote`
//!    folders in order; its `-I` folders in order; U's `include_dirs` in
//!    order; its `-isystem` folders in order; then the system;
//! 2. an angle-bracket include: the same without F's own folder and without
//!    the `-iquote` folders (the compiler searches `-iquote` for quoted
//!    includes only);
//! 3. every `-include` file of the configuration is the first include of
//!    every listed file, and its own includes are followed like any
//!    header's.
//!
//! The first folder holding N as a regular file is the one the compiler
//! takes ([`Landing`]). A file it takes that lies outside the project root
//! or under `migration/` is never project C: it is [`Landing::Outside`],
//! which a scan notes and nothing reads. Otherwise the system holds N, or
//! nothing does ([`Landing::NotProject`]).
//!
//! A header reached from two listed files whose folders differ may resolve
//! one name to two places. Such a name is an **ambiguous include**
//! ([`Ambiguous`]): no edge is recorded for it, never a silent union.
//!
//! The API, for every reader (harness-scan, harness-detect, harness-llm,
//! the program digest and staleness, and the oracle's closure readers):
//!
//! - [`Resolver::new`]`(root, ledger, &TargetSection)`, or
//!   [`Resolver::of`]`(ctx)`: `Ok(None)` for a folder target, whose M4 rule
//!   (quoted includes only, inside `source_dir`) stays the scanner's;
//! - [`Resolver::listed`] and [`Resolver::forced_includes`]: the listed
//!   files and the `-include` files, as a scan records paths;
//! - [`Resolver::search_order`]`(unit, including, quoted)`: the folders
//!   searched, in order, root-relative;
//! - [`Resolver::find`] and [`Resolver::resolve`]`(unit, including, name,
//!   quoted)`: where one include lands;
//! - [`Resolver::walk`]`(names_of)`: the whole program — every file the
//!   listed files reach and the edges a scan records — given each file's
//!   include names ([`names_on_disk`] reads them as a scan would).
//!
//! Paths are as a scan records them: links resolved, relative to the
//! canonical root, `/`-joined, `""` for the root itself.
//!
//! # The include reader
//!
//! [`include_names`] reads names lexically, as the preprocessor does:
//! continued lines joined first, then comments removed, then every line
//! `# include "name"` or `# include <name>` from every `#if` branch alike,
//! wherever it sits (inside a struct, an enum or an initializer too: the
//! X-macro pattern). An include built by a macro names no file.

use crate::config::{TargetContext, TargetSection};
use crate::error::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Largest source file a scan parses (docs/PROJECT-MAP-DESIGN.md §3.10): a
/// larger one is recorded with its hash, never parsed.
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

/// The `hash` a scan records for a file it reached but could not read (its
/// permissions, say): the file is a fact, so staleness counts it as
/// recorded while it stays unreadable, and detect skips it.
pub const UNREADABLE_HASH: &str = "unreadable";

/// The folders every scan, detect and prompt read leaves out:
/// `<root>/migration/` (every tool's ledger and the map) and the target's
/// own ledger, when it lies elsewhere.
pub fn pruned(ctx: &TargetContext) -> Vec<PathBuf> {
    pruned_at(&ctx.root, &ctx.ledger)
}

fn pruned_at(root: &Path, ledger: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.join(crate::ledger::MIGRATION_DIR)];
    if !out.iter().any(|p| p == ledger) {
        out.push(ledger.to_path_buf());
    }
    out
}

/// Whether the root-relative `rel` (`/`-joined) passes through a folder
/// whose name starts with a dot: the walk never enters one, so no reader
/// takes project C from one.
pub fn in_dot_folder(rel: &str) -> bool {
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop();
    parts
        .iter()
        .any(|p| p.starts_with('.') && *p != "." && *p != "..")
}

/// The project root and the pruned folders, canonical: where project C may
/// be read from.
#[derive(Debug, Clone)]
pub struct Confine {
    root: PathBuf,
    ledger: PathBuf,
    pruned: Vec<PathBuf>,
}

/// One listed file of a file-list target, by the path a scan records it
/// under (its links resolved, relative to the root, `/`-joined), with its
/// include folders the same way (`""` is the root itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFile {
    /// The file.
    pub path: String,
    /// Its include folders, in order.
    pub include_dirs: Vec<String>,
}

impl Confine {
    /// The confinement of `ctx`; `Err` when the root cannot be resolved.
    pub fn new(ctx: &TargetContext) -> Result<Confine, Error> {
        Confine::at(&ctx.root, &ctx.ledger)
    }

    /// The confinement of the project at `root` whose ledger is `ledger`.
    pub fn at(root: &Path, ledger: &Path) -> Result<Confine, Error> {
        let canon = root.canonicalize().map_err(|e| Error::io(root, e))?;
        let pruned = pruned_at(root, ledger)
            .iter()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
            .collect();
        Ok(Confine {
            root: canon,
            ledger: ledger.to_path_buf(),
            pruned,
        })
    }

    /// The canonical project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the canonical `path` lies inside the root and outside every
    /// pruned folder.
    pub fn allows(&self, path: &Path) -> bool {
        path.starts_with(&self.root) && !self.pruned.iter().any(|p| path.starts_with(p))
    }

    /// The canonical `path` relative to the root, `/`-joined (`""` for the
    /// root); `None` outside it.
    pub fn rel(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    /// `rel` (`.` or a clean relative path, `./` and a trailing `/`
    /// allowed) as a scan records it: its links resolved when it exists,
    /// else as written (`""` for the root). `Err(true)` when it leads under
    /// a pruned folder, `Err(false)` when it leads outside the root. The
    /// deepest part that exists decides, as for the config.
    pub fn place(&self, rel: &str) -> Result<String, bool> {
        let mut lexical = rel.trim_end_matches('/');
        while let Some(rest) = lexical.strip_prefix("./") {
            lexical = rest.trim_start_matches('/');
        }
        let lexical = if lexical == "." { "" } else { lexical };
        let under_ledger = lexical
            .split('/')
            .next()
            .is_some_and(|first| first == crate::ledger::MIGRATION_DIR);
        if lexical.starts_with('/')
            || lexical.split('/').any(|s| s == "..")
            || (!lexical.is_empty() && !crate::plan::is_clean_relative_path(lexical))
        {
            return Err(false);
        }
        if under_ledger {
            return Err(true);
        }
        let mut probe = self.root.join(lexical);
        let mut rest: Vec<std::ffi::OsString> = Vec::new();
        loop {
            match probe.canonicalize() {
                Ok(real) => {
                    if !real.starts_with(&self.root) {
                        return Err(false);
                    }
                    if !self.allows(&real) {
                        return Err(true);
                    }
                    let mut full = real;
                    full.extend(rest.iter().rev());
                    return self.rel(&full).ok_or(false);
                }
                Err(_) => match probe.file_name() {
                    Some(name) => {
                        rest.push(name.to_os_string());
                        probe.pop();
                    }
                    None => return Err(false),
                },
            }
        }
    }

    /// The one-sentence refusal of a path that leads outside the root
    /// (`ledger` false) or under `migration/` (true).
    fn refusal(&self, what: String, ledger: bool) -> Error {
        let why = if ledger {
            "lies under migration/, the harness's own folder, whose files are never read as \
             project C; name the project's own file or folder"
        } else {
            "leads outside the project root; name a path inside the project"
        };
        Error::parse(
            self.ledger.join(crate::config::CONFIG_FILE),
            format!("{what} {why}"),
        )
    }

    /// A file-list target's files and their include folders, as a scan
    /// records them; `Ok(None)` for a folder target. A file or a folder that
    /// leads outside the project root or under `migration/` is refused in
    /// one sentence naming it (a `harness.toml` pointing into the ledger
    /// would put model-written files into prompts and builds).
    pub fn listed_files(&self, ctx: &TargetContext) -> Result<Option<Vec<ListedFile>>, Error> {
        self.listed_in(&ctx.config.target)
    }

    fn listed_in(&self, target: &TargetSection) -> Result<Option<Vec<ListedFile>>, Error> {
        let Some(files) = target.files() else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let shown = crate::text::safe_line(&file.path);
            let path = self
                .place(&file.path)
                .map_err(|ledger| self.refusal(format!("the listed file `{shown}`"), ledger))?;
            let mut include_dirs = Vec::with_capacity(file.include_dirs.len());
            for dir in &file.include_dirs {
                include_dirs.push(self.place(dir).map_err(|ledger| {
                    self.refusal(
                        format!(
                            "the include folder `{}` of `{shown}`",
                            crate::text::safe_line(dir)
                        ),
                        ledger,
                    )
                })?);
            }
            out.push(ListedFile { path, include_dirs });
        }
        Ok(Some(out))
    }
}

/// Where one include lands ([`Resolver::find`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Landing {
    /// A project file, as a scan records it.
    Project(String),
    /// A file the compiler takes that lies outside the project root or
    /// under `migration/`: never read as project C (its canonical path).
    Outside(PathBuf),
    /// No project folder holds the name: the system does, or nothing.
    NotProject,
}

/// An include name that a file reached from two listed files resolves to
/// two different places (docs/PROJECT-MAP-DESIGN.md §3.1 step 3): no edge
/// is recorded for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    /// The including file.
    pub file: String,
    /// The include name, as written.
    pub name: String,
    /// `#include "name"` (true) or `#include <name>` (false).
    pub quoted: bool,
    /// For one listed file per distinct folder list, where the name lands
    /// when compiled as part of it.
    pub lands: Vec<(String, Landing)>,
}

/// An include that lands outside the project root or under `migration/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutsideInclude {
    /// The including file.
    pub file: String,
    /// The include name, as written.
    pub name: String,
    /// Where the compiler finds it (canonical).
    pub path: PathBuf,
}

/// The whole program as the include rule reads it ([`Resolver::walk`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    /// Every file reached — the listed files, the `-include` files and the
    /// headers their includes reach — with the project files its includes
    /// reach (an ambiguous name contributes none), as a scan records them.
    pub files: BTreeMap<String, BTreeSet<String>>,
    /// The files reached whose include names could not be read.
    pub unreadable: BTreeSet<String>,
    /// The ambiguous includes, sorted by file and name.
    pub ambiguous: Vec<Ambiguous>,
    /// The includes that land outside the project or under `migration/`,
    /// sorted.
    pub outside: Vec<OutsideInclude>,
}

/// The include rule of one file-list target (see the module docs).
#[derive(Debug, Clone)]
pub struct Resolver {
    confine: Confine,
    listed: Vec<ListedFile>,
    iquote: Vec<String>,
    dash_i: Vec<String>,
    isystem: Vec<String>,
    forced: Vec<String>,
}

impl Resolver {
    /// The resolver of `ctx`'s target; `Ok(None)` for a folder target.
    pub fn of(ctx: &TargetContext) -> Result<Option<Resolver>, Error> {
        Resolver::new(&ctx.root, &ctx.ledger, &ctx.config.target)
    }

    /// The resolver of the target `target` of the project at `root` whose
    /// ledger is `ledger`; `Ok(None)` for a folder target. A listed file, an
    /// include folder or a flag's path that leads outside the root or under
    /// `migration/` is refused in one sentence naming it.
    pub fn new(
        root: &Path,
        ledger: &Path,
        target: &TargetSection,
    ) -> Result<Option<Resolver>, Error> {
        let confine = Confine::at(root, ledger)?;
        let Some(listed) = confine.listed_in(target)? else {
            return Ok(None);
        };
        let mut resolver = Resolver {
            listed,
            iquote: Vec::new(),
            dash_i: Vec::new(),
            isystem: Vec::new(),
            forced: Vec::new(),
            confine,
        };
        let flags = target
            .configuration()
            .map(|c| c.flags.as_slice())
            .unwrap_or_default();
        for flag in flags {
            let Ok(crate::config::flags::Flag::Path(path)) = crate::config::flags::check_flag(flag)
            else {
                continue;
            };
            let placed = resolver.confine.place(path).map_err(|ledger| {
                resolver.confine.refusal(
                    format!("the flag `{}`", crate::text::safe_line(flag)),
                    ledger,
                )
            })?;
            let list = match &flag[..flag.len() - path.len()] {
                "-iquote" => &mut resolver.iquote,
                "-isystem" => &mut resolver.isystem,
                "-include" => &mut resolver.forced,
                _ => &mut resolver.dash_i,
            };
            list.push(placed);
        }
        Ok(Some(resolver))
    }

    /// The confinement it resolves within.
    pub fn confine(&self) -> &Confine {
        &self.confine
    }

    /// The canonical project root every path is relative to.
    pub fn root(&self) -> &Path {
        self.confine.root()
    }

    /// The listed files, as a scan records them.
    pub fn listed(&self) -> &[ListedFile] {
        &self.listed
    }

    /// The configuration's `-include` files, in order, as a scan records
    /// them: the first includes of every listed file.
    pub fn forced_includes(&self) -> &[String] {
        &self.forced
    }

    /// The configuration's path flags in their resolved forms, in order
    /// (`-Isrc/include`, `-iquote.` for the root): what the program digest
    /// hashes, so a cosmetic spelling is not a new program.
    pub fn resolved_flags(&self, flags: &[String]) -> Vec<String> {
        flags
            .iter()
            .map(|flag| match crate::config::flags::check_flag(flag) {
                Ok(crate::config::flags::Flag::Path(path)) => {
                    let prefix = &flag[..flag.len() - path.len()];
                    match self.confine.place(path) {
                        Ok(placed) if placed.is_empty() => format!("{prefix}."),
                        Ok(placed) => format!("{prefix}{placed}"),
                        Err(_) => flag.clone(),
                    }
                }
                _ => flag.clone(),
            })
            .collect()
    }

    /// The folders an include written in `including` is searched in, in
    /// order, when compiled as part of the listed file `unit` (see the
    /// module docs), root-relative (`""` is the root); the system's folders
    /// come after them. A `unit` that is not listed contributes no folders
    /// of its own.
    pub fn search_order(&self, unit: &str, including: &str, quoted: bool) -> Vec<String> {
        let own = match including.rsplit_once('/') {
            Some((dir, _)) => dir,
            None => "",
        };
        let unit_dirs = self
            .listed
            .iter()
            .find(|f| f.path == unit)
            .map(|f| f.include_dirs.as_slice())
            .unwrap_or_default();
        let mut order = Vec::new();
        if quoted {
            order.push(own.to_string());
            order.extend(self.iquote.iter().cloned());
        }
        order.extend(self.dash_i.iter().cloned());
        order.extend(unit_dirs.iter().cloned());
        order.extend(self.isystem.iter().cloned());
        order
    }

    /// Where the include `name` written in `including` lands when compiled
    /// as part of the listed file `unit`.
    pub fn find(&self, unit: &str, including: &str, name: &str, quoted: bool) -> Landing {
        if name.is_empty() {
            return Landing::NotProject;
        }
        let candidates: Vec<PathBuf> = if name.starts_with('/') {
            vec![PathBuf::from(name)]
        } else {
            self.search_order(unit, including, quoted)
                .iter()
                .map(|dir| self.root().join(dir).join(name))
                .collect()
        };
        for candidate in candidates {
            let Ok(real) = candidate.canonicalize() else {
                continue;
            };
            if !real.is_file() {
                continue;
            }
            // The compiler takes this file: project C only when confined.
            return match self
                .confine
                .allows(&real)
                .then(|| self.confine.rel(&real))
                .flatten()
            {
                Some(rel) => Landing::Project(rel),
                None => Landing::Outside(real),
            };
        }
        Landing::NotProject
    }

    /// [`Resolver::find`]'s project file, when it lands on one.
    pub fn resolve(&self, unit: &str, including: &str, name: &str, quoted: bool) -> Option<String> {
        match self.find(unit, including, name, quoted) {
            Landing::Project(rel) => Some(rel),
            _ => None,
        }
    }

    /// The whole program: from each listed file (its `-include` files
    /// first), every include followed to closure by the rule, given each
    /// reached file's include names by `names_of` (`None` when the file
    /// cannot be read; called once per file). A name that lands differently
    /// under two listed files is [`Ambiguous`] and gets no edge; the files
    /// it lands on are still reached.
    pub fn walk(&self, mut names_of: impl FnMut(&str) -> Option<Vec<(String, bool)>>) -> Program {
        let mut program = Program::default();
        let mut names: BTreeMap<String, Option<Vec<(String, bool)>>> = BTreeMap::new();
        // (file, name, quoted) → folder list → (a listed file, landing).
        type Lands<'a> = BTreeMap<&'a [String], (String, Landing)>;
        let mut lands: BTreeMap<(String, String, bool), Lands<'_>> = BTreeMap::new();
        let mut seen: BTreeSet<(String, &[String])> = BTreeSet::new();
        let mut outside = BTreeSet::new();
        for unit in &self.listed {
            let dirs = unit.include_dirs.as_slice();
            program
                .files
                .entry(unit.path.clone())
                .or_default()
                .extend(self.forced.iter().cloned());
            // Popped from the end: the listed file's `-include` files first.
            let mut queue: Vec<String> = vec![unit.path.clone()];
            queue.extend(self.forced.iter().rev().cloned());
            while let Some(file) = queue.pop() {
                if !seen.insert((file.clone(), dirs)) {
                    continue;
                }
                program.files.entry(file.clone()).or_default();
                let read = names
                    .entry(file.clone())
                    .or_insert_with(|| names_of(&file))
                    .clone();
                let Some(read) = read else {
                    program.unreadable.insert(file.clone());
                    continue;
                };
                let mut next = Vec::new();
                for (name, quoted) in read {
                    let landing = self.find(&unit.path, &file, &name, quoted);
                    match &landing {
                        Landing::Project(to) => next.push(to.clone()),
                        Landing::Outside(path) => {
                            outside.insert(OutsideInclude {
                                file: file.clone(),
                                name: name.clone(),
                                path: path.clone(),
                            });
                        }
                        Landing::NotProject => {}
                    }
                    lands
                        .entry((file.clone(), name, quoted))
                        .or_default()
                        .entry(dirs)
                        .or_insert_with(|| (unit.path.clone(), landing));
                }
                queue.extend(next.into_iter().rev());
            }
        }
        for ((file, name, quoted), by_dirs) in lands {
            let distinct: BTreeSet<&Landing> = by_dirs.values().map(|(_, l)| l).collect();
            if distinct.len() > 1 {
                program.ambiguous.push(Ambiguous {
                    file,
                    name,
                    quoted,
                    lands: by_dirs.into_values().collect(),
                });
            } else if let Some(Landing::Project(to)) = distinct.into_iter().next() {
                let to = to.clone();
                program.files.entry(file).or_default().insert(to);
            }
        }
        program.outside = outside.into_iter().collect();
        program
    }
}

/// A file's include names as a scan sees them, read through
/// [`crate::ledger::read_regular`] with the scanner's cap: `Some(empty)`
/// for a file over the cap (recorded, never parsed), `None` when it cannot
/// be read.
pub fn names_on_disk(root: &Path, rel: &str) -> Option<Vec<(String, bool)>> {
    let abs = root.join(rel);
    let meta = std::fs::metadata(&abs).ok()?;
    if meta.is_file() && meta.len() > MAX_SOURCE_BYTES {
        return Some(Vec::new());
    }
    crate::ledger::read_regular(&abs, MAX_SOURCE_BYTES)
        .ok()
        .map(|bytes| include_names(&bytes))
}

/// The include names a file's bytes write — `(name, quoted)`, sorted, each
/// once — read lexically (see the module docs): continued lines joined,
/// comments removed, then every line `# include "name"` or
/// `# include <name>`, from every `#if` branch alike, wherever it sits.
/// The one reader of include names: the scanner records what it reads.
pub fn include_names(src: &[u8]) -> Vec<(String, bool)> {
    let mut names = std::collections::BTreeSet::new();
    for line in strip_comments(&splice(src)).split(|&b| b == b'\n') {
        let Some((name, quoted)) = include_of(line) else {
            continue;
        };
        if let Ok(name) = std::str::from_utf8(name) {
            if !name.is_empty() {
                names.insert((name.to_string(), quoted));
            }
        }
    }
    names.into_iter().collect()
}

/// The name an include line writes, and whether it is quoted; `None` for
/// any other line.
fn include_of(line: &[u8]) -> Option<(&[u8], bool)> {
    let rest = trim_start(include_head(line)?);
    let (close, quoted) = match rest.first() {
        Some(b'"') => (b'"', true),
        Some(b'<') => (b'>', false),
        _ => return None,
    };
    let body = &rest[1..];
    let end = body.iter().position(|&b| b == close)?;
    Some((&body[..end], quoted))
}

/// What follows `# include` on a line, when the line starts so.
fn include_head(line: &[u8]) -> Option<&[u8]> {
    let rest = trim_start(line).strip_prefix(b"#")?;
    trim_start(rest).strip_prefix(b"include")
}

fn trim_start(s: &[u8]) -> &[u8] {
    let at = s
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'))
        .unwrap_or(s.len());
    &s[at..]
}

/// The bytes with every backslash-newline removed (the preprocessor's line
/// splicing, which comes before comments are read).
fn splice(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\\' if src.get(i + 1) == Some(&b'\n') => i += 2,
            b'\\' if src.get(i + 1) == Some(&b'\r') && src.get(i + 2) == Some(&b'\n') => i += 3,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// The bytes with every `/* … */` and `// …` comment replaced by one space
/// (newlines inside a block comment kept), string and character literals
/// passed through as they are, and an include's `<name>` passed through
/// whole (`<sys//types.h>` holds no comment).
fn strip_comments(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'<' if {
                let line_start = out.iter().rposition(|&b| b == b'\n').map_or(0, |n| n + 1);
                include_head(&out[line_start..]).is_some_and(|rest| trim_start(rest).is_empty())
            } =>
            {
                while i < src.len() && src[i] != b'\n' {
                    out.push(src[i]);
                    i += 1;
                    if out.last() == Some(&b'>') {
                        break;
                    }
                }
            }
            b'/' if src.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < src.len() && !(src[i] == b'*' && src.get(i + 1) == Some(&b'/')) {
                    if src[i] == b'\n' {
                        out.push(b'\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(src.len());
                out.push(b' ');
            }
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
                out.push(b' ');
            }
            q @ (b'"' | b'\'') => {
                out.push(q);
                i += 1;
                while i < src.len() && src[i] != q && src[i] != b'\n' {
                    if src[i] == b'\\' && i + 1 < src.len() {
                        out.push(src[i]);
                        i += 1;
                    }
                    out.push(src[i]);
                    i += 1;
                }
                if i < src.len() && src[i] == q {
                    out.push(q);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(name: &str) -> (String, bool) {
        (name.to_string(), true)
    }

    fn a(name: &str) -> (String, bool) {
        (name.to_string(), false)
    }

    /// The cases where the scanner's former parser and this reader parted
    /// (the readers review, finding 2): each is read the way the
    /// preprocessor reads it. harness-scan's
    /// `the_scanner_and_staleness_read_the_same_includes` checks that a
    /// scan records each and is fresh right after.
    #[test]
    fn include_names_reads_what_the_preprocessor_reads() {
        for (src, want) in [
            (
                &b"struct s {\n#include \"fields.h\"\n};\n"[..],
                vec![q("fields.h")],
            ),
            (
                b"union u {\n#include \"fields.h\"\n};\n",
                vec![q("fields.h")],
            ),
            (b"enum op {\n#include \"ops.def\"\n};\n", vec![q("ops.def")]),
            (
                b"static const char *names[] = {\n#include \"names.h\"\n};\n",
                vec![q("names.h")],
            ),
            (b"#inc\\\nlude \"e.h\"\n", vec![q("e.h")]),
            (b"#/**/include \"c.h\"\n", vec![q("c.h")]),
            (b"#include \"a.h\" junk\n", vec![q("a.h")]),
            (b"#include <sys//types.h>\n", vec![a("sys//types.h")]),
            (b"#include <a/*b.h>\n", vec![a("a/*b.h")]),
            (b"# include <x.h> // trailing\n", vec![a("x.h")]),
            (b"// #include \"no.h\" \\\n#include \"also-no.h\"\n", vec![]),
            (b"#include NAME\n", vec![]),
            (
                b"/* #include \"no.h\" */\n#include \"a.h\"\n  #  include <b/c.h>\n\
                  #ifdef X\n#include \"d.h\"\n#endif\n\
                  const char *s = \"#include \\\"no3.h\\\"\";\n",
                vec![q("a.h"), a("b/c.h"), q("d.h")],
            ),
        ] {
            assert_eq!(
                include_names(src),
                want,
                "{:?}",
                String::from_utf8_lossy(src)
            );
        }
    }

    #[test]
    fn a_dot_folder_is_seen_in_any_part_but_the_file() {
        assert!(in_dot_folder(".env/x.h"));
        assert!(in_dot_folder("src/.git/x.h"));
        assert!(!in_dot_folder("src/.x.h"));
        assert!(!in_dot_folder("src/x.h"));
    }
}
