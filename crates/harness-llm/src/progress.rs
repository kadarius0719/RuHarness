//! A courtesy stream of per-turn events for a live consumer
//! (docs/CLI-HARDENING.md §4): the CLI's `--json` mode installs a sink once
//! at startup; the trajectory reports every turn to it as it happens, with
//! the ledger's own values (the `Turn` it just journaled), never a
//! re-encoding. Nothing here is evidence — the ledger is the truth.

use harness_core::attempts::Turn;
use std::sync::OnceLock;

/// What a consumer learns as a trajectory runs. Every call carries the unit
/// and attempt ids, so a consumer following several runs in one process
/// (`bench check --replay`) needs no buffering.
pub trait Progress: Send + Sync {
    /// A turn is about to be posed: `index` is 1-based, `kind` the Turn's
    /// kind (`translate`/`driver`/`repair`), `request_key` HEAD's rendering
    /// of the request (a verified recorded turn's own key is in `turn_end`).
    fn turn_start(&self, unit: &str, attempt: &str, index: usize, kind: &str, request_key: &str);
    /// A turn ended and was journaled (or, verifying, reproduced): `turn` is
    /// exactly the record's entry.
    fn turn_end(&self, unit: &str, attempt: &str, index: usize, turn: &Turn);
}

struct NoProgress;

impl Progress for NoProgress {
    fn turn_start(&self, _: &str, _: &str, _: usize, _: &str, _: &str) {}
    fn turn_end(&self, _: &str, _: &str, _: usize, _: &Turn) {}
}

static SINK: OnceLock<Box<dyn Progress>> = OnceLock::new();

/// Install the process's progress sink. At most once: a second install is
/// refused and the sink handed back.
pub fn install(sink: Box<dyn Progress>) -> Result<(), Box<dyn Progress>> {
    SINK.set(sink)
}

/// The installed sink, or a silent one.
pub(crate) fn sink() -> &'static dyn Progress {
    static SILENT: NoProgress = NoProgress;
    match SINK.get() {
        Some(sink) => sink.as_ref(),
        None => &SILENT,
    }
}
