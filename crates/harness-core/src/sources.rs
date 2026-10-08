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
//!    order; its `-isystem` folders in order; then the system; then its
//!    `-idirafter` folders in order;
//! 2. an angle-bracket include: the same without F's own folder and without
//!    the `-iquote` folders (the compiler searches `-iquote` for quoted
//!    includes only);
//! 3. every `-include` file of the configuration is the first include of
//!    every listed file, and its own includes are followed like any
//!    header's.
//!
//! A folder named by `-isystem` and also by `-I` or an `include_dirs` is
//! searched at its `-isystem` place only, as the compiler does (it drops
//! the `-I` of a system folder).
//!
//! The first folder holding N as a regular file is the one the compiler
//! takes ([`Landing`]). A file it takes that lies outside the project root
//! or under `migration/` is never project C: it is [`Landing::Outside`],
//! which a scan notes and nothing reads. Otherwise the system holds N, or
//! nothing does ([`Landing::NotProject`]). An `-idirafter` folder comes
//! after the system: its file is taken only when none of the usual system
//! folders of this computer's C compiler, found on disk, holds N
//! ([`system_holds`]).
//!
//! A header reached from two listed files whose folders differ may resolve
//! one name to two places. Such a name is an **ambiguous include**
//! ([`Ambiguous`]): no edge is recorded for it, never a silent union. The
//! files a unit's compile reads are still its **closure**
//! ([`Resolver::closure`], [`unit_closure`]): what `verify`'s
//! `unit_source`, the planner's `source_hash` and `state status` hash, so
//! an edit to a header an ambiguous include lands on stales the verdict.
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
//!   include names ([`names_on_disk`] reads them as a scan would);
//! - [`Resolver::closure`]`(facts, start)` and [`unit_closure`]`(ctx,
//!   facts, start)`: the files one unit's compile reads (the facts'
//!   closure for a folder target).
//!
//! Paths are as a scan records them: links resolved, relative to the
//! canonical root, `/`-joined, `""` for the root itself.
//!
//! # The include reader
//!
//! [`include_names`] reads names lexically, as the preprocessor does:
//! a lone carriage return read as a line end, continued lines joined first
//! (a backslash, then blanks, then the line end, is a continuation too),
//! then comments removed, then every line `# include "name"` or
//! `# include <name>` from every `#if` branch alike, wherever it sits
//! (inside a struct, an enum or an initializer too: the X-macro pattern).
//! `#import` and `#include_next` are read as includes, and `%:` is read as
//! `#`. `#include_next` is resolved as an ordinary include; since no
//! include lands on the file that writes it ([`Resolver::find`]), a
//! header's `#include_next` of its own name reaches the next one, as the
//! compiler's does. A limit: when a same-named file also lies in a folder
//! searched before the including file's own, the reader lands there
//! instead of after the including file's folder. An include built by a macro names no file. The reader's time grows
//! with the file's length, never with its square.

use crate::config::{TargetContext, TargetSection};
use crate::error::Error;
use crate::facts::Facts;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Largest source file a scan parses (docs/PROJECT-MAP-DESIGN.md §3.10): a
/// larger one is recorded with its hash, never parsed.
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

/// The `hash` a scan records for a file it reached but could not read (its
/// permissions, say): the file is a fact, so staleness counts it as
/// recorded while it stays unreadable, and detect skips it.
pub const UNREADABLE_HASH: &str = "unreadable";

/// The note scan and detect both give a file they could not read, `abs`
/// being its path and `why` the error: `cannot be read: <why>; recorded as
/// unreadable`, with no machine path in it (the caller shows the file by
/// its root-relative path).
pub fn unreadable_note(abs: &Path, why: &dyn std::fmt::Display) -> String {
    let shown = abs.display().to_string();
    let text = why.to_string();
    let text = text
        .strip_prefix(&format!("io error at {shown}: "))
        .or_else(|| text.strip_prefix(&format!("{shown}: ")))
        .unwrap_or(&text);
    format!(
        "cannot be read: {}; recorded as unreadable",
        crate::text::safe_line(text)
    )
}

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

/// A reader of one file's include names, `(name, quoted)`, by its
/// root-relative path: `None` when the file cannot be read
/// ([`Resolver::closure_with`]).
pub type NamesOf<'a> = dyn FnMut(&str) -> Option<Vec<(String, bool)>> + 'a;

/// The include rule of one file-list target (see the module docs).
#[derive(Debug, Clone)]
pub struct Resolver {
    confine: Confine,
    listed: Vec<ListedFile>,
    iquote: Vec<String>,
    dash_i: Vec<String>,
    isystem: Vec<String>,
    idirafter: Vec<String>,
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
            idirafter: Vec::new(),
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
            let list = match crate::config::flags::split_path_flag(flag).map(|(p, _)| p) {
                Some("-iquote") => &mut resolver.iquote,
                Some("-isystem") => &mut resolver.isystem,
                Some("-idirafter") => &mut resolver.idirafter,
                Some("-include") => &mut resolver.forced,
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
    /// come after them, then the `-idirafter` folders
    /// ([`Resolver::after_system`]). A `unit` that is not listed
    /// contributes no folders of its own. A folder `-isystem` names is
    /// left out of the `-I` folders and `unit`'s `include_dirs`: the
    /// compiler searches it at its `-isystem` place only.
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
        let not_system = |dir: &&String| !self.isystem.contains(dir);
        let mut order = Vec::new();
        if quoted {
            order.push(own.to_string());
            order.extend(self.iquote.iter().cloned());
        }
        order.extend(self.dash_i.iter().filter(not_system).cloned());
        order.extend(unit_dirs.iter().filter(not_system).cloned());
        order.extend(self.isystem.iter().cloned());
        order
    }

    /// The configuration's `-idirafter` folders, in order, root-relative:
    /// searched after the system's folders.
    pub fn after_system(&self) -> &[String] {
        &self.idirafter
    }

    /// Where the include `name` written in `including` lands when compiled
    /// as part of the listed file `unit`.
    pub fn find(&self, unit: &str, including: &str, name: &str, quoted: bool) -> Landing {
        if name.is_empty() {
            return Landing::NotProject;
        }
        if name.starts_with('/') {
            return self.take(Path::new(name)).unwrap_or(Landing::NotProject);
        }
        // The including file itself is passed over: a header's
        // `#include_next` of its own name, read as an ordinary include,
        // reaches the next one, as the compiler's does.
        let next = |dir: &str| {
            self.take(&self.root().join(dir).join(name))
                .filter(|l| *l != Landing::Project(including.to_string()))
        };
        if let Some(landing) = self
            .search_order(unit, including, quoted)
            .iter()
            .find_map(|dir| next(dir))
        {
            return landing;
        }
        // After the system: an `-idirafter` folder only when the system
        // has no such name.
        if !self.idirafter.is_empty() && !system_holds(name) {
            if let Some(landing) = self.idirafter.iter().find_map(|dir| next(dir)) {
                return landing;
            }
        }
        Landing::NotProject
    }

    /// The landing of `candidate` when it is a regular file: the compiler
    /// takes it, project C only when confined.
    fn take(&self, candidate: &Path) -> Option<Landing> {
        let real = candidate.canonicalize().ok()?;
        if !real.is_file() {
            return None;
        }
        Some(
            match self
                .confine
                .allows(&real)
                .then(|| self.confine.rel(&real))
                .flatten()
            {
                Some(rel) => Landing::Project(rel),
                None => Landing::Outside(real),
            },
        )
    }

    /// The files the compile of `start` (root-relative, as a scan records
    /// them) reads from the project, `start` included, sorted: the files
    /// the rule reaches from every listed `.c` of `start` (each its own
    /// compile, the configuration's `-include` files first; a `start` with
    /// no listed `.c` is read as no listed file's), joined with the facts'
    /// closure — so a header an ambiguous include lands on is in, and a
    /// header the facts name stays in. Each file's include names come from
    /// `names_of` (`None` adds no includes; a caller may cache it across
    /// units).
    pub fn closure_with(
        &self,
        facts: &Facts,
        start: &[String],
        names_of: &mut NamesOf<'_>,
    ) -> Vec<String> {
        let mut out: BTreeSet<String> = facts.include_closure(start).into_iter().collect();
        // Each listed `.c` of the start is a compile of its own; a header
        // named in the start is read under each of them.
        let mut compiles: Vec<&str> = start
            .iter()
            .filter(|f| f.ends_with(".c"))
            .filter(|c| self.listed.iter().any(|l| l.path == **c))
            .map(String::as_str)
            .collect();
        if compiles.is_empty() {
            compiles.push("");
        }
        for unit in compiles {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut stack: Vec<String> = start.iter().rev().cloned().collect();
            stack.extend(self.forced.iter().rev().cloned());
            while let Some(file) = stack.pop() {
                if !seen.insert(file.clone()) {
                    continue;
                }
                for (name, quoted) in names_of(&file).unwrap_or_default() {
                    if let Some(found) = self.resolve(unit, &file, &name, quoted) {
                        stack.push(found);
                    }
                }
            }
            out.extend(seen);
        }
        out.into_iter().collect()
    }

    /// [`Resolver::closure_with`], each file read as a scan reads it
    /// ([`names_on_disk`]).
    pub fn closure(&self, facts: &Facts, start: &[String]) -> Vec<String> {
        let root = self.root().to_path_buf();
        self.closure_with(facts, start, &mut |rel| names_on_disk(&root, rel))
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

/// The files a unit's compile reads from the project, sorted
/// (docs/PROJECT-MAP-DESIGN.md §3.7; the 2026-10-08 triage, decision 11):
/// for a file-list target, [`Resolver::closure`]; for a folder target (or a
/// file list whose resolver refuses), the facts' include closure as ever.
/// What `unit_source`, the planner's `source_hash` and `state status` hash.
pub fn unit_closure(ctx: &TargetContext, facts: &Facts, start: &[String]) -> Vec<String> {
    match Resolver::of(ctx) {
        Ok(Some(resolver)) => resolver.closure(facts, start),
        _ => facts.include_closure(start),
    }
}

/// Whether one of the usual system include folders of this computer's C
/// compiler holds `name` as a file: what an `-idirafter` folder comes
/// after. The folders are found on disk, never by running a compiler (the
/// SDK's and the toolchain's on macOS; `/usr/local/include`,
/// `/usr/include`, its multiarch folders and the compilers' own on other
/// systems), and read once per process.
pub fn system_holds(name: &str) -> bool {
    system_dirs().iter().any(|dir| dir.join(name).is_file())
}

fn system_dirs() -> &'static [PathBuf] {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        // `<dir>/*/<tail>`: each version folder's include folder.
        let each = |dir: &str, tail: &str| -> Vec<PathBuf> {
            let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path().join(tail))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            found.sort();
            found
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(sdk) = std::env::var_os("SDKROOT") {
            dirs.push(PathBuf::from(sdk).join("usr/include"));
        }
        if cfg!(target_os = "macos") {
            for developer in [
                "/Applications/Xcode.app/Contents/Developer",
                "/Library/Developer/CommandLineTools",
            ] {
                let sdk = if developer.ends_with("CommandLineTools") {
                    format!("{developer}/SDKs/MacOSX.sdk/usr/include")
                } else {
                    format!("{developer}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/usr/include")
                };
                let toolchain = if developer.ends_with("CommandLineTools") {
                    format!("{developer}/usr")
                } else {
                    format!("{developer}/Toolchains/XcodeDefault.xctoolchain/usr")
                };
                dirs.push(PathBuf::from(sdk));
                dirs.push(PathBuf::from(format!("{toolchain}/include")));
                dirs.extend(each(&format!("{toolchain}/lib/clang"), "include"));
            }
        } else {
            dirs.push(PathBuf::from("/usr/local/include"));
            dirs.push(PathBuf::from("/usr/include"));
            dirs.extend(
                std::fs::read_dir("/usr/include")
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| {
                                p.file_name()
                                    .is_some_and(|n| n.to_string_lossy().contains("-linux-"))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
            for triple in each("/usr/lib/gcc", "") {
                dirs.extend(each(&triple.to_string_lossy(), "include"));
            }
            dirs.extend(each("/usr/lib/clang", "include"));
        }
        dirs.retain(|d| d.is_dir());
        dirs
    })
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

/// The include directives' words, longest first: `#include_next` is
/// resolved as an ordinary include (it can only add a file), and `#import`
/// is an include read once.
const INCLUDE_WORDS: [&[u8]; 3] = [b"include_next", b"include", b"import"];

/// What follows `# include` (or `#include_next`, `#import`, with `%:` for
/// `#`) on a line, when the line starts so.
fn include_head(line: &[u8]) -> Option<&[u8]> {
    let line = trim_start(line);
    let rest = line
        .strip_prefix(b"#")
        .or_else(|| line.strip_prefix(b"%:"))?;
    let rest = trim_start(rest);
    INCLUDE_WORDS.iter().find_map(|w| rest.strip_prefix(*w))
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

fn trim_start(s: &[u8]) -> &[u8] {
    let at = s.iter().position(|&b| !is_blank(b)).unwrap_or(s.len());
    &s[at..]
}

/// The bytes with every lone carriage return read as a line end and every
/// backslash-newline removed — a backslash, blanks, then a line end too, as
/// the compiler reads it (the preprocessor's line splicing, which comes
/// before comments are read).
fn splice(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\\' => {
                let mut j = i + 1;
                while j < src.len() && matches!(src[j], b' ' | b'\t' | b'\x0b' | b'\x0c') {
                    j += 1;
                }
                match src.get(j) {
                    Some(b'\n') => i = j + 1,
                    Some(b'\r') if src.get(j + 1) == Some(&b'\n') => i = j + 2,
                    Some(b'\r') => i = j + 1,
                    _ => {
                        out.push(b'\\');
                        i += 1;
                    }
                }
            }
            b'\r' if src.get(i + 1) != Some(&b'\n') => {
                out.push(b'\n');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Where the line being written stands against "`# include` then blanks
/// only": kept as each byte is written, so the `<` rule never searches back
/// through the line (a long line with many `<` costs its length, never its
/// square).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    /// Blanks only so far.
    Start,
    /// `%` written after blanks only (the first half of `%:`).
    Percent,
    /// `#` or `%:` written, then blanks.
    Hash,
    /// The first `n` bytes of a directive word written after the `#`.
    Word(usize),
    /// A whole include word written, then blanks only.
    Include,
    /// Anything else: no include line.
    Other,
}

/// The longest directive word read ([`INCLUDE_WORDS`]).
const MAX_WORD: usize = 12;

impl Head {
    /// The state after `b` is written, `word` holding the directive word's
    /// bytes so far.
    fn feed(self, b: u8, word: &mut [u8; MAX_WORD]) -> Head {
        if b == b'\n' {
            return Head::Start;
        }
        let blank = is_blank(b);
        match self {
            Head::Start if blank => Head::Start,
            Head::Start if b == b'#' => Head::Hash,
            Head::Start if b == b'%' => Head::Percent,
            Head::Percent if b == b':' => Head::Hash,
            Head::Hash if blank => Head::Hash,
            Head::Hash | Head::Word(_) if b.is_ascii_lowercase() || b == b'_' => {
                let n = match self {
                    Head::Word(n) => n,
                    _ => 0,
                };
                if n == MAX_WORD {
                    return Head::Other;
                }
                word[n] = b;
                Head::Word(n + 1)
            }
            Head::Word(n) if blank && INCLUDE_WORDS.contains(&&word[..n]) => Head::Include,
            Head::Include if blank => Head::Include,
            _ => Head::Other,
        }
    }

    /// Whether a `<` written now opens an include's `<name>`.
    fn opens_name(self, word: &[u8; MAX_WORD]) -> bool {
        match self {
            Head::Include => true,
            Head::Word(n) => INCLUDE_WORDS.contains(&&word[..n]),
            _ => false,
        }
    }
}

/// The bytes with every `/* … */` and `// …` comment replaced by one space
/// (newlines inside a block comment kept), string and character literals
/// passed through as they are, and an include's `<name>` passed through
/// whole (`<sys//types.h>` holds no comment).
fn strip_comments(src: &[u8]) -> Vec<u8> {
    struct Out {
        bytes: Vec<u8>,
        head: Head,
        word: [u8; MAX_WORD],
    }
    impl Out {
        fn push(&mut self, b: u8) {
            self.bytes.push(b);
            self.head = self.head.feed(b, &mut self.word);
        }
    }
    let mut out = Out {
        bytes: Vec::with_capacity(src.len()),
        head: Head::Start,
        word: [0; MAX_WORD],
    };
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'<' if out.head.opens_name(&out.word) => {
                while i < src.len() && src[i] != b'\n' {
                    out.push(src[i]);
                    i += 1;
                    if out.bytes.last() == Some(&b'>') {
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
    out.bytes
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
            // The forms the 2026-10-08 check found unread (each checked
            // against `cc -M` by harness-oracle's
            // `the_reader_reads_every_form_cc_reads`).
            (b"#import \"i.h\"\n", vec![q("i.h")]),
            (b"#include_next <n.h>\n", vec![a("n.h")]),
            (b"%:include \"d.h\"\n", vec![q("d.h")]),
            (b"%: include <d2.h>\n", vec![a("d2.h")]),
            (b"int x;\r#include \"r.h\"\rint y;\r", vec![q("r.h")]),
            (b"#inc\\  \nlude \"s.h\"\n", vec![q("s.h")]),
            (b"#include \\ \t\n\"s2.h\"\n", vec![q("s2.h")]),
            (b"#include_nextx <no.h>\n#includes <no2.h>\n", vec![]),
            (
                b"#include_next<n2.h>\n#import<i2.h>\n",
                vec![a("i2.h"), a("n2.h")],
            ),
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

    /// The `<` rule keeps where the line stands as it writes it: a 1 MiB
    /// line full of `<` (and one led by 1 MiB of blanks) reads in well under
    /// a second, where searching back for the line's start took minutes.
    #[test]
    fn a_long_line_full_of_angle_brackets_reads_in_linear_time() {
        let mut line = b"x=a<b;".repeat((1 << 20) / 6);
        line.extend_from_slice(b"\n#include <ok.h>\n");
        let mut blanks = vec![b' '; 1 << 20];
        blanks.extend(std::iter::repeat_n(b'<', 1 << 19));
        blanks.extend_from_slice(b"\n#  \t<x>\n");
        let mut hashed = b"#".to_vec();
        hashed.extend(vec![b' '; 1 << 20]);
        hashed.extend(std::iter::repeat_n(b'<', 1 << 19));
        let started = std::time::Instant::now();
        assert_eq!(include_names(&line), vec![a("ok.h")]);
        assert_eq!(include_names(&blanks), vec![]);
        assert_eq!(include_names(&hashed), vec![]);
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_secs(1),
            "three 1 MiB lines took {took:?}"
        );
    }

    #[test]
    fn the_skip_note_names_no_machine_path() {
        let abs = Path::new("/home/me/proj/src/a.c");
        let io = Error::io(
            abs,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        let note = unreadable_note(abs, &io);
        assert_eq!(
            note,
            "cannot be read: permission denied; recorded as unreadable"
        );
        let plain = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(unreadable_note(abs, &plain), note);
        let refused = Error::Invariant(format!("{}: not a regular file", abs.display()));
        assert_eq!(
            unreadable_note(abs, &refused),
            "cannot be read: not a regular file; recorded as unreadable"
        );
    }

    #[test]
    fn a_dot_folder_is_seen_in_any_part_but_the_file() {
        assert!(in_dot_folder(".env/x.h"));
        assert!(in_dot_folder("src/.git/x.h"));
        assert!(!in_dot_folder("src/.x.h"));
        assert!(!in_dot_folder("src/x.h"));
    }
}
