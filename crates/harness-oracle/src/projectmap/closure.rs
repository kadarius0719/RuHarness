//! The project map's analysis over the per-file facts
//! (docs/PROJECT-MAP-DESIGN.md §3.1 steps 6–10, §3.3, §3.5): programs and
//! their kinds, each program's closure with its duplicates, collisions,
//! outside symbols and the reasons it is incomplete, the files programs
//! share, duplicates between programs, libraries, and the files another
//! file includes.
//!
//! Pure: no I/O. Linking (§3.5) is asked of a [`Linker`]; the real one is
//! [`super::link::CcLinker`]. Without a linker every duplicate stays
//! pending and nothing is link-checked. Every output list is sorted (ids,
//! paths and names by their bytes; duplicate sets and definers by their
//! index, which follows path order), so two runs give the same output.
//!
//! The linker's rules (§3.1 step 7): a definition is *strong* unless it is
//! weak or common. Of a symbol's definers among non-program files, two or
//! more strong ones are a duplicate; one strong one defines it whatever weak
//! or common ones stand beside it — so a symbol the closure defines only
//! weakly (or as common) pulls in its single strong definer, recorded as
//! `strong_over_weak`; with no strong one the first in path order defines
//! it. Over a finished closure, a symbol two or more of its files define
//! strongly is a collision. A fuzzer's closure starts from it and its
//! driver (the driver file itself is not listed).

use super::ids;
use super::{Compiled, DefinedSymbol, FileFacts, FileKind, WalkIssue};
use harness_core::error::Error;
use std::collections::{BTreeMap, BTreeSet};

/// The most definers a duplicate set may have for linking to settle it
/// (§3.5 step 2).
pub const MAX_DEFINERS: usize = 4;
/// The most choices (combinations) linking tries for one program.
pub const MAX_CHOICES: usize = 16;
/// The symbol a fuzzer defines (libFuzzer's entry point).
pub const FUZZ_ENTRY: &str = "LLVMFuzzerTestOneInput";

/// What the parser read from one file (`harness_scan::file_facts`): the
/// functions it defines and the names it calls. Used for a file that did
/// not compile, whose object facts are empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParserFacts {
    /// Functions defined.
    pub defines: BTreeSet<String>,
    /// Names called.
    pub calls: BTreeSet<String>,
}

/// What the analysis reads.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// Every walked file's facts (any order).
    pub files: &'a [FileFacts],
    /// The parser's facts by path (a path missing here has none).
    pub parser: &'a BTreeMap<String, ParserFacts>,
    /// The walk's issues: one whose reason starts `cannot be read` is a
    /// folder (or entry) the walk could not read.
    pub walk_issues: &'a [WalkIssue],
    /// `(path, id)` of each accepted tool: a program or library at that
    /// path keeps the id.
    pub accepted: &'a [(String, String)],
}

/// A program's kind (§3.1 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProgramKind {
    /// Defines a function `main`.
    Main,
    /// Defines `LLVMFuzzerTestOneInput` and no `main`.
    Fuzz,
    /// A `main` whose need is met by two or more fuzzers.
    Driver,
}

impl ProgramKind {
    /// `main`, `fuzz` or `driver`.
    pub fn as_str(self) -> &'static str {
        match self {
            ProgramKind::Main => "main",
            ProgramKind::Fuzz => "fuzz",
            ProgramKind::Driver => "driver",
        }
    }
}

/// The label guessed from a program's first folder: never a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KindGuess {
    /// Anything else.
    Tool,
    /// `tests`, `test`, `fuzz`, `ossfuzz`, `fuzzers`.
    Test,
    /// `examples`.
    Example,
    /// `bench`, `benchmarks`.
    Benchmark,
}

impl KindGuess {
    /// `tool`, `test`, `example` or `benchmark`.
    pub fn as_str(self) -> &'static str {
        match self {
            KindGuess::Tool => "tool",
            KindGuess::Test => "test",
            KindGuess::Example => "example",
            KindGuess::Benchmark => "benchmark",
        }
    }

    /// The guess for a relative path, from its first folder ignoring case.
    pub fn of(path: &str) -> KindGuess {
        let Some((first, _)) = path.split_once('/') else {
            return KindGuess::Tool;
        };
        match first.to_ascii_lowercase().as_str() {
            "tests" | "test" | "fuzz" | "ossfuzz" | "fuzzers" => KindGuess::Test,
            "examples" => KindGuess::Example,
            "bench" | "benchmarks" => KindGuess::Benchmark,
            _ => KindGuess::Tool,
        }
    }
}

/// One program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// `t-…` (§3.3).
    pub id: String,
    /// `p1…` for a `main` program, in path order.
    pub index: Option<String>,
    /// Its file.
    pub path: String,
    /// Its kind.
    pub kind: ProgramKind,
    /// The folder-name guess.
    pub kind_guess: KindGuess,
    /// A driver's fuzzers, sorted (empty otherwise).
    pub serves: Vec<String>,
}

/// Why a closure is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IncompleteWhy {
    /// A duplicate set is still pending.
    Pending,
    /// A file that was parsed but did not compile defines (by the parser's
    /// facts) one of the closure's outside symbols.
    MayBeDefinedIn,
    /// The closure has outside symbols and this `.c` was neither parsed
    /// nor compiled.
    Unread,
    /// The walk could not read this folder.
    UnreadableFolder,
}

impl IncompleteWhy {
    /// `pending`, `may-be-defined-in`, `unread` or `unreadable-folder`.
    pub fn as_str(self) -> &'static str {
        match self {
            IncompleteWhy::Pending => "pending",
            IncompleteWhy::MayBeDefinedIn => "may-be-defined-in",
            IncompleteWhy::Unread => "unread",
            IncompleteWhy::UnreadableFolder => "unreadable-folder",
        }
    }
}

/// One reason a closure is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Incomplete {
    /// The reason.
    pub why: IncompleteWhy,
    /// The file or folder it is about (none for `pending`).
    pub path: Option<String>,
    /// The symbols it is about, sorted (the pending set's symbols, those the
    /// file may define, the outside symbols an unread file may define).
    pub symbols: Vec<String>,
}

/// A need met only by another program's file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NeedsFrom {
    /// The symbol.
    pub sym: String,
    /// The program's id.
    pub program: String,
}

/// One definer of a duplicate set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definer {
    /// `d1.1…`, in path order.
    pub index: String,
    /// Its file.
    pub path: String,
}

/// A duplicate settled by linking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The kept definer's index.
    pub keep: String,
    /// How it was settled: `links`.
    pub by: &'static str,
}

/// A set of two or more strong definers of a needed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// `d1…`, once per project.
    pub set: String,
    /// This closure's symbols the set defines, sorted.
    pub symbols: Vec<String>,
    /// The definers, in path order.
    pub definers: Vec<Definer>,
    /// The definers whose choice linked.
    pub links: Vec<String>,
    /// The choice linking made, when exactly one linked.
    pub choice: Option<Choice>,
    /// The choices (definer indexes) this set is reached under, when not
    /// every choice reaches it (empty when it is reached whatever is kept);
    /// once settled, only the kept one.
    pub under: Vec<String>,
}

/// A symbol a closure file defines only weakly (or as common) while another
/// file of the closure defines it strongly: the strong one defines it, so a
/// need met only weakly pulls in its single strong definer (§3.1 step 7).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StrongOverWeak {
    /// The symbol.
    pub sym: String,
    /// The files defining it weakly or as common, sorted.
    pub weak: Vec<String>,
    /// The files defining it strongly, sorted.
    pub strong: Vec<String>,
}

/// A symbol two or more of a closure's files define strongly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Collision {
    /// The symbol.
    pub sym: String,
    /// Its definers, sorted.
    pub definers: Vec<String>,
}

/// What a link check proved: from the symbol facts and the linker's exit
/// status, never from its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Linked {
    /// It linked.
    Ok,
    /// It did not.
    Failed {
        /// Symbols nothing linked provides, sorted.
        missing: Vec<String>,
        /// Symbols two linked files define strongly, sorted.
        doubled: Vec<String>,
        /// Unresolved symbols the probe budget left undecided, sorted:
        /// neither proven missing nor provided.
        not_checked: Vec<String>,
        /// Files that did not compile for the link, sorted.
        not_compiled: Vec<String>,
    },
}

impl Linked {
    fn missing_count(&self) -> usize {
        match self {
            Linked::Ok => 0,
            Linked::Failed { missing, .. } => missing.len(),
        }
    }
}

/// One program's closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closure {
    /// The program's id.
    pub program: String,
    /// Its files (the program's own included), sorted.
    pub files: Vec<String>,
    /// Any reason below.
    pub incomplete: bool,
    /// Why it is incomplete, sorted.
    pub incomplete_why: Vec<Incomplete>,
    /// Needed symbols no project file defines, sorted.
    pub outside: Vec<String>,
    /// Needs met only by another program's file, sorted.
    pub needs_from: Vec<NeedsFrom>,
    /// Its duplicate sets, by index.
    pub duplicates: Vec<Duplicate>,
    /// Its collisions, sorted.
    pub collisions: Vec<Collision>,
    /// Symbols its files define weakly and strongly, by symbol.
    pub strong_over_weak: Vec<StrongOverWeak>,
    /// The link check (none when not checked).
    pub linked: Option<Linked>,
    /// Sets linking could not settle (held): questions for the person.
    pub questions: Vec<String>,
}

/// A file in two or more closures.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Shared {
    /// The file.
    pub file: String,
    /// The programs' ids, sorted.
    pub programs: Vec<String>,
}

/// A symbol defined strongly by files that never meet in one closure:
/// listed, never asked.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BetweenDuplicate {
    /// The symbol.
    pub sym: String,
    /// Its definers, sorted.
    pub definers: Vec<String>,
}

/// A group of unreached `.c` files joined by the symbols they need.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Library {
    /// `l-…` (§3.3).
    pub id: String,
    /// Its files, sorted.
    pub files: Vec<String>,
    /// Files outside it that it needs, sorted.
    pub needs_from_outside: Vec<String>,
}

/// A `.c` another file includes as text (§3.1 step 10).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IncludedBy {
    /// The `.c`.
    pub file: String,
    /// The files including it, sorted.
    pub by: Vec<String>,
}

/// The whole analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// Programs, by id.
    pub programs: Vec<Program>,
    /// Files whose parser facts define `main` but that did not compile.
    pub programs_not_compiled: Vec<String>,
    /// One per `main` and `fuzz` program, by program id.
    pub closures: Vec<Closure>,
    /// Duplicates between programs that never meet, by symbol.
    pub between_program_duplicates: Vec<BetweenDuplicate>,
    /// Shared files, by path.
    pub shared: Vec<Shared>,
    /// Libraries, by id.
    pub libraries: Vec<Library>,
    /// Textual dependencies, by path.
    pub included_by: Vec<IncludedBy>,
}

/// Links a set of files into one program (§3.5). The real one compiles and
/// links under the map's sandbox; tests use a fake.
pub trait Linker {
    /// Link `files` (in path order). `unresolved` lists, sorted, the
    /// symbols they need that none of them defines: the outside symbols
    /// and any need of another program.
    fn link(&mut self, files: &[&FileFacts], unresolved: &[String]) -> Result<Linked, Error>;

    /// No closure still to be linked needs `path`: its object may go.
    fn release(&mut self, _path: &str) {}

    /// The linker stopped (the map's deadline passed): its results are not
    /// to be trusted, and the analysis stops.
    fn stopped(&self) -> bool {
        false
    }
}

/// A definition counts as strong unless it is weak or common.
pub(crate) fn strong(d: &DefinedSymbol) -> bool {
    !d.weak && d.kind != "common"
}

fn compiled_ok(f: &FileFacts) -> bool {
    f.compiled == Some(Compiled::Ok)
}

/// Symbols two or more of `files` define strongly, with their definers.
pub(crate) fn doubled(files: &[&FileFacts]) -> BTreeMap<String, Vec<String>> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        for d in f.defined.iter().filter(|d| strong(d)) {
            by.entry(d.name.clone()).or_default().push(f.path.clone());
        }
    }
    by.retain(|_, v| {
        v.sort();
        v.dedup();
        v.len() >= 2
    });
    by
}

/// Needs of `files` none of them defines, sorted.
pub(crate) fn unresolved(files: &[&FileFacts]) -> Vec<String> {
    let defined: BTreeSet<&str> = files
        .iter()
        .flat_map(|f| f.defined.iter().map(|d| d.name.as_str()))
        .collect();
    let needs: BTreeSet<&str> = files
        .iter()
        .flat_map(|f| f.needed.iter().map(|n| n.name.as_str()))
        .filter(|n| !defined.contains(n))
        .collect();
    needs.into_iter().map(str::to_string).collect()
}

/// A duplicate set: its definers' file numbers, ascending (= path order).
type SetKey = Vec<usize>;
/// The definer kept for each set.
type Choices = BTreeMap<SetKey, usize>;

/// How a needed symbol resolves among the project's files.
enum Res {
    /// One file defines it (by the linker's rules).
    One(usize),
    /// Two or more strong definers.
    Dup(SetKey),
    /// Only program files define it.
    Programs(Vec<usize>),
    /// No project file defines it.
    Outside,
}

/// One closure as computed for a given set of choices.
#[derive(Debug, Clone, Default)]
struct Core {
    files: BTreeSet<usize>,
    pending: BTreeMap<SetKey, BTreeSet<String>>,
    outside: BTreeSet<String>,
    needs_from: BTreeSet<(String, usize)>,
}

/// A choice: a set and the definer kept for it.
type Pick = (SetKey, usize);

/// A set seen while exploring a program's choices.
#[derive(Debug, Clone, Default)]
struct Seen {
    symbols: BTreeSet<String>,
    /// Raised by a closure made with no choice at all.
    direct: bool,
    /// The last choice made before each closure that raised it.
    reached: BTreeSet<Pick>,
}

/// What exploring a program's choices found.
struct Explored {
    leaves: Vec<(Choices, Core)>,
    sets: BTreeMap<SetKey, Seen>,
    over: bool,
}

impl Explored {
    /// The choices `key` is reached under, when not every choice reaches
    /// it: empty for a set reached with no choice, or reached by every
    /// definer of the set whose choice raised it (then that set's own
    /// choices count, in turn).
    fn under(&self, key: &SetKey) -> Vec<Pick> {
        self.under_at(key, 0)
    }

    fn under_at(&self, key: &SetKey, depth: usize) -> Vec<Pick> {
        let Some(seen) = self.sets.get(key) else {
            return Vec::new();
        };
        // The nesting is at most as deep as the sets are many.
        if seen.direct || depth > self.sets.len() {
            return Vec::new();
        }
        let mut by_parent: BTreeMap<&SetKey, BTreeSet<usize>> = BTreeMap::new();
        for (parent, d) in &seen.reached {
            by_parent.entry(parent).or_default().insert(*d);
        }
        let mut out = BTreeSet::new();
        for (parent, ds) in by_parent {
            if parent.iter().all(|d| ds.contains(d)) {
                // Every choice of the parent reaches it: as reached as the
                // parent is.
                let above = self.under_at(parent, depth + 1);
                if above.is_empty() {
                    return Vec::new();
                }
                out.extend(above);
            } else {
                out.extend(ds.into_iter().map(|d| (parent.clone(), d)));
            }
        }
        out.into_iter().collect()
    }
}

/// A duplicate set in one program's result, before numbering.
#[derive(Debug, Clone)]
struct Dup {
    key: SetKey,
    symbols: BTreeSet<String>,
    links: BTreeSet<usize>,
    keep: Option<usize>,
    under: Vec<Pick>,
}

/// One program's result, before numbering.
struct Outcome {
    core: Core,
    dups: Vec<Dup>,
    linked: Option<Linked>,
    held: bool,
}

/// The project's files and symbol index.
struct Project<'a> {
    /// Sorted by path: a file's number is its place here.
    files: Vec<&'a FileFacts>,
    by_path: BTreeMap<&'a str, usize>,
    program: Vec<bool>,
    /// Definers among compiled non-program files: `(file, strong)`, in path
    /// order.
    definers: BTreeMap<&'a str, Vec<(usize, bool)>>,
    /// Program files defining a symbol, in path order.
    program_definers: BTreeMap<&'a str, Vec<usize>>,
}

impl<'a> Project<'a> {
    fn new(files: Vec<&'a FileFacts>, program: Vec<bool>) -> Project<'a> {
        let by_path = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.as_str(), i))
            .collect();
        let mut definers: BTreeMap<&str, Vec<(usize, bool)>> = BTreeMap::new();
        let mut program_definers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, f) in files.iter().enumerate() {
            if f.kind != FileKind::C || !compiled_ok(f) {
                continue;
            }
            for d in &f.defined {
                if program[i] {
                    program_definers.entry(&d.name).or_default().push(i);
                } else {
                    definers.entry(&d.name).or_default().push((i, strong(d)));
                }
            }
        }
        Project {
            files,
            by_path,
            program,
            definers,
            program_definers,
        }
    }

    fn resolve(&self, sym: &str) -> Res {
        let Some(defs) = self.definers.get(sym) else {
            return match self.program_definers.get(sym) {
                Some(p) => Res::Programs(p.clone()),
                None => Res::Outside,
            };
        };
        let strong: Vec<usize> = defs.iter().filter(|d| d.1).map(|d| d.0).collect();
        match strong.len() {
            0 => Res::One(defs[0].0),
            1 => Res::One(strong[0]),
            _ => Res::Dup(strong),
        }
    }

    /// The single strong definer (among non-program files) of `sym`, when
    /// there is exactly one.
    fn single_strong(&self, sym: &str) -> Option<usize> {
        let defs = self.definers.get(sym)?;
        let mut strong = defs.iter().filter(|d| d.1);
        match (strong.next(), strong.next()) {
            (Some(&(d, _)), None) => Some(d),
            _ => None,
        }
    }

    /// The closure from `starts` with `choices` (§3.1 step 7), from
    /// scratch: each round adds every single definer of a need the set does
    /// not yet meet, and the single strong definer of a symbol the set
    /// defines only weakly (the strong one defines it), until a round adds
    /// nothing. A need met only by a program file other than the starts is
    /// recorded, never pulled in.
    fn closure(&self, starts: &BTreeSet<usize>, choices: &Choices) -> Core {
        let mut files: BTreeSet<usize> = starts.clone();
        loop {
            let defined = self.defined_in(&files);
            let strongly = self.strongly_defined_in(&files);
            let mut add = BTreeSet::new();
            for &f in &files {
                for n in &self.files[f].needed {
                    if defined.contains(n.name.as_str()) {
                        continue;
                    }
                    match self.resolve(&n.name) {
                        Res::One(d) => {
                            add.insert(d);
                        }
                        Res::Dup(key) => {
                            if let Some(&keep) = choices.get(&key) {
                                add.insert(keep);
                            }
                        }
                        Res::Programs(_) | Res::Outside => {}
                    }
                }
            }
            // Met only weakly: the single strong definer joins.
            for sym in defined.difference(&strongly) {
                if let Some(d) = self.single_strong(sym) {
                    add.insert(d);
                }
            }
            let before = files.len();
            files.extend(add);
            if files.len() == before {
                break;
            }
        }
        let defined = self.defined_in(&files);
        let mut core = Core {
            files,
            ..Core::default()
        };
        for &f in &core.files {
            for n in &self.files[f].needed {
                if defined.contains(n.name.as_str()) {
                    continue;
                }
                match self.resolve(&n.name) {
                    Res::Dup(key) => {
                        core.pending.entry(key).or_default().insert(n.name.clone());
                    }
                    Res::Programs(ps) => {
                        for p in ps.into_iter().filter(|p| !starts.contains(p)) {
                            core.needs_from.insert((n.name.clone(), p));
                        }
                    }
                    Res::Outside => {
                        core.outside.insert(n.name.clone());
                    }
                    // A single definer is always added above.
                    Res::One(_) => {}
                }
            }
        }
        core
    }

    fn defined_in(&self, files: &BTreeSet<usize>) -> BTreeSet<&'a str> {
        files
            .iter()
            .flat_map(|&f| self.files[f].defined.iter().map(|d| d.name.as_str()))
            .collect()
    }

    fn strongly_defined_in(&self, files: &BTreeSet<usize>) -> BTreeSet<&'a str> {
        files
            .iter()
            .flat_map(|&f| {
                self.files[f]
                    .defined
                    .iter()
                    .filter(|d| strong(d))
                    .map(|d| d.name.as_str())
            })
            .collect()
    }

    /// Symbols `files` define weakly (or as common) and strongly.
    fn strong_over_weak(&self, files: &BTreeSet<usize>) -> Vec<StrongOverWeak> {
        let mut by: BTreeMap<&str, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
        for &f in files {
            for d in &self.files[f].defined {
                let at = by.entry(&d.name).or_default();
                if strong(d) {
                    at.1.insert(self.files[f].path.clone());
                } else {
                    at.0.insert(self.files[f].path.clone());
                }
            }
        }
        by.into_iter()
            .filter(|(_, (weak, strong))| !weak.is_empty() && !strong.is_empty())
            .map(|(sym, (weak, strong))| StrongOverWeak {
                sym: sym.to_string(),
                weak: weak.into_iter().collect(),
                strong: strong.into_iter().collect(),
            })
            .collect()
    }

    /// Every combination of choices for `start`'s pending sets, a set
    /// raised under a choice expanded in turn (§3.5 step 2); `over` when a
    /// set has more than [`MAX_DEFINERS`] or the combinations pass
    /// [`MAX_CHOICES`].
    fn explore(&self, start: usize) -> Explored {
        let mut out = Explored {
            leaves: Vec::new(),
            sets: BTreeMap::new(),
            over: false,
        };
        self.explore_from(start, Choices::new(), None, &mut out);
        out
    }

    fn explore_from(&self, start: usize, choices: Choices, last: Option<Pick>, out: &mut Explored) {
        if out.over {
            return;
        }
        let core = self.closure(&BTreeSet::from([start]), &choices);
        for (key, symbols) in &core.pending {
            let seen = out.sets.entry(key.clone()).or_default();
            seen.symbols.extend(symbols.iter().cloned());
            match &last {
                None => seen.direct = true,
                Some(pick) => {
                    seen.reached.insert(pick.clone());
                }
            }
        }
        let Some(key) = core.pending.keys().next().cloned() else {
            if out.leaves.len() == MAX_CHOICES {
                out.over = true;
            } else {
                out.leaves.push((choices, core));
            }
            return;
        };
        if key.len() > MAX_DEFINERS {
            out.over = true;
            return;
        }
        for &d in &key {
            let mut next = choices.clone();
            next.insert(key.clone(), d);
            self.explore_from(start, next, Some((key.clone(), d)), out);
        }
    }

    fn refs(&self, files: &BTreeSet<usize>) -> Vec<&'a FileFacts> {
        files.iter().map(|&f| self.files[f]).collect()
    }

    fn link(&self, linker: &mut dyn Linker, files: &BTreeSet<usize>) -> Result<Linked, Error> {
        let refs = self.refs(files);
        linker.link(&refs, &unresolved(&refs))
    }

    /// Every file a `main` program's link checks may compile: its closure
    /// under each choice.
    fn may_link(&self, start: usize) -> BTreeSet<usize> {
        let base = self.closure(&BTreeSet::from([start]), &Choices::new());
        if base.pending.is_empty() {
            return base.files;
        }
        let explored = self.explore(start);
        let mut all = base.files;
        for (_, core) in &explored.leaves {
            all.extend(core.files.iter().copied());
        }
        all
    }

    /// A `main` program: its closure, its duplicates settled by linking
    /// when a linker is given (§3.5).
    fn settle<'l>(
        &self,
        start: usize,
        linker: Option<&mut (dyn Linker + 'l)>,
    ) -> Result<Outcome, Error> {
        let base = self.closure(&BTreeSet::from([start]), &Choices::new());
        if base.pending.is_empty() {
            let linked = match linker {
                Some(l) => Some(self.link(l, &base.files)?),
                None => None,
            };
            return Ok(Outcome {
                core: base,
                dups: Vec::new(),
                linked,
                held: false,
            });
        }
        let explored = self.explore(start);
        let seen = |links: &BTreeMap<SetKey, BTreeSet<usize>>| -> Vec<Dup> {
            explored
                .sets
                .iter()
                .map(|(key, s)| Dup {
                    key: key.clone(),
                    symbols: s.symbols.clone(),
                    links: links.get(key).cloned().unwrap_or_default(),
                    keep: None,
                    under: explored.under(key),
                })
                .collect()
        };
        let pending = |dups: Vec<Dup>, linked: Option<Linked>, held: bool| Outcome {
            core: base.clone(),
            dups,
            linked,
            held,
        };
        let Some(linker) = linker else {
            return Ok(pending(seen(&BTreeMap::new()), None, false));
        };
        if explored.over {
            return Ok(pending(seen(&BTreeMap::new()), None, true));
        }
        let mut results = Vec::new();
        for (_, core) in &explored.leaves {
            results.push(self.link(linker, &core.files)?);
            if linker.stopped() {
                return Ok(pending(seen(&BTreeMap::new()), None, false));
            }
        }
        let ok: Vec<usize> = (0..results.len())
            .filter(|&i| results[i] == Linked::Ok)
            .collect();
        match ok.as_slice() {
            [one] => {
                let (choices, core) = &explored.leaves[*one];
                let dups = choices
                    .iter()
                    .map(|(key, &keep)| Dup {
                        key: key.clone(),
                        symbols: explored.sets[key].symbols.clone(),
                        links: BTreeSet::from([keep]),
                        keep: Some(keep),
                        // Once settled, only the kept choice it was reached
                        // under.
                        under: explored
                            .under(key)
                            .into_iter()
                            .filter(|(k, d)| choices.get(k) == Some(d))
                            .collect(),
                    })
                    .collect();
                Ok(Outcome {
                    core: core.clone(),
                    dups,
                    linked: Some(Linked::Ok),
                    held: false,
                })
            }
            [] => {
                // The choice that left the fewest missing (the first on a tie).
                let best = results.iter().min_by_key(|r| r.missing_count()).cloned();
                Ok(pending(seen(&BTreeMap::new()), best, false))
            }
            several => {
                let mut links: BTreeMap<SetKey, BTreeSet<usize>> = BTreeMap::new();
                for &i in several {
                    for (key, &d) in &explored.leaves[i].0 {
                        links.entry(key.clone()).or_default().insert(d);
                    }
                }
                Ok(pending(seen(&links), None, true))
            }
        }
    }
}

/// The kinds of the compiled `.c` files that are programs, by file number.
fn find_programs(project_files: &[&FileFacts]) -> BTreeMap<usize, (ProgramKind, Vec<usize>)> {
    let has = |f: &FileFacts, name: &str| f.defined.iter().any(|d| d.name == name);
    let mut mains = Vec::new();
    let mut fuzz = BTreeSet::new();
    for (i, f) in project_files.iter().enumerate() {
        if f.kind != FileKind::C || !compiled_ok(f) {
            continue;
        }
        if f.defined
            .iter()
            .any(|d| d.name == "main" && d.kind == "function")
        {
            mains.push(i);
        } else if has(f, FUZZ_ENTRY) {
            // A data `main` beside the entry point is no program's `main`.
            fuzz.insert(i);
        }
    }
    let mut out = BTreeMap::new();
    for &i in &fuzz {
        out.insert(i, (ProgramKind::Fuzz, Vec::new()));
    }
    for m in mains {
        let serves: BTreeSet<usize> = project_files[m]
            .needed
            .iter()
            .flat_map(|n| {
                fuzz.iter()
                    .copied()
                    .filter(|&z| has(project_files[z], &n.name))
                    .collect::<Vec<_>>()
            })
            .collect();
        if serves.len() >= 2 {
            out.insert(m, (ProgramKind::Driver, serves.into_iter().collect()));
        } else {
            out.insert(m, (ProgramKind::Main, Vec::new()));
        }
    }
    out
}

/// The analysis (see the module docs). `linker` settles duplicates and
/// link-checks programs; without one nothing is linked and every duplicate
/// stays pending. An `Err` comes only from the linker.
pub fn analyze(input: &Input<'_>, mut linker: Option<&mut dyn Linker>) -> Result<Analysis, Error> {
    let mut files: Vec<&FileFacts> = input.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let kinds = find_programs(&files);
    let is_program: Vec<bool> = (0..files.len()).map(|i| kinds.contains_key(&i)).collect();
    let project = Project::new(files, is_program);
    let files = &project.files;
    let path = |i: usize| files[i].path.clone();

    // Programs, their ids and indexes.
    let program_paths: Vec<String> = kinds.keys().map(|&i| path(i)).collect();
    let ids = ids::assign("t-", &program_paths, input.accepted);
    let id_of = |i: usize| ids[&files[i].path].clone();
    let mut programs = Vec::new();
    let mut main_no = 0;
    for (&i, (kind, serves)) in &kinds {
        let index = (*kind == ProgramKind::Main).then(|| {
            main_no += 1;
            format!("p{main_no}")
        });
        programs.push(Program {
            id: id_of(i),
            index,
            path: path(i),
            kind: *kind,
            kind_guess: KindGuess::of(&files[i].path),
            serves: serves.iter().map(|&z| path(z)).collect(),
        });
    }
    programs.sort_by(|a, b| a.id.cmp(&b.id));

    let not_compiled: Vec<usize> = (0..files.len())
        .filter(|&i| {
            let f = files[i];
            f.kind == FileKind::C
                && matches!(f.compiled, Some(Compiled::Failed { .. }))
                && input
                    .parser
                    .get(&f.path)
                    .is_some_and(|p| p.defines.contains("main"))
        })
        .collect();

    // Closures: `main` programs settled by linking, fuzzers linked with the
    // project's driver.
    // A fuzzer's closure starts from it and its driver (the driver's own
    // needs join the fuzzer's link); the driver file itself is not listed.
    let driver_of = |i: usize| {
        kinds
            .iter()
            .find(|(_, (k, serves))| *k == ProgramKind::Driver && serves.contains(&i))
            .map(|(&d, _)| d)
    };
    let fuzz_starts = |i: usize| {
        let mut starts = BTreeSet::from([i]);
        starts.extend(driver_of(i));
        starts
    };
    // Each file's last program to link it, so its object goes once no
    // closure still to be linked needs it.
    let mut last_use: BTreeMap<usize, usize> = BTreeMap::new();
    if linker.is_some() {
        for (&i, (kind, _)) in &kinds {
            let may = match kind {
                ProgramKind::Main => project.may_link(i),
                ProgramKind::Fuzz => project.closure(&fuzz_starts(i), &Choices::new()).files,
                ProgramKind::Driver => BTreeSet::new(),
            };
            for f in may {
                last_use.insert(f, i);
            }
        }
    }
    let mut outcomes: BTreeMap<usize, Outcome> = BTreeMap::new();
    for (&i, (kind, _)) in &kinds {
        match kind {
            ProgramKind::Main => {
                let out = project.settle(i, linker.as_deref_mut())?;
                outcomes.insert(i, out);
            }
            ProgramKind::Fuzz => {
                let driver = driver_of(i);
                let mut core = project.closure(&fuzz_starts(i), &Choices::new());
                let linked = match (driver, linker.as_deref_mut()) {
                    (Some(_), Some(l)) if core.pending.is_empty() => {
                        Some(project.link(l, &core.files)?)
                    }
                    _ => None,
                };
                if let Some(d) = driver {
                    core.files.remove(&d);
                }
                let dups = core
                    .pending
                    .iter()
                    .map(|(key, symbols)| Dup {
                        key: key.clone(),
                        symbols: symbols.clone(),
                        links: BTreeSet::new(),
                        keep: None,
                        under: Vec::new(),
                    })
                    .collect();
                outcomes.insert(
                    i,
                    Outcome {
                        core,
                        dups,
                        linked,
                        held: false,
                    },
                );
            }
            ProgramKind::Driver => {}
        }
        if let Some(l) = linker.as_deref_mut() {
            if l.stopped() {
                break;
            }
            for (&f, _) in last_use.iter().filter(|(_, &p)| p == i) {
                l.release(&files[f].path);
            }
        }
    }

    // Set indexes, once per project (§3.3): sets reached directly first,
    // then those reached only under a choice, each group by its definers'
    // sorted paths (file numbers follow path order).
    let mut direct: BTreeSet<SetKey> = BTreeSet::new();
    let mut all: BTreeSet<SetKey> = BTreeSet::new();
    for out in outcomes.values() {
        for d in &out.dups {
            all.insert(d.key.clone());
            if d.under.is_empty() {
                direct.insert(d.key.clone());
            }
        }
    }
    let mut set_no: BTreeMap<SetKey, usize> = BTreeMap::new();
    for key in direct
        .iter()
        .chain(all.iter().filter(|k| !direct.contains(*k)))
    {
        let n = set_no.len() + 1;
        set_no.entry(key.clone()).or_insert(n);
    }
    let set_index = |key: &SetKey| format!("d{}", set_no[key]);
    let definer_index = |key: &SetKey, file: usize| {
        let at = key.iter().position(|&f| f == file).unwrap_or(0);
        format!("d{}.{}", set_no[key], at + 1)
    };

    // The facts every closure's incompleteness reads.
    let gaps = Gaps::new(input, files);

    let mut closures = Vec::new();
    let mut alternatives: BTreeSet<usize> = BTreeSet::new();
    let mut set_symbols: BTreeSet<String> = BTreeSet::new();
    let mut collision_symbols: BTreeSet<String> = BTreeSet::new();
    for (&i, out) in &outcomes {
        let core = &out.core;
        let mut why = Vec::new();
        let mut duplicates = Vec::new();
        let mut dups = out.dups.clone();
        dups.sort_by_key(|d| set_no[&d.key]);
        for d in &dups {
            alternatives.extend(d.key.iter().copied());
            set_symbols.extend(d.symbols.iter().cloned());
            if d.keep.is_none() {
                why.push(Incomplete {
                    why: IncompleteWhy::Pending,
                    path: None,
                    symbols: d.symbols.iter().cloned().collect(),
                });
            }
            duplicates.push(Duplicate {
                set: set_index(&d.key),
                symbols: d.symbols.iter().cloned().collect(),
                definers: d
                    .key
                    .iter()
                    .map(|&f| Definer {
                        index: definer_index(&d.key, f),
                        path: path(f),
                    })
                    .collect(),
                links: d.links.iter().map(|&f| definer_index(&d.key, f)).collect(),
                choice: d.keep.map(|k| Choice {
                    keep: definer_index(&d.key, k),
                    by: "links",
                }),
                under: {
                    let mut under: Vec<(usize, usize, String)> = d
                        .under
                        .iter()
                        .map(|(k, f)| {
                            let at = k.iter().position(|x| x == f).unwrap_or(0);
                            (set_no[k], at, definer_index(k, *f))
                        })
                        .collect();
                    under.sort();
                    under.into_iter().map(|u| u.2).collect()
                },
            });
        }
        why.extend(gaps.why(&core.outside));
        why.sort();
        let refs = project.refs(&core.files);
        let collisions: Vec<Collision> = doubled(&refs)
            .into_iter()
            .map(|(sym, definers)| Collision { sym, definers })
            .collect();
        collision_symbols.extend(collisions.iter().map(|c| c.sym.clone()));
        let questions = if out.held {
            duplicates.iter().map(|d| d.set.clone()).collect()
        } else {
            Vec::new()
        };
        let mut needs_from: Vec<NeedsFrom> = core
            .needs_from
            .iter()
            .map(|(sym, p)| NeedsFrom {
                sym: sym.clone(),
                program: id_of(*p),
            })
            .collect();
        needs_from.sort();
        closures.push(Closure {
            program: id_of(i),
            files: core.files.iter().map(|&f| path(f)).collect(),
            incomplete: !why.is_empty(),
            incomplete_why: why,
            outside: core.outside.iter().cloned().collect(),
            needs_from,
            duplicates,
            collisions,
            strong_over_weak: project.strong_over_weak(&core.files),
            linked: out.linked.clone(),
            questions,
        });
    }
    closures.sort_by(|a, b| a.program.cmp(&b.program));

    // Shared files, after the choices.
    let mut holders: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for (&i, out) in &outcomes {
        for &f in &out.core.files {
            holders.entry(f).or_default().insert(id_of(i));
        }
    }
    let shared: Vec<Shared> = holders
        .iter()
        .filter(|(_, p)| p.len() >= 2)
        .map(|(&f, p)| Shared {
            file: path(f),
            programs: p.iter().cloned().collect(),
        })
        .collect();

    // Libraries: unreached `.c` files, grouped by single-definer needs.
    let in_closure: BTreeSet<usize> = holders.keys().copied().collect();
    let through_failed = reached_through(&project, &not_compiled, input.parser);
    let unreached: Vec<usize> = (0..files.len())
        .filter(|&f| {
            files[f].kind == FileKind::C
                && compiled_ok(files[f])
                && !project.program[f]
                && !in_closure.contains(&f)
                && !alternatives.contains(&f)
                && !through_failed.contains(&f)
        })
        .collect();
    let libraries = libraries(&project, &unreached, input.accepted);

    // Textual dependencies.
    let mut includers: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for f in files.iter() {
        for inc in &f.includes {
            if let Some(&t) = project.by_path.get(inc.as_str()) {
                if files[t].kind == FileKind::C {
                    includers
                        .entry(files[t].path.as_str())
                        .or_default()
                        .insert(f.path.clone());
                }
            }
        }
    }

    // Duplicates between programs that never meet: definers in the
    // programs' closures only (a library's file is no program's), a `.c`
    // another file includes as text left out (its includer defines it).
    let mut strong_by: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for &i in &in_closure {
        let f = files[i];
        if f.kind != FileKind::C || !compiled_ok(f) || includers.contains_key(f.path.as_str()) {
            continue;
        }
        for d in f.defined.iter().filter(|d| strong(d)) {
            strong_by.entry(&d.name).or_default().insert(i);
        }
    }
    let between = strong_by
        .into_iter()
        .filter(|(sym, defs)| {
            defs.len() >= 2
                && *sym != "main"
                && *sym != FUZZ_ENTRY
                && !set_symbols.contains(*sym)
                && !collision_symbols.contains(*sym)
        })
        .map(|(sym, defs)| BetweenDuplicate {
            sym: sym.to_string(),
            definers: defs.into_iter().map(path).collect(),
        })
        .collect();

    let included_by = includers
        .into_iter()
        .map(|(file, by)| IncludedBy {
            file: file.to_string(),
            by: by.into_iter().collect(),
        })
        .collect();

    Ok(Analysis {
        programs,
        programs_not_compiled: not_compiled.into_iter().map(path).collect(),
        closures,
        between_program_duplicates: between,
        shared,
        libraries,
        included_by,
    })
}

/// The facts every closure's incompleteness reads (§3.1 step 8): the
/// folders the walk could not read, the `.c` files parsed but not compiled
/// (with what the parser says they define), and the `.c` files neither
/// parsed nor compiled.
struct Gaps<'a> {
    unreadable: Vec<&'a WalkIssue>,
    failed_parsed: Vec<(&'a FileFacts, &'a ParserFacts)>,
    unread: Vec<&'a FileFacts>,
}

impl<'a> Gaps<'a> {
    fn new(input: &Input<'a>, files: &[&'a FileFacts]) -> Gaps<'a> {
        Gaps {
            unreadable: input
                .walk_issues
                .iter()
                .filter(|w| w.why.starts_with("cannot be read"))
                .collect(),
            failed_parsed: files
                .iter()
                .filter(|f| {
                    f.kind == FileKind::C
                        && f.parsed
                        && matches!(f.compiled, Some(Compiled::Failed { .. }))
                })
                .filter_map(|f| input.parser.get(&f.path).map(|p| (*f, p)))
                .collect(),
            unread: files
                .iter()
                .copied()
                .filter(|f| f.kind == FileKind::C && !f.parsed && !compiled_ok(f))
                .collect(),
        }
    }

    /// The reasons a closure with these `outside` symbols is incomplete,
    /// besides its pending sets.
    fn why(&self, outside: &BTreeSet<String>) -> Vec<Incomplete> {
        let mut why = Vec::new();
        for (f, p) in &self.failed_parsed {
            let may: Vec<String> = p
                .defines
                .iter()
                .filter(|s| outside.contains(*s))
                .cloned()
                .collect();
            if !may.is_empty() {
                why.push(Incomplete {
                    why: IncompleteWhy::MayBeDefinedIn,
                    path: Some(f.path.clone()),
                    symbols: may,
                });
            }
        }
        if !outside.is_empty() {
            for f in &self.unread {
                why.push(Incomplete {
                    why: IncompleteWhy::Unread,
                    path: Some(f.path.clone()),
                    symbols: outside.iter().cloned().collect(),
                });
            }
        }
        for w in &self.unreadable {
            why.push(Incomplete {
                why: IncompleteWhy::UnreadableFolder,
                path: Some(w.path.clone()),
                symbols: Vec::new(),
            });
        }
        why
    }
}

/// A duplicate set a chosen closure still leaves open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSet {
    /// Its definers' paths, in path order.
    pub definers: Vec<String>,
    /// The closure's symbols it defines, sorted.
    pub symbols: Vec<String>,
}

/// One `main` program's closure under the person's picks
/// (docs/PROJECT-MAP-DESIGN.md §3.6): what `project accept` writes and
/// links, recomputed from the file facts with each pick applied (so a file
/// only one definer needs stays only with it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    /// Its files (the program's own included), sorted.
    pub files: Vec<String>,
    /// The duplicate sets the picks leave open, by their definers' paths.
    pub open: Vec<OpenSet>,
    /// Needed symbols no project file defines, sorted.
    pub outside: Vec<String>,
    /// Why it is incomplete besides an open set (a file that did not
    /// compile and may define an outside symbol, an unread file, an
    /// unreadable folder), sorted.
    pub incomplete_why: Vec<Incomplete>,
    /// Needs met only by another program's file: `(symbol, its file)`.
    pub needs_from: Vec<(String, String)>,
}

/// The closure of the `main` program at `program` (its file's path) with
/// `picks` applied — each `(definers' paths, kept path)` — or `None` when
/// no compiled file there is a `main` program (a fuzzer, a driver, not a
/// program). A pick naming a path the facts do not hold is ignored.
pub fn chosen_closure(
    input: &Input<'_>,
    program: &str,
    picks: &[(Vec<String>, String)],
) -> Option<Chosen> {
    let mut files: Vec<&FileFacts> = input.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let kinds = find_programs(&files);
    let is_program: Vec<bool> = (0..files.len()).map(|i| kinds.contains_key(&i)).collect();
    let project = Project::new(files, is_program);
    let start = *project.by_path.get(program)?;
    if kinds.get(&start).map(|k| k.0) != Some(ProgramKind::Main) {
        return None;
    }
    let mut choices = Choices::new();
    for (definers, keep) in picks {
        let key: Option<SetKey> = definers
            .iter()
            .map(|p| project.by_path.get(p.as_str()).copied())
            .collect();
        let (Some(mut key), Some(&keep)) = (key, project.by_path.get(keep.as_str())) else {
            continue;
        };
        key.sort_unstable();
        key.dedup();
        choices.insert(key, keep);
    }
    let core = project.closure(&BTreeSet::from([start]), &choices);
    let gaps = Gaps::new(input, &project.files);
    let mut incomplete_why = gaps.why(&core.outside);
    incomplete_why.sort();
    let path = |i: usize| project.files[i].path.clone();
    Some(Chosen {
        files: core.files.iter().map(|&f| path(f)).collect(),
        open: core
            .pending
            .iter()
            .map(|(key, symbols)| OpenSet {
                definers: key.iter().map(|&f| path(f)).collect(),
                symbols: symbols.iter().cloned().collect(),
            })
            .collect(),
        outside: core.outside.iter().cloned().collect(),
        incomplete_why,
        needs_from: core
            .needs_from
            .iter()
            .map(|(sym, p)| (sym.clone(), path(*p)))
            .collect(),
    })
}

/// Files reached from the programs that did not compile, through their
/// parser's calls and then the objects' needs: never offered as a library.
fn reached_through(
    project: &Project<'_>,
    failed: &[usize],
    parser: &BTreeMap<String, ParserFacts>,
) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    let mut queue: Vec<usize> = Vec::new();
    let take = |res: Res, queue: &mut Vec<usize>, reached: &mut BTreeSet<usize>| match res {
        Res::One(d) => {
            if reached.insert(d) {
                queue.push(d);
            }
        }
        Res::Dup(ds) => {
            for d in ds {
                if reached.insert(d) {
                    queue.push(d);
                }
            }
        }
        Res::Programs(_) | Res::Outside => {}
    };
    for &p in failed {
        if let Some(facts) = parser.get(&project.files[p].path) {
            for call in &facts.calls {
                take(project.resolve(call), &mut queue, &mut reached);
            }
        }
    }
    while let Some(f) = queue.pop() {
        for n in &project.files[f].needed {
            take(project.resolve(&n.name), &mut queue, &mut reached);
        }
    }
    reached
}

/// An accepted library keeps its id for the group holding any of its listed
/// files (the most of them when several do; a tie goes to the group whose
/// first file sorts first), so a new file that sorts first does not move
/// it: `(the group's first path, id)` pairs for [`ids::assign`]. Each id
/// goes to one group, each group keeps at most one id (the id holding the
/// most of its files, then the smaller id).
fn library_keeps(
    project: &Project<'_>,
    groups: &BTreeMap<usize, BTreeSet<usize>>,
    accepted: &[(String, String)],
) -> Vec<(String, String)> {
    let mut held: BTreeMap<(&str, usize), usize> = BTreeMap::new();
    for (path, id) in accepted.iter().filter(|(_, id)| id.starts_with("l-")) {
        let Some(&f) = project.by_path.get(path.as_str()) else {
            continue;
        };
        if let Some((&root, _)) = groups.iter().find(|(_, g)| g.contains(&f)) {
            *held.entry((id.as_str(), root)).or_default() += 1;
        }
    }
    // Most files first, then the id, then the group's first path.
    let mut ranked: Vec<(usize, &str, usize)> = held
        .into_iter()
        .map(|((id, root), n)| (n, id, root))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)).then(a.2.cmp(&b.2)));
    let mut ids_used: BTreeSet<&str> = BTreeSet::new();
    let mut groups_used: BTreeSet<usize> = BTreeSet::new();
    let mut out = Vec::new();
    for (_, id, root) in ranked {
        if ids_used.contains(id) || groups_used.contains(&root) {
            continue;
        }
        ids_used.insert(id);
        groups_used.insert(root);
        if let Some(&first) = groups[&root].first() {
            out.push((project.files[first].path.clone(), id.to_string()));
        }
    }
    out
}

/// The reasons a library over `files` (paths) is incomplete
/// (docs/PROJECT-MAP-DESIGN.md §3.6): a symbol its files need that no
/// compiled project file defines may be defined in a file that did not
/// compile, could not be read, or lies in a folder the walk could not read
/// — as a program's closure is incomplete for the same reasons. Sorted.
pub fn library_gaps(input: &Input<'_>, files: &[String]) -> Vec<Incomplete> {
    let mut all: Vec<&FileFacts> = input.files.iter().collect();
    all.sort_by(|a, b| a.path.cmp(&b.path));
    all.dedup_by(|a, b| a.path == b.path);
    let kinds = find_programs(&all);
    let is_program: Vec<bool> = (0..all.len()).map(|i| kinds.contains_key(&i)).collect();
    let project = Project::new(all, is_program);
    let group: BTreeSet<usize> = files
        .iter()
        .filter_map(|p| project.by_path.get(p.as_str()).copied())
        .collect();
    let outside: BTreeSet<String> = unresolved(&project.refs(&group))
        .into_iter()
        .filter(|sym| matches!(project.resolve(sym), Res::Outside))
        .collect();
    let mut why = Gaps::new(input, &project.files).why(&outside);
    why.sort();
    why
}

/// Group `unreached` files: two join when one needs a symbol the other
/// alone defines (§3.1 step 9).
fn libraries(
    project: &Project<'_>,
    unreached: &[usize],
    accepted: &[(String, String)],
) -> Vec<Library> {
    let set: BTreeSet<usize> = unreached.iter().copied().collect();
    let mut parent: BTreeMap<usize, usize> = set.iter().map(|&f| (f, f)).collect();
    fn find(parent: &mut BTreeMap<usize, usize>, f: usize) -> usize {
        let mut root = f;
        while parent[&root] != root {
            root = parent[&root];
        }
        parent.insert(f, root);
        root
    }
    for &f in unreached {
        for n in &project.files[f].needed {
            if let Res::One(d) = project.resolve(&n.name) {
                if set.contains(&d) {
                    let (a, b) = (find(&mut parent, f), find(&mut parent, d));
                    // The smaller number (first path) is the group's root.
                    parent.insert(a.max(b), a.min(b));
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &f in unreached {
        let root = find(&mut parent, f);
        groups.entry(root).or_default().insert(f);
    }
    let firsts: Vec<String> = groups
        .values()
        .filter_map(|g| g.first().map(|&f| project.files[f].path.clone()))
        .collect();
    let ids = ids::assign("l-", &firsts, &library_keeps(project, &groups, accepted));
    let mut out: Vec<Library> = groups
        .values()
        .map(|group| {
            let refs = project.refs(group);
            let mut needs = BTreeSet::new();
            for sym in unresolved(&refs) {
                let found = match project.resolve(&sym) {
                    Res::One(d) => vec![d],
                    Res::Dup(ds) => ds,
                    Res::Programs(_) | Res::Outside => Vec::new(),
                };
                needs.extend(found.into_iter().filter(|d| !group.contains(d)));
            }
            let first = &project.files[*group.first().unwrap_or(&0)].path;
            Library {
                id: ids.get(first).cloned().unwrap_or_default(),
                files: group
                    .iter()
                    .map(|&f| project.files[f].path.clone())
                    .collect(),
                needs_from_outside: needs
                    .into_iter()
                    .map(|f| project.files[f].path.clone())
                    .collect(),
            }
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests;
