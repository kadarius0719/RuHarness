//! RuHarness LLM layer (M2): provider adapters behind
//! [`harness_core::traits::ProviderAdapter`] and the observer triage pass
//! (docs/SCHEMAS.md "Triage call contract").
//!
//! This crate owns everything that talks to a model: the live Anthropic
//! adapter, the trace-based replay/external adapter, prompt assembly under
//! the injection posture (nonce-delimited untrusted slices, `<` escaping,
//! no source-derived text in the trusted region), harness-computed content
//! hashes, and response validation. It never persists an API key. Which
//! endpoint and which key variable to use is never the target's decision:
//! the target's `harness.toml` (hostile input) only NAMES a provider
//! profile; endpoints and credentials come from built-in profiles or the
//! user-level profiles file (see [`providers`]).
//!
//! Every completion — triage and executor alike — is requested through
//! [`checked_complete`], which refuses a prompt that cannot fit the
//! profile's declared context window and rejects a reply whose reported
//! token count shows that the server truncated the prompt.
//!
//! # Trace keys and nonces (normative for this crate)
//!
//! - **Trace key** = first 8 lowercase hex of
//!   blake3(`serde_json::to_string(&CompletionRequest)`): the compact
//!   serialization in struct field order (`model`, `system`, `user`,
//!   `max_tokens`), no whitespace. Trace files are
//!   `<key>.request.json` / `<key>.response.json` (pretty-printed for
//!   humans; the pretty form is not what is hashed). Because every prompt
//!   byte participates, a stale trace can never match a changed request.
//!   A pair is recorded only after the reply validated, under the key of
//!   the ORIGINAL request — a live run's validated retry reply is stored
//!   under that same key, so replay never needs the retry.
//! - **Nonce** (untrusted-slice delimiter, `<c_source_<nonce> …>`) = first
//!   12 lowercase hex of blake3(batch key ‖ sorted finding ids joined `","`
//!   ‖ each slice's raw bytes in call order), concatenated without
//!   separators. It is a pure function of the request inputs (deterministic
//!   for replay) and depends on every slice byte, so a slice cannot contain
//!   its own delimiter without a hash fixed point.
//! - **Usage key** (per call, in [`TriageOutcome::usage`]) =
//!   `<owning unit or "shared">.<first 8 hex of blake3(sorted finding ids
//!   joined ",")>` — a batch identity for accounting, distinct from the
//!   trace key.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod adapters;
pub mod driver_gen;
pub mod emission;
pub mod migrate;
mod openai_compat;
pub mod progress;
pub mod providers;
mod trajectory;
pub mod triage;

pub use adapters::{AnthropicAdapter, TraceAdapter};
pub use driver_gen::{run_driver_generation, DriverOutcome};
pub use emission::{deny_scan, parse_emission, EmissionResult};
pub use migrate::{
    candidate_manifest, human_edit_hash, record_human_attempt, run_migration, validate_note,
    HumanEdit, MigrateParams, MigrationOutcome, SteerArgs, CANDIDATE_LIB_RS, MAX_HUMAN_NOTE_BYTES,
    MAX_STEER_NOTE_BYTES,
};
pub use progress::Progress;
pub use providers::{checked_complete, ProviderProfile, ResolvedProvider};
pub use triage::{run_triage, TriageOutcome};

/// `text` reduced to printable ASCII and cut to `max_bytes` — how
/// on-disk, target-owned strings are echoed in reports.
pub fn printable(text: &str, max_bytes: usize) -> String {
    trajectory::printable(text, max_bytes)
}

/// The conformance report of a verified attempt (docs/REPLAY-DESIGN.md §R
/// R-4): `conformant`, or `drifted (turns 1, 3)` — 1-based turns whose
/// request HEAD would render differently from the recorded one.
pub fn conformance(drifted: &[usize]) -> String {
    if drifted.is_empty() {
        "conformant".to_string()
    } else {
        let turns: Vec<String> = drifted.iter().map(|i| (i + 1).to_string()).collect();
        format!("drifted (turns {})", turns.join(", "))
    }
}
