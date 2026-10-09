//! `harness project accept <id>` (docs/PROJECT-MAP-DESIGN.md §3.6, §3.8):
//! make a mapped program or library a tool. The checks, the re-link and the
//! `harness.toml` it writes are harness-oracle's
//! ([`harness_oracle::projectmap::accept`]); this command takes the locks
//! (the project lock, then an existing tool's ledger lock, in that order),
//! prints each pick in words **before** writing, writes the one file and
//! says what to do next. Exit 1 when refused (one sentence), 2 usage.

use crate::{lock_ledger, out, report, require_sandbox};
use anyhow::{bail, Result};
use harness_core::adopt;
use harness_core::config as hconfig;
use harness_core::ledger::{Ledger, WriterLock};
use harness_core::text::safe_line;
use harness_oracle::projectmap::{self, accept};
use serde::Serialize;
use std::path::PathBuf;

/// One pick, as the `--json` stream carries it.
#[derive(Serialize)]
struct PickEvent<'a> {
    set: &'a str,
    definers: &'a [String],
    keep: &'a str,
    by: &'static str,
}

/// The accepted tool, as the `--json` stream carries it.
#[derive(Serialize)]
struct AcceptEvent<'a> {
    k: &'static str,
    id: &'a str,
    path: &'a str,
    library: bool,
    replaced: bool,
    files: Vec<&'a str>,
    configuration: &'a str,
    flags: &'a [String],
    extra_link_args: &'a [String],
    run_name: &'a str,
    picks: Vec<PickEvent<'a>>,
    not_kept: &'a [String],
}

/// `harness project accept <id> --target DIR [--keep …]… [--run-name NAME]`.
pub(crate) fn cmd_accept(
    id: String,
    target: PathBuf,
    keeps: Vec<String>,
    run_name: Option<String>,
    allow_unsandboxed: bool,
) -> Result<u8> {
    if !target.is_dir() {
        bail!(
            "{} is not a folder: point --target at the mapped project's folder",
            safe_line(&target.display().to_string())
        );
    }
    let root = target.canonicalize()?;
    // The folder holds no harness.toml of its own, so no `TargetContext`
    // runs the adoption check for this command: it runs here.
    adopt::check(&root)?;
    projectmap::refuse_root(&root)?;
    require_sandbox(allow_unsandboxed, "project accept")?;
    // The project lock, then an existing tool's ledger lock (§3.6).
    let _project = WriterLock::acquire_project(&root, "project accept")?;
    let tool_dir = hconfig::tool_dir(&root, &id);
    let _tool = if std::fs::symlink_metadata(tool_dir.join(hconfig::CONFIG_FILE)).is_ok() {
        Some(lock_ledger(
            &Ledger::at(&root, &tool_dir),
            "project accept",
        )?)
    } else {
        None
    };
    let request = accept::Request {
        id: id.clone(),
        keeps,
        run_name,
    };
    let prepared = accept::prepare(&root, &request)?;
    let json = report::mode() == report::Mode::Json;
    // Each pick in words before anything is written (§3.3: a new file can
    // shift the indexes, so the person reads which file is kept).
    for p in &prepared.picks {
        out(format!("project accept {id}: {}", p.words()));
    }
    for f in &prepared.not_kept {
        out(format!("  {}: alternative not kept", safe_line(f)));
    }
    accept::write(&root, &prepared)?;
    if json {
        report::event(&AcceptEvent {
            k: "project-accept",
            id: &prepared.id,
            path: &prepared.rel,
            library: prepared.library,
            replaced: prepared.existed,
            files: prepared
                .shape
                .files
                .iter()
                .map(|(p, _)| p.as_str())
                .collect(),
            configuration: &prepared.shape.name,
            flags: &prepared.shape.flags,
            extra_link_args: &prepared.libs,
            run_name: &prepared.run_name,
            picks: prepared
                .picks
                .iter()
                .map(|p| PickEvent {
                    set: &p.set,
                    definers: &p.definers,
                    keep: &p.keep,
                    by: p.by,
                })
                .collect(),
            not_kept: &prepared.not_kept,
        });
    }
    out(closing_line(&prepared, &target));
    Ok(0)
}

/// What was written and what to do next, in one sentence.
fn closing_line(p: &accept::Prepared, target: &std::path::Path) -> String {
    let flags = if p.shape.flags.is_empty() {
        "no flags".to_string()
    } else {
        format!(
            "flags {}",
            p.shape
                .flags
                .iter()
                .map(|f| safe_line(f))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let what = if p.library {
        format!(
            "library, {} file(s), compiled, not linked",
            p.shape.files.len()
        )
    } else {
        format!(
            "{} file(s), linked{}, run as {}",
            p.shape.files.len(),
            if p.libs.is_empty() {
                String::new()
            } else {
                format!(" with {}", p.libs.join(" "))
            },
            safe_line(&p.run_name)
        )
    };
    let again = if p.existed {
        "; its ledger (plan, units, verdicts) is kept"
    } else {
        ""
    };
    let quoted = report::shell_quote(&target.to_string_lossy());
    format!(
        "project accept: wrote {} ({what}; configuration {}, {flags}{again}); review it with \
         `git diff`, then scan it: `harness scan --target {quoted} --tool {}`",
        p.rel,
        safe_line(&p.shape.name),
        p.id
    )
}
