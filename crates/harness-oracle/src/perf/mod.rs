//! C-vs-Rust performance baselines (docs/PERF-DESIGN.md): builds as verify
//! builds them, the units' archives, and (later steps) the launcher and the
//! measurement. Information only: perf writes no verdict and no plan
//! status.

pub(crate) mod archive;
pub(crate) mod build;
pub(crate) mod launcher;
pub mod measure;
pub(crate) mod tools;
