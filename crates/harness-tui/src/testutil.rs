//! Fixture targets for the front end's tests: committed ledgers copied to a
//! scratch dir (build products and the lock file left out), so a test can
//! write to its ledger and nothing it reads races another command.

use std::path::{Path, PathBuf};

/// The workspace root.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The tractor case with a verified unit, a superseded attempt and a
/// promoted one (`u-lib`, provenance `a-13c941dfff95`).
pub const READ_SCALEFACTORS: &str =
    "targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib";

fn copy(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == "build" || name == ".lock" {
            continue;
        }
        let (from, to) = (entry.path(), dst.join(&name));
        if from.is_dir() {
            copy(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// A scratch copy of the committed target at `rel`, unique per `tag`.
pub fn scratch_target(rel: &str, tag: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("harness-tui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy(&repo().join(rel), &dst);
    dst.canonicalize().unwrap()
}
