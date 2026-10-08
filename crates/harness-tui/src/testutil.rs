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
    let dst = std::env::temp_dir().join(format!("harness-tui-{tag}-{:010}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy(&repo().join(rel), &dst);
    dst.canonicalize().unwrap()
}

/// Like [`scratch_target`], without the person's features
/// (`migration/features`): the target before `features init`.
pub fn scratch_target_without_features(rel: &str, tag: &str) -> PathBuf {
    let dst = scratch_target(rel, tag);
    match std::fs::remove_dir_all(dst.join("migration/features")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("{e}"),
        _ => dst,
    }
}

/// A scratch directory removed on drop — also when a test fails.
pub struct TmpDir(pub PathBuf);

impl TmpDir {
    /// A fresh one, unique per `tag`.
    pub fn new(tag: &str) -> TmpDir {
        let dir = std::env::temp_dir().join(format!("harness-tui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir.canonicalize().unwrap())
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A test's child that never ends by itself (a `while :` loop, a stopped
/// shell): SIGKILLed when the guard drops — when the test ends, passing or
/// failing — unless it is gone, or its pid now names another program (its
/// command name is checked first, so a reused pid is never killed).
pub struct KillOnDrop {
    pid: u32,
    comm: String,
}

impl KillOnDrop {
    /// Guard `pid`, read now as the program it is.
    pub fn new(pid: u32) -> KillOnDrop {
        KillOnDrop {
            pid,
            comm: comm_of(pid),
        }
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.comm.is_empty() || comm_of(self.pid) != self.comm {
            return;
        }
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", &self.pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// `pid`'s command name (`ps -o comm=`), empty when it is gone.
fn comm_of(pid: u32) -> String {
    std::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
