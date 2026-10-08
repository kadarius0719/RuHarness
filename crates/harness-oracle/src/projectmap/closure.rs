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
//! or common ones stand beside it; with no strong one the first in path
//! order defines it. Over a finished closure, a symbol two or more of its
//! files define strongly is a collision.

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
    /// The choice (a definer index) this set is reached only under.
    pub under: Option<String>,
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

/// A set seen while exploring a program's choices.
#[derive(Debug, Clone)]
struct Seen {
    symbols: BTreeSet<String>,
    under: Option<(SetKey, usize)>,
}

/// What exploring a program's choices found.
struct Explored {
    leaves: Vec<(Choices, Core)>,
    sets: BTreeMap<SetKey, Seen>,
    over: bool,
}

/// A duplicate set in one program's result, before numbering.
#[derive(Debug, Clone)]
struct Dup {
    key: SetKey,
    symbols: BTreeSet<String>,
    links: BTreeSet<usize>,
    keep: Option<usize>,
    under: Option<(SetKey, usize)>,
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

    /// The closure from `start` with `choices` (§3.1 step 7), from scratch:
    /// each round adds every single definer of a need the set does not yet
    /// meet, until a round adds nothing.
    fn closure(&self, start: usize, choices: &Choices) -> Core {
        let mut files: BTreeSet<usize> = BTreeSet::from([start]);
        loop {
            let defined = self.defined_in(&files);
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
                        for p in ps.into_iter().filter(|&p| p != start) {
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

    fn explore_from(
        &self,
        start: usize,
        choices: Choices,
        under: Option<(SetKey, usize)>,
        out: &mut Explored,
    ) {
        if out.over {
            return;
        }
        let core = self.closure(start, &choices);
        for (key, symbols) in &core.pending {
            out.sets
                .entry(key.clone())
                .or_insert_with(|| Seen {
                    symbols: BTreeSet::new(),
                    under: under.clone(),
                })
                .symbols
                .extend(symbols.iter().cloned());
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

    /// A `main` program: its closure, its duplicates settled by linking
    /// when a linker is given (§3.5).
    fn settle<'l>(
        &self,
        start: usize,
        linker: Option<&mut (dyn Linker + 'l)>,
    ) -> Result<Outcome, Error> {
        let base = self.closure(start, &Choices::new());
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
                    under: s.under.clone(),
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
                        under: explored.sets[key].under.clone(),
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
        } else if has(f, FUZZ_ENTRY) && !has(f, "main") {
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
    let mut outcomes: BTreeMap<usize, Outcome> = BTreeMap::new();
    for (&i, (kind, _)) in &kinds {
        match kind {
            ProgramKind::Main => {
                let out = project.settle(i, linker.as_deref_mut())?;
                outcomes.insert(i, out);
            }
            ProgramKind::Fuzz => {
                let core = project.closure(i, &Choices::new());
                let driver = kinds
                    .iter()
                    .find(|(_, (k, serves))| *k == ProgramKind::Driver && serves.contains(&i))
                    .map(|(&d, _)| d);
                let linked = match (driver, linker.as_deref_mut()) {
                    (Some(d), Some(l)) if core.pending.is_empty() => {
                        let mut with = core.files.clone();
                        with.insert(d);
                        Some(project.link(l, &with)?)
                    }
                    _ => None,
                };
                let dups = core
                    .pending
                    .iter()
                    .map(|(key, symbols)| Dup {
                        key: key.clone(),
                        symbols: symbols.clone(),
                        links: BTreeSet::new(),
                        keep: None,
                        under: None,
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
    }

    // Set indexes, once per project (§3.3): sets reached directly first,
    // then those reached only under a choice, each group by its definers'
    // sorted paths (file numbers follow path order).
    let mut direct: BTreeSet<SetKey> = BTreeSet::new();
    let mut all: BTreeSet<SetKey> = BTreeSet::new();
    for out in outcomes.values() {
        for d in &out.dups {
            all.insert(d.key.clone());
            if d.under.is_none() {
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
    let unreadable: Vec<&WalkIssue> = input
        .walk_issues
        .iter()
        .filter(|w| w.why.starts_with("cannot be read"))
        .collect();
    let failed_parsed: Vec<(&FileFacts, &ParserFacts)> = files
        .iter()
        .filter(|f| {
            f.kind == FileKind::C && f.parsed && matches!(f.compiled, Some(Compiled::Failed { .. }))
        })
        .filter_map(|f| input.parser.get(&f.path).map(|p| (*f, p)))
        .collect();
    let unread: Vec<&FileFacts> = files
        .iter()
        .copied()
        .filter(|f| f.kind == FileKind::C && !f.parsed && !compiled_ok(f))
        .collect();

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
                under: d.under.as_ref().map(|(k, f)| definer_index(k, *f)),
            });
        }
        for (f, p) in &failed_parsed {
            let may: Vec<String> = p
                .defines
                .iter()
                .filter(|s| core.outside.contains(*s))
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
        if !core.outside.is_empty() {
            for f in &unread {
                why.push(Incomplete {
                    why: IncompleteWhy::Unread,
                    path: Some(f.path.clone()),
                    symbols: core.outside.iter().cloned().collect(),
                });
            }
        }
        for w in &unreadable {
            why.push(Incomplete {
                why: IncompleteWhy::UnreadableFolder,
                path: Some(w.path.clone()),
                symbols: Vec::new(),
            });
        }
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

    // Duplicates between programs that never meet.
    let mut strong_by: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        if f.kind != FileKind::C || !compiled_ok(f) {
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
    let ids = ids::assign("l-", &firsts, accepted);
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
