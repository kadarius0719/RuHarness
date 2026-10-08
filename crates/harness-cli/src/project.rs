//! `harness project map` (docs/PROJECT-MAP-DESIGN.md §3.8), its first form
//! (§5 step a): one folder's per-file facts — each file's compile, include
//! folders, ambiguous includes and symbol counts, and the walk's issues and
//! skipped folders. Nothing is written yet; the map file comes with step b.
//!
//! Every string from the project is printed through
//! [`harness_core::text::safe_line`] (newlines and tabs too: a file name can
//! hold a newline, and the review gate must not be forged); `--json` events
//! carry them raw, escaped by [`report::event`].

use crate::{out, report, require_sandbox};
use anyhow::{bail, Result};
use harness_core::adopt;
use harness_core::config::TargetConfig;
use harness_core::text::safe_line;
use harness_oracle::projectmap::{self, Compiled, FileFacts, FileKind, FolderMap};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// `harness project map --target DIR`: map `DIR`'s `source_dir` when it
/// holds a `harness.toml`, else `DIR` itself. Exit 0 when every file was
/// visited; 1 refused (no C found, a cap hit, no sandbox unless allowed, a
/// root that is or holds the home folder or the cargo or rustup home, a
/// ledger made elsewhere not adopted).
pub(crate) fn cmd_map(target: PathBuf, allow_unsandboxed: bool) -> Result<u8> {
    if !target.is_dir() {
        bail!(
            "{} is not a folder: point --target at a C project's folder",
            safe_line(&target.display().to_string())
        );
    }
    let root = target.canonicalize()?;
    // The folder may hold no harness.toml, so no `TargetContext::load` runs
    // the adoption check for this command: it runs here.
    adopt::check(&root)?;
    projectmap::refuse_root(&root)?;
    require_sandbox(allow_unsandboxed, "project map")?;
    let folder = if root.join("harness.toml").is_file() {
        TargetConfig::load(&root)?.target.source_dir
    } else {
        ".".to_string()
    };
    let map = projectmap::map_folder(&root, Path::new(&folder))?;
    show(&map);
    if map.files.is_empty() {
        bail!(
            "no C files (.c or .h) were found in {}: point --target at a folder that holds a C \
             project",
            safe_line(&shown_folder(&map))
        );
    }
    if let Some(hit) = map.limits_hit.first() {
        bail!(
            "the map stopped at its limit of {}: the files past it were not visited; map a \
             smaller folder",
            hit.at
        );
    }
    Ok(0)
}

fn shown_folder(map: &FolderMap) -> String {
    if map.folder == "." {
        map.root.display().to_string()
    } else {
        map.root.join(&map.folder).display().to_string()
    }
}

/// One file, as the `--json` stream carries it.
#[derive(Serialize)]
struct FileEvent<'a> {
    k: &'static str,
    path: &'a str,
    kind: &'static str,
    compiled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    header: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    too_large: bool,
    include_dirs: &'a [String],
    ambiguous: Vec<AmbiguousEvent<'a>>,
    defined: usize,
    needed: usize,
    outside_includes: bool,
    #[serde(skip_serializing_if = "is_zero")]
    withheld: usize,
    #[serde(skip_serializing_if = "is_zero")]
    odd_names: usize,
}

#[derive(Serialize)]
struct AmbiguousEvent<'a> {
    header: &'a str,
    candidates: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    used: Option<&'a str>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn file_event(f: &FileFacts) -> FileEvent<'_> {
    let (reason, header, detail, at) = match &f.compiled {
        Some(Compiled::Failed {
            reason,
            header,
            detail,
            at,
        }) => (
            Some(reason.as_str()),
            header.as_deref(),
            *detail,
            at.as_deref(),
        ),
        _ => (None, None, None, None),
    };
    FileEvent {
        k: "project-file",
        path: &f.path,
        kind: f.kind.as_str(),
        compiled: f.compiled == Some(Compiled::Ok),
        reason,
        header,
        detail,
        at,
        too_large: f.too_large,
        include_dirs: &f.include_dirs,
        ambiguous: f
            .ambiguous
            .iter()
            .map(|a| AmbiguousEvent {
                header: &a.header,
                candidates: &a.candidates,
                used: a.used.as_deref(),
            })
            .collect(),
        defined: f.defined.len(),
        needed: f.needed.len(),
        outside_includes: f.outside_includes,
        withheld: f.withheld_names,
        odd_names: f.odd_names,
    }
}

/// The compile, in words.
fn compiled_words(f: &FileFacts) -> String {
    if f.too_large {
        return format!(
            "too large to read (over {} MiB), not compiled",
            projectmap::MAX_SOURCE_BYTES >> 20
        );
    }
    match (&f.compiled, f.kind) {
        (None, FileKind::H) => "header".into(),
        (None, FileKind::C) => "not compiled".into(),
        (Some(Compiled::Ok), _) => "compiled".into(),
        (
            Some(Compiled::Failed {
                reason,
                header,
                detail,
                at,
            }),
            _,
        ) => {
            let mut s = format!("did not compile ({}", reason.as_str());
            for part in [header.as_deref(), *detail].into_iter().flatten() {
                s.push(' ');
                s.push_str(&safe_line(part));
            }
            if let Some(at) = at {
                s.push_str(&format!(" at {}", safe_line(at)));
            }
            s.push(')');
            s
        }
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "none".into()
    } else {
        items
            .iter()
            .map(|i| safe_line(i))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Print the map: events under `--json`, lines otherwise.
fn show(map: &FolderMap) {
    let json = report::mode() == report::Mode::Json;
    out(format!(
        "project map: {} with {} ({}), flags {}",
        safe_line(&shown_folder(map)),
        safe_line(&map.toolchain.cc),
        safe_line(&map.toolchain.target),
        map.toolchain.cflags.join(" ")
    ));
    for f in &map.files {
        if json {
            report::event(&file_event(f));
            continue;
        }
        let mut line = format!(
            "  {} — {}; include folders: {}",
            safe_line(&f.path),
            compiled_words(f),
            list(&f.include_dirs)
        );
        if f.kind == FileKind::C && f.compiled == Some(Compiled::Ok) {
            if f.outside_includes {
                line.push_str(&format!(
                    "; read files outside the project, so its {} symbol names are withheld",
                    f.withheld_names
                ));
            } else {
                line.push_str(&format!(
                    "; defines {}, needs {}",
                    f.defined.len(),
                    f.needed.len()
                ));
            }
            if f.odd_names > 0 {
                line.push_str(&format!("; {} odd symbol names counted", f.odd_names));
            }
        }
        if f.not_utf8 {
            line.push_str("; not UTF-8");
        }
        out(line);
        for a in &f.ambiguous {
            out(format!(
                "      ambiguous include {}: {}; {}",
                safe_line(&a.header),
                list(&a.candidates),
                match &a.used {
                    Some(u) => format!("the compile used {}", safe_line(u)),
                    None => "none used".into(),
                }
            ));
        }
        if !f.included_other.is_empty() {
            out(format!("      also read: {}", list(&f.included_other)));
        }
        if let Some(said) = &f.message {
            out(format!("      the compiler said: {}", safe_line(said)));
        }
    }
    for issue in &map.walk_issues {
        out(format!(
            "  not walked: {} — {}",
            safe_line(&issue.path),
            safe_line(&issue.why)
        ));
    }
    for s in &map.skipped_folders {
        out(format!(
            "  skipped folder: {} ({}{} C files)",
            safe_line(&s.path),
            if s.complete { "" } else { "at least " },
            s.files
        ));
    }
    let c = map.files.iter().filter(|f| f.kind == FileKind::C).count();
    let ok = map
        .files
        .iter()
        .filter(|f| f.compiled == Some(Compiled::Ok))
        .count();
    let ambiguous: usize = map.files.iter().map(|f| f.ambiguous.len()).sum();
    out(format!(
        "project map: {} files ({c} .c, {} .h): {ok} compiled, {} did not; {ambiguous} \
         ambiguous include(s); nothing written yet",
        map.files.len(),
        map.files.len() - c,
        c - ok
    ));
}
