//! Target configuration (`harness.toml`, docs/SCHEMAS.md) and the
//! [`TargetContext`] threaded through frontends, planners and oracles.

use crate::error::Error;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub mod flags;

/// The newest config schema this build understands: 1 is the folder form
/// (`source_dir`), 2 the file-list form (`files`).
pub const CONFIG_SCHEMA_VERSION: u64 = 2;
/// `schema_version` of the folder form.
pub const FOLDER_FORM_VERSION: u64 = 1;
/// `schema_version` of the file-list form.
pub const FILE_LIST_FORM_VERSION: u64 = 2;
/// The config file's name, at a folder-form target's root and in each
/// mapped tool's folder.
pub const CONFIG_FILE: &str = "harness.toml";
/// Where a project's mapped tools live, under its `migration/`.
pub const TOOLS_DIR: &str = "tools";
/// Longest `[target] configuration.name`, in bytes.
pub const MAX_CONFIGURATION_NAME: usize = 64;
/// Upper bound on any target-configured response token budget.
pub const MAX_TOKENS_LIMIT: u64 = 65_536;
/// Upper bound on target-configured repair turns per attempt.
pub const MAX_REPAIRS_LIMIT: u64 = 10;
/// Allowed range of `[driver] max_mutants` (clamped from BELOW too: a hostile
/// target must not be able to make the mutation gate meaningless).
pub const MAX_MUTANTS_RANGE: (u32, u32) = (16, 64);
/// Default `[driver] max_mutants`.
pub const DEFAULT_MAX_MUTANTS: u32 = 24;
/// Allowed range of `[driver] min_kill_ratio`, in permille.
pub const MIN_KILL_PERMILLE_RANGE: (u32, u32) = (500, 1000);
/// Default `[driver] min_kill_ratio` in permille (0.6 — a judgment call,
/// recorded as uncalibrated in DECISIONS.md).
pub const DEFAULT_MIN_KILL_PERMILLE: u32 = 600;

/// Parsed `harness.toml`, read version first: `schema_version` 1 is the
/// folder form, 2 the file-list form (docs/SCHEMAS.md). Unknown top-level
/// fields are tolerated and preserved on disk (this struct is read-only; the
/// file is never rewritten by the harness).
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Schema version of the file.
    pub schema_version: u64,
    /// `[target]` section.
    pub target: TargetSection,
    /// `[oracle]` section: `allowlist` is core-owned; every other key belongs
    /// to the configured oracle kind and is handed over opaquely.
    pub oracle: toml::Table,
    /// `[llm]` section (docs/SCHEMAS.md M2 additions).
    pub llm: LlmSection,
    /// `[driver]` section: driver-generation validation policy (M4).
    pub driver: DriverSection,
}

/// The sections every form shares.
#[derive(Deserialize)]
struct Shared {
    #[serde(default)]
    oracle: toml::Table,
    #[serde(default)]
    llm: LlmSection,
    #[serde(default)]
    driver: DriverSection,
}

impl<'de> Deserialize<'de> for TargetConfig {
    /// `toml::from_str::<TargetConfig>` reads version first, as
    /// [`TargetConfig::load`] does, with the checks that need no project
    /// root.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let table = toml::Table::deserialize(d)?;
        TargetConfig::from_table(table).map_err(serde::de::Error::custom)
    }
}

/// `[driver]`: the self-validation policy for generated differential drivers
/// (docs/SCHEMAS.md "M4 additions"). Both keys are range-checked at load.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DriverSection {
    /// Mutants sampled per validation (default [`DEFAULT_MAX_MUTANTS`], range
    /// [`MAX_MUTANTS_RANGE`]).
    #[serde(default)]
    pub max_mutants: Option<u32>,
    /// Minimum kill ratio when at least 10 mutants compiled (default 0.6,
    /// range 0.5–1.0).
    #[serde(default)]
    pub min_kill_ratio: Option<f64>,
}

/// The effective, validated driver-validation policy. Recorded verbatim in
/// every `driver-validation.json` so the bar a driver cleared is evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct DriverPolicy {
    /// Mutants sampled per validation.
    pub max_mutants: u32,
    /// Minimum kill ratio in permille (integer: canonical bytes never depend
    /// on float formatting).
    pub min_kill_permille: u32,
}

/// The `[llm]` section of `harness.toml`. Target config is hostile input: it
/// may only NAME a provider profile and a model — endpoints and credentials
/// live in user-level provider profiles (docs/SCHEMAS.md M3 additions).
#[derive(Debug, Clone, Deserialize)]
pub struct LlmSection {
    /// Provider profile name (`external | replay | anthropic` built in, or a
    /// user-defined profile).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Model identifier (Tier-2 default per briefing §16).
    #[serde(default = "default_model")]
    pub model: String,
    /// Response token budget.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Optional `[llm.migrate]` stage override (§13.2 per-stage routing).
    #[serde(default)]
    pub migrate: Option<MigrateSection>,
    /// Optional `[llm.driver]` stage override for driver generation (M4);
    /// same keys and clamps as `[llm.migrate]`.
    #[serde(default)]
    pub driver: Option<MigrateSection>,
}

/// `[llm.migrate]` / `[llm.driver]`: stage routing and loop bounds.
#[derive(Debug, Clone, Deserialize)]
pub struct MigrateSection {
    /// Provider profile for the executor (falls back to `[llm] provider`).
    #[serde(default)]
    pub provider: Option<String>,
    /// Model for the executor (falls back to `[llm] model`).
    #[serde(default)]
    pub model: Option<String>,
    /// Response token budget (falls back to `[llm] max_tokens`).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Stateless repair turns after the translate turn (default 3).
    #[serde(default)]
    pub max_repairs: Option<u32>,
    /// Whether a green `migrate` attempt is promoted at once (default true).
    /// `false` makes `harness promote` (or an explicit `--promote`) the only
    /// promotion path — "Accept = an explicit act", sticky across the
    /// several invocations one `external` attempt takes
    /// (docs/CLI-HARDENING.md §2). Set by the user; clients never write it.
    #[serde(default)]
    pub promote_on_green: Option<bool>,
}

impl Default for LlmSection {
    fn default() -> Self {
        LlmSection {
            provider: default_provider(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            migrate: None,
            driver: None,
        }
    }
}

fn default_provider() -> String {
    "external".into()
}
fn default_model() -> String {
    "claude-sonnet-5".into()
}
fn default_max_tokens() -> u32 {
    8192
}

/// The `[target]` section of `harness.toml`: the run name and the form.
#[derive(Debug, Clone)]
pub struct TargetSection {
    /// Human label for the target; in the file-list form, the run name (what
    /// features run the program as).
    pub name: String,
    /// Which files make up the target.
    pub form: Form,
}

/// Which files make up a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// `schema_version = 1`: every C file of one folder.
    Folder(FolderForm),
    /// `schema_version = 2`: a list of files, each with its include folders,
    /// built under one configuration.
    FileList(FileList),
}

/// The folder form (`schema_version = 1`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FolderForm {
    /// Directory of the source files, relative to the target root.
    pub source_dir: String,
    /// Extra quoted-include search dirs (M4), relative to the target root;
    /// each must lie inside `source_dir`. Searched after the including file's
    /// own directory, in order.
    #[serde(default)]
    pub include_dirs: Vec<String>,
}

/// The file-list form (`schema_version = 2`, docs/PROJECT-MAP-DESIGN.md §3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileList {
    /// The listed files, in file order.
    pub files: Vec<TargetFile>,
    /// The configuration they are built under.
    pub configuration: Configuration,
    /// The map this target was accepted from (absent when hand-written).
    pub map: Option<MapStamp>,
    /// The person's picks between duplicate definers.
    pub picks: Vec<Pick>,
}

/// One listed file and the folders its includes are searched in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetFile {
    /// Clean path relative to the project root, never under `migration/`.
    pub path: String,
    /// Its include folders, in order, each `.` or a clean path relative to
    /// the project root, never under `migration/`.
    #[serde(default)]
    pub include_dirs: Vec<String>,
}

/// The named build a tool is migrated under (docs/PROJECT-MAP-DESIGN.md §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// `make`, `meson`, `cmake` or the person's own word.
    pub name: String,
    /// What it stands for.
    pub from: ConfigurationFrom,
    /// The flags, in order, each checked by [`flags::check_flag`].
    pub flags: Vec<String>,
}

/// What a configuration stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationFrom {
    /// The project's Makefile.
    Make,
    /// Its Meson build.
    Meson,
    /// Its CMake build.
    Cmake,
    /// A `compile_commands.json`.
    CompileCommands,
    /// The person's own statement.
    Stated,
}

/// The map a target was accepted from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapStamp {
    /// The map's `root_hash`.
    pub root_hash: String,
    /// The map's `inputs_hash`.
    pub inputs_hash: String,
}

/// A pick between duplicate definers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pick {
    /// The definers' paths.
    pub definers: Vec<String>,
    /// The one kept.
    pub keep: String,
    /// Who picked: `person` or `links`.
    pub by: String,
}

/// The `[target]` keys of the file-list form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileListTarget {
    name: String,
    files: Vec<TargetFile>,
    configuration: Configuration,
    #[serde(default)]
    map: Option<MapStamp>,
    #[serde(default)]
    picks: Vec<Pick>,
}

/// The `[target]` keys of the folder form (unknown keys tolerated, as
/// always).
#[derive(Deserialize)]
struct FolderTarget {
    name: String,
    source_dir: String,
    #[serde(default)]
    include_dirs: Vec<String>,
}

impl TargetSection {
    /// The folder form's `source_dir`; `None` for a file-list target.
    pub fn source_dir(&self) -> Option<&str> {
        match &self.form {
            Form::Folder(f) => Some(&f.source_dir),
            Form::FileList(_) => None,
        }
    }

    /// The folder form's `include_dirs`; empty for a file-list target (its
    /// folders are per file: [`TargetSection::files`]).
    pub fn include_dirs(&self) -> &[String] {
        match &self.form {
            Form::Folder(f) => &f.include_dirs,
            Form::FileList(_) => &[],
        }
    }

    /// The file-list form; `None` for a folder target.
    pub fn file_list(&self) -> Option<&FileList> {
        match &self.form {
            Form::Folder(_) => None,
            Form::FileList(l) => Some(l),
        }
    }

    /// The listed files; `None` for a folder target.
    pub fn files(&self) -> Option<&[TargetFile]> {
        self.file_list().map(|l| l.files.as_slice())
    }

    /// The configuration; `None` for a folder target.
    pub fn configuration(&self) -> Option<&Configuration> {
        self.file_list().map(|l| &l.configuration)
    }

    /// The folder form, or the one-sentence refusal of a reader that does
    /// not read the file-list form yet: `what` names it (`harness scan`).
    pub fn folder(&self, what: &str) -> Result<&FolderForm, Error> {
        match &self.form {
            Form::Folder(f) => Ok(f),
            Form::FileList(_) => Err(Error::FileListNotRead { what: what.into() }),
        }
    }
}

/// Lexical checks of the folder form: every `include_dirs` entry a clean
/// relative path inside `source_dir`.
fn check_folder(folder: &FolderForm) -> Result<(), String> {
    // `source_dir = "."` (or "./", "./src") is the root's own spelling:
    // normalised before the prefix test (check 7).
    let source_dir = folder.source_dir.trim_end_matches('/');
    let source_dir = source_dir.strip_prefix("./").unwrap_or(source_dir);
    let source_dir = if source_dir == "." { "" } else { source_dir };
    for dir in &folder.include_dirs {
        let inside = source_dir.is_empty()
            || dir
                .strip_prefix(source_dir)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        if !crate::plan::is_clean_relative_path(dir) || !inside {
            return Err(format!(
                "[target] include_dirs entry {dir:?} must be a clean relative path inside \
                 source_dir {:?}",
                folder.source_dir
            ));
        }
    }
    Ok(())
}

/// Lexical checks of the file-list form (the grammar included); the links
/// are resolved by [`TargetConfig::load_file`], which knows the project root.
fn check_file_list(list: &FileList) -> Result<(), String> {
    if list.files.is_empty() {
        return Err("[target] files is empty; list the program's .c files".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for file in &list.files {
        let shown = crate::text::safe_line(&file.path);
        if !crate::plan::is_clean_relative_path(&file.path)
            || !flags::inside_root_lexically(&file.path)
        {
            return Err(format!(
                "[target] files entry `{shown}` must be a clean path relative to the project \
                 root, outside migration/; write it like src/main.c"
            ));
        }
        if !seen.insert(file.path.as_str()) {
            return Err(format!(
                "[target] files lists `{shown}` twice; keep one entry"
            ));
        }
        for dir in &file.include_dirs {
            if !flags::inside_root_lexically(dir) {
                return Err(format!(
                    "the include folder `{}` of `{shown}` must be `.` or a clean path relative \
                     to the project root, outside migration/; fix that entry",
                    crate::text::safe_line(dir)
                ));
            }
        }
    }
    let c = &list.configuration;
    let name_ok = !c.name.is_empty()
        && c.name.len() <= MAX_CONFIGURATION_NAME
        && c.name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if !name_ok {
        return Err(format!(
            "[target] configuration name `{}` must be a word of letters, digits, _ - or . (at \
             most {MAX_CONFIGURATION_NAME}); rename it",
            crate::text::safe_line(&c.name)
        ));
    }
    for flag in &c.flags {
        flags::check_flag(flag)?;
    }
    if let Some(map) = &list.map {
        for (key, value) in [
            ("root_hash", &map.root_hash),
            ("inputs_hash", &map.inputs_hash),
        ] {
            if !is_digest(value) {
                return Err(format!(
                    "[target] map.{key} is not a digest the map writes; copy it from the map, \
                     or remove [target] map"
                ));
            }
        }
    }
    for pick in &list.picks {
        if !matches!(pick.by.as_str(), "person" | "links") {
            return Err(format!(
                "a [target] picks entry says by = `{}`; it is `person` or `links`",
                crate::text::safe_line(&pick.by)
            ));
        }
        if !pick.definers.contains(&pick.keep) {
            return Err(format!(
                "a [target] picks entry keeps `{}`, which is not one of its definers; keep one \
                 of them",
                crate::text::safe_line(&pick.keep)
            ));
        }
    }
    Ok(())
}

/// A digest as the map writes one: 64 lowercase hex digits, optionally
/// after `blake3:`.
fn is_digest(value: &str) -> bool {
    let hex = value.strip_prefix("blake3:").unwrap_or(value);
    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `rel` (`.` or a clean relative path) resolved against `root`: the
/// deepest part that exists, with its links followed, must lie inside the
/// root and outside its `migration/`. A part that does not exist yet
/// cannot lead anywhere.
fn resolves_inside(root: &Path, rel: &str) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let ledger = root.join(crate::ledger::MIGRATION_DIR);
    let mut probe = root.join(rel);
    loop {
        match probe.canonicalize() {
            Ok(real) => return real.starts_with(&root) && !real.starts_with(&ledger),
            Err(_) => {
                if !probe.pop() {
                    return false;
                }
            }
        }
    }
}

impl TargetConfig {
    /// Load and validate `harness.toml` from a target root.
    pub fn load(target_root: &Path) -> Result<TargetConfig, Error> {
        TargetConfig::load_file(&target_root.join(CONFIG_FILE), target_root)
    }

    /// Load and validate the config at `path` for the project at `root` (a
    /// mapped tool's file lies under `root/migration/tools/<id>/`; its paths
    /// are the project root's). Read version first: a newer version is
    /// [`Error::SchemaTooNew`] before any other key is looked at.
    pub fn load_file(path: &Path, root: &Path) -> Result<TargetConfig, Error> {
        let path = path.to_path_buf();
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| Error::parse(&path, e.to_string()))?;
        if let Some(found) = table
            .get("schema_version")
            .and_then(toml::Value::as_integer)
            .and_then(|v| u64::try_from(v).ok())
        {
            if found > CONFIG_SCHEMA_VERSION {
                return Err(Error::SchemaTooNew {
                    path,
                    found,
                    supported: CONFIG_SCHEMA_VERSION,
                });
            }
        }
        let config = TargetConfig::from_table(table).map_err(|m| Error::parse(&path, m))?;
        config.check_budgets().map_err(|m| Error::parse(&path, m))?;
        config.driver_policy().map_err(|m| Error::parse(&path, m))?;
        match &config.target.form {
            Form::Folder(folder) => check_folder(folder),
            Form::FileList(list) => check_paths_resolve(root, list),
        }
        .map_err(|m| Error::parse(&path, m))?;
        Ok(config)
    }

    /// Build a config from the parsed file, version first: its shape, the
    /// two forms kept apart, and the file-list form's lexical rules and flag
    /// grammar. [`TargetConfig::load_file`] adds the budgets, the driver
    /// policy, the folder form's `include_dirs` rule and the links of the
    /// file-list form's paths. `Err` is one sentence.
    pub fn from_table(mut table: toml::Table) -> Result<TargetConfig, String> {
        let schema_version = match table.remove("schema_version") {
            None => return Err("`schema_version` is missing; write schema_version = 1".into()),
            Some(v) => v
                .as_integer()
                .and_then(|v| u64::try_from(v).ok())
                .filter(|v| *v >= 1)
                .ok_or("`schema_version` must be a whole number from 1")?,
        };
        if schema_version > CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "schema_version {schema_version} is newer than this harness reads \
                 ({CONFIG_SCHEMA_VERSION}); upgrade the harness"
            ));
        }
        let target = match table.remove("target") {
            Some(toml::Value::Table(t)) => t,
            Some(_) => return Err("[target] must be a table".into()),
            None => return Err("missing field `target`".into()),
        };
        if target.contains_key("files") && target.contains_key("source_dir") {
            return Err(
                "[target] holds both `files` and `source_dir`; a target is one folder \
                 (source_dir, schema_version = 1) or a list of files (files, schema_version = 2), \
                 so keep one"
                    .into(),
            );
        }
        let target = if schema_version == FOLDER_FORM_VERSION {
            if target.contains_key("files") {
                return Err(
                    "[target] files is the file-list form; write schema_version = 2".into(),
                );
            }
            let t: FolderTarget = toml::Value::Table(target)
                .try_into()
                .map_err(|e: toml::de::Error| e.message().to_string())?;
            let folder = FolderForm {
                source_dir: t.source_dir,
                include_dirs: t.include_dirs,
            };
            TargetSection {
                name: t.name,
                form: Form::Folder(folder),
            }
        } else {
            if target.contains_key("source_dir") {
                return Err(
                    "[target] source_dir is the folder form; write schema_version = 1, or list \
                     the files with [target] files"
                        .into(),
                );
            }
            let t: FileListTarget = toml::Value::Table(target)
                .try_into()
                .map_err(|e: toml::de::Error| format!("[target]: {}", e.message()))?;
            let list = FileList {
                files: t.files,
                configuration: t.configuration,
                map: t.map,
                picks: t.picks,
            };
            check_file_list(&list)?;
            TargetSection {
                name: t.name,
                form: Form::FileList(list),
            }
        };
        let shared: Shared = toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| e.message().to_string())?;
        let config = TargetConfig {
            schema_version,
            target,
            oracle: shared.oracle,
            llm: shared.llm,
            driver: shared.driver,
        };
        Ok(config)
    }

    /// harness.toml is target-owned, hostile input: it must not be able to
    /// turn one command into an unbounded stream of billable calls.
    fn check_budgets(&self) -> Result<(), String> {
        let mut budgets: Vec<(String, u64, u64)> = vec![(
            "[llm] max_tokens".to_string(),
            u64::from(self.llm.max_tokens),
            MAX_TOKENS_LIMIT,
        )];
        for (stage, section) in [
            ("[llm.migrate]", &self.llm.migrate),
            ("[llm.driver]", &self.llm.driver),
        ] {
            let Some(m) = section else { continue };
            if let Some(t) = m.max_tokens {
                budgets.push((
                    format!("{stage} max_tokens"),
                    u64::from(t),
                    MAX_TOKENS_LIMIT,
                ));
            }
            if let Some(r) = m.max_repairs {
                budgets.push((
                    format!("{stage} max_repairs"),
                    u64::from(r),
                    MAX_REPAIRS_LIMIT,
                ));
            }
        }
        for (key, value, limit) in budgets {
            if value > limit {
                return Err(format!(
                    "{key} = {value} exceeds the harness limit of {limit}"
                ));
            }
        }
        Ok(())
    }

    /// The validated `[driver]` policy (defaults applied). `Err` names the
    /// offending key; [`TargetConfig::load`] refuses such a config.
    pub fn driver_policy(&self) -> Result<DriverPolicy, String> {
        let max_mutants = self.driver.max_mutants.unwrap_or(DEFAULT_MAX_MUTANTS);
        let (lo, hi) = MAX_MUTANTS_RANGE;
        if !(lo..=hi).contains(&max_mutants) {
            return Err(format!(
                "[driver] max_mutants = {max_mutants} is outside the allowed range {lo}..={hi}"
            ));
        }
        let min_kill_permille = match self.driver.min_kill_ratio {
            None => DEFAULT_MIN_KILL_PERMILLE,
            Some(r) if r.is_finite() && (0.0..=1.0).contains(&r) => (r * 1000.0).round() as u32,
            Some(r) => return Err(format!("[driver] min_kill_ratio = {r} is not in 0.0..=1.0")),
        };
        let (lo, hi) = MIN_KILL_PERMILLE_RANGE;
        if !(lo..=hi).contains(&min_kill_permille) {
            return Err(format!(
                "[driver] min_kill_ratio = {} is outside the allowed range 0.5..=1.0",
                f64::from(min_kill_permille) / 1000.0
            ));
        }
        Ok(DriverPolicy {
            max_mutants,
            min_kill_permille,
        })
    }

    /// The oracle executable allowlist (core-owned key; defaults to empty).
    pub fn oracle_allowlist(&self) -> Vec<String> {
        self.oracle
            .get("allowlist")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Every path of a file-list target, its links followed, lies inside the
/// project root and outside `migration/`.
fn check_paths_resolve(root: &Path, list: &FileList) -> Result<(), String> {
    let outside = |what: String| {
        format!(
            "{what} leads outside the project or into migration/ through a link; name a path \
             inside the project"
        )
    };
    for file in &list.files {
        let shown = crate::text::safe_line(&file.path);
        if !resolves_inside(root, &file.path) {
            return Err(outside(format!("[target] files entry `{shown}`")));
        }
        for dir in &file.include_dirs {
            if !resolves_inside(root, dir) {
                return Err(outside(format!(
                    "the include folder `{}` of `{shown}`",
                    crate::text::safe_line(dir)
                )));
            }
        }
    }
    for flag in &list.configuration.flags {
        if let Ok(flags::Flag::Path(p)) = flags::check_flag(flag) {
            if !resolves_inside(root, p) {
                return Err(outside(format!(
                    "the flag `{}`",
                    crate::text::safe_line(flag)
                )));
            }
        }
    }
    Ok(())
}

/// Whether `id` is a tool id (docs/PROJECT-MAP-DESIGN.md §3.3):
/// `^[tl]-[a-z0-9_-]{1,64}$`.
pub fn is_tool_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("t-").or_else(|| id.strip_prefix("l-")) else {
        return false;
    };
    (1..=64).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

/// `--tool`'s check: `Err` is the one-sentence refusal.
pub fn check_tool_id(id: &str) -> Result<(), String> {
    if is_tool_id(id) {
        Ok(())
    } else {
        Err(format!(
            "`{}` is not a tool id: it is t- or l- followed by 1 to 64 lowercase letters, digits, \
             _ or -, as `harness project map` prints them",
            crate::text::safe_line(id)
        ))
    }
}

/// `<root>/migration/tools/`.
pub fn tools_dir(root: &Path) -> PathBuf {
    root.join(crate::ledger::MIGRATION_DIR).join(TOOLS_DIR)
}

/// A mapped tool's folder — its `harness.toml` and its ledger:
/// `<root>/migration/tools/<id>/`.
pub fn tool_dir(root: &Path, id: &str) -> PathBuf {
    tools_dir(root).join(id)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// The project's mapped tools, sorted: each real folder of
/// `migration/tools/` named by a tool id and holding a `harness.toml` (a
/// link is never followed).
pub fn mapped_tools(root: &Path) -> Vec<String> {
    let dir = tools_dir(root);
    if !is_real_dir(&root.join(crate::ledger::MIGRATION_DIR)) || !is_real_dir(&dir) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|id| {
            is_tool_id(id)
                && is_real_dir(&dir.join(id))
                && is_real_file(&dir.join(id).join(CONFIG_FILE))
        })
        .collect();
    ids.sort();
    ids
}

/// Which target `--target <root> [--tool <id>]` names (the lookup order of
/// docs/PROJECT-MAP-DESIGN.md §3.7), before anything is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The root's own `harness.toml`: the folder-form layout, ledger
    /// `<root>/migration/`.
    Root,
    /// A mapped tool: `migration/tools/<id>/harness.toml`, ledger
    /// `migration/tools/<id>/`.
    Tool(String),
}

/// The lookup order: with `tool`, that mapped tool (never the root file);
/// without, the root's `harness.toml`, else the only mapped tool; else a
/// one-sentence refusal that names the tools. A root with neither is
/// [`Found::Root`], whose load then says no `harness.toml` was found.
pub fn find_target(root: &Path, tool: Option<&str>) -> Result<Found, Error> {
    let tools = || mapped_tools(root);
    let named = |ids: &[String]| match ids {
        [] => "it has none".to_string(),
        ids => format!("its tools are {}", ids.join(", ")),
    };
    if let Some(id) = tool {
        check_tool_id(id).map_err(|why| Error::NoTarget { why })?;
        if !tools().iter().any(|t| t == id) {
            return Err(Error::NoTarget {
                why: format!(
                    "{} has no mapped tool {id} ({}); pick one of them with --tool, or run \
                     `harness project map` to see its programs",
                    root.display(),
                    named(&tools())
                ),
            });
        }
        return Ok(Found::Tool(id.to_string()));
    }
    if root.join(CONFIG_FILE).exists() {
        return Ok(Found::Root);
    }
    match tools().as_slice() {
        [] => Ok(Found::Root),
        [one] => Ok(Found::Tool(one.clone())),
        several => Err(Error::NoTarget {
            why: format!(
                "{} has {} mapped tools and no harness.toml of its own; pick one with --tool \
                 ({})",
                root.display(),
                several.len(),
                several.join(", ")
            ),
        }),
    }
}

/// A target root plus its parsed configuration — the context every trait
/// implementation receives.
#[derive(Debug, Clone)]
pub struct TargetContext {
    /// Absolute path of the target repository root (the project root: the
    /// base of every path in the facts and the plan, the containment root,
    /// the sandbox's read root).
    pub root: PathBuf,
    /// Absolute path of the ledger folder: `<root>/migration/` for a
    /// folder-form target, `<root>/migration/tools/<id>/` for a mapped tool.
    /// Every ledger path derives from it ([`crate::ledger::Ledger::of`]).
    pub ledger: PathBuf,
    /// The mapped tool's id, when this target is one.
    pub tool: Option<String>,
    /// Parsed `harness.toml`.
    pub config: TargetConfig,
}

impl TargetContext {
    /// Build a context for `root` by the lookup order without `--tool`
    /// ([`find_target`]). A ledger made elsewhere is refused first
    /// ([`crate::adopt::check`], once for the project root, covering its
    /// tools): every command that opens a ledger — the CLI, the cockpit's
    /// read model, harness-mcp — goes through here.
    pub fn load(root: impl Into<PathBuf>) -> Result<TargetContext, Error> {
        TargetContext::open(root, None)
    }

    /// [`TargetContext::load`] with `--tool`: `Some(id)` loads
    /// `migration/tools/<id>/harness.toml` (never the root file), its
    /// ledger `migration/tools/<id>/`.
    pub fn open(root: impl Into<PathBuf>, tool: Option<&str>) -> Result<TargetContext, Error> {
        let root = root.into();
        let root = root.canonicalize().map_err(|e| Error::io(&root, e))?;
        crate::adopt::check(&root)?;
        match find_target(&root, tool)? {
            Found::Root => {
                let config = TargetConfig::load(&root)?;
                Ok(TargetContext {
                    ledger: root.join(crate::ledger::MIGRATION_DIR),
                    tool: None,
                    root,
                    config,
                })
            }
            Found::Tool(id) => {
                let ledger = tool_dir(&root, &id);
                let config = TargetConfig::load_file(&ledger.join(CONFIG_FILE), &root)?;
                Ok(TargetContext {
                    root,
                    ledger,
                    tool: Some(id),
                    config,
                })
            }
        }
    }

    /// A folder-form context over `root` with an already parsed config
    /// (tests, and callers that build a config in memory).
    pub fn folder_form(root: PathBuf, config: TargetConfig) -> TargetContext {
        TargetContext {
            ledger: root.join(crate::ledger::MIGRATION_DIR),
            tool: None,
            root,
            config,
        }
    }

    /// The ledger folder relative to the root, as `/`-separated parts
    /// (`["migration"]`, or `["migration", "tools", "t-x"]`).
    pub fn ledger_parts(&self) -> Vec<String> {
        self.ledger
            .strip_prefix(&self.root)
            .map(|rel| {
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_else(|_| vec![crate::ledger::MIGRATION_DIR.to_string()])
    }

    /// The ledger folder relative to the root, `/`-joined (`migration` or
    /// `migration/tools/t-x`), for messages and generated views.
    pub fn ledger_rel(&self) -> String {
        self.ledger_parts().join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ruharness-config-{tag}-{}-{}",
            std::process::id(),
            crate::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).expect("tmp");
        dir.canonicalize().expect("canonical")
    }

    const FOLDER: &str = "schema_version = 1\n[target]\nname = \"lib\"\nsource_dir = \"src\"\n";

    /// A liblzg-shaped file list: a tool and a library folder, headers in
    /// `src/include`.
    fn file_list(flags: &str) -> String {
        format!(
            "schema_version = 2\n\
             [target]\n\
             name = \"lzg\"\n\
             files = [\n\
               {{ path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] }},\n\
               {{ path = \"src/lib/encode.c\", include_dirs = [\"src/include\", \".\"] }},\n\
               {{ path = \"src/lib/decode.c\" }},\n\
             ]\n\
             configuration = {{ name = \"make\", from = \"make\", flags = [{flags}] }}\n"
        )
    }

    fn project(tag: &str) -> PathBuf {
        let root = tmp(tag);
        for dir in ["src/tools", "src/lib", "src/include"] {
            std::fs::create_dir_all(root.join(dir)).expect("dirs");
        }
        root
    }

    fn write_tool(root: &Path, id: &str, text: &str) {
        let dir = tool_dir(root, id);
        std::fs::create_dir_all(&dir).expect("tool dir");
        std::fs::write(dir.join(CONFIG_FILE), text).expect("write");
    }

    fn load_text(root: &Path, text: &str) -> Result<TargetConfig, Error> {
        std::fs::write(root.join(CONFIG_FILE), text).expect("write");
        TargetConfig::load(root)
    }

    #[test]
    fn a_newer_schema_reads_too_new_before_anything_else() {
        let root = tmp("too-new");
        // No [target] at all: a struct-first loader would say "missing field".
        let err = load_text(&root, "schema_version = 3\n[other]\nx = 1\n").unwrap_err();
        assert!(
            matches!(
                err,
                Error::SchemaTooNew {
                    found: 3,
                    supported: 2,
                    ..
                }
            ),
            "{err}"
        );
        assert!(err.to_string().contains("schema_version 3"), "{err}");
    }

    #[test]
    fn the_folder_form_reads_as_before() {
        let root = tmp("folder");
        let config = load_text(&root, FOLDER).expect("loads");
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.target.source_dir(), Some("src"));
        assert!(config.target.include_dirs().is_empty());
        assert!(config.target.files().is_none());
        assert!(config.target.configuration().is_none());
        assert!(config.target.folder("harness scan").is_ok());
        let err = load_text(&root, "schema_version = 1\n[target]\nname = \"x\"\n").unwrap_err();
        assert!(
            err.to_string().contains("missing field `source_dir`"),
            "{err}"
        );
        let err = load_text(
            &root,
            "schema_version = 1\n[target]\nname = \"x\"\nsource_dir = \"src\"\n\
             include_dirs = [\"other\"]\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside source_dir"), "{err}");
    }

    #[test]
    fn a_file_list_loads_with_its_files_folders_configuration_and_name() {
        let root = project("v2");
        let config = load_text(
            &root,
            &format!(
                "{}map = {{ root_hash = \"blake3:{}\", inputs_hash = \"{}\" }}\n\
                 picks = [{{ definers = [\"a.c\", \"b.c\"], keep = \"a.c\", by = \"person\" }}]\n\
                 [oracle]\ntimeout_secs = 30\n",
                file_list("\"-DLZG_FAST\", \"-Isrc/include\", \"-std=c99\", \"-O2\""),
                "a".repeat(64),
                "0".repeat(64)
            ),
        )
        .expect("loads");
        assert_eq!(config.schema_version, 2);
        assert_eq!(config.target.name, "lzg");
        assert_eq!(config.target.source_dir(), None);
        assert!(config.target.include_dirs().is_empty());
        let files = config.target.files().expect("files");
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["src/tools/lzg.c", "src/lib/encode.c", "src/lib/decode.c"]
        );
        assert_eq!(files[1].include_dirs, ["src/include", "."]);
        assert!(files[2].include_dirs.is_empty());
        let c = config.target.configuration().expect("configuration");
        assert_eq!(c.name, "make");
        assert_eq!(c.from, ConfigurationFrom::Make);
        assert_eq!(c.flags, ["-DLZG_FAST", "-Isrc/include", "-std=c99", "-O2"]);
        let list = config.target.file_list().expect("list");
        assert!(list.map.is_some());
        assert_eq!(list.picks[0].keep, "a.c");
        assert_eq!(
            config
                .oracle
                .get("timeout_secs")
                .and_then(|v| v.as_integer()),
            Some(30)
        );
        // A reader of the folder form refuses it in one sentence.
        let err = config.target.folder("harness scan").unwrap_err();
        assert_eq!(
            err.to_string(),
            "this target lists its files; harness scan does not read that form yet"
        );
    }

    #[test]
    fn a_file_holding_both_forms_is_refused_by_name() {
        let root = project("both");
        for version in [1, 2] {
            let text = format!(
                "schema_version = {version}\n[target]\nname = \"x\"\nsource_dir = \"src\"\n\
                 files = [{{ path = \"src/lib/encode.c\" }}]\n\
                 configuration = {{ name = \"make\", from = \"make\", flags = [] }}\n"
            );
            let err = load_text(&root, &text).unwrap_err().to_string();
            assert!(err.contains("both `files` and `source_dir`"), "{err}");
        }
        let v1_list = file_list("").replace("schema_version = 2", "schema_version = 1");
        let err = load_text(&root, &v1_list).unwrap_err().to_string();
        assert!(err.contains("write schema_version = 2"), "{err}");
        let v2_folder = FOLDER.replace("schema_version = 1", "schema_version = 2");
        let err = load_text(&root, &v2_folder).unwrap_err().to_string();
        assert!(err.contains("source_dir is the folder form"), "{err}");
    }

    #[test]
    fn a_flag_or_folder_outside_the_grammar_is_refused_by_name() {
        let root = project("grammar");
        for (flag, says) in [
            (
                "\"-fuse-ld=/x\"",
                "`-fuse-ld=/x` is not one the harness passes",
            ),
            ("\"-I@f\"", "`-I@f` has a value starting with `@`"),
            (
                "\"-DFOO BAR\"",
                "`-DFOO BAR` does not define a C identifier",
            ),
            (
                "\"-I../outside\"",
                "`-I../outside` names a path outside the project",
            ),
            (
                "\"-Imigration/map\"",
                "`-Imigration/map` names a path outside the project",
            ),
        ] {
            let err = load_text(&root, &file_list(flag)).unwrap_err().to_string();
            assert!(err.contains(says), "{flag}: {err}");
        }
        // A folder outside the root, or under migration/, in a file's own list.
        for dir in ["../elsewhere", "/usr/include", "migration/tools/t-x"] {
            let text = file_list("").replace("[\"src/include\"]", &format!("[\"{dir}\"]"));
            let err = load_text(&root, &text).unwrap_err().to_string();
            assert!(
                err.contains(&format!("include folder `{dir}`")),
                "{dir}: {err}"
            );
        }
        // A listed file under migration/.
        let text = file_list("").replace("src/lib/decode.c", "migration/x.c");
        let err = load_text(&root, &text).unwrap_err().to_string();
        assert!(
            err.contains("`migration/x.c` must be a clean path"),
            "{err}"
        );
        // A folder inside the root lexically, outside it through a link.
        let outside = tmp("outside");
        std::os::unix::fs::symlink(&outside, root.join("linked")).expect("link");
        let text = file_list("").replace("[\"src/include\"]", "[\"linked/inc\"]");
        let err = load_text(&root, &text).unwrap_err().to_string();
        assert!(
            err.contains("`linked/inc`") && err.contains("through a link"),
            "{err}"
        );
        let err = load_text(&root, &file_list("\"-Ilinked\""))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("the flag `-Ilinked`") && err.contains("through a link"),
            "{err}"
        );
        // A configuration with an unknown key, a bad `from`, a bad name.
        for (configuration, says) in [
            (
                "{ name = \"make\", from = \"make\", flags = [], extra = 1 }",
                "unknown field",
            ),
            (
                "{ name = \"make\", from = \"ninja\", flags = [] }",
                "unknown variant",
            ),
            (
                "{ name = \"a b\", from = \"stated\", flags = [] }",
                "configuration name",
            ),
            (
                "{ name = \"make\", from = \"make\" }",
                "missing field `flags`",
            ),
        ] {
            let text = file_list("").replace(
                "{ name = \"make\", from = \"make\", flags = [] }",
                configuration,
            );
            let err = load_text(&root, &text).unwrap_err().to_string();
            assert!(err.contains(says), "{configuration}: {err}");
        }
    }

    #[test]
    fn a_tool_id_is_checked() {
        let long_ok = format!("t-{}", "a".repeat(64));
        let long_bad = format!("t-{}", "a".repeat(65));
        for ok in ["t-lz4", "l-lib_x", "t-a-2", long_ok.as_str()] {
            assert!(is_tool_id(ok), "{ok}");
        }
        for bad in [
            "t-",
            "x-lz4",
            "t-LZ4",
            "t-a/b",
            "t-..",
            "lz4",
            "t-a b",
            long_bad.as_str(),
        ] {
            assert!(!is_tool_id(bad), "{bad}");
            assert!(check_tool_id(bad).unwrap_err().contains("is not a tool id"));
        }
    }

    #[test]
    fn the_lookup_order_finds_the_target() {
        crate::adopt::testing::adoption_file();
        let root = project("lookup");
        // Neither: the root's load says what is missing.
        let err = TargetContext::load(&root).unwrap_err();
        assert!(err.is_not_found(), "{err}");
        // One mapped tool: opened without --tool.
        write_tool(&root, "t-lzg", &file_list(""));
        crate::adopt::testing::adopt(&root);
        let ctx = TargetContext::load(&root).expect("the only tool");
        assert_eq!(ctx.tool.as_deref(), Some("t-lzg"));
        assert_eq!(ctx.ledger, root.join("migration/tools/t-lzg"));
        assert_eq!(ctx.ledger_rel(), "migration/tools/t-lzg");
        // Two: refused, both named.
        write_tool(&root, "t-x", &file_list(""));
        let err = TargetContext::load(&root).unwrap_err().to_string();
        assert!(
            err.contains("2 mapped tools") && err.contains("t-lzg, t-x"),
            "{err}"
        );
        assert!(err.contains("--tool"), "{err}");
        // --tool picks one.
        let ctx = TargetContext::open(&root, Some("t-x")).expect("t-x");
        assert_eq!(ctx.tool.as_deref(), Some("t-x"));
        assert_eq!(ctx.ledger, root.join("migration/tools/t-x"));
        // A bad id, an unknown one.
        let err = TargetContext::open(&root, Some("T-X"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a tool id"), "{err}");
        let err = TargetContext::open(&root, Some("t-nope"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no mapped tool t-nope") && err.contains("t-lzg, t-x"),
            "{err}"
        );
        // A root harness.toml wins only without --tool.
        std::fs::write(root.join(CONFIG_FILE), FOLDER).expect("root file");
        let ctx = TargetContext::load(&root).expect("root");
        assert_eq!(ctx.tool, None);
        assert_eq!(ctx.ledger, root.join("migration"));
        assert_eq!(ctx.config.target.source_dir(), Some("src"));
        let ctx = TargetContext::open(&root, Some("t-lzg")).expect("tool");
        assert_eq!(ctx.config.target.source_dir(), None);
        // Every ledger path of a tool lies under migration/tools/<id>/.
        let ledger = crate::ledger::Ledger::of(&ctx);
        let under = root.join("migration/tools/t-lzg");
        for path in [
            ledger.dir(),
            ledger.facts_path(),
            ledger.plan_path(),
            ledger.unit_dir("u-a"),
            ledger.verdict_latest_path("u-a"),
            ledger.driver_path("u-a"),
            ledger.build_dir(),
            ledger.lock_path(),
            crate::features::features_path(&ledger),
            crate::features::map_path(&ledger),
            crate::perf::perf_dir(&ledger),
            crate::perf::workloads::workloads_path(&ledger),
            crate::observer::ObserverPaths::findings(&ledger),
        ] {
            assert!(path.starts_with(&under), "{}", path.display());
        }
        // A tool folder that is a link is no tool.
        std::fs::remove_dir_all(tool_dir(&root, "t-x")).expect("rm");
        std::os::unix::fs::symlink(tool_dir(&root, "t-lzg"), tool_dir(&root, "t-x")).expect("ln");
        assert_eq!(mapped_tools(&root), ["t-lzg"]);
    }
}
