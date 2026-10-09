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
use super::evidence::CompileCommands;
use super::link::LinkSetup;
use super::{Compiled, FileKind, FolderMap, Toolchain};
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
    /// Per accepted tool whose map digests differ from this map's: what
    /// changed since it was accepted ([`what_changed`]), filled in by the
    /// command that writes the file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accepted_tools: Vec<ToolRecord>,
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
    /// blake3 of the canonical JSON of `{flags, from, name, system_headers}`
    /// (`system_headers` only when not empty).
    pub digest: String,
    /// The `config.toml` entry came with the project and is only proposed
    /// (its source is then `guessed`).
    #[serde(skip_serializing_if = "is_false")]
    pub proposed: bool,
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
    /// Symbols its files define weakly (or as common) and strongly: the
    /// strong one defines it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strong_over_weak: Vec<StrongOverWeakRec>,
    /// Its `.c` files another file includes as text.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub included_as_text: Vec<IncludedRec>,
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
    /// The choices it is reached under, when not every choice reaches it;
    /// once settled, the kept one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub under: Vec<String>,
}

/// A symbol defined weakly and strongly in one closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StrongOverWeakRec {
    /// The symbol.
    pub sym: String,
    /// The files defining it weakly or as common.
    pub weak: Vec<String>,
    /// The files defining it strongly.
    pub strong: Vec<String>,
}

/// A `.c` another file includes as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncludedRec {
    /// The `.c`.
    pub file: String,
    /// The files including it.
    pub by: Vec<String>,
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
        /// Unresolved symbols the probe budget left undecided.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        not_checked: Vec<String>,
        /// Files that did not compile for the link.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        not_compiled: Vec<String>,
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
    /// Walked `.c` files no entry lists.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_in_compile_commands: Vec<String>,
    /// Other `compile_commands.json` files one level down, not read.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also_found: Vec<String>,
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

/// The analysis of a map, with its link checks (§3.5): `None` past a cap,
/// when no closure may be computed (§3.10). When the time budget runs out
/// during the link checks, the `budget` limit is added to `map` and `None`
/// returned.
pub fn analyze(map: &mut FolderMap) -> Result<Option<Analysis>, Error> {
    if !map.closures_possible() {
        return Ok(None);
    }
    // A program (or library) at the path of an accepted tool keeps its id.
    let accepted = accepted_ids(&map.root, &map.files);
    let input = Input {
        files: &map.files,
        parser: &map.parser,
        walk_issues: &map.walk_issues,
        accepted: &accepted,
    };
    let setup = LinkSetup {
        walked: map.files.iter().map(|f| f.path.as_str()).collect(),
        system_headers: &map.configuration.system_headers,
        deadline: map.deadline,
    };
    let analysis = super::link::analyze_linked(&map.root, &input, setup)?;
    if analysis.is_none() {
        map.limits_hit.push(super::LimitHit {
            limit: "budget",
            at: format!(
                "a time budget of {} s, during the link checks",
                map.budget.as_secs_f64()
            ),
        });
    }
    Ok(analysis)
}

/// The largest mapped tool's `harness.toml` the map reads.
pub const MAX_TOOL_CONFIG_BYTES: u64 = 1 << 20;

/// The mapped tool `id`'s `harness.toml` under `root`, read without
/// following a link and without checking its paths (a file of the tool may
/// be gone: that is what the map reports): `None` when it is missing, a
/// link, over [`MAX_TOOL_CONFIG_BYTES`] or not a config this harness reads.
pub fn tool_config(root: &Path, id: &str) -> Option<harness_core::config::TargetConfig> {
    let path = harness_core::config::tool_dir(root, id).join(harness_core::config::CONFIG_FILE);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_TOOL_CONFIG_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let table: toml::Table = text.parse().ok()?;
    harness_core::config::TargetConfig::from_table(table).ok()
}

/// `(path, id)` for each accepted tool (§3.3: a program at the path of an
/// accepted tool keeps that tool's id across maps): a `t-` tool's listed
/// files that are `main` programs in `files`, every listed file of an `l-`
/// tool (a library keeps its id for the group holding the most of its
/// files, `closure::libraries`). Tools in id order.
pub fn accepted_ids(root: &Path, files: &[super::FileFacts]) -> Vec<(String, String)> {
    let mains: BTreeSet<&str> = files
        .iter()
        .filter(|f| {
            f.kind == FileKind::C
                && f.compiled == Some(Compiled::Ok)
                && f.defined
                    .iter()
                    .any(|d| d.name == "main" && d.kind == "function")
        })
        .map(|f| f.path.as_str())
        .collect();
    let mut pairs = Vec::new();
    for id in harness_core::config::mapped_tools(root) {
        let Some(config) = tool_config(root, &id) else {
            continue;
        };
        let Some(list) = config.target.file_list() else {
            continue;
        };
        for f in &list.files {
            if id.starts_with("l-") || mains.contains(f.path.as_str()) {
                pairs.push((f.path.clone(), id.clone()));
            }
        }
    }
    pairs
}

/// What changed for one accepted tool since it was accepted
/// (docs/PROJECT-MAP-DESIGN.md §3.6), as the map file records it in
/// `accepted_tools`: `state status`, the cockpit and harness-mcp read this
/// record (through `harness_core::ledger::project_changed_notice`) instead
/// of comparing digests themselves, so every screen says the same sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ToolRecord {
    /// The tool's id.
    pub id: String,
    /// `none`, `files`, `configuration`, `closure` or `link` (see
    /// [`CHANGED_WORDS`]).
    pub changed: String,
    /// The sentence every screen shows (none for `none`).
    pub says: String,
}

/// The words of a [`ToolRecord`]'s `changed`: `none` — only a file
/// elsewhere in the project changed, nothing to do; `files` — the tool's
/// own files changed, its closure, configuration and link did not;
/// `configuration` — its configuration (or the compiler) changed;
/// `closure` — the files it needs changed (or its program is gone); `link`
/// — it no longer links.
pub const CHANGED_WORDS: &[&str] = &["none", "files", "configuration", "closure", "link"];

/// The sentence of a `none` record.
pub const NOTHING_TO_DO: &str =
    "a file elsewhere in the project changed; nothing to do for this tool";

fn words(items: &[String]) -> String {
    items
        .iter()
        .map(|s| harness_core::text::safe_line(s))
        .collect::<Vec<_>>()
        .join(", ")
}

fn bare(h: &str) -> &str {
    h.strip_prefix(harness_core::hash::HASH_PREFIX).unwrap_or(h)
}

/// What a new map needs of the map file it replaces (read before it is
/// replaced): its `root_hash`, each file's hash, its programs and its
/// `accepted_tools` records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Previous {
    /// Its `root_hash`.
    pub root_hash: String,
    /// `path → blake3` of every walked file.
    pub files: BTreeMap<String, String>,
    /// Its records.
    pub tools: Vec<ToolRecord>,
}

/// The map file under `root` as it stands, read before a new map replaces
/// it: `None` when there is none or it cannot be read.
pub fn previous(root: &Path) -> Option<Previous> {
    #[derive(serde::Deserialize)]
    struct File {
        path: String,
        blake3: String,
    }
    #[derive(serde::Deserialize)]
    struct Map {
        root_hash: String,
        #[serde(default)]
        files: Vec<File>,
        #[serde(default)]
        accepted_tools: Vec<ToolRecord>,
    }
    let path = root.join(MAP_FILE);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_MAP_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let m: Map = serde_json::from_slice(&bytes).ok()?;
    Some(Previous {
        root_hash: m.root_hash,
        files: m.files.into_iter().map(|f| (f.path, f.blake3)).collect(),
        tools: m.accepted_tools,
    })
}

/// The mapped tools whose `harness.toml` cannot be read (so nothing is
/// compared for them), in id order.
pub fn unreadable_tools(root: &Path) -> Vec<String> {
    harness_core::config::mapped_tools(root)
        .into_iter()
        .filter(|id| tool_config(root, id).is_none())
        .collect()
}

/// The `main` programs of `file` that are not accepted tools, `(id, path)`
/// in id order: listed on every map while any tool exists.
pub fn not_accepted(root: &Path, file: &MapFile) -> Vec<(String, String)> {
    let tools = harness_core::config::mapped_tools(root);
    file.programs
        .iter()
        .filter(|p| p.kind == "main" && !tools.contains(&p.id))
        .map(|p| (p.id.clone(), p.path.clone()))
        .collect()
}

/// `files` and every project file their includes reach, transitively.
fn with_includes(map: &FolderMap, files: &[String]) -> BTreeSet<String> {
    let by_path: BTreeMap<&str, &super::FileFacts> =
        map.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = files.to_vec();
    while let Some(p) = queue.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        if let Some(f) = by_path.get(p.as_str()) {
            queue.extend(f.includes.iter().cloned());
        }
    }
    seen
}

/// For each accepted tool (a `harness.toml` with `map`) whose map digests
/// differ from `file`'s, the record of what changed since it was accepted:
/// which digest moved — the project's files, or the configuration or the
/// compiler — and what that did to the tool: its closure (files it now
/// needs or no longer needs, include folders, a duplicate set open again, a
/// program or library gone), its configuration, its link. When none of
/// those changed, the tool's own files (and the headers they include) are
/// compared with the map it was accepted under, through `prev` (the map
/// file this one replaces): unchanged is `none` ([`NOTHING_TO_DO`]),
/// changed is `files`. A hand-written tool (no `map`), a tool whose digests
/// match, or one whose `harness.toml` cannot be read gets no record.
pub fn what_changed(map: &FolderMap, file: &MapFile, prev: Option<&Previous>) -> Vec<ToolRecord> {
    let mut out = Vec::new();
    for id in harness_core::config::mapped_tools(&map.root) {
        let Some(config) = tool_config(&map.root, &id) else {
            continue;
        };
        let Some(list) = config.target.file_list() else {
            continue;
        };
        let Some(stamp) = &list.map else {
            continue;
        };
        let root_moved = bare(&stamp.root_hash) != bare(&file.root_hash);
        let inputs_moved = bare(&stamp.inputs_hash) != bare(&file.inputs_hash);
        if !root_moved && !inputs_moved {
            continue;
        }
        let input = Input {
            files: &map.files,
            parser: &map.parser,
            walk_issues: &map.walk_issues,
            accepted: &[],
        };
        let listed_paths: Vec<String> = list.files.iter().map(|f| f.path.clone()).collect();
        let shown = harness_core::text::safe_line(&id);
        // What the tool is now, by kind of change.
        let mut closure_says: Vec<String> = Vec::new();
        let mut config_says: Vec<String> = Vec::new();
        let mut link_says: Vec<String> = Vec::new();
        // The id to accept again under (a library's id may have moved).
        let mut accept_as = id.clone();
        let expected: Option<Vec<String>> = if id.starts_with("t-") {
            match file.programs.iter().find(|p| p.id == id) {
                None => {
                    closure_says.push(
                        "its program is no longer in the map (its file is gone, or no longer \
                         defines main)"
                            .to_string(),
                    );
                    None
                }
                Some(p) => {
                    let picks: Vec<(Vec<String>, String)> = list
                        .picks
                        .iter()
                        .map(|k| (k.definers.clone(), k.keep.clone()))
                        .collect();
                    match closure::chosen_closure(&input, &p.path, &picks) {
                        None => {
                            closure_says.push(format!(
                                "{} is no longer a main program",
                                harness_core::text::safe_line(&p.path)
                            ));
                            None
                        }
                        Some(chosen) => {
                            for open in &chosen.open {
                                closure_says.push(format!(
                                    "a duplicate set is open that its picks do not settle ({} \
                                     each define {})",
                                    words(&open.definers),
                                    words(&open.symbols)
                                ));
                            }
                            Some(chosen.files)
                        }
                    }
                }
            }
        } else {
            match file.libraries.iter().find(|l| l.id == id) {
                Some(l) => Some(l.files.clone()),
                None => {
                    // Its files may now be another library's: named, so the
                    // accept command given is one that is taken.
                    let mut holders: Vec<(usize, &LibraryRec)> = file
                        .libraries
                        .iter()
                        .map(|l| {
                            let n = l.files.iter().filter(|f| listed_paths.contains(f)).count();
                            (n, l)
                        })
                        .filter(|(n, _)| *n > 0)
                        .collect();
                    holders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
                    match holders.first() {
                        Some((_, l)) => {
                            closure_says.push(format!(
                                "its files are now in library {} ({})",
                                harness_core::text::safe_line(&l.id),
                                words(&l.files)
                            ));
                            accept_as = l.id.clone();
                        }
                        None => {
                            closure_says.push("it is no longer a library in the map".to_string())
                        }
                    }
                    None
                }
            }
        };
        if let Some(files) = expected {
            match super::accept::shape(map, &files) {
                Err(why) => config_says.push(why),
                Ok(shape) => {
                    let listed: BTreeMap<&str, &[String]> = list
                        .files
                        .iter()
                        .map(|f| (f.path.as_str(), f.include_dirs.as_slice()))
                        .collect();
                    let now: BTreeMap<&str, &[String]> = shape
                        .files
                        .iter()
                        .map(|(p, d)| (p.as_str(), d.as_slice()))
                        .collect();
                    let added: Vec<String> = now
                        .keys()
                        .filter(|p| !listed.contains_key(*p))
                        .map(|p| p.to_string())
                        .collect();
                    let removed: Vec<String> = listed
                        .keys()
                        .filter(|p| !now.contains_key(*p))
                        .map(|p| p.to_string())
                        .collect();
                    let moved: Vec<String> = now
                        .iter()
                        .filter(|(p, d)| listed.get(*p).is_some_and(|l| l != *d))
                        .map(|(p, _)| p.to_string())
                        .collect();
                    if !added.is_empty() {
                        closure_says
                            .push(format!("closure changed: it now needs {}", words(&added)));
                    }
                    if !removed.is_empty() {
                        closure_says.push(format!(
                            "closure changed: it no longer needs {}",
                            words(&removed)
                        ));
                    }
                    if !moved.is_empty() {
                        closure_says.push(format!(
                            "closure changed: the include folders of {} are different",
                            words(&moved)
                        ));
                    }
                    let was = &list.configuration;
                    if was.name != shape.name || was.from != shape.from || was.flags != shape.flags
                    {
                        let say = |name: &str, from, flags: &[String]| {
                            format!(
                                "{}, from {}, flags {}",
                                harness_core::text::safe_line(name),
                                super::accept::from_word(from),
                                if flags.is_empty() {
                                    "none".to_string()
                                } else {
                                    words(flags)
                                }
                            )
                        };
                        config_says.push(format!(
                            "configuration changed: it was {}, the map's is {}",
                            say(&was.name, was.from, &was.flags),
                            say(&shape.name, shape.from, &shape.flags)
                        ));
                    }
                }
            }
        }
        let linked = file
            .closures
            .iter()
            .find(|c| c.program == id)
            .and_then(|c| c.linked.as_ref());
        if let Some(LinkedRec::Failed {
            missing, doubled, ..
        }) = linked
        {
            let mut why = Vec::new();
            if !missing.is_empty() {
                why.push(format!("missing {}", words(missing)));
            }
            if !doubled.is_empty() {
                why.push(format!("defined twice {}", words(doubled)));
            }
            link_says.push(format!(
                "it no longer links{}",
                if why.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", why.join("; "))
                }
            ));
        }
        let moved = match (root_moved, inputs_moved) {
            (true, false) => "the project's files changed",
            (false, true) => "the configuration or the compiler changed",
            _ => "the project's files and the configuration or the compiler changed",
        };
        let changed = if !closure_says.is_empty() {
            "closure"
        } else if !link_says.is_empty() {
            "link"
        } else if !config_says.is_empty() {
            "configuration"
        } else {
            ""
        };
        let accept_cmd = format!(
            "accept it again with `harness project accept {}`",
            harness_core::text::safe_line(&accept_as)
        );
        let record = if !changed.is_empty() {
            let mut what = closure_says;
            what.extend(config_says);
            what.extend(link_says);
            ToolRecord {
                id,
                changed: changed.into(),
                says: format!("{moved}: {}; {accept_cmd}", what.join("; ")),
            }
        } else if inputs_moved {
            ToolRecord {
                id,
                changed: "configuration".into(),
                says: format!(
                    "{moved}, but this tool's files, flags and link are the same: {accept_cmd} \
                     only to record that"
                ),
            }
        } else {
            // Only the project's files moved, and nothing of the tool did:
            // its own files (and the headers they include) against the map
            // it was accepted under.
            let own = with_includes(map, &listed_paths);
            let now: BTreeMap<&str, &str> = map
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.blake3.as_str()))
                .collect();
            let differ = |base: &BTreeMap<String, String>| -> Vec<String> {
                own.iter()
                    .filter(|p| now.get(p.as_str()).copied() != base.get(*p).map(String::as_str))
                    .cloned()
                    .collect()
            };
            let baseline: Option<Result<&BTreeMap<String, String>, ()>> = prev.and_then(|p| {
                if bare(&p.root_hash) == bare(&stamp.root_hash) {
                    return Some(Ok(&p.files));
                }
                match p
                    .tools
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.changed.as_str())
                {
                    // The map before found its own files as accepted.
                    Some("none") => Some(Ok(&p.files)),
                    // The map before found them changed already.
                    Some("files") => Some(Err(())),
                    _ => None,
                }
            });
            let scan = format!(
                "scan it to read them (`harness scan --tool {shown}`); accepting it again only \
                 clears this note"
            );
            match baseline {
                Some(Ok(base)) => {
                    let changed_files = differ(base);
                    if changed_files.is_empty() {
                        ToolRecord {
                            id,
                            changed: "none".into(),
                            says: NOTHING_TO_DO.into(),
                        }
                    } else {
                        ToolRecord {
                            id,
                            changed: "files".into(),
                            says: format!(
                                "its own files changed since it was accepted ({}); its closure, \
                                 configuration and link are the same: {scan}",
                                words(&changed_files)
                            ),
                        }
                    }
                }
                Some(Err(())) => ToolRecord {
                    id,
                    changed: "files".into(),
                    says: format!(
                        "its own files changed since it was accepted; its closure, \
                         configuration and link are the same: {scan}"
                    ),
                },
                None => ToolRecord {
                    id,
                    changed: "files".into(),
                    says: format!(
                        "the project's files changed since it was accepted (the map it was \
                         accepted under is gone, so its own files are not compared); its \
                         closure, configuration and link are the same: {scan}"
                    ),
                },
            }
        };
        out.push(record);
    }
    out
}

/// The largest map file written (§3.10): past it the map is a limit hit.
pub const MAX_MAP_BYTES: usize = 64 << 20;

/// [`render`] within [`MAX_MAP_BYTES`]: when the full map is larger, the
/// `size` limit is added to `map` and the file facts alone are rendered;
/// when even those are larger, a refusal in one sentence (nothing is
/// written). The file and its bytes.
pub fn render_bounded(
    map: &mut FolderMap,
    analysis: Option<&Analysis>,
) -> Result<(MapFile, Vec<u8>), Error> {
    let file = render(map, analysis)?;
    let bytes = to_bytes(&file);
    if bytes.len() <= MAX_MAP_BYTES {
        return Ok((file, bytes));
    }
    map.limits_hit.push(super::LimitHit {
        limit: "size",
        at: format!("a map file of {} MiB", MAX_MAP_BYTES >> 20),
    });
    let file = render(map, None)?;
    let bytes = to_bytes(&file);
    if bytes.len() <= MAX_MAP_BYTES {
        return Ok((file, bytes));
    }
    Err(Error::Invariant(format!(
        "the map's file facts alone are over {} MiB, so no map was written: map a smaller folder",
        MAX_MAP_BYTES >> 20
    )))
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
            // Over 8 MiB, by its size and head, as a walked file is.
            let abs = map.root.join(rel);
            let bytes = std::fs::metadata(&abs)
                .map_err(|e| Error::io(&abs, e))?
                .len();
            pairs.insert(rel.to_string(), super::map_hash(&abs, bytes)?);
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

/// The map's `toolchain` record.
pub fn toolchain_rec(t: &Toolchain) -> ToolchainRec {
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

/// The numbers of an index (`d10.2` → `[10, 2]`, `d3` → `[3]`), so `d2`
/// sorts before `d10` and `d1.2` before `d1.10`.
pub fn index_numbers(index: &str) -> Vec<u64> {
    index
        .trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .split('.')
        .map(|n| n.parse().unwrap_or(u64::MAX))
        .collect()
}

/// Indexes sorted by their numbers, each once.
fn by_number(mut v: Vec<String>) -> Vec<String> {
    v.sort_by_key(|i| index_numbers(i));
    v.dedup();
    v
}

/// An ambiguous include is settled when the configuration names the header
/// as the system's, or one of the `-I`, `-iquote`, `-isystem` or
/// `-idirafter` folders the file compiles with (`flags`: its own, which
/// under `compile_commands` are its entry's then the configuration's) holds
/// the candidate (`-include` names a file, never a folder).
fn settled(map: &FolderMap, flags: &[String], a: &super::Ambiguous) -> bool {
    let c = &map.configuration;
    if c.system_headers.contains(&a.header) {
        return true;
    }
    flags.iter().any(|flag| {
        harness_core::config::flags::split_path_flag(flag).is_some_and(|(prefix, dir)| {
            prefix != "-include" && {
                let held = if dir == "." {
                    a.header.clone()
                } else {
                    format!("{}/{}", dir.trim_end_matches('/'), a.header)
                };
                a.candidates.contains(&held)
            }
        })
    })
}

/// The ambiguous includes of `files` (and their headers') the configuration
/// does not settle, sorted: each file read with its own flags (a `.h` with
/// the configuration's).
pub fn unsettled_ambiguous(map: &FolderMap, files: &[String]) -> Vec<AmbiguousRec> {
    let by_path: BTreeMap<&str, &super::FileFacts> =
        map.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut unsettled: BTreeSet<AmbiguousRec> = BTreeSet::new();
    for p in files {
        if let Some(f) = by_path.get(p.as_str()) {
            let flags = if f.kind == FileKind::C {
                &f.flags
            } else {
                &map.configuration.flags
            };
            for a in f.ambiguous.iter().filter(|a| !settled(map, flags, a)) {
                unsettled.insert(AmbiguousRec {
                    header: a.header.clone(),
                    candidates: a.candidates.clone(),
                    used: a.used.clone(),
                });
            }
        }
    }
    unsettled.into_iter().collect()
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
        let not_listed: BTreeSet<&str> = map
            .evidence
            .not_in_compile_commands
            .iter()
            .map(String::as_str)
            .collect();
        let from_cc =
            map.configuration.from == harness_core::config::ConfigurationFrom::CompileCommands;
        for c in &a.closures {
            let rec = closure_rec(map, &by_path, a, c);
            // Flags that differ, or under `from = "compile_commands"` a
            // file no entry lists, keep the configuration a guess.
            let unlisted = from_cc && rec.files.iter().any(|f| not_listed.contains(f.as_str()));
            if !rec.flags_differ.is_empty() || unlisted {
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
            proposed: c.proposed,
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
            not_in_compile_commands: sorted(ev.not_in_compile_commands.clone()),
            also_found: sorted(ev.also_found.clone()),
        },
        limits_hit,
        accepted_tools: Vec::new(),
    })
}

fn closure_rec(
    map: &FolderMap,
    by_path: &BTreeMap<&str, &super::FileFacts>,
    analysis: &Analysis,
    c: &closure::Closure,
) -> ClosureRec {
    let included_as_text: Vec<IncludedRec> = analysis
        .included_by
        .iter()
        .filter(|i| c.files.contains(&i.file))
        .map(|i| IncludedRec {
            file: i.file.clone(),
            by: sorted(i.by.clone()),
        })
        .collect();
    let strong_over_weak: Vec<StrongOverWeakRec> = c
        .strong_over_weak
        .iter()
        .map(|s| StrongOverWeakRec {
            sym: s.sym.clone(),
            weak: sorted(s.weak.clone()),
            strong: sorted(s.strong.clone()),
        })
        .collect();
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
    let unsettled = unsettled_ambiguous(map, &c.files);
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
            definers.sort_by_key(|a| index_numbers(&a.index));
            DuplicateRec {
                set: d.set.clone(),
                symbols: sorted(d.symbols.clone()),
                definers,
                links: by_number(d.links.clone()),
                choice: d.choice.as_ref().map(|ch| ChoiceRec {
                    keep: ch.keep.clone(),
                    by: ch.by,
                }),
                under: by_number(d.under.clone()),
            }
        })
        .collect();
    duplicates.sort_by_key(|a| index_numbers(&a.set));
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
        strong_over_weak,
        included_as_text,
        ambiguous_unsettled: unsettled,
        linked: c.linked.as_ref().map(|l| match l {
            Linked::Ok => LinkedRec::Ok("ok"),
            Linked::Failed {
                missing,
                doubled,
                not_checked,
                not_compiled,
            } => LinkedRec::Failed {
                missing: sorted(missing.clone()),
                doubled: sorted(doubled.clone()),
                not_checked: sorted(not_checked.clone()),
                not_compiled: sorted(not_compiled.clone()),
            },
        }),
        questions: by_number(c.questions.clone()),
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
    write_bytes(root, &to_bytes(file))
}

/// Write the map file's `bytes` ([`to_bytes`]) under `root` in full
/// (atomically): its path.
pub fn write_bytes(root: &Path, bytes: &[u8]) -> Result<PathBuf, Error> {
    let path = root.join(MAP_FILE);
    harness_core::ledger::write_atomic(&path, bytes)?;
    Ok(path)
}

/// The text of `migration/.gitignore`.
pub fn gitignore_text() -> String {
    let mut text = String::from(
        "# Written by the harness the first time it made this folder; never overwritten.\n\
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

    /// Indexes sort by their numbers: `d2` before `d10`, `d1.2` before
    /// `d1.10` (the file's "definers in path order").
    #[test]
    fn indexes_sort_by_their_numbers() {
        let v: Vec<String> = ["d10", "d1.10", "d2", "d1.2", "d1.1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(by_number(v), ["d1.1", "d1.2", "d1.10", "d2", "d10"]);
        let mut defs = [
            DefinerRec {
                index: "d1.10".into(),
                path: "z.c".into(),
            },
            DefinerRec {
                index: "d1.9".into(),
                path: "y.c".into(),
            },
        ];
        defs.sort_by_key(|d| index_numbers(&d.index));
        assert_eq!(defs[0].index, "d1.9");
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
        // Any command's first ledger writes it, not only `project map`.
        assert_eq!(
            text.lines().next(),
            Some("# Written by the harness the first time it made this folder; never overwritten.")
        );
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
