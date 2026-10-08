//! C-vs-Rust performance baselines (docs/PERF-DESIGN.md): the person's
//! workloads file, the results files and their strict readers, and the
//! words — everything perf says is computed here, from numbers, on every
//! read. Information only: nothing here gates `verify`, `migrate`,
//! `promote` or `bench check`.

use std::path::PathBuf;

pub mod currency;
pub mod estimate;
pub mod results;
pub mod stats;
pub mod words;
pub mod workloads;

/// The launcher's version (§3.3): `perfrun` and `perfgo`'s sources are
/// pinned to it by a test beside them; a new version builds a new cache
/// folder.
pub const PERF_LAUNCHER: &str = "perf-launcher-2";

/// The command perf's writer lock records — a lock holder whose command
/// starts with it is a perf run (§3.11: the cockpit hashes no input then).
pub const PERF_RUN_LOCK: &str = "perf run";

/// How a row was measured (§3.9 `inputs.recipe`): a change to the method
/// makes every earlier row out of date. Recipe 2: a short run is one side
/// under both legs of the floor, not either (§3.5 step 2).
pub const PERF_RECIPE: &str = "perf-recipe-2";

/// Directory of perf's files, inside the ledger dir.
pub const PERF_DIR: &str = "perf";

/// `perf/` in the ledger (`migration/perf/` for a folder-form target).
pub fn perf_dir(ledger: &crate::ledger::Ledger) -> PathBuf {
    ledger.dir().join(PERF_DIR)
}

/// The kept outputs of behaves-differently rows, in `migration/build/`.
pub const PERF_OUT_DIR: &str = ".perf-out";
/// A unit's kept outputs, under [`PERF_OUT_DIR`]: `units/<id>/`.
pub const KEPT_UNITS: &str = "units";
/// The program as it stands's (and the C alone's), under
/// [`PERF_OUT_DIR`].
pub const KEPT_PROGRAM: &str = "program";

/// Where a side's kept outputs are in the ledger:
/// `build/.perf-out/units/<id>/`, or `…/program/` without a unit. Each is
/// `<workload>.{c,other}.{stdout,stderr}`.
pub fn kept_outputs_dir(ledger: &crate::ledger::Ledger, unit: Option<&str>) -> PathBuf {
    let out = ledger.build_dir().join(PERF_OUT_DIR);
    match unit {
        Some(id) => out.join(KEPT_UNITS).join(id),
        None => out.join(KEPT_PROGRAM),
    }
}

/// What perf and the cockpit say about a unit whose Accept was interrupted
/// (§3.2; the cockpit's `Cause::PromotionInterrupted` says the same):
/// `attempt` is the `.promote-<id>/` marker's attempt id, or `legacy` for
/// a bare `.<crate>.prev`. The next writing command — `verify` among them
/// — finishes or undoes it by evidence.
pub fn accept_interrupted_words(attempt: &str, unit: &str) -> String {
    let what = if attempt == "legacy" {
        "an Accept".to_string()
    } else {
        format!("an Accept of {attempt}")
    };
    format!("{what} was interrupted — Re-check {unit} (or run harness verify {unit}) to finish or undo it")
}

/// A crate manifest's `[profile.release]` settings away from Cargo's
/// defaults — `opt-level`, `lto`, `codegen-units`, `panic` — as (key, value
/// as written), named on its rows (docs/PERF-DESIGN.md build note 25). A
/// value over 32 characters or holding a character unsafe to show is left
/// out; `.cargo/config.toml` is not read (a §6 residual).
pub fn manifest_profile(manifest: &str) -> Vec<(String, String)> {
    let Ok(doc) = manifest.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(release) = doc
        .get("profile")
        .and_then(|p| p.get("release"))
        .and_then(|r| r.as_table())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, default) in [
        ("opt-level", "3"),
        ("lto", "false"),
        ("codegen-units", "16"),
        ("panic", "unwind"),
    ] {
        if let Some(v) = release.get(key) {
            let shown = match v {
                toml::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let safe = shown.len() <= 32 && !shown.chars().any(crate::text::unsafe_to_show);
            if shown != default && safe {
                out.push((key.to_string(), shown));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifests_release_profile_away_from_the_defaults() {
        let m = "[package]\nname = \"u\"\n[profile.release]\nlto = \"thin\"\nopt-level = 3\n\
                 panic = \"abort\"\ncodegen-units = 1\n";
        assert_eq!(
            manifest_profile(m),
            vec![
                ("lto".to_string(), "thin".to_string()),
                ("codegen-units".to_string(), "1".to_string()),
                ("panic".to_string(), "abort".to_string()),
            ]
        );
        assert!(manifest_profile("[package]\nname = \"u\"\n").is_empty());
        assert!(manifest_profile("not toml [").is_empty());
    }

    #[test]
    fn the_interrupted_accept_words() {
        assert_eq!(
            accept_interrupted_words("a-1234", "u001"),
            "an Accept of a-1234 was interrupted — Re-check u001 (or run harness verify u001) \
             to finish or undo it"
        );
        assert!(accept_interrupted_words("legacy", "u001").starts_with("an Accept was"));
    }
}
