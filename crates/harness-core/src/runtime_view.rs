//! Runtime-facing generated view (§14.3, docs/SCHEMAS.md CLI additions):
//! managed blocks in the project's AGENTS.md — one for a folder-form
//! target, and one per mapped tool, marked with its id
//! (docs/PROJECT-MAP-DESIGN.md §3.7). The ledger is the only source of
//! truth; each block is a derived view — drift is resolved by regeneration.

use crate::error::Error;
use crate::plan::Plan;
use crate::risk::UnitRisk;

const BEGIN_PREFIX: &str = "<!-- BEGIN RUHARNESS GENERATED v1";
const END_PREFIX: &str = "<!-- END RUHARNESS GENERATED";

/// Which target a block describes: `tool` is a mapped tool's id (`None` for
/// a folder-form target), `ledger` its ledger folder relative to the
/// project root (`migration` or `migration/tools/<id>`).
#[derive(Debug, Clone, Copy)]
pub struct BlockTarget<'a> {
    /// The mapped tool's id, when the target is one.
    pub tool: Option<&'a str>,
    /// The ledger folder, root-relative, `/`-separated, no trailing `/`.
    pub ledger: &'a str,
}

impl BlockTarget<'_> {
    /// A folder-form target: ledger `migration`.
    pub const FOLDER: BlockTarget<'static> = BlockTarget {
        tool: None,
        ledger: "migration",
    };

    /// The start of this target's BEGIN marker, up to the `(source:`.
    fn begin(&self) -> String {
        match self.tool {
            None => format!("{BEGIN_PREFIX} ("),
            Some(id) => format!("{BEGIN_PREFIX} tool={id} ("),
        }
    }
}

/// A `harness` command line as a next-step hint spells it: `harness <cmd>`,
/// with `--tool <id>` after it when a mapped tool is open — the one
/// spelling every hint and the generated block use, so a hint run as
/// written opens the same tool (an agent copies commands literally).
pub fn command_line(cmd: &str, tool: Option<&str>) -> String {
    match tool {
        None => format!("harness {cmd}"),
        Some(id) => format!("harness {cmd} --tool {id}"),
    }
}

/// The mapped tool whose ledger folder is `ledger_rel` (root-relative:
/// `migration/tools/<id>`), or `None` for the folder form's `migration`.
pub fn tool_of(ledger_rel: &str) -> Option<&str> {
    ledger_rel
        .strip_prefix(crate::ledger::MIGRATION_DIR)?
        .strip_prefix('/')?
        .strip_prefix(crate::config::TOOLS_DIR)?
        .strip_prefix('/')
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// The block's line for agents on adoption (docs/PROJECT-MAP-DESIGN.md
/// §3.7): the trust decision is the person's.
pub const AGENTS_NEVER_ADOPT: &str = "If a command says this folder holds migration results \
     made elsewhere, ask the person to adopt it (`harness … --adopt`, or the cockpit's \
     question); an agent never adopts.\n";

/// Render the managed block body (without markers). Deterministic, ≤60 lines,
/// only non-derivable ledger state (per the M2 spike's context-file evidence).
/// A folder-form target's body is the same bytes as before tools existed.
pub fn render_block_body(
    target_name: &str,
    at: BlockTarget<'_>,
    plan: &Plan,
    risk: &[UnitRisk],
) -> String {
    let ledger = at.ledger;
    let mut b = String::new();
    match at.tool {
        None => b.push_str(&format!("## RuHarness migration state — {target_name}\n\n")),
        Some(id) => b.push_str(&format!(
            "## RuHarness migration state — {target_name} (tool {id})\n\n"
        )),
    }
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
        "{} of {} units verified/merged. The ledger under `{ledger}/` is the source of truth — never hand-edit generated files; drive everything through `harness`.\n\n",
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
    let commands = [
        "scan",
        "plan",
        "detect",
        "observe",
        "verify <unit>",
        "state status",
        "review <finding>",
    ]
    .map(|cmd| format!("`{}`", command_line(cmd, at.tool)))
    .join(" · ");
    b.push_str(&format!("\nCommands: {commands} (all take `--target`).\n"));
    b.push_str(&format!("Read first: `{ledger}/plan.toml`, `{ledger}/observer/observations.md`, `{ledger}/DECISIONS.md`, `docs/SCHEMAS.md` (harness repo).\n"));
    b.push_str(AGENTS_NEVER_ADOPT);
    b
}

/// Wrap a body in the target's managed markers (content-hash in the end
/// marker). A folder-form target's markers are the same bytes as before
/// tools existed; a tool's BEGIN marker carries `tool=<id>`.
pub fn wrap_block(body: &str, at: BlockTarget<'_>) -> String {
    let hash = blake3::hash(body.as_bytes()).to_hex().to_string();
    let run = match at.tool {
        None => "harness sync-runtime".to_string(),
        Some(id) => format!("harness sync-runtime --tool {id}"),
    };
    format!(
        "{}source: {}/ — do not edit; run `{run}`) -->\n{body}{END_PREFIX} (content-hash: blake3:{}) -->\n",
        at.begin(),
        at.ledger,
        &hash[..16]
    )
}

/// Splice the target's managed block into existing AGENTS.md content
/// (replaces that target's block, else appends). Human prose outside the
/// markers, and every other target's block, survive.
///
/// Refuses (never guesses a replacement range) when the file holds this
/// target's BEGIN marker without a well-formed END marker after it (before
/// any other block begins), or more than one BEGIN marker for this target —
/// a corrupted block must be repaired by a human, not overwritten.
pub fn apply(existing: Option<&str>, at: BlockTarget<'_>, block: &str) -> Result<String, Error> {
    let text = match existing {
        None => return Ok(format!("# Agent guide\n\n{block}")),
        Some(t) => t,
    };
    let begin = at.begin();
    let begins = text.matches(begin.as_str()).count();
    if begins > 1 {
        return Err(Error::Invariant(
            "AGENTS.md contains more than one RUHARNESS GENERATED block for this target; repair \
             it by hand"
                .into(),
        ));
    }
    // An old harness's marker read another way (a stray BEGIN line that is
    // no target's) is left alone: only this target's marker is replaced.
    match text.find(begin.as_str()) {
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
            // The END found must be this block's own: another block's BEGIN
            // before it means this one lost its END.
            if after_begin[BEGIN_PREFIX.len()..end_rel].contains(BEGIN_PREFIX) {
                return Err(Error::Invariant(
                    "AGENTS.md has a BEGIN RUHARNESS GENERATED marker without its END marker; repair it by hand"
                        .into(),
                ));
            }
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

    const FOLDER: BlockTarget<'static> = BlockTarget::FOLDER;

    #[test]
    fn apply_replaces_only_the_managed_block() {
        let block1 = wrap_block("state A\n", FOLDER);
        let with_prose = format!("# My notes\n\nhuman text\n\n{block1}\nmore human text\n");
        let block2 = wrap_block("state B\n", FOLDER);
        let updated = apply(Some(&with_prose), FOLDER, &block2).unwrap();
        assert!(updated.contains("human text"));
        assert!(updated.contains("more human text"));
        assert!(updated.contains("state B"));
        assert!(!updated.contains("state A"));
        // idempotent
        assert_eq!(apply(Some(&updated), FOLDER, &block2).unwrap(), updated);
    }

    #[test]
    fn apply_refuses_corrupted_markers_instead_of_eating_prose() {
        let block = wrap_block("state A\n", FOLDER);
        let corrupted = format!("intro\n\n{}\n## Human notes\nkeep me\n", block)
            .replace("END RUHARNESS GENERATED", "END RUHARNESS BROKEN");
        let err = apply(Some(&corrupted), FOLDER, &wrap_block("state B\n", FOLDER)).unwrap_err();
        assert!(err.to_string().contains("repair"), "{err}");
        let doubled = format!("{block}\n{block}");
        assert!(apply(Some(&doubled), FOLDER, &block).is_err());
    }

    /// The folder form's markers are the bytes every committed AGENTS.md
    /// holds (a `--check` on them stays clean).
    #[test]
    fn the_folder_forms_markers_are_unchanged() {
        let block = wrap_block("x\n", FOLDER);
        assert!(block.starts_with(
            "<!-- BEGIN RUHARNESS GENERATED v1 (source: migration/ — do not edit; run `harness \
             sync-runtime`) -->\nx\n<!-- END RUHARNESS GENERATED (content-hash: blake3:"
        ));
    }

    /// Two tools' blocks live side by side; each is replaced alone, and a
    /// tool whose block lost its END does not eat the next tool's block.
    #[test]
    fn two_tools_keep_two_blocks() {
        let a = BlockTarget {
            tool: Some("t-a"),
            ledger: "migration/tools/t-a",
        };
        let b = BlockTarget {
            tool: Some("t-b"),
            ledger: "migration/tools/t-b",
        };
        let text = apply(None, a, &wrap_block("A1\n", a)).unwrap();
        let text = apply(Some(&text), b, &wrap_block("B1\n", b)).unwrap();
        assert!(text.contains("A1\n") && text.contains("B1\n"), "{text}");
        assert!(text.contains("tool=t-a (source: migration/tools/t-a/"));
        assert!(text.contains("run `harness sync-runtime --tool t-b`"));
        let text = apply(Some(&text), a, &wrap_block("A2\n", a)).unwrap();
        assert!(text.contains("A2\n") && !text.contains("A1\n") && text.contains("B1\n"));
        assert_eq!(text.matches(BEGIN_PREFIX).count(), 2);
        // A folder-form block beside them is its own.
        let text = apply(Some(&text), FOLDER, &wrap_block("F\n", FOLDER)).unwrap();
        assert_eq!(text.matches(BEGIN_PREFIX).count(), 3);
        // t-a's END gone: refused, t-b's block untouched.
        let broken = text.replacen(
            &wrap_block("A2\n", a),
            &wrap_block("A2\n", a).replace(END_PREFIX, "<!-- gone"),
            1,
        );
        assert!(apply(Some(&broken), a, &wrap_block("A3\n", a)).is_err());
    }

    #[test]
    fn a_tools_body_names_its_ledger_and_its_tool() {
        let plan = Plan {
            schema_version: 1,
            target: "lz4".into(),
            units: Vec::new(),
        };
        let body = render_block_body(
            "lz4",
            BlockTarget {
                tool: Some("t-lz4"),
                ledger: "migration/tools/t-lz4",
            },
            &plan,
            &[],
        );
        assert!(body.contains("(tool t-lz4)"), "{body}");
        assert!(body.contains("`migration/tools/t-lz4/plan.toml`"), "{body}");
        // Every command is spelled as it must be run (an agent copies it).
        assert!(
            body.contains(
                "Commands: `harness scan --tool t-lz4` · `harness plan --tool t-lz4` · \
                 `harness detect --tool t-lz4` · `harness observe --tool t-lz4` · \
                 `harness verify <unit> --tool t-lz4` · `harness state status --tool t-lz4` · \
                 `harness review <finding> --tool t-lz4` (all take `--target`).\n"
            ),
            "{body}"
        );
        assert!(body.ends_with(AGENTS_NEVER_ADOPT), "{body}");
        let folder = render_block_body("lz4", FOLDER, &plan, &[]);
        assert!(folder.contains("`migration/plan.toml`") && !folder.contains("--tool"));
    }

    /// The folder form's body, pinned byte for byte: what every committed
    /// AGENTS.md holds, plus the one line that tells agents never to adopt
    /// (added deliberately; a `sync-runtime` regenerates the block once).
    #[test]
    fn the_folder_forms_body_is_pinned() {
        let plan = Plan {
            schema_version: 1,
            target: "z".into(),
            units: Vec::new(),
        };
        assert_eq!(
            render_block_body("z", FOLDER, &plan, &[]),
            "## RuHarness migration state — z\n\n\
             0 of 0 units verified/merged. The ledger under `migration/` is the source of truth \
             — never hand-edit generated files; drive everything through `harness`.\n\n\
             Next units (execution order · risk 0-100):\n\
             \nCommands: `harness scan` · `harness plan` · `harness detect` · `harness observe` \
             · `harness verify <unit>` · `harness state status` · `harness review <finding>` \
             (all take `--target`).\n\
             Read first: `migration/plan.toml`, `migration/observer/observations.md`, \
             `migration/DECISIONS.md`, `docs/SCHEMAS.md` (harness repo).\n\
             If a command says this folder holds migration results made elsewhere, ask the \
             person to adopt it (`harness … --adopt`, or the cockpit's question); an agent never \
             adopts.\n"
        );
    }
}
