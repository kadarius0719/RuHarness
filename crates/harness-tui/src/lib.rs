//! The review cockpit (docs/TUI-DESIGN.md): a read-mostly view of a target's
//! migration ledger that spawns the `harness` CLI for every write.
//!
//! Without the `tui` feature the crate is what any client of the CLI needs
//! and nothing terminal-bound — the READ MODEL ([`model`], [`pairs`],
//! [`display`]), the `ruharness-events` reader ([`events`]), the read
//! preflight ([`preflight`]) and the child process a client spawns
//! ([`spawn`]) — so other clients (harness-mcp)
//! reuse it with `default-features = false`. The `tui` feature adds the
//! terminal front end (`highlight`, `view`, `app`) and the `harness-tui`
//! binary.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod display;
pub mod events;
pub mod load;
pub mod model;
pub mod pairs;
pub mod preflight;
pub mod spawn;

#[cfg(feature = "tui")]
pub mod app;
#[cfg(feature = "tui")]
pub mod handedit;
#[cfg(feature = "tui")]
pub mod highlight;
#[cfg(feature = "tui")]
pub mod termguard;
#[cfg(feature = "tui")]
pub mod view;

#[cfg(all(test, feature = "tui"))]
mod testutil;
