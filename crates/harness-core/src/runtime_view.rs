//! Runtime-facing generated view (§14.3, docs/SCHEMAS.md CLI additions):
//! a managed block in the target's AGENTS.md. The ledger is the only source
//! of truth; this is a derived view — drift is resolved by regeneration.

use crate::error::Error;
use crate::plan::Plan;
use crate::risk::UnitRisk;

const BEGIN_PREFIX: &str = "<!-- BEGIN RUHARNESS GENERATED v1";
const END_PREFIX: &str = "<!-- END RUHARNESS GENERATED";

/// Render the managed block body (without markers). Deterministic, ≤60 lines,
/// only non-derivable ledger state (per the M2 spike's context-file evidence).
pub fn render_block_body(target_name: &str, plan: &Plan, risk: &[UnitRisk]) -> String {
    let mut b = String::new();
    b.push_str(&format!("## RuHarness migration state — {target_name}\n\n"));
    let verified = plan
        .units
        .iter()
        .filter(|u| {
            matches!(
                u.status,
                crate::plan::UnitStatus::Verified | crate::plan::UnitStatus::Merged
            )
        })
        .count();
    b.push_str(&format!(
        "{} of {} units verified/merged. The ledger under `migration/` is the source of truth — never hand-edit generated files; drive everything through `harness`.\n\n",
        verified,
        plan.units.len()
    ));
    b.push_str("Next units (execution order · risk 0-100):\n");
    if let Ok(order) = plan.execution_order() {
        for unit in order
            .iter()
            .filter(|u| {
                matches!(
                    u.status,
                    crate::plan::UnitStatus::Pending | crate::plan::UnitStatus::InProgress
                )
            })
            .take(5)
        {
            let score = risk
                .iter()
                .find(|r| r.unit == unit.id)
                .map(|r| r.score.to_string())
                .unwrap_or_else(|| "?".into());
            b.push_str(&format!(
                "- `{}` [{}] risk {}\n",
                unit.id,
                unit.status.as_str(),
                score
            ));
        }
    }
    b.push_str(
        "\nCommands: `harness scan` · `harness plan` · `harness detect` · `harness observe` · `harness verify <unit>` · `harness state status` · `harness review <finding>` (all take `--target`).\n",
    );
    b.push_str("Read first: `migration/plan.toml`, `migration/observer/observations.md`, `migration/DECISIONS.md`, `docs/SCHEMAS.md` (harness repo).\n");
    b
}

/// Wrap a body in the managed markers (content-hash in the end marker).
pub fn wrap_block(body: &str) -> String {
    let hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    format!(
        "{BEGIN_PREFIX} (source: migration/ — do not edit; run `harness sync-runtime`) -->\n{body}{END_PREFIX} (content-hash: blake3:{}) -->\n",
        &hash[..16]
    )
}

/// Splice the managed block into existing AGENTS.md content (replaces an
/// existing block, else appends). Human prose outside the markers survives.
///
/// Refuses (never guesses a replacement range) when the file holds a BEGIN
/// marker without a well-formed END marker after it, or more than one BEGIN
/// marker — a corrupted block must be repaired by a human, not overwritten.
pub fn apply(existing: Option<&str>, block: &str) -> Result<String, Error> {
    let text = match existing {
        None => return Ok(format!("# Agent guide\n\n{block}")),
        Some(t) => t,
    };
    let begins = text.matches(BEGIN_PREFIX).count();
    if begins > 1 {
        return Err(Error::Invariant(
            "AGENTS.md contains more than one RUHARNESS GENERATED block; repair it by hand".into(),
        ));
    }
    match text.find(BEGIN_PREFIX) {
        None => {
            let mut out = text.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(block);
            Ok(out)
        }
        Some(start) => {
            let after_begin = &text[start..];
            let end_rel = after_begin.find(END_PREFIX).ok_or_else(|| {
                Error::Invariant(
                    "AGENTS.md has a BEGIN RUHARNESS GENERATED marker without its END marker; repair it by hand"
                        .into(),
                )
            })?;
            let close_rel = after_begin[end_rel..].find("-->\n").ok_or_else(|| {
                Error::Invariant(
                    "AGENTS.md END RUHARNESS GENERATED marker is not terminated by `-->`; repair it by hand"
                        .into(),
                )
            })?;
            let end = start + end_rel + close_rel + 4;
            Ok(format!("{}{}{}", &text[..start], block, &text[end..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_replaces_only_the_managed_block() {
        let block1 = wrap_block("state A\n");
        let with_prose = format!("# My notes\n\nhuman text\n\n{block1}\nmore human text\n");
        let block2 = wrap_block("state B\n");
        let updated = apply(Some(&with_prose), &block2).unwrap();
        assert!(updated.contains("human text"));
        assert!(updated.contains("more human text"));
        assert!(updated.contains("state B"));
        assert!(!updated.contains("state A"));
        // idempotent
        assert_eq!(apply(Some(&updated), &block2).unwrap(), updated);
    }

    #[test]
    fn apply_refuses_corrupted_markers_instead_of_eating_prose() {
        let block = wrap_block("state A\n");
        let corrupted = format!("intro\n\n{}\n## Human notes\nkeep me\n", block)
            .replace("END RUHARNESS GENERATED", "END RUHARNESS BROKEN");
        let err = apply(Some(&corrupted), &wrap_block("state B\n")).unwrap_err();
        assert!(err.to_string().contains("repair"), "{err}");
        let doubled = format!("{block}\n{block}");
        assert!(apply(Some(&doubled), &block).is_err());
    }
}
