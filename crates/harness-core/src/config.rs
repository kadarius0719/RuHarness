//! Target configuration (`harness.toml`, docs/SCHEMAS.md) and the
//! [`TargetContext`] threaded through frontends, planners and oracles.

use crate::error::Error;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Version of the config schema this build understands.
pub const CONFIG_SCHEMA_VERSION: u64 = 1;
/// Upper bound on any target-configured response token budget.
pub const MAX_TOKENS_LIMIT: u64 = 65_536;
/// Upper bound on target-configured repair turns per attempt.
pub const MAX_REPAIRS_LIMIT: u64 = 10;

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
    /// `[llm]` section (docs/SCHEMAS.md M2 additions).
    #[serde(default)]
    pub llm: LlmSection,
}

/// The `[llm]` section of `harness.toml`. Target config is hostile input: it
/// may only NAME a provider profile and a model — endpoints and credentials
/// live in user-level provider profiles (docs/SCHEMAS.md M3 additions).
#[derive(Debug, Clone, Deserialize)]
pub struct LlmSection {
    /// Provider profile name (`external | replay | anthropic` built in, or a
    /// user-defined profile).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Model identifier (Tier-2 default per briefing §16).
    #[serde(default = "default_model")]
    pub model: String,
    /// Response token budget.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Optional `[llm.migrate]` stage override (§13.2 per-stage routing).
    #[serde(default)]
    pub migrate: Option<MigrateSection>,
}

/// `[llm.migrate]`: executor-stage routing and loop bounds.
#[derive(Debug, Clone, Deserialize)]
pub struct MigrateSection {
    /// Provider profile for the executor (falls back to `[llm] provider`).
    #[serde(default)]
    pub provider: Option<String>,
    /// Model for the executor (falls back to `[llm] model`).
    #[serde(default)]
    pub model: Option<String>,
    /// Response token budget (falls back to `[llm] max_tokens`).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Stateless repair turns after the translate turn (default 3).
    #[serde(default)]
    pub max_repairs: Option<u32>,
}

impl Default for LlmSection {
    fn default() -> Self {
        LlmSection {
            provider: default_provider(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            migrate: None,
        }
    }
}

fn default_provider() -> String {
    "external".into()
}
fn default_model() -> String {
    "claude-sonnet-5".into()
}
fn default_max_tokens() -> u32 {
    8192
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
        // harness.toml is target-owned, hostile input: it must not be able to
        // turn one command into an unbounded stream of billable calls.
        let mut budgets: Vec<(&str, u64, u64)> = vec![(
            "[llm] max_tokens",
            u64::from(config.llm.max_tokens),
            MAX_TOKENS_LIMIT,
        )];
        if let Some(m) = &config.llm.migrate {
            if let Some(t) = m.max_tokens {
                budgets.push(("[llm.migrate] max_tokens", u64::from(t), MAX_TOKENS_LIMIT));
            }
            if let Some(r) = m.max_repairs {
                budgets.push(("[llm.migrate] max_repairs", u64::from(r), MAX_REPAIRS_LIMIT));
            }
        }
        for (key, value, limit) in budgets {
            if value > limit {
                return Err(Error::parse(
                    &path,
                    format!("{key} = {value} exceeds the harness limit of {limit}"),
                ));
            }
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
