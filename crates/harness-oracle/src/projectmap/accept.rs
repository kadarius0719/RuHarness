//! `harness project accept <id>` (docs/PROJECT-MAP-DESIGN.md §3.6, §3.8):
//! the person's gate from a mapped program or library to a tool. It reads
//! the map file for the ids and indexes the person saw, maps the project
//! again to check the map still describes the tree, applies the person's
//! picks to the held duplicate sets, recomputes the program's closure under
//! them, **links it again** (a reply or the map's own link result is never
//! taken as proof), and only then renders the tool's `harness.toml` in the
//! file-list form (docs/SCHEMAS.md "The file-list form").
//!
//! Every refusal is one sentence saying what to do next. Nothing is written
//! by [`prepare`]; [`write`] writes the one file, under the locks the
//! caller holds (the project lock, then an existing tool's ledger lock).
//!
//! The written configuration is the map's own copy: its name, `from` and
//! the flags every file of the closure compiled with, then `-idirafter
//! <dir>` for each include folder the map passed that way (a folder holding
//! one of the configuration's `system_headers`); each file keeps its other
//! include folders. A library is compiled (by the map) and never linked: its
//! target has no program, and its id is its `name`.

use super::closure::{self, Incomplete, IncompleteWhy, Input, Linked, Linker};
use super::config::{ConfigSource, GUESSED_NAME};
use super::link::{CcLinker, LinkSetup};
use super::{mapfile, Compiled, FileFacts, FileKind, FolderMap, MapOptions};
use harness_core::config::{self as hconfig, ConfigurationFrom};
use harness_core::error::Error;
use harness_core::text::safe_line;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The `[oracle] allowlist` an accepted tool gets (as `bench init` writes
/// a case's).
pub const ALLOWLIST: &[&str] = &["cc", "cargo", "rustc", "nm"];
/// The `[llm] max_tokens` an accepted tool gets (as `bench init` writes a
/// case's).
pub const MAX_TOKENS: u32 = 16_384;
/// The longest run name.
pub const MAX_RUN_NAME: usize = 64;

fn refuse(sentence: impl Into<String>) -> Error {
    Error::Invariant(sentence.into())
}

// ---------- the map file, as accept reads it ----------

/// What `accept` reads of `migration/map/project-map.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Stored {
    /// The map's `root_hash`.
    pub root_hash: String,
    /// The map's `inputs_hash`.
    pub inputs_hash: String,
    /// Its configuration's name.
    pub configuration: StoredConfiguration,
    /// Its programs.
    #[serde(default)]
    pub programs: Vec<StoredProgram>,
    /// Its closures.
    #[serde(default)]
    pub closures: Vec<StoredClosure>,
    /// Its libraries.
    #[serde(default)]
    pub libraries: Vec<StoredLibrary>,
    /// The caps it reached.
    #[serde(default)]
    pub limits_hit: Vec<serde_json::Value>,
}

/// The stored configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredConfiguration {
    /// Its name.
    pub name: String,
}

/// A stored program.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredProgram {
    /// Its id.
    pub id: String,
    /// Its file.
    pub path: String,
    /// `main`, `fuzz` or `driver`.
    pub kind: String,
}

/// A stored closure's files and duplicate sets.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredClosure {
    /// The program's id.
    pub program: String,
    /// Its files.
    #[serde(default)]
    pub files: Vec<String>,
    /// Its duplicate sets.
    #[serde(default)]
    pub duplicates: Vec<StoredSet>,
}

/// A stored duplicate set.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredSet {
    /// `d1…`.
    pub set: String,
    /// The closure's symbols it defines.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Its definers.
    pub definers: Vec<StoredDefiner>,
}

/// A stored definer.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredDefiner {
    /// `d1.1…`.
    pub index: String,
    /// Its file.
    pub path: String,
}

/// A stored library.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredLibrary {
    /// Its id.
    pub id: String,
    /// Its files.
    pub files: Vec<String>,
}

impl StoredSet {
    fn path_of(&self, index: &str) -> Option<&str> {
        self.definers
            .iter()
            .find(|d| d.index == index)
            .map(|d| d.path.as_str())
    }

    /// `d1.1 a.c, d1.2 b.c`.
    fn named(&self) -> String {
        self.definers
            .iter()
            .map(|d| format!("{} {}", d.index, safe_line(&d.path)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Read the map file under `root`: refused in one sentence when there is
/// none, it is a link, too large or not a map this harness reads.
pub fn read_stored(root: &Path) -> Result<Stored, Error> {
    let path = root.join(mapfile::MAP_FILE);
    let meta =
        match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => return Err(refuse(
                "there is no project map yet: run `harness project map` first, read its screen, \
                 then accept a program or library by the id it prints",
            )),
        };
    if !meta.file_type().is_file() || meta.len() > mapfile::MAX_MAP_BYTES as u64 {
        return Err(refuse(format!(
            "{} is not a map file this harness reads (a link, or over {} MiB): run `harness \
             project map` again",
            mapfile::MAP_FILE,
            mapfile::MAX_MAP_BYTES >> 20
        )));
    }
    let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
    serde_json::from_slice(&bytes).map_err(|_| {
        refuse(format!(
            "{} cannot be read as a project map: run `harness project map` again",
            mapfile::MAP_FILE
        ))
    })
}

// ---------- what a tool is made of ----------

/// A tool's files and configuration, as `accept` writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    /// Each `.c` with its include folders, in path order.
    pub files: Vec<(String, Vec<String>)>,
    /// The configuration's name.
    pub name: String,
    /// What it stands for.
    pub from: ConfigurationFrom,
    /// Its flags: those every file compiled with, then `-idirafter<dir>`.
    pub flags: Vec<String>,
}

/// `from` as `harness.toml` spells it.
pub fn from_word(from: ConfigurationFrom) -> &'static str {
    match from {
        ConfigurationFrom::Make => "make",
        ConfigurationFrom::Meson => "meson",
        ConfigurationFrom::Cmake => "cmake",
        ConfigurationFrom::CompileCommands => "compile_commands",
        ConfigurationFrom::Stated => "stated",
    }
}

/// The shape of a tool over `files` (paths of the map's `.c` files): `Err`
/// is the refusal when their flags differ.
pub fn shape(map: &FolderMap, files: &[String]) -> Result<Shape, String> {
    let by_path: BTreeMap<&str, &FileFacts> =
        map.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let walked: BTreeSet<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
    let mut sorted: Vec<&FileFacts> = files
        .iter()
        .filter_map(|p| by_path.get(p.as_str()).copied())
        .filter(|f| f.kind == FileKind::C)
        .collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    sorted.dedup_by(|a, b| a.path == b.path);
    let flags = match sorted.first() {
        Some(first) => first.flags.clone(),
        None => map.configuration.flags.clone(),
    };
    if let Some(other) = sorted.iter().find(|f| f.flags != flags) {
        return Err(format!(
            "the files compile with different flags ({} differs from {}), so the configuration \
             is still a guess: state one build in {} and map again",
            safe_line(&other.path),
            safe_line(&sorted[0].path),
            super::config::CONFIG_FILE
        ));
    }
    let system = &map.configuration.system_headers;
    let holds_system = |dir: &str| {
        system.iter().any(|name| {
            let held = if dir == "." {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            walked.contains(held.as_str())
        })
    };
    let mut after: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for f in &sorted {
        let mut dirs = Vec::new();
        for dir in &f.include_dirs {
            if holds_system(dir) {
                if !after.contains(dir) {
                    after.push(dir.clone());
                }
            } else {
                dirs.push(dir.clone());
            }
        }
        out.push((f.path.clone(), dirs));
    }
    let mut all = flags;
    all.extend(after.iter().map(|d| format!("-idirafter{d}")));
    Ok(Shape {
        files: out,
        name: map.configuration.name.clone(),
        from: map.configuration.from,
        flags: all,
    })
}

// ---------- the request and its result ----------

/// What the person asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// The program's or library's id.
    pub id: String,
    /// Each `--keep` as typed: `d1=d1.2` or `d1=<path>`.
    pub keeps: Vec<String>,
    /// `--run-name`.
    pub run_name: Option<String>,
}

/// A pick, in the tool's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickOut {
    /// The set's index in the map.
    pub set: String,
    /// The definers' paths, sorted.
    pub definers: Vec<String>,
    /// The kept path.
    pub keep: String,
    /// `person` or `links`.
    pub by: &'static str,
    /// The closure's symbols the set defines.
    pub symbols: Vec<String>,
}

impl PickOut {
    /// "keeping `a.c` over `b.c` for `f, g`".
    pub fn words(&self) -> String {
        let others: Vec<String> = self
            .definers
            .iter()
            .filter(|d| **d != self.keep)
            .map(|d| format!("`{}`", safe_line(d)))
            .collect();
        let mut s = format!(
            "keeping `{}` over {} for `{}`",
            safe_line(&self.keep),
            others.join(" and "),
            self.symbols
                .iter()
                .map(|s| safe_line(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if self.by == "links" {
            s.push_str(" (settled by linking)");
        }
        s
    }
}

/// Everything `accept` will write, checked: nothing is written yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The id.
    pub id: String,
    /// The tool's `harness.toml`, absolute.
    pub path: PathBuf,
    /// The same, relative to the root.
    pub rel: String,
    /// Its text.
    pub text: String,
    /// The tool was accepted before (its ledger is kept).
    pub existed: bool,
    /// A library: no program, nothing linked.
    pub library: bool,
    /// The picks recorded.
    pub picks: Vec<PickOut>,
    /// Definers not kept: "alternative not kept".
    pub not_kept: Vec<String>,
    /// The files and the configuration.
    pub shape: Shape,
    /// The guessed outside libraries.
    pub libs: Vec<String>,
    /// The run name.
    pub run_name: String,
    /// The keys of the `harness.toml` there that `accept` does not own and
    /// carried over, as dotted names (`oracle.whole_program`, `llm.model`).
    pub kept: Vec<String>,
    /// A program whose whole-program check is not configured: the file
    /// holds a commented example.
    pub whole_program_off: bool,
    /// The sets the person picked that these picks do not reach: not
    /// recorded.
    pub dropped: Vec<String>,
}

/// Parse one `--keep`: `(set, value)`.
fn parse_keep(raw: &str) -> Result<(String, String), Error> {
    let bad = || {
        refuse(format!(
            "--keep {} is not a pick: write --keep d1=d1.2 (a set and one of its definers' \
             indexes) or --keep d1=<path> (that definer's path), as the map's screen prints them",
            safe_line(raw)
        ))
    };
    let (set, value) = raw.split_once('=').ok_or_else(bad)?;
    let digits = set.strip_prefix('d').ok_or_else(bad)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || value.is_empty() {
        return Err(bad());
    }
    Ok((set.to_string(), value.to_string()))
}

fn bare(h: &str) -> &str {
    h.strip_prefix(harness_core::hash::HASH_PREFIX).unwrap_or(h)
}

fn incomplete_words(i: &Incomplete) -> String {
    let path = i.path.as_deref().map(safe_line).unwrap_or_default();
    let syms = i
        .symbols
        .iter()
        .map(|s| safe_line(s))
        .collect::<Vec<_>>()
        .join(", ");
    match i.why {
        IncompleteWhy::Pending => format!("a duplicate set is still open ({syms})"),
        IncompleteWhy::MayBeDefinedIn => format!("{path} did not compile and may define {syms}"),
        IncompleteWhy::Unread => format!("{path} could not be read and may define {syms}"),
        IncompleteWhy::UnreadableFolder => format!("the folder {path} could not be read"),
    }
}

fn linked_words(l: &Linked) -> String {
    let Linked::Failed {
        missing,
        doubled,
        not_checked,
        not_compiled,
    } = l
    else {
        return "linked".into();
    };
    let names = |v: &[String]| {
        v.iter()
            .map(|s| safe_line(s))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !not_compiled.is_empty() {
        parts.push(format!(
            "a file did not compile for the link ({})",
            names(not_compiled)
        ));
    }
    if !missing.is_empty() {
        parts.push(format!("missing {}", names(missing)));
    }
    if !doubled.is_empty() {
        parts.push(format!("defined twice {}", names(doubled)));
    }
    if !not_checked.is_empty() {
        parts.push(format!("not checked {}", names(not_checked)));
    }
    if parts.is_empty() {
        parts.push("for a reason the harness does not read".into());
    }
    parts.join("; ")
}

/// A run name: 1 to [`MAX_RUN_NAME`] letters, digits, `.`, `_` or `-`,
/// not starting with `.` or `-`.
pub fn is_run_name(name: &str) -> bool {
    (1..=MAX_RUN_NAME).contains(&name.len())
        && !name.starts_with('.')
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A program file's stem: its name without `.c`.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".c").unwrap_or(name)
}

/// A program id, a library id, a set index and a definer index of the map
/// file have their shapes (`t-…`/`l-…` tool ids, `d<n>`, `d<n>.<m>`): the
/// map file lives in the project, so nothing else is printed or compared.
fn check_shapes(stored: &Stored) -> Result<(), Error> {
    let number = |s: &str| {
        (1..=9).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
    };
    let set_ok = |s: &str| s.strip_prefix('d').is_some_and(number);
    let definer_ok = |set: &str, s: &str| {
        s.strip_prefix(set)
            .and_then(|r| r.strip_prefix('.'))
            .is_some_and(number)
    };
    let ok = stored
        .programs
        .iter()
        .all(|p| p.id.starts_with("t-") && hconfig::is_tool_id(&p.id))
        && stored
            .libraries
            .iter()
            .all(|l| l.id.starts_with("l-") && hconfig::is_tool_id(&l.id))
        && stored.closures.iter().all(|c| {
            hconfig::is_tool_id(&c.program)
                && c.duplicates.iter().all(|d| {
                    set_ok(&d.set) && d.definers.iter().all(|x| definer_ok(&d.set, &x.index))
                })
        });
    if ok {
        Ok(())
    } else {
        Err(refuse(format!(
            "{} holds an id or an index that is not one the map writes: run `harness project \
             map` again",
            mapfile::MAP_FILE
        )))
    }
}

/// The folders a tool is written under — `migration/`, `migration/tools/`
/// and `migration/tools/<id>/` — are real folders where they exist (a link
/// or a file there is refused), checked before any lock is taken, so no
/// lock file is made or emptied outside the project.
pub fn check_folders(root: &Path, id: &str) -> Result<(), Error> {
    hconfig::check_tool_id(id).map_err(refuse)?;
    let migration = root.join(harness_core::ledger::MIGRATION_DIR);
    let tools = hconfig::tools_dir(root);
    let dir = hconfig::tool_dir(root, id);
    for path in [&migration, &tools, &dir] {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                let rel = path.strip_prefix(root).unwrap_or(path);
                return Err(refuse(format!(
                    "{} is not a folder (a link or a file), so no tool is written there and no \
                     lock is taken in it: move it away",
                    safe_line(&rel.display().to_string())
                )));
            }
            Err(_) => break,
        }
    }
    Ok(())
}

/// What the stored map says of the program or library `id` against the
/// map made again now: the same programs (id, file, kind), the same
/// libraries (id, files) and, for `id`, the same closure files and
/// duplicate sets (set, definers). The map file only turns the person's
/// indexes into paths; every file list, path and choice comes from the
/// fresh map.
fn same_as_fresh(stored: &Stored, fresh: &mapfile::MapFile, id: &str) -> bool {
    type Sets = Vec<(String, Vec<(String, String)>)>;
    fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
        v.sort();
        v
    }
    let stored_programs = sorted(
        stored
            .programs
            .iter()
            .map(|p| (p.id.clone(), p.path.clone(), p.kind.clone()))
            .collect(),
    );
    let fresh_programs = sorted(
        fresh
            .programs
            .iter()
            .map(|p| (p.id.clone(), p.path.clone(), p.kind.to_string()))
            .collect(),
    );
    let stored_libraries = sorted(
        stored
            .libraries
            .iter()
            .map(|l| (l.id.clone(), sorted(l.files.clone())))
            .collect(),
    );
    let fresh_libraries = sorted(
        fresh
            .libraries
            .iter()
            .map(|l| (l.id.clone(), sorted(l.files.clone())))
            .collect(),
    );
    let stored_closure: Option<(Vec<String>, Sets)> =
        stored.closures.iter().find(|c| c.program == id).map(|c| {
            (
                sorted(c.files.clone()),
                c.duplicates
                    .iter()
                    .map(|d| {
                        (
                            d.set.clone(),
                            d.definers
                                .iter()
                                .map(|x| (x.index.clone(), x.path.clone()))
                                .collect(),
                        )
                    })
                    .collect(),
            )
        });
    let fresh_closure: Option<(Vec<String>, Sets)> =
        fresh.closures.iter().find(|c| c.program == id).map(|c| {
            (
                sorted(c.files.clone()),
                c.duplicates
                    .iter()
                    .map(|d| {
                        (
                            d.set.clone(),
                            d.definers
                                .iter()
                                .map(|x| (x.index.clone(), x.path.clone()))
                                .collect(),
                        )
                    })
                    .collect(),
            )
        });
    stored_programs == fresh_programs
        && stored_libraries == fresh_libraries
        && stored_closure == fresh_closure
}

/// The `harness.toml` already at `path`, as a table: `None` when there is
/// none; refused when it is not a regular file this harness reads.
fn read_existing(path: &Path) -> Result<Option<toml::Table>, Error> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(None);
    };
    let unreadable = |why: &str| {
        refuse(format!(
            "the harness.toml already there ({}) {why}, and accepting again keeps what you added \
             to it: fix it or move it away, then accept",
            safe_line(&path.display().to_string())
        ))
    };
    if !meta.file_type().is_file() {
        return Err(unreadable("is not a regular file"));
    }
    if meta.len() > mapfile::MAX_TOOL_CONFIG_BYTES {
        return Err(unreadable("is over 1 MiB"));
    }
    let text = std::fs::read_to_string(path).map_err(|_| unreadable("cannot be read as text"))?;
    text.parse::<toml::Table>()
        .map(Some)
        .map_err(|_| unreadable("cannot be read as TOML"))
}

/// Check everything and render the tool's `harness.toml` (see the module
/// docs). `root` is the project root. Nothing is written.
pub fn prepare(root: &Path, req: &Request) -> Result<Prepared, Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    let id = req.id.as_str();
    hconfig::check_tool_id(id).map_err(refuse)?;
    if std::fs::symlink_metadata(root.join(hconfig::CONFIG_FILE)).is_ok() {
        return Err(refuse(
            "this project is already a folder-form target (it has a harness.toml at its root), so \
             no mapped tool is accepted into it: move that harness.toml away, or map a copy of \
             the project",
        ));
    }
    let stored = read_stored(&root)?;
    if !stored.limits_hit.is_empty() {
        return Err(refuse(
            "the map stopped at a limit and holds no programs: map a smaller folder (see `harness \
             project map`), then accept",
        ));
    }
    check_shapes(&stored)?;
    let program = stored.programs.iter().find(|p| p.id == id);
    let library = stored.libraries.iter().find(|l| l.id == id);
    if program.is_none() && library.is_none() {
        let mut ids: Vec<&str> = stored.programs.iter().map(|p| p.id.as_str()).collect();
        ids.extend(stored.libraries.iter().map(|l| l.id.as_str()));
        let named = if ids.is_empty() {
            "it holds none".to_string()
        } else {
            format!("its ids are {}", ids.join(", "))
        };
        return Err(refuse(format!(
            "the map has no program or library {id} ({named}): accept one of them, or run \
             `harness project map` again if the project changed"
        )));
    }
    if let Some(p) = program {
        match p.kind.as_str() {
            "main" => {}
            "fuzz" => {
                return Err(refuse(format!(
                    "{id} is a fuzzer, linked only with the project's fuzz driver: accept a main \
                     program or a library"
                )))
            }
            _ => {
                return Err(refuse(format!(
                    "{id} is a fuzz driver, never a tool alone: accept a main program or a \
                     library"
                )))
            }
        }
    }
    if library.is_some() && req.run_name.is_some() {
        return Err(refuse(format!(
            "--run-name names the program a tool runs, and {id} is a library, which runs \
             nothing: leave --run-name out (a library's name is its id)"
        )));
    }
    let stored_sets: Vec<StoredSet> = stored
        .closures
        .iter()
        .find(|c| c.program == id)
        .map(|c| c.duplicates.clone())
        .unwrap_or_default();
    // The person's picks, by set: the map file turns each index into a
    // path (the paths are checked against the fresh map below).
    let mut person: BTreeMap<String, String> = BTreeMap::new();
    for raw in &req.keeps {
        let (set, value) = parse_keep(raw)?;
        let Some(s) = stored_sets.iter().find(|s| s.set == set) else {
            let named = if stored_sets.is_empty() {
                format!("{id} has none")
            } else {
                format!(
                    "{id}'s are {}",
                    stored_sets
                        .iter()
                        .map(|s| s.set.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(refuse(format!(
                "--keep {} names no duplicate set of {id} ({named}): pick only among the sets the \
                 map shows for it",
                safe_line(raw)
            )));
        };
        let path = s
            .path_of(&value)
            .map(str::to_string)
            .or_else(|| {
                s.definers
                    .iter()
                    .find(|d| d.path == value)
                    .map(|d| d.path.clone())
            })
            .ok_or_else(|| {
                refuse(format!(
                    "--keep {} names no definer of {set}: its definers are {}",
                    safe_line(raw),
                    s.named()
                ))
            })?;
        if let Some(before) = person.get(&set) {
            if *before != path {
                return Err(refuse(format!(
                    "--keep names two definers for {set} ({} and {}): keep one",
                    safe_line(before),
                    safe_line(&path)
                )));
            }
        }
        person.insert(set, path);
    }

    // The map again: the tree must still be the one the map describes.
    let options = MapOptions {
        configuration: (stored.configuration.name != GUESSED_NAME)
            .then(|| stored.configuration.name.clone()),
        ..MapOptions::default()
    };
    let mut map = super::map_root(&root, &options)?;
    if !map.closures_possible() {
        return Err(refuse(
            "mapping the project again stopped at a limit, so nothing can be accepted: map a \
             smaller folder",
        ));
    }
    let root_hash = mapfile::root_hash(&map)?;
    let inputs_hash = mapfile::inputs_hash(
        &map.configuration.digest,
        &mapfile::toolchain_rec(&map.toolchain),
    );
    if bare(&root_hash) != bare(&stored.root_hash)
        || bare(&inputs_hash) != bare(&stored.inputs_hash)
    {
        return Err(refuse(
            "the project changed since the map was made (its files, its configuration or the \
             compiler): run `harness project map` again, read the screen, then accept",
        ));
    }
    if map.configuration.proposed {
        return Err(refuse(
            "the configuration came with the project: state it with `harness project map \
             --adopt`, or write your own migration/map/config.toml",
        ));
    }
    if map.configuration.source == ConfigSource::Guessed {
        return Err(refuse(format!(
            "the configuration is a guess, and a tool is built under a stated one: write the \
             build's name, from and flags in {} (or use `harness project ask --build`), map \
             again, then accept",
            super::config::CONFIG_FILE
        )));
    }

    // The programs, closures, duplicate sets and libraries again, with
    // their link checks, as `project map` computed them: the stored map is
    // trusted for none of them.
    let Some(analysis) = mapfile::analyze(&mut map)? else {
        return Err(refuse(
            "mapping the project again ran out of time during the link checks, so nothing can be \
             accepted: map a smaller folder",
        ));
    };
    let fresh = mapfile::render(&map, Some(&analysis))?;
    if !same_as_fresh(&stored, &fresh, id) {
        return Err(refuse(format!(
            "{} does not say what mapping the project again finds (its programs, libraries or \
             the files and choices of {id} differ): run `harness project map` again, read the \
             screen, then accept",
            mapfile::MAP_FILE
        )));
    }
    let sets: Vec<mapfile::DuplicateRec> = fresh
        .closures
        .iter()
        .find(|c| c.program == id)
        .map(|c| c.duplicates.clone())
        .unwrap_or_default();
    let named = |s: &mapfile::DuplicateRec| {
        s.definers
            .iter()
            .map(|d| format!("{} {}", d.index, safe_line(&d.path)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let paths_of = |s: &mapfile::DuplicateRec| {
        let mut v: Vec<String> = s.definers.iter().map(|d| d.path.clone()).collect();
        v.sort();
        v
    };

    let input = Input {
        files: &map.files,
        parser: &map.parser,
        walk_issues: &map.walk_issues,
        accepted: &[],
    };
    let fresh_program = fresh.programs.iter().find(|p| p.id == id);
    let mut picks: Vec<PickOut> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    let files: Vec<String> = if let Some(p) = fresh_program {
        for s in &sets {
            // A choice is the person's, or linking's when exactly one
            // choice links in the map made again now.
            let (keep, by) = match (person.get(&s.set), &s.choice) {
                (Some(path), _) => (path.clone(), "person"),
                (None, Some(c)) => match s.definers.iter().find(|d| d.index == c.keep) {
                    Some(d) => (d.path.clone(), "links"),
                    None => continue,
                },
                (None, None) => continue,
            };
            picks.push(PickOut {
                set: s.set.clone(),
                definers: paths_of(s),
                keep,
                by,
                symbols: s.symbols.clone(),
            });
        }
        let pairs: Vec<(Vec<String>, String)> = picks
            .iter()
            .map(|p| (p.definers.clone(), p.keep.clone()))
            .collect();
        let chosen = closure::chosen_closure(&input, &p.path, &pairs).ok_or_else(|| {
            refuse(format!(
                "{} no longer compiles as a main program: run `harness project map` again",
                safe_line(&p.path)
            ))
        })?;
        if let Some(open) = chosen.open.first() {
            let syms = open
                .symbols
                .iter()
                .map(|s| safe_line(s))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(match sets.iter().find(|s| paths_of(s) == open.definers) {
                Some(s) => refuse(format!(
                    "duplicate set {} of {id} ({syms}) is not settled: pick its definer yourself \
                     with --keep {}=<index or path> (its definers: {})",
                    s.set,
                    s.set,
                    named(s)
                )),
                None => refuse(format!(
                    "these picks reach a choice the map did not list for {id} (the files {} all \
                     define {syms}): keep the definers linking settled, or map again and read \
                     the screen",
                    open.definers
                        .iter()
                        .map(|p| safe_line(p))
                        .collect::<Vec<_>>()
                        .join(", "),
                )),
            });
        }
        if !chosen.incomplete_why.is_empty() {
            return Err(refuse(format!(
                "the closure of {id} is incomplete ({}): make those files compile or readable, map \
                 again, then accept",
                chosen
                    .incomplete_why
                    .iter()
                    .map(incomplete_words)
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        // Only the picks this closure reached are recorded; the person's
        // others are said to be dropped.
        for p in &picks {
            if !chosen.files.contains(&p.keep) && p.by == "person" {
                dropped.push(p.set.clone());
            }
        }
        picks.retain(|p| chosen.files.contains(&p.keep));
        chosen.files
    } else {
        let lib = fresh
            .libraries
            .iter()
            .find(|l| l.id == id)
            .ok_or_else(|| refuse(format!("{id} is no library of the map made again")))?;
        let by_path: BTreeMap<&str, &FileFacts> =
            map.files.iter().map(|f| (f.path.as_str(), f)).collect();
        for f in &lib.files {
            match by_path.get(f.as_str()) {
                Some(facts) if facts.compiled == Some(Compiled::Ok) => {}
                _ => {
                    return Err(refuse(format!(
                        "{} of {id} does not compile under this configuration: fix it or state \
                         the build in {}, map again, then accept",
                        safe_line(f),
                        super::config::CONFIG_FILE
                    )))
                }
            }
        }
        let gaps = closure::library_gaps(&input, &lib.files);
        if !gaps.is_empty() {
            return Err(refuse(format!(
                "the library {id} is incomplete ({}): make those files compile or readable, map \
                 again, then accept",
                gaps.iter()
                    .map(incomplete_words)
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        lib.files.clone()
    };
    let unsettled = mapfile::unsettled_ambiguous(&map, &files);
    if let Some(a) = unsettled.first() {
        return Err(refuse(format!(
            "a file of {id} includes {}, which {} each hold, and the configuration does not say \
             which: settle it in {} with -I or system_headers, map again, then accept",
            safe_line(&a.header),
            a.candidates
                .iter()
                .map(|c| safe_line(c))
                .collect::<Vec<_>>()
                .join(" and "),
            super::config::CONFIG_FILE
        )));
    }
    if map.configuration.from == ConfigurationFrom::CompileCommands {
        if let Some(f) = files
            .iter()
            .find(|f| map.evidence.not_in_compile_commands.contains(f))
        {
            return Err(refuse(format!(
                "{} is in no compile_commands.json entry, so its flags are a guess: regenerate \
                 compile_commands.json or state the build in {}, map again, then accept",
                safe_line(f),
                super::config::CONFIG_FILE
            )));
        }
    }
    let shape = shape(&map, &files).map_err(refuse)?;

    // The link, again, before anything is written (a library is compiled,
    // never linked).
    let libs = if fresh_program.is_some() {
        let setup = LinkSetup {
            walked: map.files.iter().map(|f| f.path.as_str()).collect(),
            system_headers: &map.configuration.system_headers,
            deadline: map.deadline,
        };
        let mut linker = CcLinker::new(&root, setup)?;
        let by_path: BTreeMap<&str, &FileFacts> =
            map.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let refs: Vec<&FileFacts> = files
            .iter()
            .filter_map(|f| by_path.get(f.as_str()).copied())
            .collect();
        let unresolved = closure::unresolved(&refs);
        let linked = linker.link(&refs, &unresolved)?;
        if linked != Linked::Ok {
            return Err(refuse(format!(
                "{id} does not link with these files ({}): pick another definer with --keep, or \
                 fix the program, map again, then accept",
                linked_words(&linked)
            )));
        }
        linker.libraries(&unresolved)?
    } else {
        Vec::new()
    };

    let run_name = match (&req.run_name, fresh_program) {
        (Some(name), _) => name.clone(),
        (None, Some(p)) => stem(&p.path).to_string(),
        (None, None) => id.to_string(),
    };
    if fresh_program.is_some() && !is_run_name(&run_name) {
        return Err(refuse(format!(
            "`{}` cannot be the run name (the file name features run the program as): give one \
             with --run-name, 1 to {MAX_RUN_NAME} letters, digits, ., _ or -",
            safe_line(&run_name)
        )));
    }
    let not_kept: Vec<String> = picks
        .iter()
        .flat_map(|p| p.definers.iter().filter(|d| **d != p.keep).cloned())
        .collect();
    let dir = hconfig::tool_dir(&root, id);
    let path = dir.join(hconfig::CONFIG_FILE);
    let existing = read_existing(&path)?;
    let rendered = render(
        &Owned {
            id,
            run_name: &run_name,
            shape: &shape,
            root_hash: &stored.root_hash,
            inputs_hash: &stored.inputs_hash,
            picks: &picks,
            libs: &libs,
            library: fresh_program.is_none(),
        },
        existing.as_ref(),
    );
    // What is written must be what every command reads.
    let table: toml::Table = rendered.text.parse().map_err(|e: toml::de::Error| {
        refuse(format!(
            "the harness.toml for {id} would not be TOML ({}): move the harness.toml there away, \
             then accept",
            safe_line(e.message())
        ))
    })?;
    hconfig::TargetConfig::from_table(table).map_err(|m| {
        refuse(format!(
            "the harness.toml for {id} would not load ({}): rename the file it names, fix what \
             you added to the harness.toml there, or write the tool by hand",
            safe_line(&m)
        ))
    })?;
    Ok(Prepared {
        id: id.to_string(),
        existed: existing.is_some(),
        rel: format!(
            "{}/{}/{id}/{}",
            harness_core::ledger::MIGRATION_DIR,
            hconfig::TOOLS_DIR,
            hconfig::CONFIG_FILE
        ),
        path,
        text: rendered.text,
        library: fresh_program.is_none(),
        picks,
        not_kept,
        shape,
        libs,
        run_name,
        kept: rendered.kept,
        whole_program_off: fresh_program.is_some() && !rendered.whole_program,
        dropped,
    })
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn q_list<S: AsRef<str>>(items: &[S]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| q(s.as_ref()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// What `accept` owns of a tool's `harness.toml` (docs/SCHEMAS.md "The
/// file-list form"): `schema_version`, the whole `[target]` (files and
/// their folders, configuration, map stamp, picks, run name), `[oracle]
/// extra_link_args` and the allowlist's fixed entries.
#[derive(Debug, Clone, Copy)]
pub struct Owned<'a> {
    /// The id.
    pub id: &'a str,
    /// The run name.
    pub run_name: &'a str,
    /// The files and the configuration.
    pub shape: &'a Shape,
    /// The map's `root_hash`.
    pub root_hash: &'a str,
    /// The map's `inputs_hash`.
    pub inputs_hash: &'a str,
    /// The picks.
    pub picks: &'a [PickOut],
    /// The outside libraries the link used.
    pub libs: &'a [String],
    /// A library (no program: no whole-program example).
    pub library: bool,
}

/// A rendered `harness.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Its text.
    pub text: String,
    /// What was carried over from the file there (dotted names).
    pub kept: Vec<String>,
    /// It configures `[oracle.whole_program]` (carried over).
    pub whole_program: bool,
}

/// `key = value` for a kept value (inline form).
fn kept_line(key: &str, value: &toml::Value) -> String {
    format!("{} = {value}\n", toml_key(key))
}

/// A key as TOML writes it: bare when it can be, else quoted.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        key.to_string()
    } else {
        q(key)
    }
}

/// The sub-table at `path` (`["oracle", "whole_program"]`) as TOML text
/// with its headers.
fn kept_table(path: &[&str], table: &toml::Table) -> String {
    let mut wrapped = toml::Value::Table(table.clone());
    for key in path.iter().rev() {
        let mut t = toml::Table::new();
        t.insert((*key).to_string(), wrapped);
        wrapped = toml::Value::Table(t);
    }
    match wrapped {
        toml::Value::Table(t) => toml::to_string(&t).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The tool's `harness.toml` (docs/SCHEMAS.md "The file-list form"):
/// `accept`'s own keys from `owned`, every other key or section of
/// `existing` (the file there, when accepting again) carried over
/// unchanged. Comments are `accept`'s own: the person's are not carried
/// over. A program with no `[oracle.whole_program]` gets a commented
/// example; a file with no `[llm] model` a commented `model =` line.
pub fn render(owned: &Owned<'_>, existing: Option<&toml::Table>) -> Rendered {
    let empty = toml::Table::new();
    let old = existing.unwrap_or(&empty);
    let table_of = |key: &str| old.get(key).and_then(toml::Value::as_table);
    let mut kept: Vec<String> = Vec::new();
    let id = owned.id;
    let shape = owned.shape;
    let mut t = format!(
        "# Written by `harness project accept {id}` from {}.\n\
         # Read it as written (`git diff` shows changes once it is committed). Accepting {id} again rewrites [target] and what the map\n\
         # decides of [oracle], and keeps every other key you add (not its comments).\n\
         schema_version = 2\n",
        mapfile::MAP_FILE,
    );
    // Top-level keys that are no table: before any header.
    for (key, value) in old {
        if matches!(key.as_str(), "schema_version" | "target" | "oracle" | "llm")
            || value.is_table()
        {
            continue;
        }
        t.push_str(&kept_line(key, value));
        kept.push(key.clone());
    }
    t.push_str(&format!(
        "\n[target]\nname = {}\nfiles = [\n",
        q(owned.run_name)
    ));
    for (path, dirs) in &shape.files {
        t.push_str(&format!(
            "  {{ path = {}, include_dirs = {} }},\n",
            q(path),
            q_list(dirs)
        ));
    }
    t.push_str("]\n");
    t.push_str(&format!(
        "configuration = {{ name = {}, from = {}, flags = {} }}\n",
        q(&shape.name),
        q(from_word(shape.from)),
        q_list(&shape.flags)
    ));
    t.push_str(&format!(
        "map = {{ root_hash = {}, inputs_hash = {} }}\n",
        q(owned.root_hash),
        q(owned.inputs_hash)
    ));
    if !owned.picks.is_empty() {
        t.push_str("picks = [\n");
        for p in owned.picks {
            t.push_str(&format!(
                "  {{ definers = {}, keep = {}, by = {} }},\n",
                q_list(&p.definers),
                q(&p.keep),
                q(p.by)
            ));
        }
        t.push_str("]\n");
    }

    // [oracle]: the fixed allowlist, then the person's own entries.
    let oracle = table_of("oracle");
    let mut allow: Vec<String> = ALLOWLIST.iter().map(|s| s.to_string()).collect();
    let mut extra_allowed = Vec::new();
    if let Some(list) = oracle
        .and_then(|o| o.get("allowlist"))
        .and_then(toml::Value::as_array)
    {
        for v in list {
            if let Some(s) = v.as_str() {
                if !allow.iter().any(|a| a == s) {
                    allow.push(s.to_string());
                    extra_allowed.push(s.to_string());
                }
            }
        }
    }
    if !extra_allowed.is_empty() {
        kept.push(format!("oracle.allowlist ({})", extra_allowed.join(", ")));
    }
    t.push_str(&format!("\n[oracle]\nallowlist = {}\n", q_list(&allow)));
    if !owned.libs.is_empty() {
        t.push_str(&format!("extra_link_args = {}\n", q_list(owned.libs)));
    }
    let mut oracle_tables: Vec<(&String, &toml::Table)> = Vec::new();
    if let Some(o) = oracle {
        for (key, value) in o {
            if matches!(key.as_str(), "allowlist" | "extra_link_args") {
                continue;
            }
            match value.as_table() {
                Some(sub) => oracle_tables.push((key, sub)),
                None => t.push_str(&kept_line(key, value)),
            }
            kept.push(format!("oracle.{key}"));
        }
    }
    let whole_program = oracle_tables.iter().any(|(k, _)| *k == "whole_program");
    for (key, sub) in &oracle_tables {
        t.push('\n');
        t.push_str(&kept_table(&["oracle", key], sub));
    }
    if !owned.library && !whole_program {
        t.push_str(
            "\n# The whole-program check is off until this is filled in: verify then runs the C\n\
             # program and its Rust port with these arguments on the same samples and compares\n\
             # what they print. Flags only (at most 4); the sample's path is added last.\n\
             # [oracle.whole_program]\n\
             # args = [\"-c\"]\n",
        );
    }

    // [llm]: the person's, else the defaults.
    t.push_str("\n[llm]\n");
    let mut llm_tables: Vec<(&String, &toml::Table)> = Vec::new();
    let defaults = [
        ("provider", toml::Value::String("external".into())),
        ("max_tokens", toml::Value::Integer(i64::from(MAX_TOKENS))),
    ];
    match table_of("llm") {
        Some(llm) => {
            for (key, value) in llm {
                match value.as_table() {
                    Some(sub) => llm_tables.push((key, sub)),
                    None => t.push_str(&kept_line(key, value)),
                }
                let default = defaults.iter().any(|(k, v)| k == key && v == value);
                if !default {
                    kept.push(format!("llm.{key}"));
                }
            }
        }
        None => {
            for (key, value) in &defaults {
                t.push_str(&kept_line(key, value));
            }
        }
    }
    if !table_of("llm").is_some_and(|l| l.contains_key("model")) {
        t.push_str(&format!(
            "# Who answers the hand-offs, recorded with every attempt (when left out, `{}`):\n\
             # name them, for example the model you answer with, or yourself.\n\
             # model = \"my-claude-code\"\n",
            harness_core::config::LlmSection::default().model
        ));
    }
    for (key, sub) in &llm_tables {
        t.push('\n');
        t.push_str(&kept_table(&["llm", key], sub));
    }
    // Every other section, as it was.
    for (key, value) in old {
        if matches!(key.as_str(), "schema_version" | "target" | "oracle" | "llm") {
            continue;
        }
        if let Some(sub) = value.as_table() {
            t.push('\n');
            t.push_str(&kept_table(&[key], sub));
            kept.push(key.clone());
        }
    }
    Rendered {
        text: t,
        kept,
        whole_program,
    }
}

/// A real folder at `path`, made when absent; a link or a file there is
/// refused.
fn real_dir(path: &Path) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(refuse(format!(
            "{} is not a folder (a link or a file), so no tool is written there: move it away",
            path.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(|e| Error::io(path, e))
        }
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Write the prepared `harness.toml` (atomically), making
/// `migration/tools/<id>/` when absent; never through a link. Only this
/// one file is written: an existing tool keeps its ledger.
pub fn write(root: &Path, prepared: &Prepared) -> Result<(), Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    let migration = root.join(harness_core::ledger::MIGRATION_DIR);
    real_dir(&migration)?;
    let tools = hconfig::tools_dir(&root);
    real_dir(&tools)?;
    let dir = hconfig::tool_dir(&root, &prepared.id);
    real_dir(&dir)?;
    let path = dir.join(hconfig::CONFIG_FILE);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(refuse(format!(
            "{} is not a regular file (a link or a folder), so it is not overwritten: move it \
             away",
            path.display()
        )));
    }
    harness_core::ledger::write_atomic(&path, prepared.text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keep_is_a_set_and_a_value() {
        assert_eq!(
            parse_keep("d1=d1.2").unwrap(),
            ("d1".to_string(), "d1.2".to_string())
        );
        assert_eq!(
            parse_keep("d12=src/a b.c").unwrap(),
            ("d12".to_string(), "src/a b.c".to_string())
        );
        for bad in ["d1", "x1=d1.1", "d=d1.1", "d1=", "dx=a.c", "=a.c"] {
            assert!(parse_keep(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_run_name_is_a_plain_file_name() {
        for good in ["zopfli", "zopfli_bin", "lz4.1", "a-b"] {
            assert!(is_run_name(good), "{good}");
        }
        for bad in ["", ".x", "-x", "a/b", "a b", &"x".repeat(65)] {
            assert!(!is_run_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_rendered_target_loads_and_quotes_every_string() {
        let shape = Shape {
            files: vec![
                ("src/a.c".into(), vec!["include".into()]),
                ("src/b.c".into(), vec![]),
            ],
            name: "plain".into(),
            from: ConfigurationFrom::Stated,
            flags: vec!["-DX=\"y\"".into(), "-idirafterinclude".into()],
        };
        let picks = [PickOut {
            set: "d1".into(),
            definers: vec!["src/a.c".into(), "src/c.c".into()],
            keep: "src/a.c".into(),
            by: "person",
            symbols: vec!["f".into()],
        }];
        let (root_hash, inputs_hash) = (
            format!("blake3:{}", "a".repeat(64)),
            format!("blake3:{}", "b".repeat(64)),
        );
        let libs = ["-lm".to_string()];
        let owned = Owned {
            id: "t-a",
            run_name: "a",
            shape: &shape,
            root_hash: &root_hash,
            inputs_hash: &inputs_hash,
            picks: &picks,
            libs: &libs,
            library: false,
        };
        let rendered = render(&owned, None);
        assert!(rendered.kept.is_empty() && !rendered.whole_program);
        let text = rendered.text;
        let table: toml::Table = text.parse().unwrap();
        let config = hconfig::TargetConfig::from_table(table).unwrap();
        let list = config.target.file_list().unwrap();
        assert_eq!(config.target.name, "a");
        assert_eq!(list.files.len(), 2);
        assert_eq!(list.files[0].include_dirs, ["include"]);
        assert_eq!(list.configuration.flags, ["-DX=\"y\"", "-idirafterinclude"]);
        assert_eq!(list.picks[0].keep, "src/a.c");
        assert_eq!(list.picks[0].by, "person");
        assert!(list.map.is_some());
        assert_eq!(config.oracle_allowlist(), ALLOWLIST);
        assert!(text.contains("extra_link_args = [\"-lm\"]"), "{text}");
        assert_eq!(config.llm.provider, "external");
        assert_eq!(config.llm.max_tokens, MAX_TOKENS);
        // The whole-program check and the model, as commented examples.
        assert!(
            text.contains("# [oracle.whole_program]\n# args = [\"-c\"]\n"),
            "{text}"
        );
        assert!(text.contains("# model = \"my-claude-code\"\n"), "{text}");
        assert!(config.oracle.get("whole_program").is_none());

        // Accepted again over a file the person added to: their keys and
        // sections are carried over unchanged, accept's own are rewritten.
        let mut old: toml::Table = text.parse().unwrap();
        let person: toml::Table = "schema_version = 2\n\
             [target]\nname = \"old\"\nfiles = []\n\
             [oracle]\nallowlist = [\"cc\", \"rustfmt\"]\nextra_link_args = [\"-lz\"]\n\
             timeout_secs = 30\n\
             [oracle.whole_program]\nargs = [\"-9\"]\n\
             [llm]\nprovider = \"external\"\nmax_tokens = 16384\nmodel = \"me\"\n\
             [llm.driver]\nmax_repairs = 2\n\
             [driver]\nmax_mutants = 24\n"
            .parse()
            .unwrap();
        old.extend(person);
        let again = render(&owned, Some(&old));
        let config =
            hconfig::TargetConfig::from_table(again.text.parse().unwrap()).unwrap_or_else(|e| {
                panic!("{e}\n{}", again.text);
            });
        assert_eq!(config.target.name, "a");
        assert_eq!(
            config.oracle_allowlist(),
            ["cc", "cargo", "rustc", "nm", "rustfmt"]
        );
        assert_eq!(
            config.oracle["whole_program"]["args"],
            toml::Value::Array(vec![toml::Value::String("-9".into())])
        );
        assert_eq!(config.oracle["timeout_secs"], toml::Value::Integer(30));
        assert_eq!(config.llm.model, "me");
        assert_eq!(config.llm.driver.as_ref().unwrap().max_repairs, Some(2));
        assert!(again.text.contains("extra_link_args = [\"-lm\"]"));
        assert!(
            !again.text.contains("# [oracle.whole_program]"),
            "{}",
            again.text
        );
        assert!(!again.text.contains("# model ="), "{}", again.text);
        assert!(again.whole_program);
        assert_eq!(
            again.kept,
            [
                "oracle.allowlist (rustfmt)",
                "oracle.timeout_secs",
                "oracle.whole_program",
                "llm.driver",
                "llm.model",
                "driver"
            ]
        );
    }

    #[test]
    fn a_pick_is_said_in_words() {
        let p = PickOut {
            set: "d1".into(),
            definers: vec!["src/decode.c".into(), "src/mini.c".into()],
            keep: "src/mini.c".into(),
            by: "person",
            symbols: vec!["lzg_decode".into(), "lzg_size".into()],
        };
        assert_eq!(
            p.words(),
            "keeping `src/mini.c` over `src/decode.c` for `lzg_decode, lzg_size`"
        );
    }
}
