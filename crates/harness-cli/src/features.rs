//! `harness features init | save | map` (docs/FEATURES-DESIGN.md §5, §7).

use crate::{lock_ledger, out, report, require_sandbox, safe_ledger_dir};
use anyhow::{bail, Context, Result};
use harness_core::features::{self, FeatureSnapshot};
use harness_core::ledger::Ledger;
use harness_core::{Facts, TargetContext};
use std::path::{Path, PathBuf};

/// `harness features init`: the starter, never over an existing file.
pub(crate) fn cmd_init(target: PathBuf) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "features init")?;
    let dir = safe_ledger_dir(
        &ctx.root,
        &[harness_core::ledger::MIGRATION_DIR, features::FEATURES_DIR],
    )?;
    let path = dir.join(features::FEATURES_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!(
            "{} exists; `features init` never overwrites it",
            display(&ctx.root, &path)
        ),
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
    harness_core::ledger::write_atomic(&path, features::starter(&ctx.config).as_bytes())?;
    out(format!(
        "features: wrote a starter to {} — add your features, then run `harness features map`",
        display(&ctx.root, &path)
    ));
    Ok(0)
}

/// Largest text `features save` takes (the loader's cap).
const MAX_SAVE_BYTES: u64 = features::MAX_FEATURES_BYTES;

/// `harness features save`: the new text on stdin, `--bytes` long, saved
/// only when it validates and the file on disk is still the one `--expect`
/// names (its blake3, or `none`).
pub(crate) fn cmd_save(target: PathBuf, expect: String, bytes: u64) -> Result<u8> {
    use std::io::{IsTerminal, Read};
    if bytes > MAX_SAVE_BYTES {
        bail!("--bytes {bytes} is more than the {MAX_SAVE_BYTES} a features file may hold");
    }
    if std::io::stdin().is_terminal() {
        bail!("stdin is a terminal: pipe the new features file in");
    }
    let mut text = Vec::new();
    std::io::stdin()
        .lock()
        .take(bytes + 1)
        .read_to_end(&mut text)
        .context("reading the new features file from stdin")?;
    if text.len() as u64 != bytes {
        bail!(
            "{} bytes on stdin, not the {bytes} --bytes names: cut short or changed, not saved",
            text.len()
        );
    }
    let text = String::from_utf8(text).context("the features file must be UTF-8")?;
    if expect != "none" && !expect.starts_with(harness_core::hash::HASH_PREFIX) {
        bail!("--expect takes the blake3 of the file's current bytes, or `none`");
    }

    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "features save")?;
    let dir = safe_ledger_dir(
        &ctx.root,
        &[harness_core::ledger::MIGRATION_DIR, features::FEATURES_DIR],
    )?;
    let path = dir.join(features::FEATURES_FILE);
    let current = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "none".to_string(),
        Ok(m) if m.file_type().is_file() => harness_core::hash::bytes_hash(
            &harness_core::ledger::read_regular(&path, MAX_SAVE_BYTES + 1)?,
        ),
        Ok(_) => bail!(
            "{} is not a regular file; replace it with one outside the harness",
            display(&ctx.root, &path)
        ),
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", path.display())),
    };
    if current != expect {
        bail!(
            "{} changed since the edit started; nothing was saved",
            display(&ctx.root, &path)
        );
    }
    features::parse(&text, &path)?;
    harness_core::ledger::write_atomic(&path, text.as_bytes())?;
    out(format!("features: saved {}", display(&ctx.root, &path)));
    Ok(0)
}

/// `harness features map` (§5.1).
pub(crate) fn cmd_map(target: PathBuf, allow_unsandboxed: bool) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness features map")?;
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "features map")?;
    let (features, digest) = match FeatureSnapshot::load(&ctx) {
        FeatureSnapshot::None => bail!(
            "there is no {}; write one first (`harness features init` gives a starter)",
            display(&ctx.root, &features::features_path(&ctx.root))
        ),
        FeatureSnapshot::Invalid(why) => bail!("{why}"),
        FeatureSnapshot::Valid { features, digest } => (features, digest),
    };
    if features.scenarios.is_empty() {
        bail!("your features file has no scenario to map");
    }
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let stale = crate::stale_fact_files(&ctx, &facts);
    if stale > 0 {
        bail!(
            "{stale} scanned file(s) changed since the scan: scan the project first (the map's \
             function ids come from the facts)"
        );
    }
    // The facts describe the program as it is — the digest's own rule, so a
    // scan always lets the map run (fix check 2 N5).
    if features::program_digest_now(&ctx, &facts) == features::STALE_PROGRAM {
        bail!(
            "the program's C changed since the scan (a file added, changed or gone): scan the \
             project first"
        );
    }
    let mut progress = Progress;
    let map = match harness_oracle::map_features(&ctx, &facts, &features, &digest, &mut progress) {
        Ok(map) => map,
        Err(e) => {
            if main_count(&ctx, &facts) != 1 {
                out("features: features need a program with one main()".into());
            }
            return Err(e.into());
        }
    };
    let path = features::map_path(&ctx.root);
    harness_core::ledger::write_atomic(&path, &map.to_bytes()?)?;
    let look = map
        .scenarios
        .iter()
        .filter(|r| !r.stable || !r.probe_agrees || r.noted != "complete")
        .count();
    out(format!(
        "features: mapped {} scenario{} — wrote {}{}",
        map.scenarios.len(),
        if map.scenarios.len() == 1 { "" } else { "s" },
        display(&ctx.root, &path),
        match look {
            0 => String::new(),
            n => format!(" ({n} need a look)"),
        }
    ));
    Ok(0)
}

/// The progress of a map: human lines and `scenario` events (§5.5).
struct Progress;

impl harness_oracle::MapProgress for Progress {
    fn message(&mut self, text: &str) {
        out(format!("features: {text}"));
    }

    fn scenario(&mut self, record: &features::ScenarioRecord, n: usize, of: usize) {
        out(format!(
            "features: mapped {}/{} ({n} of {of}) — {}, {} function{}{}",
            record.feature,
            record.scenario,
            record.end,
            record.functions.len(),
            if record.functions.len() == 1 { "" } else { "s" },
            if !record.stable {
                " — its output differs between runs"
            } else if !record.probe_agrees {
                " — the run with notes behaved differently"
            } else if record.noted != "complete" {
                " — no notes were recorded"
            } else {
                ""
            }
        ));
        #[derive(serde::Serialize)]
        struct ScenarioEvent<'a> {
            k: &'static str,
            feature: &'a str,
            scenario: &'a str,
            n: usize,
            of: usize,
            end: &'a str,
            stable: bool,
            probe_agrees: bool,
            noted: &'a str,
            functions: usize,
        }
        report::event(&ScenarioEvent {
            k: "scenario",
            feature: &record.feature,
            scenario: &record.scenario,
            n,
            of,
            end: &record.end,
            stable: record.stable,
            probe_agrees: record.probe_agrees,
            noted: &record.noted,
            functions: record.functions.len(),
        });
    }
}

/// How many distinct public `main`s the facts record among the top-level
/// `.c` of `source_dir` — advisory only: the link decides (§5.1).
fn main_count(ctx: &TargetContext, facts: &Facts) -> usize {
    let mut files: Vec<&str> = facts
        .symbols
        .iter()
        .filter(|s| s.name == "main")
        .filter(|s| features::directly_in(&ctx.config.target.source_dir, &s.file))
        .map(|s| s.file.as_str())
        .collect();
    files.sort();
    files.dedup();
    files.len()
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
