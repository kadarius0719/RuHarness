//! The pieces a piece calls, as the program stands
//! (docs/BEYOND-LEAVES-DESIGN.md §1, §4): which units a unit depends on,
//! transitively, in the plan's execution order; whether each one is linked
//! as its accepted Rust or as its C; and the digest a verdict records for
//! it, so that a changed callee makes its callers' verdicts out of date.
//!
//! Pure filesystem reads and hashing — never a subprocess — so `state
//! status`, the cockpit and the oracle compute the same list. The oracle
//! (`harness_oracle::deps`) builds the Rust ones and falls back to their C
//! when a crate does not build.

use crate::config::TargetContext;
use crate::error::Error;
use crate::facts::Facts;
use crate::hash;
use crate::ledger::Ledger;
use crate::plan::{Plan, Unit, UnitStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The digest recorded for an input that cannot be read: never equal to a
/// real digest, so it always reads as changed.
pub const UNREADABLE: &str = "blake3:unreadable";

/// How a dependency was linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyKind {
    /// Its C files (`replaces`, else its `.c` files).
    C,
    /// Its accepted Rust: the staticlib of its unit crate.
    Rust,
}

/// One piece a unit calls, as a build linked it: the verdict's and the
/// driver-validation record's `dependencies` entries (docs/SCHEMAS.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The unit id of the piece.
    pub unit: String,
    /// `c` or `rust`.
    pub kind: DependencyKind,
    /// `c`: the file-set hash of its C files and what they include (as a
    /// verdict's `unit_source`); `rust`: its unit crate's digest (as a
    /// verdict's `rust_crate`) — sources, not the staticlib's bytes, so the
    /// record is the same on every machine and `state status` can compare it
    /// without building anything.
    pub digest: String,
}

/// Which form of each callee a build links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    /// The program as it stands: an accepted callee's Rust, else its C
    /// (verify's builds).
    AsItStands,
    /// Every callee's C (driver validation, which holds a test program to
    /// the ORIGINAL C program).
    OriginalC,
}

/// Every unit `unit` depends on, transitively, in the plan's execution
/// order (leaves first). `unit` itself is never in the list. Empty — and
/// the plan never read — when `unit.depends_on` is empty. A `depends_on`
/// naming a unit the plan does not hold is an invalid plan.
pub fn transitive<'p>(plan: &'p Plan, unit: &Unit) -> Result<Vec<&'p Unit>, Error> {
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = unit.depends_on.iter().map(String::as_str).collect();
    while let Some(id) = stack.pop() {
        if id == unit.id || !wanted.insert(id) {
            continue;
        }
        let dep = plan.unit(id).map_err(|_| {
            Error::InvalidPlan(format!("unit `{}` depends on unknown unit `{id}`", unit.id))
        })?;
        stack.extend(dep.depends_on.iter().map(String::as_str));
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    Ok(plan
        .execution_order()?
        .into_iter()
        .filter(|u| wanted.contains(u.id.as_str()))
        .collect())
}

/// The plan `unit`'s dependencies are read from: `None` for a unit that
/// depends on nothing (the plan is then never read).
pub fn plan_for(ledger: &Ledger, unit: &Unit) -> Result<Option<Plan>, Error> {
    if unit.depends_on.is_empty() {
        return Ok(None);
    }
    Plan::load(&ledger.plan_path()).map(Some)
}

/// The C files linked for `dep` when its C is linked: its `replaces`
/// entries, else the `.c` files it owns (a pending unit with no
/// `[unit.oracle]` yet).
pub fn c_files(dep: &Unit) -> Vec<String> {
    let replaces = dep.oracle_param_list("replaces");
    if !replaces.is_empty() {
        return replaces;
    }
    dep.files
        .iter()
        .filter(|f| f.ends_with(".c"))
        .cloned()
        .collect()
}

/// `dep`'s accepted Rust: its unit crate folder when its status is
/// `verified` or `merged` and the folder exists. Whether the crate builds
/// is the oracle's to find out.
pub fn accepted_crate(ledger: &Ledger, dep: &Unit) -> Option<PathBuf> {
    if !matches!(dep.status, UnitStatus::Verified | UnitStatus::Merged) {
        return None;
    }
    let name = dep.oracle_param_str("rust_crate")?;
    let dir = ledger.unit_dir(&dep.id).join(name);
    dir.is_dir().then_some(dir)
}

/// The `c` digest of `dep`: the file-set hash of its C files and their
/// include closure, [`UNREADABLE`] when a file cannot be read.
pub fn c_digest(ctx: &TargetContext, facts: &Facts, dep: &Unit) -> String {
    let closure = crate::sources::unit_closure(ctx, facts, &c_files(dep));
    hash::file_set_hash_on_disk(&ctx.root, &closure).unwrap_or_else(|_| UNREADABLE.into())
}

/// The `rust` digest of a crate folder ([`hash::unit_crate_file_set_hash`]),
/// [`UNREADABLE`] when it cannot be read.
pub fn rust_digest(ctx: &TargetContext, crate_dir: &std::path::Path) -> String {
    hash::unit_crate_file_set_hash(&ctx.root, crate_dir).unwrap_or_else(|_| UNREADABLE.into())
}

/// The dependencies of `unit` as `stand` links them today, from the files
/// on disk: what a build made now would record (assuming every accepted
/// crate builds).
pub fn now(
    ctx: &TargetContext,
    facts: &Facts,
    plan: &Plan,
    unit: &Unit,
    stand: Stand,
) -> Result<Vec<Dependency>, Error> {
    let ledger = Ledger::of(ctx);
    Ok(transitive(plan, unit)?
        .into_iter()
        .map(|dep| match (stand, accepted_crate(&ledger, dep)) {
            (Stand::AsItStands, Some(dir)) => Dependency {
                unit: dep.id.clone(),
                kind: DependencyKind::Rust,
                digest: rust_digest(ctx, &dir),
            },
            _ => Dependency {
                unit: dep.id.clone(),
                kind: DependencyKind::C,
                digest: c_digest(ctx, facts, dep),
            },
        })
        .collect())
}

/// [`now`] for a unit whose plan may not be readable: `None` when it cannot
/// be computed (a missing or broken plan, an unknown dependency) — a
/// recorded list then never reads as current.
pub fn now_or_none(
    ctx: &TargetContext,
    facts: &Facts,
    unit: &Unit,
    stand: Stand,
) -> Option<Vec<Dependency>> {
    match plan_for(&Ledger::of(ctx), unit) {
        Ok(None) => Some(Vec::new()),
        Ok(Some(plan)) => now(ctx, facts, &plan, unit, stand).ok(),
        Err(_) => None,
    }
}

/// The symbols of every unit `unit` depends on, transitively: what its Rust
/// may declare in an `extern "C"` block and import (docs/
/// BEYOND-LEAVES-DESIGN.md §3). Sorted, without duplicates.
pub fn symbols(plan: &Plan, unit: &Unit) -> Result<Vec<String>, Error> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for dep in transitive(plan, unit)? {
        out.extend(dep.symbols.iter().cloned());
    }
    Ok(out.into_iter().collect())
}

impl Unit {
    /// The unit defines the program's `main` (its plan symbols name it):
    /// such a piece stays C in this version (docs/BEYOND-LEAVES-DESIGN.md
    /// §1) — its test program would have to replace `main`.
    pub fn holds_main(&self) -> bool {
        self.symbols.iter().any(|s| s == "main")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str) -> Plan {
        Plan::parse(std::path::Path::new("plan.toml"), text).expect("plan parses")
    }

    fn unit_block(id: &str, deps: &[&str]) -> String {
        let deps: Vec<String> = deps.iter().map(|d| format!("{d:?}")).collect();
        format!(
            "[[unit]]\nid = \"{id}\"\nstatus = \"pending\"\nfiles = [\"src/{id}.c\"]\n\
             symbols = [\"{id}_fn\"]\ndepends_on = [{}]\n\n",
            deps.join(", ")
        )
    }

    fn ids(units: &[&Unit]) -> Vec<String> {
        units.iter().map(|u| u.id.clone()).collect()
    }

    #[test]
    fn a_chain_is_transitive_in_execution_order() {
        // c calls b calls a; listed out of order in the file.
        let p = plan(&format!(
            "schema_version = 1\n\n{}{}{}",
            unit_block("c", &["b"]),
            unit_block("a", &[]),
            unit_block("b", &["a"]),
        ));
        let c = p.unit("c").expect("c");
        assert_eq!(ids(&transitive(&p, c).expect("deps")), ["a", "b"]);
        let b = p.unit("b").expect("b");
        assert_eq!(ids(&transitive(&p, b).expect("deps")), ["a"]);
        let a = p.unit("a").expect("a");
        assert!(transitive(&p, a).expect("deps").is_empty());
        assert_eq!(symbols(&p, c).expect("symbols"), ["a_fn", "b_fn"]);
    }

    #[test]
    fn a_diamond_lists_each_piece_once() {
        // top calls left and right; both call base.
        let p = plan(&format!(
            "schema_version = 1\n\n{}{}{}{}",
            unit_block("top", &["left", "right"]),
            unit_block("right", &["base"]),
            unit_block("left", &["base"]),
            unit_block("base", &[]),
        ));
        let top = p.unit("top").expect("top");
        assert_eq!(
            ids(&transitive(&p, top).expect("deps")),
            ["base", "left", "right"]
        );
    }

    #[test]
    fn an_unknown_dependency_is_an_invalid_plan() {
        let p = plan(&format!("schema_version = 1\n\n{}", unit_block("a", &[])));
        let mut stray = p.unit("a").expect("a").clone();
        stray.id = "z".into();
        stray.depends_on = vec!["nowhere".into()];
        let err = transitive(&p, &stray).expect_err("refused");
        assert!(err.to_string().contains("unknown unit `nowhere`"), "{err}");
    }

    #[test]
    fn c_files_are_the_replaces_else_the_units_c() {
        let p = plan(
            "schema_version = 1\n\n[[unit]]\nid = \"a\"\nstatus = \"pending\"\n\
             files = [\"src/a.c\", \"src/a.h\"]\n\n[[unit]]\nid = \"b\"\nstatus = \"pending\"\n\
             files = [\"src/b.c\"]\n\n[unit.oracle]\nkind = \"c-abi-differential\"\n\
             replaces = [\"src/b_impl.c\"]\n",
        );
        assert_eq!(c_files(p.unit("a").expect("a")), ["src/a.c"]);
        assert_eq!(c_files(p.unit("b").expect("b")), ["src/b_impl.c"]);
    }

    #[test]
    fn the_piece_holding_main_is_known_by_its_symbols() {
        let p = plan(
            "schema_version = 1\n\n[[unit]]\nid = \"tool\"\nstatus = \"pending\"\n\
             symbols = [\"usage\", \"main\"]\n\n[[unit]]\nid = \"lib\"\nstatus = \"pending\"\n\
             symbols = [\"mainly\"]\n",
        );
        assert!(p.unit("tool").expect("tool").holds_main());
        assert!(!p.unit("lib").expect("lib").holds_main());
    }

    #[test]
    fn dependency_records_serialize_in_lowercase() {
        let d = Dependency {
            unit: "u-a".into(),
            kind: DependencyKind::Rust,
            digest: "blake3:00".into(),
        };
        assert_eq!(
            serde_json::to_string(&d).expect("json"),
            r#"{"unit":"u-a","kind":"rust","digest":"blake3:00"}"#
        );
    }
}
