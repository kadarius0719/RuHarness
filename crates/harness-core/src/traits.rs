//! Extension seams (§13.1 of the briefing). Kept deliberately minimal —
//! adding a method later is easy; removing one is a breaking change.
//!
//! At M1 only [`LanguageFrontend`] and [`OracleStrategy`] exist. The planner
//! is a plain function ([`crate::planner::compute_units`]) until a second
//! strategy exists; `Detector` and `ProviderAdapter` are defined at the
//! milestones that implement them (M2/M3).

use crate::config::TargetContext;
use crate::error::Error;
use crate::facts::Facts;
use crate::plan::Unit;
use crate::verdict::Verdict;

/// Scans a target codebase into the language-neutral fact model.
pub trait LanguageFrontend {
    /// Frontend identifier recorded in the facts header (e.g. `c-tree-sitter`).
    fn name(&self) -> &'static str;
    /// Produce facts for the target. Deterministic: identical trees yield
    /// byte-identical canonical facts.
    fn scan(&self, target: &TargetContext) -> Result<Facts, Error>;
}

/// Verifies one migration unit and returns content-bound evidence.
pub trait OracleStrategy {
    /// The `[unit.oracle] kind` string this strategy handles.
    fn kind(&self) -> &'static str;
    /// Run the oracle for `unit`. Implementations own every `[unit.oracle]`
    /// key other than `kind`, and must compute verdict input digests from the
    /// tree they actually tested.
    fn verify(&self, target: &TargetContext, unit: &Unit) -> Result<Verdict, Error>;
}
