//! The probed build (docs/FEATURES-PROBE-REDESIGN.md §3.4): one real compile
//! per top-level file, then the link. A note the compiler rejects is taken
//! out of exactly the function the error lands in, with the compiler's
//! words as the reason; an error no body holds is found by a search over
//! the notes; the link's undefined symbols name their functions. The
//! objects and the link are the probed program.

use crate::exec::{ChildEnd, Runner};
use crate::featuremap::MapProgress;
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
        let kind = [": fatal error: ", ": error: "]
            .iter()
            .find_map(|k| line.find(k).map(|at| (at, k.len())));
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
    pub times: &'a [(PathBuf, std::time::SystemTime)],
}

/// Per-file and whole-pass bounds (§3.4 step 6).
const PLACED_ROUNDS: usize = 8;
const FILE_COMPILES: usize = 64;
const PASS_COMPILES: usize = 400;
const LINK_ROUNDS: usize = 32;

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
        let at = offset(&text, line, col)?;
        probe
            .notes(&rel)
            .iter()
            .filter(|n| n.body.0 <= at && at < n.body.1)
            .min_by_key(|n| n.body.1 - n.body.0)
            .map(|n| (rel.clone(), n.id.clone()))
    }

    /// Compile top-level file `n` until it compiles, taking notes out as
    /// §3.4 steps 3–6 say.
    fn settle(
        &self,
        n: usize,
        probe: &mut Probe,
        pass: &mut usize,
        progress: &mut dyn MapProgress,
    ) -> Result<(), Error> {
        let mut rounds = 0;
        let mut compiles = 0;
        let rels = &self.reads[n];
        let gcc = matches!(self.cc, Cc::Gcc(_));
        let mut eliminated: Vec<(String, String)> = Vec::new();
        loop {
            progress.message(&format!(
                "Checking where the notes compile… {} (round {})",
                self.shown(&self.units[n]),
                rounds + 1
            ));
            compiles += 1;
            *pass += 1;
            let stderr = match self.compile(n, false)? {
                Outcome::Ok => {
                    // The restore pass: each note the search took out is put
                    // back alone once and kept if the file still compiles.
                    for (rel, id) in std::mem::take(&mut eliminated) {
                        let reason = probe.reasons.get(&(rel.clone(), id.clone())).cloned();
                        probe.put_back(&rel, &id);
                        self.rewrite(probe, rels, &[])?;
                        *pass += 1;
                        if !matches!(self.compile(n, false)?, Outcome::Ok) {
                            if let Some(reason) = reason {
                                probe.take_out(&rel, &id, reason);
                            }
                            self.rewrite(probe, rels, &[])?;
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
            if rounds >= PLACED_ROUNDS || compiles >= FILE_COMPILES || *pass >= PASS_COMPILES {
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
                rounds += 1;
                self.rewrite(probe, rels, &[])?;
                continue;
            }
            let why = unplaced.unwrap_or_else(|| crate::exec::stderr_excerpt(&stderr));
            if let Some(found) = self.search(n, probe, &why, &mut compiles, pass)? {
                eliminated.push(found);
            }
            rounds += 1;
        }
    }

    /// §3.4 step 4: compile with every note of the file and the probed
    /// files it reads taken out — still failing, the copy differs (a
    /// harness fault); compiling, a binary search finds one note to take
    /// out.
    fn search(
        &self,
        n: usize,
        probe: &mut Probe,
        why: &str,
        compiles: &mut usize,
        pass: &mut usize,
    ) -> Result<Option<(String, String)>, Error> {
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
        let trial = |k: usize,
                     probe: &mut Probe,
                     compiles: &mut usize,
                     pass: &mut usize|
         -> Result<bool, Error> {
            self.rewrite(probe, rels, &notes[..k])?;
            *compiles += 1;
            *pass += 1;
            Ok(matches!(self.compile(n, false)?, Outcome::Ok))
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
            if *compiles >= FILE_COMPILES || *pass >= PASS_COMPILES {
                break;
            }
            let mid = lo + (hi - lo) / 2;
            if trial(mid, probe, compiles, pass)? {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let found = notes.get(hi.saturating_sub(1)).cloned();
        if let Some((rel, id)) = &found {
            probe.take_out(rel, id, Reason::new(Kind::Elimination, why));
        }
        self.rewrite(probe, rels, &[])?;
        Ok(found)
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
    /// changed, then link; returns the probed program.
    pub(crate) fn run(
        &self,
        probe: &mut Probe,
        progress: &mut dyn MapProgress,
    ) -> Result<PathBuf, Error> {
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
        for n in 0..self.units.len() {
            if pass >= PASS_COMPILES {
                for rel in &self.reads[n] {
                    probe.unprobe(rel, Reason::new(Kind::NotChecked, ""));
                }
                self.rewrite(probe, &self.reads[n], &[])?;
            }
            self.settle(n, probe, &mut pass, progress)?;
            compiled_with.push(notes_of(probe));
        }
        // Re-compiles: a file whose probed files lost notes after it compiled.
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
        self.link(probe, progress, &mut pass)
    }

    /// §3.4 step 5: the runtime's object first, the program's objects in the
    /// plain build's order, its libraries; undefined symbols attributed.
    fn link(
        &self,
        probe: &mut Probe,
        progress: &mut dyn MapProgress,
        pass: &mut usize,
    ) -> Result<PathBuf, Error> {
        let program = self.out.join("probed");
        let link_map = self.out.join("probed.map");
        for _ in 0..LINK_ROUNDS {
            let mut argv = vec![
                "cc".to_string(),
                "-o".to_string(),
                crate::featuremap::path_text(&program)?,
            ];
            // The link's map says which object defines each name (§3.5).
            let map_text = crate::featuremap::path_text(&link_map)?;
            argv.push(match self.cc {
                Cc::Clang => format!("-Wl,-map,{map_text}"),
                Cc::Gcc(_) => format!("-Wl,-Map={map_text},--cref"),
            });
            argv.push(crate::featuremap::path_text(self.runtime)?);
            for n in 0..self.units.len() {
                argv.push(crate::featuremap::path_text(&self.object(n))?);
            }
            argv.extend(self.link_args.iter().cloned());
            let run = self.runner.tool_run(&argv)?;
            let stderr = match run.end {
                ChildEnd::Exited(status) if status.success() => {
                    let text = std::fs::read(&link_map).unwrap_or_default();
                    let objects: Vec<PathBuf> =
                        (0..self.units.len()).map(|n| self.object(n)).collect();
                    if let Some(name) = defined_by_program(&text, &objects) {
                        return Err(Error::Invariant(format!(
                            "the program defines {name}(), which the probe's runtime also uses \
                             before main — the features map cannot map it"
                        )));
                    }
                    return Ok(program);
                }
                ChildEnd::Exited(_) => run.stderr,
                _ => {
                    return Err(Error::Invariant(format!(
                        "the scratch copy's link did not finish: {}",
                        crate::exec::stderr_excerpt(&run.stderr)
                    )))
                }
            };
            let undefined = undefined_symbols(&stderr);
            if undefined.is_empty() {
                return Err(Error::Invariant(format!(
                    "the scratch copy does not link: {}",
                    crate::exec::stderr_excerpt(&stderr)
                )));
            }
            let mut changed = BTreeSet::new();
            for (symbol, referrers) in &undefined {
                let why = format!("the program does not link with its note: {symbol} is undefined");
                // (a) the symbol is a watched function still carrying a note.
                let mut done = false;
                for rel in probe.rels() {
                    if probe
                        .notes(&rel)
                        .iter()
                        .any(|note| id_name(&note.id) == symbol)
                    {
                        let id = probe
                            .notes(&rel)
                            .iter()
                            .find(|note| id_name(&note.id) == symbol)
                            .map(|note| note.id.clone())
                            .unwrap_or_default();
                        if probe.take_out(&rel, &id, Reason::new(Kind::Link, &why)) {
                            changed.insert(rel.clone());
                            done = true;
                        }
                    }
                }
                if done {
                    continue;
                }
                // (b) the referencing function the linker names.
                for referrer in referrers {
                    for rel in probe.rels() {
                        let hit = probe
                            .notes(&rel)
                            .iter()
                            .find(|note| id_name(&note.id) == referrer)
                            .map(|note| note.id.clone());
                        if let Some(id) = hit {
                            if probe.take_out(&rel, &id, Reason::new(Kind::Link, &why)) {
                                changed.insert(rel.clone());
                            }
                        }
                    }
                }
            }
            if changed.is_empty() {
                return Err(Error::Invariant(format!(
                    "the scratch copy does not link: {}",
                    crate::exec::stderr_excerpt(&stderr)
                )));
            }
            self.rewrite(probe, &changed, &[])?;
            for n in 0..self.units.len() {
                if self.reads[n].iter().any(|rel| changed.contains(rel)) {
                    self.settle(n, probe, pass, progress)?;
                }
            }
        }
        Err(Error::Invariant(
            "the scratch copy did not link within its bound — a fault in the harness, not your \
             program; please report it"
                .to_string(),
        ))
    }
}

/// The names the probe's runtime uses (docs/FEATURES-PROBE-REDESIGN.md
/// §3.5); a unit test pins them against the runtime object's imports.
pub(crate) const RUNTIME_IMPORTS: &[&str] = &["open", "fstat", "mmap", "close", "environ"];

/// The first of [`RUNTIME_IMPORTS`] the link's map says one of the
/// program's `objects` defines — ld64's map (`[ n] path` object lines, then
/// `0x… 0x… [ n] _name` symbol lines) or GNU ld's cross-reference table
/// (`name  definer` then referrers). `None` when none is, or the map cannot
/// be read.
pub(crate) fn defined_by_program(map: &[u8], objects: &[PathBuf]) -> Option<String> {
    let text = String::from_utf8_lossy(map);
    let is_program = |path: &str| {
        let path = Path::new(path.trim());
        objects.iter().any(|o| o == path)
    };
    // ld64
    let mut files: BTreeMap<usize, bool> = BTreeMap::new();
    let mut section = "";
    let bracket = |l: &str| -> Option<(usize, String)> {
        let open = l.find('[')?;
        let close = l[open..].find(']')? + open;
        let n = l[open + 1..close].trim().parse().ok()?;
        Some((n, l[close + 1..].trim().to_string()))
    };
    for line in text.lines() {
        if line.starts_with("# Object files:") {
            section = "objects";
        } else if line.starts_with("# Symbols:") {
            section = "symbols";
        } else if line.starts_with("# Sections:") || line.starts_with("# Dead Stripped Symbols:") {
            section = "";
        } else if line.starts_with('#') {
        } else if section == "objects" {
            if let Some((n, path)) = bracket(line) {
                files.insert(n, is_program(&path));
            }
        } else if section == "symbols" {
            if let Some((n, name)) = bracket(line) {
                let name = name.strip_prefix('_').unwrap_or(&name);
                if RUNTIME_IMPORTS.contains(&name) && files.get(&n) == Some(&true) {
                    return Some(name.to_string());
                }
            }
        }
    }
    // GNU ld: after "Cross Reference Table", a symbol at column 0 is
    // followed by its definer.
    if let Some(at) = text.find("Cross Reference Table") {
        for line in text[at..].lines().skip(1) {
            if line.is_empty() || line.starts_with(char::is_whitespace) {
                continue;
            }
            let mut words = line.split_whitespace();
            if let (Some(name), Some(definer)) = (words.next(), words.next()) {
                if RUNTIME_IMPORTS.contains(&name) && is_program(definer) {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// A canonical id's function name (a static's `file::name` read as `name`).
fn id_name(id: &str) -> &str {
    id.rsplit("::").next().unwrap_or(id)
}

/// The undefined symbols of a failed link, each with the functions the
/// linker says refer to it — ld64 (`"_name", referenced from:` then
/// indented `_fn in f.o` lines), GNU ld (``in function `fn':`` … ``undefined
/// reference to `name'``) and lld (`undefined symbol: name` then `>>>
/// referenced by … (fn)`). Mach-O's leading `_` dropped.
pub(crate) fn undefined_symbols(stderr: &[u8]) -> Vec<(String, Vec<String>)> {
    let text = String::from_utf8_lossy(stderr);
    let strip = |s: &str| s.strip_prefix('_').unwrap_or(s).to_string();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut gnu_function: Option<String> = None;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim();
        // ld64
        if let Some(rest) = t.strip_prefix('"') {
            if let Some(end) = rest.find("\", referenced from:") {
                let symbol = strip(&rest[..end]);
                let mut refs = Vec::new();
                while let Some(next) = lines.peek() {
                    let n = next.trim();
                    if let Some((f, _)) = n.split_once(" in ") {
                        refs.push(strip(f));
                        lines.next();
                    } else {
                        break;
                    }
                }
                out.push((symbol, refs));
                continue;
            }
        }
        // GNU ld
        if let Some(at) = t.find("in function `") {
            let rest = &t[at + "in function `".len()..];
            gnu_function = rest.split('\'').next().map(str::to_string);
            continue;
        }
        if let Some(at) = t.find("undefined reference to `") {
            let rest = &t[at + "undefined reference to `".len()..];
            if let Some(symbol) = rest.split('\'').next() {
                out.push((symbol.to_string(), gnu_function.iter().cloned().collect()));
            }
            continue;
        }
        // lld
        if let Some(rest) = t.strip_prefix("ld.lld: error: undefined symbol: ") {
            let symbol = rest.to_string();
            let mut refs = Vec::new();
            while let Some(next) = lines.peek() {
                let n = next.trim();
                if let Some(r) = n.strip_prefix(">>>") {
                    if let (Some(open), Some(close)) = (r.rfind('('), r.rfind(')')) {
                        if open < close {
                            refs.push(r[open + 1..close].to_string());
                        }
                    }
                    lines.next();
                } else {
                    break;
                }
            }
            out.push((symbol, refs));
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
    fn the_link_map_says_who_defines_the_runtimes_names() {
        let objects = vec![
            PathBuf::from("/o/probed-0.o"),
            PathBuf::from("/o/probed-1.o"),
        ];
        let ld64 = b"# Path: /o/probed\n# Object files:\n[  0] linker synthesized\n\
                     [  1] /o/fnprobe.o\n[  2] /o/probed-0.o\n[  3] /usr/lib/libSystem.tbd\n\
                     # Sections:\n# Address Size Segment Section\n\
                     # Symbols:\n# Address\tSize    \tFile  Name\n\
                     0x100003E6C\t0x00000070\t[  2] _main\n\
                     0x100003F00\t0x00000010\t[  2] _close\n";
        assert_eq!(
            defined_by_program(ld64, &objects),
            Some("close".to_string())
        );
        let fine = String::from_utf8_lossy(ld64).replace("[  2] _close", "[  3] _close");
        assert_eq!(defined_by_program(fine.as_bytes(), &objects), None);
        let gnu = b"Cross Reference Table\n\nSymbol                File\n\
                    close                 /o/probed-1.o\n                      /o/fnprobe.o\n\
                    open                  /lib/libc.so.6\n";
        assert_eq!(defined_by_program(gnu, &objects), Some("close".to_string()));
        assert_eq!(defined_by_program(b"", &objects), None);
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
        let ld64 = b"Undefined symbols for architecture arm64:\n  \"_step\", referenced from:\n      _main in probed-0.o\n      _helper in probed-1.o\nld: symbol(s) not found\n";
        assert_eq!(
            undefined_symbols(ld64),
            vec![(
                "step".to_string(),
                vec!["main".to_string(), "helper".to_string()]
            )]
        );
        let gnu = b"/usr/bin/ld: probed-0.o: in function `main':\nmain.c:(.text+0x9): undefined reference to `step'\n";
        assert_eq!(
            undefined_symbols(gnu),
            vec![("step".to_string(), vec!["main".to_string()])]
        );
        let lld = b"ld.lld: error: undefined symbol: step\n>>> referenced by main.c\n>>>               probed-0.o:(main)\n";
        assert_eq!(
            undefined_symbols(lld),
            vec![("step".to_string(), vec!["main".to_string()])]
        );
    }
}
