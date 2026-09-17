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

/// Flags hazards in the scanned source (M2, docs/SCHEMAS.md findings).
pub trait Detector {
    /// Detector-suite identifier recorded in the findings header.
    fn name(&self) -> &'static str;
    /// Produce findings. Deterministic: identical trees yield identical
    /// findings (canonical ids included). No plan-derived data.
    fn detect(
        &self,
        target: &TargetContext,
        facts: &Facts,
    ) -> Result<Vec<crate::observer::Finding>, Error>;
}

/// A plain LLM completion request (provider-agnostic, §13.1).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionRequest {
    /// Model identifier (provider-specific string from config).
    pub model: String,
    /// System prompt (trusted, harness-generated only).
    pub system: String,
    /// User content (may embed untrusted material per the injection posture).
    pub user: String,
    /// Response token budget.
    pub max_tokens: u32,
}

/// A completion response with usage accounting (§16.3).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionResponse {
    /// Concatenated text output.
    pub text: String,
    /// Input tokens billed.
    pub input_tokens: u64,
    /// Output tokens billed.
    pub output_tokens: u64,
    /// Provider stop reason (e.g. `end_turn`).
    pub stop_reason: String,
}

/// A model provider behind a plain completion interface (§13.1). Capability
/// metadata is deferred until a consumer exists (recorded in DECISIONS.md).
pub trait ProviderAdapter {
    /// Adapter identifier (e.g. `anthropic`, `replay`, `external`).
    fn name(&self) -> &'static str;
    /// Execute one completion.
    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, Error>;
}
