//! The review cockpit (docs/TUI-DESIGN.md): a read-mostly view of a target's
//! migration ledger that spawns the `harness` CLI for every write.
//!
//! The library half is the READ MODEL — [`model`], [`pairs`], [`display`] —
//! with no terminal dependency, so other clients (harness-mcp) can reuse it
//! with `default-features = false`. The `tui` feature will add the terminal
//! front end and the binary (docs/TUI-DESIGN.md §8 step 4, not built yet).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod display;
pub mod model;
pub mod pairs;
