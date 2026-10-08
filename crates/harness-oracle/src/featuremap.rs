//! `harness features map` (docs/FEATURES-DESIGN.md §5): every scenario run
//! on the plain C program and on a probed copy in which each watched
//! function notes, the first time it runs, that it ran — plain, probed,
//! plain — and the notes read back into `map.json`'s records. Nothing here
//! gates anything; the map only says which functions each scenario ran.

use crate::confine::{Collected, Confinement, NotesFile, ScenarioEnd, ScenarioRun};
use crate::exec::Runner;
use crate::features::place;
use crate::probebuild::{Build, Cc};
use crate::probecopy::{self, scan_text, Kind, Probe, Reason};
use crate::sandbox::{self, HostDirs, ProfileSpec};
use crate::scrub::Scrubber;
use crate::{
    cc_compile, extra_link_args, inside, irregular_c_file, program_c_files_in, project_path,
    sandbox_mode, Base, CcInvocation, FileArgs, Layout,
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
        probe: features::MAP_PROBE.to_string(),
    };
    let link_args = extra_link_args(target)?;
    let base = Base::resolve(target, FEATURES_BUILD_DIR, &["cc"])?;
    let root = base.root.clone();
    // The facts name files by the configured `source_dir`, the mirror by the
    // path under the canonical one: a `source_dir` reached through a link or
    // spelled in another case would probe nothing (fix check 4 F5). A file
    // list's files are checked one by one where the mirror copies them.
    if let (Some(source_dir), Some(base_source_dir)) =
        (target.config.target.source_dir(), base.source_dir())
    {
        let configured = Path::new(source_dir);
        let configured = configured
            .strip_prefix(&target.root)
            .or_else(|_| configured.strip_prefix(&root))
            .unwrap_or(configured);
        let spelled: PathBuf = configured
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect();
        let canonical = base_source_dir
            .strip_prefix(&root)
            .unwrap_or(base_source_dir);
        if spelled != canonical {
            return Err(Error::Invariant(format!(
                "source_dir = {:?} resolves to {} (through a link, or in another case): the \
                 features map finds functions by the path the scan recorded — set source_dir \
                 to {:?}",
                source_dir,
                canonical.display(),
                canonical.display().to_string()
            )));
        }
    }
    let build = scratch_dir(&Ledger::of_under(target, &root))?;
    // Everything made after the mirror goes to a fresh folder outside the
    // target, out of reach of the copy's -I folders; removed on every way out
    // (docs/FEATURES-PROBE-REDESIGN.md §3.4 "Order").
    let out = MapOut::create()?;
    // The compiler driver's own temporaries (one object per `.c` of a
    // compile-and-link, gcc's `.s` files) go in the random folder too, so
    // they leave with it on every way out, a signal included (fix pass 2's
    // check: the plain build's objects stayed in $TMPDIR, mode 0644).
    let tool_tmp = out.path().join("tmp");
    {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&tool_tmp)
            .map_err(|e| Error::io(&tool_tmp, e))?;
    }
    let write_dirs = vec![build.clone(), out.path().to_path_buf()];

    let host = match sandbox_mode() {
        "sandbox-exec" => Some(HostDirs::from_env()?),
        _ => None,
    };
    let tool_profile = match &host {
        Some(host) => Some(sandbox::render_profile(&ProfileSpec {
            host,
            target_root: &root,
            toolchain: true,
            write_dirs: &write_dirs,
            write_files: &[],
        })?),
        None => None,
    };
    let runner = Runner {
        tool_tmpdir: Some(tool_tmp),
        ..Runner::new(&root, base.allowlist.clone(), base.timeout, tool_profile)?
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
    if let Some(source_dir) = base.source_dir() {
        if let Some(out) = c_files.iter().find(|p| !p.starts_with(source_dir)) {
            return Err(Error::Invariant(format!(
                "a .c file in source_dir links outside source_dir (to {}): the features map \
                 copies only source_dir, so it cannot map this program",
                shown(out, &root).display()
            )));
        }
    }

    let cc = Cc::detect(&runner)?;
    let gcc = matches!(cc, Cc::Gcc(_));

    // The probed copy: the mirror, the notes, the runtime.
    progress.message(match base.source_dir() {
        Some(_) => "Copying source_dir into a scratch copy that notes each function it runs…",
        None => "Copying the listed files into a scratch copy that notes each function it runs…",
    });
    let index = PairIndex::from_facts(facts);
    let mirror = build.join("mirror");
    let mut probe = Probe::new(gcc);
    let times = write_mirror(&base, facts, &index, &mirror, &mut probe)?;
    // What the scratch copy holds, in the refusals below.
    let copied_what = match base.source_dir() {
        Some(_) => "source_dir",
        None => "the listed files and the headers they reach",
    };
    let index_of = |rel: &str| {
        let rel = rel.to_string();
        let index = &index;
        move |id: &str| index.of(&rel, id)
    };
    let to_mirror = |p: &Path| -> Result<PathBuf, Error> {
        let rel = p.strip_prefix(&root).map_err(|_| {
            Error::Invariant(format!("{} is not under the target root", p.display()))
        })?;
        Ok(mirror.join(rel))
    };
    // Each file's own arguments, for the program and for its copy (every
    // folder and every configuration path moved into the mirror).
    let program_args: Vec<FileArgs> = c_files
        .iter()
        .map(|c| base.file_args(c))
        .collect::<Result<_, _>>()?;
    let copy_flags = base.flags_mapped(&|p| to_mirror(p))?;
    let copy_args: Vec<FileArgs> = program_args
        .iter()
        .map(|a| -> Result<FileArgs, Error> {
            Ok(FileArgs {
                flags: copy_flags.clone(),
                includes: a
                    .includes
                    .iter()
                    .map(|d| to_mirror(d))
                    .collect::<Result<_, _>>()?,
            })
        })
        .collect::<Result<_, _>>()?;
    let header = out.path().join("fnprobe.h");
    let runtime_src = out.path().join("fnprobe.c");
    // Into the folder as made, never a folder made again: after a signal's
    // cleanup removed it, these fail instead of leaving it behind.
    for (path, text) in [(&header, FNPROBE_H), (&runtime_src, FNPROBE_C)] {
        std::fs::write(path, text.as_bytes()).map_err(|e| Error::io(path, e))?;
    }
    let header = header.canonicalize().map_err(|e| Error::io(&header, e))?;
    let probed_inputs: Vec<PathBuf> = c_files
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
             copies each file of {copied_what} once, so it cannot map this program",
            shown(c, &root).display()
        )));
    }
    let mirror_canonical = mirror.canonicalize().map_err(|e| Error::io(&mirror, e))?;
    let rel_of_mirror = |canonical: &Path| -> Option<String> {
        canonical
            .strip_prefix(&mirror_canonical)
            .ok()
            .and_then(|r| r.to_str())
            .map(str::to_string)
    };
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
    // build. The same runs give the preprocessed text for §3.2–§3.3.
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
    let root_for_notes = root.clone();
    let probe_rels: std::collections::BTreeSet<String> = probe.rels().into_iter().collect();
    let has_notes = move |canonical: &Path| -> bool {
        canonical
            .strip_prefix(&root_for_notes)
            .ok()
            .and_then(|r| r.to_str())
            .is_some_and(|rel| probe_rels.contains(rel))
    };
    let copy = CopyPaths {
        has_notes: &has_notes,
        build: &build,
        header: &header,
        mirror: &mirror,
        root: &root,
        source_dir: base.source_dir().unwrap_or(&root),
    };
    let listing_out = |side: &str, n: usize| {
        (
            out.path().join(format!("{side}-{n}.i")),
            out.path().join(format!("{side}-{n}.d")),
        )
    };
    let mut programs: Vec<Reads> = Vec::with_capacity(c_files.len());
    for (n, c_file) in c_files.iter().enumerate() {
        let (i, d) = listing_out("program", n);
        let program =
            reads(&runner, &program_args[n], &[], c_file, (&i, &d), None)?.map_err(|why| {
                // A list the compiler printed but that cannot be read back
                // is not a build failure (check 7).
                if why.to_string().contains("the compiler's list names") {
                    Error::Invariant(format!(
                        "the compiler's list of what {} reads cannot be read back: {why}",
                        shown(c_file, &root).display()
                    ))
                } else {
                    build_failed("the C program does not build", why)
                }
            })?;
        // Its text stays on disk until §3.3 step 4 reads it back: one file's
        // text in memory at a time (review: every file's, twice, was 1.7 GB
        // for a 300-file program).
        programs.push(Reads {
            text: Vec::new(),
            ..program
        });
    }
    let mut unit_reads: Vec<std::collections::BTreeSet<String>> = Vec::new();
    // Per top-level file: the notes its preprocessed copy holds as code —
    // in the last listing pass, and in any pass (a note taken out later is
    // still a definition the unit compiles).
    let mut copy_notes: Vec<std::collections::BTreeSet<u32>> = Vec::new();
    let mut notes_ever: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); c_files.len()];
    // Per top-level file: the notes whose head, macros expanded, is
    // inline-only (`extern inline`), in any listing pass.
    let mut inline_ever: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); c_files.len()];
    // Per top-level file: each note's head words, macros expanded, in any
    // listing pass (the name its definition is compiled under is there).
    let mut heads_ever: Vec<std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>> =
        vec![std::collections::BTreeMap::new(); c_files.len()];
    for pass in 0.. {
        unit_reads.clear();
        copy_notes.clear();
        let mut changed = false;
        for (n, (c_file, copied)) in c_files.iter().zip(&probed_inputs).enumerate() {
            let program = &programs[n];
            let (ci, cd) = listing_out("copy", n);
            let copied_reads = match reads(
                &runner,
                &copy_args[n],
                &cflags,
                copied,
                (&ci, &cd),
                Some(&copy),
            )? {
                Ok(reads) => reads,
                Err(why) => {
                    return Err(Error::Invariant(format!(
                        "the scratch copy cannot find what the program includes (an include \
                         outside {copied_what}, or a folder or file linked into it) — the features \
                         map copies only {copied_what}: {why}"
                    )))
                }
            };
            compare_reads(c_file, program, &copied_reads, &root, copied_what)?;
            let own = copied.canonicalize().ok();
            // §3.3 step 1: a probed file listed and not entered is read as
            // data (`#embed`, `__has_embed`, `__has_include`).
            for listed in copied_reads
                .listed
                .difference(&copied_reads.entered_as_read)
            {
                if Some(listed) == own.as_ref() || *listed == header {
                    continue;
                }
                if let Some(rel) = rel_of_mirror(listed) {
                    let why = format!(
                        "listed but never included by {} (#embed, __has_embed or __has_include)",
                        shown(c_file, &root).display()
                    );
                    changed |= probe.unprobe(&rel, Reason::new(Kind::Data, &why));
                }
            }
            let scan = scan_text(&copied_reads.text);
            // §3.3 step 2: a note turned into text.
            for n in &scan.in_literals {
                if let Some((file, id)) = index.pairs.get(*n as usize) {
                    changed |= probe.take_out(file, id, Reason::new(Kind::Stringized, ""));
                }
            }
            // §3.2: a note in a branch the build skips.
            for n in scan.skipped_branches() {
                if let Some((file, id)) = index.pairs.get(n as usize) {
                    changed |= probe.take_out(file, id, Reason::new(Kind::SkippedBranch, ""));
                }
            }
            // §3.3 step 1: files `.incbin` reads.
            for name in &scan.incbins {
                let why = format!("read by .incbin in {}", shown(c_file, &root).display());
                let found = name.as_ref().and_then(|name| {
                    std::iter::once(root.join(name))
                        .chain(copy_args[n].includes.iter().map(|d| d.join(name)))
                        .find_map(|p| p.canonicalize().ok())
                });
                match (name, found) {
                    (Some(_), Some(path)) => {
                        if let Some(rel) = rel_of_mirror(&path) {
                            changed |= probe.unprobe(&rel, Reason::new(Kind::Data, &why));
                        }
                    }
                    _ => {
                        for rel in probe.rels() {
                            changed |= probe.unprobe(&rel, Reason::new(Kind::Data, &why));
                        }
                    }
                }
            }
            let mut entered: std::collections::BTreeSet<String> = copied_reads
                .entered_as_read
                .iter()
                .chain(own.iter())
                .filter_map(|p| rel_of_mirror(p))
                .collect();
            entered.retain(|rel| probe.rels().contains(rel));
            unit_reads.push(entered);
            notes_ever[n].extend(scan.notes.keys().copied());
            inline_ever[n].extend(scan.inline_notes.iter().copied());
            for (k, words) in scan.note_heads {
                heads_ever[n].entry(k).or_default().extend(words);
            }
            copy_notes.push(scan.notes.keys().copied().collect());
        }
        if !changed {
            break;
        }
        if pass == 2 {
            return Err(Error::Invariant(
                "the scratch copy keeps changing while it is checked — a fault in the harness, \
                 not your program; please report it"
                    .to_string(),
            ));
        }
        for rel in probe.rels() {
            probe.write(&mirror, &rel, &index_of(&rel), true)?;
        }
        keep_times(&times);
    }
    // §3.3 step 4: apart from its notes, the copy is the program's code —
    // each file's two texts read back from the random folder (the last
    // pass's listings), compared, then removed.
    for (n, c_file) in c_files.iter().enumerate() {
        let (program_i, _) = listing_out("program", n);
        let (copy_i, _) = listing_out("copy", n);
        let program_text = std::fs::read(&program_i).map_err(|e| Error::io(&program_i, e))?;
        let copy_text = std::fs::read(&copy_i).map_err(|e| Error::io(&copy_i, e))?;
        if let Err(line) = probecopy::same_code(&program_text, &copy_text, &header) {
            return Err(Error::Invariant(format!(
                "the scratch copy of {} is not the same program near: {line} — the features \
                 map cannot map this program",
                shown(c_file, &root).display()
            )));
        }
        let _ = std::fs::remove_file(&program_i);
        let _ = std::fs::remove_file(&copy_i);
    }
    // The compile copies: no end tokens.
    for rel in probe.rels() {
        probe.write(&mirror, &rel, &index_of(&rel), false)?;
    }
    keep_times(&times);

    // The plain program: the whole-program check's build of `whole_c`.
    progress.message("Building the C program…");
    let plain = out.path().join("plain");
    crate::perf::build::whole_cc_into(&base, &link_args, &runner, &plain, &c_files)
        .map_err(|e| build_failed("the C program does not build", e))?;
    progress.message("Building the scratch copy…");
    // The runtime: its own compile — no target include folder, no builtins
    // (its imports stay open, fstat, mmap, close) — linked first
    // (docs/FEATURES-PROBE-REDESIGN.md §3.5, §3.4 step 5).
    let runtime = out.path().join("fnprobe.o");
    cc_compile(
        &runner,
        &CcInvocation {
            args: &FileArgs::default(),
            cflags: &[
                "-c".to_string(),
                "-fno-builtin".to_string(),
                format!("-DRUHARNESS_FNPROBE_N={}", index.len()),
            ],
            quiet: true,
            out: &runtime,
            inputs: std::slice::from_ref(&runtime_src),
            libs: &[],
        },
    )
    .map_err(|e| build_failed("the probe's runtime does not build", e))?;
    let index_pair = |rel: &str, id: &str| index.of(rel, id);
    // The configuration's `-pthread` reaches the copy's link too.
    let copy_link_args: Vec<String> = base
        .link_file_args()
        .flags
        .into_iter()
        .chain(link_args.iter().cloned())
        .collect();
    let build_copy = Build {
        runner: &runner,
        cc,
        root: &root,
        mirror: &mirror,
        args: &copy_args,
        cflags: &cflags,
        units: &probed_inputs,
        reads: &unit_reads,
        runtime: &runtime,
        link_args: &copy_link_args,
        out: out.path(),
        index_of: &index_pair,
        functions: &index.pairs,
        times: &times,
        bounds: crate::probebuild::bounds(),
    };
    let built = build_copy.run(&mut probe, progress)?;
    let probed = built.program;
    // A watched function whose note no compile holds, in a file the
    // compiles enter, while a compiled object defines a function of its name
    // that no other watched definition compiled there explains: its visible
    // definition sits in a branch the build skips, and one the parser cannot
    // read (made by a macro, a parenthesized name) is compiled in its place —
    // unwatched, never "not run" (review: an #if sibling shares its id). A
    // compiler's clone of it (gcc's `f.isra.0`, `f.part.0`) is its name.
    let name_of = |id: &str| id.rsplit("::").next().unwrap_or(id).to_string();
    for (n, entered) in unit_reads.iter().enumerate() {
        for rel in entered {
            for (file, id) in index.pairs.iter().filter(|(file, _)| file == rel) {
                let Some(number) = index.of(file, id) else {
                    continue;
                };
                if probe.reasons.contains_key(&(file.clone(), id.clone())) || !probe.is_probed(file)
                {
                    continue;
                }
                let compiled = (0..unit_reads.len())
                    .any(|m| unit_reads[m].contains(rel) && copy_notes[m].contains(&number));
                let name = name_of(id);
                let external = !id.contains("::");
                let defined = defines_function(&built.defined[n], &name, external);
                // Another watched definition of the name, its note code in
                // one of this unit's listings and compiled under the name (its
                // head, macros expanded, holds it — fix pass 4's check: a
                // macro can rename it), explains the symbol: C allows one
                // definition of a name in a unit, so the hidden variant cannot
                // be there too — save GNU's inline-only idiom, so never an
                // `extern inline` one, however spelled, K&R heads too (fix
                // passes 3 and 4's checks). A definition kept unwatched by a
                // rule never explains: nothing shows whether the unit compiles
                // it.
                let explained = || {
                    index.pairs.iter().any(|(other_file, other)| {
                        (other_file, other) != (file, id)
                            && name_of(other) == name
                            && !probe.inline_only(other_file, other)
                            && index.of(other_file, other).is_some_and(|k| {
                                notes_ever[n].contains(&k)
                                    && !inline_ever[n].contains(&k)
                                    && heads_ever[n].get(&k).is_some_and(|h| h.contains(&name))
                            })
                    })
                };
                if !compiled && defined && !explained() {
                    probe.take_out(
                        file,
                        id,
                        Reason::new(
                            Kind::Parser,
                            "the one compiled is another definition, in another #if branch",
                        ),
                    );
                }
            }
        }
    }

    // The runs: plain, probed, plain — all at the one path of §4.1.
    let run_path = build.join("f").join(features::program_name(&target.config));
    let notes_len = index.len() as u64 + 1;
    let of = features.scenarios.len();
    let mut records = Vec::with_capacity(of);
    for (i, scenario) in features.scenarios.iter().enumerate() {
        let argv = scenario.argv();
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let sample = scenario.input.map(|s| (s.file_name(), s.bytes()));
        let input = sample
            .as_ref()
            .map(|(name, bytes)| (*name, bytes.as_slice()));
        let run = |bin: &Path, read: bool| -> Result<ScenarioRun, Error> {
            place(bin, &run_path)?;
            let notes = NotesFile {
                len: notes_len,
                read,
            };
            confined.run_scenario(&run_path, &args, input, Some(notes))
        };
        let first = run(&plain, false)?;
        let noted = run(&probed, true)?;
        let second = run(&plain, false)?;
        let record = record(scenario, &first, &noted, &second, &index);
        progress.scenario(&record, i + 1, of);
        records.push(record);
    }

    // The two programs stay in the build folder for a look afterwards; the
    // random folder goes with its guard.
    for (from, name) in [(&plain, "plain"), (&probed, "probed")] {
        let to = build.join(name);
        std::fs::copy(from, &to).map_err(|e| Error::io(&to, e))?;
    }
    Ok(FeatureMap {
        schema: features::MAP_SCHEMA_NAME.to_string(),
        schema_version: features::MAP_SCHEMA_VERSION,
        inputs,
        unwatched: probe.unwatched(),
        unwatched_reasons: probe
            .reasons
            .iter()
            .map(|((file, id), reason)| features::UnwatchedReason {
                file: file.clone(),
                id: id.clone(),
                kind: reason.kind.name().to_string(),
                detail: reason.detail.clone(),
            })
            .collect(),
        scenarios: records,
    })
}

/// The same files, by the same names, and the same headers entered in order
/// — the program's compile of `c_file` and the copy's (§5.3).
/// `copied_what` names what the copy holds (`source_dir`, or the listed
/// files and their headers).
fn compare_reads(
    c_file: &Path,
    program: &Reads,
    copied: &Reads,
    root: &Path,
    copied_what: &str,
) -> Result<(), Error> {
    // One the program reads that the copy would not, or one only the copy
    // reads (a lookup that finds the harness's own build folder from the
    // mirror; fix check 6).
    let spelled = |read: &Read| shown(&read.0, root).display().to_string();
    if let Some(missed) = program.files.difference(&copied.files).next() {
        return Err(Error::Invariant(format!(
            "the program reads {}, which the scratch copy would not (it is outside \
             {copied_what}, in migration/ or .git/ there, or reached through a folder linked \
             into it): the features map copies only {copied_what}, so it cannot map this \
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
            shown(c_file, root).display(),
            at + 1,
            name(program.entered.get(at)),
            name(copied.entered.get(at)),
        )));
    }
    Ok(())
}

/// Whether an object defines a function named `name` — code only (a
/// variable of the name is not one), external when `external`, a
/// compiler's clone (gcc's `f.isra.0`, `f.part.0`) counting as `f`.
fn defines_function(defined: &[crate::objsyms::Defined], name: &str, external: bool) -> bool {
    defined.iter().any(|d| {
        d.function && (d.external || !external) && crate::probebuild::function_name(&d.name) == name
    })
}

/// The folder a map makes everything in after the mirror
/// (`$TMPDIR/ruharness-map-<random>`), removed when dropped — on success,
/// refusal and error — and by [`remove_live_scratch_dirs`] when a signal
/// ends the harness (the process dies by it, so no drop runs).
struct MapOut(PathBuf);

/// The random folders of maps in progress, and every run's temp folder.
static LIVE_DIRS: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Remove every registered folder — the signal handler's part, after it has
/// killed the children that write there. Final, as the process registry is:
/// the registry stays locked for good, so no folder is made afterwards
/// while the process is on its way out. Returns how many it removed.
pub fn remove_live_scratch_dirs() -> usize {
    let live = LIVE_DIRS.lock().unwrap_or_else(|e| e.into_inner());
    for dir in live.iter() {
        let _ = std::fs::remove_dir_all(dir);
    }
    let n = live.len();
    std::mem::forget(live);
    n
}

/// Make a folder with `make` and register it for the signal's cleanup —
/// under the registry's lock, after a check for a signal, so the cleanup
/// either sees the folder or ran before it (fix pass 2's check: a run's temp
/// folder outlived a Ctrl-C).
pub(crate) fn make_live_dir(
    make: impl FnOnce() -> Result<PathBuf, Error>,
) -> Result<PathBuf, Error> {
    let mut live = LIVE_DIRS.lock().unwrap_or_else(|e| e.into_inner());
    if crate::exec::cancelled() {
        return Err(Error::Interrupted);
    }
    let dir = make()?;
    live.insert(dir.clone());
    Ok(dir)
}

/// Whether `dir` is registered for the signal's cleanup (tests).
#[cfg(test)]
pub(crate) fn is_live_dir(dir: &Path) -> bool {
    LIVE_DIRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(dir)
}

/// Remove a folder [`make_live_dir`] made, and forget it — after a signal
/// the registry stays locked for good (see [`remove_live_scratch_dirs`]):
/// nothing to forget.
pub(crate) fn drop_live_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    if crate::exec::cancelled() {
        return;
    }
    LIVE_DIRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(dir);
}

impl MapOut {
    fn create() -> Result<MapOut, Error> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base_raw = std::env::temp_dir();
        let base = base_raw
            .canonicalize()
            .map_err(|e| Error::io(&base_raw, e))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        make_live_dir(|| {
            for _ in 0..1000 {
                let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let tag = harness_core::hash::bytes_hash(
                    format!("{}-{nanos}-{n}", std::process::id()).as_bytes(),
                );
                let hex = tag
                    .strip_prefix(harness_core::hash::HASH_PREFIX)
                    .unwrap_or(&tag);
                let dir = base.join(format!("ruharness-map-{}", &hex[..16]));
                // Only the person can read it: it holds the whole preprocessed
                // program and its binaries (review: a shared /tmp).
                let made = {
                    let mut builder = std::fs::DirBuilder::new();
                    #[cfg(unix)]
                    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                    builder.create(&dir)
                };
                match made {
                    Ok(()) => return Ok(dir),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(Error::io(&dir, e)),
                }
            }
            Err(Error::Invariant(format!(
                "could not create a fresh folder for the features map under {}",
                base.display()
            )))
        })
        .map(MapOut)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for MapOut {
    fn drop(&mut self) {
        drop_live_dir(&self.0);
    }
}

/// Give each mirror file its original's modification time (for the build).
pub(crate) fn keep_file_times(times: &[(PathBuf, std::time::SystemTime)]) {
    keep_times(times);
}

/// A path as UTF-8 text (for an argv).
pub(crate) fn path_text(p: &Path) -> Result<String, Error> {
    path_string(p)
}

/// The ledger's `build/.features/`, recreated, canonical and contained.
fn scratch_dir(ledger: &Ledger) -> Result<PathBuf, Error> {
    let root = ledger.target_root();
    let build_root_raw = ledger.build_dir();
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

/// Write `bytes` to `path`, its folders made first.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    write(path, bytes)
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
    /// Whether a canonical file of the target is a probed file with notes.
    has_notes: &'a dyn Fn(&Path) -> bool,
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
    /// Every file listed, canonical as the compile read it (the copy's
    /// mirror paths kept) — for the files read as data (§3.3 step 1).
    listed: std::collections::BTreeSet<PathBuf>,
    /// Every header entered, canonical as the compile read it.
    entered_as_read: std::collections::BTreeSet<PathBuf>,
    /// The preprocessed text.
    text: Vec<u8>,
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
    args: &FileArgs,
    cflags: &[String],
    input: &Path,
    out: (&Path, &Path),
    copy: Option<&CopyPaths<'_>>,
) -> Result<Result<Reads, Error>, Error> {
    let (text_path, deps_path) = out;
    let mut listing = cflags.to_vec();
    listing.extend([
        "-E".to_string(),
        "-MD".to_string(),
        "-MF".to_string(),
        path_string(deps_path)?,
        "-H".to_string(),
    ]);
    let mut argv = crate::cc_argv(&CcInvocation {
        args,
        cflags: &listing,
        quiet: true,
        out: text_path,
        inputs: std::slice::from_ref(&input.to_path_buf()),
        libs: &[],
    })?;
    let run = runner.tool_run(&argv)?;
    let headers = match run.end {
        crate::exec::ChildEnd::Exited(status) if status.success() => run.stderr,
        // No re-run after a timeout or an overflow (check 7: an included
        // FIFO would wait twice), and said as what it is — never blamed on
        // the includes (review).
        crate::exec::ChildEnd::TimedOut => {
            return Err(Error::Invariant(format!(
                "the compiler's listing of {} did not finish in {} s — raise [oracle] timeout_secs",
                input.file_name().unwrap_or_default().to_string_lossy(),
                runner.timeout.as_secs()
            )))
        }
        crate::exec::ChildEnd::OutputOverflow => {
            return Err(Error::Invariant(format!(
                "the compiler's listing of {} printed more than {} bytes",
                input.file_name().unwrap_or_default().to_string_lossy(),
                runner.max_output
            )))
        }
        crate::exec::ChildEnd::Exited(_) => {
            // The compiler's own words, from a run without -H, whose header
            // lines would bury them (fix check 6 L2).
            argv.retain(|a| a != "-H");
            let why = match runner.tool_outcome(&argv)? {
                Err(plain) => plain,
                Ok(_) => crate::exec::stderr_excerpt(&run.stderr),
            };
            return Ok(Err(Error::Invariant(why)));
        }
    };
    let rules = std::fs::read(deps_path).map_err(|e| Error::io(deps_path, e))?;
    let text = std::fs::read(text_path).map_err(|e| Error::io(text_path, e))?;
    let canonical_of = |p: &Path| p.canonicalize().map_err(|e| Error::io(p, e));
    let copy = match copy {
        Some(c) => Some((
            canonical_of(c.build)?,
            canonical_of(c.header)?,
            c.mirror,
            canonical_of(c.mirror)?,
            c.root,
            c.source_dir,
            c.has_notes,
        )),
        None => None,
    };
    // `Ok(None)`: the probe's header, left out.
    let name = |token: &str| -> Result<Result<Option<Read>, Error>, Error> {
        let spelled = runner.work.join(token);
        // A file the compiler read resolves; one that does not is a list
        // misread, never a file to skip (fix check 3 N8).
        let Ok(canonical) = spelled.canonicalize() else {
            return Ok(Err(Error::Invariant(format!(
                "the compiler's list names {token:?}, which does not resolve"
            ))));
        };
        let Some((build, header, mirror, mirror_canonical, root, source_dir, has_notes)) = &copy
        else {
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
            // Only a file with notes matters: its original runs unwatched
            // (check 7: a function-less header read by its own path is the
            // same file either way).
            Err(_)
                if canonical.starts_with(source_dir)
                    && !canonical.starts_with(build)
                    && has_notes(&canonical) =>
            {
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
    let mut listed = std::collections::BTreeSet::new();
    let mut entered_as_read = std::collections::BTreeSet::new();
    for token in make_prerequisites(&String::from_utf8_lossy(&rules)) {
        if let Ok(read) = runner.work.join(&token).canonicalize() {
            listed.insert(read);
        }
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
        if let Ok(read) = runner.work.join(path).canonicalize() {
            entered_as_read.insert(read);
        }
        match name(path)? {
            Ok(Some(read)) => entered.push((depth, read)),
            Ok(None) => {}
            Err(why) => return Ok(Err(why)),
        }
    }
    Ok(Ok(Reads {
        files,
        entered,
        listed,
        entered_as_read,
        text,
    }))
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
            // ASCII blanks only: a no-break or ideographic space is part of
            // a name (check 7).
            ' ' | '\t' | '\r' => flush(&mut word, &mut words, targets),
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

/// Copy every regular file under `source_dir` (not `migration/`, nor a
/// dot-folder such as `.git/`) into `mirror` at its repo-relative path, each
/// file the facts give functions probed. A link inside `source_dir` is not
/// descended: its files are copied once, under their real paths (the walk's
/// aliases are not copied). Returns the unwatched pairs.
fn write_mirror(
    base: &Base,
    facts: &Facts,
    index: &PairIndex,
    mirror: &Path,
    probe: &mut Probe,
) -> Result<Vec<(PathBuf, std::time::SystemTime)>, Error> {
    let source_dir = match &base.layout {
        Layout::Folder { source_dir, .. } => source_dir,
        Layout::FileList { files } => {
            return write_mirror_files(base, facts, index, mirror, probe, files)
        }
    };
    let skipped_dirs = [base.root.join("migration")];
    let walked = walk::confined_except(
        source_dir,
        walk::ALL_FILES,
        walk::Limits {
            max_files: Some(MIRROR_MAX_FILES),
            max_depth: None,
        },
        &skipped_dirs,
    );
    if let Some((path, why)) = walked.first_unreadable() {
        return Err(Error::io(path, std::io::Error::other(why.to_string())));
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
    let mut times: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
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
        // An unreadable file stays out of the copy (check 7): if the program
        // reads it, the listings differ and the map refuses by name.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            times.push((mirror.join(rel_path), modified));
        }
        total += bytes.len() as u64;
        if total > MIRROR_MAX_BYTES {
            return Err(Error::Invariant(format!(
                "the source dir holds more than {} MiB; the features map does not copy that much",
                MIRROR_MAX_BYTES / (1024 * 1024)
            )));
        }
        if with_functions.contains(rel) {
            // The listing copy first (end tokens, §3.2).
            probe.add(mirror, rel, bytes, &|id| index.of(rel, id), true)?;
        } else {
            write(&mirror.join(rel_path), &bytes)?;
        }
    }
    keep_times(&times);
    Ok(times)
}

/// [`write_mirror`] for a file-list target: the listed files and every
/// header they reach (the facts' include closure), and every file the
/// configuration `-include`s, each copied at its project-relative path — and
/// each inside the root, never under `migration/` (any tool's ledger, the
/// map) and not reached through a link: nothing the harness or a model
/// wrote reaches the copy as the project's C (docs/PROJECT-MAP-DESIGN.md
/// §3.7, "Confinement").
fn write_mirror_files(
    base: &Base,
    facts: &Facts,
    index: &PairIndex,
    mirror: &Path,
    probe: &mut Probe,
    files: &[crate::Listed],
) -> Result<Vec<(PathBuf, std::time::SystemTime)>, Error> {
    let listed: Vec<String> = files.iter().map(|l| l.rel.clone()).collect();
    let mut copied: std::collections::BTreeSet<String> =
        facts.include_closure(&listed).into_iter().collect();
    copied.extend(listed);
    if copied.len() > MIRROR_MAX_FILES {
        return Err(Error::Invariant(format!(
            "the listed files and their headers are more than {MIRROR_MAX_FILES} files; the \
             features map does not copy that many"
        )));
    }
    let mut paths: Vec<(String, PathBuf)> = Vec::with_capacity(copied.len());
    for rel in copied {
        let path = project_path(
            FEATURES_BUILD_DIR,
            "a file the scratch copy holds",
            &base.root,
            &base.ledger,
            &rel,
        )?;
        if path != base.root.join(&rel) {
            return Err(Error::Invariant(format!(
                "{} is reached through a link: the features map copies each listed file and \
                 header once, at its own path, so it cannot map this program",
                harness_core::text::safe_line(&rel)
            )));
        }
        paths.push((rel, path));
    }
    for path in base.forced_includes() {
        let rel = path
            .strip_prefix(&base.root)
            .ok()
            .and_then(|r| r.to_str())
            .map(str::to_string)
            .ok_or_else(|| {
                Error::Invariant(format!("{} is not under the target root", path.display()))
            })?;
        if !paths.iter().any(|(r, _)| *r == rel) {
            paths.push((rel, path));
        }
    }
    let with_functions: std::collections::BTreeSet<&str> =
        facts.symbols.iter().map(|s| s.file.as_str()).collect();
    let mut total: u64 = 0;
    let mut times: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for (rel, path) in &paths {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        total += bytes.len() as u64;
        if total > MIRROR_MAX_BYTES {
            return Err(Error::Invariant(format!(
                "the listed files and their headers hold more than {} MiB; the features map \
                 does not copy that much",
                MIRROR_MAX_BYTES / (1024 * 1024)
            )));
        }
        if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
            times.push((mirror.join(rel), modified));
        }
        if with_functions.contains(rel.as_str()) {
            probe.add(mirror, rel, bytes, &|id| index.of(rel, id), true)?;
        } else {
            write(&mirror.join(rel), &bytes)?;
        }
    }
    // Every folder a compile of the copy searches exists there.
    for listed in files {
        for dir in &listed.includes {
            if let Ok(rel) = dir.strip_prefix(&base.root) {
                let made = mirror.join(rel);
                std::fs::create_dir_all(&made).map_err(|e| Error::io(&made, e))?;
            }
        }
    }
    keep_times(&times);
    Ok(times)
}

/// Give each mirror file its original's modification time: gcc's
/// `__TIMESTAMP__` reads it (`SOURCE_DATE_EPOCH` covers it on clang only;
/// docs/FEATURES-PROBE-REDESIGN.md §3.3 step 4).
fn keep_times(times: &[(PathBuf, std::time::SystemTime)]) {
    for (path, modified) in times {
        if let Ok(file) = std::fs::File::options().write(true).open(path) {
            let _ = file.set_modified(*modified);
        }
    }
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
        Some(Collected::Bytes(bytes)) => match decode_notes(bytes, index) {
            Ok(functions) => ("complete", None, functions),
            Err(why) => ("unavailable", Some(why), Vec::new()),
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

/// The notes as `[file, id]` pairs, sorted, each once
/// (docs/FEATURES-PROBE-REDESIGN.md §3.6): exactly one byte per watched pair
/// and the attach byte, each 0 or 1. The attach byte 0 means the runtime's
/// setup did not run — the program changed `TMPDIR`, exited inside its own
/// constructor — never a record where nothing ran.
fn decode_notes(bytes: &[u8], index: &PairIndex) -> Result<Vec<(String, String)>, &'static str> {
    if bytes.len() != index.len() + 1 || bytes.iter().any(|b| *b > 1) {
        return Err("unreadable");
    }
    if bytes[index.len()] != 1 {
        return Err("the probe's setup did not run");
    }
    let mut pairs: Vec<(String, String)> = bytes[..index.len()]
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == 1)
        .map(|(i, _)| index.pairs[i].clone())
        .collect();
    pairs.sort();
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    /// Fix pass 2's check: a compiler's clone of a function (gcc's
    /// `scale.isra.0`, `scale.part.0`) is that function; a variable of the
    /// name is not; an external id needs an external symbol.
    #[test]
    fn a_function_defined_is_read_by_its_c_name_and_kind() {
        let d = |name: &str, external: bool, function: bool| crate::objsyms::Defined {
            name: name.to_string(),
            external,
            function,
        };
        assert!(defines_function(
            &[d("scale.isra.0", false, true)],
            "scale",
            false
        ));
        assert!(defines_function(
            &[d("scale.part.0", false, true)],
            "scale",
            false
        ));
        assert!(
            !defines_function(&[d("scale", false, false)], "scale", false),
            "data"
        );
        assert!(
            !defines_function(&[d("scale", false, true)], "scale", true),
            "a static"
        );
        assert!(defines_function(&[d("scale", true, true)], "scale", true));
        assert!(!defines_function(&[d("scaler", true, true)], "scale", true));
    }

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
        // A no-break space is part of a name (check 7).
        assert_eq!(
            make_prerequisites("a.o: /t/caf\u{e9}\u{a0}dir/a.c /t/b.h\n"),
            ["/t/caf\u{e9}\u{a0}dir/a.c", "/t/b.h"]
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
        // Three pairs: main, helper, api — then the attach byte.
        assert_eq!(
            decode_notes(&[1, 0, 1, 1], &index),
            Ok(vec![
                ("src/a.c".into(), "main".into()),
                ("src/b.c".into(), "api".into())
            ]),
            "sorted, each once"
        );
        assert_eq!(decode_notes(&[0, 0, 0, 1], &index), Ok(vec![]));
        assert_eq!(
            decode_notes(&[1, 1, 1, 0], &index),
            Err("the probe's setup did not run"),
            "never a record where nothing ran"
        );
        assert_eq!(decode_notes(&[], &index), Err("unreadable"));
        assert_eq!(decode_notes(&[1, 0, 1], &index), Err("unreadable"), "short");
        assert_eq!(
            decode_notes(&[1, 0, 1, 1, 0], &index),
            Err("unreadable"),
            "long"
        );
        assert_eq!(
            decode_notes(&[2, 0, 1, 1], &index),
            Err("unreadable"),
            "not 0 or 1"
        );
    }

    /// Review: the random folder is the person's alone, named with 16 hex
    /// digits, registered for the signal handler while it lives, and gone
    /// after.
    #[cfg(unix)]
    #[test]
    fn the_random_folder_is_private_and_goes() {
        use std::os::unix::fs::PermissionsExt;
        let out = MapOut::create().expect("made");
        let path = out.path().to_path_buf();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{path:?}");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let hex = name.strip_prefix("ruharness-map-").expect("named");
        assert!(
            hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}"
        );
        assert!(LIVE_DIRS.lock().unwrap().contains(&path));
        drop(out);
        assert!(!path.exists());
        assert!(!LIVE_DIRS.lock().unwrap().contains(&path));
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
