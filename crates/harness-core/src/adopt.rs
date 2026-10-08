//! Adoption of a ledger made elsewhere (docs/PROJECT-MAP-DESIGN.md §3.7).
//!
//! A download can ship a ready-made `migration/` — green verdicts, a promoted
//! crate with a build script, a `target/` folder. Nothing inside the project
//! can prove who made it, so trust is decided once per checkout (each folder
//! on this computer, by its canonical path: a second worktree of the same
//! project is asked again) and kept OUTSIDE the project, in the **adoption
//! file**: `$RUHARNESS_ADOPTED` when set, else
//! `~/Library/Application Support/ruharness/adopted.toml` (the same spelling
//! on Linux). It lists the canonical roots whose ledgers the harness created
//! on this computer and the roots the person adopted, each with a random
//! token also written into the project (`<root>/migration/.ruharness-adopted`
//! for a project, `<suite>/.ruharness-adopted` for a benchmark suite), so a
//! different tree unpacked at the same path is not trusted by its path alone.
//!
//! [`check`] is the one test every reader goes through
//! ([`TargetContext::load`](crate::TargetContext::load) calls it): a ledger
//! whose root is not listed, or whose token is missing or different, is
//! refused with [`Error::NotAdopted`]; a `migration/` that holds no results
//! (only hand-written tool files and the map's `config.toml`) needs no
//! adoption, and one that is not the harness's at all is refused as the
//! project's own first. [`adopt`] and [`adopt_suite`] are the person's
//! `--adopt` (a project's adoption always writes a fresh token);
//! [`record_created`] is the first command that makes a ledger. An adoption
//! records its time; a verdict written before it is shown as made
//! elsewhere until `harness verify` runs it here ([`made_before_adoption`]).
//! The ledger alone still holds everything needed to resume; only the trust
//! question is asked once per checkout.

use crate::error::Error;
use crate::ledger::{read_regular, write_atomic, MIGRATION_DIR};
use crate::plan::{Plan, UnitStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The environment variable naming the adoption file (tests point it at a
/// temporary file; the person may move theirs).
pub const ADOPTED_ENV: &str = "RUHARNESS_ADOPTED";

/// The token file's name: inside `migration/` for a project, at the top of
/// a benchmark suite.
pub const TOKEN_FILE: &str = ".ruharness-adopted";

/// The ledger's fixed names (docs/SCHEMAS.md "The ledger's fixed names"): a
/// `migration/` holding anything else is the project's own.
pub const FIXED_NAMES: &[&str] = &[
    "facts.jsonl",
    "plan.toml",
    "DECISIONS.md",
    "observer",
    "units",
    "features",
    "perf",
    "map",
    "tools",
    "build",
    ".lock",
    ".gitignore",
    TOKEN_FILE,
];

/// A `migration/` is the harness's only when it holds one of these.
const HARNESS_MARKERS: &[&str] = &["facts.jsonl", "plan.toml", "map", "tools"];

/// Finder's folder file, ignored wherever it appears.
const IGNORED: &str = ".DS_Store";

/// The adoption file's schema version.
pub const ADOPTED_SCHEMA_VERSION: u64 = 1;

const MAX_ADOPTED_BYTES: u64 = 4 << 20;
const MAX_TOKEN_BYTES: u64 = 256;
const MAX_PLAN_BYTES: u64 = 64 << 20;
/// How deep a suite's `cases/` folder is searched for ledgers.
const MAX_CASE_DEPTH: usize = 6;

/// What an adopted root covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// One project: the ledger at `<root>/migration/`, its token inside it.
    Project,
    /// A benchmark suite: every ledger under the root, the token at
    /// `<suite>/.ruharness-adopted`.
    Suite,
}

/// How a root came to be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// A command on this computer created its first ledger.
    Created,
    /// The person adopted it (`--adopt`, or the cockpit's dialog).
    Adopted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    token: String,
    scope: Scope,
    how: How,
    /// The hash of the `migration/map/config.toml` the project held when a
    /// command first recorded it (`how = created`): a configuration that
    /// came with the project, which the map shows as proposed while the
    /// file keeps that hash. Never set by the person's `--adopt`, which
    /// states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shipped_config: Option<String>,
    /// When the person adopted it, in milliseconds since 1970 (UTC): a
    /// verdict written before this was made elsewhere ([`adopted_at`]).
    /// Absent for a root made here, and for an adoption recorded before the
    /// time was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adopted_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdoptedFile {
    schema_version: u64,
    #[serde(default, rename = "root")]
    roots: Vec<Entry>,
}

impl Default for AdoptedFile {
    fn default() -> Self {
        AdoptedFile {
            schema_version: ADOPTED_SCHEMA_VERSION,
            roots: Vec::new(),
        }
    }
}

/// What [`adopt`] or [`adopt_suite`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    /// The canonical root.
    pub root: PathBuf,
    /// `false` when the root was already listed with its token (nothing was
    /// deleted), or held no ledger at all (nothing to adopt).
    pub newly: bool,
    /// The root holds a ledger (for a suite: at least one case does).
    pub had_ledger: bool,
    /// The build folders deleted (paths; a link is deleted, never its target).
    pub deleted: Vec<PathBuf>,
    /// Units in the plan(s).
    pub units: usize,
    /// Of which verified or merged.
    pub verified: usize,
}

impl Adoption {
    /// The lines a command prints for it, in plain words; `tool` is the
    /// mapped tool the command opens, so the commands it names run as
    /// printed.
    pub fn describe(&self, tool: Option<&str>) -> Vec<String> {
        let root = self.root.display();
        if !self.had_ledger {
            return vec![format!(
                "adopt: {root} holds no migration results yet; nothing to adopt"
            )];
        }
        if !self.newly {
            return vec![format!(
                "adopt: {root} is already trusted on this computer; nothing deleted"
            )];
        }
        let mut lines = vec![format!(
            "adopt: {root} is now trusted on this computer ({})",
            counted(self.units, self.verified)
        )];
        // Each half only when it is true: deletions when something was
        // deleted, claims when something is verified.
        if !self.deleted.is_empty() {
            let n = self.deleted.len();
            let s = if n == 1 { "" } else { "s" };
            lines.push(format!(
                "adopt: deleted {n} build folder{s} made elsewhere; the harness builds what it \
                 needs again here"
            ));
        }
        if self.verified > 0 {
            let (n, s) = (self.verified, if self.verified == 1 { "" } else { "s" });
            lines.push(format!(
                "adopt: {n} verified unit{s} came with it, marked \"made elsewhere\" until you run \
                 `{}` here (`{}` lists them)",
                crate::runtime_view::command_line("verify <unit>", tool),
                crate::runtime_view::command_line("state status", tool),
            ));
        }
        lines
    }
}

/// Who a not-adopted refusal speaks to: each reader is told only its own
/// way to adopt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// A `harness` command: add `--adopt` once.
    Command,
    /// The cockpit away from a terminal: start it in one and answer.
    Cockpit,
    /// An agent (harness-mcp): ask the person; an agent never adopts.
    Agent,
}

/// The not-adopted refusal, one sentence: the folder, what it claims, and
/// what `way`'s reader does next.
pub fn refusal(root: &Path, units: usize, verified: usize, way: Way) -> String {
    let head = made_elsewhere(root, units, verified);
    match way {
        Way::Command => format!("{head}: to trust them here, add `--adopt` once"),
        Way::Cockpit => format!(
            "{head}: to trust them here, start the cockpit in a terminal and answer its question"
        ),
        Way::Agent => agent_refusal(root, units, verified, None),
    }
}

/// The refusal an agent (harness-mcp) meets: ask the person, with the
/// command spelled for the server's own `--tool` so it runs as printed.
pub fn agent_refusal(root: &Path, units: usize, verified: usize, tool: Option<&str>) -> String {
    let mut command = format!(
        "harness state status --adopt --target {}",
        crate::runtime_view::shell_quote(&root.display().to_string())
    );
    if let Some(id) = tool {
        command.push_str(&format!(" --tool {id}"));
    }
    format!(
        "{}: ask the person to adopt it (`{command}`, or the cockpit's question); an agent never \
         adopts",
        made_elsewhere(root, units, verified)
    )
}

/// The refusal's first half: the folder and what its ledger claims.
pub fn made_elsewhere(root: &Path, units: usize, verified: usize) -> String {
    format!(
        "{}: this folder already holds migration results made elsewhere ({})",
        root.display(),
        counted(units, verified)
    )
}

/// `3 units, 1 verified` (the refusal's words).
pub fn counted(units: usize, verified: usize) -> String {
    let s = if units == 1 { "" } else { "s" };
    format!("{units} unit{s}, {verified} verified")
}

/// The adoption file's path: `$RUHARNESS_ADOPTED` when set (and not empty),
/// else `~/Library/Application Support/ruharness/adopted.toml`.
pub fn adoption_file() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os(ADOPTED_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            Error::Invariant(
                "cannot find the adoption file: neither $RUHARNESS_ADOPTED nor $HOME is set; \
                 set one of them"
                    .into(),
            )
        })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("ruharness")
        .join("adopted.toml"))
}

/// Where a root's token lives.
pub fn token_path(root: &Path, scope: Scope) -> PathBuf {
    match scope {
        Scope::Project => root.join(MIGRATION_DIR).join(TOKEN_FILE),
        Scope::Suite => root.join(TOKEN_FILE),
    }
}

/// `root` holds a ledger: `<root>/migration` exists (as anything — a link
/// to a folder counts too).
pub fn has_ledger(root: &Path) -> bool {
    std::fs::symlink_metadata(root.join(MIGRATION_DIR)).is_ok()
}

/// `root` holds results: its `migration/` exists and is not one that holds
/// none ([`holds_no_results`]). Only such a folder is asked about.
pub fn holds_results(root: &Path) -> bool {
    has_ledger(root) && !holds_no_results(&root.join(MIGRATION_DIR))
}

/// Does `dir` (a `migration/` folder) hold no results? It does when it is a
/// real folder holding only what a person writes by hand —
/// `tools/<id>/harness.toml` files and `map/config.toml` — and the
/// harness's bookkeeping (`.gitignore`, the writer locks, the token;
/// `.DS_Store` ignored): no facts, plan, units, map file or verdicts. Such
/// a folder is the project's own ledger, made here; an empty one too.
pub fn holds_no_results(dir: &Path) -> bool {
    let only_files = |dir: &Path, allowed: &[&str]| {
        is_real_dir(dir)
            && names_in(dir).is_some_and(|names| {
                names
                    .iter()
                    .all(|(name, path)| allowed.contains(&name.as_str()) && is_real_file(path))
            })
    };
    if !is_real_dir(dir) {
        return false;
    }
    let Some(names) = names_in(dir) else {
        return false;
    };
    names.iter().all(|(name, path)| match name.as_str() {
        ".gitignore" | ".lock" | TOKEN_FILE => is_real_file(path),
        "map" => only_files(path, &["config.toml", ".lock"]),
        "tools" => {
            is_real_dir(path)
                && names_in(path).is_some_and(|tools| {
                    tools
                        .iter()
                        .all(|(_, tool)| only_files(tool, &["harness.toml", ".lock"]))
                })
        }
        _ => false,
    })
}

/// The entries of `dir` with their paths, `.DS_Store` left out; `None` when
/// it cannot be read or holds a name that is not UTF-8.
fn names_in(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().into_string().ok()?;
        if name != IGNORED {
            out.push((name, entry.path()));
        }
    }
    Some(out)
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// Is `dir` (a `migration/` folder) the harness's? It is when it is a real
/// folder holding `facts.jsonl`, `plan.toml`, `map/` or `tools/` and nothing
/// outside [`FIXED_NAMES`] (`.DS_Store` ignored); otherwise it is the
/// project's own.
pub fn is_harness_ledger(dir: &Path) -> bool {
    if !is_real_dir(dir) {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut marker = false;
    for entry in entries {
        let Ok(entry) = entry else { return false };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        if name == IGNORED {
            continue;
        }
        if !FIXED_NAMES.contains(&name) {
            return false;
        }
        marker |= HARNESS_MARKERS.contains(&name);
    }
    marker
}

// ---------- the roots trusted for this process ----------

/// Roots whose ledger this very process created (they held none when it
/// started): trusted until the process records them in the adoption file.
static CREATED_HERE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Note that `root` held no ledger when this command started: what the
/// command itself writes there is trusted for the rest of the process (the
/// command records it with [`record_created`] when it ends).
pub fn note_created_here(root: &Path) {
    let root = canonical(root);
    if let Ok(mut roots) = CREATED_HERE.lock() {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
}

fn created_here(root: &Path) -> bool {
    CREATED_HERE
        .lock()
        .map(|roots| root.ancestors().any(|a| roots.iter().any(|r| r == a)))
        .unwrap_or(false)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

// ---------- the check ----------

/// The one check every reader of a ledger goes through: `Ok` when `root`
/// holds no results ([`holds_results`]), or its root (or a benchmark suite
/// above it) is listed in the adoption file with the token the project
/// holds; else [`Error::ForeignMigration`] when its `migration/` is not the
/// harness's at all (said before any adoption question), else
/// [`Error::NotAdopted`], saying how many units and verified units the
/// ledger claims.
pub fn check(root: &Path) -> Result<(), Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    if !holds_results(&root) || created_here(&root) {
        return Ok(());
    }
    let file = read_adopted(&adoption_file()?)?;
    if trusted(&file, &root) {
        return Ok(());
    }
    if !is_harness_ledger(&root.join(MIGRATION_DIR)) {
        return Err(Error::ForeignMigration { root });
    }
    let (units, verified) = count_units(&root);
    Err(Error::NotAdopted {
        root,
        units,
        verified,
    })
}

/// [`check`] for a benchmark suite: `Ok` when no case under it holds a
/// ledger, or the suite is listed with its token; else
/// [`Error::NotAdopted`] with the cases' totals.
pub fn check_suite(suite: &Path) -> Result<(), Error> {
    let suite = suite.canonicalize().map_err(|e| Error::io(suite, e))?;
    let cases = case_ledgers(&suite);
    if cases.is_empty() || created_here(&suite) {
        return Ok(());
    }
    let file = read_adopted(&adoption_file()?)?;
    if listed(&file, &suite, Scope::Suite) {
        return Ok(());
    }
    let (units, verified) = cases.iter().fold((0, 0), |(u, v), case| {
        let (cu, cv) = count_units(case);
        (u + cu, v + cv)
    });
    Err(Error::NotAdopted {
        root: suite,
        units,
        verified,
    })
}

/// Whether `suite` holds a case with a ledger.
pub fn suite_has_ledger(suite: &Path) -> bool {
    !case_ledgers(&canonical(suite)).is_empty()
}

fn trusted(file: &AdoptedFile, root: &Path) -> bool {
    listed(file, root, Scope::Project)
        || root
            .ancestors()
            .any(|a| a != root && listed(file, a, Scope::Suite))
}

/// `root` is listed with `scope` and the token its project holds.
fn listed(file: &AdoptedFile, root: &Path, scope: Scope) -> bool {
    let Some(token) = read_token(&token_path(root, scope)) else {
        return false;
    };
    file.roots
        .iter()
        .any(|e| e.scope == scope && e.path == root && e.token == token)
}

/// The token a project holds, when it is a well-formed one (32 lowercase
/// hex digits in a regular file, never read through a link).
fn read_token(path: &Path) -> Option<String> {
    let bytes = read_regular(path, MAX_TOKEN_BYTES).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let token = text.trim();
    (token.len() == 32
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    .then(|| token.to_string())
}

/// Units and verified units the project's ledgers claim: the root's
/// `migration/` and each mapped tool's under `migration/tools/` (0, 0 for a
/// ledger with no readable plan).
fn count_units(root: &Path) -> (usize, usize) {
    let migration = root.join(MIGRATION_DIR);
    std::iter::once(migration.clone())
        .chain(real_dirs(&migration.join(crate::config::TOOLS_DIR)))
        .map(|ledger| count_plan(&ledger.join("plan.toml")))
        .fold((0, 0), |(u, v), (cu, cv)| (u + cu, v + cv))
}

/// Units and verified units one plan claims.
fn count_plan(path: &Path) -> (usize, usize) {
    let Ok(bytes) = read_regular(path, MAX_PLAN_BYTES) else {
        return (0, 0);
    };
    let Ok(plan) = Plan::parse(path, &String::from_utf8_lossy(&bytes)) else {
        return (0, 0);
    };
    let verified = plan
        .units
        .iter()
        .filter(|u| matches!(u.status, UnitStatus::Verified | UnitStatus::Merged))
        .count();
    (plan.units.len(), verified)
}

/// The case roots under `<suite>/cases/` that hold a ledger (real folders
/// only; a case's own folders are not searched below its ledger).
fn case_ledgers(suite: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if has_ledger(dir) {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for d in dirs {
            walk(&d, depth - 1, out);
        }
    }
    let mut out = Vec::new();
    let cases = suite.join(crate::bench::CASES_DIR);
    if std::fs::symlink_metadata(&cases).is_ok_and(|m| m.file_type().is_dir()) {
        walk(&cases, MAX_CASE_DEPTH, &mut out);
    }
    out
}

// ---------- the adoption file ----------

fn read_adopted(path: &Path) -> Result<AdoptedFile, Error> {
    // The person's own file, outside every project, replaced whole by a
    // rename while others read it: one open, then a bounded read of that
    // handle (never a look-then-open, which a concurrent rename would fail).
    let bytes = match std::fs::File::open(path) {
        Ok(f) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            f.take(MAX_ADOPTED_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| Error::io(path, e))?;
            if bytes.len() as u64 > MAX_ADOPTED_BYTES {
                return Err(Error::parse(
                    path,
                    format!("longer than {MAX_ADOPTED_BYTES} bytes; fix or delete the file"),
                ));
            }
            bytes
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AdoptedFile::default()),
        Err(e) => return Err(Error::io(path, e)),
    };
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::parse(path, "not UTF-8; fix or delete the file"))?;
    let file: AdoptedFile = toml::from_str(&text)
        .map_err(|e| Error::parse(path, format!("{e}; fix or delete the file")))?;
    if file.schema_version > ADOPTED_SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            path: path.into(),
            found: file.schema_version,
            supported: ADOPTED_SCHEMA_VERSION,
        });
    }
    Ok(file)
}

/// Hold the adoption file's lock (an exclusive `flock` on
/// `<file>.lock`, waited for) while `change` edits the file; entries whose
/// root no longer exists are dropped, and the file is replaced atomically.
fn update<T>(change: impl FnOnce(&mut AdoptedFile) -> Result<T, Error>) -> Result<T, Error> {
    let path = adoption_file()?;
    let dir = path
        .parent()
        .ok_or_else(|| Error::Invariant(format!("{} has no parent folder", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let mut lock_name = path.file_name().unwrap_or_default().to_os_string();
    lock_name.push(".lock");
    let lock_path = dir.join(lock_name);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| Error::io(&lock_path, e))?;
    lock.lock().map_err(|e| Error::io(&lock_path, e))?;
    let mut file = read_adopted(&path)?;
    let out = change(&mut file)?;
    file.roots
        .retain(|e| std::fs::symlink_metadata(&e.path).is_ok());
    file.schema_version = ADOPTED_SCHEMA_VERSION;
    let mut text = String::from(
        "# RuHarness: the folders whose migration results this computer trusts.\n\
         # Written by the harness (`--adopt`, and the first command that makes a ledger).\n",
    );
    text.push_str(
        &toml::to_string(&file)
            .map_err(|e| Error::Invariant(format!("cannot write the adoption file: {e}")))?,
    );
    write_atomic(&path, text.as_bytes())?;
    drop(lock);
    Ok(out)
}

/// A fresh random token written at `path`, replacing any there: what the
/// person's `--adopt` of a project and the first command that makes a
/// project's ledger record, so a token a download shipped is never trusted.
fn fresh_token(path: &Path) -> Result<String, Error> {
    let token = crate::hash::random_hex(16);
    write_atomic(path, format!("{token}\n").as_bytes())?;
    Ok(token)
}

/// The token at `path` when well formed, else a new one written there —
/// made only when no file is there, so processes racing on one fixture
/// agree on one token. Only the test helper and a benchmark suite's
/// adoption (whose trust is `corpus.lock`) keep a token already there.
fn ensure_token(path: &Path) -> Result<String, Error> {
    if let Some(token) = read_token(path) {
        return Ok(token);
    }
    use std::io::Write;
    let token = crate::hash::random_hex(16);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    // Created only when nothing is there (never through a link): the one
    // that creates it writes its 33 bytes; another that finds it reads it
    // once written.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(format!("{token}\n").as_bytes())
                .map_err(|e| Error::io(path, e))?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            for _ in 0..100 {
                if let Some(theirs) = read_token(path) {
                    return Ok(theirs);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // A file there that is no token: replaced.
            fresh_token(path)
        }
        Err(e) => Err(Error::io(path, e)),
    }
}

/// The token a root is recorded with: a project's is always fresh; a
/// benchmark suite keeps the one it holds.
fn token_for(root: &Path, scope: Scope) -> Result<String, Error> {
    match scope {
        Scope::Project => fresh_token(&token_path(root, scope)),
        Scope::Suite => ensure_token(&token_path(root, scope)),
    }
}

fn upsert(file: &mut AdoptedFile, root: &Path, token: String, scope: Scope, how: How) {
    file.roots.retain(|e| e.path != root);
    file.roots.push(Entry {
        path: root.to_path_buf(),
        token,
        scope,
        how,
        // At first sight, a project's `config.toml` came with it.
        shipped_config: match (scope, how) {
            (Scope::Project, How::Created) => map_config_hash(root),
            _ => None,
        },
        adopted_ms: None,
    });
    file.roots.sort_by(|a, b| a.path.cmp(&b.path));
}

// ---------- the map's configuration that came with the project ----------

/// The map's configuration file, relative to the project root.
const MAP_CONFIG: &str = "map/config.toml";

/// The hash (`blake3:<hex>` of its bytes) of `root`'s
/// `migration/map/config.toml` when it is a regular file of at most 1 MiB.
fn map_config_hash(root: &Path) -> Option<String> {
    let path = root.join(MIGRATION_DIR).join(MAP_CONFIG);
    read_regular(&path, 1 << 20)
        .ok()
        .map(|b| crate::hash::bytes_hash(&b))
}

/// The person stated the project's `config.toml` (`--adopt`): the hash
/// recorded at first sight is dropped.
fn state_config(file: &mut AdoptedFile, root: &Path) {
    for e in file.roots.iter_mut().filter(|e| e.path == root) {
        e.shipped_config = None;
    }
}

/// The hash of the `migration/map/config.toml` that came with the project
/// at `root` (docs/PROJECT-MAP-DESIGN.md §3.2): for a root this computer
/// has recorded with its token, the file's hash when it was first recorded
/// (`None` when it held none then, or the person's `--adopt` stated it);
/// for a root not recorded yet, the file's hash now — it is being seen for
/// the first time. While the file still has this hash, the map shows its
/// configuration as proposed by the project, never as the person's.
pub fn shipped_config_hash(root: &Path) -> Option<String> {
    let root = canonical(root);
    let Ok(file) = adoption_file().and_then(|p| read_adopted(&p)) else {
        return map_config_hash(&root);
    };
    if listed(&file, &root, Scope::Project) {
        return file
            .roots
            .iter()
            .find(|e| e.scope == Scope::Project && e.path == root)
            .and_then(|e| e.shipped_config.clone());
    }
    if trusted(&file, &root) {
        // Under an adopted suite: the person's.
        return None;
    }
    map_config_hash(&root)
}

/// Milliseconds since 1970 now (0 on a clock set before 1970).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// When the person adopted the root that covers `root` (the project
/// itself, or a benchmark suite above it), as recorded in the adoption
/// file; `None` when it was made here, is not listed with the token it
/// holds, or was adopted before the time was kept. A verdict written before
/// this time was made elsewhere until `harness verify` runs it here.
pub fn adopted_at(root: &Path) -> Option<std::time::SystemTime> {
    let root = root.canonicalize().ok()?;
    let file = read_adopted(&adoption_file().ok()?).ok()?;
    let entry = root.ancestors().find_map(|a| {
        let scope = if a == root {
            Scope::Project
        } else {
            Scope::Suite
        };
        listed(&file, a, scope)
            .then(|| file.roots.iter().find(|e| e.path == a && e.scope == scope))
            .flatten()
    })?;
    let ms = entry.adopted_ms.filter(|_| entry.how == How::Adopted)?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

/// Was the verdict file at `verdict` written before the person adopted
/// `root` ([`adopted_at`])? A verdict records no time of its own (it binds
/// content, never timestamps: docs/SCHEMAS.md "Verdicts"), so its file's
/// modification time stands in. That is the weakness: a tool that sets
/// times (`touch`, an unpacker that restores none) can make a verdict made
/// elsewhere look made here, and a clock set back can do the opposite; a
/// copy or a checkout gives it the time of the copy, which comes before
/// the adoption, as it should.
pub fn made_before_adoption(root: &Path, verdict: &Path) -> bool {
    let Some(adopted) = adopted_at(root) else {
        return false;
    };
    std::fs::symlink_metadata(verdict)
        .and_then(|m| m.modified())
        .is_ok_and(|written| written < adopted)
}

/// Record that a command on this computer created `root`'s first ledger
/// (for a suite: its cases' first ledgers): a fresh token is written into
/// the project (a suite keeps its own) and the root listed. Nothing when it
/// is already listed, or holds no ledger after all.
pub fn record_created(root: &Path, scope: Scope) -> Result<(), Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    let holds = match scope {
        Scope::Project => has_ledger(&root),
        Scope::Suite => suite_has_ledger(&root),
    };
    if !holds {
        return Ok(());
    }
    update(|file| {
        if listed(file, &root, scope) {
            return Ok(());
        }
        let token = token_for(&root, scope)?;
        upsert(file, &root, token, scope, How::Created);
        Ok(())
    })
}

/// The person's `--adopt` for one project: a root already listed with its
/// token is left as it is (nothing deleted); a `migration/` that is the
/// project's own is refused ([`Error::ForeignMigration`]); one that holds no
/// results has nothing to adopt (the command records it as made here);
/// otherwise the harness's build folders made elsewhere are deleted (see
/// [`delete_build_folders`]) and the root recorded with a fresh token,
/// replacing any the folder holds.
pub fn adopt(root: &Path) -> Result<Adoption, Error> {
    adopt_inner(root, Scope::Project, true)
}

/// The person's `--adopt` for a benchmark suite: the suite root adopted as
/// one root covering every case ledger under it, with the token the suite
/// holds (else a new one).
pub fn adopt_suite(suite: &Path) -> Result<Adoption, Error> {
    adopt_inner(suite, Scope::Suite, true)
}

fn adopt_inner(root: &Path, scope: Scope, delete: bool) -> Result<Adoption, Error> {
    let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
    let ledgers = match scope {
        Scope::Project => {
            if holds_results(&root) {
                vec![root.clone()]
            } else {
                Vec::new()
            }
        }
        Scope::Suite => case_ledgers(&root),
    };
    let (units, verified) = ledgers.iter().fold((0, 0), |(u, v), r| {
        let (cu, cv) = count_units(r);
        (u + cu, v + cv)
    });
    let mut adoption = Adoption {
        root: root.clone(),
        newly: false,
        had_ledger: !ledgers.is_empty(),
        deleted: Vec::new(),
        units,
        verified,
    };
    // The person's `--adopt` also states a `config.toml` that came with the
    // project (§3.2), even in a folder that holds no results yet.
    let states_config = scope == Scope::Project && map_config_hash(&root).is_some();
    if ledgers.is_empty() {
        if states_config {
            update(|file| {
                if listed(file, &root, scope) {
                    state_config(file, &root);
                } else {
                    let token = token_for(&root, scope)?;
                    upsert(file, &root, token, scope, How::Adopted);
                }
                Ok(())
            })?;
        }
        return Ok(adoption);
    }
    update(|file| {
        if listed(file, &root, scope) {
            state_config(file, &root);
            return Ok(());
        }
        for ledger_root in &ledgers {
            if !is_harness_ledger(&ledger_root.join(MIGRATION_DIR)) {
                return Err(Error::ForeignMigration {
                    root: ledger_root.clone(),
                });
            }
        }
        if delete {
            for ledger_root in &ledgers {
                adoption
                    .deleted
                    .extend(delete_build_folders(&ledger_root.join(MIGRATION_DIR))?);
            }
        }
        let token = token_for(&root, scope)?;
        upsert(file, &root, token, scope, How::Adopted);
        // The adoption's time: every verdict written before it is shown as
        // made elsewhere until `verify` runs it here.
        if let Some(entry) = file.roots.iter_mut().find(|e| e.path == root) {
            entry.adopted_ms = Some(now_ms());
        }
        adoption.newly = true;
        Ok(())
    })?;
    Ok(adoption)
}

/// Delete the build folders a ledger made elsewhere may carry, and only
/// those: `migration/build/`, each unit crate's `target/`, each attempt's
/// `candidate/target/` and every `units/<id>/.promote-*/`, in the root's
/// ledger and in each mapped tool's under `migration/tools/`. A link in any
/// of those places is deleted itself, never its target; a linked folder is
/// never descended into. Returns what was deleted.
pub fn delete_build_folders(migration: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut deleted = delete_ledger_builds(migration)?;
    for tool in real_dirs(&migration.join(crate::config::TOOLS_DIR)) {
        deleted.extend(delete_ledger_builds(&tool)?);
    }
    Ok(deleted)
}

/// [`delete_build_folders`] for one ledger folder.
fn delete_ledger_builds(migration: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut deleted = Vec::new();
    remove(&migration.join("build"), &mut deleted)?;
    for unit in real_dirs(&migration.join("units")) {
        let Ok(entries) = std::fs::read_dir(&unit) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".promote-") {
                remove(&path, &mut deleted)?;
            } else if name == "attempts" {
                for attempt in real_dirs(&path) {
                    let candidate = attempt.join("candidate");
                    if is_real_dir(&candidate) {
                        remove(&candidate.join("target"), &mut deleted)?;
                    }
                }
            } else if is_real_dir(&path) {
                remove(&path.join("target"), &mut deleted)?;
            }
        }
    }
    Ok(deleted)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// The real folders directly inside `dir` (none when `dir` is a link).
fn real_dirs(dir: &Path) -> Vec<PathBuf> {
    if !is_real_dir(dir) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_real_dir(p))
        .collect();
    out.sort();
    out
}

/// Remove `path`: a link or a file itself, a folder with everything in it
/// (`remove_dir_all` never follows a link inside).
fn remove(path: &Path, deleted: &mut Vec<PathBuf>) -> Result<(), Error> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(path, e)),
    };
    if meta.file_type().is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| Error::io(path, e))?;
    } else {
        std::fs::remove_file(path).map_err(|e| Error::io(path, e))?;
    }
    deleted.push(path.to_path_buf());
    Ok(())
}

/// The test suite's adoption (docs/PROJECT-MAP-DESIGN.md §3.7): every test
/// that opens a ledger goes through here, so no test ever writes the
/// person's own adoption file.
pub mod testing {
    use super::*;
    use std::sync::OnceLock;

    static FILE: OnceLock<PathBuf> = OnceLock::new();

    /// Point `$RUHARNESS_ADOPTED` at a temporary file, once per test process
    /// (children the test starts inherit it); its path.
    pub fn adoption_file() -> &'static Path {
        FILE.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!(
                "ruharness-adopted-test-{}-{}",
                std::process::id(),
                crate::hash::random_hex(4)
            ));
            // A test utility: a temp folder that cannot be made is a broken
            // test machine, and the test must stop there.
            std::fs::create_dir_all(&dir).expect("make the test adoption folder");
            let file = dir.join("adopted.toml");
            std::env::set_var(ADOPTED_ENV, &file);
            // RuHarness's committed fixtures, adopted as they stand for this
            // test process: zopfli and the benchmark suite. Their tokens are
            // not committed (a person adopts them once per computer); the
            // first test process to need one writes it, and every other
            // process, running at the same time, records that same token.
            let targets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets");
            let fixtures = [
                ("zopfli", MIGRATION_DIR, Scope::Project),
                ("tractor", crate::bench::CASES_DIR, Scope::Suite),
            ];
            for (fixture, inside, scope) in fixtures {
                let root = targets.join(fixture);
                if is_real_dir(&root.join(inside)) {
                    record(&root, scope);
                }
            }
            file
        })
    }

    /// List `root` with the token it holds (or a new one, made only when no
    /// other process made one first), deleting nothing.
    fn record(root: &Path, scope: Scope) {
        let root = root.canonicalize().expect("the fixture root exists");
        update(|file| {
            if !listed(file, &root, scope) {
                let token = ensure_token(&token_path(&root, scope))?;
                upsert(file, &root, token, scope, How::Adopted);
            }
            Ok(())
        })
        .expect("adopt the test fixture");
    }

    /// Adopt the fixture at `root` for this test process, through the same
    /// file and token rules as the person's `--adopt` but deleting nothing
    /// (tests share the repository's fixtures): a root under a folder that
    /// holds a suite token (`targets/tractor`) adopts that suite; a root
    /// with no ledger yet gets an empty `migration/` holding its token, as
    /// the first command that makes a ledger would leave it.
    pub fn adopt(root: impl AsRef<Path>) {
        adoption_file();
        let root = root
            .as_ref()
            .canonicalize()
            .expect("the fixture root exists");
        if let Some(suite) = root
            .ancestors()
            .find(|a| a.join(TOKEN_FILE).is_file() && a.join(crate::bench::CASES_DIR).is_dir())
        {
            record(suite, Scope::Suite);
            return;
        }
        if !has_ledger(&root) {
            std::fs::create_dir_all(root.join(MIGRATION_DIR)).expect("make migration/");
        }
        // Already listed with its token: nothing to write (no lock taken).
        let listed_now =
            read_adopted(adoption_file()).is_ok_and(|file| listed(&file, &root, Scope::Project));
        if !listed_now {
            record(&root, Scope::Project);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ruharness-adopt-{tag}-{}-{}",
            std::process::id(),
            crate::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A ledger as another computer would ship it: a plan with two units,
    /// one verified, build folders, a crate's target and a candidate's.
    fn shipped(root: &Path) {
        let m = root.join("migration");
        std::fs::create_dir_all(m.join("build/x")).unwrap();
        std::fs::write(m.join("facts.jsonl"), "").unwrap();
        std::fs::write(
            m.join("plan.toml"),
            "schema_version = 1\n\n[[unit]]\nid = \"u1\"\nsymbols = [\"a\"]\nfiles = []\n\
             status = \"verified\"\n\n[[unit]]\nid = \"u2\"\nsymbols = [\"b\"]\nfiles = []\n\
             status = \"pending\"\n",
        )
        .unwrap();
        let unit = m.join("units/u1");
        std::fs::create_dir_all(unit.join("u1_rs/target/debug")).unwrap();
        std::fs::create_dir_all(unit.join("u1_rs/src")).unwrap();
        std::fs::write(unit.join("u1_rs/src/lib.rs"), "").unwrap();
        std::fs::create_dir_all(unit.join("attempts/a-1/candidate/target")).unwrap();
        std::fs::create_dir_all(unit.join("attempts/a-1/candidate/src")).unwrap();
        std::fs::create_dir_all(unit.join(".promote-a-1")).unwrap();
        std::fs::write(unit.join("contract.md"), "keep").unwrap();
    }

    #[test]
    fn a_ledger_at_an_unlisted_root_is_refused_with_the_sentence() {
        testing::adoption_file();
        let root = tmp("unlisted");
        shipped(&root);
        let err = check(&root).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{}: this folder already holds migration results made elsewhere (2 units, 1 \
                 verified): to trust them here, add `--adopt` once",
                root.display()
            )
        );
        // Each reader is told its own way, and an agent never adopts.
        let Error::NotAdopted {
            units, verified, ..
        } = err
        else {
            panic!("not the refusal");
        };
        assert!(refusal(&root, units, verified, Way::Cockpit).ends_with(
            "to trust them here, start the cockpit in a terminal and answer its question"
        ));
        let agent = refusal(&root, units, verified, Way::Agent);
        assert!(
            agent.ends_with(&format!(
                "ask the person to adopt it (`harness state status --adopt --target {}`, or the \
                 cockpit's question); an agent never adopts",
                root.display()
            )),
            "{agent}"
        );
        // The server's tool is spelled, so the command runs as printed in a
        // project with several tools.
        assert!(
            agent_refusal(&root, units, verified, Some("t-lzg")).ends_with(&format!(
                "(`harness state status --adopt --target {} --tool t-lzg`, or the cockpit's \
             question); an agent never adopts",
                root.display()
            ))
        );
        // A folder with no ledger has nothing to trust.
        assert!(check(&tmp("empty")).is_ok());
    }

    #[test]
    fn adopting_records_the_root_and_deletes_only_the_build_folders() {
        testing::adoption_file();
        let root = tmp("adopt");
        shipped(&root);
        // A link where a crate's target would be: the link goes, its target stays.
        let outside = tmp("outside");
        std::fs::write(outside.join("precious"), "x").unwrap();
        let unit = root.join("migration/units/u1");
        std::fs::create_dir_all(unit.join("v_rs")).unwrap();
        std::os::unix::fs::symlink(&outside, unit.join("v_rs/target")).unwrap();
        let done = adopt(&root).unwrap();
        assert!(done.newly);
        assert_eq!((done.units, done.verified), (2, 1));
        let m = root.join("migration");
        assert!(!m.join("build").exists());
        assert!(!unit.join("u1_rs/target").exists());
        assert!(unit.join("u1_rs/src/lib.rs").exists());
        assert!(!unit.join("attempts/a-1/candidate/target").exists());
        assert!(unit.join("attempts/a-1/candidate/src").exists());
        assert!(!unit.join(".promote-a-1").exists());
        assert!(std::fs::symlink_metadata(unit.join("v_rs/target")).is_err());
        assert!(outside.join("precious").exists());
        assert!(unit.join("contract.md").exists());
        assert!(m.join("facts.jsonl").exists());
        assert_eq!(done.deleted.len(), 5, "{:?}", done.deleted);
        // The token is in the project and in the file; the check passes.
        let token = read_token(&m.join(TOKEN_FILE)).unwrap();
        let file = read_adopted(testing::adoption_file()).unwrap();
        assert!(file
            .roots
            .iter()
            .any(|e| e.path == root && e.token == token && e.how == How::Adopted));
        check(&root).unwrap();
        // A second adoption deletes nothing.
        std::fs::create_dir_all(m.join("build/y")).unwrap();
        let again = adopt(&root).unwrap();
        assert!(!again.newly);
        assert!(again.deleted.is_empty());
        assert!(m.join("build/y").exists());
    }

    #[test]
    fn a_different_tree_at_the_same_path_is_refused_again() {
        testing::adoption_file();
        let root = tmp("swap");
        shipped(&root);
        adopt(&root).unwrap();
        check(&root).unwrap();
        // Another tree unpacked in its place, with its own token.
        std::fs::remove_dir_all(root.join("migration")).unwrap();
        shipped(&root);
        std::fs::write(
            root.join("migration").join(TOKEN_FILE),
            format!("{}\n", "0".repeat(32)),
        )
        .unwrap();
        assert!(matches!(check(&root), Err(Error::NotAdopted { .. })));
        // And with no token at all.
        std::fs::remove_file(root.join("migration").join(TOKEN_FILE)).unwrap();
        assert!(matches!(check(&root), Err(Error::NotAdopted { .. })));
        // Adopting a root that shipped a token writes a fresh one over it:
        // a later tree shipped with the same token is not trusted.
        let shipped_token = "1".repeat(32);
        std::fs::write(
            root.join("migration").join(TOKEN_FILE),
            format!("{shipped_token}\n"),
        )
        .unwrap();
        adopt(&root).unwrap();
        let fresh = read_token(&root.join("migration").join(TOKEN_FILE)).unwrap();
        assert_ne!(fresh, shipped_token);
        check(&root).unwrap();
        std::fs::remove_dir_all(root.join("migration")).unwrap();
        shipped(&root);
        std::fs::write(
            root.join("migration").join(TOKEN_FILE),
            format!("{shipped_token}\n"),
        )
        .unwrap();
        assert!(matches!(check(&root), Err(Error::NotAdopted { .. })));
    }

    /// The first command that makes a ledger also writes a fresh token over
    /// one a download shipped beside hand-written files.
    #[test]
    fn a_ledger_made_here_never_keeps_a_shipped_token() {
        testing::adoption_file();
        let root = tmp("made-here");
        let m = root.join("migration");
        std::fs::create_dir_all(m.join("tools/t-a")).unwrap();
        std::fs::write(m.join("tools/t-a/harness.toml"), "schema_version = 2\n").unwrap();
        let shipped_token = "2".repeat(32);
        std::fs::write(m.join(TOKEN_FILE), format!("{shipped_token}\n")).unwrap();
        record_created(&root, Scope::Project).unwrap();
        assert_ne!(read_token(&m.join(TOKEN_FILE)).unwrap(), shipped_token);
    }

    /// The map's `config.toml` that came with the project: its hash is
    /// recorded when the root is first recorded, stays "shipped" while the
    /// file keeps it, and the person's `--adopt` states it.
    #[test]
    fn a_config_toml_that_came_with_the_project_is_remembered_until_adopted() {
        testing::adoption_file();
        let root = tmp("shipped-config");
        let config = root.join("migration/map/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "[[configuration]]\nname = \"x\"\n").unwrap();
        let first = crate::hash::bytes_hash(&std::fs::read(&config).unwrap());
        // Not recorded yet: seen for the first time, it came with the project.
        assert_eq!(shipped_config_hash(&root), Some(first.clone()));
        // The first command that makes the ledger records that hash.
        record_created(&root, Scope::Project).unwrap();
        assert_eq!(shipped_config_hash(&root), Some(first.clone()));
        // The person's edit gives another hash: the map compares and finds
        // it is theirs (the recorded hash stays as it was).
        std::fs::write(&config, "[[configuration]]\nname = \"y\"\n").unwrap();
        assert_eq!(shipped_config_hash(&root), Some(first));
        // `--adopt` states it, even in a folder that holds no results.
        adopt(&root).unwrap();
        assert_eq!(shipped_config_hash(&root), None);

        // A root first recorded with no config.toml has none shipped.
        let plain = tmp("no-shipped-config");
        std::fs::create_dir_all(plain.join("migration")).unwrap();
        record_created(&plain, Scope::Project).unwrap();
        std::fs::create_dir_all(plain.join("migration/map")).unwrap();
        std::fs::write(plain.join("migration/map/config.toml"), "mine\n").unwrap();
        assert_eq!(shipped_config_hash(&plain), None);
    }

    /// A `migration/` holding only hand-written tool files and the map's
    /// configuration holds no results: no adoption question, nothing to
    /// adopt, and no "claims" line. Anything more is asked about.
    #[test]
    fn hand_written_tools_hold_no_results() {
        testing::adoption_file();
        let root = tmp("hand-written");
        let m = root.join("migration");
        std::fs::create_dir_all(m.join("tools/t-a")).unwrap();
        std::fs::create_dir_all(m.join("map")).unwrap();
        std::fs::write(m.join("tools/t-a/harness.toml"), "schema_version = 2\n").unwrap();
        std::fs::write(m.join("map/config.toml"), "").unwrap();
        std::fs::write(m.join(".DS_Store"), "x").unwrap();
        assert!(holds_no_results(&m) && !holds_results(&root));
        check(&root).unwrap();
        // Adopting such a folder (a copy: adopting states its config.toml,
        // which records the root) has nothing to adopt.
        let copy = tmp("hand-written-adopted");
        std::fs::create_dir_all(copy.join("migration/map")).unwrap();
        std::fs::write(copy.join("migration/map/config.toml"), "").unwrap();
        let done = adopt(&copy).unwrap();
        assert!(!done.newly && !done.had_ledger);
        assert!(!done
            .describe(None)
            .iter()
            .any(|l| l.contains("verified unit")));
        // An empty migration/ holds none either.
        let empty = tmp("empty-ledger");
        std::fs::create_dir_all(empty.join("migration")).unwrap();
        check(&empty).unwrap();
        // A tool's plan, the map file, or facts are results.
        for (path, body) in [
            ("tools/t-a/plan.toml", "schema_version = 1\n"),
            ("map/project-map.json", "{}"),
            ("facts.jsonl", ""),
        ] {
            std::fs::write(m.join(path), body).unwrap();
            assert!(holds_results(&root), "{path}");
            assert!(
                matches!(check(&root), Err(Error::NotAdopted { .. })),
                "{path}"
            );
            std::fs::remove_file(m.join(path)).unwrap();
        }
        check(&root).unwrap();
        // A link in place of a tool's file is no hand-written file.
        std::fs::remove_file(m.join("tools/t-a/harness.toml")).unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), m.join("tools/t-a/harness.toml"))
            .unwrap();
        assert!(holds_results(&root));
    }

    /// A `migration/` that is not the harness's is refused as the project's
    /// own before any adoption question.
    #[test]
    fn the_projects_own_migration_folder_is_named_before_adoption() {
        testing::adoption_file();
        let root = tmp("own-first");
        std::fs::create_dir_all(root.join("migration")).unwrap();
        std::fs::write(root.join("migration/001_init.sql"), "create table").unwrap();
        assert!(matches!(check(&root), Err(Error::ForeignMigration { .. })));
    }

    /// The adoption line speaks of deletions only when something was
    /// deleted, and of verified units only when some came with it; the
    /// commands it names carry the tool.
    #[test]
    fn the_adoption_line_says_only_what_is_true() {
        let quiet = Adoption {
            root: PathBuf::from("/p"),
            newly: true,
            had_ledger: true,
            deleted: Vec::new(),
            units: 4,
            verified: 0,
        };
        assert_eq!(
            quiet.describe(None),
            ["adopt: /p is now trusted on this computer (4 units, 0 verified)"]
        );
        let deleted = Adoption {
            deleted: vec![PathBuf::from("/p/migration/build")],
            ..quiet.clone()
        };
        assert_eq!(
            deleted.describe(None)[1],
            "adopt: deleted 1 build folder made elsewhere; the harness builds what it needs \
             again here"
        );
        assert_eq!(deleted.describe(None).len(), 2);
        let verified = Adoption {
            verified: 2,
            ..quiet.clone()
        };
        assert_eq!(
            verified.describe(Some("t-lzg")),
            [
                "adopt: /p is now trusted on this computer (4 units, 2 verified)".to_string(),
                "adopt: 2 verified units came with it, marked \"made elsewhere\" until you run \
                 `harness verify <unit> --tool t-lzg` here (`harness state status --tool t-lzg` \
                 lists them)"
                    .to_string(),
            ]
        );
    }

    /// An adoption records its time; a verdict file written before it is
    /// made elsewhere, one written after it (by `verify` here) is not, and
    /// a root made here has no adoption time at all.
    #[test]
    fn a_verdict_older_than_the_adoption_is_made_elsewhere() {
        testing::adoption_file();
        let root = tmp("made-elsewhere");
        shipped(&root);
        let verdict = root.join("migration/units/u1/oracle-latest.json");
        std::fs::create_dir_all(verdict.parent().unwrap()).unwrap();
        std::fs::write(&verdict, "{}").unwrap();
        assert_eq!(adopted_at(&root), None, "not adopted yet");
        std::thread::sleep(std::time::Duration::from_millis(20));
        adopt(&root).unwrap();
        assert!(adopted_at(&root).is_some());
        assert!(made_before_adoption(&root, &verdict));
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&verdict, "{}").unwrap();
        assert!(!made_before_adoption(&root, &verdict), "verified here");
        // A ledger made here: no time, nothing made elsewhere.
        let here = tmp("made-here");
        std::fs::create_dir_all(here.join("migration/units/u1")).unwrap();
        let old = here.join("migration/units/u1/oracle-latest.json");
        std::fs::write(&old, "{}").unwrap();
        std::fs::write(here.join("migration/facts.jsonl"), "").unwrap();
        record_created(&here, Scope::Project).unwrap();
        assert_eq!(adopted_at(&here), None);
        assert!(!made_before_adoption(&here, &old));
    }

    /// Adopting deletes each mapped tool's build folders too: its `build/`,
    /// its crates' `target/`, an attempt's `candidate/target/`, a
    /// `.promote-*/`.
    #[test]
    fn adopting_deletes_a_tools_build_folders() {
        testing::adoption_file();
        let root = tmp("tool-builds");
        shipped(&root);
        let tool = root.join("migration/tools/t-a");
        std::fs::create_dir_all(tool.join("build/obj")).unwrap();
        std::fs::write(tool.join("harness.toml"), "schema_version = 2\n").unwrap();
        let unit = tool.join("units/u9");
        std::fs::create_dir_all(unit.join("u9_rs/target/debug")).unwrap();
        std::fs::create_dir_all(unit.join("u9_rs/src")).unwrap();
        std::fs::create_dir_all(unit.join("attempts/a-2/candidate/target")).unwrap();
        std::fs::create_dir_all(unit.join(".promote-a-2")).unwrap();
        let done = adopt(&root).unwrap();
        assert!(done.newly);
        assert!(!tool.join("build").exists());
        assert!(!unit.join("u9_rs/target").exists());
        assert!(unit.join("u9_rs/src").exists());
        assert!(!unit.join("attempts/a-2/candidate/target").exists());
        assert!(!unit.join(".promote-a-2").exists());
        assert!(tool.join("harness.toml").exists());
        assert!(
            done.deleted.contains(&tool.join("build")),
            "{:?}",
            done.deleted
        );
    }

    /// Test processes racing on one fixture agree on one token.
    #[test]
    fn racing_helpers_agree_on_one_token() {
        let root = tmp("race-token");
        let path = root.join("migration").join(TOKEN_FILE);
        let tokens: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..16)
                .map(|_| s.spawn(|| ensure_token(&path).unwrap()))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(tokens.iter().all(|t| *t == tokens[0]), "{tokens:?}");
        assert_eq!(read_token(&path).as_ref(), Some(&tokens[0]));
        // Nothing is left beside it.
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["migration"]);
    }

    #[test]
    fn the_projects_own_migration_folder_is_refused_for_adoption() {
        testing::adoption_file();
        let root = tmp("own");
        shipped(&root);
        std::fs::write(root.join("migration/0001_users.sql"), "create table").unwrap();
        let err = adopt(&root).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{}/migration is the project's own folder, and the harness needs that name for \
                 its ledger: rename the folder, or run the harness on a copy of the project with \
                 it renamed",
                root.display()
            )
        );
        assert!(root.join("migration/build").exists(), "nothing deleted");
        // A folder with only fixed names but none of the harness's markers.
        let bare = tmp("bare");
        std::fs::create_dir_all(bare.join("migration/units")).unwrap();
        assert!(!is_harness_ledger(&bare.join("migration")));
        assert!(matches!(adopt(&bare), Err(Error::ForeignMigration { .. })));
    }

    #[test]
    fn finders_folder_file_is_ignored() {
        let root = tmp("dsstore");
        shipped(&root);
        std::fs::write(root.join("migration/.DS_Store"), "x").unwrap();
        assert!(is_harness_ledger(&root.join("migration")));
        std::fs::write(root.join("migration/other"), "x").unwrap();
        assert!(!is_harness_ledger(&root.join("migration")));
    }

    #[test]
    fn a_missing_roots_entry_is_dropped_on_the_next_write() {
        testing::adoption_file();
        let gone = tmp("gone");
        shipped(&gone);
        adopt(&gone).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        let other = tmp("other");
        shipped(&other);
        adopt(&other).unwrap();
        let file = read_adopted(testing::adoption_file()).unwrap();
        assert!(file.roots.iter().all(|e| e.path != gone));
        assert!(file.roots.iter().any(|e| e.path == other));
    }

    #[test]
    fn a_suite_adopted_once_covers_every_case() {
        testing::adoption_file();
        let suite = tmp("suite");
        for case in ["Public/a_lib", "Hidden/b_lib"] {
            shipped(&suite.join("cases").join(case));
        }
        assert!(matches!(
            check_suite(&suite),
            Err(Error::NotAdopted {
                units: 4,
                verified: 2,
                ..
            })
        ));
        assert!(check(&suite.join("cases/Public/a_lib")).is_err());
        let done = adopt_suite(&suite).unwrap();
        assert!(done.newly);
        assert!(suite.join(TOKEN_FILE).is_file());
        check_suite(&suite).unwrap();
        check(&suite.join("cases/Public/a_lib")).unwrap();
        check(&suite.join("cases/Hidden/b_lib")).unwrap();
        assert!(!suite.join("cases/Hidden/b_lib/migration/build").exists());
    }

    #[test]
    fn two_adopters_at_once_lose_no_entry() {
        testing::adoption_file();
        let roots: Vec<PathBuf> = (0..16)
            .map(|i| {
                let r = tmp(&format!("race{i}"));
                shipped(&r);
                r
            })
            .collect();
        std::thread::scope(|s| {
            for r in &roots {
                s.spawn(move || adopt(r).unwrap());
            }
        });
        for r in &roots {
            check(r).unwrap();
        }
    }
}
