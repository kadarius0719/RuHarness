//! `harness features init | save | map` (docs/FEATURES-DESIGN.md §5, §7).

use crate::{lock_ledger, out, report, require_sandbox, safe_ledger_dir, TargetArg};
use anyhow::{bail, Context, Result};
use harness_core::features::{self, FeatureSnapshot};
use harness_core::ledger::Ledger;
use harness_core::{Facts, TargetContext};
use std::path::Path;

/// `harness features init`: the starter, never over an existing file.
pub(crate) fn cmd_init(target: TargetArg) -> Result<u8> {
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "features init")?;
    let dir = safe_ledger_dir(&ctx, &[features::FEATURES_DIR])?;
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
pub(crate) fn cmd_save(target: TargetArg, expect: String, bytes: u64) -> Result<u8> {
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

    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "features save")?;
    let dir = safe_ledger_dir(&ctx, &[features::FEATURES_DIR])?;
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
pub(crate) fn cmd_map(target: TargetArg, allow_unsandboxed: bool) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness features map")?;
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "features map")?;
    let (features, digest) = match FeatureSnapshot::load(&ctx) {
        FeatureSnapshot::None => bail!(
            "there is no {}; write one first (`harness features init` gives a starter)",
            display(&ctx.root, &features::features_path(&Ledger::of(&ctx)))
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
            // Only when the program's link failed on main (check 7: a unity
            // build's other refusals are not; review: nor a refusal that
            // merely names main.c).
            if main_count(&ctx, &facts) != 1 && link_failed_on_main(&e.to_string()) {
                out("features: features need a program with one main()".into());
            }
            return Err(e.into());
        }
    };
    // Why functions have no note, one line per kind (§3.7): a count and one
    // example, the words shown as a person reads them.
    let mut kinds: std::collections::BTreeMap<&str, (usize, &features::UnwatchedReason)> =
        std::collections::BTreeMap::new();
    for r in &map.unwatched_reasons {
        kinds.entry(r.kind.as_str()).or_insert((0, r)).0 += 1;
    }
    // The example (a compiler's message may quote source text) only for a
    // person's terminal; events carry the counts (§3.7: reasons stay out of
    // events and prompts).
    let human = crate::report::mode() == crate::report::Mode::Human;
    for (kind, (count, example)) in &kinds {
        let plural = if *count == 1 { "" } else { "s" };
        out(if human {
            format!(
                "features: {count} function{plural} unwatched — {} (e.g. {}, {})",
                features::unwatched_words(kind, &example.detail),
                example.file,
                example.id,
            )
        } else {
            // The kind's words without the detail: a person reads these in
            // the cockpit, and they quote no source text.
            format!(
                "features: {count} function{plural} unwatched — {}",
                features::unwatched_words(kind, "")
            )
        });
    }
    let path = features::map_path(&Ledger::of(&ctx));
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

/// How many distinct public `main`s the facts record among the whole
/// program's own files — the top-level `.c` of `source_dir`, or every listed
/// file of a file-list target — advisory only: the link decides (§5.1).
fn main_count(ctx: &TargetContext, facts: &Facts) -> usize {
    let mut files: Vec<&str> = facts
        .symbols
        .iter()
        .filter(|s| s.name == "main")
        .filter(|s| ctx.config.target.is_program_file(&s.file))
        .map(|s| s.file.as_str())
        .collect();
    files.sort();
    files.dedup();
    files.len()
}

/// Whether a refusal is the plain build's link failing on `main` — none, or
/// more than one (ld64, GNU ld and lld spellings).
fn link_failed_on_main(refusal: &str) -> bool {
    // lld's spelling is unquoted: only the whole name (not `main_loop`).
    let lld = |needle: &str| {
        refusal.match_indices(needle).any(|(at, _)| {
            refusal[at + needle.len()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
    };
    refusal.contains("the C program does not build")
        && (["'_main'", "\"_main\"", "`main'", "'main'"]
            .iter()
            .any(|m| refusal.contains(m))
            || lld("symbol: main")
            || lld("symbol: _main"))
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The main() count reads the program's own files by the target's form:
    /// top-level `.c` of `source_dir`, or every listed file of a file list
    /// (wherever its folder) — never a file the list leaves out.
    #[test]
    fn mains_are_counted_among_the_programs_own_files() {
        let facts = Facts {
            symbols: ["src/tools/lzg.c", "src/other/demo.c", "src/lib/lib.c"]
                .iter()
                .map(|file| harness_core::facts::SymbolRecord {
                    name: if file.ends_with("lib.c") { "f" } else { "main" }.into(),
                    kind: "function".into(),
                    file: (*file).into(),
                    visibility: "public".into(),
                    signature: String::new(),
                    span: (1, 1),
                })
                .collect(),
            ..Facts::default()
        };
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-mains-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = |text: &str| {
            std::fs::write(dir.join("harness.toml"), text).unwrap();
            TargetContext::folder_form(
                dir.clone(),
                harness_core::config::TargetConfig::load(&dir).unwrap(),
            )
        };
        let listed = ctx("schema_version = 2\n[target]\nname = \"lzg\"\n\
             files = [{ path = \"src/tools/lzg.c\" }, { path = \"src/lib/lib.c\" }]\n\
             configuration = { name = \"make\", from = \"stated\", flags = [] }\n");
        assert_eq!(main_count(&listed, &facts), 1);
        let folder = ctx("schema_version = 1\n[target]\nname = \"x\"\nsource_dir = \"src\"\n");
        assert_eq!(main_count(&folder, &facts), 0);
        let folder =
            ctx("schema_version = 1\n[target]\nname = \"x\"\nsource_dir = \"src/other\"\n");
        assert_eq!(main_count(&folder, &facts), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review: the main() hint is for a link that failed on main, not for
    /// any refusal whose words name a file such as src/main.c.
    #[test]
    fn the_main_hint_is_for_a_link_that_failed_on_main() {
        for refusal in [
            "the C program does not build: duplicate symbol '_main' in: a.o b.o",
            "the C program does not build: Undefined symbols for architecture arm64:\n  \"_main\", referenced from:",
            "the C program does not build: /usr/bin/ld: b.o: multiple definition of `main'; a.o: first defined here",
            "the C program does not build: ld.lld: error: duplicate symbol: main",
            "the C program does not build: ld.lld: error: undefined symbol: main\n>>> referenced by crt1.o",
        ] {
            assert!(link_failed_on_main(refusal), "{refusal}");
        }
        for refusal in [
            "the scratch copy of src/main.c is not the same program near: int x;",
            "the C program does not build: src/main.c:3:1: error: expected ';'",
            // Review: lld's unquoted name, only whole.
            "the C program does not build: ld.lld: error: undefined symbol: main_loop\n>>> referenced by app.c",
            "the C program does not build: ld.lld: error: duplicate symbol: _main_window",
        ] {
            assert!(!link_failed_on_main(refusal), "{refusal}");
        }
    }
}
