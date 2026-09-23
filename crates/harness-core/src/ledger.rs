//! Ledger layout: where migration state lives inside a target repo
//! (docs/SCHEMAS.md). All paths derive from `<target root>/migration/`.

use crate::error::Error;
use std::path::{Path, PathBuf};

/// Atomically replace `path` with `bytes`: write to a temp file in the same
/// directory, then rename over the target. An interrupted write can never
/// truncate or corrupt committed ledger evidence (docs/SCHEMAS.md).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::Invariant(format!("{} has no parent dir", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "ledger".into()),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

/// Directory name of the ledger inside a target repo.
pub const MIGRATION_DIR: &str = "migration";

/// Path helpers over a target root. Pure path arithmetic, no I/O.
#[derive(Debug, Clone)]
pub struct Ledger {
    root: PathBuf,
}

impl Ledger {
    /// A ledger rooted at the given target repository root.
    pub fn new(target_root: impl Into<PathBuf>) -> Ledger {
        Ledger {
            root: target_root.into(),
        }
    }

    /// `<root>/migration/`.
    pub fn dir(&self) -> PathBuf {
        self.root.join(MIGRATION_DIR)
    }

    /// The canonical committed fact file.
    pub fn facts_path(&self) -> PathBuf {
        self.dir().join("facts.jsonl")
    }

    /// The plan contract file.
    pub fn plan_path(&self) -> PathBuf {
        self.dir().join("plan.toml")
    }

    /// A unit's directory.
    pub fn unit_dir(&self, unit_id: &str) -> PathBuf {
        self.dir().join("units").join(unit_id)
    }

    /// A unit's latest verdict (green or red).
    pub fn verdict_latest_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-latest.json")
    }

    /// A unit's last green verdict (preserved across red re-runs).
    pub fn verdict_last_green_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-last-green.json")
    }

    /// The human rendering of the latest verdict.
    pub fn verdict_md_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-latest.md")
    }

    /// A unit's generated (or human-written) differential driver.
    pub fn driver_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("driver.c")
    }
    /// A unit's promoted driver self-validation record (M4).
    pub fn driver_validation_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("driver-validation.json")
    }
    /// Gitignored scratch/build directory for oracle runs.
    pub fn build_dir(&self) -> PathBuf {
        self.dir().join("build")
    }

    /// The target root this ledger belongs to.
    pub fn target_root(&self) -> &Path {
        &self.root
    }
}
