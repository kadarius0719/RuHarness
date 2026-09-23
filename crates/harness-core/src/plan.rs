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
    /// The configured oracle kind, if any.
    pub fn oracle_kind(&self) -> Option<&str> {
        self.oracle.as_ref()?.get("kind")?.as_str()
    }

    /// A kind-owned oracle parameter as a string.
    pub fn oracle_param_str(&self, key: &str) -> Option<&str> {
        self.oracle.as_ref()?.get(key)?.as_str()
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
    /// `files`, `driver`, `replaces` as clean relative paths.
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
    /// Display signatures for `interface` (only written on first creation).
    pub interface: Vec<String>,
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
                // Planner-owned fields only; status/comments/unknowns untouched.
                let old_hash = t.get("source_hash").and_then(|v| v.as_str()).unwrap_or("");
                if old_hash != cu.source_hash {
                    changes.push(format!("unit {}: source changed (hash updated)", cu.id));
                }
                t["files"] = toml_edit::value(str_array(&cu.files));
                t["symbols"] = toml_edit::value(str_array(&cu.symbols));
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
                            .set_suffix(" # planner: files no longer present in facts");
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
