//! `harness features map` (docs/FEATURES-DESIGN.md §5): every scenario run
//! on the plain C program and on a probed copy in which each watched
//! function notes, the first time it runs, that it ran — plain, probed,
//! plain — and the notes read back into `map.json`'s records. Nothing here
//! gates anything; the map only says which functions each scenario ran.

use crate::confine::{Collected, Confinement, ScenarioEnd, ScenarioRun};
use crate::exec::{self, Runner};
use crate::features::place;
use crate::sandbox::{self, HostDirs, ProfileSpec};
use crate::scrub::Scrubber;
use crate::{
    cc_compile, extra_link_args, inside, irregular_c_file, program_c_files_in, sandbox_mode, Base,
    CcInvocation,
};
use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::features::{self, FeatureMap, Features, MapInputs, ScenarioRecord};
use harness_core::ledger::Ledger;
use harness_core::walk;
use harness_core::Facts;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The runtime's declarations, `-include`d into every translation unit.
const FNPROBE_H: &str = include_str!("fnprobe/fnprobe.h");
/// The runtime.
const FNPROBE_C: &str = include_str!("fnprobe/fnprobe.c");
/// The file a probed run leaves its notes in, in its temp dir.
const NOTES_FILE: &str = "ruharness-fnprobe";
/// The mirror's bounds (§5.3).
const MIRROR_MAX_FILES: usize = 20_000;
const MIRROR_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// The scratch dir's name under the ledger build dir (never a unit id).
pub const FEATURES_BUILD_DIR: &str = ".features";

/// What `map_features` reports while it works.
pub trait MapProgress {
    /// A line in words ("Building the C program…").
    fn message(&mut self, text: &str);
    /// A scenario's record, `n` of `of` (1-based).
    fn scenario(&mut self, record: &ScenarioRecord, n: usize, of: usize);
}

/// Map `features` (validated, with their `digest`) on the target's program
/// (§5.1). The caller has checked the refusals of §5.1 that need no build;
/// a build that fails is an error naming what failed. Returns the map; the
/// caller writes it.
pub fn map_features(
    target: &TargetContext,
    facts: &Facts,
    features: &Features,
    digest: &str,
    progress: &mut dyn MapProgress,
) -> Result<FeatureMap, Error> {
    let scrubber = Scrubber::from_env(&target.root);
    map_inner(target, facts, features, digest, progress).map_err(|e| scrubber.scrub_error(e))
}

fn map_inner(
    target: &TargetContext,
    facts: &Facts,
    features: &Features,
    digest: &str,
    progress: &mut dyn MapProgress,
) -> Result<FeatureMap, Error> {
    // What the map describes, before anything is built (review M10): a C
    // edit while the scenarios run makes the map out of date, never current
    // for a program it did not run.
    let inputs = MapInputs {
        facts: features::facts_digest(facts)?,
        features: digest.to_string(),
        program: features::program_digest_now(target, facts),
        platform: features::platform(),
    };
    let link_args = extra_link_args(target)?;
    let base = Base::resolve(target, FEATURES_BUILD_DIR, &["cc"])?;
    let root = base.root.clone();
    // The facts name files by the configured `source_dir`, the mirror by the
    // path under the canonical one: a `source_dir` reached through a link or
    // spelled in another case would probe nothing (fix check 4 F5).
    let configured = Path::new(&target.config.target.source_dir);
    let configured = configured
        .strip_prefix(&target.root)
        .or_else(|_| configured.strip_prefix(&root))
        .unwrap_or(configured);
    let spelled: PathBuf = configured
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect();
    let canonical = base
        .source_dir
        .strip_prefix(&root)
        .unwrap_or(&base.source_dir);
    if spelled != canonical {
        return Err(Error::Invariant(format!(
            "source_dir = {:?} resolves to {} (through a link, or in another case): the \
             features map finds functions by the path the scan recorded — set source_dir to \
             {:?}",
            target.config.target.source_dir,
            canonical.display(),
            canonical.display().to_string()
        )));
    }
    let build = scratch_dir(&root)?;

    let host = match sandbox_mode() {
        "sandbox-exec" => Some(HostDirs::from_env()?),
        _ => None,
    };
    let tool_profile = match &host {
        Some(host) => Some(sandbox::render_profile(&ProfileSpec {
            host,
            target_root: &root,
            toolchain: true,
            write_dirs: std::slice::from_ref(&build),
            write_files: &[],
        })?),
        None => None,
    };
    let runner = Runner {
        cwd: root.clone(),
        allowlist: base.allowlist.clone(),
        timeout: base.timeout,
        max_output: exec::DEFAULT_MAX_OUTPUT,
        tool_profile,
    };
    let confined = Confinement {
        runner: &runner,
        host: host.as_ref(),
        target_root: &root,
    };

    let c_files = program_c_files_in(&base, FEATURES_BUILD_DIR)?;
    // Refused by name before any build: a `.c` that is not a regular file
    // (a FIFO would hang the build until the timeout), and one linked out of
    // `source_dir` — the copy holds `source_dir` only, so the include words
    // below would name the wrong cause.
    if let Some(odd) = c_files
        .iter()
        .find(|c| c.to_string_lossy().chars().any(char::is_control))
    {
        return Err(Error::Invariant(format!(
            "{:?} has a control character in its name: the compiler's lists of what the program \
             reads could not be read back, so the features map cannot map it",
            shown(odd, &root)
        )));
    }
    // `-fmacro-prefix-map` takes the first `=` as its separator (fix check 6
    // L4): the copy's `__FILE__` would be another string.
    if root.to_string_lossy().contains('=') {
        return Err(Error::Invariant(format!(
            "the target's folder {} has '=' in its path, which the scratch copy's compile \
             cannot map back: move the target to a folder without '='",
            root.display()
        )));
    }
    if let Some(odd) = irregular_c_file(&c_files) {
        return Err(Error::Invariant(format!(
            "{} is not a regular file, so the C program cannot be built from it",
            shown(odd, &root).display()
        )));
    }
    if let Some(out) = c_files.iter().find(|p| !p.starts_with(&base.source_dir)) {
        return Err(Error::Invariant(format!(
            "a .c file in source_dir links outside source_dir (to {}): the features map \
             copies only source_dir, so it cannot map this program",
            shown(out, &root).display()
        )));
    }

    // The probed copy: the mirror, the notes, the runtime.
    progress.message("Copying source_dir into a scratch copy that notes each function it runs…");
    let index = PairIndex::from_facts(facts);
    let mirror = build.join("mirror");
    let unwatched = write_mirror(&base, facts, &index, &mirror)?;
    let to_mirror = |p: &Path| -> Result<PathBuf, Error> {
        let rel = p.strip_prefix(&root).map_err(|_| {
            Error::Invariant(format!("{} is not under the target root", p.display()))
        })?;
        Ok(mirror.join(rel))
    };
    let includes: Vec<PathBuf> = base
        .includes()
        .iter()
        .map(|d| to_mirror(d))
        .collect::<Result<_, _>>()?;
    let header = build.join("fnprobe.h");
    let runtime = build.join("fnprobe.c");
    write(&header, FNPROBE_H.as_bytes())?;
    write(&runtime, FNPROBE_C.as_bytes())?;
    let mut probed_inputs: Vec<PathBuf> = c_files
        .iter()
        .map(|p| to_mirror(p))
        .collect::<Result<_, _>>()?;
    // A `.c` the copy does not hold under its own path (one linked into
    // `migration/`, or reached first through a linked folder): refused by
    // name, never with include words (fix check 4 F6).
    if let Some((c, _)) = c_files
        .iter()
        .zip(&probed_inputs)
        .find(|(_, m)| !m.is_file())
    {
        return Err(Error::Invariant(format!(
            "{} is reached through a link the scratch copy does not hold: the features map \
             copies each file of source_dir once, so it cannot map this program",
            shown(c, &root).display()
        )));
    }
    // The mirror holds `source_dir` only (one path per file and folder): an
    // include that leaves it, or a folder linked into it, would fall through
    // in the copy to a system header of the same name — a different program,
    // mapped silently (review M7). The compiler says what each compile reads
    // (`-M`: every file, system headers too) and which headers it enters, in
    // order (`-H`); for each `.c` the program's files must be among the
    // copy's and the headers entered the same, one by one — a set over all
    // compiles missed a linked folder reached by both its names in two files
    // (fix check 5 M1), and a file linked to another is one file to
    // `#pragma once` but two in the copy (L1). Refused, named, before any
    // build.
    let cflags = vec![
        "-include".to_string(),
        path_string(&header)?,
        format!(
            "-fmacro-prefix-map={}={}",
            path_string(&mirror)?,
            path_string(&root)?
        ),
        format!("-DRUHARNESS_FNPROBE_N={}", index.len()),
    ];
    let copy = CopyPaths {
        build: &build,
        header: &header,
        mirror: &mirror,
        root: &root,
        source_dir: &base.source_dir,
    };
    for (c_file, copied) in c_files.iter().zip(&probed_inputs) {
        let program = reads(&runner, &base.includes(), &[], c_file, None)?
            .map_err(|why| build_failed("the C program does not build", why))?;
        let copied = match reads(&runner, &includes, &cflags, copied, Some(&copy))? {
            Ok(reads) => reads,
            Err(why) => {
                return Err(Error::Invariant(format!(
                    "the scratch copy cannot find what the program includes (an include \
                     outside source_dir, or a folder linked into it) — the features map copies \
                     only source_dir: {why}"
                )))
            }
        };
        // The same files, by the same names: one the program reads that the
        // copy would not, or one only the copy reads (a lookup that finds the
        // harness's own build folder from the mirror; fix check 6).
        let spelled = |read: &Read| shown(&read.0, &root).display().to_string();
        if let Some(missed) = program.files.difference(&copied.files).next() {
            return Err(Error::Invariant(format!(
                "the program reads {}, which the scratch copy would not (it is outside \
                 source_dir, in migration/ or .git/ there, or reached through a folder linked \
                 into it): the features map copies only source_dir, so it cannot map this \
                 program",
                spelled(missed)
            )));
        }
        if let Some(extra) = copied.files.difference(&program.files).next() {
            return Err(Error::Invariant(format!(
                "the scratch copy would read {}, which the program does not: the features map \
                 cannot map this program",
                spelled(extra)
            )));
        }
        if program.entered != copied.entered {
            let at = program
                .entered
                .iter()
                .zip(&copied.entered)
                .take_while(|(a, b)| a == b)
                .count();
            let name = |e: Option<&(usize, Read)>| {
                e.map_or("nothing more".to_string(), |(depth, read)| {
                    format!("{} (depth {depth})", spelled(read))
                })
            };
            return Err(Error::Invariant(format!(
                "compiling {}, the program's header #{} is {} and the scratch copy's would be {} \
                 (a file or folder of source_dir reached by two names, or an include outside \
                 it): the features map copies each file of source_dir once, so it cannot map \
                 this program",
                shown(c_file, &root).display(),
                at + 1,
                name(program.entered.get(at)),
                name(copied.entered.get(at)),
            )));
        }
    }

    // The plain program: the whole-program check's build of `whole_c`.
    progress.message("Building the C program…");
    let plain = build.join("plain");
    compile(&runner, &base.includes(), &[], &plain, &c_files, &link_args)
        .map_err(|e| build_failed("the C program does not build", e))?;
    progress.message("Building the scratch copy…");
    probed_inputs.push(runtime);
    let probed = build.join("probed");
    compile(
        &runner,
        &includes,
        &cflags,
        &probed,
        &probed_inputs,
        &link_args,
    )
    .map_err(|e| build_failed("the probed copy does not build", e))?;

    // The runs: plain, probed, plain — all at the one path of §4.1.
    let run_path = build.join("f").join(features::program_name(&target.config));
    let cap = 4 * (index.len().max(1) as u64) * 64;
    let of = features.scenarios.len();
    let mut records = Vec::with_capacity(of);
    for (i, scenario) in features.scenarios.iter().enumerate() {
        let argv = scenario.argv();
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let sample = scenario.input.map(|s| (s.file_name(), s.bytes()));
        let input = sample
            .as_ref()
            .map(|(name, bytes)| (*name, bytes.as_slice()));
        let run = |bin: &Path, collect: Option<(&str, u64)>| -> Result<ScenarioRun, Error> {
            place(bin, &run_path)?;
            confined.run_scenario(&run_path, &args, input, collect)
        };
        let first = run(&plain, None)?;
        let noted = run(&probed, Some((NOTES_FILE, cap)))?;
        let second = run(&plain, None)?;
        let record = record(scenario, &first, &noted, &second, &index);
        progress.scenario(&record, i + 1, of);
        records.push(record);
    }

    Ok(FeatureMap {
        schema: features::MAP_SCHEMA_NAME.to_string(),
        schema_version: features::MAP_SCHEMA_VERSION,
        inputs,
        unwatched,
        scenarios: records,
    })
}

/// `migration/build/.features/`, recreated, canonical and contained.
fn scratch_dir(root: &Path) -> Result<PathBuf, Error> {
    let build_root_raw = Ledger::new(root.to_path_buf()).build_dir();
    std::fs::create_dir_all(&build_root_raw).map_err(|e| Error::io(&build_root_raw, e))?;
    let build_root = inside(
        FEATURES_BUILD_DIR,
        "ledger build dir",
        &build_root_raw,
        root,
    )?;
    let raw = build_root.join(FEATURES_BUILD_DIR);
    match std::fs::symlink_metadata(&raw) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            std::fs::remove_file(&raw).map_err(|e| Error::io(&raw, e))?
        }
        Ok(_) => std::fs::remove_dir_all(&raw).map_err(|e| Error::io(&raw, e))?,
        Err(_) => {}
    }
    std::fs::create_dir(&raw).map_err(|e| Error::io(&raw, e))?;
    inside(FEATURES_BUILD_DIR, "features build dir", &raw, &build_root)
}

fn compile(
    runner: &Runner,
    includes: &[PathBuf],
    cflags: &[String],
    out: &Path,
    inputs: &[PathBuf],
    libs: &[String],
) -> Result<(), Error> {
    cc_compile(
        runner,
        &CcInvocation {
            includes,
            cflags,
            quiet: true,
            out,
            inputs,
            libs,
        },
    )
}

/// `path` relative to the target root when it is under it.
fn shown<'a>(path: &'a Path, root: &Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn build_failed(what: &str, e: Error) -> Error {
    match e {
        Error::Interrupted => Error::Interrupted,
        other => Error::Invariant(format!("{what}: {other}")),
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::io(path, e))
}

fn path_string(p: &Path) -> Result<String, Error> {
    p.to_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", p.display())))
}

/// The facts' distinct `(file, canonical id)` pairs, numbered in facts
/// order: a note's number is its pair's index (§5.3).
struct PairIndex {
    pairs: Vec<(String, String)>,
    by_pair: BTreeMap<(String, String), u32>,
}

impl PairIndex {
    fn from_facts(facts: &Facts) -> PairIndex {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut by_pair = BTreeMap::new();
        for s in &facts.symbols {
            let key = (s.file.clone(), s.name.clone());
            if !by_pair.contains_key(&key) {
                by_pair.insert(key.clone(), pairs.len() as u32);
                pairs.push(key);
            }
        }
        PairIndex { pairs, by_pair }
    }

    fn len(&self) -> usize {
        self.pairs.len()
    }

    fn of(&self, file: &str, id: &str) -> Option<u32> {
        self.by_pair
            .get(&(file.to_string(), id.to_string()))
            .copied()
    }
}

/// Where the scratch copy lives, for [`reads`].
struct CopyPaths<'a> {
    build: &'a Path,
    header: &'a Path,
    mirror: &'a Path,
    root: &'a Path,
    source_dir: &'a Path,
}

/// One file a compile reads: the path as the compiler spelled it (the
/// mirror's prefix put back to the root's) and the file it names.
type Read = (PathBuf, PathBuf);

/// What one compile reads (see [`reads`]).
struct Reads {
    /// Every file the compiler lists.
    files: std::collections::BTreeSet<Read>,
    /// The headers it enters, in order, each with its depth — the probe's
    /// header left out.
    entered: Vec<(usize, Read)>,
}

/// What a compile of `input` with `includes` and `cflags` reads: the files
/// the compiler lists (`cc -M`: system headers too — `-MM` drops a project
/// file included from a `system_header`; fix check 4 F1) and the headers it
/// enters, in order and at their depth (`-H`, on stderr). Each is the path as
/// the compiler spelled it and the file it names: the spelling is what a
/// lookup through a folder linked into `source_dir` differs by — the copy
/// holds one path per folder, so `__has_include("sys/x.h")` can be true in
/// the program and false in the copy while both read `x.h` (fix check 6 M2).
/// With `copy`, a path in the mirror is named as the target's at the same
/// place (the file then canonical, as the program's own build resolves it),
/// and a file of `source_dir` read by its own path — the unprobed original,
/// through `#include __FILE__` under the copy's `-fmacro-prefix-map` (fix
/// check 4 F2) — is refused. The compiler runs with the build's own flags
/// (`-O2` defines `__OPTIMIZE__`: an `#ifdef` on it picks the same branch;
/// fix check 3 N9) less `-o`. `Ok(Err(why))` when the compiler could not
/// list them (its words from a run without `-H`, whose header lines would
/// bury them; fix check 6 L2), or named a file that does not resolve.
fn reads(
    runner: &Runner,
    includes: &[PathBuf],
    cflags: &[String],
    input: &Path,
    copy: Option<&CopyPaths<'_>>,
) -> Result<Result<Reads, Error>, Error> {
    let unused = Path::new("-");
    let mut listing = cflags.to_vec();
    listing.push("-M".to_string());
    listing.push("-H".to_string());
    let mut argv = crate::cc_argv(&CcInvocation {
        includes,
        cflags: &listing,
        quiet: true,
        out: unused,
        inputs: std::slice::from_ref(&input.to_path_buf()),
        libs: &[],
    })?;
    if let Some(at) = argv.iter().position(|a| a == "-o") {
        argv.drain(at..at + 2);
    }
    let (rules, headers) = match runner.tool_outcome_both(&argv)? {
        Ok(out) => out,
        Err(why) => {
            argv.retain(|a| a != "-H");
            let why = match runner.tool_outcome(&argv)? {
                Err(plain) => plain,
                Ok(_) => why,
            };
            return Ok(Err(Error::Invariant(why)));
        }
    };
    let canonical_of = |p: &Path| p.canonicalize().map_err(|e| Error::io(p, e));
    let copy = match copy {
        Some(c) => Some((
            canonical_of(c.build)?,
            canonical_of(c.header)?,
            c.mirror,
            canonical_of(c.mirror)?,
            c.root,
            c.source_dir,
        )),
        None => None,
    };
    // `Ok(None)`: the probe's header, left out.
    let name = |token: &str| -> Result<Result<Option<Read>, Error>, Error> {
        let spelled = runner.cwd.join(token);
        // A file the compiler read resolves; one that does not is a list
        // misread, never a file to skip (fix check 3 N8).
        let Ok(canonical) = spelled.canonicalize() else {
            return Ok(Err(Error::Invariant(format!(
                "the compiler's list names {token:?}, which does not resolve"
            ))));
        };
        let Some((build, header, mirror, mirror_canonical, root, source_dir)) = &copy else {
            return Ok(Ok(Some((spelled, canonical))));
        };
        if canonical == *header {
            return Ok(Ok(None));
        }
        let spelled = match spelled.strip_prefix(mirror) {
            Ok(rel) => root.join(rel),
            Err(_) => spelled,
        };
        let file = match canonical.strip_prefix(mirror_canonical) {
            Ok(rel) => root
                .join(rel)
                .canonicalize()
                .unwrap_or_else(|_| root.join(rel)),
            Err(_) if canonical.starts_with(source_dir) && !canonical.starts_with(build) => {
                return Err(Error::Invariant(format!(
                    "the scratch copy would read {} itself, not its copy (an include of \
                     __FILE__, or an absolute include), so its functions would run unwatched: \
                     the features map cannot map this program",
                    canonical.strip_prefix(root).unwrap_or(&canonical).display()
                )));
            }
            Err(_) => canonical,
        };
        Ok(Ok(Some((spelled, file))))
    };
    let mut files = std::collections::BTreeSet::new();
    for token in make_prerequisites(&String::from_utf8_lossy(&rules)) {
        match name(&token)? {
            Ok(Some(read)) => {
                files.insert(read);
            }
            Ok(None) => {}
            Err(why) => return Ok(Err(why)),
        }
    }
    // `-H`: one line per header entered, its depth in dots, a space, its
    // path; any other line (a gcc trailer) is not one.
    let mut entered = Vec::new();
    for line in String::from_utf8_lossy(&headers).lines() {
        let rest = line.trim_start_matches('.');
        let depth = line.len() - rest.len();
        let Some(path) = rest.strip_prefix(' ').filter(|_| depth > 0) else {
            continue;
        };
        match name(path)? {
            Ok(Some(read)) => entered.push((depth, read)),
            Ok(None) => {}
            Err(why) => return Ok(Err(why)),
        }
    }
    Ok(Ok(Reads { files, entered }))
}

/// The prerequisites of make rules (`a.o: a.c a\ b.h \` continued lines):
/// every word after a rule's first `:`, with make's escapes read — `\ ` a
/// space, `\#` a `#`, `$$` a `$` (fix check 3 N8: a folder with a space).
/// Only a line that starts a rule (after a newline that is no continuation)
/// has targets: a file named `o:` is a prerequisite (fix check 4 F3).
fn make_prerequisites(rules: &str) -> Vec<String> {
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut word = String::new();
    let mut targets = true;
    let mut chars = rules.chars().peekable();
    let flush = |word: &mut String, words: &mut Vec<(String, bool)>, target: bool| {
        if !word.is_empty() {
            words.push((std::mem::take(word), target));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some('\n') => {
                    chars.next();
                    flush(&mut word, &mut words, false);
                }
                Some(&next @ (' ' | '#' | '\\')) => {
                    chars.next();
                    word.push(next);
                }
                _ => word.push('\\'),
            },
            '$' if chars.peek() == Some(&'$') => {
                chars.next();
                word.push('$');
            }
            ':' if targets && matches!(chars.peek(), Some(' ' | '\n' | '\t') | None) => {
                flush(&mut word, &mut words, true);
                targets = false;
            }
            '\n' => {
                flush(&mut word, &mut words, targets);
                targets = true;
            }
            c if c.is_whitespace() => flush(&mut word, &mut words, targets),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut words, false);
    words
        .into_iter()
        .filter(|(_, target)| !target)
        .map(|(w, _)| w)
        .collect()
}

/// Copy every regular file under `source_dir` (not `migration/` or `.git/`
/// when it is the target root) into `mirror` at its repo-relative path, each
/// file the facts give functions probed. Returns the unwatched pairs.
fn write_mirror(
    base: &Base,
    facts: &Facts,
    index: &PairIndex,
    mirror: &Path,
) -> Result<Vec<(String, String)>, Error> {
    let skipped_dirs = [base.root.join("migration"), base.root.join(".git")];
    let walked = walk::confined_except(
        &base.source_dir,
        walk::ALL_FILES,
        walk::Limits {
            max_files: Some(MIRROR_MAX_FILES),
            max_depth: None,
        },
        &skipped_dirs,
    );
    if let Some((path, why)) = walked.errors.first() {
        return Err(Error::io(path, std::io::Error::other(why.clone())));
    }
    if walked.truncated {
        return Err(Error::Invariant(format!(
            "the source dir holds more than {MIRROR_MAX_FILES} files; the features map does not \
             copy that many"
        )));
    }
    let with_functions: std::collections::BTreeSet<&str> =
        facts.symbols.iter().map(|s| s.file.as_str()).collect();
    let mut total: u64 = 0;
    let mut unwatched: Vec<(String, String)> = Vec::new();
    for path in walked.files {
        if skipped_dirs.iter().any(|d| path.starts_with(d)) {
            continue;
        }
        let rel_path = path.strip_prefix(&base.root).map_err(|_| {
            Error::Invariant(format!("{} is not under the target root", path.display()))
        })?;
        let rel = rel_path
            .to_str()
            .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", rel_path.display())))?;
        // The compiler writes a newline or a tab in a name as is, so its
        // lists could not be read back (fix check 5 L2): such a file stays
        // out of the copy — a Finder `Icon\r` is no reason to refuse (fix
        // check 6 L1); one the program reads makes the copy's listing
        // differ, and the map refuses then.
        if rel.chars().any(char::is_control) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        total += bytes.len() as u64;
        if total > MIRROR_MAX_BYTES {
            return Err(Error::Invariant(format!(
                "the source dir holds more than {} MiB; the features map does not copy that much",
                MIRROR_MAX_BYTES / (1024 * 1024)
            )));
        }
        let out = if with_functions.contains(rel) {
            let probed = harness_scan::probe_source(rel, &bytes, &|id| index.of(rel, id))?;
            unwatched.extend(probed.unwatched.into_iter().map(|id| (rel.to_string(), id)));
            probed.source
        } else {
            bytes
        };
        write(&mirror.join(rel_path), &out)?;
    }
    unwatched.sort();
    unwatched.dedup();
    Ok(unwatched)
}

/// How a run ended, as the map says it.
fn end_words(end: &ScenarioEnd) -> String {
    match end {
        ScenarioEnd::Exited(code) => format!("exit {code}"),
        ScenarioEnd::Signaled(n) => format!("signal {n}"),
        ScenarioEnd::TimedOut => "timed out".into(),
        ScenarioEnd::Overflow => "too much output".into(),
        ScenarioEnd::ExecFailed => "could not start".into(),
    }
}

/// A scenario's record from its three runs (§5.2).
fn record(
    scenario: &features::Scenario,
    first: &ScenarioRun,
    noted: &ScenarioRun,
    second: &ScenarioRun,
    index: &PairIndex,
) -> ScenarioRecord {
    let same = |a: &ScenarioRun, b: &ScenarioRun| a.same_result(b);
    let (noted_word, reason, functions) = match &noted.collected {
        // Opened, never written: the notes were lost (review M3).
        Some(Collected::Bytes(bytes)) if bytes.is_empty() => {
            ("unavailable", Some("none written"), Vec::new())
        }
        Some(Collected::Bytes(bytes)) => match decode_notes(bytes, index) {
            Some(functions) => ("complete", None, functions),
            None => ("unavailable", Some("unreadable"), Vec::new()),
        },
        Some(Collected::Missing) => ("unavailable", Some("none written"), Vec::new()),
        Some(Collected::Invalid) | None => ("unavailable", Some("unreadable"), Vec::new()),
    };
    ScenarioRecord {
        feature: scenario.feature.clone(),
        scenario: scenario.id.clone(),
        end: end_words(&first.end),
        stdout_bytes: crate::confine::shown_len(&first.stdout) as u64,
        stderr_bytes: crate::confine::shown_len(&first.stderr) as u64,
        stderr_head: features::stderr_head(&first.stderr),
        stable: same(first, second),
        probe_agrees: same(first, noted),
        noted: noted_word.to_string(),
        reason: reason.map(str::to_string),
        functions,
    }
}

/// The notes as `[file, id]` pairs, sorted, each once — `None` when they are
/// not whole 4-byte records of known ids.
fn decode_notes(bytes: &[u8], index: &PairIndex) -> Option<Vec<(String, String)>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut ids = std::collections::BTreeSet::new();
    for chunk in bytes.chunks_exact(4) {
        let id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
        if id >= index.len() {
            return None;
        }
        ids.insert(id);
    }
    let mut pairs: Vec<(String, String)> =
        ids.into_iter().map(|i| index.pairs[i].clone()).collect();
    pairs.sort();
    Some(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::facts::SymbolRecord;

    #[test]
    fn make_rules_read_as_their_prerequisites() {
        let rules = "a.o: src/a.c src/a.h \\\n  src/sub/b.h\nb.o: src/b.c\n";
        assert_eq!(
            make_prerequisites(rules),
            ["src/a.c", "src/a.h", "src/sub/b.h", "src/b.c"]
        );
        // Fix check 3 N8: make's escapes.
        let rules = "a.o: /t/sp\\ ace/a.c /t/sp\\ ace/x\\#1.h /t/d$$/y.h\n";
        assert_eq!(
            make_prerequisites(rules),
            ["/t/sp ace/a.c", "/t/sp ace/x#1.h", "/t/d$/y.h"]
        );
        // Fix check 4 F3: only a rule's first `:` ends its targets.
        let rules = "a.o: /t/a.c /t/o: \\\n  /t/y.h\nb.o c.o: /t/b.c\n";
        assert_eq!(
            make_prerequisites(rules),
            ["/t/a.c", "/t/o:", "/t/y.h", "/t/b.c"]
        );
    }

    fn facts() -> Facts {
        let sym = |file: &str, name: &str| SymbolRecord {
            name: name.into(),
            kind: "function".into(),
            file: file.into(),
            visibility: "public".into(),
            signature: String::new(),
            span: (1, 1),
        };
        Facts {
            symbols: vec![
                sym("src/a.c", "main"),
                sym("src/a.c", "src/a.c::helper"),
                sym("src/a.c", "src/a.c::helper"),
                sym("src/b.c", "api"),
            ],
            ..Facts::default()
        }
    }

    #[test]
    fn ids_are_the_facts_distinct_pairs_in_order() {
        let index = PairIndex::from_facts(&facts());
        assert_eq!(index.len(), 3, "#if variants share one id");
        assert_eq!(index.of("src/a.c", "main"), Some(0));
        assert_eq!(index.of("src/a.c", "src/a.c::helper"), Some(1));
        assert_eq!(index.of("src/b.c", "api"), Some(2));
        assert_eq!(index.of("src/b.c", "main"), None);
    }

    #[test]
    fn notes_decode_strictly() {
        let index = PairIndex::from_facts(&facts());
        let rec = |ids: &[u32]| {
            ids.iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        assert_eq!(
            decode_notes(&rec(&[2, 0, 2]), &index),
            Some(vec![
                ("src/a.c".into(), "main".into()),
                ("src/b.c".into(), "api".into())
            ]),
            "sorted, each once (a forked child writes its own)"
        );
        assert_eq!(decode_notes(&[], &index), Some(vec![]));
        assert_eq!(decode_notes(&rec(&[3]), &index), None, "an id out of range");
        assert_eq!(decode_notes(&[0, 0, 0], &index), None, "a torn record");
    }

    #[test]
    fn ends_read_as_the_map_writes_them() {
        assert_eq!(end_words(&ScenarioEnd::Exited(0)), "exit 0");
        assert_eq!(end_words(&ScenarioEnd::Exited(-1)), "exit -1");
        assert_eq!(end_words(&ScenarioEnd::Signaled(6)), "signal 6");
        assert_eq!(end_words(&ScenarioEnd::TimedOut), "timed out");
        assert_eq!(end_words(&ScenarioEnd::Overflow), "too much output");
        assert_eq!(end_words(&ScenarioEnd::ExecFailed), "could not start");
    }
}
