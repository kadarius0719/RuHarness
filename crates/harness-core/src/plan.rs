//! The plan contract (`migration/plan.toml`, docs/SCHEMAS.md): typed read,
//! topological execution order, and surgical mutation via `toml_edit` so
//! human comments and unknown fields survive every harness write.

use crate::error::Error;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Version of the plan schema this build understands.
pub const PLAN_SCHEMA_VERSION: u64 = 1;

/// Migration status of a unit. This enum is CLOSED (docs/SCHEMAS.md): adding
/// a value is a breaking schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnitStatus {
    /// Planned, not started.
    Pending,
    /// Translation attempted or underway; not currently proven green.
    InProgress,
    /// Latest oracle run on the recorded inputs was green.
    Verified,
    /// Verified and merged to the mainline.
    Merged,
    /// Cannot proceed (reason recorded in the unit's folder or a comment).
    Blocked,
}

impl UnitStatus {
    /// The kebab-case string used on disk.
    pub fn as_str(&self) -> &'static str {
        match self {
            UnitStatus::Pending => "pending",
            UnitStatus::InProgress => "in-progress",
            UnitStatus::Verified => "verified",
            UnitStatus::Merged => "merged",
            UnitStatus::Blocked => "blocked",
        }
    }
}

/// One `[[unit]]` entry. Unknown fields are tolerated (and preserved on disk
/// because mutation goes through `toml_edit`, never through this struct).
#[derive(Debug, Clone, Deserialize)]
pub struct Unit {
    /// Unit id (human slug, stable across replanning).
    pub id: String,
    /// Current status.
    pub status: UnitStatus,
    /// Source files this unit owns (repo-relative).
    #[serde(default)]
    pub files: Vec<String>,
    /// File-set hash of `files` plus their transitive project includes, at
    /// planning time.
    #[serde(default)]
    pub source_hash: String,
    /// Canonical ids of the public symbols the unit owns.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Informational display signatures (source-language syntax). The
    /// authoritative contract is the unit's contract.md plus its oracle.
    #[serde(default)]
    pub interface: Vec<String>,
    /// Functions the scan found in the source that this build does not
    /// compile (a function under an `#if` the build leaves out): kept out of
    /// `symbols`, so no driver is asked to call them.
    #[serde(default)]
    pub not_built: Vec<String>,
    /// Function names the build defines that the scan does not know and no
    /// one prefix explains (a build flag that renames them some other way):
    /// when this is non-empty, nothing was left out of `symbols`, since the
    /// build and the scan do not clearly agree, and no question about the
    /// unit or a unit calling it is posed ([`crate::deps::unfollowed_names`]).
    #[serde(default)]
    pub build_only: Vec<String>,
    /// The functions this build compiles under another name than the
    /// source's (a build flag that puts a prefix in front of each, lz4's
    /// `-DXXH_NAMESPACE=LZ4_`): source name to built name, for the renamed
    /// ones only. `symbols` keep the source's names; every place a name
    /// meets the linker asks [`Unit::link_name`].
    #[serde(default)]
    pub link_names: BTreeMap<String, String>,
    /// Unit ids this unit's code depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Prose test strategy.
    #[serde(default)]
    pub test_strategy: String,
    /// Prose done-criteria.
    #[serde(default)]
    pub done_criteria: String,
    /// `[unit.oracle]`: `kind` is core-owned; all other keys belong to the
    /// named oracle kind and are handed over opaquely.
    #[serde(default)]
    pub oracle: Option<toml::Table>,
}

impl Unit {
    /// What the function the source calls `source` is called at the link:
    /// its built name when this build renames it (`link_names`), else
    /// `source` itself.
    pub fn link_name<'a>(&'a self, source: &'a str) -> &'a str {
        self.link_names.get(source).map_or(source, String::as_str)
    }

    /// The unit's `symbols` as the linker and the compiled objects see
    /// them ([`Unit::link_name`] of each), in the plan's order.
    pub fn link_symbols(&self) -> Vec<String> {
        self.symbols
            .iter()
            .map(|s| self.link_name(s).to_string())
            .collect()
    }

    /// The configured oracle kind, if any.
    pub fn oracle_kind(&self) -> Option<&str> {
        self.oracle.as_ref()?.get("kind")?.as_str()
    }

    /// A kind-owned oracle parameter as a string.
    pub fn oracle_param_str(&self, key: &str) -> Option<&str> {
        self.oracle.as_ref()?.get(key)?.as_str()
    }

    /// A kind-owned oracle parameter as a boolean (`None` when absent or not
    /// a boolean — the kind decides whether that is an error).
    pub fn oracle_param_bool(&self, key: &str) -> Option<bool> {
        self.oracle.as_ref()?.get(key)?.as_bool()
    }

    /// A kind-owned oracle parameter as a list of strings.
    pub fn oracle_param_list(&self, key: &str) -> Vec<String> {
        self.oracle
            .as_ref()
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The parsed plan.
#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    /// Schema version of the file.
    pub schema_version: u64,
    /// Human label of the target.
    #[serde(default)]
    pub target: String,
    /// Units, in file (advisory) order.
    #[serde(default, rename = "unit")]
    pub units: Vec<Unit>,
}

impl Plan {
    /// Load and validate plan.toml.
    pub fn load(path: &Path) -> Result<Plan, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Plan::parse(path, &text)
    }

    /// Parse plan content (version-checked); `path` is for error messages.
    pub fn parse(path: &Path, text: &str) -> Result<Plan, Error> {
        let plan: Plan = toml::from_str(text).map_err(|e| Error::parse(path, e.to_string()))?;
        if plan.schema_version > PLAN_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: plan.schema_version,
                supported: PLAN_SCHEMA_VERSION,
            });
        }
        plan.validate_paths()?;
        Ok(plan)
    }

    /// plan.toml is target-owned, hostile input (docs/SCHEMAS.md M3 trust
    /// boundaries): every field that becomes a path component is validated
    /// here, once, at load — `id` and `rust_crate` as single clean segments;
    /// `files`, `driver`, `replaces` as clean relative paths; and
    /// `link_names`, whose names reach prompts, as pairs of C names.
    fn validate_paths(&self) -> Result<(), Error> {
        for u in &self.units {
            if !is_clean_segment(&u.id) {
                return Err(Error::InvalidPlan(format!(
                    "unit id {:?} is not a clean path segment (expected ^[A-Za-z0-9][A-Za-z0-9._-]*$)",
                    u.id
                )));
            }
            for f in &u.files {
                if !is_clean_relative_path(f) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}`: file {:?} is not a clean relative path",
                        u.id, f
                    )));
                }
            }
            if let Some(c) = u.oracle_param_str("rust_crate") {
                if !is_clean_segment(c) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}`: rust_crate {:?} is not a clean path segment",
                        u.id, c
                    )));
                }
            }
            if let Some(d) = u.oracle_param_str("driver") {
                if !is_clean_relative_path(d) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}`: driver {:?} is not a clean relative path",
                        u.id, d
                    )));
                }
            }
            for (source, built) in &u.link_names {
                if !is_c_identifier(source) || !is_c_identifier(built) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}`: link_names entry {:?} = {:?} is not a pair of C names",
                        u.id, source, built
                    )));
                }
            }
            for r in u.oracle_param_list("replaces") {
                if !is_clean_relative_path(&r) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}`: replaces entry {:?} is not a clean relative path",
                        u.id, r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Find a unit by id.
    pub fn unit(&self, id: &str) -> Result<&Unit, Error> {
        self.units
            .iter()
            .find(|u| u.id == id)
            .ok_or_else(|| Error::UnknownUnit(id.to_string()))
    }

    /// Topologically derived execution order (docs/SCHEMAS.md: block order is
    /// advisory). Deterministic Kahn's algorithm with unit-id tiebreak.
    /// Hard-fails on a `depends_on` reference to a missing unit or a cycle.
    pub fn execution_order(&self) -> Result<Vec<&Unit>, Error> {
        let ids: BTreeSet<&str> = self.units.iter().map(|u| u.id.as_str()).collect();
        if ids.len() != self.units.len() {
            return Err(Error::InvalidPlan("duplicate unit ids".into()));
        }
        let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for u in &self.units {
            indegree.entry(u.id.as_str()).or_insert(0);
            for dep in &u.depends_on {
                if !ids.contains(dep.as_str()) {
                    return Err(Error::InvalidPlan(format!(
                        "unit `{}` depends on unknown unit `{dep}`",
                        u.id
                    )));
                }
                *indegree.entry(u.id.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .insert(u.id.as_str());
            }
        }
        let mut ready: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut order: Vec<&Unit> = Vec::with_capacity(self.units.len());
        while let Some(&id) = ready.iter().next() {
            ready.remove(id);
            order.push(self.unit(id)?);
            if let Some(deps) = dependents.get(id) {
                for &d in deps {
                    let e = indegree.get_mut(d).ok_or_else(|| {
                        Error::InvalidPlan(format!("internal: missing indegree for `{d}`"))
                    })?;
                    *e -= 1;
                    if *e == 0 {
                        ready.insert(d);
                    }
                }
            }
        }
        if order.len() != self.units.len() {
            let stuck: Vec<&str> = self
                .units
                .iter()
                .map(|u| u.id.as_str())
                .filter(|id| !order.iter().any(|u| u.id == *id))
                .collect();
            return Err(Error::InvalidPlan(format!(
                "dependency cycle among units: {}",
                stuck.join(", ")
            )));
        }
        Ok(order)
    }
}

/// The prefix every entry of a `link_names` map puts in front of its source
/// name, when they all put the same one; `None` for an empty map or any
/// other shape (a hand-edited plan).
pub fn link_prefix(link_names: &BTreeMap<String, String>) -> Option<&str> {
    let mut prefix: Option<&str> = None;
    for (source, built) in link_names {
        let p = built
            .strip_suffix(source.as_str())
            .filter(|p| !p.is_empty())?;
        match prefix {
            Some(q) if q != p => return None,
            _ => prefix = Some(p),
        }
    }
    prefix
}

/// The plain words for a rename, `LZ4_ in front of its 19 function names
/// (XXH32 is built as LZ4_XXH32)`: `of` is how many functions the unit has,
/// so a partial rename reads `in front of 3 of its 5 function names`. The
/// example is the first renamed name. `None` when the map has no one prefix
/// ([`link_prefix`]).
pub fn rename_words(link_names: &BTreeMap<String, String>, of: usize) -> Option<String> {
    let prefix = link_prefix(link_names)?;
    let (source, built) = link_names.iter().next()?;
    let n = link_names.len();
    let count = match (n == of, n) {
        (true, 1) => "its one function name".to_string(),
        (true, _) => format!("its {n} function names"),
        (false, _) => format!("{n} of its {of} function names"),
    };
    Some(format!(
        "{prefix} in front of {count} ({source} is built as {built})"
    ))
}

/// A C signature with the declared function's name replaced: the first
/// `source` that stands as a whole identifier right before a `(` (blanks
/// between allowed) becomes `built`. A signature that is the bare name (the
/// facts held none) becomes `built`; any other is returned as it was.
pub fn link_signature(signature: &str, source: &str, built: &str) -> String {
    if source.is_empty() {
        return signature.to_string();
    }
    if signature.trim() == source {
        return built.to_string();
    }
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(at) = signature[from..].find(source).map(|i| i + from) {
        let end = at + source.len();
        let before_ok = signature[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !ident(c));
        let after_ok = signature[end..].trim_start().starts_with('(');
        if before_ok && after_ok {
            return format!("{}{built}{}", &signature[..at], &signature[end..]);
        }
        from = end;
    }
    signature.to_string()
}

/// A C identifier of at most 255 bytes: what a `link_names` entry must be
/// on both sides (plan.toml is hostile input; the names reach prompts).
fn is_c_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.len() <= 255
}

/// A single clean path segment: `^[A-Za-z0-9][A-Za-z0-9._-]*$` (so no
/// separators, no `..`, no leading dot, no control characters).
pub fn is_clean_segment(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && s != ".."
}

/// A clean relative path: non-empty, `/`-separated clean components (each a
/// [`is_clean_segment`] — which also rules out `.`/`..`, leading dots,
/// backslashes, rooted paths, and control characters).
pub fn is_clean_relative_path(p: &str) -> bool {
    !p.is_empty() && p.split('/').all(is_clean_segment)
}

/// A planner-derived unit proposal, fed into [`reconcile`].
#[derive(Debug, Clone)]
pub struct ComputedUnit {
    /// Proposed id (existing id when the planner matched an existing unit).
    pub id: String,
    /// Source files of the cluster.
    pub files: Vec<String>,
    /// Canonical ids of owned public symbols.
    pub symbols: Vec<String>,
    /// Display signatures for `interface` (rewritten with `symbols` on every
    /// reconcile: the two lists describe the same functions).
    pub interface: Vec<String>,
    /// Functions in the source this build does not compile, left out of
    /// `symbols` and `interface` ([`crate::planner::keep_built`]).
    pub not_built: Vec<String>,
    /// Function names the build defines that the scan does not know, sorted;
    /// when non-empty, nothing was left out.
    pub build_only: Vec<String>,
    /// The functions the build compiles under a prefixed name, source name
    /// to built name ([`crate::planner::keep_built`]'s rename rule).
    pub link_names: BTreeMap<String, String>,
    /// Unit ids this cluster depends on.
    pub depends_on: Vec<String>,
    /// File-set hash of files + transitive includes.
    pub source_hash: String,
}

fn str_array(values: &[String]) -> toml_edit::Value {
    let mut arr = toml_edit::Array::new();
    for v in values {
        arr.push(v.as_str());
    }
    arr.into()
}

/// Map planner-proposed unit ids onto existing plan ids by file overlap, and
/// rewrite `depends_on` accordingly. Run before [`reconcile`] so that a
/// re-plan updates existing units in place instead of duplicating them under
/// fresh ids.
///
/// Each existing id is claimed by at most one computed cluster (first, in
/// computed order, wins) — when a previously-merged cycle unit splits into
/// several clusters, the remaining clusters keep their fresh planner ids
/// instead of collapsing onto the same table entry. Self-dependencies that
/// a rename would create are dropped.
pub fn adopt_existing_ids(computed: &mut [ComputedUnit], existing: &Plan) {
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for cu in computed.iter_mut() {
        if let Some(existing_unit) = existing.units.iter().find(|u| {
            !claimed.contains(&u.id)
                && (u.id == cu.id || u.files.iter().any(|f| cu.files.contains(f)))
        }) {
            claimed.insert(existing_unit.id.clone());
            if existing_unit.id != cu.id {
                rename.insert(cu.id.clone(), existing_unit.id.clone());
                cu.id = existing_unit.id.clone();
            }
        }
    }
    for cu in computed.iter_mut() {
        for dep in cu.depends_on.iter_mut() {
            if let Some(new) = rename.get(dep) {
                *dep = new.clone();
            }
        }
        let own = cu.id.clone();
        cu.depends_on.retain(|d| *d != own);
        cu.depends_on.dedup();
    }
}
/// Write a unit's `not_built` and `build_only` (what the build said about
/// its functions, [`crate::planner::keep_built`]): each is written when
/// non-empty and its key removed when empty, so a plan with neither stays
/// as it was. A value that differs from the plan's gets a change line, and
/// so does a `not_built` that goes back to empty (its functions are in
/// `symbols` again).
fn build_fields(t: &mut toml_edit::Table, cu: &ComputedUnit, changes: &mut Vec<String>) {
    let had = |t: &toml_edit::Table, key: &str| -> Vec<String> {
        t.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    for (key, now) in [("not_built", &cu.not_built), ("build_only", &cu.build_only)] {
        let before = had(t, key);
        if now.is_empty() {
            // A function that was left out is in the piece again: said, so
            // its return to `symbols` is never silent.
            if key == "not_built" && !before.is_empty() {
                changes.push(format!(
                    "unit {}: no longer leaves out {}",
                    cu.id,
                    before.join(", ")
                ));
            }
            t.remove(key);
            continue;
        }
        if before != *now {
            changes.push(if key == "not_built" {
                format!(
                    "unit {}: left out {} — in the source, not compiled in this build",
                    cu.id,
                    now.join(", ")
                )
            } else {
                format!(
                    "unit {}: the build's function names differ from the source's ({} the scan \
                     does not know) — nothing left out",
                    cu.id,
                    now.len()
                )
            });
        }
        t[key] = toml_edit::value(str_array(now));
    }
    link_names_field(t, cu, changes);
}

/// Write a unit's `link_names` (the functions the build compiles under a
/// prefixed name): written when non-empty, the key removed when empty, so a
/// plan without a rename stays byte for byte as it was. A rename recorded
/// or changed gets a change line in words, and so does one that goes away.
fn link_names_field(t: &mut toml_edit::Table, cu: &ComputedUnit, changes: &mut Vec<String>) {
    let before: BTreeMap<String, String> = t
        .get("link_names")
        .and_then(|v| v.as_table_like())
        .map(|tbl| {
            tbl.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.to_string(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if cu.link_names.is_empty() {
        if !before.is_empty() {
            changes.push(format!(
                "unit {}: the build no longer renames its functions",
                cu.id
            ));
        }
        t.remove("link_names");
        return;
    }
    if before != cu.link_names {
        let words = rename_words(&cu.link_names, cu.symbols.len()).map_or_else(
            || format!("gives {} of its functions other names", cu.link_names.len()),
            |w| format!("puts {w}"),
        );
        changes.push(format!("unit {}: this build {words} — recorded", cu.id));
    }
    let mut tbl = toml_edit::InlineTable::new();
    for (source, built) in &cu.link_names {
        tbl.insert(source, built.as_str().into());
    }
    t["link_names"] = toml_edit::value(tbl);
}

/// The comment pass 2 of [`reconcile`] leaves on a `status = "blocked"` it
/// set itself; pass 1 reads it to know the block is the planner's to lift.
const PLANNER_BLOCK_NOTE: &str = "planner: files no longer present in facts";

/// Reconcile plan.toml with planner output (docs/SCHEMAS.md): keyed by unit
/// id (or file overlap for renamed clusters); existing units keep `status`,
/// comments, and every field the planner doesn't own; new units append as
/// `pending`; units no longer computed are marked `blocked`. Creates the file
/// if absent. Returns a human-readable change summary.
pub fn reconcile(
    path: &Path,
    target_name: &str,
    computed: &[ComputedUnit],
) -> Result<Vec<String>, Error> {
    let existing_text = if path.exists() {
        Some(std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?)
    } else {
        None
    };
    let (content, changes) =
        reconcile_to_string(path, existing_text.as_deref(), target_name, computed)?;
    crate::ledger::write_atomic(path, content.as_bytes())?;
    Ok(changes)
}

/// [`reconcile`] without the filesystem: takes the current file text (if
/// any), returns the new content and change summary. Callers that must
/// validate the result before persisting (the CLI does — docs/SCHEMAS.md)
/// use this and write only after validation. `path` is for error messages.
pub fn reconcile_to_string(
    path: &Path,
    existing_text: Option<&str>,
    target_name: &str,
    computed: &[ComputedUnit],
) -> Result<(String, Vec<String>), Error> {
    {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for cu in computed {
            if !seen.insert(cu.id.as_str()) {
                return Err(Error::InvalidPlan(format!(
                    "planner produced duplicate unit id `{}` (a split cluster \
                     collided with an existing unit — resolve ids before reconciling)",
                    cu.id
                )));
            }
        }
    }
    let mut doc: toml_edit::DocumentMut = if let Some(text) = existing_text {
        let doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e| Error::parse(path, format!("{e}")))?;
        let version = doc
            .get("schema_version")
            .and_then(|v| v.as_integer())
            .unwrap_or(1) as u64;
        if version > PLAN_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: version,
                supported: PLAN_SCHEMA_VERSION,
            });
        }
        doc
    } else {
        let mut doc = toml_edit::DocumentMut::new();
        doc["schema_version"] = toml_edit::value(PLAN_SCHEMA_VERSION as i64);
        doc["target"] = toml_edit::value(target_name);
        doc
    };

    if doc.get("unit").is_none() {
        doc["unit"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let mut changes: Vec<String> = Vec::new();

    // Pass 1: update existing units / append new ones.
    for cu in computed {
        let units = doc["unit"]
            .as_array_of_tables_mut()
            .ok_or_else(|| Error::parse(path, "`unit` is not an array of tables"))?;
        let existing = units
            .iter_mut()
            .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(cu.id.as_str()));
        match existing {
            Some(t) => {
                // Planner-owned fields only; status/comments/unknowns untouched
                // — except a block the planner itself set (pass 2's comment
                // marks it): computed again, the unit is pending again. The
                // lz4 run of 2026-11-08: a scanner fix made xxhash's functions
                // public again, and the piece would have stayed blocked for
                // good.
                let planner_blocked = t.get("status").is_some_and(|v| {
                    v.as_str() == Some("blocked")
                        && v.as_value().is_some_and(|val| {
                            val.decor().suffix().is_some_and(|d| {
                                d.as_str().is_some_and(|d| d.contains(PLANNER_BLOCK_NOTE))
                            })
                        })
                });
                if planner_blocked {
                    t["status"] = toml_edit::value("pending");
                    changes.push(format!(
                        "unit {}: pending again (its files are back in the facts)",
                        cu.id
                    ));
                }
                let old_hash = t.get("source_hash").and_then(|v| v.as_str()).unwrap_or("");
                if old_hash != cu.source_hash {
                    changes.push(format!("unit {}: source changed (hash updated)", cu.id));
                }
                t["files"] = toml_edit::value(str_array(&cu.files));
                t["symbols"] = toml_edit::value(str_array(&cu.symbols));
                // The signatures travel with the symbols: the driver
                // contract shows `interface` as the unit's external
                // functions, so a refresh that dropped a symbol (a helper
                // the scanner now knows is static, the lz4 run of
                // 2026-11-08) must drop its signature too.
                t["interface"] = toml_edit::value(str_array(&cu.interface));
                build_fields(t, cu, &mut changes);
                t["depends_on"] = toml_edit::value(str_array(&cu.depends_on));
                t["source_hash"] = toml_edit::value(cu.source_hash.as_str());
            }
            None => {
                let mut t = toml_edit::Table::new();
                t["id"] = toml_edit::value(cu.id.as_str());
                t["status"] = toml_edit::value("pending");
                t["files"] = toml_edit::value(str_array(&cu.files));
                t["source_hash"] = toml_edit::value(cu.source_hash.as_str());
                t["symbols"] = toml_edit::value(str_array(&cu.symbols));
                t["interface"] = toml_edit::value(str_array(&cu.interface));
                build_fields(&mut t, cu, &mut changes);
                t["depends_on"] = toml_edit::value(str_array(&cu.depends_on));
                t["test_strategy"] = toml_edit::value("");
                t["done_criteria"] = toml_edit::value("");
                changes.push(format!("unit {}: added (pending)", cu.id));
                let units = doc["unit"]
                    .as_array_of_tables_mut()
                    .ok_or_else(|| Error::parse(path, "`unit` is not an array of tables"))?;
                units.push(t);
            }
        }
    }

    // Pass 2: block units the planner no longer computes.
    {
        let units = doc["unit"]
            .as_array_of_tables_mut()
            .ok_or_else(|| Error::parse(path, "`unit` is not an array of tables"))?;
        for t in units.iter_mut() {
            let id = t
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let still = computed.iter().any(|cu| cu.id == id);
            if !still && t.get("status").and_then(|v| v.as_str()) != Some("blocked") {
                t["status"] = toml_edit::value("blocked");
                if let Some(v) = t.get_mut("status") {
                    if let Some(item) = v.as_value_mut() {
                        item.decor_mut()
                            .set_suffix(format!(" # {PLANNER_BLOCK_NOTE}"));
                    }
                }
                changes.push(format!("unit {id}: blocked (files no longer in facts)"));
            }
        }
    }

    Ok((doc.to_string(), changes))
}

/// Set a unit's status (the `harness verify` writer path). Surgical edit;
/// everything else in the file is preserved.
pub fn set_status(path: &Path, unit_id: &str, status: UnitStatus) -> Result<(), Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| Error::parse(path, format!("{e}")))?;
    let units = doc
        .get_mut("unit")
        .and_then(|i| i.as_array_of_tables_mut())
        .ok_or_else(|| Error::parse(path, "`unit` is not an array of tables"))?;
    let t = units
        .iter_mut()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(unit_id))
        .ok_or_else(|| Error::UnknownUnit(unit_id.to_string()))?;
    t["status"] = toml_edit::value(status.as_str());
    crate::ledger::write_atomic(path, doc.to_string().as_bytes())
}

/// The `[unit.oracle]` writer used by `harness bench init` (docs/SCHEMAS.md
/// "M4 additions" writer table): when the unit has NO `oracle` table, create
/// one with `entries` (string or string-list values, in order) and return
/// `true`; an existing table is never touched (`false`). Surgical edit via
/// `toml_edit`; everything else in the file is preserved.
pub fn set_oracle_table_if_absent(
    path: &Path,
    unit_id: &str,
    entries: &[(&str, OracleValue)],
) -> Result<bool, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| Error::parse(path, format!("{e}")))?;
    let units = doc
        .get_mut("unit")
        .and_then(|i| i.as_array_of_tables_mut())
        .ok_or_else(|| Error::parse(path, "`unit` is not an array of tables"))?;
    let t = units
        .iter_mut()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(unit_id))
        .ok_or_else(|| Error::UnknownUnit(unit_id.to_string()))?;
    if t.contains_key("oracle") {
        return Ok(false);
    }
    let mut table = toml_edit::Table::new();
    for (key, value) in entries {
        let item = match value {
            OracleValue::Str(s) => toml_edit::value(s.as_str()),
            OracleValue::List(items) => {
                let mut array = toml_edit::Array::new();
                for s in items {
                    array.push(s.as_str());
                }
                toml_edit::value(array)
            }
        };
        table.insert(key, item);
    }
    t.insert("oracle", toml_edit::Item::Table(table));
    crate::ledger::write_atomic(path, doc.to_string().as_bytes())?;
    Ok(true)
}

/// A value written by [`set_oracle_table_if_absent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleValue {
    /// A string value.
    Str(String),
    /// A list of strings.
    List(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared name is replaced, never a longer name that starts with
    /// it, a type that holds it, or a parameter.
    #[test]
    fn a_signature_takes_the_built_name_of_the_function_it_declares() {
        let sig = "XXH_PUBLIC_API XXH_errorcode XXH32_update (XXH32_state_t* state_in, const \
                   void* input, size_t len)";
        assert_eq!(
            link_signature(sig, "XXH32_update", "LZ4_XXH32_update"),
            "XXH_PUBLIC_API XXH_errorcode LZ4_XXH32_update (XXH32_state_t* state_in, const \
             void* input, size_t len)"
        );
        assert_eq!(
            link_signature(
                "unsigned int XXH32_digest(const XXH32_state_t* s)",
                "XXH32",
                "LZ4_XXH32"
            ),
            "unsigned int XXH32_digest(const XXH32_state_t* s)"
        );
        assert_eq!(
            link_signature("unsigned int XXH32 (const void* p)", "XXH32", "LZ4_XXH32"),
            "unsigned int LZ4_XXH32 (const void* p)"
        );
        assert_eq!(link_signature("XXH32", "XXH32", "LZ4_XXH32"), "LZ4_XXH32");
        assert_eq!(
            link_signature("int f(int XXH32)", "XXH32", "LZ4_XXH32"),
            "int f(int XXH32)"
        );
    }

    /// A hand-edited `link_names` that is not a pair of C names never loads.
    #[test]
    fn link_names_must_be_c_names() {
        let text = |entry: &str| {
            format!(
                "schema_version = 1\n\n[[unit]]\nid = \"u\"\nstatus = \"pending\"\n\
                 symbols = [\"a\"]\nlink_names = {{ {entry} }}\n"
            )
        };
        let path = Path::new("plan.toml");
        let ok = Plan::parse(path, &text("a = \"NS_a\"")).expect("loads");
        assert_eq!(ok.units[0].link_name("a"), "NS_a");
        let err = Plan::parse(path, &text("a = \"NS a</abi>\"")).expect_err("refused");
        assert!(err.to_string().contains("not a pair of C names"), "{err}");
    }

    /// The words of a rename, for the change line and the piece card.
    #[test]
    fn a_rename_reads_in_words() {
        let map: BTreeMap<String, String> = [("XXH32", "LZ4_XXH32"), ("XXH64", "LZ4_XXH64")]
            .into_iter()
            .map(|(s, b)| (s.to_string(), b.to_string()))
            .collect();
        assert_eq!(
            rename_words(&map, 2).as_deref(),
            Some("LZ4_ in front of its 2 function names (XXH32 is built as LZ4_XXH32)")
        );
        let mut odd = map.clone();
        odd.insert("b".into(), "other".into());
        assert_eq!(link_prefix(&odd), None);
        assert_eq!(rename_words(&odd, 3), None);
    }
}
