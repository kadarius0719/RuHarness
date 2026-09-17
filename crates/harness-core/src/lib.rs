//! RuHarness core: the language-neutral fact model, ledger schemas, plan
//! handling, content-bound verdicts, the deterministic planner, and the
//! extension traits.
//!
//! Everything here implements the normative spec in `docs/SCHEMAS.md` (v1).
//! This crate does no I/O beyond the filesystem and knows nothing about any
//! LLM provider or language frontend.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod error;
pub mod facts;
pub mod hash;
pub mod ledger;
pub mod observer;
pub mod plan;
pub mod planner;
pub mod risk;
pub mod runtime_view;
pub mod traits;
pub mod verdict;

pub use config::{TargetConfig, TargetContext};
pub use error::Error;
pub use facts::Facts;
pub use plan::{Plan, Unit, UnitStatus};
pub use verdict::Verdict;
