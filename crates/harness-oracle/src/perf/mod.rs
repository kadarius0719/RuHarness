//! C-vs-Rust performance baselines (docs/PERF-DESIGN.md): builds as verify
//! builds them, the units' archives, and (later steps) the launcher and the
//! measurement. Information only: perf writes no verdict and no plan
//! status.

// Wired into the measurement in step (d) of docs/PERF-DESIGN.md §5; until
// then only the tests use these.
#![allow(dead_code)]

pub(crate) mod archive;
pub(crate) mod build;
pub(crate) mod launcher;
