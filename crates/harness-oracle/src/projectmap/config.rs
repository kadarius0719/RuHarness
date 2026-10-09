//! The configuration a map is made under (docs/PROJECT-MAP-DESIGN.md §3.2):
//! `migration/map/config.toml`'s `[[configuration]]` entries, the one picked
//! (`--configuration NAME`, else the only entry), or a **guess** when the
//! file holds none. Every flag goes through the grammar
//! ([`harness_core::config::flags::check_flag`]) and every path flag must
//! resolve inside the project root, links followed, outside `migration/` —
//! whoever wrote it.

use harness_core::config::flags::{self, Flag};
use harness_core::config::{ConfigurationFrom, MAX_CONFIGURATION_NAME};
use harness_core::error::Error;
use harness_core::text::safe_line;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Where the person's configuration file lives, relative to the root.
pub const CONFIG_FILE: &str = "migration/map/config.toml";
/// A configuration file larger than this is refused unread.
pub const MAX_CONFIG_BYTES: u64 = 1 << 20;
/// The name of the guessed configuration.
pub const GUESSED_NAME: &str = "guessed";

/// One `[[configuration]]` entry of `config.toml`, as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigEntry {
    /// `make`, `meson`, `cmake` or the person's own word.
    pub name: String,
    /// What it stands for.
    pub from: ConfigurationFrom,
    /// The flags, in order (`flags = []` states "no flags").
    pub flags: Vec<String>,
    /// Header names the project means the system's (§3.1 step 3).
    #[serde(default)]
    pub system_headers: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFileShape {
    #[serde(default)]
    configuration: Vec<ConfigEntry>,
}

/// Who the map's configuration comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// `config.toml` names `from = "compile_commands"`, a
    /// `compile_commands.json` was read, and no file is listed twice with
    /// other flags.
    CompileCommands,
    /// `config.toml` states it.
    Stated,
    /// Nothing states it: no `config.toml` entry, a `compile_commands.json`
    /// whose flags differ, or a `config.toml` that came with the project
    /// and is only proposed ([`MapConfiguration::proposed`]).
    Guessed,
}

impl ConfigSource {
    /// `compile_commands`, `stated` or `guessed`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSource::CompileCommands => "compile_commands",
            ConfigSource::Stated => "stated",
            ConfigSource::Guessed => "guessed",
        }
    }
}

/// The configuration a map was made under (one at a time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapConfiguration {
    /// Its name; [`GUESSED_NAME`] for a guess.
    pub name: String,
    /// What it stands for. A guess says `compile_commands` when a
    /// `compile_commands.json` was found (its proposal), else `stated` with
    /// no flags.
    pub from: ConfigurationFrom,
    /// Who it comes from.
    pub source: ConfigSource,
    /// Its flags, in order, as written (paths relative to the root). `-O`
    /// levels are recorded here and never applied.
    pub flags: Vec<String>,
    /// Header names the project means the system's.
    pub system_headers: Vec<String>,
    /// blake3 of the canonical JSON of `{flags, from, name, system_headers}`
    /// (`system_headers` only when not empty).
    pub digest: String,
    /// The `config.toml` entry came with the project (the file still has
    /// the hash it had when this computer first recorded the root): shown
    /// as proposed, its source kept `guessed` until the person states it
    /// (`--adopt` once, or an edit of their own).
    pub proposed: bool,
}

impl MapConfiguration {
    /// Each file's compile takes its own `compile_commands.json` entry's
    /// flags (a guess with no `config.toml` entry, or `from =
    /// "compile_commands"`); otherwise the configuration's flags.
    pub fn uses_entry_flags(&self) -> bool {
        (self.source == ConfigSource::Guessed && !self.proposed)
            || self.from == ConfigurationFrom::CompileCommands
    }
}

/// blake3 of the canonical JSON (keys sorted, no blanks) of
/// `{flags, from, name, system_headers}`: flag order counts, and
/// `system_headers` is left out when empty (so a configuration without it
/// keeps the digest it always had).
pub fn digest(
    name: &str,
    from: ConfigurationFrom,
    flags: &[String],
    system_headers: &[String],
) -> String {
    // Fields in key order: serde writes them as declared.
    #[derive(serde::Serialize)]
    struct Canonical<'a> {
        flags: &'a [String],
        from: ConfigurationFrom,
        name: &'a str,
        #[serde(skip_serializing_if = "<[String]>::is_empty")]
        system_headers: &'a [String],
    }
    let json = serde_json::to_vec(&Canonical {
        flags,
        from,
        name,
        system_headers,
    })
    .unwrap_or_else(|_| unreachable!("strings and a unit enum always serialise"));
    harness_core::hash::bytes_hash(&json)
}

/// `config.toml`'s entries and the hash of its bytes (`blake3:<hex>`), which
/// is compared with the hash recorded when this computer first saw the root
/// (a file that came with the project is proposed, §3.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigFile {
    /// The entries, checked.
    pub entries: Vec<ConfigEntry>,
    /// The file's hash; `None` when there is no file.
    pub hash: Option<String>,
}

/// A refusal of `config.toml`, led by its path relative to the root.
fn refused(message: impl std::fmt::Display) -> Error {
    Error::Invariant(format!("{CONFIG_FILE}: {message}"))
}

/// Read `config.toml`'s entries (none when the file is absent), each checked:
/// its name, its flags through the grammar with their paths resolved, its
/// `system_headers`. A file reached through a link out of the root, too
/// large, unparseable, two entries of one name, or a bad entry is refused in
/// one sentence.
pub fn read_entries(root: &Path) -> Result<ConfigFile, Error> {
    let path = root.join(CONFIG_FILE);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigFile::default()),
        Err(e) => return Err(Error::io(&path, e)),
    };
    let real = path.canonicalize().map_err(|e| Error::io(&path, e))?;
    if !real.starts_with(root) || !meta.is_file() {
        return Err(refused(
            "the configuration file is not a plain file inside the project; write it at \
             migration/map/config.toml",
        ));
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(refused(format!(
            "the configuration file is over {} KiB; keep only its [[configuration]] entries",
            MAX_CONFIG_BYTES >> 10
        )));
    }
    let bytes = std::fs::read(&real).map_err(|e| Error::io(&path, e))?;
    let hash = harness_core::hash::bytes_hash(&bytes);
    let text = String::from_utf8(bytes)
        .map_err(|_| refused("the configuration file is not UTF-8; fix it"))?;
    // toml's message can quote a key holding a newline: one line, with the
    // line it stopped at.
    let shape: ConfigFileShape = toml::from_str(&text).map_err(|e| {
        let line = e.span().map(|s| {
            text.as_bytes()[..s.start.min(text.len())]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1
        });
        refused(format!(
            "{}{}; fix the file (its shape: one [[configuration]] table per build, with name, \
             from and flags, as docs/SCHEMAS.md shows)",
            line.map(|l| format!("line {l}: ")).unwrap_or_default(),
            safe_line(e.message())
        ))
    })?;
    let mut seen = std::collections::BTreeSet::new();
    for entry in &shape.configuration {
        check_entry(root, entry).map_err(refused)?;
        if !seen.insert(entry.name.as_str()) {
            return Err(refused(format!(
                "two [[configuration]] entries are named `{}`; give each its own name",
                safe_line(&entry.name)
            )));
        }
    }
    Ok(ConfigFile {
        entries: shape.configuration,
        hash: Some(hash),
    })
}

/// One entry's checks.
fn check_entry(root: &Path, entry: &ConfigEntry) -> Result<(), String> {
    let name_ok = !entry.name.is_empty()
        && entry.name.len() <= MAX_CONFIGURATION_NAME
        && entry
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if !name_ok {
        return Err(format!(
            "the configuration name `{}` must be a word of letters, digits, _ - or . (at most \
             {MAX_CONFIGURATION_NAME}); rename it",
            safe_line(&entry.name)
        ));
    }
    check_flags(root, &entry.flags)?;
    for header in &entry.system_headers {
        let clean = harness_core::plan::is_clean_relative_path(header)
            && !header.chars().any(char::is_control);
        if !clean {
            return Err(format!(
                "the system header `{}` of configuration `{}` must be a header name like \
                 unistd.h or sys/types.h; fix it",
                safe_line(header),
                safe_line(&entry.name)
            ));
        }
    }
    Ok(())
}

/// A flag the map has no use for: a warning, debug-information or tuning
/// flag (`-Wall`, `-W`, `-w`, `-pedantic`, `-g`, `-funroll-loops`,
/// `-march=native`, `-pipe`). It is refused like any flag outside the
/// grammar, and named as one to drop. A flag that hands something to
/// another tool or names a file (`-Wl,…`, `-fplugin=…`, `-fuse-ld=…`, any
/// path) is no such flag: it keeps the grammar's own sentence.
fn droppable(flag: &str) -> bool {
    if flag.contains('/') || ["-Wl,", "-Wa,", "-Wp,"].iter().any(|p| flag.starts_with(p)) {
        return false;
    }
    let tuning_f = flag.starts_with("-f") && !flag.contains('=');
    ["-W", "-pedantic", "-g", "-march=", "-mtune=", "-mcpu="]
        .iter()
        .any(|p| flag.starts_with(p))
        || tuning_f
        || matches!(flag, "-w" | "-pipe")
}

/// Every flag through the grammar, and every path flag's path resolved
/// against `root` with links followed: inside the root, outside
/// `migration/`. `Err` names every refused flag in one message: a path flag
/// written with a blank (`-I src/include`) is told to be written joined, and
/// warning and tuning flags are named together as ones to drop.
pub fn check_flags(root: &Path, list: &[String]) -> Result<(), String> {
    let mut refused: Vec<String> = Vec::new();
    let mut drop: Vec<String> = Vec::new();
    for flag in list {
        let shown = safe_line(flag);
        if let Some((prefix, rest)) = flags::split_path_flag(flag) {
            let joined = rest.trim_start();
            if rest.starts_with(char::is_whitespace) && !joined.is_empty() {
                refused.push(format!(
                    "`{shown}` has a blank after {prefix}: write it joined, like \
                     {prefix}{}",
                    safe_line(joined)
                ));
                continue;
            }
        }
        match flags::check_flag(flag) {
            Err(_) if droppable(flag) => drop.push(format!("`{shown}`")),
            Err(why) => refused.push(why),
            Ok(Flag::Path(path)) if !resolves_inside(root, path) => refused.push(format!(
                "the flag `{shown}` names a path that leaves the project or reaches migration/ \
                 through a link; name the real folder inside the project"
            )),
            Ok(_) => {}
        }
    }
    if !drop.is_empty() {
        refused.push(format!(
            "{} {} warning or tuning flag{} the harness does not pass: drop {}, the map does \
             not need {}",
            drop.join(", "),
            if drop.len() == 1 { "is a" } else { "are" },
            if drop.len() == 1 { "" } else { "s" },
            if drop.len() == 1 { "it" } else { "them" },
            if drop.len() == 1 { "it" } else { "them" },
        ));
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(refused.join("; "))
    }
}

/// `rel` resolved against `root`: the deepest part that exists, links
/// followed, lies inside the root and outside its `migration/`.
pub(crate) fn resolves_inside(root: &Path, rel: &str) -> bool {
    let ledger = root.join(harness_core::ledger::MIGRATION_DIR);
    let mut probe: PathBuf = root.join(rel);
    loop {
        match probe.canonicalize() {
            Ok(real) => return real.starts_with(root) && !real.starts_with(&ledger),
            Err(_) => {
                if !probe.pop() {
                    return false;
                }
            }
        }
    }
}

/// Pick the configuration: `wanted` by name, else the only entry; several
/// entries and no name are refused naming them; no entry is a guess.
/// `compile_commands` says a `compile_commands.json` was read,
/// `flags_differ` that it lists a file twice with other flags, and
/// `shipped` that `config.toml` came with the project (its entry is then
/// proposed, the source `guessed`).
pub fn choose(
    entries: &[ConfigEntry],
    wanted: Option<&str>,
    compile_commands: bool,
    flags_differ: bool,
    shipped: bool,
) -> Result<MapConfiguration, String> {
    let names = || {
        entries
            .iter()
            .map(|e| safe_line(&e.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let entry = match (wanted, entries) {
        (Some(want), _) => match entries.iter().find(|e| e.name == want) {
            Some(e) => Some(e),
            None if entries.is_empty() => {
                return Err(format!(
                    "--configuration {} names no configuration: {CONFIG_FILE} holds none; \
                     write one there or drop --configuration",
                    safe_line(want)
                ))
            }
            None => {
                return Err(format!(
                    "--configuration {} names no configuration in {CONFIG_FILE}; it holds {}",
                    safe_line(want),
                    names()
                ))
            }
        },
        (None, []) => None,
        (None, [only]) => Some(only),
        (None, _) => {
            return Err(format!(
                "{CONFIG_FILE} holds several configurations ({}); pick one with \
                 --configuration NAME",
                names()
            ))
        }
    };
    Ok(match entry {
        Some(e) => {
            let source = match e.from {
                ConfigurationFrom::CompileCommands if compile_commands && !flags_differ => {
                    ConfigSource::CompileCommands
                }
                ConfigurationFrom::CompileCommands => ConfigSource::Guessed,
                _ => ConfigSource::Stated,
            };
            MapConfiguration {
                name: e.name.clone(),
                from: e.from,
                source: if shipped {
                    ConfigSource::Guessed
                } else {
                    source
                },
                flags: e.flags.clone(),
                system_headers: e.system_headers.clone(),
                digest: digest(&e.name, e.from, &e.flags, &e.system_headers),
                proposed: shipped,
            }
        }
        None => {
            let from = if compile_commands {
                ConfigurationFrom::CompileCommands
            } else {
                ConfigurationFrom::Stated
            };
            MapConfiguration {
                name: GUESSED_NAME.into(),
                from,
                source: ConfigSource::Guessed,
                flags: Vec::new(),
                system_headers: Vec::new(),
                digest: digest(GUESSED_NAME, from, &[], &[]),
                proposed: false,
            }
        }
    })
}
