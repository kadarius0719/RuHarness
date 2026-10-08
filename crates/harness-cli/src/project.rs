//! `harness project map` (docs/PROJECT-MAP-DESIGN.md §3.6, §3.8): map the
//! whole project — every folder's C files, their compiles and symbols, the
//! programs and the files each needs, the link checks — write
//! `migration/map/project-map.json` (`ruharness-project-map` v1) and show
//! it. Nothing becomes a tool until `harness project accept <id>`.
//!
//! The command takes the project lock (`migration/map/.lock`), and on a
//! folder-form project that target's ledger lock too (the map's folder sits
//! inside its ledger). The first map writes `migration/.gitignore`, never
//! over an existing one.
//!
//! Every string from the project is printed through
//! [`harness_core::text::safe_line`] (newlines and tabs too: a file name can
//! hold a newline, and the review gate must not be forged); `--json` events
//! carry them raw, escaped by [`report::event`].

use crate::{lock_ledger, out, report, require_sandbox};
use anyhow::{bail, Result};
use harness_core::adopt;
use harness_core::ledger::{Ledger, WriterLock};
use harness_core::text::safe_line;
use harness_oracle::projectmap::evidence::CompileCommands;
use harness_oracle::projectmap::mapfile::{
    self, ClosureRec, CompiledRec, DuplicateRec, LinkedRec, MapFile, ProgramRec,
};
use harness_oracle::projectmap::{self, Compiled, FileFacts, FolderMap, MapOptions};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// `harness project map --target DIR`: map the whole project at `DIR` and
/// write its map. Exit 0 when the map was written in full (its closures may
/// still be incomplete: that is a fact in it); 1 refused or cut short (no C
/// found; a cap hit — the file facts are still written; no sandbox unless
/// allowed; a root that is or holds the home folder or the cargo or rustup
/// home; a ledger made elsewhere not adopted; the lock held).
pub(crate) fn cmd_map(
    target: PathBuf,
    allow_unsandboxed: bool,
    configuration: Option<String>,
) -> Result<u8> {
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
    // The project lock, then a folder-form target's ledger lock (§3.7).
    let _project = WriterLock::acquire_project(&root, "project map")?;
    let _ledger = if root.join("harness.toml").is_file() {
        Some(lock_ledger(&Ledger::new(&root), "project map")?)
    } else {
        None
    };
    let options = MapOptions {
        configuration,
        ..MapOptions::default()
    };
    let map = projectmap::map_root(&root, &options)?;
    if map.files.is_empty() {
        bail!(
            "no C files (.c or .h) were found in {}: point --target at a folder that holds a C \
             project",
            safe_line(&root.display().to_string())
        );
    }
    let analysis = mapfile::analyze(&map)?;
    let file = mapfile::render(&map, analysis.as_ref())?;
    let wrote_ignore = mapfile::write_gitignore(&root)?;
    mapfile::write(&root, &file)?;
    let json = report::mode() == report::Mode::Json;
    if let Some(hit) = map.limits_hit.first() {
        if json {
            for f in &map.files {
                report::event(&file_event(f));
            }
            show_build(&map, &file, true);
        }
        bail!(
            "the map stopped at its {} limit of {}: {} holds the file facts only and no \
             programs, because files past the limit were not visited or compiled; map a smaller \
             folder",
            hit.limit,
            hit.at,
            mapfile::MAP_FILE
        );
    }
    show(&map, &file, json);
    let mut wrote = mapfile::MAP_FILE.to_string();
    if wrote_ignore {
        wrote.push_str(&format!(" and {}", mapfile::GITIGNORE));
    }
    let target_arg = shell_word(&root.display().to_string());
    out(format!(
        "project map: wrote {wrote} ({} program(s), {} librar{}; the project's own files were \
         not changed); next, `harness project ask --target {target_arg}` asks a model for \
         advice on held choices, or `harness project accept <id> --target {target_arg}` makes \
         one of them a tool",
        file.programs.len(),
        file.libraries.len(),
        if file.libraries.len() == 1 {
            "y"
        } else {
            "ies"
        },
    ));
    Ok(0)
}

/// `text` as one shell word, quoted when it needs it (display only).
fn shell_word(text: &str) -> String {
    let text = safe_line(text);
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+,:@".contains(c))
    {
        text
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

// ---------- the `--json` events ----------

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
            header.as_deref().and_then(mapfile::clean_header),
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

/// One program (§3.8).
#[derive(Serialize)]
struct ProgramEvent<'a> {
    k: &'static str,
    id: &'a str,
    path: &'a str,
    kind: &'static str,
    kind_guess: &'static str,
    files: &'a [String],
    outside: &'a [String],
    incomplete: bool,
    held: &'a [String],
}

/// One link check (§3.8): `ok`, or what was missing and doubled.
#[derive(Serialize)]
struct LinkEvent<'a> {
    k: &'static str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doubled: Option<&'a [String]>,
}

/// The configuration, the build evidence, as `--json` carries them.
#[derive(Serialize)]
struct BuildEvent<'a> {
    k: &'static str,
    configuration: &'a str,
    from: harness_core::config::ConfigurationFrom,
    source: &'static str,
    flags: &'a [String],
    system_headers: &'a [String],
    digest: &'a str,
    compile_commands: Option<&'a str>,
    ignored_entries: usize,
    unfound_entries: &'a [String],
    build_files: &'a [String],
    flags_differ: Vec<&'a str>,
    ignored_flags: Vec<(&'a str, &'a str, usize)>,
    ignored_flag_count: usize,
    set_aside: Vec<(&'a str, &'static str, usize)>,
    limits_hit: Vec<&'static str>,
}

// ---------- the screen ----------

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

/// The configuration, what the build says and what was set aside.
fn show_build(map: &FolderMap, file: &MapFile, json: bool) {
    let c = &file.configuration;
    let ev = &map.evidence;
    let cc_path = match &ev.compile_commands {
        CompileCommands::Present { path } => Some(path.as_str()),
        _ => None,
    };
    if json {
        report::event(&BuildEvent {
            k: "project-build",
            configuration: &c.name,
            from: c.from,
            source: c.source,
            flags: &c.flags,
            system_headers: &c.system_headers,
            digest: &c.digest,
            compile_commands: cc_path,
            ignored_entries: ev.ignored_entries,
            unfound_entries: &ev.unfound_entries,
            build_files: &ev.build_files,
            flags_differ: ev.flags_differ.iter().map(|f| f.path.as_str()).collect(),
            ignored_flags: ev
                .ignored_flags
                .iter()
                .map(|f| (f.flag.as_str(), f.why.as_str(), f.count))
                .collect(),
            ignored_flag_count: ev.ignored_flag_count,
            set_aside: map
                .set_aside
                .iter()
                .map(|s| (s.folder.as_str(), s.lang, s.count))
                .collect(),
            limits_hit: map.limits_hit.iter().map(|h| h.limit).collect(),
        });
        return;
    }
    out(format!(
        "configuration: {} ({}), flags {}{}",
        safe_line(&c.name),
        c.source,
        if c.flags.is_empty() {
            "none".into()
        } else {
            list(&c.flags)
        },
        if c.source == projectmap::ConfigSource::Guessed.as_str() {
            "; a guess: failed compiles are expected until migration/map/config.toml states \
             the build"
        } else {
            ""
        }
    ));
    match &ev.compile_commands {
        CompileCommands::Absent => {}
        CompileCommands::Present { path } => out(format!(
            "  compile_commands.json read from {}: {} entries ignored, {} flags ignored, {} \
             files it names not found",
            safe_line(path),
            ev.ignored_entries,
            ev.ignored_flag_count,
            ev.unfound_entries.len()
        )),
        CompileCommands::Unreadable { path, why } => out(format!(
            "  compile_commands.json at {} not read: {}",
            safe_line(path),
            safe_line(why)
        )),
    }
    for f in &ev.ignored_flags {
        out(format!(
            "  ignored flag {} ({}×): {}",
            safe_line(&f.flag),
            f.count,
            safe_line(&f.why)
        ));
    }
    for f in &ev.flags_differ {
        out(format!(
            "  flags differ: {} is listed {} times with different flags",
            safe_line(&f.path),
            f.flags.len()
        ));
    }
    if !ev.build_files.is_empty() {
        out(format!("  build files: {}", list(&ev.build_files)));
    }
}

/// `files` grouped by folder: `./ a.c, b.h; lib/ x.c`.
fn by_folder(files: &[String]) -> String {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in files {
        let (folder, name) = f.rsplit_once('/').unwrap_or((".", f));
        groups.entry(folder).or_default().push(name);
    }
    groups
        .iter()
        .map(|(folder, names)| {
            format!(
                "{}/ {}",
                safe_line(folder),
                names
                    .iter()
                    .map(|n| safe_line(n))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Whether the place a compile stopped (`<path>:<line>`) is an `#error`
/// directive: the line is read to tell, never shown or kept.
fn is_error_directive(root: &Path, at: &str) -> bool {
    let Some((path, line)) = at.rsplit_once(':') else {
        return false;
    };
    let Ok(n) = line.parse::<usize>() else {
        return false;
    };
    let abs = root.join(path);
    let small = std::fs::symlink_metadata(&abs)
        .is_ok_and(|m| m.is_file() && m.len() <= projectmap::MAX_SOURCE_BYTES);
    if n == 0 || !small {
        return false;
    }
    let Ok(bytes) = std::fs::read(&abs) else {
        return false;
    };
    let Some(text) = bytes.split(|b| *b == b'\n').nth(n - 1) else {
        return false;
    };
    let text = String::from_utf8_lossy(text);
    text.trim_start()
        .strip_prefix('#')
        .is_some_and(|rest| rest.trim_start().starts_with("error"))
}

/// Why a `.c` did not compile, in words.
fn not_compiled_words(root: &Path, c: &CompiledRec) -> String {
    let CompiledRec::Failed {
        reason,
        header,
        detail,
        at,
    } = c
    else {
        return "compiled".into();
    };
    let at_words = |a: &Option<String>| {
        a.as_deref()
            .map(|a| format!(" at {}", safe_line(a)))
            .unwrap_or_default()
    };
    match *reason {
        "missing-header" => format!(
            "a header was not found{}{}",
            header
                .as_deref()
                .map(|h| format!(" ({})", safe_line(h)))
                .unwrap_or_default(),
            at_words(at)
        ),
        "syntax" => match at {
            Some(a) if is_error_directive(root, a) => {
                format!("stopped at an #error directive{}", at_words(at))
            }
            _ => format!("a compile error{}", at_words(at)),
        },
        _ => format!(
            "the compile did not finish{}",
            detail.map(|d| format!(" ({d})")).unwrap_or_default()
        ),
    }
}

fn duplicate_line(program: &str, d: &DuplicateRec, held: bool) -> String {
    let name = |index: &str| {
        d.definers
            .iter()
            .find(|x| x.index == index)
            .map(|x| format!("{} {}", x.index, safe_line(&x.path)))
            .unwrap_or_else(|| index.to_string())
    };
    let under = d
        .under
        .as_deref()
        .map(|u| format!(", reached only when {u} is kept"))
        .unwrap_or_default();
    let what = format!("duplicate set {} ({}{under})", d.set, list(&d.symbols));
    if let Some(choice) = &d.choice {
        return format!("{what}: settled by linking, keeps {}", name(&choice.keep));
    }
    let all: Vec<String> = d.definers.iter().map(|x| name(&x.index)).collect();
    if held {
        let first = d.definers.first().map(|x| x.index.as_str()).unwrap_or("");
        format!(
            "{what}: held, linking cannot tell {} apart; pick one with `harness project accept \
             {} --keep {}={first}`",
            all.join(" from "),
            safe_line(program),
            d.set
        )
    } else {
        format!("{what}: {}", all.join(", "))
    }
}

fn linked_words(l: &Option<LinkedRec>) -> String {
    match l {
        None => "not link-checked".into(),
        Some(LinkedRec::Ok(_)) => "linked".into(),
        Some(LinkedRec::Failed { missing, doubled }) => {
            let mut s = "did not link".to_string();
            if !missing.is_empty() {
                s.push_str(&format!("; missing {}", list(missing)));
            }
            if !doubled.is_empty() {
                s.push_str(&format!("; defined twice {}", list(doubled)));
            }
            s
        }
    }
}

fn incomplete_words(c: &ClosureRec) -> Vec<String> {
    c.incomplete_why
        .iter()
        .map(|i| {
            let path = i.path.as_deref().map(safe_line).unwrap_or_default();
            let syms = list(&i.symbols);
            match i.why {
                "pending" => format!("a duplicate set is still open ({syms})"),
                "may-be-defined-in" => {
                    format!("{path} did not compile and may define {syms}")
                }
                "unread" => format!("{path} could not be read and may define {syms}"),
                "unreadable-folder" => format!("the folder {path} could not be read"),
                other => format!("{other} {path}"),
            }
        })
        .collect()
}

/// One program with its closure.
fn show_program(p: &ProgramRec, c: Option<&ClosureRec>, file: &MapFile) {
    let index = p
        .index
        .as_deref()
        .map(|i| format!("{i} "))
        .unwrap_or_default();
    out(format!(
        "  {index}{} — {} ({}; kind guess from its folder: {})",
        safe_line(&p.id),
        safe_line(&p.path),
        p.kind,
        p.kind_guess
    ));
    if p.kind == "driver" {
        out(format!(
            "      a fuzz driver serving {}: never linked alone, each fuzzer is linked with it",
            list(&p.serves)
        ));
        for f in &p.serves {
            let fuzzer = file.programs.iter().find(|q| &q.path == f);
            let linked = fuzzer
                .and_then(|q| file.closures.iter().find(|c| c.program == q.id))
                .map(|c| linked_words(&c.linked))
                .unwrap_or_else(|| "not link-checked".into());
            out(format!("      fuzzer {}: {linked}", safe_line(f)));
        }
        return;
    }
    let Some(c) = c else { return };
    out(format!("      files: {}", by_folder(&c.files)));
    let libs = projectmap::link::guess_libs(&c.outside, true, cfg!(target_vendor = "apple"));
    out(format!(
        "      outside symbols: {}; guessed libraries: {}",
        list(&c.outside),
        if libs.is_empty() {
            "none".into()
        } else {
            libs.join(" ")
        }
    ));
    let link = if c.linked.is_none() && !c.questions.is_empty() {
        format!("not linked while {} is open", c.questions.join(", "))
    } else {
        linked_words(&c.linked)
    };
    out(format!("      link check: {link}"));
    if c.incomplete {
        out(format!(
            "      incomplete: {}",
            incomplete_words(c).join("; ")
        ));
    }
    for d in &c.duplicates {
        let held = c.questions.contains(&d.set);
        out(format!("      {}", duplicate_line(&p.id, d, held)));
    }
    for n in &c.needs_from {
        out(format!(
            "      needs {} from {}'s files",
            safe_line(&n.sym),
            safe_line(&n.program)
        ));
    }
    for x in &c.collisions {
        out(format!(
            "      {} is defined by {} (a collision)",
            safe_line(&x.sym),
            x.definers
                .iter()
                .map(|d| safe_line(d))
                .collect::<Vec<_>>()
                .join(" and ")
        ));
    }
    for a in &c.ambiguous_unsettled {
        out(format!(
            "      ambiguous include {}: {}; {}; settle it in migration/map/config.toml (-I or \
             system_headers)",
            safe_line(&a.header),
            list(&a.candidates),
            match &a.used {
                Some(u) => format!("the compile used {}", safe_line(u)),
                None => "none was used".into(),
            }
        ));
    }
    if !c.flags_differ.is_empty() {
        out(format!(
            "      flags differ between its files, so the configuration stays a guess: {}",
            c.flags_differ
                .iter()
                .map(|f| format!("{} [{}]", safe_line(&f.path), list(&f.flags)))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !file.build_evidence.build_files.is_empty() {
        out(format!(
            "      the project's build ({}) may link more than these files; the map never runs it",
            list(&file.build_evidence.build_files)
        ));
    }
}

/// The screen (§3.6, §3.8): events under `--json`, lines otherwise.
fn show(map: &FolderMap, file: &MapFile, json: bool) {
    if json {
        for f in &map.files {
            report::event(&file_event(f));
        }
        show_build(map, file, true);
        let empty: Vec<String> = Vec::new();
        for p in &file.programs {
            let c = file.closures.iter().find(|c| c.program == p.id);
            report::event(&ProgramEvent {
                k: "project-program",
                id: &p.id,
                path: &p.path,
                kind: p.kind,
                kind_guess: p.kind_guess,
                files: c.map(|c| c.files.as_slice()).unwrap_or(&empty),
                outside: c.map(|c| c.outside.as_slice()).unwrap_or(&empty),
                incomplete: c.is_some_and(|c| c.incomplete),
                held: c.map(|c| c.questions.as_slice()).unwrap_or(&empty),
            });
            if let Some(linked) = c.and_then(|c| c.linked.as_ref()) {
                report::event(&match linked {
                    LinkedRec::Ok(_) => LinkEvent {
                        k: "project-link",
                        id: &p.id,
                        ok: Some(true),
                        missing: None,
                        doubled: None,
                    },
                    LinkedRec::Failed { missing, doubled } => LinkEvent {
                        k: "project-link",
                        id: &p.id,
                        ok: None,
                        missing: Some(missing),
                        doubled: Some(doubled),
                    },
                });
            }
        }
        return;
    }
    out(format!(
        "project map of {}: compiled with {} ({}); the harness's own flags on every compile: {}",
        safe_line(&map.root.display().to_string()),
        safe_line(&map.toolchain.cc),
        safe_line(&map.toolchain.target),
        map.toolchain.cflags.join(" ")
    ));
    show_build(map, file, false);

    out(format!("programs: {}", file.programs.len()));
    // Shown in path order (a `main` program's index follows it).
    let mut programs: Vec<&ProgramRec> = file.programs.iter().collect();
    programs.sort_by(|a, b| a.path.cmp(&b.path));
    for p in programs {
        let c = file.closures.iter().find(|c| c.program == p.id);
        show_program(p, c, file);
    }
    if !file.programs.is_empty() {
        out(
            "what the link check proves: each linked program's files, with this configuration \
             and the guessed libraries, define every symbol it needs exactly once; it does not \
             prove the program is a tool rather than a test, that the right file was kept when \
             several link, that this is the configuration the project's own build uses, or that \
             the program runs"
                .into(),
        );
    }
    for s in &file.shared {
        out(format!(
            "shared file: {} (in {})",
            safe_line(&s.file),
            list(&s.programs)
        ));
    }
    for l in &file.libraries {
        let needs = if l.needs_from_outside.is_empty() {
            String::new()
        } else {
            format!("; needs {}", list(&l.needs_from_outside))
        };
        out(format!(
            "library {}: {}{needs}",
            safe_line(&l.id),
            by_folder(&l.files)
        ));
    }
    for d in &file.between_program_duplicates {
        out(format!(
            "defined in two programs' files that never meet (listed, never asked): {} in {}",
            safe_line(&d.sym),
            list(&d.definers)
        ));
    }
    for p in &file.programs_not_compiled {
        out(format!("a program that did not compile: {}", safe_line(p)));
    }
    // Every `.c` no program, closure or library holds.
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    reached.extend(file.programs.iter().map(|p| p.path.as_str()));
    reached.extend(file.programs_not_compiled.iter().map(String::as_str));
    for c in &file.closures {
        reached.extend(c.files.iter().map(String::as_str));
        // A duplicate's definers are its program's alternatives.
        for d in &c.duplicates {
            reached.extend(d.definers.iter().map(|x| x.path.as_str()));
        }
    }
    for l in &file.libraries {
        reached.extend(l.files.iter().map(String::as_str));
    }
    let unreached: Vec<String> = file
        .files
        .iter()
        .filter(|f| f.kind == "c" && !reached.contains(f.path.as_str()))
        .map(|f| f.path.clone())
        .collect();
    if !unreached.is_empty() {
        out(format!(
            "unreached: {} .c file(s) in no program or library: {}",
            unreached.len(),
            list(&unreached)
        ));
    }
    for f in &file.files {
        if let Some(c @ CompiledRec::Failed { .. }) = &f.compiled {
            out(format!(
                "did not compile: {} — {}",
                safe_line(&f.path),
                not_compiled_words(&map.root, c)
            ));
        }
        if f.too_large {
            out(format!(
                "too large to read (over {} MiB): {}",
                projectmap::MAX_SOURCE_BYTES >> 20,
                safe_line(&f.path)
            ));
        }
        if f.outside_includes {
            out(format!(
                "read files outside the project, so its {} symbol names are withheld: {}",
                f.withheld_names,
                safe_line(&f.path)
            ));
        }
    }
    let mut ambiguous: BTreeMap<(&str, &[String]), Option<&str>> = BTreeMap::new();
    for f in &file.files {
        for a in &f.ambiguous_includes {
            ambiguous
                .entry((a.header.as_str(), a.candidates.as_slice()))
                .or_insert(a.used.as_deref());
        }
    }
    for ((header, candidates), used) in &ambiguous {
        out(format!(
            "ambiguous include {}: {}{}",
            safe_line(header),
            list(candidates),
            used.map(|u| format!("; the compile used {}", safe_line(u)))
                .unwrap_or_default()
        ));
    }
    for s in &map.set_aside {
        out(format!(
            "set aside in {}: {} {} file(s), not read",
            safe_line(&s.folder),
            s.count,
            s.lang
        ));
    }
    for s in &map.skipped_folders {
        let count = format!(
            "{}{} C files",
            if s.complete { "" } else { "at least " },
            s.files
        );
        let why = if s.path == harness_core::ledger::MIGRATION_DIR {
            "the harness's own files".to_string()
        } else if s
            .path
            .rsplit('/')
            .next()
            .is_some_and(|n| n.starts_with('.'))
        {
            format!("a dot-folder, {count}")
        } else {
            count
        };
        out(format!("skipped folder: {} ({why})", safe_line(&s.path)));
    }
    for issue in &map.walk_issues {
        out(format!(
            "not walked: {} — {}",
            safe_line(&issue.path),
            safe_line(&issue.why)
        ));
    }
}
