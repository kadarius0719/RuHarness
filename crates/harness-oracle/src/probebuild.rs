//! The probed build (docs/FEATURES-PROBE-REDESIGN.md §3.4): one real compile
//! per top-level file, then the link. A note the compiler rejects is taken
//! out of exactly the function the error lands in, with the compiler's
//! words as the reason; an error no body holds is found by a search over
//! the notes; the link's undefined symbols name their functions. The
//! objects and the link are the probed program.

use crate::exec::{ChildEnd, Runner};
use crate::featuremap::MapProgress;
use crate::objsyms;
use crate::probecopy::{Kind, Probe, Reason};
use crate::CcInvocation;
use harness_core::error::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Which compiler `cc` is (§3.4 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cc {
    /// Clang (Apple's or LLVM's).
    Clang,
    /// gcc, with its major version.
    Gcc(u32),
}

impl Cc {
    /// Read `cc -dM -E -x c /dev/null`: `__clang__` is clang; `__GNUC__`
    /// without it is gcc; neither is refused.
    pub(crate) fn detect(runner: &Runner) -> Result<Cc, Error> {
        let argv: Vec<String> = ["cc", "-dM", "-E", "-x", "c", "/dev/null"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let macros = String::from_utf8_lossy(&runner.tool(&argv)?).into_owned();
        let value = |name: &str| {
            macros.lines().find_map(|l| {
                let mut words = l.split_whitespace();
                (words.next() == Some("#define") && words.next() == Some(name))
                    .then(|| words.next().unwrap_or("").to_string())
            })
        };
        if value("__clang__").is_some() {
            Ok(Cc::Clang)
        } else if let Some(major) = value("__GNUC__") {
            Ok(Cc::Gcc(major.parse().unwrap_or(0)))
        } else {
            Err(Error::Invariant(
                "the features map knows clang and gcc; `cc` here is neither".to_string(),
            ))
        }
    }

    /// The flags that make every error readable (§3.4 step 1).
    fn error_flags(self) -> Vec<String> {
        let flags: &[&str] = match self {
            Cc::Clang => &[
                "-ferror-limit=0",
                "-Xclang",
                "-fno-diagnostics-use-presumed-location",
            ],
            Cc::Gcc(major) if major >= 11 => &[
                "-fmax-errors=0",
                "-ftrack-macro-expansion=0",
                "-fdiagnostics-column-unit=byte",
            ],
            Cc::Gcc(_) => &["-fmax-errors=0", "-ftrack-macro-expansion=0"],
        };
        flags.iter().map(|s| s.to_string()).collect()
    }
}

/// One error the compiler reported: its located file, line and byte
/// column when it gave them, the include chain printed last before it
/// (innermost first), and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub at: Option<(PathBuf, usize, usize)>,
    pub chain: Vec<(PathBuf, usize)>,
    pub message: String,
}

/// Read a compile's stderr into its errors (`error:` and `fatal error:`
/// lines). The include chain is carried from the last one printed — the
/// compiler prints it only when it changes; clang prints it outermost
/// first, one "In file included from" per line; gcc innermost first, the
/// rest as "from" lines.
pub(crate) fn diagnostics(stderr: &[u8], cc: Cc) -> Vec<Diagnostic> {
    let text = String::from_utf8_lossy(stderr);
    let mut out = Vec::new();
    let mut chain: Vec<(PathBuf, usize)> = Vec::new();
    let mut building = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("In file included from ") {
            if !building {
                chain.clear();
                building = true;
            }
            if let Some(frame) = frame(rest) {
                chain.push(frame);
            }
            continue;
        }
        if building && cc != Cc::Clang {
            if let Some(rest) = trimmed.strip_prefix("from ") {
                if let Some(frame) = frame(rest) {
                    chain.push(frame);
                }
                continue;
            }
        }
        // The earlier of the two: a message may itself hold the other.
        let kind = [": fatal error: ", ": error: "]
            .iter()
            .filter_map(|k| line.find(k).map(|at| (at, k.len())))
            .min();
        if let Some((at, len)) = kind {
            let located = located(&line[..at]);
            let message = line[at + len..].to_string();
            let chain_in = match cc {
                // clang: outermost first — reversed to innermost first.
                Cc::Clang => chain.iter().rev().cloned().collect(),
                Cc::Gcc(_) => chain.clone(),
            };
            out.push(Diagnostic {
                at: located,
                chain: if building { chain_in } else { Vec::new() },
                message,
            });
            building = false;
        } else if line.starts_with("clang: error:")
            || line.starts_with("cc1: error:")
            || line.starts_with("cc: error:")
            || line.starts_with("gcc: error:")
        {
            out.push(Diagnostic {
                at: None,
                chain: Vec::new(),
                message: line.to_string(),
            });
        }
    }
    out
}

/// `path:line[:column]` from an include-chain frame (a trailing `:` or `,`
/// dropped).
fn frame(text: &str) -> Option<(PathBuf, usize)> {
    let text = text.trim_end_matches([':', ',']);
    let mut parts = text.rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next();
    let first = parts.next();
    match (first, middle, last.parse::<usize>()) {
        // path:line:col
        (Some(path), Some(line), Ok(_)) if line.parse::<usize>().is_ok() => {
            Some((PathBuf::from(path), line.parse().ok()?))
        }
        // path:line
        (_, Some(path_or_first), Ok(line)) => {
            let path = match first {
                Some(f) => format!("{f}:{path_or_first}"),
                None => path_or_first.to_string(),
            };
            Some((PathBuf::from(path), line))
        }
        _ => None,
    }
}

/// `path:line:column` before an error's kind.
fn located(text: &str) -> Option<(PathBuf, usize, usize)> {
    let mut parts = text.rsplitn(3, ':');
    let col: usize = parts.next()?.parse().ok()?;
    let line: usize = parts.next()?.parse().ok()?;
    let path = parts.next()?;
    Some((PathBuf::from(path), line, col))
}

/// The byte offset of `line` (1-based) and byte `col` (1-based) in `text`,
/// lines ending at `\n`, `\r\n` or a lone `\r`, as the compilers count them.
pub(crate) fn offset(text: &[u8], line: usize, col: usize) -> Option<usize> {
    let mut current = 1;
    let mut i = 0;
    while current < line {
        match text.get(i)? {
            b'\n' => current += 1,
            b'\r' => {
                current += 1;
                if text.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Some(i + col.saturating_sub(1))
}

/// What the build needs to know about the copy.
pub(crate) struct Build<'a> {
    pub runner: &'a Runner,
    pub cc: Cc,
    pub root: &'a Path,
    pub mirror: &'a Path,
    pub includes: &'a [PathBuf],
    pub cflags: &'a [String],
    /// The copy's top-level files (mirror paths), in the plain build's
    /// input order.
    pub units: &'a [PathBuf],
    /// Per top-level file: the probed files it enters (its own included).
    pub reads: &'a [BTreeSet<String>],
    pub runtime: &'a Path,
    pub link_args: &'a [String],
    pub out: &'a Path,
    pub index_of: &'a dyn Fn(&str, &str) -> Option<u32>,
    /// Every function the facts record: `(file, canonical id)`.
    pub functions: &'a [(String, String)],
    pub times: &'a [(PathBuf, std::time::SystemTime)],
    pub bounds: MapBounds,
}

/// The per-file and whole-pass bounds of the notes check (§3.4 step 6).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapBounds {
    /// Rounds that placed a note, per top-level file.
    pub placed_rounds: usize,
    /// Compiles per top-level file (a step 7 re-compile starts afresh).
    pub file_compiles: usize,
    /// Compiles for the whole pass beyond each file's first compile and
    /// step 7's re-compiles.
    pub pass_compiles: usize,
}

/// The bounds the design names.
const BOUNDS: MapBounds = MapBounds {
    placed_rounds: 8,
    file_compiles: 64,
    pass_compiles: 400,
};
const LINK_ROUNDS: usize = 32;

thread_local! {
    static BOUNDS_HERE: std::cell::Cell<MapBounds> = const { std::cell::Cell::new(BOUNDS) };
}

/// The bounds a map on this thread uses.
pub(crate) fn bounds() -> MapBounds {
    BOUNDS_HERE.with(std::cell::Cell::get)
}

/// Run `f` with other bounds on this thread — for tests that reach a bound
/// in a few compiles rather than hundreds.
#[doc(hidden)]
pub fn with_map_bounds<R>(bounds: MapBounds, f: impl FnOnce() -> R) -> R {
    struct Restore(MapBounds);
    impl Drop for Restore {
        fn drop(&mut self) {
            BOUNDS_HERE.with(|b| b.set(self.0));
        }
    }
    let _restore = Restore(BOUNDS_HERE.with(|b| b.replace(bounds)));
    f()
}

/// How a search over the notes ended (§3.4 step 4).
enum Search {
    /// Note k: with it out (and the ones before it), the file compiles.
    Found((String, String)),
    /// A bound stopped it before it narrowed to one note.
    Cut,
}

/// How a compile or link ended, for the pass.
enum Outcome {
    Ok,
    Failed(Vec<u8>),
}

impl Build<'_> {
    fn object(&self, n: usize) -> PathBuf {
        self.out.join(format!("probed-{n}.o"))
    }

    fn shown(&self, p: &Path) -> String {
        let rel = p
            .strip_prefix(self.mirror)
            .ok()
            .map(|r| r.to_path_buf())
            .unwrap_or_else(|| p.strip_prefix(self.root).unwrap_or(p).to_path_buf());
        rel.display().to_string()
    }

    fn rewrite(
        &self,
        probe: &mut Probe,
        rels: &BTreeSet<String>,
        extra: &[(String, String)],
    ) -> Result<(), Error> {
        for rel in rels {
            let ids: Vec<String> = extra
                .iter()
                .filter(|(r, _)| r == rel)
                .map(|(_, id)| id.clone())
                .collect();
            let index_of = |id: &str| (self.index_of)(rel, id);
            probe.write_with(self.mirror, rel, &index_of, false, &ids)?;
        }
        crate::featuremap::keep_file_times(self.times);
        Ok(())
    }

    fn compile(&self, n: usize, syntax_only: bool) -> Result<Outcome, Error> {
        let object = self.object(n);
        let mut cflags = self.cflags.to_vec();
        cflags.extend(self.cc.error_flags());
        cflags.push(if syntax_only { "-fsyntax-only" } else { "-c" }.to_string());
        let mut argv = crate::cc_argv(&CcInvocation {
            includes: self.includes,
            cflags: &cflags,
            quiet: true,
            out: &object,
            inputs: std::slice::from_ref(&self.units[n]),
            libs: &[],
        })?;
        if syntax_only {
            if let Some(at) = argv.iter().position(|a| a == "-o") {
                argv.drain(at..at + 2);
            }
        }
        let run = self.runner.tool_run(&argv)?;
        match run.end {
            ChildEnd::Exited(status) if status.success() => Ok(Outcome::Ok),
            ChildEnd::Exited(_) => Ok(Outcome::Failed(run.stderr)),
            ChildEnd::TimedOut => Err(Error::Invariant(format!(
                "the scratch copy's compile of {} did not finish in {} s — raise [oracle] \
                 timeout_secs",
                self.shown(&self.units[n]),
                self.runner.timeout.as_secs()
            ))),
            ChildEnd::OutputOverflow => Err(Error::Invariant(format!(
                "the scratch copy's compile of {} printed more than {} bytes",
                self.shown(&self.units[n]),
                self.runner.max_output
            ))),
        }
    }

    /// The probed file and the innermost watched body a located error falls
    /// in (the chain's innermost probed frame when the error's own file is
    /// not probed).
    fn place(&self, probe: &Probe, d: &Diagnostic, gcc_presumed: bool) -> Option<(String, String)> {
        let mirror = self.mirror.canonicalize().ok()?;
        let rel_of = |p: &Path| -> Option<String> {
            let canonical = if p.is_absolute() {
                p.canonicalize().ok()?
            } else {
                self.runner.cwd.join(p).canonicalize().ok()?
            };
            canonical
                .strip_prefix(&mirror)
                .ok()
                .and_then(|r| r.to_str())
                .map(str::to_string)
        };
        let (path, line, col) = d.at.as_ref()?;
        let spot = match rel_of(path).filter(|rel| probe.is_probed(rel)) {
            Some(rel) => Some((rel, *line, *col)),
            None => d.chain.iter().find_map(|(p, line)| {
                rel_of(p)
                    .filter(|rel| probe.is_probed(rel))
                    .map(|rel| (rel, *line, 1))
            }),
        };
        let (rel, line, col) = spot?;
        if gcc_presumed && probe.has_line_directives(&rel) {
            return None;
        }
        let text = std::fs::read(self.mirror.join(&rel)).ok()?;
        // gcc counts a byte-order mark's bytes in line 1's columns its own
        // way: such an error goes to the search (§3.4 step 3).
        if matches!(self.cc, Cc::Gcc(_)) && line == 1 && text.starts_with(b"\xef\xbb\xbf") {
            return None;
        }
        let at = offset(&text, line, col)?;
        probe
            .notes(&rel)
            .iter()
            .filter(|n| n.body.0 <= at && at < n.body.1)
            .min_by_key(|n| n.body.1 - n.body.0)
            .map(|n| (rel.clone(), n.id.clone()))
    }

    /// Compile top-level file `n` until it compiles, taking notes out as
    /// §3.4 steps 3–6 say. `pass` counts every compile beyond the call's
    /// first (a file's first compile, or a step 7 re-compile).
    fn settle(
        &self,
        n: usize,
        probe: &mut Probe,
        pass: &mut usize,
        progress: &mut dyn MapProgress,
    ) -> Result<(), Error> {
        let bounds = self.bounds;
        // Rounds that placed a note (the per-file bound), and every round
        // (what the person sees).
        let mut placed_rounds = 0;
        let mut round = 0;
        let mut compiles = 0;
        let rels = &self.reads[n];
        let gcc = matches!(self.cc, Cc::Gcc(_));
        let mut eliminated: Vec<(String, String)> = Vec::new();
        loop {
            round += 1;
            progress.message(&format!(
                "Checking where the notes compile… {} (round {round})",
                self.shown(&self.units[n]),
            ));
            if compiles > 0 {
                *pass += 1;
            }
            compiles += 1;
            let stderr = match self.compile(n, false)? {
                Outcome::Ok => {
                    // The restore pass: each note the search took out is put
                    // back alone once and kept if the file still compiles —
                    // within the bounds; past them the rest stay out.
                    for (rel, id) in std::mem::take(&mut eliminated) {
                        if compiles >= bounds.file_compiles || *pass >= bounds.pass_compiles {
                            break;
                        }
                        let reason = probe.reasons.get(&(rel.clone(), id.clone())).cloned();
                        probe.put_back(&rel, &id);
                        self.rewrite(probe, rels, &[])?;
                        compiles += 1;
                        *pass += 1;
                        if !matches!(self.compile(n, false)?, Outcome::Ok) {
                            if let Some(reason) = reason {
                                probe.take_out(&rel, &id, reason);
                            }
                            self.rewrite(probe, rels, &[])?;
                            compiles += 1;
                            *pass += 1;
                            if !matches!(self.compile(n, false)?, Outcome::Ok) {
                                return Err(Error::Invariant(format!(
                                    "the scratch copy of {} stopped compiling while its notes \
                                     were put back — a fault in the harness, not your program; \
                                     please report it",
                                    self.shown(&self.units[n])
                                )));
                            }
                        }
                    }
                    return Ok(());
                }
                Outcome::Failed(stderr) => stderr,
            };
            if placed_rounds >= bounds.placed_rounds
                || compiles >= bounds.file_compiles
                || *pass >= bounds.pass_compiles
            {
                return self.give_up(n, probe, &stderr);
            }
            let found = diagnostics(&stderr, self.cc);
            let mut placed = false;
            let mut unplaced: Option<String> = None;
            for d in &found {
                match self.place(probe, d, gcc) {
                    Some((rel, id)) => {
                        let reason = Reason::new(Kind::Compile, &d.message);
                        placed |= probe.take_out(&rel, &id, reason);
                    }
                    None => {
                        unplaced.get_or_insert_with(|| d.message.clone());
                    }
                }
            }
            if found.is_empty() {
                unplaced = Some(crate::exec::stderr_excerpt(&stderr));
            }
            if placed {
                placed_rounds += 1;
                self.rewrite(probe, rels, &[])?;
                continue;
            }
            let why = unplaced.unwrap_or_else(|| crate::exec::stderr_excerpt(&stderr));
            match self.search(n, probe, &why, &mut compiles, pass)? {
                Search::Found(pair) => eliminated.push(pair),
                // A search a bound stopped has found nothing: the file goes
                // back unprobed, no note blamed.
                Search::Cut => return self.give_up(n, probe, &stderr),
            }
        }
    }

    /// §3.4 step 4: compile with every note of the file and the probed
    /// files it reads taken out — still failing, the copy differs (a
    /// harness fault); compiling, a binary search finds one note to take
    /// out. The trials compile `-fsyntax-only` when the chased error also
    /// shows there.
    fn search(
        &self,
        n: usize,
        probe: &mut Probe,
        why: &str,
        compiles: &mut usize,
        pass: &mut usize,
    ) -> Result<Search, Error> {
        let bounds = self.bounds;
        let rels = &self.reads[n];
        let notes: Vec<(String, String)> = rels
            .iter()
            .flat_map(|rel| {
                let mut ns: Vec<_> = probe.notes(rel).to_vec();
                ns.sort_by_key(|note| note.body.0);
                ns.into_iter().map(move |note| (rel.clone(), note.id))
            })
            .collect();
        let mut seen = BTreeSet::new();
        let notes: Vec<(String, String)> = notes
            .into_iter()
            .filter(|p| seen.insert(p.clone()))
            .collect();
        *compiles += 1;
        *pass += 1;
        let syntax_only = match self.compile(n, true)? {
            Outcome::Failed(stderr) => diagnostics(&stderr, self.cc)
                .iter()
                .any(|d| d.message == why),
            Outcome::Ok => false,
        };
        let trial = |k: usize,
                     probe: &mut Probe,
                     compiles: &mut usize,
                     pass: &mut usize|
         -> Result<bool, Error> {
            self.rewrite(probe, rels, &notes[..k])?;
            *compiles += 1;
            *pass += 1;
            Ok(matches!(self.compile(n, syntax_only)?, Outcome::Ok))
        };
        if !trial(notes.len(), probe, compiles, pass)? {
            self.rewrite(probe, rels, &[])?;
            return Err(Error::Invariant(format!(
                "the scratch copy of {} does not compile even without notes: {} — a fault in the \
                 harness, not your program; please report it",
                self.shown(&self.units[n]),
                crate::probecopy::detail_text(why)
            )));
        }
        let (mut lo, mut hi) = (0usize, notes.len());
        while hi - lo > 1 {
            if *compiles >= bounds.file_compiles || *pass >= bounds.pass_compiles {
                self.rewrite(probe, rels, &[])?;
                return Ok(Search::Cut);
            }
            let mid = lo + (hi - lo) / 2;
            if trial(mid, probe, compiles, pass)? {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let found = notes.get(hi.saturating_sub(1)).cloned();
        self.rewrite(probe, rels, &[])?;
        match found {
            Some((rel, id)) => {
                probe.take_out(&rel, &id, Reason::new(Kind::Elimination, why));
                self.rewrite(probe, rels, &[])?;
                Ok(Search::Found((rel, id)))
            }
            None => Ok(Search::Cut),
        }
    }

    /// Past a bound: the file and every probed file it reads go back
    /// unprobed, and it must compile then.
    fn give_up(&self, n: usize, probe: &mut Probe, stderr: &[u8]) -> Result<(), Error> {
        let first = diagnostics(stderr, self.cc)
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| crate::exec::stderr_excerpt(stderr));
        for rel in &self.reads[n] {
            probe.unprobe(rel, Reason::new(Kind::FileLimit, &first));
        }
        self.rewrite(probe, &self.reads[n], &[])?;
        match self.compile(n, false)? {
            Outcome::Ok => Ok(()),
            Outcome::Failed(stderr) => Err(Error::Invariant(format!(
                "the scratch copy of {} does not compile even without notes: {} — a fault in the \
                 harness, not your program; please report it",
                self.shown(&self.units[n]),
                crate::exec::stderr_excerpt(&stderr)
            ))),
        }
    }

    /// Compile every top-level file, re-compile those whose probed files
    /// changed, then link — again after a link that took notes out; returns
    /// the probed program and what each object defines.
    pub(crate) fn run(
        &self,
        probe: &mut Probe,
        progress: &mut dyn MapProgress,
    ) -> Result<Probed, Error> {
        let mut pass = 0usize;
        let notes_of = |probe: &Probe| -> BTreeMap<String, usize> {
            probe
                .rels()
                .into_iter()
                .map(|r| {
                    let len = probe.notes(&r).len();
                    (r, len)
                })
                .collect()
        };
        let mut compiled_with: Vec<BTreeMap<String, usize>> = Vec::with_capacity(self.units.len());
        // The probed files a settled compile has checked: past the pass
        // bound, only the others go back unprobed.
        let mut checked: BTreeSet<String> = BTreeSet::new();
        for n in 0..self.units.len() {
            if pass >= self.bounds.pass_compiles {
                let unchecked: BTreeSet<String> =
                    self.reads[n].difference(&checked).cloned().collect();
                for rel in &unchecked {
                    probe.unprobe(rel, Reason::new(Kind::NotChecked, ""));
                }
                self.rewrite(probe, &unchecked, &[])?;
            }
            self.settle(n, probe, &mut pass, progress)?;
            checked.extend(self.reads[n].iter().cloned());
            compiled_with.push(notes_of(probe));
        }
        for _ in 0..LINK_ROUNDS {
            // Re-compiles: a file whose probed files lost notes after it
            // compiled, or whose object a link search left in a trial state.
            loop {
                let now = notes_of(probe);
                let stale: Vec<usize> = (0..self.units.len())
                    .filter(|n| {
                        self.reads[*n]
                            .iter()
                            .any(|rel| compiled_with[*n].get(rel) != now.get(rel))
                    })
                    .collect();
                if stale.is_empty() {
                    break;
                }
                for n in stale {
                    self.settle(n, probe, &mut pass, progress)?;
                    compiled_with[n] = notes_of(probe);
                }
            }
            match self.link(probe, &mut pass)? {
                Linked::Program(probed) => return Ok(probed),
                Linked::Changed(dirty) => {
                    for n in dirty {
                        compiled_with[n].clear();
                    }
                }
            }
        }
        Err(Error::Invariant(
            "the scratch copy did not link within its bound — a fault in the harness, not your \
             program; please report it"
                .to_string(),
        ))
    }

    /// One link of the probed program: the runtime's object first, the
    /// program's objects in the plain build's order, its libraries.
    /// `Ok(None)` when it linked, else the linker's stderr.
    fn link_once(&self, program: &Path) -> Result<Option<Vec<u8>>, Error> {
        let mut argv = vec![
            "cc".to_string(),
            "-o".to_string(),
            crate::featuremap::path_text(program)?,
            crate::featuremap::path_text(self.runtime)?,
        ];
        for n in 0..self.units.len() {
            argv.push(crate::featuremap::path_text(&self.object(n))?);
        }
        argv.extend(self.link_args.iter().cloned());
        let run = self.runner.tool_run(&argv)?;
        match run.end {
            ChildEnd::Exited(status) if status.success() => Ok(None),
            ChildEnd::Exited(_) => Ok(Some(run.stderr)),
            _ => Err(Error::Invariant(format!(
                "the scratch copy's link did not finish: {}",
                crate::exec::stderr_excerpt(&run.stderr)
            ))),
        }
    }

    /// The top-level files whose objects the linker names (every one when
    /// it names none it made).
    fn units_named(&self, refs: &[Referrer]) -> BTreeSet<usize> {
        let named: BTreeSet<usize> = refs
            .iter()
            .filter_map(|r| r.object.as_deref().and_then(unit_of_object))
            .filter(|n| *n < self.units.len())
            .collect();
        if named.is_empty() {
            (0..self.units.len()).collect()
        } else {
            named
        }
    }

    /// §3.4 step 5: link; undefined symbols take notes out — (a) a function
    /// of the program still carrying a note, (b) the referencing function
    /// the linker names, in the objects it names; when neither has a note
    /// left, a search over the notes those objects read, with the relink as
    /// the test.
    fn link(&self, probe: &mut Probe, pass: &mut usize) -> Result<Linked, Error> {
        let program = self.out.join("probed");
        let Some(stderr) = self.link_once(&program)? else {
            let defined = self.defined()?;
            if let Some(name) = runtime_name_defined(&defined) {
                let shown = if name == "environ" {
                    name
                } else {
                    format!("{name}()")
                };
                return Err(Error::Invariant(format!(
                    "the program defines {shown}, which the probe's runtime also uses before \
                     main — the features map cannot map it"
                )));
            }
            return Ok(Linked::Program(Probed { program, defined }));
        };
        let refuse = |stderr: &[u8]| {
            Error::Invariant(format!(
                "the scratch copy does not link: {}",
                crate::exec::stderr_excerpt(stderr)
            ))
        };
        let undefined = undefined_symbols(&stderr);
        if undefined.is_empty() {
            return Err(refuse(&stderr));
        }
        let mut changed = BTreeSet::new();
        let mut unresolved: Option<(String, BTreeSet<usize>)> = None;
        for u in &undefined {
            let why = format!("{} is undefined", u.symbol);
            let units = self.units_named(&u.refs);
            // (a) a function of the program: an external one, whose id is
            // its name (a static is never an undefined symbol).
            if self.functions.iter().any(|(_, id)| *id == u.symbol) {
                let mut done = false;
                for rel in probe.rels() {
                    let noted = probe.notes(&rel).iter().any(|note| note.id == u.symbol);
                    if noted && probe.take_out(&rel, &u.symbol, Reason::new(Kind::Link, &why)) {
                        changed.insert(rel);
                        done = true;
                    }
                }
                if !done {
                    unresolved.get_or_insert((u.symbol.clone(), units));
                }
                continue;
            }
            // (b) the referencing function, in the files its object reads.
            let mut done = false;
            for r in &u.refs {
                let Some(function) = &r.function else {
                    continue;
                };
                let rels: Vec<String> = match r.object.as_deref().and_then(unit_of_object) {
                    Some(n) if n < self.units.len() => self.reads[n].iter().cloned().collect(),
                    _ => probe.rels(),
                };
                for rel in rels {
                    let as_static = format!("{rel}::{function}");
                    let hit = probe
                        .notes(&rel)
                        .iter()
                        .find(|note| note.id == *function || note.id == as_static)
                        .map(|note| note.id.clone());
                    if let Some(id) = hit {
                        if probe.take_out(&rel, &id, Reason::new(Kind::Link, &why)) {
                            changed.insert(rel);
                            done = true;
                        }
                    }
                }
            }
            if !done {
                unresolved.get_or_insert((u.symbol.clone(), units));
            }
        }
        if !changed.is_empty() {
            self.rewrite(probe, &changed, &[])?;
            return Ok(Linked::Changed(BTreeSet::new()));
        }
        let Some((symbol, units)) = unresolved else {
            return Err(refuse(&stderr));
        };
        let why = format!("{symbol} is undefined");
        let rels: BTreeSet<String> = units
            .iter()
            .flat_map(|n| self.reads[*n].iter().cloned())
            .collect();
        match self.link_search(probe, &symbol, &units, &rels, &program, pass)? {
            Some(LinkSearch::Found(rel, id)) => {
                probe.take_out(&rel, &id, Reason::new(Kind::Link, &why));
            }
            Some(LinkSearch::Cut) => {
                for rel in &rels {
                    probe.unprobe(rel, Reason::new(Kind::FileLimit, &why));
                }
            }
            None => return Err(refuse(&stderr)),
        }
        self.rewrite(probe, &rels, &[])?;
        Ok(Linked::Changed(units))
    }

    /// The search of §3.4 step 4 over the notes the named objects' files
    /// carry, the test a relink that no longer leaves `symbol` undefined.
    /// `None` when even every note out does not.
    fn link_search(
        &self,
        probe: &mut Probe,
        symbol: &str,
        units: &BTreeSet<usize>,
        rels: &BTreeSet<String>,
        program: &Path,
        pass: &mut usize,
    ) -> Result<Option<LinkSearch>, Error> {
        let notes: Vec<(String, String)> = rels
            .iter()
            .flat_map(|rel| {
                let mut ns: Vec<_> = probe.notes(rel).to_vec();
                ns.sort_by_key(|note| note.body.0);
                ns.into_iter().map(move |note| (rel.clone(), note.id))
            })
            .collect();
        let trial = |k: usize, probe: &mut Probe, pass: &mut usize| -> Result<bool, Error> {
            self.rewrite(probe, rels, &notes[..k])?;
            for n in units {
                *pass += 1;
                if !matches!(self.compile(*n, false)?, Outcome::Ok) {
                    return Ok(false);
                }
            }
            Ok(match self.link_once(program)? {
                None => true,
                Some(stderr) => !undefined_symbols(&stderr)
                    .iter()
                    .any(|u| u.symbol == symbol),
            })
        };
        if !trial(notes.len(), probe, pass)? {
            return Ok(None);
        }
        let (mut lo, mut hi) = (0usize, notes.len());
        while hi - lo > 1 {
            if *pass >= self.bounds.pass_compiles {
                return Ok(Some(LinkSearch::Cut));
            }
            let mid = lo + (hi - lo) / 2;
            if trial(mid, probe, pass)? {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Ok(Some(match notes.get(hi.saturating_sub(1)) {
            Some((rel, id)) => LinkSearch::Found(rel.clone(), id.clone()),
            None => LinkSearch::Cut,
        }))
    }

    /// What each top-level file's object defines.
    fn defined(&self) -> Result<Vec<Vec<objsyms::Defined>>, Error> {
        (0..self.units.len())
            .map(|n| {
                let object = self.object(n);
                let bytes = std::fs::read(&object).map_err(|e| Error::io(&object, e))?;
                objsyms::defined(&bytes).map_err(|why| {
                    Error::Invariant(format!(
                        "the scratch copy's object for {} cannot be read: {why} — the features \
                         map reads 64-bit little-endian Mach-O and ELF objects",
                        self.shown(&self.units[n])
                    ))
                })
            })
            .collect()
    }
}

/// The probed program and what each top-level file's object defines.
pub(crate) struct Probed {
    pub program: PathBuf,
    pub defined: Vec<Vec<objsyms::Defined>>,
}

/// How one link ended for the pass.
enum Linked {
    /// It linked.
    Program(Probed),
    /// Notes were taken out; these files' objects are in a trial state and
    /// compile again.
    Changed(BTreeSet<usize>),
}

/// How a link search ended.
enum LinkSearch {
    Found(String, String),
    Cut,
}

/// The names the probe's runtime uses (docs/FEATURES-PROBE-REDESIGN.md
/// §3.5); a unit test pins them against the runtime object's imports.
pub(crate) const RUNTIME_IMPORTS: &[&str] = &["open", "fstat", "mmap", "close", "environ"];

/// The first of [`RUNTIME_IMPORTS`] one of the program's objects defines as
/// an external symbol — the only kind the runtime's import can bind to (a
/// static `close` cannot). A Mach-O variant suffix (`fstat$INODE64` on
/// Intel Macs) is the same name.
pub(crate) fn runtime_name_defined(objects: &[Vec<objsyms::Defined>]) -> Option<String> {
    objects.iter().flatten().find_map(|d| {
        let name = d.name.split('$').next().unwrap_or(&d.name);
        (d.external && RUNTIME_IMPORTS.contains(&name)).then(|| name.to_string())
    })
}

/// A function the linker names, without a compiler's clone suffix (gcc's
/// `.constprop.0`, `.isra.0`, `.part.0`, `.cold`: a C name has no `.`).
fn function_name(name: &str) -> String {
    name.split('.').next().unwrap_or(name).to_string()
}

/// The top-level file a probed object is: `probed-<n>.o`.
fn unit_of_object(object: &str) -> Option<usize> {
    let name = object.rsplit(['/', '\\']).next().unwrap_or(object);
    name.strip_prefix("probed-")?
        .strip_suffix(".o")?
        .parse()
        .ok()
}

/// One reference the linker names to an undefined symbol: the function it
/// is in (when the linker says) and the object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Referrer {
    pub function: Option<String>,
    pub object: Option<String>,
}

/// An undefined symbol of a failed link and its referrers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Undefined {
    pub symbol: String,
    pub refs: Vec<Referrer>,
}

/// The name quoted at the start of `text` — `` `name' ``, `'name'` or
/// `‘name’`.
fn quoted_name(text: &str) -> Option<String> {
    let rest = text
        .strip_prefix('`')
        .or_else(|| text.strip_prefix('\''))
        .or_else(|| text.strip_prefix('‘'))?;
    let end = rest.find(['\'', '’'])?;
    Some(rest[..end].to_string())
}

/// The object a GNU ld message names: the words after the linker's own
/// `ld: ` prefix, up to `:(` or `: in function`.
fn gnu_object(line: &str) -> Option<String> {
    let at = line.find("ld: ").map_or(0, |at| at + "ld: ".len());
    let rest = &line[at..];
    let lower = rest.to_ascii_lowercase();
    let end = [rest.find(":("), lower.find(": in function")]
        .into_iter()
        .flatten()
        .min()?;
    Some(rest[..end].trim().to_string())
}

/// The undefined symbols of a failed link, each with its referrers — ld64
/// (`"_name", referenced from:` then indented `_fn in f.o` lines), GNU ld
/// (`` f.o: in function `fn': `` then `` undefined reference to `name' ``
/// lines — any case, any quotes; a reference of its own, from data, names
/// no function) and lld (`undefined symbol: name` then `>>> referenced by …
/// f.o:(fn)`). Mach-O's leading `_` dropped.
pub(crate) fn undefined_symbols(stderr: &[u8]) -> Vec<Undefined> {
    let text = String::from_utf8_lossy(stderr);
    let strip = |s: &str| s.strip_prefix('_').unwrap_or(s).to_string();
    let mut out: Vec<Undefined> = Vec::new();
    let mut gnu_context: Option<Referrer> = None;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        // ld64
        if let Some(rest) = t.strip_prefix('"') {
            if let Some(end) = rest.find("\", referenced from:") {
                let symbol = strip(&rest[..end]);
                let mut refs = Vec::new();
                while let Some(next) = lines.peek() {
                    let n = next.trim();
                    if let Some((f, object)) = n.split_once(" in ") {
                        refs.push(Referrer {
                            function: Some(function_name(&strip(f))),
                            object: Some(object.trim().to_string()),
                        });
                        lines.next();
                    } else {
                        break;
                    }
                }
                out.push(Undefined { symbol, refs });
                continue;
            }
        }
        // GNU ld
        if let Some(at) = lower.find("in function ") {
            if !lower.contains("undefined reference to ") {
                gnu_context = Some(Referrer {
                    function: quoted_name(&t[at + "in function ".len()..])
                        .map(|f| function_name(&f)),
                    object: gnu_object(t),
                });
                continue;
            }
        }
        if let Some(at) = lower.find("undefined reference to ") {
            let Some(symbol) = quoted_name(&t[at + "undefined reference to ".len()..]) else {
                continue;
            };
            // A message of its own (the linker's prefix before it) is not
            // under the last "in function" line: a reference from data.
            let own = t[..at].contains("ld: ");
            let referrer = if own {
                Referrer {
                    function: None,
                    object: gnu_object(t),
                }
            } else {
                gnu_context.clone().unwrap_or(Referrer {
                    function: None,
                    object: None,
                })
            };
            out.push(Undefined {
                symbol,
                refs: vec![referrer],
            });
            continue;
        }
        // lld
        if let Some(rest) = t.strip_prefix("ld.lld: error: undefined symbol: ") {
            let symbol = rest.to_string();
            let mut refs = Vec::new();
            while let Some(next) = lines.peek() {
                let n = next.trim();
                let Some(r) = n.strip_prefix(">>>") else {
                    break;
                };
                if let (Some(open), Some(close)) = (r.rfind(":("), r.rfind(')')) {
                    if open < close {
                        let inside = &r[open + 2..close];
                        let function = inside
                            .chars()
                            .next()
                            .filter(|c| c.is_alphabetic() || *c == '_')
                            .map(|_| function_name(inside));
                        refs.push(Referrer {
                            function,
                            object: Some(r[..open].trim().to_string()),
                        });
                    }
                }
                lines.next();
            }
            out.push(Undefined { symbol, refs });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clang_errors_and_their_chains_read_back() {
        let stderr = b"In file included from /m/src/a.c:3:\nIn file included from /m/src/b.h:5:\n\
                       /usr/include/c.h:9:2: error: '#pragma STDC FENV_ACCESS' can only appear at file scope\n\
                       /m/src/a.c:12:7: error: expected expression\n\
                       clang: error: unable to execute command\n";
        let d = diagnostics(stderr, Cc::Clang);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].at, Some((PathBuf::from("/usr/include/c.h"), 9, 2)));
        assert_eq!(
            d[0].chain,
            vec![
                (PathBuf::from("/m/src/b.h"), 5),
                (PathBuf::from("/m/src/a.c"), 3)
            ],
            "innermost first"
        );
        assert!(d[1].chain.is_empty(), "no chain printed before it");
        assert_eq!(d[1].message, "expected expression");
        assert_eq!(d[2].at, None);
    }

    #[test]
    fn gcc_errors_and_their_chains_read_back() {
        let stderr = b"In file included from /m/src/b.h:5,\n                 from /m/src/a.c:3:\n\
                       /m/src/inc.h:2:15: error: expected ';' before 'x'\n\
                       /m/src/inc.h:4:1: error: again\n";
        let d = diagnostics(stderr, Cc::Gcc(13));
        assert_eq!(d.len(), 2);
        assert_eq!(
            d[0].chain,
            vec![
                (PathBuf::from("/m/src/b.h"), 5),
                (PathBuf::from("/m/src/a.c"), 3)
            ]
        );
        assert!(d[1].chain.is_empty());
    }

    #[test]
    fn a_line_and_column_are_a_byte_offset_as_the_compiler_counts() {
        let text = b"a\r\nbc\rdef\n\tgh";
        assert_eq!(offset(text, 1, 1), Some(0));
        assert_eq!(offset(text, 2, 2), Some(4));
        assert_eq!(offset(text, 3, 3), Some(8), "a lone CR ends a line");
        assert_eq!(offset(text, 4, 2), Some(11), "a tab is one byte");
    }

    #[test]
    fn only_an_external_definition_takes_a_runtime_name() {
        let d = |name: &str, external: bool| objsyms::Defined {
            name: name.to_string(),
            external,
        };
        let objects = vec![
            vec![d("main", true), d("close", false)],
            vec![d("helper", true)],
        ];
        assert_eq!(runtime_name_defined(&objects), None, "a static close");
        let objects = vec![vec![d("main", true)], vec![d("close", true)]];
        assert_eq!(runtime_name_defined(&objects), Some("close".to_string()));
        // Intel Macs: `fstat$INODE64` is fstat.
        let objects = vec![vec![d("fstat$INODE64", true)]];
        assert_eq!(runtime_name_defined(&objects), Some("fstat".to_string()));
        let objects = vec![vec![d("environ", true)]];
        assert_eq!(runtime_name_defined(&objects), Some("environ".to_string()));
    }

    #[test]
    fn a_message_holding_fatal_error_keeps_its_place() {
        let stderr = b"/m/src/a.c:12:7: error: call to 'chk' declared with 'error' attribute: size: fatal error: too big\n";
        let d = diagnostics(stderr, Cc::Clang);
        assert_eq!(d[0].at, Some((PathBuf::from("/m/src/a.c"), 12, 7)));
        assert!(d[0].message.starts_with("call to 'chk'"));
    }

    /// The names [`RUNTIME_IMPORTS`] lists are the runtime object's own
    /// imports (reserved `__` names aside).
    #[test]
    fn the_runtimes_imports_are_pinned() {
        let dir = std::env::temp_dir().join(format!("rh-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.c");
        let obj = dir.join("fnprobe.o");
        let built = std::process::Command::new("cc")
            .args(["-O2", "-c", "-fno-builtin", "-DRUHARNESS_FNPROBE_N=4", "-o"])
            .arg(&obj)
            .arg(&src)
            .status()
            .expect("cc runs");
        assert!(built.success());
        let nm = std::process::Command::new("nm")
            .arg("-u")
            .arg(&obj)
            .output()
            .expect("nm runs");
        let names: Vec<String> = String::from_utf8_lossy(&nm.stdout)
            .split_whitespace()
            .filter(|w| *w != "U")
            // Mach-O's one leading `_`; a name that still starts with `_`
            // is reserved to the implementation (`__stack_chk_fail`,
            // `___chkstk_darwin`), never a program's own.
            .map(|w| {
                if cfg!(target_os = "macos") {
                    w.strip_prefix('_').unwrap_or(w).to_string()
                } else {
                    w.to_string()
                }
            })
            // `fstat$INODE64` on Intel Macs is fstat.
            .map(|w| w.split('$').next().unwrap_or(&w).to_string())
            .filter(|w| !w.is_empty())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        for name in &names {
            let reserved = name.starts_with('_');
            assert!(
                reserved || RUNTIME_IMPORTS.contains(&name.as_str()),
                "the runtime imports {name}, which RUNTIME_IMPORTS does not list: {names:?}"
            );
        }
    }

    #[test]
    fn undefined_symbols_name_their_referrers() {
        let r = |function: Option<&str>, object: Option<&str>| Referrer {
            function: function.map(str::to_string),
            object: object.map(str::to_string),
        };
        let u = |symbol: &str, refs: Vec<Referrer>| Undefined {
            symbol: symbol.to_string(),
            refs,
        };
        let ld64 = b"Undefined symbols for architecture arm64:\n  \"_step\", referenced from:\n      _main in probed-0.o\n      _helper in probed-1.o\nld: symbol(s) not found\n";
        assert_eq!(
            undefined_symbols(ld64),
            vec![u(
                "step",
                vec![
                    r(Some("main"), Some("probed-0.o")),
                    r(Some("helper"), Some("probed-1.o"))
                ]
            )]
        );
        let gnu = b"/usr/bin/ld: /o/probed-0.o: in function `main':\nmain.c:(.text+0x9): undefined reference to `step'\n";
        assert_eq!(
            undefined_symbols(gnu),
            vec![u("step", vec![r(Some("main"), Some("/o/probed-0.o"))])]
        );
        // Review: a reference from data names no function — never the last
        // one named for another object.
        let gnu = b"/usr/bin/ld: /o/probed-0.o: in function `main':\nmain.c:(.text+0x9): undefined reference to `step'\n\
                    /usr/bin/ld: /o/probed-1.o:(.data.rel.local+0x0): undefined reference to `other'\n";
        assert_eq!(
            undefined_symbols(gnu)[1],
            u("other", vec![r(None, Some("/o/probed-1.o"))])
        );
        // Review: gcc's clones, and older binutils' "In function".
        let gnu = b"probed-0.o: In function `w.constprop.0':\nmain.c:(.text+0x9): undefined reference to `bad_size'\n";
        assert_eq!(
            undefined_symbols(gnu),
            vec![u("bad_size", vec![r(Some("w"), Some("probed-0.o"))])]
        );
        let lld = b"ld.lld: error: undefined symbol: step\n>>> referenced by main.c\n>>>               probed-0.o:(main)\n>>> referenced by d.c\n>>>               probed-1.o:(.data+0x0)\n";
        assert_eq!(
            undefined_symbols(lld),
            vec![u(
                "step",
                vec![
                    r(Some("main"), Some("probed-0.o")),
                    r(None, Some("probed-1.o"))
                ]
            )]
        );
        assert_eq!(unit_of_object("/t/ruharness-map-x/probed-12.o"), Some(12));
        assert_eq!(unit_of_object("main.o"), None);
    }
}
