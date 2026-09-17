//! M0 end-to-end thread (briefing §5): the harness in embryo.
//!
//! `harness-m0 scan`   — parse the vendored zopfli C sources, build a
//!                       function-level call graph, rank leaf units.
//! `harness-m0 oracle` — build and run the differential oracle for unit
//!                       u001-katajainen (C vs Rust behind the same ABI).
//! `harness-m0 all`    — both (default).

#![forbid(unsafe_code)]

mod oracle;
mod scan;

use std::path::PathBuf;
use std::process::ExitCode;

/// Repo root, resolved from this crate's location at compile time (m0/..).
fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());
    let root = repo_root();
    let result = match cmd.as_str() {
        "scan" => scan::run(&root),
        "oracle" => oracle::run(&root),
        "all" => scan::run(&root).and_then(|()| oracle::run(&root)),
        other => Err(format!(
            "unknown command `{other}` (expected scan | oracle | all)"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
