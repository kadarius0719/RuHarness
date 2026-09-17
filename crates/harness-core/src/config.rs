//! Target configuration (`harness.toml`, docs/SCHEMAS.md) and the
//! [`TargetContext`] threaded through frontends, planners and oracles.

use crate::error::Error;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Version of the config schema this build understands.
pub const CONFIG_SCHEMA_VERSION: u64 = 1;

/// Parsed `harness.toml`. Unknown fields are tolerated and preserved on disk
/// (this struct is read-only; the file is never rewritten by the harness).
#[derive(Debug, Clone, Deserialize)]
pub struct TargetConfig {
    /// Schema version of the file.
    pub schema_version: u64,
    /// `[target]` section.
    pub target: TargetSection,
    /// `[oracle]` section: `allowlist` is core-owned; every other key belongs
    /// to the configured oracle kind and is handed over opaquely.
    #[serde(default)]
    pub oracle: toml::Table,
}

/// The `[target]` section of `harness.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetSection {
    /// Human label for the target.
    pub name: String,
    /// Directory of the source files, relative to the target root.
    pub source_dir: String,
}

impl TargetConfig {
    /// Load and validate `harness.toml` from a target root.
    pub fn load(target_root: &Path) -> Result<TargetConfig, Error> {
        let path = target_root.join("harness.toml");
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let config: TargetConfig =
            toml::from_str(&text).map_err(|e| Error::parse(&path, e.to_string()))?;
        if config.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path,
                found: config.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        Ok(config)
    }

    /// The oracle executable allowlist (core-owned key; defaults to empty).
    pub fn oracle_allowlist(&self) -> Vec<String> {
        self.oracle
            .get("allowlist")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A target root plus its parsed configuration — the context every trait
/// implementation receives.
#[derive(Debug, Clone)]
pub struct TargetContext {
    /// Absolute path of the target repository root.
    pub root: PathBuf,
    /// Parsed `harness.toml`.
    pub config: TargetConfig,
}

impl TargetContext {
    /// Build a context by loading `harness.toml` under `root`.
    pub fn load(root: impl Into<PathBuf>) -> Result<TargetContext, Error> {
        let root = root.into();
        let root = root.canonicalize().map_err(|e| Error::io(&root, e))?;
        let config = TargetConfig::load(&root)?;
        Ok(TargetContext { root, config })
    }
}
