//! The build evidence (docs/PROJECT-MAP-DESIGN.md §3.1 step 4): a
//! `compile_commands.json` at the root or one level down, **read, never
//! executed**, and the build files found by their fixed names. Nothing the
//! project ships is run.
//!
//! The file is read into typed entries (only `directory`, `file`,
//! `arguments`, `command`), at most [`MAX_ENTRIES`] of them, at most
//! [`MAX_ENTRY_FLAGS`] flags and [`MAX_ENTRY_BYTES`] of flags kept an entry;
//! the rest is counted as ignored. An entry's `command` is split by POSIX
//! shell word rules with no expansion (or its `arguments` taken as they
//! are); its compiler (the first word), its `output` and its bookkeeping
//! (`-c`, `-o`, the `-M` family) are never used — `cc` from the allowlist
//! always compiles. A
//! separate-form option takes its next argument and is checked joined, so a
//! refused option drops its value with it; a path is resolved against the
//! entry's `directory` and must land inside the project root. What passes
//! the grammar is kept per file, root-relative; the rest is counted and
//! named.

use super::config::resolves_inside;
use super::rel_of;
use harness_core::config::flags::check_flag;
use harness_core::text::safe_line;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

/// The file's fixed name.
pub const COMPILE_COMMANDS: &str = "compile_commands.json";
/// A `compile_commands.json` larger than this is not read.
pub const MAX_COMPILE_COMMANDS_BYTES: u64 = 64 << 20;
/// Most distinct ignored flags named; past it they are only counted.
pub const MAX_NAMED_IGNORED: usize = 200;
/// Most entries read (§3.10); the rest are counted as ignored entries.
pub const MAX_ENTRIES: usize = 50_000;
/// Most flags kept from one entry (§3.10); the rest are counted as ignored.
pub const MAX_ENTRY_FLAGS: usize = 64;
/// Most bytes of flags kept from one entry (§3.10); the rest are counted as
/// ignored.
pub const MAX_ENTRY_BYTES: usize = 16 << 10;

/// Options whose value is the next argument (and checked joined to it).
pub const SEPARATE_FORM: &[&str] = &[
    "-I",
    "-D",
    "-U",
    "-include",
    "-imacros",
    "-iquote",
    "-isystem",
    "-idirafter",
    "-F",
    "-L",
    "-o",
    "-MF",
    "-MT",
    "-MQ",
    "-x",
    "-arch",
    "-isysroot",
    "-target",
    "-Xclang",
    "-Xpreprocessor",
    "-mllvm",
];

/// The grammar's path flags, longest first (as [`check_flag`] reads them).
pub(crate) const PATH_PREFIXES: &[&str] = harness_core::config::flags::PATH_FLAGS;

/// Build files by their fixed names (`*.mk` by its extension).
pub const BUILD_FILE_NAMES: &[&str] = &[
    "Makefile",
    "GNUmakefile",
    "CMakeLists.txt",
    "configure",
    "configure.ac",
    "meson.build",
];

/// Whether a `compile_commands.json` was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileCommands {
    /// None at the root or one level down.
    Absent,
    /// Read: its path relative to the root.
    Present {
        /// Relative to the root.
        path: String,
    },
    /// Found and not read: too large, not JSON, not a list.
    Unreadable {
        /// Relative to the root.
        path: String,
        /// Why, in words.
        why: String,
    },
}

/// A file listed twice or more with different flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagsDiffer {
    /// Relative to the root.
    pub path: String,
    /// Each distinct list of kept flags, in the file's order.
    pub flags: Vec<Vec<String>>,
}

/// A flag of an entry that was not kept, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredFlag {
    /// As the entry wrote it (a separate-form option with its value after a
    /// blank).
    pub flag: String,
    /// The grammar's sentence.
    pub why: String,
    /// How many entries carried it.
    pub count: usize,
}

/// What the project's build says, read and never run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildEvidence {
    /// The `compile_commands.json` read, if any.
    pub compile_commands: CompileCommands,
    /// Other `compile_commands.json` files one level down, not read.
    pub also_found: Vec<String>,
    /// Entries not used: a `directory` outside the root or unresolvable, no
    /// file, no command, a command that does not split.
    pub ignored_entries: usize,
    /// Files entries name that the walk did not find (never compiled):
    /// relative to the root when inside it, else as written; sorted.
    pub unfound_entries: Vec<String>,
    /// Build files by fixed name, relative to the root, sorted.
    pub build_files: Vec<String>,
    /// Files listed twice with different flags, sorted by path.
    pub flags_differ: Vec<FlagsDiffer>,
    /// Flags not kept, named (at most [`MAX_NAMED_IGNORED`] distinct),
    /// sorted.
    pub ignored_flags: Vec<IgnoredFlag>,
    /// Every flag not kept, counted.
    pub ignored_flag_count: usize,
    /// The kept flags of each walked file's first entry, root-relative.
    pub file_flags: BTreeMap<String, Vec<String>>,
    /// Walked `.c` files no entry lists (when a `compile_commands.json` was
    /// read), sorted: compiled with the configuration's flags alone, and a
    /// closure holding one keeps its configuration a guess.
    pub not_in_compile_commands: Vec<String>,
}

impl BuildEvidence {
    /// Nothing found.
    pub fn none(build_files: Vec<String>) -> BuildEvidence {
        BuildEvidence {
            compile_commands: CompileCommands::Absent,
            also_found: Vec::new(),
            ignored_entries: 0,
            unfound_entries: Vec::new(),
            build_files,
            flags_differ: Vec::new(),
            ignored_flags: Vec::new(),
            ignored_flag_count: 0,
            file_flags: BTreeMap::new(),
            not_in_compile_commands: Vec::new(),
        }
    }

    /// A `compile_commands.json` was read.
    pub fn has_compile_commands(&self) -> bool {
        matches!(self.compile_commands, CompileCommands::Present { .. })
    }
}

/// A build file by name.
pub fn is_build_file(name: &str) -> bool {
    BUILD_FILE_NAMES.contains(&name) || name.ends_with(".mk")
}

/// Gather the evidence: find and read the `compile_commands.json` (entries
/// for files in `walked`, relative to `root`), with `build_files` from the
/// walk. `false` beside it when `deadline` passed before every entry was
/// read (the map then stops at its time budget).
pub(crate) fn gather(
    root: &Path,
    walked: &BTreeSet<&str>,
    build_files: Vec<String>,
    deadline: Instant,
) -> (BuildEvidence, bool) {
    let mut ev = BuildEvidence::none(build_files);
    let mut found = find(root);
    if found.is_empty() {
        return (ev, true);
    }
    let (path, rel) = found.remove(0);
    ev.also_found = found.into_iter().map(|(_, r)| r).collect();
    let unreadable = |why: String| CompileCommands::Unreadable {
        path: rel.clone(),
        why,
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_COMPILE_COMMANDS_BYTES {
        ev.compile_commands = unreadable(format!("over {} MiB", MAX_COMPILE_COMMANDS_BYTES >> 20));
        return (ev, true);
    }
    let entries: Entries = match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(why) => {
            ev.compile_commands = unreadable(format!("not a list of entries: {why}"));
            return (ev, true);
        }
    };
    ev.compile_commands = CompileCommands::Present { path: rel };
    ev.ignored_entries = entries.past_cap;
    let base = path.parent().unwrap_or(root).to_path_buf();
    let mut lists: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut seen: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    let mut unfound = BTreeSet::new();
    let mut ignored: BTreeMap<(String, String), usize> = BTreeMap::new();
    for entry in &entries.kept {
        if Instant::now() >= deadline {
            return (ev, false);
        }
        let Some(read) = read_entry(root, &base, entry) else {
            ev.ignored_entries += 1;
            continue;
        };
        ev.ignored_flag_count += read.past_cap;
        for (flag, why) in read.refused {
            ev.ignored_flag_count += 1;
            let key = (flag, why);
            if let Some(n) = ignored.get_mut(&key) {
                *n += 1;
            } else if ignored.len() < MAX_NAMED_IGNORED {
                ignored.insert(key, 1);
            }
        }
        match read.file {
            Some(rel) if walked.contains(rel.as_str()) => {
                // A set beside the list: a file listed thousands of times
                // is checked in log time, not by a scan of the list.
                if seen
                    .entry(rel.clone())
                    .or_default()
                    .insert(read.kept.clone())
                {
                    lists.entry(rel).or_default().push(read.kept);
                }
            }
            Some(rel) => {
                unfound.insert(rel);
            }
            None => {
                unfound.insert(read.written);
            }
        }
    }
    ev.unfound_entries = unfound.into_iter().collect();
    ev.ignored_flags = ignored
        .into_iter()
        .map(|((flag, why), count)| IgnoredFlag { flag, why, count })
        .collect();
    for (path, mut flags) in lists {
        if flags.len() > 1 {
            ev.flags_differ.push(FlagsDiffer {
                path: path.clone(),
                flags: flags.clone(),
            });
        }
        ev.file_flags.insert(path, flags.swap_remove(0));
    }
    (ev, true)
}

/// The entries of a `compile_commands.json`, read typed: the first
/// [`MAX_ENTRIES`] kept, the rest counted (never held in memory).
struct Entries {
    kept: Vec<RawEntry>,
    past_cap: usize,
}

/// One entry as written: only the fields the map reads, each `None` when
/// absent or not of its type (the entry is then ignored and counted).
#[derive(Default)]
struct RawEntry {
    directory: Option<String>,
    file: Option<String>,
    /// `None` when absent or not a list of strings.
    arguments: Option<Vec<String>>,
    /// Arguments past [`MAX_RAW_ARGS`], counted.
    arguments_past_cap: usize,
    /// The `arguments` field was present (even when not a list of strings).
    has_arguments: bool,
    command: Option<String>,
}

/// Most arguments of one entry held in memory before the flag caps.
const MAX_RAW_ARGS: usize = 4 * MAX_ENTRY_FLAGS + 16;

impl<'de> serde::Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Entries, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Entries;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of entries")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Entries, A::Error> {
                let mut out = Entries {
                    kept: Vec::new(),
                    past_cap: 0,
                };
                while out.kept.len() < MAX_ENTRIES {
                    match seq.next_element::<RawEntry>()? {
                        Some(e) => out.kept.push(e),
                        None => return Ok(out),
                    }
                }
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    out.past_cap += 1;
                }
                Ok(out)
            }
        }
        d.deserialize_seq(V)
    }
}

impl<'de> serde::Deserialize<'de> for RawEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<RawEntry, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = RawEntry;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an entry")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<RawEntry, A::Error> {
                let mut e = RawEntry::default();
                while let Some(key) = map.next_key::<Lenient>()? {
                    match key.0.as_deref() {
                        Some("directory") => e.directory = map.next_value::<Lenient>()?.0,
                        Some("file") => e.file = map.next_value::<Lenient>()?.0,
                        Some("command") => e.command = map.next_value::<Lenient>()?.0,
                        Some("arguments") => {
                            let args = map.next_value::<Args>()?;
                            e.has_arguments = true;
                            e.arguments = args.kept;
                            e.arguments_past_cap = args.past_cap;
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(e)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<RawEntry, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(RawEntry::default())
            }
            fn visit_str<E>(self, _: &str) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
            fn visit_bool<E>(self, _: bool) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
            fn visit_i64<E>(self, _: i64) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
            fn visit_u64<E>(self, _: u64) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
            fn visit_f64<E>(self, _: f64) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
            fn visit_unit<E>(self) -> Result<RawEntry, E> {
                Ok(RawEntry::default())
            }
        }
        d.deserialize_any(V)
    }
}

/// A string, or `None` for a value of any other type (skipped unread).
struct Lenient(Option<String>);

impl<'de> serde::Deserialize<'de> for Lenient {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Lenient, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Lenient;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any value")
            }
            fn visit_str<E>(self, v: &str) -> Result<Lenient, E> {
                Ok(Lenient(Some(v.to_string())))
            }
            fn visit_string<E>(self, v: String) -> Result<Lenient, E> {
                Ok(Lenient(Some(v)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Lenient, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Lenient(None))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Lenient, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(Lenient(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Lenient, E> {
                Ok(Lenient(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Lenient, E> {
                Ok(Lenient(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Lenient, E> {
                Ok(Lenient(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Lenient, E> {
                Ok(Lenient(None))
            }
            fn visit_unit<E>(self) -> Result<Lenient, E> {
                Ok(Lenient(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// An `arguments` list: the first [`MAX_RAW_ARGS`] strings kept, the rest
/// counted; `kept: None` when it is not a list of strings.
struct Args {
    kept: Option<Vec<String>>,
    past_cap: usize,
}

impl<'de> serde::Deserialize<'de> for Args {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Args, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Args;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of arguments")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Args, A::Error> {
                let mut kept = Vec::new();
                let mut past_cap = 0;
                let mut all_strings = true;
                while let Some(arg) = seq.next_element::<Lenient>()? {
                    match arg.0 {
                        None => all_strings = false,
                        Some(_) if kept.len() >= MAX_RAW_ARGS => past_cap += 1,
                        Some(a) => kept.push(a),
                    }
                }
                Ok(Args {
                    kept: all_strings.then_some(kept),
                    past_cap,
                })
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Args, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_str<E>(self, _: &str) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_bool<E>(self, _: bool) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_i64<E>(self, _: i64) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_u64<E>(self, _: u64) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_f64<E>(self, _: f64) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
            fn visit_unit<E>(self) -> Result<Args, E> {
                Ok(Args {
                    kept: None,
                    past_cap: 0,
                })
            }
        }
        d.deserialize_any(V)
    }
}

/// `compile_commands.json` at the root, then in each folder one level down
/// (sorted; dot-folders and `migration/` skipped), each a plain file whose
/// real path is inside the root: `(real path, path relative to the root)`.
fn find(root: &Path) -> Vec<(PathBuf, String)> {
    let mut candidates = vec![root.join(COMPILE_COMMANDS)];
    let mut below: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.')
                && name != harness_core::ledger::MIGRATION_DIR
                && e.path().is_dir()
        })
        .map(|e| e.path().join(COMPILE_COMMANDS))
        .collect();
    below.sort();
    candidates.extend(below);
    let ledger = root.join(harness_core::ledger::MIGRATION_DIR);
    candidates
        .into_iter()
        .filter_map(|p| {
            let real = p.canonicalize().ok()?;
            (real.is_file() && real.starts_with(root) && !real.starts_with(&ledger))
                .then(|| rel_of(root, &p).map(|rel| (real, rel)))
                .flatten()
        })
        .collect()
}

/// One entry, read.
struct Entry {
    /// The file, relative to the root (`None` when outside it).
    file: Option<String>,
    /// The file as written (for an outside one).
    written: String,
    /// The flags kept, root-relative.
    kept: Vec<String>,
    /// The flags refused: as written, and why.
    refused: Vec<(String, String)>,
    /// Arguments past the entry's caps, counted as ignored.
    past_cap: usize,
}

/// Read one entry; `None` when it is ignored (counted by the caller).
fn read_entry(root: &Path, base: &Path, entry: &RawEntry) -> Option<Entry> {
    let directory = entry.directory.as_deref()?;
    let written = entry.file.clone()?;
    let (args, mut past_cap): (Vec<String>, usize) = match (&entry.arguments, &entry.command) {
        (Some(a), _) => (a.clone(), entry.arguments_past_cap),
        (None, _) if entry.has_arguments => return None,
        (None, Some(c)) => {
            let mut words = split_command(c).ok()?;
            let past = words.len().saturating_sub(MAX_RAW_ARGS);
            words.truncate(MAX_RAW_ARGS);
            (words, past)
        }
        (None, None) => return None,
    };
    let dir = resolve(base, directory)?;
    if !dir.starts_with(root) || !dir.is_dir() {
        return None;
    }
    let file_abs = resolve(&dir, &written);
    let file = file_abs.as_deref().and_then(|p| rel_of(root, p));
    let (kept, refused, over) = entry_flags(root, &dir, &args, file_abs.as_deref());
    past_cap += over;
    Some(Entry {
        file,
        written,
        kept,
        refused,
        past_cap,
    })
}

/// `path` (absolute, or relative to `base`) made absolute: the deepest part
/// that exists canonical, the rest added lexically. `None` when a `..`
/// follows a part that does not exist.
fn resolve(base: &Path, path: &str) -> Option<PathBuf> {
    let joined = base.join(path);
    if let Ok(real) = joined.canonicalize() {
        return Some(real);
    }
    let mut existing = joined.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        let last = existing.components().next_back()?;
        match last {
            Component::Normal(p) => rest.push(p.to_os_string()),
            Component::CurDir => {}
            _ => return None,
        }
        if !existing.pop() {
            return None;
        }
    }
}

/// The build's own bookkeeping options, dropped silently with their values
/// like `-c`: the object's name and the dependency-list options every
/// CMake and Meson entry carries (each entry's `-o` differs, so naming them
/// would flood the screen).
const BOOKKEEPING: &[&str] = &["-o", "-MF", "-MT", "-MQ", "-MD", "-MMD"];

/// An entry's arguments, its compiler dropped: `(kept, refused, past the
/// caps)`. `-c`, the build's bookkeeping ([`BOOKKEEPING`]) and the entry's
/// own file are expected and dropped silently. Past [`MAX_ENTRY_FLAGS`]
/// kept flags or [`MAX_ENTRY_BYTES`] of them, every further argument is
/// counted as ignored.
fn entry_flags(
    root: &Path,
    dir: &Path,
    args: &[String],
    file_abs: Option<&Path>,
) -> (Vec<String>, Vec<(String, String)>, usize) {
    let mut kept: Vec<String> = Vec::new();
    let mut kept_bytes = 0;
    let mut refused = Vec::new();
    let mut past_cap = 0;
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "-c" || arg == "-MD" || arg == "-MMD" {
            continue;
        }
        let separate = SEPARATE_FORM.contains(&arg.as_str());
        if BOOKKEEPING.contains(&arg.as_str()) {
            if separate {
                it.next();
            }
            continue;
        }
        let (joined, shown) = if separate {
            match it.next() {
                Some(value) => (format!("{arg}{value}"), format!("{arg} {value}")),
                None => (arg.clone(), arg.clone()),
            }
        } else {
            (arg.clone(), arg.clone())
        };
        // The joined spellings: `-ofoo.o`, `-MFdeps.d`, `-MTx`, `-MQx`.
        if ["-o", "-MF", "-MT", "-MQ"]
            .iter()
            .any(|b| joined.starts_with(b))
        {
            continue;
        }
        if !joined.starts_with(['-', '@'])
            && file_abs.is_some()
            && resolve(dir, &joined).as_deref() == file_abs
        {
            continue;
        }
        if kept.len() >= MAX_ENTRY_FLAGS || kept_bytes >= MAX_ENTRY_BYTES {
            past_cap += 1;
            continue;
        }
        match entry_flag(root, dir, &joined) {
            Ok(flag) => {
                kept_bytes += flag.len();
                kept.push(flag);
            }
            Err(why) => refused.push((shown, why)),
        }
    }
    (kept, refused, past_cap)
}

/// One joined flag of an entry through the grammar; a path flag's path
/// resolved against `dir` and rewritten relative to the root.
fn entry_flag(root: &Path, dir: &Path, joined: &str) -> Result<String, String> {
    for prefix in PATH_PREFIXES {
        let Some(value) = joined.strip_prefix(prefix) else {
            continue;
        };
        if value.is_empty() || value.starts_with(['@', '-']) || value.chars().any(char::is_control)
        {
            // The grammar's own sentence for these.
            check_flag(joined)?;
        }
        let refuse = || {
            format!(
                "the flag `{}` names a path outside the project; only folders and files inside \
                 it are passed",
                safe_line(joined)
            )
        };
        let abs = resolve(dir, value).ok_or_else(refuse)?;
        let rel = rel_of(root, &abs).ok_or_else(refuse)?;
        let rewritten = format!("{prefix}{rel}");
        check_flag(&rewritten)?;
        if !resolves_inside(root, &rel) {
            return Err(refuse());
        }
        return Ok(rewritten);
    }
    check_flag(joined).map(|_| joined.to_string())
}

/// Split a `command` by POSIX shell word rules with **no** expansion: blanks
/// separate words; `'…'` is literal; inside `"…"` a backslash escapes only
/// `$`, `` ` ``, `"`, `\` and a newline; outside quotes a backslash escapes
/// the next character and a backslash-newline joins lines. `$`, `` ` ``, `*`,
/// `?`, `[` and `~` stay as they are. An unclosed quote is an error.
pub fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(n) => {
                    cur.push(n);
                    started = true;
                }
                None => {
                    cur.push('\\');
                    started = true;
                }
            },
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(n) => cur.push(n),
                        None => return Err("a single quote is not closed".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('\n') => {}
                            Some(n @ ('$' | '`' | '"' | '\\')) => cur.push(n),
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                            None => return Err("a double quote is not closed".into()),
                        },
                        Some(n) => cur.push(n),
                        None => return Err("a double quote is not closed".into()),
                    }
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(cur);
    }
    Ok(words)
}
