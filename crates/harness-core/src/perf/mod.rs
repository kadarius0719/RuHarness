//! C-vs-Rust performance baselines (docs/PERF-DESIGN.md): the person's
//! workloads file, the results files and their strict readers, and the
//! words — everything perf says is computed here, from numbers, on every
//! read. Information only: nothing here gates `verify`, `migrate`,
//! `promote` or `bench check`.

use std::path::{Path, PathBuf};

pub mod results;
pub mod stats;
pub mod workloads;

/// The launcher's version (§3.3): `perfrun` and `perfgo`'s sources are
/// pinned to it by a test beside them; a new version builds a new cache
/// folder.
pub const PERF_LAUNCHER: &str = "perf-launcher-1";

/// The command perf's writer lock records — a lock holder whose command
/// starts with it is a perf run (§3.11: the cockpit hashes no input then).
pub const PERF_RUN_LOCK: &str = "perf run";

/// How a row was measured (§3.9 `inputs.recipe`): a change to the method
/// makes every earlier row out of date.
pub const PERF_RECIPE: &str = "perf-recipe-1";

/// Directory of perf's files, inside the ledger dir.
pub const PERF_DIR: &str = "perf";

/// `migration/perf/` under `root`.
pub fn perf_dir(root: &Path) -> PathBuf {
    root.join(crate::ledger::MIGRATION_DIR).join(PERF_DIR)
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

#[cfg(test)]
mod tests {
    use super::*;

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
