//! The build evidence (docs/PROJECT-MAP-DESIGN.md §3.1 step 4): a
//! `compile_commands.json` at the root or one level down, **read, never
//! executed**, and the build files found by their fixed names. Nothing the
//! project ships is run.
//!
//! An entry's `command` is split by POSIX shell word rules with no expansion
//! (or its `arguments` taken as they are); its compiler (the first word) and
//! its `output` are never used — `cc` from the allowlist always compiles. A
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

/// The file's fixed name.
pub const COMPILE_COMMANDS: &str = "compile_commands.json";
/// A `compile_commands.json` larger than this is not read.
pub const MAX_COMPILE_COMMANDS_BYTES: u64 = 64 << 20;
/// Most distinct ignored flags named; past it they are only counted.
pub const MAX_NAMED_IGNORED: usize = 200;

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
pub(crate) const PATH_PREFIXES: &[&str] = &["-isystem", "-include", "-iquote", "-I"];

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
/// walk.
pub(crate) fn gather(
    root: &Path,
    walked: &BTreeSet<&str>,
    build_files: Vec<String>,
) -> BuildEvidence {
    let mut ev = BuildEvidence::none(build_files);
    let mut found = find(root);
    if found.is_empty() {
        return ev;
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
        return ev;
    }
    let entries: Vec<serde_json::Value> = match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(why) => {
            ev.compile_commands = unreadable(format!("not a list of entries: {why}"));
            return ev;
        }
    };
    ev.compile_commands = CompileCommands::Present { path: rel };
    let base = path.parent().unwrap_or(root).to_path_buf();
    let mut lists: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut unfound = BTreeSet::new();
    let mut ignored: BTreeMap<(String, String), usize> = BTreeMap::new();
    for entry in &entries {
        let Some(read) = read_entry(root, &base, entry) else {
            ev.ignored_entries += 1;
            continue;
        };
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
                let at = lists.entry(rel).or_default();
                if !at.contains(&read.kept) {
                    at.push(read.kept);
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
    ev
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
}

/// Read one entry; `None` when it is ignored (counted by the caller).
fn read_entry(root: &Path, base: &Path, entry: &serde_json::Value) -> Option<Entry> {
    let directory = entry.get("directory")?.as_str()?;
    let written = entry.get("file")?.as_str()?.to_string();
    let args: Vec<String> = match (entry.get("arguments"), entry.get("command")) {
        (Some(a), _) => a
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<_>>()?,
        (None, Some(c)) => split_command(c.as_str()?).ok()?,
        (None, None) => return None,
    };
    let dir = resolve(base, directory)?;
    if !dir.starts_with(root) || !dir.is_dir() {
        return None;
    }
    let file_abs = resolve(&dir, &written);
    let file = file_abs.as_deref().and_then(|p| rel_of(root, p));
    let (kept, refused) = entry_flags(root, &dir, &args, file_abs.as_deref());
    Some(Entry {
        file,
        written,
        kept,
        refused,
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

/// An entry's arguments, its compiler dropped: `(kept, refused)`. `-c` and
/// the entry's own file are expected and dropped silently.
fn entry_flags(
    root: &Path,
    dir: &Path,
    args: &[String],
    file_abs: Option<&Path>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut kept = Vec::new();
    let mut refused = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "-c" {
            continue;
        }
        let (joined, shown) = if SEPARATE_FORM.contains(&arg.as_str()) {
            match it.next() {
                Some(value) => (format!("{arg}{value}"), format!("{arg} {value}")),
                None => (arg.clone(), arg.clone()),
            }
        } else {
            (arg.clone(), arg.clone())
        };
        if !joined.starts_with(['-', '@'])
            && file_abs.is_some()
            && resolve(dir, &joined).as_deref() == file_abs
        {
            continue;
        }
        match entry_flag(root, dir, &joined) {
            Ok(flag) => kept.push(flag),
            Err(why) => refused.push((shown, why)),
        }
    }
    (kept, refused)
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
