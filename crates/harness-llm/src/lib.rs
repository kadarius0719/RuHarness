//! RuHarness LLM layer (M2): provider adapters behind
//! [`harness_core::traits::ProviderAdapter`] and the observer triage pass
//! (docs/SCHEMAS.md "Triage call contract").
//!
//! This crate owns everything that talks to a model: the live Anthropic
//! adapter, the trace-based replay/external adapter, prompt assembly under
//! the injection posture (nonce-delimited untrusted slices, `<` escaping,
//! no source-derived text in the trusted region), harness-computed content
//! hashes, and response validation. It never persists an API key, and it
//! only ever reads one from an `ANTHROPIC_*` variable (the name comes from
//! the target's `harness.toml`, which is untrusted).
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
pub mod emission;
pub mod migrate;
mod openai_compat;
pub mod providers;
pub mod triage;

pub use adapters::{AnthropicAdapter, TraceAdapter};
pub use emission::{deny_scan, parse_emission, EmissionResult};
pub use migrate::{run_migration, MigrateParams, MigrationOutcome};
pub use providers::{checked_complete, ProviderProfile, ResolvedProvider};
pub use triage::{run_triage, TriageOutcome};
