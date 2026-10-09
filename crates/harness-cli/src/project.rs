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
use harness_core::ledger::{Ledger, WriterLock, MIGRATION_DIR};
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
    // Before anything is written: a folder with no C is refused untouched.
    if !projectmap::holds_c(&root) {
        bail!(
            "no C files (.c or .h) were found in {}: point --target at a folder that holds a C \
             project",
            safe_line(&root.display().to_string())
        );
    }
    let had_migration = std::fs::symlink_metadata(root.join(MIGRATION_DIR)).is_ok();
    let result = map_locked(&root, configuration);
    if result.is_err() && !had_migration {
        // A refusal leaves nothing this run created but what it wrote on
        // purpose (a map cut short at a limit keeps its file facts).
        undo_created(&root);
    }
    result
}

/// Remove `migration/map/.lock`, then `migration/map/` and `migration/`
/// when they hold nothing else: what a refused first run made.
fn undo_created(root: &Path) {
    let map_dir = root.join(harness_core::ledger::MAP_DIR);
    let only_lock = std::fs::read_dir(&map_dir)
        .map(|names| names.flatten().all(|e| e.file_name() == ".lock"))
        .unwrap_or(false);
    if only_lock {
        let _ = std::fs::remove_file(map_dir.join(".lock"));
        let _ = std::fs::remove_dir(&map_dir);
    }
    // Only when empty.
    let _ = std::fs::remove_dir(root.join(MIGRATION_DIR));
}

/// The map under the project lock (and a folder-form target's ledger
/// lock), written and shown.
fn map_locked(root: &Path, configuration: Option<String>) -> Result<u8> {
    // The project lock, then a folder-form target's ledger lock (§3.7).
    let _project = WriterLock::acquire_project(root, "project map")?;
    let _ledger = if root.join("harness.toml").is_file() {
        Some(lock_ledger(&Ledger::new(root), "project map")?)
    } else {
        None
    };
    let options = MapOptions {
        configuration,
        ..MapOptions::default()
    };
    let mut map = projectmap::map_root(root, &options)?;
    if map.files.is_empty() {
        bail!(
            "no C files (.c or .h) were found in {}: point --target at a folder that holds a C \
             project",
            safe_line(&root.display().to_string())
        );
    }
    let analysis = mapfile::analyze(&mut map)?;
    let analysis = if map.closures_possible() {
        analysis
    } else {
        None
    };
    let (file, bytes) = mapfile::render_bounded(&mut map, analysis.as_ref())?;
    // The programs of the map this one replaces, for the "what changed"
    // report.
    let before = mapfile::previous_programs(root);
    let wrote_ignore = mapfile::write_gitignore(root)?;
    mapfile::write_bytes(root, &bytes)?;
    let json = report::mode() == report::Mode::Json;
    if let Some(hit) = map.limits_hit.first() {
        if json {
            for f in &map.files {
                report::event(&file_event(f));
            }
            show_build(&map, &file, true);
        }
        let because = match hit.limit {
            "files" | "depth" => "files past the limit were not visited",
            "symbols" => "files past the limit were not compiled",
            "budget" => {
                "the time ran out before every file was compiled and every program link-checked"
            }
            _ => "the whole map would have been larger",
        };
        bail!(
            "the map stopped at its {} limit of {}: {} holds the file facts only and no \
             programs, because {because}; map a smaller folder",
            hit.limit,
            hit.at,
            mapfile::MAP_FILE
        );
    }
    show(&map, &file, json);
    show_changes(&map, &file, before.as_deref(), json);
    let mut wrote = mapfile::MAP_FILE.to_string();
    if wrote_ignore {
        wrote.push_str(&format!(" and {}", mapfile::GITIGNORE));
    }
    out(closing_line(&wrote, &file));
    Ok(0)
}

/// One accepted tool's changes, as the `--json` stream carries them.
#[derive(Serialize)]
struct ToolChangedEvent<'a> {
    k: &'static str,
    id: &'a str,
    what: &'a [String],
}

/// The "what changed" report (§3.6): for each accepted tool whose map
/// digests differ from this map's, what changed — and, when any tool was
/// accepted, the programs new since the map this one replaced.
fn show_changes(map: &FolderMap, file: &MapFile, before: Option<&[(String, String)]>, json: bool) {
    let changes = mapfile::what_changed(map, file);
    for c in &changes {
        if json {
            report::event(&ToolChangedEvent {
                k: "project-tool-changed",
                id: &c.id,
                what: &c.what,
            });
        }
        out(format!(
            "accepted tool {} changed since it was accepted: {}; accept it again with `harness \
             project accept {}`",
            safe_line(&c.id),
            c.what.join("; "),
            safe_line(&c.id)
        ));
    }
    let accepted = harness_core::config::mapped_tools(&map.root)
        .iter()
        .any(|id| mapfile::tool_config(&map.root, id).is_some());
    if let (true, Some(before)) = (accepted, before) {
        let new = mapfile::new_programs(file, before);
        if !new.is_empty() {
            out(format!(
                "new programs since the last map: {}",
                new.iter()
                    .map(|(id, path)| format!("{} ({})", safe_line(id), safe_line(path)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
}

/// The closing sentence (§3.6): what was written, and the next step —
/// `harness project accept <id>`; `ask` named only when a choice is held.
fn closing_line(wrote: &str, file: &MapFile) -> String {
    let held: Vec<String> = {
        let mut sets: Vec<String> = file
            .closures
            .iter()
            .flat_map(|c| c.questions.iter().cloned())
            .collect();
        sets.sort_by_key(|s| mapfile::index_numbers(s));
        sets.dedup();
        sets
    };
    let libraries = file.libraries.len();
    let mut line = format!(
        "project map: wrote {wrote} ({} program(s), {libraries} librar{}; the project's own \
         files were not changed); next, make a program or library a tool with `harness \
         project accept <id>`",
        file.programs.len(),
        if libraries == 1 { "y" } else { "ies" },
    );
    if !held.is_empty() {
        line.push_str(&format!(
            "; the held choices ({}) are yours to make: name the file to keep with --keep \
             <set>=<index or path> (`harness project ask` advises)",
            held.join(", ")
        ));
    }
    line
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
    #[serde(skip_serializing_if = "Option::is_none")]
    not_checked: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    not_compiled: Option<&'a [String]>,
}

/// The configuration, the build evidence, as `--json` carries them.
#[derive(Serialize)]
struct BuildEvent<'a> {
    k: &'static str,
    configuration: &'a str,
    from: harness_core::config::ConfigurationFrom,
    source: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    proposed: bool,
    flags: &'a [String],
    system_headers: &'a [String],
    digest: &'a str,
    compile_commands: Option<&'a str>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    also_found: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    not_in_compile_commands: &'a [String],
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

/// Most items a screen list names before "… and N more".
const MAX_LISTED: usize = 24;

/// `items` like [`list`], at most [`MAX_LISTED`] of them named, so one
/// line stays readable.
fn capped(items: &[String]) -> String {
    if items.len() <= MAX_LISTED {
        return list(items);
    }
    format!(
        "{} … and {} more",
        list(&items[..MAX_LISTED]),
        items.len() - MAX_LISTED
    )
}

/// A configuration's `from`, as `config.toml` spells it.
fn from_word(from: harness_core::config::ConfigurationFrom) -> &'static str {
    use harness_core::config::ConfigurationFrom as F;
    match from {
        F::Make => "make",
        F::Meson => "meson",
        F::Cmake => "cmake",
        F::CompileCommands => "compile_commands",
        F::Stated => "stated",
    }
}

/// The configuration line: its name, what it stands for, whose it is and
/// its flags, in words.
fn configuration_line(map: &FolderMap, file: &MapFile) -> String {
    let c = &file.configuration;
    let flags = if c.flags.is_empty() {
        "none".to_string()
    } else {
        list(&c.flags)
    };
    let from = from_word(c.from);
    let headers = if c.system_headers.is_empty() {
        String::new()
    } else {
        format!(", system headers {}", list(&c.system_headers))
    };
    if c.name == projectmap::config::GUESSED_NAME && !c.proposed {
        let what = if map.evidence.has_compile_commands() {
            "from compile_commands: each file compiles with its own entry's flags".to_string()
        } else {
            format!("flags {flags}")
        };
        return format!(
            "configuration: a guess (no {}), {what}; failed compiles are expected until that \
             file states the build",
            projectmap::config::CONFIG_FILE
        );
    }
    let name = safe_line(&c.name);
    if c.proposed {
        return format!(
            "configuration: {name}, from {from}, flags {flags}{headers}; it came with the \
             project, so it is proposed, not yours yet: run once with --adopt to state it, or \
             edit {}",
            projectmap::config::CONFIG_FILE
        );
    }
    let whose = match c.source {
        "compile_commands" => "read from compile_commands.json",
        "stated" => "stated in config.toml",
        _ if !map.evidence.has_compile_commands() => {
            "still a guess: no compile_commands.json was read"
        }
        _ => "still a guess: a file is listed with other flags, or not listed at all",
    };
    format!("configuration: {name}, from {from} ({whose}), flags {flags}{headers}")
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
            proposed: c.proposed,
            flags: &c.flags,
            system_headers: &c.system_headers,
            digest: &c.digest,
            compile_commands: cc_path,
            also_found: &ev.also_found,
            not_in_compile_commands: &ev.not_in_compile_commands,
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
    out(configuration_line(map, file));
    match &ev.compile_commands {
        CompileCommands::Absent => {}
        CompileCommands::Present { path } => out(format!(
            "  compile_commands.json read from {}: {} entries ignored, {} flags not used by the \
             map, {} files it names not found",
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
    if !ev.also_found.is_empty() {
        out(format!(
            "  also found, not read (only the first is): {}",
            capped(&ev.also_found)
        ));
    }
    if !ev.ignored_flags.is_empty() {
        let named: Vec<String> = ev
            .ignored_flags
            .iter()
            .map(|f| format!("{} ({}×)", f.flag, f.count))
            .collect();
        out(format!(
            "  flags not used by the map (outside the flags it allows, docs/SCHEMAS.md): {}",
            capped(&named)
        ));
    }
    if !ev.not_in_compile_commands.is_empty() {
        out(format!(
            "  not in compile_commands.json, so compiled with the configuration's flags alone: {}",
            capped(&ev.not_in_compile_commands)
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

/// How a duplicate set stands for its program: settled, held, none of its
/// choices linking, or not link-checked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetState {
    Held,
    NoneLinks,
    Open,
}

fn duplicate_line(d: &DuplicateRec, state: SetState) -> String {
    let name = |index: &str| {
        d.definers
            .iter()
            .find(|x| x.index == index)
            .map(|x| format!("{} {}", x.index, safe_line(&x.path)))
            .unwrap_or_else(|| index.to_string())
    };
    let under = match d.under.as_slice() {
        [] => String::new(),
        [one] if d.choice.is_some() => format!(", reached because {one} is kept"),
        some => format!(", reached only when {} is kept", some.join(" or ")),
    };
    let what = format!("duplicate set {} ({}{under})", d.set, capped(&d.symbols));
    if let Some(choice) = &d.choice {
        return format!("{what}: settled by linking, keeps {}", name(&choice.keep));
    }
    let all: Vec<String> = d.definers.iter().map(|x| name(&x.index)).collect();
    match state {
        // The choice is the person's: no definer is suggested.
        SetState::Held => format!(
            "{what}: held, linking cannot tell {} apart, so the choice is yours",
            all.join(" from ")
        ),
        SetState::NoneLinks => format!(
            "{what}: neither choice links ({}); the link check below is the closest one",
            all.join(", ")
        ),
        SetState::Open => format!("{what}: {}", all.join(", ")),
    }
}

/// A runtime or compiler name (`__stack_chk_guard`, `__chkstk_darwin`,
/// `__stderrp`): reserved for the implementation, not the project's.
fn is_runtime_name(sym: &str) -> bool {
    sym.starts_with("__")
}

/// The outside symbols: the project's own first, the compiler's and
/// runtime's folded into a count.
fn outside_words(outside: &[String]) -> String {
    let (runtime, own): (Vec<String>, Vec<String>) =
        outside.iter().cloned().partition(|s| is_runtime_name(s));
    let mut s = capped(&own);
    if !runtime.is_empty() {
        let n = runtime.len();
        let names = if n == 1 { "name" } else { "names" };
        s = if own.is_empty() {
            format!("{n} compiler or runtime {names}")
        } else {
            format!("{s}, and {n} compiler or runtime {names}")
        };
    }
    s
}

fn linked_words(l: &Option<LinkedRec>) -> String {
    match l {
        None => "not link-checked".into(),
        Some(LinkedRec::Ok(_)) => "linked".into(),
        Some(LinkedRec::Failed {
            missing,
            doubled,
            not_checked,
            not_compiled,
        }) => {
            let mut s = "did not link".to_string();
            if !not_compiled.is_empty() {
                s.push_str(&format!(
                    "; a file did not compile for the link: {}",
                    capped(not_compiled)
                ));
            }
            if !missing.is_empty() {
                s.push_str(&format!("; missing {}", capped(missing)));
            }
            if !doubled.is_empty() {
                s.push_str(&format!("; defined twice {}", capped(doubled)));
            }
            if !not_checked.is_empty() {
                s.push_str(&format!(
                    "; not checked (the probe budget ran out): {}",
                    capped(not_checked)
                ));
            }
            if missing.is_empty()
                && doubled.is_empty()
                && not_compiled.is_empty()
                && not_checked.is_empty()
            {
                s.push_str(
                    ", for a reason the map does not read (the linker's own words are never kept)",
                );
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
    for t in &c.included_as_text {
        out(format!(
            "      {} is included as text by {}: it is compiled inside them, so it is no unit \
             of its own",
            safe_line(&t.file),
            capped(&t.by)
        ));
    }
    out(format!(
        "      outside symbols: {}; guessed libraries: {}",
        outside_words(&c.outside),
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
        let state = if c.questions.contains(&d.set) {
            SetState::Held
        } else if matches!(c.linked, Some(LinkedRec::Failed { .. })) {
            SetState::NoneLinks
        } else {
            SetState::Open
        };
        out(format!("      {}", duplicate_line(d, state)));
    }
    for s in &c.strong_over_weak {
        out(format!(
            "      {} is defined weakly in {} and strongly in {}: the strong one defines it, so \
             the closure holds it",
            safe_line(&s.sym),
            capped(&s.weak),
            capped(&s.strong)
        ));
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
                        not_checked: None,
                        not_compiled: None,
                    },
                    LinkedRec::Failed {
                        missing,
                        doubled,
                        not_checked,
                        not_compiled,
                    } => LinkEvent {
                        k: "project-link",
                        id: &p.id,
                        ok: None,
                        missing: Some(missing),
                        doubled: Some(doubled),
                        not_checked: (!not_checked.is_empty()).then_some(not_checked.as_slice()),
                        not_compiled: (!not_compiled.is_empty()).then_some(not_compiled.as_slice()),
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

    let fuzzers = file.programs.iter().filter(|p| p.kind == "fuzz").count();
    let drivers = file.programs.iter().filter(|p| p.kind == "driver").count();
    let mut kinds = Vec::new();
    if fuzzers > 0 {
        kinds.push(format!(
            "{fuzzers} fuzzer{}",
            if fuzzers == 1 { "" } else { "s" }
        ));
    }
    if drivers > 0 {
        kinds.push(format!(
            "{drivers} driver{}",
            if drivers == 1 { "" } else { "s" }
        ));
    }
    out(format!(
        "programs: {}{}",
        file.programs.len(),
        if kinds.is_empty() {
            String::new()
        } else {
            format!(" ({})", kinds.join(", "))
        }
    ));
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
        // A `.c` another file includes as text: warned before it is offered.
        for f in file.files.iter().filter(|f| l.files.contains(&f.path)) {
            if !f.included_by.is_empty() {
                out(format!(
                    "  warning: {} is included as text by {}: moving it to Rust leaves them \
                     compiling its C text, so it is no library of its own",
                    safe_line(&f.path),
                    capped(&f.included_by)
                ));
            }
        }
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
