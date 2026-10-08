//! The project map file, `migration/map/project-map.json`
//! (`ruharness-project-map` v1, docs/PROJECT-MAP-DESIGN.md §3.3,
//! docs/SCHEMAS.md "The project map file"): the per-file facts, the
//! configuration and the toolchain, and — when no cap was hit — the
//! programs, their closures with the link checks of §3.5, shared files,
//! libraries and duplicates between programs.
//!
//! Facts and summaries only, **no source text**. Every string from the
//! project (a path, a header name, a symbol name) is stored raw, so it can be
//! matched exactly; a caller filters it for a terminal. Flags and include
//! folders keep their order; every other list is sorted by its first field's
//! bytes, so the same project gives the same bytes on every run.
//!
//! `root_hash` is the file-set hash (docs/SCHEMAS.md "Hashes") of every
//! walked `.c`/`.h`, every `included_other` file and `compile_commands.json`
//! when one was read; `inputs_hash` is blake3 of the canonical JSON (keys
//! sorted, no blanks) of `{configuration: <configuration.digest>,
//! toolchain: {cc, cflags, system_include_dirs, target}}`.

use super::closure::{self, Analysis, Input, Linked};
use super::evidence::{CompileCommands, PATH_PREFIXES};
use super::{Compiled, FileKind, FolderMap, Toolchain};
use harness_core::config::flags::{check_flag, Flag};
use harness_core::error::Error;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The map file, relative to the project root.
pub const MAP_FILE: &str = "migration/map/project-map.json";
/// The `schema` field.
pub const SCHEMA: &str = "ruharness-project-map";
/// The `schema_version` field.
pub const SCHEMA_VERSION: u32 = 1;
/// The project-level `.gitignore` the first map writes, relative to the
/// root.
pub const GITIGNORE: &str = "migration/.gitignore";
/// The names `migration/.gitignore` ignores: the ledgers' scratch and the
/// map's lock and reply (docs/PROJECT-MAP-DESIGN.md §3.7).
pub const IGNORED_NAMES: &[&str] = &[
    "build/",
    ".lock",
    "traces/",
    ".promote-*/",
    ".*.prev/",
    ".replay-*/",
    "target/",
    ".ruharness-adopted",
    "map/.lock",
    "map/project-map.reply.json",
];

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MapFile {
    /// [`SCHEMA`].
    pub schema: &'static str,
    /// [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// See the module docs.
    pub root_hash: String,
    /// See the module docs.
    pub inputs_hash: String,
    /// The toolchain every compile ran under.
    pub toolchain: ToolchainRec,
    /// The configuration.
    pub configuration: ConfigurationRec,
    /// Every walked `.c` and `.h`.
    pub files: Vec<FileRec>,
    /// The programs (none past a cap).
    pub programs: Vec<ProgramRec>,
    /// Files whose parser facts define `main` but that did not compile.
    pub programs_not_compiled: Vec<String>,
    /// One per `main` and `fuzz` program (none past a cap).
    pub closures: Vec<ClosureRec>,
    /// Symbols defined by files that never meet in one closure.
    pub between_program_duplicates: Vec<SymDefiners>,
    /// Files in two or more closures.
    pub shared: Vec<SharedRec>,
    /// Groups of unreached `.c` files.
    pub libraries: Vec<LibraryRec>,
    /// Non-C files counted per folder and language.
    pub set_aside: Vec<SetAsideRec>,
    /// Folders the walk skipped.
    pub skipped_folders: Vec<SkippedRec>,
    /// Entries the walk did not walk.
    pub walk_issues: Vec<WalkIssueRec>,
    /// What the project's build says.
    pub build_evidence: EvidenceRec,
    /// The caps reached: while any is, there are no closures.
    pub limits_hit: Vec<LimitRec>,
}

/// `toolchain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolchainRec {
    /// The first line of `cc --version`.
    pub cc: String,
    /// `cc -dumpmachine`.
    pub target: String,
    /// The judge's base flags, in order.
    pub cflags: Vec<String>,
    /// The compiler's own include folders, in order.
    pub system_include_dirs: Vec<String>,
}

/// `configuration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigurationRec {
    /// Its name.
    pub name: String,
    /// What it stands for.
    pub from: harness_core::config::ConfigurationFrom,
    /// `compile_commands`, `stated` or `guessed` (a closure whose files'
    /// entry flags differ keeps it `guessed`).
    pub source: &'static str,
    /// Its flags, in order.
    pub flags: Vec<String>,
    /// Header names meant as the system's, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub system_headers: Vec<String>,
    /// blake3 of the canonical JSON of `{flags, from, name}`.
    pub digest: String,
}

/// One walked file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRec {
    /// Relative to the root.
    pub path: String,
    /// Its other paths through links.
    pub aliases: Vec<String>,
    /// `c` or `h`.
    pub kind: &'static str,
    /// Its size.
    pub bytes: u64,
    /// `blake3:<hex>` of its bytes.
    pub blake3: String,
    /// The scanner read it.
    pub parsed: bool,
    /// Over the read cap.
    #[serde(skip_serializing_if = "is_false")]
    pub too_large: bool,
    /// Not UTF-8.
    #[serde(skip_serializing_if = "is_false")]
    pub not_utf8: bool,
    /// A `.c`'s compile: `"ok"` or the closed reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled: Option<CompiledRec>,
    /// The compile read outside the root and the toolchain.
    #[serde(skip_serializing_if = "is_false")]
    pub outside_includes: bool,
    /// Functions the scanner found defined.
    pub functions: usize,
    /// Project files its includes reach directly.
    pub includes: Vec<String>,
    /// Its include folders, in order.
    pub include_dirs: Vec<String>,
    /// Its ambiguous includes (and its headers').
    pub ambiguous_includes: Vec<AmbiguousRec>,
    /// Files whose includes reach it directly.
    pub included_by: Vec<String>,
    /// Files inside the root the compile read that the walk did not record.
    pub included_other: Vec<String>,
    /// External symbols its object defines.
    pub defined_symbols: Vec<DefinedRec>,
    /// External symbols its object needs.
    pub needed_symbols: Vec<NeededRec>,
    /// Names not shaped like an identifier, counted.
    pub odd_names: usize,
    /// Names withheld because of `outside_includes`, counted.
    #[serde(skip_serializing_if = "is_zero")]
    pub withheld_names: usize,
}

/// A compile: the word `ok`, or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CompiledRec {
    /// `"ok"`.
    Ok(&'static str),
    /// The closed reason.
    Failed {
        /// `missing-header`, `syntax` or `other`.
        reason: &'static str,
        /// The missing header's name, only when it is a clean relative path.
        #[serde(skip_serializing_if = "Option::is_none")]
        header: Option<String>,
        /// A fixed word for `other`.
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<&'static str>,
        /// `<relative path>:<line>`.
        #[serde(skip_serializing_if = "Option::is_none")]
        at: Option<String>,
    },
}

/// An ambiguous include.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct AmbiguousRec {
    /// The name as written.
    pub header: String,
    /// Every candidate (`system` for the system's).
    pub candidates: Vec<String>,
    /// The one the compile read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<String>,
}

/// A defined symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefinedRec {
    /// The C name.
    pub name: String,
    /// `function`, `data`, `read-only`, `bss` or `common`.
    pub kind: &'static str,
    /// A weak definition.
    #[serde(skip_serializing_if = "is_false")]
    pub weak: bool,
}

/// A needed symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeededRec {
    /// The C name.
    pub name: String,
    /// A weak reference.
    #[serde(skip_serializing_if = "is_false")]
    pub weak: bool,
}

/// A program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramRec {
    /// `t-…`.
    pub id: String,
    /// `p1…` for a `main` program.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    /// Its file.
    pub path: String,
    /// `main`, `fuzz` or `driver`.
    pub kind: &'static str,
    /// The folder-name guess: `tool`, `test`, `example` or `benchmark`.
    pub kind_guess: &'static str,
    /// A driver's fuzzers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub serves: Vec<String>,
}

/// One program's closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosureRec {
    /// The program's id.
    pub program: String,
    /// Its files.
    pub files: Vec<String>,
    /// The flags every `.c` of it compiles with, in order, when they agree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<Vec<String>>,
    /// Each `.c` with its own flags, when they do not agree.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flags_differ: Vec<PathFlags>,
    /// Any reason below.
    pub incomplete: bool,
    /// Why it is incomplete.
    pub incomplete_why: Vec<IncompleteRec>,
    /// Needed symbols no project file defines.
    pub outside: Vec<String>,
    /// Needs met only by another program's file.
    pub needs_from: Vec<NeedsFromRec>,
    /// Its duplicate sets.
    pub duplicates: Vec<DuplicateRec>,
    /// Symbols two of its files define strongly.
    pub collisions: Vec<SymDefiners>,
    /// Its files' ambiguous includes the configuration does not settle.
    pub ambiguous_unsettled: Vec<AmbiguousRec>,
    /// The link check: `"ok"` or what failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked: Option<LinkedRec>,
    /// The held sets: questions for the person.
    pub questions: Vec<String>,
}

/// A file and its flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathFlags {
    /// The file.
    pub path: String,
    /// Its flags, in order.
    pub flags: Vec<String>,
}

/// One reason a closure is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncompleteRec {
    /// `pending`, `may-be-defined-in`, `unread` or `unreadable-folder`.
    pub why: &'static str,
    /// The file or folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The symbols.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
}

/// A need met by another program's file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedsFromRec {
    /// The symbol.
    pub sym: String,
    /// The program's id.
    pub program: String,
}

/// A duplicate set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateRec {
    /// `d1…`, once per project.
    pub set: String,
    /// This closure's symbols the set defines.
    pub symbols: Vec<String>,
    /// The definers.
    pub definers: Vec<DefinerRec>,
    /// The definers whose choice linked.
    pub links: Vec<String>,
    /// Settled by linking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<ChoiceRec>,
    /// The choice it is reached only under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub under: Option<String>,
}

/// A definer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefinerRec {
    /// `d1.1…`.
    pub index: String,
    /// Its file.
    pub path: String,
}

/// A settled choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceRec {
    /// The kept definer's index.
    pub keep: String,
    /// `links`.
    pub by: &'static str,
}

/// A symbol and its definers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymDefiners {
    /// The symbol.
    pub sym: String,
    /// Its definers.
    pub definers: Vec<String>,
}

/// A link check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LinkedRec {
    /// `"ok"`.
    Ok(&'static str),
    /// What failed.
    Failed {
        /// Symbols nothing linked provides.
        missing: Vec<String>,
        /// Symbols two linked files define strongly.
        doubled: Vec<String>,
    },
}

/// A shared file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedRec {
    /// The file.
    pub file: String,
    /// The programs' ids.
    pub programs: Vec<String>,
}

/// A library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryRec {
    /// `l-…`.
    pub id: String,
    /// Its files.
    pub files: Vec<String>,
    /// Files outside it that it needs.
    pub needs_from_outside: Vec<String>,
}

/// Set-aside files of one folder and language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetAsideRec {
    /// The folder.
    pub folder: String,
    /// The language.
    pub lang: &'static str,
    /// Its files.
    pub count: usize,
}

/// A skipped folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedRec {
    /// The folder.
    pub path: String,
    /// Its `.c`/`.h` files.
    pub count: usize,
    /// `count` is a lower bound.
    #[serde(skip_serializing_if = "is_false")]
    pub at_least: bool,
}

/// A walk issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WalkIssueRec {
    /// The entry.
    pub path: String,
    /// Why, in words.
    pub why: String,
}

/// `build_evidence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceRec {
    /// `present`, `absent` or `unreadable`.
    pub compile_commands: &'static str,
    /// The file read (or found and not read), relative to the root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_commands_path: Option<String>,
    /// Entries not used.
    pub ignored_entries: usize,
    /// Files entries name that the walk did not find.
    pub unfound_entries: Vec<String>,
    /// Build files by name.
    pub build_files: Vec<String>,
    /// Files listed twice with different flags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flags_differ: Vec<ListedTwice>,
}

/// A file a `compile_commands.json` lists with different flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedTwice {
    /// The file.
    pub path: String,
    /// Each distinct list of flags, in the file's order.
    pub flags: Vec<Vec<String>>,
}

/// A cap reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LimitRec {
    /// `files`, `depth`, `symbols` or `budget`.
    pub limit: &'static str,
    /// The cap, in words.
    pub at: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A missing header's name worth keeping: a clean relative path (no `/`
/// first, no `..` or empty part, no control character). An absolute name
/// can be a machine path (docs/PROJECT-MAP-DESIGN.md §3.1 step 5).
pub fn clean_header(name: &str) -> Option<&str> {
    let clean = !name.is_empty()
        && !name.starts_with('/')
        && !name.chars().any(char::is_control)
        && name.split('/').all(|p| !p.is_empty() && p != "..");
    clean.then_some(name)
}

/// The configuration's flags as the link checks pass them to `cc`: a path
/// made absolute under `root` (`-I` joined, the others as two arguments, as
/// every map compile writes them), `-O` levels dropped (recorded, never
/// applied).
pub fn link_flags(root: &Path, flags: &[String]) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    for flag in flags {
        match check_flag(flag).map_err(Error::Invariant)? {
            Flag::Optimization => {}
            Flag::Path(_) => {
                let (prefix, rel) = PATH_PREFIXES
                    .iter()
                    .find_map(|p| flag.strip_prefix(p).map(|rest| (*p, rest)))
                    .ok_or_else(|| Error::Invariant(format!("unknown path flag {flag}")))?;
                let abs = crate::path_str(&root.join(rel))?.to_string();
                if prefix == "-I" {
                    out.push(format!("-I{abs}"));
                } else {
                    out.extend([prefix.to_string(), abs]);
                }
            }
            _ => out.push(flag.clone()),
        }
    }
    Ok(out)
}

/// The analysis of a map, with its link checks (§3.5): `None` past a cap,
/// when no closure may be computed (§3.10).
pub fn analyze(map: &FolderMap) -> Result<Option<Analysis>, Error> {
    if !map.closures_possible() {
        return Ok(None);
    }
    let input = Input {
        files: &map.files,
        parser: &map.parser,
        walk_issues: &map.walk_issues,
        // Accepted tools keep their ids once `project accept` writes them
        // (step e); none are read yet.
        accepted: &[],
    };
    let flags = link_flags(&map.root, &map.configuration.flags)?;
    super::link::analyze_linked(&map.root, &input, &flags).map(Some)
}

/// `root_hash`: see the module docs.
pub fn root_hash(map: &FolderMap) -> Result<String, Error> {
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    for f in &map.files {
        pairs.insert(f.path.clone(), f.blake3.clone());
    }
    let mut others: BTreeSet<&str> = BTreeSet::new();
    for f in &map.files {
        others.extend(f.included_other.iter().map(String::as_str));
    }
    if let CompileCommands::Present { path } = &map.evidence.compile_commands {
        others.insert(path);
    }
    for rel in others {
        if !pairs.contains_key(rel) {
            let hash = harness_core::hash::file_hash(&map.root.join(rel))?;
            pairs.insert(rel.to_string(), hash);
        }
    }
    let pairs: Vec<(String, String)> = pairs.into_iter().collect();
    Ok(harness_core::hash::file_set_hash(&pairs))
}

/// `inputs_hash`: see the module docs.
pub fn inputs_hash(configuration_digest: &str, toolchain: &ToolchainRec) -> String {
    // Fields in key order: serde writes them as declared.
    #[derive(Serialize)]
    struct Tool<'a> {
        cc: &'a str,
        cflags: &'a [String],
        system_include_dirs: &'a [String],
        target: &'a str,
    }
    #[derive(Serialize)]
    struct Canonical<'a> {
        configuration: &'a str,
        toolchain: Tool<'a>,
    }
    let json = serde_json::to_vec(&Canonical {
        configuration: configuration_digest,
        toolchain: Tool {
            cc: &toolchain.cc,
            cflags: &toolchain.cflags,
            system_include_dirs: &toolchain.system_include_dirs,
            target: &toolchain.target,
        },
    })
    .unwrap_or_else(|_| unreachable!("strings always serialise"));
    harness_core::hash::bytes_hash(&json)
}

fn toolchain_rec(t: &Toolchain) -> ToolchainRec {
    ToolchainRec {
        cc: t.cc.clone(),
        target: t.target.clone(),
        cflags: t.cflags.clone(),
        system_include_dirs: t
            .system_include_dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect(),
    }
}

fn compiled_rec(c: &Compiled) -> CompiledRec {
    match c {
        Compiled::Ok => CompiledRec::Ok("ok"),
        Compiled::Failed {
            reason,
            header,
            detail,
            at,
        } => CompiledRec::Failed {
            reason: reason.as_str(),
            header: header.as_deref().and_then(clean_header).map(str::to_string),
            detail: *detail,
            at: at.clone(),
        },
    }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

/// An ambiguous include is settled when the configuration names the header
/// as the system's, or one of its `-I`, `-iquote` or `-isystem` folders
/// holds the candidate.
fn settled(map: &FolderMap, a: &super::Ambiguous) -> bool {
    let c = &map.configuration;
    if c.system_headers.contains(&a.header) {
        return true;
    }
    c.flags.iter().any(|flag| {
        ["-isystem", "-iquote", "-I"].iter().any(|p| {
            flag.strip_prefix(p).is_some_and(|dir| {
                let held = if dir == "." {
                    a.header.clone()
                } else {
                    format!("{}/{}", dir.trim_end_matches('/'), a.header)
                };
                a.candidates.contains(&held)
            })
        })
    })
}

/// The map file for `map` and its analysis (`None` past a cap: the file
/// facts only).
pub fn render(map: &FolderMap, analysis: Option<&Analysis>) -> Result<MapFile, Error> {
    let toolchain = toolchain_rec(&map.toolchain);
    let by_path: BTreeMap<&str, &super::FileFacts> =
        map.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut included_by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in &map.files {
        for inc in &f.includes {
            included_by.entry(inc).or_default().insert(&f.path);
        }
    }
    let mut files: Vec<FileRec> = map
        .files
        .iter()
        .map(|f| {
            let mut ambiguous: Vec<AmbiguousRec> = f
                .ambiguous
                .iter()
                .map(|a| AmbiguousRec {
                    header: a.header.clone(),
                    candidates: a.candidates.clone(),
                    used: a.used.clone(),
                })
                .collect();
            ambiguous.sort();
            FileRec {
                path: f.path.clone(),
                aliases: sorted(f.aliases.clone()),
                kind: f.kind.as_str(),
                bytes: f.bytes,
                blake3: f.blake3.clone(),
                parsed: f.parsed,
                too_large: f.too_large,
                not_utf8: f.not_utf8,
                compiled: f.compiled.as_ref().map(compiled_rec),
                outside_includes: f.outside_includes,
                functions: f.functions,
                includes: sorted(f.includes.clone()),
                include_dirs: f.include_dirs.clone(),
                ambiguous_includes: ambiguous,
                included_by: included_by
                    .get(f.path.as_str())
                    .map(|s| s.iter().map(|p| p.to_string()).collect())
                    .unwrap_or_default(),
                included_other: sorted(f.included_other.clone()),
                defined_symbols: {
                    let mut d: Vec<DefinedRec> = f
                        .defined
                        .iter()
                        .map(|d| DefinedRec {
                            name: d.name.clone(),
                            kind: d.kind,
                            weak: d.weak,
                        })
                        .collect();
                    d.sort_by(|a, b| a.name.cmp(&b.name));
                    d
                },
                needed_symbols: {
                    let mut n: Vec<NeededRec> = f
                        .needed
                        .iter()
                        .map(|n| NeededRec {
                            name: n.name.clone(),
                            weak: n.weak,
                        })
                        .collect();
                    n.sort_by(|a, b| a.name.cmp(&b.name));
                    n
                },
                odd_names: f.odd_names,
                withheld_names: f.withheld_names,
            }
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut source = map.configuration.source.as_str();
    let mut programs = Vec::new();
    let mut programs_not_compiled = Vec::new();
    let mut closures = Vec::new();
    let mut between = Vec::new();
    let mut shared = Vec::new();
    let mut libraries = Vec::new();
    if let Some(a) = analysis {
        programs = a
            .programs
            .iter()
            .map(|p| ProgramRec {
                id: p.id.clone(),
                index: p.index.clone(),
                path: p.path.clone(),
                kind: p.kind.as_str(),
                kind_guess: p.kind_guess.as_str(),
                serves: sorted(p.serves.clone()),
            })
            .collect();
        programs.sort_by(|a: &ProgramRec, b| a.id.cmp(&b.id));
        programs_not_compiled = sorted(a.programs_not_compiled.clone());
        for c in &a.closures {
            let rec = closure_rec(map, &by_path, c);
            if !rec.flags_differ.is_empty() {
                source = super::ConfigSource::Guessed.as_str();
            }
            closures.push(rec);
        }
        closures.sort_by(|a: &ClosureRec, b| a.program.cmp(&b.program));
        between = a
            .between_program_duplicates
            .iter()
            .map(|d| SymDefiners {
                sym: d.sym.clone(),
                definers: sorted(d.definers.clone()),
            })
            .collect();
        between.sort_by(|a: &SymDefiners, b| a.sym.cmp(&b.sym));
        shared = a
            .shared
            .iter()
            .map(|s| SharedRec {
                file: s.file.clone(),
                programs: sorted(s.programs.clone()),
            })
            .collect();
        shared.sort_by(|a: &SharedRec, b| a.file.cmp(&b.file));
        libraries = a
            .libraries
            .iter()
            .map(|l| LibraryRec {
                id: l.id.clone(),
                files: sorted(l.files.clone()),
                needs_from_outside: sorted(l.needs_from_outside.clone()),
            })
            .collect();
        libraries.sort_by(|a: &LibraryRec, b| a.id.cmp(&b.id));
    }

    let ev = &map.evidence;
    let (cc_word, cc_path) = match &ev.compile_commands {
        CompileCommands::Absent => ("absent", None),
        CompileCommands::Present { path } => ("present", Some(path.clone())),
        CompileCommands::Unreadable { path, .. } => ("unreadable", Some(path.clone())),
    };
    let c = &map.configuration;
    let mut set_aside: Vec<SetAsideRec> = map
        .set_aside
        .iter()
        .map(|s| SetAsideRec {
            folder: s.folder.clone(),
            lang: s.lang,
            count: s.count,
        })
        .collect();
    set_aside.sort_by(|a, b| (&a.folder, a.lang).cmp(&(&b.folder, b.lang)));
    let mut skipped: Vec<SkippedRec> = map
        .skipped_folders
        .iter()
        .map(|s| SkippedRec {
            path: s.path.clone(),
            count: s.files,
            at_least: !s.complete,
        })
        .collect();
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    let mut walk_issues: Vec<WalkIssueRec> = map
        .walk_issues
        .iter()
        .map(|w| WalkIssueRec {
            path: w.path.clone(),
            why: w.why.clone(),
        })
        .collect();
    walk_issues.sort_by(|a, b| (&a.path, &a.why).cmp(&(&b.path, &b.why)));
    let mut flags_differ: Vec<ListedTwice> = ev
        .flags_differ
        .iter()
        .map(|f| ListedTwice {
            path: f.path.clone(),
            flags: f.flags.clone(),
        })
        .collect();
    flags_differ.sort_by(|a, b| a.path.cmp(&b.path));
    let mut limits_hit: Vec<LimitRec> = map
        .limits_hit
        .iter()
        .map(|l| LimitRec {
            limit: l.limit,
            at: l.at.clone(),
        })
        .collect();
    limits_hit.sort_by(|a, b| a.limit.cmp(b.limit));

    Ok(MapFile {
        schema: SCHEMA,
        schema_version: SCHEMA_VERSION,
        root_hash: root_hash(map)?,
        inputs_hash: inputs_hash(&c.digest, &toolchain),
        toolchain,
        configuration: ConfigurationRec {
            name: c.name.clone(),
            from: c.from,
            source,
            flags: c.flags.clone(),
            system_headers: c.system_headers.clone(),
            digest: c.digest.clone(),
        },
        files,
        programs,
        programs_not_compiled,
        closures,
        between_program_duplicates: between,
        shared,
        libraries,
        set_aside,
        skipped_folders: skipped,
        walk_issues,
        build_evidence: EvidenceRec {
            compile_commands: cc_word,
            compile_commands_path: cc_path,
            ignored_entries: ev.ignored_entries,
            unfound_entries: sorted(ev.unfound_entries.clone()),
            build_files: sorted(ev.build_files.clone()),
            flags_differ,
        },
        limits_hit,
    })
}

fn closure_rec(
    map: &FolderMap,
    by_path: &BTreeMap<&str, &super::FileFacts>,
    c: &closure::Closure,
) -> ClosureRec {
    // The flags each `.c` compiles with: one list when they agree, else
    // each file's (a flags-differ fact that keeps the source guessed).
    let mut per_file: Vec<PathFlags> = c
        .files
        .iter()
        .filter_map(|p| by_path.get(p.as_str()))
        .filter(|f| f.kind == FileKind::C)
        .map(|f| PathFlags {
            path: f.path.clone(),
            flags: f.flags.clone(),
        })
        .collect();
    per_file.sort_by(|a, b| a.path.cmp(&b.path));
    let agree = per_file.windows(2).all(|w| w[0].flags == w[1].flags);
    let (flags, flags_differ) = if agree {
        (
            Some(
                per_file
                    .first()
                    .map(|f| f.flags.clone())
                    .unwrap_or_else(|| map.configuration.flags.clone()),
            ),
            Vec::new(),
        )
    } else {
        (None, per_file)
    };
    let mut unsettled: BTreeSet<AmbiguousRec> = BTreeSet::new();
    for p in &c.files {
        if let Some(f) = by_path.get(p.as_str()) {
            for a in f.ambiguous.iter().filter(|a| !settled(map, a)) {
                unsettled.insert(AmbiguousRec {
                    header: a.header.clone(),
                    candidates: a.candidates.clone(),
                    used: a.used.clone(),
                });
            }
        }
    }
    let mut incomplete_why: Vec<IncompleteRec> = c
        .incomplete_why
        .iter()
        .map(|i| IncompleteRec {
            why: i.why.as_str(),
            path: i.path.clone(),
            symbols: sorted(i.symbols.clone()),
        })
        .collect();
    incomplete_why.sort_by(|a, b| (a.why, &a.path, &a.symbols).cmp(&(b.why, &b.path, &b.symbols)));
    let mut needs_from: Vec<NeedsFromRec> = c
        .needs_from
        .iter()
        .map(|n| NeedsFromRec {
            sym: n.sym.clone(),
            program: n.program.clone(),
        })
        .collect();
    needs_from.sort_by(|a, b| (&a.sym, &a.program).cmp(&(&b.sym, &b.program)));
    let mut duplicates: Vec<DuplicateRec> = c
        .duplicates
        .iter()
        .map(|d| {
            let mut definers: Vec<DefinerRec> = d
                .definers
                .iter()
                .map(|x| DefinerRec {
                    index: x.index.clone(),
                    path: x.path.clone(),
                })
                .collect();
            definers.sort_by(|a, b| a.index.cmp(&b.index));
            DuplicateRec {
                set: d.set.clone(),
                symbols: sorted(d.symbols.clone()),
                definers,
                links: sorted(d.links.clone()),
                choice: d.choice.as_ref().map(|ch| ChoiceRec {
                    keep: ch.keep.clone(),
                    by: ch.by,
                }),
                under: d.under.clone(),
            }
        })
        .collect();
    duplicates.sort_by(|a, b| a.set.cmp(&b.set));
    let mut collisions: Vec<SymDefiners> = c
        .collisions
        .iter()
        .map(|x| SymDefiners {
            sym: x.sym.clone(),
            definers: sorted(x.definers.clone()),
        })
        .collect();
    collisions.sort_by(|a, b| a.sym.cmp(&b.sym));
    ClosureRec {
        program: c.program.clone(),
        files: sorted(c.files.clone()),
        flags,
        flags_differ,
        incomplete: c.incomplete,
        incomplete_why,
        outside: sorted(c.outside.clone()),
        needs_from,
        duplicates,
        collisions,
        ambiguous_unsettled: unsettled.into_iter().collect(),
        linked: c.linked.as_ref().map(|l| match l {
            Linked::Ok => LinkedRec::Ok("ok"),
            Linked::Failed { missing, doubled } => LinkedRec::Failed {
                missing: sorted(missing.clone()),
                doubled: sorted(doubled.clone()),
            },
        }),
        questions: sorted(c.questions.clone()),
    }
}

/// The file's bytes: pretty JSON (two-space indent) and a final newline.
pub fn to_bytes(file: &MapFile) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(file)
        .unwrap_or_else(|_| unreachable!("strings, numbers and lists always serialise"));
    bytes.push(b'\n');
    bytes
}

/// Write the map file under `root` in full (atomically): its path.
pub fn write(root: &Path, file: &MapFile) -> Result<PathBuf, Error> {
    let path = root.join(MAP_FILE);
    harness_core::ledger::write_atomic(&path, &to_bytes(file))?;
    Ok(path)
}

/// The text of `migration/.gitignore`.
pub fn gitignore_text() -> String {
    let mut text = String::from(
        "# Written by `harness project map` once; edit freely, it is never overwritten.\n\
         # The harness's scratch: build folders, locks, traces and the map's reply.\n",
    );
    for name in IGNORED_NAMES {
        text.push_str(name);
        text.push('\n');
    }
    text
}

/// Write `migration/.gitignore` when there is none (never overwriting one,
/// never through a link): true when it was written.
pub fn write_gitignore(root: &Path) -> Result<bool, Error> {
    use std::io::Write;
    let path = root.join(GITIGNORE);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(false);
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(Error::io(&path, e)),
    };
    file.write_all(gitignore_text().as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| Error::io(&path, e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_header_is_kept_only_as_a_clean_relative_path() {
        assert_eq!(clean_header("proj/api.h"), Some("proj/api.h"));
        assert_eq!(clean_header("x.h"), Some("x.h"));
        for bad in [
            "/Users/me/x.h",
            "../x.h",
            "a/../b.h",
            "a//b.h",
            "",
            "a\nb.h",
        ] {
            assert_eq!(clean_header(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn inputs_hash_moves_with_the_configuration_and_the_toolchain() {
        let t = ToolchainRec {
            cc: "cc 1".into(),
            target: "x86_64".into(),
            cflags: vec!["-O2".into()],
            system_include_dirs: vec!["/usr/include".into()],
        };
        let base = inputs_hash("blake3:aa", &t);
        assert_eq!(base, inputs_hash("blake3:aa", &t.clone()));
        assert_ne!(base, inputs_hash("blake3:bb", &t));
        let mut other = t.clone();
        other.cc = "cc 2".into();
        assert_ne!(base, inputs_hash("blake3:aa", &other));
        let mut other = t.clone();
        other.system_include_dirs.push("/opt/include".into());
        assert_ne!(base, inputs_hash("blake3:aa", &other));
    }

    #[test]
    fn link_flags_make_paths_absolute_and_drop_optimisation_levels() {
        let root = Path::new("/p");
        let flags: Vec<String> = ["-DX=1", "-O2", "-Iinc", "-isystemsys", "-pthread"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            link_flags(root, &flags).unwrap(),
            ["-DX=1", "-I/p/inc", "-isystem", "/p/sys", "-pthread"]
        );
    }

    #[test]
    fn the_gitignore_is_written_once_and_never_overwritten() {
        let root = std::env::temp_dir().join(format!(
            "ruharness-mapfile-gitignore-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(root.join("migration")).unwrap();
        assert!(write_gitignore(&root).unwrap());
        let text = std::fs::read_to_string(root.join(GITIGNORE)).unwrap();
        for name in IGNORED_NAMES {
            assert!(text.lines().any(|l| l == *name), "{name}");
        }
        std::fs::write(root.join(GITIGNORE), "mine\n").unwrap();
        assert!(!write_gitignore(&root).unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join(GITIGNORE)).unwrap(),
            "mine\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
