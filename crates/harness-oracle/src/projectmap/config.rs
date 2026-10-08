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
    /// Nothing states it: no `config.toml` entry, or a `compile_commands.json`
    /// whose flags differ.
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
    /// blake3 of the canonical JSON of `{flags, from, name}`.
    pub digest: String,
}

impl MapConfiguration {
    /// Each file's compile takes its own `compile_commands.json` entry's
    /// flags (a guess, or `from = "compile_commands"`); otherwise the
    /// configuration's flags.
    pub fn uses_entry_flags(&self) -> bool {
        self.source == ConfigSource::Guessed || self.from == ConfigurationFrom::CompileCommands
    }
}

/// blake3 of the canonical JSON (keys sorted, no blanks) of
/// `{flags, from, name}`: flag order counts.
pub fn digest(name: &str, from: ConfigurationFrom, flags: &[String]) -> String {
    // Fields in key order: serde writes them as declared.
    #[derive(serde::Serialize)]
    struct Canonical<'a> {
        flags: &'a [String],
        from: ConfigurationFrom,
        name: &'a str,
    }
    let json = serde_json::to_vec(&Canonical { flags, from, name })
        .unwrap_or_else(|_| unreachable!("strings and a unit enum always serialise"));
    harness_core::hash::bytes_hash(&json)
}

/// Read `config.toml`'s entries (none when the file is absent), each checked:
/// its name, its flags through the grammar with their paths resolved, its
/// `system_headers`. A file reached through a link out of the root, too
/// large, unparseable, two entries of one name, or a bad entry is refused in
/// one sentence.
pub fn read_entries(root: &Path) -> Result<Vec<ConfigEntry>, Error> {
    let path = root.join(CONFIG_FILE);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&path, e)),
    };
    let real = path.canonicalize().map_err(|e| Error::io(&path, e))?;
    if !real.starts_with(root) || !meta.is_file() {
        return Err(Error::parse(
            &path,
            "the configuration file is not a plain file inside the project; write it at \
             migration/map/config.toml",
        ));
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(Error::parse(
            &path,
            format!(
                "the configuration file is over {} KiB; keep only its [[configuration]] entries",
                MAX_CONFIG_BYTES >> 10
            ),
        ));
    }
    let text = std::fs::read_to_string(&real).map_err(|e| Error::io(&path, e))?;
    let shape: ConfigFileShape =
        toml::from_str(&text).map_err(|e| Error::parse(&path, e.message().to_string()))?;
    let mut seen = std::collections::BTreeSet::new();
    for entry in &shape.configuration {
        check_entry(root, entry).map_err(|m| Error::parse(&path, m))?;
        if !seen.insert(entry.name.as_str()) {
            return Err(Error::parse(
                &path,
                format!(
                    "two [[configuration]] entries are named `{}`; give each its own name",
                    safe_line(&entry.name)
                ),
            ));
        }
    }
    Ok(shape.configuration)
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

/// Every flag through the grammar, and every path flag's path resolved
/// against `root` with links followed: inside the root, outside
/// `migration/`. `Err` names the first refused flag.
pub fn check_flags(root: &Path, list: &[String]) -> Result<(), String> {
    for flag in list {
        if let Flag::Path(path) = flags::check_flag(flag)? {
            if !resolves_inside(root, path) {
                return Err(format!(
                    "the flag `{}` names a path that leaves the project or reaches migration/ \
                     through a link; name the real folder inside the project",
                    safe_line(flag)
                ));
            }
        }
    }
    Ok(())
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
/// `compile_commands` says a `compile_commands.json` was read and
/// `flags_differ` that it lists a file twice with other flags.
pub fn choose(
    entries: &[ConfigEntry],
    wanted: Option<&str>,
    compile_commands: bool,
    flags_differ: bool,
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
                source,
                flags: e.flags.clone(),
                system_headers: e.system_headers.clone(),
                digest: digest(&e.name, e.from, &e.flags),
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
                digest: digest(GUESSED_NAME, from, &[]),
            }
        }
    })
}
