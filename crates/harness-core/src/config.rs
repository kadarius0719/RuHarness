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
/// Allowed range of `[driver] max_mutants` (clamped from BELOW too: a hostile
/// target must not be able to make the mutation gate meaningless).
pub const MAX_MUTANTS_RANGE: (u32, u32) = (16, 64);
/// Default `[driver] max_mutants`.
pub const DEFAULT_MAX_MUTANTS: u32 = 24;
/// Allowed range of `[driver] min_kill_ratio`, in permille.
pub const MIN_KILL_PERMILLE_RANGE: (u32, u32) = (500, 1000);
/// Default `[driver] min_kill_ratio` in permille (0.6 — a judgment call,
/// recorded as uncalibrated in DECISIONS.md).
pub const DEFAULT_MIN_KILL_PERMILLE: u32 = 600;

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
    /// `[driver]` section: driver-generation validation policy (M4).
    #[serde(default)]
    pub driver: DriverSection,
}

/// `[driver]`: the self-validation policy for generated differential drivers
/// (docs/SCHEMAS.md "M4 additions"). Both keys are range-checked at load.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DriverSection {
    /// Mutants sampled per validation (default [`DEFAULT_MAX_MUTANTS`], range
    /// [`MAX_MUTANTS_RANGE`]).
    #[serde(default)]
    pub max_mutants: Option<u32>,
    /// Minimum kill ratio when at least 10 mutants compiled (default 0.6,
    /// range 0.5–1.0).
    #[serde(default)]
    pub min_kill_ratio: Option<f64>,
}

/// The effective, validated driver-validation policy. Recorded verbatim in
/// every `driver-validation.json` so the bar a driver cleared is evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct DriverPolicy {
    /// Mutants sampled per validation.
    pub max_mutants: u32,
    /// Minimum kill ratio in permille (integer: canonical bytes never depend
    /// on float formatting).
    pub min_kill_permille: u32,
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
    /// Optional `[llm.driver]` stage override for driver generation (M4);
    /// same keys and clamps as `[llm.migrate]`.
    #[serde(default)]
    pub driver: Option<MigrateSection>,
}

/// `[llm.migrate]` / `[llm.driver]`: stage routing and loop bounds.
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
            driver: None,
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
    /// Extra quoted-include search dirs (M4), relative to the target root;
    /// each must lie inside `source_dir`. Searched after the including file's
    /// own directory, in order.
    #[serde(default)]
    pub include_dirs: Vec<String>,
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
        let mut budgets: Vec<(String, u64, u64)> = vec![(
            "[llm] max_tokens".to_string(),
            u64::from(config.llm.max_tokens),
            MAX_TOKENS_LIMIT,
        )];
        for (stage, section) in [
            ("[llm.migrate]", &config.llm.migrate),
            ("[llm.driver]", &config.llm.driver),
        ] {
            let Some(m) = section else { continue };
            if let Some(t) = m.max_tokens {
                budgets.push((
                    format!("{stage} max_tokens"),
                    u64::from(t),
                    MAX_TOKENS_LIMIT,
                ));
            }
            if let Some(r) = m.max_repairs {
                budgets.push((
                    format!("{stage} max_repairs"),
                    u64::from(r),
                    MAX_REPAIRS_LIMIT,
                ));
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
        config.driver_policy().map_err(|m| Error::parse(&path, m))?;
        for dir in &config.target.include_dirs {
            let inside = dir
                .strip_prefix(config.target.source_dir.trim_end_matches('/'))
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
            if !crate::plan::is_clean_relative_path(dir) || !inside {
                return Err(Error::parse(
                    &path,
                    format!(
                        "[target] include_dirs entry {dir:?} must be a clean relative path \
                         inside source_dir {:?}",
                        config.target.source_dir
                    ),
                ));
            }
        }
        Ok(config)
    }

    /// The validated `[driver]` policy (defaults applied). `Err` names the
    /// offending key; [`TargetConfig::load`] refuses such a config.
    pub fn driver_policy(&self) -> Result<DriverPolicy, String> {
        let max_mutants = self.driver.max_mutants.unwrap_or(DEFAULT_MAX_MUTANTS);
        let (lo, hi) = MAX_MUTANTS_RANGE;
        if !(lo..=hi).contains(&max_mutants) {
            return Err(format!(
                "[driver] max_mutants = {max_mutants} is outside the allowed range {lo}..={hi}"
            ));
        }
        let min_kill_permille = match self.driver.min_kill_ratio {
            None => DEFAULT_MIN_KILL_PERMILLE,
            Some(r) if r.is_finite() && (0.0..=1.0).contains(&r) => (r * 1000.0).round() as u32,
            Some(r) => return Err(format!("[driver] min_kill_ratio = {r} is not in 0.0..=1.0")),
        };
        let (lo, hi) = MIN_KILL_PERMILLE_RANGE;
        if !(lo..=hi).contains(&min_kill_permille) {
            return Err(format!(
                "[driver] min_kill_ratio = {} is outside the allowed range 0.5..=1.0",
                f64::from(min_kill_permille) / 1000.0
            ));
        }
        Ok(DriverPolicy {
            max_mutants,
            min_kill_permille,
        })
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
