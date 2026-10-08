//! RuHarness language frontends. At M1 there is one: [`CFrontend`], a
//! tree-sitter based C scanner implementing
//! [`harness_core::traits::LanguageFrontend`].
//!
//! Ported and extended from the M0 embryo scanner (`m0/src/scan.rs`). No LLM
//! involvement; identical trees yield byte-identical canonical facts.
//!
//! Known, deliberate gap (recorded in DECISIONS.md): calls made through
//! function pointers (e.g. qsort comparators) are invisible here — that is
//! detector material for M2, not a scan feature.
//!
//! M4 additions:
//! - quoted includes resolve against the including file's directory, then
//!   each `[target] include_dirs` entry in order (first hit in the scanned
//!   file set wins); a resolution outside `source_dir` is never recorded,
//!   and the file walk never follows a symlink out of `source_dir` — so the
//!   include closure (and every prompt built from it) is confined to
//!   `source_dir` (docs/M4-DESIGN.md R2);
//! - [`mutants`]: the mutation sites of one `.c` file (R5);
//! - [`lint_driver`]: the driver source lint of the `driver-shape` gate (R1).
//!
//! Post-M4 (design B, docs/ORACLE-HARDENING.md §B.6): [`parse_interface`]
//! reads a plan `interface` line into the boundary check's call-wrapper shape.
//!
//! The project map, step (c) (docs/PROJECT-MAP-DESIGN.md §3.7): a file-list
//! target is read as its listed files plus every header their includes
//! reach, by harness-core's one include rule ([`sources::Resolver`]: the
//! configuration's path flags and each file's own folders in the
//! compiler's order, its `-include` files first), every include name read
//! by the one include reader ([`sources::include_names`]), and
//! every scan prunes `migration/` and keeps walk errors, files over 8 MiB
//! and non-UTF-8 files as notes ([`ScanNotes`]) instead of stopping.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod interface;
mod lint;
mod mutate;
mod probe;

pub use interface::{
    is_c_identifier, parse_interface, InterfaceParam, InterfaceSig, MAX_INTERFACE_LINE,
};
pub use lint::{lint_driver, DRIVER_SYSTEM_INCLUDES};
pub use mutate::mutants;
pub use probe::{probe_source, PlacedNote, ProbeOptions, Probed};

use harness_core::config::{FolderForm, Form, TargetContext};
use harness_core::error::Error;
use harness_core::facts::{Facts, FileRecord, RefRecord, SymbolRecord};
use harness_core::hash::file_hash;
use harness_core::sources;
use harness_core::traits::LanguageFrontend;
use harness_core::walk;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// C language frontend backed by tree-sitter.
///
/// Symbol-identity mapping (docs/SCHEMAS.md): functions with external
/// linkage use their plain linkage name and visibility `public`; `static`
/// functions use `<repo-relative-file>::<name>` and visibility `internal`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CFrontend;

/// A function definition found in a source file (pre-resolution).
#[derive(Debug)]
struct FnDef {
    /// Raw source name of the function.
    name: String,
    /// Repo-relative path of the defining file.
    file: String,
    /// Whether the definition carries the `static` storage class.
    is_static: bool,
    /// Declaration text up to the body, whitespace-normalized.
    signature: String,
    /// 1-based (start_line, end_line) span of the whole definition.
    span: (u32, u32),
    /// Names appearing as direct callees inside this definition.
    calls: BTreeSet<String>,
    /// Where the features probe can put a note (docs/FEATURES-PROBE-REDESIGN.md
    /// §3.1): the byte of the body's opening `{` and the body's end, or why
    /// no note can go there. Whether a note there compiles is the
    /// compiler's to say (§3.4), not this rule's.
    note_at: Result<(usize, usize), NoNote>,
    /// The head names `naked` (its parameters and K&R declarations aside):
    /// on gcc such a function is not watched (§3.1 rule 4).
    naked_head: bool,
    /// The parser put it inside another definition's body — C has no
    /// nested functions, so the outer one was misread (an `#if` whose
    /// branches each open a brace): recorded, never watched (rule 1).
    nested: bool,
}

/// Why the features probe cannot put a note in a definition
/// (docs/FEATURES-PROBE-REDESIGN.md §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoNote {
    /// Rule 1: the definition lies under a parse error — its body's bounds
    /// are a guess.
    Parser,
    /// Rule 2: the body is not a real `{ … }` block.
    NotABlock,
    /// Rule 3: the body's `{` sits in a conditional group its head is not
    /// wholly in.
    ConditionalBrace,
    /// Rule 4 (gcc only): a naked function.
    Naked,
}

impl FnDef {
    /// Canonical symbol id per the symbol-identity rule.
    fn canonical_id(&self) -> String {
        if self.is_static {
            format!("{}::{}", self.file, self.name)
        } else {
            self.name.clone()
        }
    }
}

impl LanguageFrontend for CFrontend {
    fn name(&self) -> &'static str {
        "c-tree-sitter"
    }

    fn scan(&self, target: &TargetContext) -> Result<Facts, Error> {
        self.scan_reporting(target).map(|(facts, _)| facts)
    }
}

/// The C frontend's source extensions.
pub const C_EXTENSIONS: [&str; 2] = ["c", "h"];

/// What a scan says beside its facts (docs/PROJECT-MAP-DESIGN.md §3.7 "The
/// scanner"): a walk error or an odd file is a note of that path, never a
/// stop. The facts themselves keep their schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanNotes {
    /// Entries left out, each with why in words: not a regular file (a FIFO
    /// named `a.c` would block the scan forever), a folder or a file that
    /// cannot be read, a link that leaves the scanned folder or points
    /// nowhere.
    pub skipped: Vec<(PathBuf, String)>,
    /// Files over [`sources::MAX_SOURCE_BYTES`], root-relative: recorded
    /// with their hash, never parsed.
    pub too_large: Vec<String>,
    /// Files that are not UTF-8, root-relative: parsed from their bytes like
    /// any other (Latin-1 comments are common in older C).
    pub not_utf8: Vec<String>,
    /// Files that cannot be read, root-relative: recorded with
    /// [`sources::UNREADABLE_HASH`] and no includes (also in `skipped`,
    /// with the error in words), so staleness counts them as recorded.
    pub unreadable: Vec<String>,
    /// A file-list target's ambiguous includes: a header reached from two
    /// listed files whose folders resolve one of its include names to two
    /// places. No edge is recorded for such a name.
    pub ambiguous: Vec<sources::Ambiguous>,
}

impl ScanNotes {
    /// Each ambiguous include in one sentence, for the person.
    pub fn ambiguous_lines(&self) -> Vec<String> {
        self.ambiguous
            .iter()
            .map(|a| {
                let lands: Vec<String> = a
                    .lands
                    .iter()
                    .map(|(unit, landing)| {
                        let at = match landing {
                            sources::Landing::Project(rel) => rel.clone(),
                            sources::Landing::Outside(_) => "a file outside the project".into(),
                            sources::Landing::NotProject => "no project file".into(),
                        };
                        format!("{at} when compiled for {unit}")
                    })
                    .collect();
                let (open, close) = if a.quoted { ('"', '"') } else { ('<', '>') };
                format!(
                    "{}'s include {open}{}{close} reaches {}: an ambiguous include, recorded for \
                     none; give those files the same include folder for it",
                    a.file,
                    a.name,
                    lands.join(" but ")
                )
            })
            .collect()
    }
}

/// One file the scan records.
struct Scanned {
    /// Root-relative path.
    rel: String,
    /// Its content hash ([`sources::UNREADABLE_HASH`] for a file that
    /// cannot be read).
    hash: String,
    /// The project files its includes reach, root-relative.
    includes: BTreeSet<String>,
    /// Its parse and bytes; `None` for a file too large to parse, or one
    /// that cannot be read.
    parse: Option<(tree_sitter::Tree, Vec<u8>)>,
}

/// A file's include names, `(name, quoted)`, as [`sources::include_names`]
/// reads them.
type Names = Vec<(String, bool)>;

/// One file's hash and parse, as both readers take it.
struct Source {
    hash: String,
    parse: Option<(tree_sitter::Tree, Vec<u8>)>,
}

/// Read and parse `abs` (root-relative `rel`), its size checked before it
/// is read, with the include names [`sources::include_names`] reads in it
/// (none for a file too large to parse). A file that cannot be read is
/// recorded all the same, with [`sources::UNREADABLE_HASH`] and a note
/// (the readers review, finding 4: a fact, so staleness counts it as
/// recorded); `None` only for an entry that is not a regular file, noted.
fn read_source(
    abs: &Path,
    rel: &str,
    parser: &mut tree_sitter::Parser,
    notes: &mut ScanNotes,
) -> Option<(Source, Names)> {
    use std::io::Read as _;
    let unreadable = |notes: &mut ScanNotes, e: &dyn std::fmt::Display| {
        notes.skipped.push((
            abs.to_path_buf(),
            format!("cannot be read: {e}; recorded as unreadable"),
        ));
        notes.unreadable.push(rel.to_string());
        Some((
            Source {
                hash: sources::UNREADABLE_HASH.to_string(),
                parse: None,
            },
            Vec::new(),
        ))
    };
    let meta = match std::fs::metadata(abs) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => {
            notes
                .skipped
                .push((abs.to_path_buf(), "not a regular file".into()));
            return None;
        }
        Err(e) => return unreadable(notes, &e),
    };
    let mut bytes = Vec::new();
    if meta.len() <= sources::MAX_SOURCE_BYTES {
        let read = std::fs::File::open(abs).and_then(|f| {
            f.take(sources::MAX_SOURCE_BYTES + 1)
                .read_to_end(&mut bytes)
        });
        if let Err(e) = read {
            return unreadable(notes, &e);
        }
    }
    if meta.len() > sources::MAX_SOURCE_BYTES || bytes.len() as u64 > sources::MAX_SOURCE_BYTES {
        // Hashed by streaming, never parsed.
        return match file_hash(abs) {
            Ok(hash) => {
                notes.too_large.push(rel.to_string());
                Some((Source { hash, parse: None }, Vec::new()))
            }
            Err(e) => unreadable(notes, &e),
        };
    }
    let names = sources::include_names(&bytes);
    let hash = harness_core::hash::bytes_hash(&bytes);
    let Some(tree) = parser.parse(&bytes, None) else {
        notes.skipped.push((
            abs.to_path_buf(),
            "the parser could not read it; recorded, not parsed".into(),
        ));
        return Some((Source { hash, parse: None }, names));
    };
    if std::str::from_utf8(&bytes).is_err() {
        notes.not_utf8.push(rel.to_string());
    }
    Some((
        Source {
            hash,
            parse: Some((tree, bytes)),
        },
        names,
    ))
}

/// A folder target (`source_dir`): every `.c` and `.h` the confined walk
/// finds under it, `migration/` and the ledger pruned; quoted includes
/// resolved as since M4 (the file's own folder, then `include_dirs`, the
/// first hit in the scanned set inside `source_dir`), their names read by
/// [`sources::include_names`], the one reader.
fn read_folder(
    target: &TargetContext,
    folder: &FolderForm,
    parser: &mut tree_sitter::Parser,
    notes: &mut ScanNotes,
) -> Result<Vec<Scanned>, Error> {
    let src_dir = target.root.join(&folder.source_dir);
    let walked = walk::confined_except(
        &src_dir,
        &C_EXTENSIONS,
        walk::Limits::default(),
        &sources::pruned(target),
    );
    for issue in &walked.issues {
        if let (true, walk::Why::Unreadable(why)) = (issue.path == src_dir, &issue.why) {
            // The folder itself: there is nothing to scan.
            return Err(Error::io(&src_dir, std::io::Error::other(why.clone())));
        }
        notes.skipped.push((issue.path.clone(), issue.why.words()));
    }
    let source_rel = lexical_segments(&folder.source_dir).unwrap_or_default();
    let include_dirs: Vec<Vec<String>> = folder
        .include_dirs
        .iter()
        .filter_map(|d| lexical_segments(d))
        .collect();

    // (repo-relative path, absolute path), sorted by relative path.
    let mut files: Vec<(String, PathBuf)> = Vec::with_capacity(walked.files.len());
    for abs in walked.files {
        files.push((repo_relative(&target.root, &abs)?, abs));
    }
    files.sort();
    let mut read: Vec<(String, Source, Names)> = Vec::with_capacity(files.len());
    for (rel, abs) in &files {
        if let Some((source, names)) = read_source(abs, rel, parser, notes) {
            read.push((rel.clone(), source, names));
        }
    }
    let scanned: BTreeSet<String> = read.iter().map(|(rel, _, _)| rel.clone()).collect();
    Ok(read
        .into_iter()
        .map(|(rel, source, names)| {
            // `<...>` system includes are deliberately skipped by the scan.
            let includes = names
                .iter()
                .filter(|(_, quoted)| *quoted)
                .filter_map(|(raw, _)| {
                    resolve_quoted_include(&rel, raw, &include_dirs, &source_rel, &scanned)
                })
                .collect();
            Scanned {
                rel,
                hash: source.hash,
                includes,
                parse: source.parse,
            }
        })
        .collect())
}

/// A file-list target: what the include rule ([`sources::Resolver`])
/// reaches — the listed files, the configuration's `-include` files and
/// every header their includes reach through the configuration's folders
/// and each listed file's own, both include forms, to closure — inside the
/// project root and never under `migration/` (docs/PROJECT-MAP-DESIGN.md
/// §3.1 step 3, §3.7). An include that lands outside the project, or under
/// `migration/`, is a note; an include a header resolves differently under
/// two listed files is an ambiguous include, recorded for neither.
fn read_file_list(
    resolver: &sources::Resolver,
    parser: &mut tree_sitter::Parser,
    notes: &mut ScanNotes,
) -> Vec<Scanned> {
    let mut read: BTreeMap<String, Source> = BTreeMap::new();
    let program = resolver.walk(|rel| {
        let abs = resolver.root().join(rel);
        let (source, names) = read_source(&abs, rel, parser, notes)?;
        read.insert(rel.to_string(), source);
        Some(names)
    });
    for outside in &program.outside {
        notes.skipped.push((
            outside.path.clone(),
            format!(
                "{} includes it as `{}`, but it lies outside the project or under migration/: \
                 never read as project C",
                outside.file, outside.name
            ),
        ));
    }
    notes.ambiguous = program.ambiguous;
    program
        .files
        .into_iter()
        .filter_map(|(rel, includes)| {
            let source = read.remove(&rel)?;
            Some(Scanned {
                rel,
                hash: source.hash,
                includes,
                parse: source.parse,
            })
        })
        .collect()
}

impl CFrontend {
    /// [`LanguageFrontend::scan`], also returning what the scan noted beside
    /// the facts ([`ScanNotes`]): the caller reports them. A folder target
    /// is walked with harness-core's confined walk (a file reached through
    /// a link scanned once, under its path with no link in it; dot-folders,
    /// `migration/` and the ledger not scanned); a file-list target is read
    /// as its listed files plus the headers their includes reach
    /// (docs/PROJECT-MAP-DESIGN.md §3.7), and a listed file or include
    /// folder outside the project root or under `migration/` is refused in
    /// one sentence. Only a `source_dir` that cannot be read stops the scan.
    pub fn scan_reporting(&self, target: &TargetContext) -> Result<(Facts, ScanNotes), Error> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;
        let mut notes = ScanNotes::default();
        let mut files = match (&target.config.target.form, sources::Resolver::of(target)?) {
            (_, Some(resolver)) => read_file_list(&resolver, &mut parser, &mut notes),
            (Form::Folder(folder), None) => read_folder(target, folder, &mut parser, &mut notes)?,
            (Form::FileList(_), None) => Vec::new(),
        };
        files.sort_by(|a, b| a.rel.cmp(&b.rel));

        let mut defs: Vec<FnDef> = Vec::new();
        let mut file_records: Vec<FileRecord> = Vec::with_capacity(files.len());
        for file in files {
            if let Some((tree, src)) = &file.parse {
                collect_functions(tree.root_node(), src, &file.rel, &mut defs);
            }
            file_records.push(FileRecord {
                path: file.rel,
                hash: file.hash,
                includes: file.includes.into_iter().collect(),
            });
        }

        // Resolution maps: statics keyed by (file, name); publics by name.
        let mut statics: BTreeSet<(String, String)> = BTreeSet::new();
        let mut publics: BTreeSet<String> = BTreeSet::new();
        for d in &defs {
            if d.is_static {
                statics.insert((d.file.clone(), d.name.clone()));
            } else {
                publics.insert(d.name.clone());
            }
        }

        let symbols: Vec<SymbolRecord> = defs
            .iter()
            .map(|d| SymbolRecord {
                name: d.canonical_id(),
                kind: "function".to_string(),
                file: d.file.clone(),
                visibility: if d.is_static { "internal" } else { "public" }.to_string(),
                signature: d.signature.clone(),
                span: d.span,
            })
            .collect();

        // Two-pass call resolution; dedup identical (from, file, to, refkind).
        let mut edges: BTreeSet<(String, String, String, bool)> = BTreeSet::new();
        for d in &defs {
            let from = d.canonical_id();
            for callee in &d.calls {
                let (to, resolved) = if statics.contains(&(d.file.clone(), callee.clone())) {
                    (format!("{}::{}", d.file, callee), true)
                } else if publics.contains(callee) {
                    (callee.clone(), true)
                } else {
                    (callee.clone(), false)
                };
                edges.insert((from.clone(), d.file.clone(), to, resolved));
            }
        }
        let refs: Vec<RefRecord> = edges
            .into_iter()
            .map(|(from, file, to, resolved)| RefRecord {
                from,
                file,
                to,
                refkind: "call".to_string(),
                resolved,
            })
            .collect();

        Ok((
            Facts {
                frontend: self.name().to_string(),
                files: file_records,
                symbols,
                refs,
            },
            notes,
        ))
    }
}

/// Render `path` relative to `root` with forward slashes.
fn repo_relative(root: &Path, path: &Path) -> Result<String, Error> {
    let rel = path.strip_prefix(root).map_err(|_| {
        Error::Invariant(format!(
            "{} escapes target root {}",
            path.display(),
            root.display()
        ))
    })?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

/// The clean segments of a repo-relative path (`.`/empty segments dropped,
/// `..` applied). `None` when the path is absolute or escapes the root.
fn lexical_segments(path: &str) -> Option<Vec<String>> {
    if path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s.to_string()),
        }
    }
    Some(parts)
}

/// Lexically resolve the quoted include `raw` against the segments of
/// `base_dir`. `None` when the result escapes the target root.
fn resolve_against(base_dir: &[String], raw: &str) -> Option<Vec<String>> {
    if raw.starts_with('/') {
        return None;
    }
    let mut parts: Vec<String> = base_dir.to_vec();
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s.to_string()),
        }
    }
    Some(parts)
}

/// Resolve the quoted include `raw` of the repo-relative `including` file the
/// way a compiler with `-I<include_dirs…>` would: the including file's own
/// directory first, then each include dir in order. The first candidate
/// that lies inside `source_dir` AND is in the scanned file set wins; a
/// candidate outside `source_dir` is never recorded (R2: prompt-bound reads
/// are confined to `source_dir`).
fn resolve_quoted_include(
    including: &str,
    raw: &str,
    include_dirs: &[Vec<String>],
    source_dir: &[String],
    scanned: &BTreeSet<String>,
) -> Option<String> {
    let mut own_dir: Vec<String> = including.split('/').map(str::to_string).collect();
    own_dir.pop(); // drop the file name, keeping its directory
    std::iter::once(&own_dir)
        .chain(include_dirs.iter())
        .filter_map(|base| resolve_against(base, raw))
        .filter(|parts| parts.len() > source_dir.len() && parts.starts_with(source_dir))
        .map(|parts| parts.join("/"))
        .find(|candidate| scanned.contains(candidate))
}

/// One `#include` directive's name, as written between its quotes or its
/// angle brackets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Include {
    /// The name (`util.h`, `proj/api.h`), raw.
    pub name: String,
    /// `#include "name"` (true) or `#include <name>` (false).
    pub quoted: bool,
}

/// What the scanner reads from one file, for the project map
/// (docs/PROJECT-MAP-DESIGN.md §3.1 step 2): the functions it defines, the
/// names they call and every include, from every `#if` branch (the parser
/// evaluates none).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// The names of the functions defined, `static` ones included, in
    /// source order.
    pub functions: Vec<String>,
    /// The names called directly from those functions.
    pub calls: BTreeSet<String>,
    /// Every include directive, sorted.
    pub includes: Vec<Include>,
}

/// Parse one C file's bytes — UTF-8 or not: Latin-1 comments parse like any
/// others — into its [`FileFacts`], with the parser and rules `harness scan`
/// uses.
pub fn file_facts(source: &[u8]) -> Result<FileFacts, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::Invariant("tree-sitter parse failed".into()))?;
    let root = tree.root_node();
    let mut defs = Vec::new();
    collect_functions(root, source, "", &mut defs);
    Ok(FileFacts {
        calls: defs.iter().flat_map(|d| d.calls.iter().cloned()).collect(),
        functions: defs.into_iter().map(|d| d.name).collect(),
        // The one include reader, as `harness scan` takes names.
        includes: sources::include_names(source)
            .into_iter()
            .map(|(name, quoted)| Include { name, quoted })
            .collect(),
    })
}

/// Recursively collect function definitions with their storage class,
/// signature, span, and direct-call names.
fn collect_functions(node: tree_sitter::Node, src: &[u8], file: &str, defs: &mut Vec<FnDef>) {
    collect_functions_in(node, src, file, false, false, defs);
}

/// [`collect_functions`], knowing whether an ERROR node encloses `node` and
/// whether a definition does. A definition's own body is searched too: a
/// misread `#if` can make the parser run one body on over every later
/// definition (sqlite3.c's winWrite holds 19 700 lines of them), which
/// would otherwise vanish from the facts.
fn collect_functions_in(
    node: tree_sitter::Node,
    src: &[u8],
    file: &str,
    under_error: bool,
    nested: bool,
    defs: &mut Vec<FnDef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            let at = defs.len();
            // A keyword "name" (`if( rc==0 ){` misread as a definition) is a
            // statement, not a function: never recorded, only searched.
            let name = function_name(child, src).filter(|n| !C_KEYWORDS.contains(&n.as_str()));
            if let Some(name) = name {
                let mut c = child.walk();
                let mut is_static = child
                    .children(&mut c)
                    .any(|n| n.kind() == "storage_class_specifier" && text(n, src) == "static");
                drop(c);
                let mut signature = signature_of(child, src);
                // A first head folded into a parse error before this one:
                // only what follows that head's parameters is this
                // definition's (fix pass 4's check).
                if let Some((folded_static, start)) = folded_head(child, src) {
                    is_static = folded_static;
                    let end = child
                        .child_by_field_name("body")
                        .map_or(child.end_byte(), |b| b.start_byte());
                    signature = String::from_utf8_lossy(&src[start..end])
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                let mut calls = BTreeSet::new();
                collect_calls(child, src, &mut calls);
                defs.push(FnDef {
                    name,
                    file: file.to_string(),
                    is_static,
                    signature,
                    span: (
                        (child.start_position().row + 1) as u32,
                        (child.end_position().row + 1) as u32,
                    ),
                    calls,
                    note_at: note_point(child, src, under_error || nested),
                    naked_head: naked_head(child, src),
                    nested,
                });
            }
            let recorded = defs.len() > at;
            // Two heads run together: the second is a function of its own,
            // recorded with rule 1, never lost from the facts; a macro before
            // the real name gives the definition the real name (see [`heads`]).
            if recorded {
                match heads(child, src) {
                    Heads::Two {
                        name,
                        call,
                        first_end,
                        outer_end,
                    } => {
                        defs[at].calls.clear();
                        let second =
                            second_head(child, name, call, (first_end, outer_end), src, file);
                        defs.push(second);
                    }
                    Heads::MacroFirst { name } => {
                        defs[at].name = name;
                        defs[at].note_at = Err(NoNote::Parser);
                    }
                    Heads::One | Heads::Unreadable => {}
                }
            }
            // Only a body the parser misread — an #if group with two branches
            // that each move the brace depth, read by the parser as both —
            // holds definitions of its own (fix pass 1's check: a statement
            // macro or a GNU nested function is no file-scope definition).
            let misread = child
                .child_by_field_name("body")
                .is_some_and(|b| body_misread(&src[b.start_byte()..b.end_byte()]));
            if misread {
                let before = defs.len();
                collect_functions_in(child, src, file, under_error, true, defs);
                // Its bounds are a guess, as under a parse error (rule 1).
                if recorded && defs.len() > before {
                    defs[at].note_at = Err(NoNote::Parser);
                }
            }
        } else {
            let error = under_error || child.is_error();
            collect_functions_in(child, src, file, error, nested, defs);
        }
    }
}

/// C's keywords: never a function's name.
const C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Bool",
    "_Complex",
    "_Generic",
    "_Imaginary",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "alignas",
    "alignof",
    "bool",
    "constexpr",
    "false",
    "nullptr",
    "static_assert",
    "thread_local",
    "true",
    "typeof",
    "typeof_unqual",
];

/// Where a note can go in `def` (see [`FnDef::note_at`]): rules 1–3 of
/// docs/FEATURES-PROBE-REDESIGN.md §3.1 — nothing else is guessed.
fn note_point(
    def: tree_sitter::Node,
    src: &[u8],
    under_error: bool,
) -> Result<(usize, usize), NoNote> {
    if under_error {
        return Err(NoNote::Parser);
    }
    let body = def.child_by_field_name("body").ok_or(NoNote::NotABlock)?;
    let declarator = def
        .child_by_field_name("declarator")
        .ok_or(NoNote::Parser)?;
    if body.kind() != "compound_statement" || body.is_missing() {
        return Err(NoNote::NotABlock);
    }
    let at = body.start_byte();
    if src.get(at) != Some(&b'{') {
        return Err(NoNote::NotABlock);
    }
    // Rule 3, from the start of the function's name (the parser can fold an
    // `#ifdef` into the declarator; a directive before the name — a
    // `__declspec` under `#ifdef _WIN32` — is the head's own).
    let from = name_start(declarator).unwrap_or(declarator.start_byte());
    if brace_is_conditional(&src[from.min(at)..at]) {
        return Err(NoNote::ConditionalBrace);
    }
    // Rule 1's other form: the body is the next definition's, or the head is
    // a macro's.
    if !matches!(heads(def, src), Heads::One) {
        return Err(NoNote::Parser);
    }
    Ok((at, body.end_byte()))
}

/// The byte where the declared function's name starts: through pointer and
/// parenthesized declarators to the identifier.
fn name_start(declarator: tree_sitter::Node) -> Option<usize> {
    let mut d = declarator;
    loop {
        if d.kind() == "identifier" {
            return Some(d.start_byte());
        }
        d = match d.child_by_field_name("declarator") {
            Some(inner) => inner,
            None => {
                let mut cursor = d.walk();
                let inner = d
                    .named_children(&mut cursor)
                    .find(|c| c.kind().ends_with("declarator") || c.kind() == "identifier");
                inner?
            }
        };
    }
}

/// Rule 3 (docs/FEATURES-PROBE-REDESIGN.md §3.1): whether the `{` that ends
/// `head` — the text from the function's name to its body's `{` — sits in a
/// conditional group the head is not wholly in: an `#if`/`#ifdef`/`#ifndef`
/// opened in `head` still open at its end, or an `#else`/`#elif…`/`#endif`
/// of a group opened before it (two heads for one body). A directive line
/// starts, after blanks and comments, with `#` or the digraph `%:`; line
/// splices are joined first.
fn brace_is_conditional(head: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut in_comment = false;
    for line in logical_lines(head) {
        let mut rest = skip_blanks_and_comments(&line, &mut in_comment);
        let directive = if let Some(r) = rest.strip_prefix(b"#") {
            Some(r)
        } else {
            rest.strip_prefix(b"%:")
        };
        if let Some(after) = directive {
            let mut after_comment = in_comment;
            let words = skip_blanks_and_comments(after, &mut after_comment);
            let keyword: Vec<u8> = words
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .copied()
                .collect();
            match keyword.as_slice() {
                b"if" | b"ifdef" | b"ifndef" => depth += 1,
                b"else" | b"elif" | b"elifdef" | b"elifndef" => {
                    if depth == 0 {
                        return true;
                    }
                }
                b"endif" => {
                    if depth == 0 {
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            rest = words;
        }
        // A comment opened later on the line runs on into the next.
        track_comments(rest, &mut in_comment);
    }
    depth > 0
}

/// `text` split into lines (`\n`, `\r\n` or a lone `\r`), each line ending
/// in a backslash joined to the next — as the preprocessor reads them.
fn logical_lines(text: &[u8]) -> Vec<Vec<u8>> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let b = text[i];
        let end = match b {
            b'\n' => Some(1),
            b'\r' if text.get(i + 1) == Some(&b'\n') => Some(2),
            b'\r' => Some(1),
            _ => None,
        };
        match end {
            Some(len) => {
                if current.last() == Some(&b'\\') {
                    current.pop();
                } else {
                    lines.push(std::mem::take(&mut current));
                }
                i += len;
            }
            None => {
                current.push(b);
                i += 1;
            }
        }
    }
    lines.push(current);
    lines
}

/// `line` past its leading blanks and comments (a `/* */` comment may have
/// begun on an earlier line: `in_comment`).
fn skip_blanks_and_comments<'a>(mut line: &'a [u8], in_comment: &mut bool) -> &'a [u8] {
    loop {
        if *in_comment {
            match line.windows(2).position(|w| w == b"*/") {
                Some(end) => {
                    line = &line[end + 2..];
                    *in_comment = false;
                }
                None => return &[],
            }
        }
        let trimmed = line
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'))
            .map_or(&line[line.len()..], |p| &line[p..]);
        if let Some(r) = trimmed.strip_prefix(b"/*") {
            *in_comment = true;
            line = r;
            continue;
        }
        if trimmed.starts_with(b"//") {
            return &[];
        }
        return trimmed;
    }
}

/// Follow `line`'s code to its end, noting whether a `/* */` comment is
/// left open (string and character literals skipped).
fn track_comments(line: &[u8], in_comment: &mut bool) {
    let mut i = 0;
    while i < line.len() {
        if *in_comment {
            match line[i..].windows(2).position(|w| w == b"*/") {
                Some(end) => {
                    i += end + 2;
                    *in_comment = false;
                }
                None => return,
            }
            continue;
        }
        match line[i] {
            b'/' if line.get(i + 1) == Some(&b'*') => {
                *in_comment = true;
                i += 2;
            }
            b'/' if line.get(i + 1) == Some(&b'/') => return,
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < line.len() && line[i] != quote {
                    i += if line[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Whether `def`'s head names `naked` or `__naked__` as a word, its
/// parameter list and K&R declarations aside (a parameter named `naked` is
/// no attribute) — rule 4, applied on gcc only.
fn naked_head(def: tree_sitter::Node, src: &[u8]) -> bool {
    let (Some(body), Some(declarator)) = (
        def.child_by_field_name("body"),
        def.child_by_field_name("declarator"),
    ) else {
        return false;
    };
    let at = body.start_byte();
    let mut cut: Vec<(usize, usize)> = parameter_list(declarator)
        .map(|p| (p.start_byte(), p.end_byte()))
        .into_iter()
        .collect();
    let mut cursor = def.walk();
    cut.extend(
        def.children(&mut cursor)
            .filter(|c| c.kind() == "declaration")
            .map(|c| (c.start_byte(), c.end_byte())),
    );
    cut.sort_unstable();
    let mut head = String::new();
    let mut from = def.start_byte();
    for (start, end) in cut {
        head.push_str(&String::from_utf8_lossy(
            &src[from..start.max(from).min(at)],
        ));
        head.push(' ');
        from = end.max(from);
    }
    head.push_str(&String::from_utf8_lossy(&src[from.min(at)..at]));
    head.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == "naked" || word == "__naked__")
}

/// The parameter list of the function a declarator declares (through
/// pointer and parenthesized declarators).
fn parameter_list(declarator: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut d = declarator;
    loop {
        if d.kind() == "function_declarator" {
            return d.child_by_field_name("parameters");
        }
        d = inner_declarator(d)?;
    }
}

/// The definition's source text from its start to the start of its body
/// (`compound_statement`), whitespace-normalized to single spaces, trimmed.
fn signature_of(def: tree_sitter::Node, src: &[u8]) -> String {
    let end = def
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or_else(|| def.end_byte());
    let raw = String::from_utf8_lossy(&src[def.start_byte()..end]);
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How a definition's head reads after its parameter list.
enum Heads<'t> {
    /// One head: the function its declarator names.
    One,
    /// Two heads ran together (review: `int g(void) NOT_IMPLEMENTED` — a
    /// macro that supplies the body — then `int after(void) { … }`, read as
    /// one definition of `g` with `after`'s body): the first has no body of
    /// its own, the second is `call`, named `name`. Both rule 1.
    Two {
        name: String,
        call: tree_sitter::Node<'t>,
        /// Where the first head's parameter list ends.
        first_end: usize,
        /// Where the second head's declarator ends: past a nested head's
        /// own parameters (the outermost function declarator's end), else at
        /// its call.
        outer_end: usize,
    },
    /// The declarator is a macro (`SIZED(size) alloc_a(int size)`,
    /// `pg_attribute_unused()` then `RT_DUMP_NODE(RT_NODE * node)`): the
    /// definition is the function the call after it names — rule 1, under
    /// that name (never the macro's).
    MacroFirst { name: String },
    /// Something after the parameters the parser could not read: rule 1.
    Unreadable,
}

/// C's type and storage words: a return type's, before a second head.
const TYPE_WORDS: &[&str] = &[
    "void",
    "char",
    "short",
    "int",
    "long",
    "float",
    "double",
    "signed",
    "unsigned",
    "_Bool",
    "bool",
    "const",
    "volatile",
    "struct",
    "union",
    "enum",
    "static",
    "extern",
    "inline",
    "register",
    "restrict",
    "_Atomic",
    "_Noreturn",
];

/// A word that can only be (part of) a type: a type or storage keyword, or
/// a `…_t` name.
fn type_word(word: &str) -> bool {
    TYPE_WORDS.contains(&word) || (word.len() > 2 && word.ends_with("_t"))
}

/// A word that completes a return type on its own (`int`, `size_t`) — after
/// it, `X (name)` is a macro-made name, not a typedef'd return type.
fn base_type_word(word: &str) -> bool {
    matches!(
        word,
        "void"
            | "char"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "_Bool"
            | "bool"
    ) || (word.len() > 2 && word.ends_with("_t"))
}

/// Whether an argument list's text reads as a parameter list: `(void)`, a
/// first item starting with a type word (`int size`), or two or more names
/// with only `*` between (`size_t n`, `RT_NODE * node`). An annotation's
/// arguments (`(1, 2)`, `(lock)`, `(&lock)`, `(macos(11.0))`) never do (fix
/// pass 2's check).
fn decl_shaped(args: &str) -> bool {
    let inner = args.trim();
    let inner = inner
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .unwrap_or(inner);
    let mut depth = 0i32;
    let first = inner
        .split(|c: char| {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ => {}
            }
            c == ',' && depth == 0
        })
        .next()
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        return false;
    }
    let first = first.split('[').next().unwrap_or(first);
    let mut words = 0;
    for token in first.split(|c: char| c.is_whitespace() || c == '*') {
        if token.is_empty() {
            continue;
        }
        let word = token
            .chars()
            .enumerate()
            .all(|(k, c)| c == '_' || c.is_ascii_alphabetic() || (k > 0 && c.is_ascii_digit()));
        if !word {
            return false;
        }
        if words == 0 && type_word(token) {
            return true;
        }
        words += 1;
    }
    words >= 2
}

/// The names a parameter list declares: each top-level item's last plain
/// word that is not a type word (`(size_t n, const char *s)` → n, s).
fn parameter_names(list: &str) -> Vec<String> {
    let inner = list.trim();
    let inner = inner
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .unwrap_or(inner);
    top_level_items(inner)
        .iter()
        .filter_map(|item| {
            let item = item.split('[').next().unwrap_or(item);
            item.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .rfind(|w| {
                    !w.is_empty() && !type_word(w) && !w.starts_with(|c: char| c.is_ascii_digit())
                })
                .map(str::to_string)
        })
        .collect()
}

/// The top-level, comma-separated items of an argument list's inside.
fn top_level_items(inner: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut from = 0;
    for (at, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                items.push(inner[from..at].trim());
                from = at + 1;
            }
            _ => {}
        }
    }
    items.push(inner[from..].trim());
    items
}

/// Whether every word of `args` (numbers aside) is one of `names` — an
/// annotation naming the function's own parameters (`__sized_by(n * size)`
/// after `(size_t n, size_t size)`), where a second head declares new ones
/// (fix pass 3's check).
fn names_only(args: &str, names: &[String]) -> bool {
    let words: Vec<&str> = args
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty() && !w.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    !words.is_empty() && words.iter().all(|w| names.iter().any(|n| n == w))
}

/// Whether every top-level item of an argument list is one plain name (K&R
/// `after(x, y)`).
fn plain_names(args: &str) -> bool {
    let inner = args.trim();
    let inner = inner
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .unwrap_or(inner);
    !inner.trim().is_empty()
        && top_level_items(inner).iter().all(|item| {
            !item.is_empty()
                && item.chars().enumerate().all(|(k, c)| {
                    c == '_' || c.is_ascii_alphabetic() || (k > 0 && c.is_ascii_digit())
                })
        })
}

/// Whether a function declarator holds anything after its parameter list —
/// a nested declarator is two heads only then (fix pass 3's check: a bare
/// `PREFIX(name)(params)` is a macro-made name, not two heads).
fn has_trailing(declarator: tree_sitter::Node) -> bool {
    let Some(parameters) = declarator.child_by_field_name("parameters") else {
        return false;
    };
    let mut cursor = declarator.walk();
    let found = declarator
        .named_children(&mut cursor)
        .skip_while(|c| c.id() != parameters.id())
        .nth(1)
        .is_some();
    found
}

/// What follows a definition's parameter list (fix pass 3: the second head
/// in any layout — GNU's return type on its own line, one line, K&R, a
/// pointer or parenthesized name — while annotation macros keep the note).
/// A call after the parameters is a head when a type word stands right
/// before it (`int`, `static`, `size_t`; `*` skipped), a type word follows
/// it (K&R), its arguments read as parameters, or its "function" is a type
/// keyword (`int (after)(void)`, `void (*after(void))(int)`). A declarator
/// whose declarator is another function declarator (no C function returns
/// a function) is two heads.
fn heads<'t>(def: tree_sitter::Node<'t>, src: &[u8]) -> Heads<'t> {
    let Some(mut node) = def.child_by_field_name("declarator") else {
        return Heads::One;
    };
    while node.kind() != "function_declarator" {
        match inner_declarator(node) {
            Some(inner) => node = inner,
            None => return Heads::One,
        }
    }
    let outer_end = node.end_byte();
    let mut nested = false;
    while let Some(inner) = node
        .child_by_field_name("declarator")
        .filter(|d| d.kind() == "function_declarator" && has_trailing(*d))
    {
        node = inner;
        nested = true;
    }
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return Heads::One;
    };
    // Comments blanked, as in the calls' arguments below (fix pass 7's
    // check: read both sides alike).
    let first_owned =
        String::from_utf8(blank_comments(text(parameters, src).as_bytes())).unwrap_or_default();
    let first_params = first_owned.as_str();
    let own_names = parameter_names(first_params);
    // A stray parse error between the declarator and the body: K&R
    // declarations the parser could not read — a second head's (fix pass 3's
    // check: `Count\nafter(x)\n\tCount x;`).
    let stray = match (
        def.child_by_field_name("declarator").map(|d| d.end_byte()),
        def.child_by_field_name("body").map(|b| b.start_byte()),
    ) {
        (Some(end), Some(body)) => {
            let mut c = def.walk();
            let found = def
                .children(&mut c)
                .any(|n| n.is_error() && n.start_byte() >= end && n.end_byte() <= body);
            found
        }
        _ => false,
    };
    let mut cursor = node.walk();
    // Comments are no part of a head (fix pass 5's check: `after(/* void
    // */)`, `Count /* r */ after()`).
    let trailing: Vec<tree_sitter::Node> = node
        .named_children(&mut cursor)
        .skip_while(|c| c.id() != parameters.id())
        .skip(1)
        .filter(|c| c.kind() != "comment")
        .collect();
    let star =
        |n: &tree_sitter::Node| n.is_error() && text(*n, src).trim().chars().all(|c| c == '*');
    let word_node = |n: &tree_sitter::Node| {
        matches!(
            n.kind(),
            "identifier" | "primitive_type" | "type_identifier" | "sized_type_specifier"
        )
    };
    // The qualifying calls, ranked (fix pass 3's check: a body macro with
    // arguments before the next head, `NOT_IMPL(-1)` then `int after(void)`,
    // must not take the name): the first call a type word stands before (or
    // whose "function" is a keyword) wins; else the last that qualifies by
    // its arguments or a K&R word after it. The last call of a nested
    // declarator, a parenthesized name, is the head the outer parameters
    // belong to.
    type Candidate<'a> = (Option<String>, tree_sitter::Node<'a>, bool);
    let mut by_position: Option<Candidate> = None;
    let mut by_shape: Option<Candidate> = None;
    // A K&R head's parameter names: a later `int cb(int)` declares one of
    // them, not a head (fix pass 4's check).
    let mut knr_params: Vec<String> = Vec::new();
    // Whether the by-shape head names its parameters (`(void)`, `(int x)`,
    // K&R names): an empty call after it is an annotation (`after(void) A
    // B NAME()`), where after `STUB(int)` it is the head (`Count after()`;
    // fix pass 5's check).
    let mut by_shape_named = false;
    for (k, call) in trailing.iter().enumerate() {
        if call.kind() != "call_expression" {
            continue;
        }
        let before: Vec<&tree_sitter::Node> = trailing[..k].iter().filter(|n| !star(n)).collect();
        // Attributes between the return type and the name are skipped.
        let typed: Vec<&tree_sitter::Node> = before
            .iter()
            .copied()
            .filter(|n| n.kind() != "attribute_specifier")
            .collect();
        let args = call.child_by_field_name("arguments");
        // Comments blanked: `after(/* in */ int x)` reads as parameters (fix
        // pass 6's check).
        let args_owned = String::from_utf8(blank_comments(
            args.map(|a| text(a, src)).unwrap_or("").as_bytes(),
        ))
        .unwrap_or_default();
        let args_text = args_owned.as_str();
        // An empty parameter list after a word that is not the first after
        // the parameters: a typedef or tag return type (`Count\nafter()`,
        // `struct s after()`; fix pass 4's check). One annotation word then
        // `NAME()` keeps the note.
        let empty_after_word = args.is_some_and(|a| {
            let mut c = a.walk();
            let none = a.named_children(&mut c).all(|n| n.kind() == "comment");
            none
        }) && !args_text.contains("...")
            && typed.len() >= 2
            && typed.last().is_some_and(|n| word_node(n));
        let typed_type_word = typed
            .last()
            .is_some_and(|n| word_node(n) && type_word(text(**n, src)));
        let word_before = typed_type_word || empty_after_word;
        let words_after: Vec<&str> = trailing[k + 1..]
            .iter()
            .filter(|n| word_node(n))
            .map(|n| text(*n, src))
            .collect();
        let knr = trailing
            .get(k + 1)
            .is_some_and(|n| word_node(n) && type_word(text(*n, src)))
            || (stray
                && plain_names(args_text)
                && parameter_names(args_text)
                    .iter()
                    .any(|name| words_after.contains(&name.as_str())));
        let decl = decl_shaped(args_text) && !names_only(args_text, &own_names);
        let function = call.child_by_field_name("function");
        let keyword_fn = function.is_some_and(|f| C_KEYWORDS.contains(&text(f, src)));
        // A nested declarator's last call with one name inside, after no
        // base type word: a parenthesized head with a typedef'd return type
        // (`static Count (after)(void)`; fix pass 4's check), where `int
        // PREFIX(name)(void)` is a macro-made name.
        let one_inside = args.is_some_and(|a| {
            let mut c = a.walk();
            let inside = a
                .named_children(&mut c)
                .filter(|n| !n.is_error() && n.kind() != "comment")
                .count();
            inside == 1
        });
        let paren_head = nested
            && k + 1 == trailing.len()
            && one_inside
            && !typed
                .last()
                .is_some_and(|n| word_node(n) && base_type_word(text(**n, src)));
        if !(word_before || knr || decl || keyword_fn || paren_head) {
            continue;
        }
        if stray
            && function.is_some_and(|f| {
                f.kind() == "identifier" && knr_params.iter().any(|p| p == text(f, src))
            })
        {
            continue;
        }
        if knr && plain_names(args_text) {
            knr_params.extend(parameter_names(args_text));
        }
        let name = if keyword_fn || paren_head {
            args.and_then(|a| named_inside(a, src))
        } else {
            function
                .filter(|f| f.kind() == "identifier")
                .map(|f| text(f, src).to_string())
        }
        .filter(|n| !C_KEYWORDS.contains(&n.as_str()));
        // A macro before the real name: only attributes between the macro's
        // parentheses and the call, and the macro's arguments are not a
        // parameter list of their own (or name the call's parameters).
        let macro_first = decl
            && !word_before
            && !knr
            && !keyword_fn
            && !nested
            && before.iter().all(|n| n.kind() == "attribute_specifier")
            && (!decl_shaped(first_params)
                || names_only(first_params, &parameter_names(args_text)));
        let candidate = (name, *call, macro_first);
        if paren_head {
            by_position = Some(candidate);
        } else if empty_after_word && !typed_type_word && !keyword_fn {
            // An empty call after words ranks with the calls that qualify
            // by shape: a body macro spelled `STUB_BODY()` never takes a
            // later head's place (fix pass 5's check).
            if !by_shape_named {
                by_shape = Some(candidate);
            }
        } else if word_before || keyword_fn {
            if by_position.is_none() {
                by_position = Some(candidate);
            }
        } else {
            let inner = args_text.trim();
            let inner = inner
                .strip_prefix('(')
                .and_then(|r| r.strip_suffix(')'))
                .unwrap_or(inner);
            by_shape_named = inner.trim() == "void"
                || (knr && plain_names(args_text))
                || (!inner.trim().is_empty()
                    && top_level_items(inner).iter().all(|item| {
                        *item == "..."
                            || item
                                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                                .filter(|w| !w.is_empty())
                                .count()
                                >= 2
                    }));
            by_shape = Some(candidate);
        }
    }
    if let Some((name, call, macro_first)) = by_position.or(by_shape) {
        let Some(name) = name else {
            return Heads::Unreadable;
        };
        return if macro_first {
            Heads::MacroFirst { name }
        } else {
            // A nested head's declarator runs past its call to the outer
            // parameters; any other head's ends at its call (fix pass 4's
            // check: not at the first head's declarator's end).
            let last = trailing.last().is_some_and(|t| t.id() == call.id());
            Heads::Two {
                name,
                call,
                first_end: parameters.end_byte(),
                outer_end: if nested && last {
                    outer_end
                } else {
                    call.end_byte()
                },
            }
        };
    }
    if nested || stray || trailing.iter().any(|c| c.is_error()) {
        Heads::Unreadable
    } else {
        Heads::One
    }
}

/// The function named inside a keyword-headed call's arguments: `(after)`
/// or `(*after(void))`.
fn named_inside(args: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    let found = args
        .named_children(&mut cursor)
        .find_map(|n| match n.kind() {
            "identifier" => Some(text(n, src).to_string()),
            "call_expression" => n
                .child_by_field_name("function")
                .filter(|f| f.kind() == "identifier")
                .map(|f| text(f, src).to_string()),
            _ => None,
        });
    found
}

/// The second of two heads (see [`heads`]), with the body the parser gave
/// the first: recorded, unwatched (rule 1). Its signature runs from the type
/// words right before it; it is static when `static` stands between the
/// first head's parameters and it.
fn second_head(
    def: tree_sitter::Node,
    name: String,
    call: tree_sitter::Node,
    (parameters_end, outer_end): (usize, usize),
    src: &[u8],
    file: &str,
) -> FnDef {
    let between = &src[parameters_end.min(call.start_byte())..call.start_byte()];
    let words: Vec<&[u8]> = between
        .split(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
        .filter(|w| !w.is_empty())
        .collect();
    let is_static = words.contains(&&b"static"[..]);
    let start = return_type_start(src, parameters_end, call.start_byte());
    // A nested second head's declarator runs past the call (`int
    // (after)(void)`, `void (*after(int x))(int)`): its signature does too.
    let end = outer_end.max(call.end_byte());
    let raw = String::from_utf8_lossy(&src[start..end]);
    let mut calls = BTreeSet::new();
    if let Some(body) = def.child_by_field_name("body") {
        collect_calls(body, src, &mut calls);
    }
    FnDef {
        name,
        file: file.to_string(),
        is_static,
        signature: raw.split_whitespace().collect::<Vec<_>>().join(" "),
        span: (
            (call.start_position().row + 1) as u32,
            (def.end_position().row + 1) as u32,
        ),
        calls,
        note_at: Err(NoNote::Parser),
        naked_head: false,
        nested: false,
    }
}

/// Where a head's return type starts: the trailing run of type words (and
/// `*`) before `at`, back no further than `floor` (the first head's
/// parameters' end).
fn return_type_start(src: &[u8], floor: usize, at: usize) -> usize {
    let mut start = at;
    let mut at = at;
    loop {
        let head = &src[floor.min(at)..at];
        let trimmed = head.trim_ascii_end();
        if let Some(b'*') = trimmed.last() {
            at = floor + trimmed.len() - 1;
            continue;
        }
        let word_at = trimmed
            .iter()
            .rposition(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
            .map_or(0, |p| p + 1);
        let word = String::from_utf8_lossy(&trimmed[word_at..]);
        if word.is_empty() || !type_word(&word) {
            break;
        }
        at = floor + word_at;
        start = at;
    }
    start
}

/// A first head the parser folded into a parse error before the declarator
/// (`int g(void) NI` then `static char **after(void) {`, or `static Count
/// *\nafter(int x)`; fix pass 4's check): whether this definition is static
/// — only a `static` after that head's parameters is its own — and where its
/// signature starts: its first type word after them, else the last word
/// before the declarator (a typedef'd return type). A first head names a
/// function and declares parameters (an attribute macro, `static int M(x)
/// **after(int x)`, is none), and a `;` after it means a declaration stood
/// there (SDK prototypes): `None`, read as before.
fn folded_head(def: tree_sitter::Node, src: &[u8]) -> Option<(bool, usize)> {
    // A first head returning a pointer (`int *g(void) NI`) leaves its parse
    // error inside the definition's pointer declarator: look before each
    // declarator down that chain (fix pass 5's check).
    let mut scope = def;
    let mut declarator = def.child_by_field_name("declarator")?;
    let error = loop {
        let mut c = scope.walk();
        let found = scope
            .children(&mut c)
            .find(|n| n.is_error() && n.end_byte() <= declarator.start_byte());
        if let Some(found) = found {
            break found;
        }
        if declarator.kind() != "pointer_declarator" {
            return None;
        }
        scope = declarator;
        declarator = declarator.child_by_field_name("declarator")?;
    };
    let mut e = error.walk();
    let head = error
        .named_children(&mut e)
        .find(|n| n.kind() == "function_declarator")?;
    let parameters = head.child_by_field_name("parameters")?;
    let params = text(parameters, src);
    // The first head names a function: an identifier, or one inside a
    // parenthesized declarator (`int (*g(void))(int)`, a function pointer).
    let first = head.child_by_field_name("declarator");
    let mut inner = first;
    while let Some(n) = inner.filter(|n| {
        matches!(
            n.kind(),
            "parenthesized_declarator" | "pointer_declarator" | "function_declarator"
        )
    }) {
        inner = inner_declarator(n);
    }
    let named = first.is_some_and(|d| d.kind() == "identifier")
        || (first.is_some_and(|d| d.kind() == "parenthesized_declarator")
            && inner.is_some_and(|d| d.kind() == "identifier"));
    let empty = params.split_whitespace().collect::<String>() == "()";
    if !named || !(decl_shaped(params) || empty) {
        return None;
    }
    let first_end = parameters.end_byte();
    let gap = &src[first_end..declarator.start_byte()];
    if gap.contains(&b';') {
        return None;
    }
    // A comment between the heads is no word of either.
    let gap = &blank_comments(gap)[..];
    let mut is_static = false;
    let mut first_type = None;
    let mut last_word = None;
    let mut at = 0;
    while at < gap.len() {
        if gap[at].is_ascii_alphanumeric() || gap[at] == b'_' {
            let end = gap[at..]
                .iter()
                .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
                .map_or(gap.len(), |p| at + p);
            let word = std::str::from_utf8(&gap[at..end]).unwrap_or("");
            is_static |= word == "static";
            if first_type.is_none() && type_word(word) {
                first_type = Some(at);
            }
            last_word = Some(at);
            at = end;
        } else {
            at += 1;
        }
    }
    // A first head leaves its body macro between it and the second head: an
    // empty-parentheses macro right before `**name` is no first head (fix
    // pass 6's check).
    last_word?;
    let start = first_type
        .or(last_word)
        .map_or(declarator.start_byte(), |w| first_end + w);
    Some((is_static, start))
}

/// `gap` with its comments turned to spaces (offsets kept).
fn blank_comments(gap: &[u8]) -> Vec<u8> {
    let mut out = gap.to_vec();
    let mut i = 0;
    while i < out.len() {
        let end = if out[i] == b'/' && out.get(i + 1) == Some(&b'*') {
            out[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(out.len(), |at| i + 2 + at + 2)
        } else if out[i] == b'/' && out.get(i + 1) == Some(&b'/') {
            out[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map_or(out.len(), |at| i + at)
        } else {
            i += 1;
            continue;
        };
        for b in &mut out[i..end] {
            *b = b' ';
        }
        i = end;
    }
    out
}

/// The declarator inside `node` (a pointer, parenthesized or C23
/// attributed declarator).
fn inner_declarator(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    node.child_by_field_name("declarator").or_else(|| {
        let mut cursor = node.walk();
        let found = node
            .named_children(&mut cursor)
            .find(|c| c.kind().ends_with("declarator"));
        found
    })
}

/// What a body holds that bears on its reading: `#if`-group directives and
/// braces (comments, string and character literals and line splices
/// skipped).
enum BodyEvent {
    If,
    /// `#else` (true) or an `#elif` (false).
    Else(bool),
    Endif,
    Brace(i64, usize),
}

fn body_events(body: &[u8]) -> Vec<BodyEvent> {
    let mut events = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < body.len() {
        let b = body[i];
        match b {
            b'\n' => {
                line_start = true;
                i += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                i += 1;
                continue;
            }
            b'\\' if matches!(body.get(i + 1), Some(b'\n' | b'\r')) => {
                i += 2;
                continue;
            }
            b'/' if body.get(i + 1) == Some(&b'*') => {
                i = body[i + 2..]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .map_or(body.len(), |at| i + 2 + at + 2);
                continue;
            }
            b'/' if body.get(i + 1) == Some(&b'/') => {
                i = body[i..]
                    .iter()
                    .position(|c| *c == b'\n')
                    .map_or(body.len(), |at| i + at);
                continue;
            }
            b'"' | b'\'' => {
                let mut j = i + 1;
                while j < body.len() && body[j] != b && body[j] != b'\n' {
                    j += if body[j] == b'\\' { 2 } else { 1 };
                }
                i = (j + 1).min(body.len());
                line_start = false;
                continue;
            }
            b'#' if line_start => {
                let rest = &body[i + 1..];
                let word_at = rest
                    .iter()
                    .position(|c| !matches!(c, b' ' | b'\t'))
                    .unwrap_or(rest.len());
                let word: Vec<u8> = rest[word_at..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .copied()
                    .collect();
                match word.as_slice() {
                    b"if" | b"ifdef" | b"ifndef" => events.push(BodyEvent::If),
                    b"elif" | b"elifdef" | b"elifndef" => events.push(BodyEvent::Else(false)),
                    b"else" => events.push(BodyEvent::Else(true)),
                    b"endif" => events.push(BodyEvent::Endif),
                    _ => {}
                }
                // The rest of the directive line, splices joined.
                while i < body.len() && body[i] != b'\n' {
                    i += if body[i] == b'\\' { 2 } else { 1 };
                }
                continue;
            }
            b'{' => events.push(BodyEvent::Brace(1, i)),
            b'}' => events.push(BodyEvent::Brace(-1, i)),
            _ => {}
        }
        line_start = false;
        i += 1;
    }
    events
}

/// Whether a function body as the parser read it holds an `#if` group with
/// two or more branches that each change the brace depth — the parser reads
/// every branch, so it runs the body on (sqlite3.c's winWrite and
/// decodeIntArray) — **and** some configuration does not read the same body:
/// its braces close before the parser's closing brace, or not at it. Every
/// combination of branches is walked over the groups a brace is read under
/// (a build may take any branch of each group, whatever its condition: an
/// `#ifndef A` beside an `#ifdef A`, or two macros of their own — fix pass
/// 4's check), with "none taken" for a group without `#else`; past
/// [`MOST_CONFIGURATIONS`] the body counts as misread (rule 1, which errs
/// safe). Groups whose excess braces cancel in every combination (a lock
/// taken in one `#if` and released in a later one, alike in each branch)
/// are read right (fix pass 2's check).
fn body_misread(body: &[u8]) -> bool {
    let events = body_events(body);
    // Per group (in order of its `#if`): its branches' brace changes, and
    // whether it has a plain `#else`.
    let mut groups: Vec<Vec<i64>> = Vec::new();
    let mut has_else: Vec<bool> = Vec::new();
    // Whether a brace is read under the group (directly or in a nested one).
    let mut relevant: Vec<bool> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    for event in &events {
        match event {
            BodyEvent::If => {
                open.push(groups.len());
                groups.push(vec![0]);
                has_else.push(false);
                relevant.push(false);
            }
            BodyEvent::Else(plain) => {
                if let Some(g) = open.last() {
                    groups[*g].push(0);
                    has_else[*g] |= *plain;
                }
            }
            BodyEvent::Endif => {
                open.pop();
            }
            BodyEvent::Brace(step, _) => {
                for g in &open {
                    relevant[*g] = true;
                }
                if let Some(g) = open.last() {
                    if let Some(depth) = groups[*g].last_mut() {
                        *depth += step;
                    }
                }
            }
        }
    }
    let two_branch = groups
        .iter()
        .any(|g| g.iter().filter(|d| **d != 0).count() >= 2);
    if !two_branch {
        return false;
    }
    let last_brace = body.iter().rposition(|c| *c == b'}');
    let axes: Vec<usize> = (0..groups.len()).filter(|g| relevant[*g]).collect();
    // A group's choices: each branch, plus none taken when it has no `#else`.
    let choices: Vec<usize> = axes
        .iter()
        .map(|g| groups[*g].len() + usize::from(!has_else[*g]))
        .collect();
    let mut total: usize = 1;
    for c in &choices {
        total = total.saturating_mul(*c);
        if total > MOST_CONFIGURATIONS {
            return true;
        }
    }
    let mut pick = vec![0usize; groups.len()];
    (0..total).any(|mut n| {
        for (axis, g) in axes.iter().enumerate() {
            pick[*g] = n % choices[axis];
            n /= choices[axis];
        }
        // Per open group: its index and the branch being read.
        let mut open: Vec<(usize, usize)> = Vec::new();
        let mut next_group = 0;
        let mut depth = 0i64;
        let mut closed_early = false;
        for event in &events {
            match event {
                BodyEvent::If => {
                    open.push((next_group, 0));
                    next_group += 1;
                }
                BodyEvent::Else(_) => {
                    if let Some((_, branch)) = open.last_mut() {
                        *branch += 1;
                    }
                }
                BodyEvent::Endif => {
                    open.pop();
                }
                BodyEvent::Brace(step, at) => {
                    let active = open.iter().all(|(g, branch)| *branch == pick[*g]);
                    if active {
                        depth += step;
                        if depth == 0 && Some(*at) != last_brace {
                            closed_early = true;
                        }
                    }
                }
            }
        }
        closed_early || depth != 0
    })
}

/// The most branch combinations [`body_misread`] walks; past it a body
/// counts as misread.
const MOST_CONFIGURATIONS: usize = 4096;

/// The identifier a function definition names, through pointer, parenthesized
/// and C23-attributed declarators to the function declarator (and, for two
/// heads run together, into the first head's).
fn function_name(def: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut node = def.child_by_field_name("declarator")?;
    loop {
        match node.kind() {
            "function_declarator" => {
                let mut decl = node.child_by_field_name("declarator")?;
                // C23: `int f [[gnu::cold]] (void)`.
                while decl.kind() == "attributed_declarator" {
                    decl = decl.named_child(0)?;
                }
                // Two heads run together (`int g(void) NI` then `int
                // (after)(void)`): the first head's name.
                // (Only when the inner declarator holds something after its
                // parameters: a bare `PREFIX(name)(params)` is a macro-made
                // name, unread — fix pass 3's check.)
                if decl.kind() == "function_declarator" {
                    if !has_trailing(decl) {
                        return None;
                    }
                    node = decl;
                    continue;
                }
                // A parenthesized name stays unread (the hidden-variant check).
                return if decl.kind() == "identifier" {
                    Some(text(decl, src).to_string())
                } else {
                    None
                };
            }
            "pointer_declarator" | "parenthesized_declarator" | "attributed_declarator" => {
                let next = match node.child_by_field_name("declarator") {
                    Some(n) => Some(n),
                    None => {
                        let mut c = node.walk();
                        let found = node.children(&mut c).find(|n| n.is_named());
                        found
                    }
                };
                node = next?;
            }
            _ => return None,
        }
    }
}

/// Recursively collect names appearing as direct (identifier) callees.
fn collect_calls(node: tree_sitter::Node, src: &[u8], out: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(f) = child.child_by_field_name("function") {
                if f.kind() == "identifier" {
                    out.insert(text(f, src).to_string());
                }
            }
        }
        collect_calls(child, src, out);
    }
}

fn text<'a>(node: tree_sitter::Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_facts_reads_both_include_forms_from_every_branch_and_any_bytes() {
        // A Latin-1 comment (0xE9) is not UTF-8; the parse takes bytes.
        let src: &[u8] = b"/* caf\xe9 */\n#include \"util.h\"\n#ifdef X\n#include <proj/api.h>\n\
            #else\n#include <stdio.h>\n#endif\n#include NAME\n\
            static int helper(void) { return 1; }\nint run(void) { return helper() + go(); }\n";
        let facts = file_facts(src).expect("parses");
        assert_eq!(facts.functions, ["helper", "run"]);
        assert_eq!(
            facts.calls.iter().map(String::as_str).collect::<Vec<_>>(),
            ["go", "helper"]
        );
        let inc = |name: &str, quoted| Include {
            name: name.into(),
            quoted,
        };
        assert_eq!(
            facts.includes,
            [
                inc("proj/api.h", false),
                inc("stdio.h", false),
                inc("util.h", true)
            ]
        );
    }

    /// The real zopfli target vendored in this repository.
    fn zopfli_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli")
    }

    #[test]
    fn scans_real_zopfli_target() {
        harness_core::adopt::testing::adopt(zopfli_root());
        let target = TargetContext::load(zopfli_root()).expect("load target context");
        let facts = CFrontend.scan(&target).expect("scan");

        assert_eq!(facts.frontend, "c-tree-sitter");
        assert!(
            facts.symbols.len() >= 100,
            "expected >=100 symbols, got {}",
            facts.symbols.len()
        );

        let public = facts
            .symbols
            .iter()
            .find(|s| s.name == "ZopfliLengthLimitedCodeLengths")
            .expect("ZopfliLengthLimitedCodeLengths symbol");
        assert_eq!(public.visibility, "public");
        assert_eq!(public.file, "src/zopfli/katajainen.c");
        assert_eq!(public.kind, "function");

        let internal = facts
            .symbols
            .iter()
            .find(|s| s.name == "src/zopfli/katajainen.c::BoundaryPM")
            .expect("BoundaryPM symbol");
        assert_eq!(internal.visibility, "internal");

        let katajainen = facts
            .files
            .iter()
            .find(|f| f.path == "src/zopfli/katajainen.c")
            .expect("katajainen.c file record");
        assert!(
            katajainen
                .includes
                .contains(&"src/zopfli/katajainen.h".to_string()),
            "includes = {:?}",
            katajainen.includes
        );

        assert!(
            facts.refs.iter().any(|r| {
                r.from == "src/zopfli/katajainen.c::BoundaryPM"
                    && r.to == "src/zopfli/katajainen.c::BoundaryPM"
                    && r.refkind == "call"
                    && r.resolved
            }),
            "self-recursion edge for BoundaryPM missing"
        );

        let again = CFrontend.scan(&target).expect("second scan");
        assert_eq!(
            facts.to_canonical_bytes().expect("bytes"),
            again.to_canonical_bytes().expect("bytes"),
            "canonical bytes differ between two scans"
        );
    }

    /// A throwaway target root under the system temp dir, removed on drop.
    struct TempTarget(PathBuf);

    impl TempTarget {
        fn new(tag: &str, include_dirs: &str) -> TempTarget {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ruharness-scan-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("src")).expect("src dir");
            std::fs::write(
                dir.join("harness.toml"),
                format!(
                    "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n\
                     include_dirs = {include_dirs}\n"
                ),
            )
            .expect("harness.toml");
            TempTarget(dir.canonicalize().expect("canonical"))
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        }

        fn includes_of(&self, rel: &str) -> Vec<String> {
            let target = TargetContext::load(&self.0).expect("target loads");
            let facts = CFrontend.scan(&target).expect("scan");
            facts
                .files
                .iter()
                .find(|f| f.path == rel)
                .map(|f| f.includes.clone())
                .unwrap_or_else(|| panic!("{rel} not scanned: {:?}", facts.files))
        }
    }

    impl Drop for TempTarget {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn quoted_includes_search_own_dir_then_include_dirs_in_order() {
        let t = TempTarget::new("incdirs", "[\"src/include\", \"src/more\"]");
        t.write(
            "src/lib/a.c",
            "#include \"lib.h\"\n#include \"dup.h\"\n#include \"only_more.h\"\n\
             #include \"missing.h\"\nint a(void) { return 0; }\n",
        );
        t.write("src/lib/dup.h", "/* own dir wins */\n");
        t.write("src/include/lib.h", "int a(void);\n");
        t.write("src/include/dup.h", "/* shadowed */\n");
        t.write(
            "src/more/lib.h",
            "/* shadowed by the first include dir */\n",
        );
        t.write("src/more/only_more.h", "\n");
        assert_eq!(
            t.includes_of("src/lib/a.c"),
            vec![
                "src/include/lib.h".to_string(),
                "src/lib/dup.h".to_string(),
                "src/more/only_more.h".to_string(),
            ]
        );
    }

    #[test]
    fn a_resolution_outside_source_dir_is_never_recorded() {
        let t = TempTarget::new("incescape", "[\"src/include\"]");
        t.write(
            "src/a.c",
            "#include \"../../outside/x.h\"\n#include \"../outside/x.h\"\n\
             #include \"../src/ok.h\"\nint a(void) { return 0; }\n",
        );
        t.write("src/ok.h", "\n");
        t.write("src/include/.keep.h", "\n");
        // Exists, but outside source_dir: from src/include, `../../outside/x.h`
        // lands on it lexically, and from src/, `../outside/x.h` does too.
        t.write("outside/x.h", "secret\n");
        assert_eq!(t.includes_of("src/a.c"), vec!["src/ok.h".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_walk_never_follows_a_symlink_out_of_source_dir() {
        let t = TempTarget::new("symlink", "[]");
        t.write(
            "src/a.c",
            "#include \"escape/x.h\"\nint a(void) { return 0; }\n",
        );
        t.write("outside/x.h", "secret\n");
        std::os::unix::fs::symlink(t.0.join("outside"), t.0.join("src/escape")).expect("symlink");
        // A cycle back into source_dir is walked once, not forever.
        std::os::unix::fs::symlink(t.0.join("src"), t.0.join("src/loop")).expect("symlink");
        let target = TargetContext::load(&t.0).expect("target loads");
        let facts = CFrontend.scan(&target).expect("scan terminates");
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.c"]);
        assert!(facts.files[0].includes.is_empty());
    }

    /// docs/COCKPIT-WRAPPER-DESIGN.md §2.1 and docs/PROJECT-MAP-DESIGN.md
    /// §5 (a): on the shared walk the facts are byte-identical to the
    /// committed ones — every committed `migration/facts.jsonl` in the
    /// repository: zopfli and the 100 benchmark cases.
    #[test]
    fn the_shared_walk_keeps_the_committed_facts_byte_identical() {
        fn roots(dir: &Path, out: &mut Vec<PathBuf>) {
            if dir.join("migration/facts.jsonl").is_file() {
                out.push(dir.to_path_buf());
            }
            let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                .expect("read dir")
                .map(|e| e.expect("entry").path())
                .collect();
            entries.sort();
            for path in entries {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let real_dir = std::fs::symlink_metadata(&path)
                    .map(|m| m.is_dir())
                    .unwrap_or(false);
                if real_dir && !name.starts_with('.') && name != "migration" && name != "target" {
                    roots(&path, out);
                }
            }
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut found = Vec::new();
        roots(&repo.join("targets"), &mut found);
        assert_eq!(found.len(), 101, "{found:?}");
        for root in found {
            harness_core::adopt::testing::adopt(&root);
            let target = TargetContext::load(&root).expect("target loads");
            let facts = CFrontend.scan(&target).expect("scan");
            let committed =
                std::fs::read(root.join("migration/facts.jsonl")).expect("committed facts");
            assert!(
                facts.to_canonical_bytes().expect("bytes") == committed,
                "{}: the facts differ from the committed ones",
                root.display()
            );
        }
    }

    /// A FIFO (or any non-regular file) with a C name is skipped and
    /// reported — never read, so it can no longer hang a scan — and is a
    /// walk issue. A link to a file inside source_dir is not a second file:
    /// the file is scanned once, under its real path.
    #[cfg(unix)]
    #[test]
    fn a_fifo_named_like_c_is_skipped_and_an_inside_link_is_scanned_once() {
        let t = TempTarget::new("fifo", "[]");
        t.write("src/a.c", "int a(void) { return 0; }\n");
        std::os::unix::fs::symlink(t.0.join("src/a.c"), t.0.join("src/alias.c")).expect("symlink");
        assert!(std::process::Command::new("mkfifo")
            .arg(t.0.join("src/pipe.c"))
            .status()
            .expect("mkfifo")
            .success());
        let target = TargetContext::load(&t.0).expect("target loads");
        let (facts, notes) = CFrontend.scan_reporting(&target).expect("scan terminates");
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.c"]);
        assert_eq!(
            notes.skipped,
            vec![(t.0.join("src/pipe.c"), "not a regular file".to_string())]
        );
        let walked = walk::confined(&t.0.join("src"), &C_EXTENSIONS, walk::Limits::default());
        assert_eq!(
            walked.issues,
            [walk::Issue {
                path: t.0.join("src/pipe.c"),
                why: walk::Why::NotRegular,
            }]
        );
        assert_eq!(walked.aliases.len(), 1);
    }

    /// A liblzg-shaped project (docs/PROJECT-MAP-DESIGN.md §4): a library
    /// folder whose header includes `"../include/lzg.h"`, a tool including
    /// `<lzg.h>` from `src/include`, a header nothing includes, and a
    /// model-written file in the tool's ledger. Removed on drop.
    struct Lzg(PathBuf);

    impl Drop for Lzg {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const LZG_FILES: &[(&str, &str)] = &[
        (
            "src/lib/checksum.c",
            "#include \"internal.h\"\n\
             unsigned lzg_checksum(const unsigned char *p, unsigned n) {\n\
             \x20   unsigned s = 0;\n\
             \x20   while (n--) s += *p++;\n\
             \x20   return s;\n}\n",
        ),
        (
            "src/lib/encode.c",
            "#include \"internal.h\"\nint lzg_level = 3;\n\
             static int clamp(int x) { return lzg_min(x, 9); }\n\
             unsigned lzg_encode(const unsigned char *p, unsigned n) {\n\
             \x20   return lzg_checksum(p, n) + (unsigned)clamp(lzg_level);\n}\n",
        ),
        (
            "src/lib/version.c",
            "#include \"lzg.h\"\nint lzg_version(void) { return LZG_VERSION; }\n",
        ),
        (
            "src/lib/internal.h",
            "#include \"../include/lzg.h\"\n\
             static inline int lzg_min(int a, int b) { return a < b ? a : b; }\n",
        ),
        (
            "src/include/lzg.h",
            "#define LZG_VERSION 0x010304\n#define LZG_MAX(a, b) ((a) > (b) ? (a) : (b))\n\
             unsigned lzg_checksum(const unsigned char *p, unsigned n);\n\
             unsigned lzg_encode(const unsigned char *p, unsigned n);\n\
             int lzg_version(void);\n",
        ),
        (
            "src/tools/lzg.c",
            "#include <stdio.h>\n#include <lzg.h>\n\
             static void say(const char *fmt, ...) { (void)fmt; }\n\
             int main(void) {\n\
             \x20   say(\"%d\", lzg_version());\n\
             \x20   return (int)lzg_encode(0, 0);\n}\n",
        ),
    ];

    const LZG_TOML: &str = "schema_version = 2\n[target]\nname = \"lzg\"\nfiles = [\n\
        { path = \"src/lib/checksum.c\", include_dirs = [\"src/include\"] },\n\
        { path = \"src/lib/encode.c\", include_dirs = [\"src/include\"] },\n\
        { path = \"src/lib/version.c\", include_dirs = [\"src/include\"] },\n\
        { path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] },\n]\n\
        configuration = { name = \"make\", from = \"stated\", flags = [] }\n";

    fn tmp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ruharness-scan-lzg-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).expect("tmp");
        dir.canonicalize().expect("canonical")
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    /// The fixture as the mapped tool `t-lzg`, its context built by hand
    /// (no adoption needed: nothing here opens a ledger made elsewhere).
    fn lzg(tag: &str) -> (Lzg, TargetContext) {
        let root = tmp_root(tag);
        for (rel, text) in LZG_FILES {
            put(&root, rel, text);
        }
        put(
            &root,
            "src/include/unused.h",
            "static inline int never_read(void) { return 0; }\n",
        );
        let ledger = harness_core::config::tool_dir(&root, "t-lzg");
        put(&ledger, "harness.toml", LZG_TOML);
        put(
            &ledger,
            "driver.c",
            "int model_written(void) { return 1; }\n",
        );
        let config =
            harness_core::config::TargetConfig::load_file(&ledger.join("harness.toml"), &root)
                .expect("the file list loads");
        let ctx = TargetContext {
            root: root.clone(),
            ledger,
            tool: Some("t-lzg".into()),
            config,
        };
        (Lzg(root), ctx)
    }

    /// Facts with every path's folder dropped (`src/lib/encode.c::clamp` is
    /// `encode.c::clamp`), for comparing two layouts of the same files.
    fn flattened(facts: &Facts) -> BTreeSet<String> {
        let flat = |s: &str| {
            ["src/lib/", "src/tools/", "src/include/", "flat/"]
                .iter()
                .fold(s.to_string(), |s, dir| s.replace(dir, ""))
        };
        let mut out = BTreeSet::new();
        for f in &facts.files {
            out.insert(format!("file {} {}", flat(&f.path), f.hash.len()));
            for to in &f.includes {
                out.insert(format!("include {} -> {}", flat(&f.path), flat(to)));
            }
        }
        for s in &facts.symbols {
            out.insert(format!(
                "symbol {} {} {} {:?}",
                flat(&s.name),
                flat(&s.file),
                s.visibility,
                s.span
            ));
        }
        for r in &facts.refs {
            out.insert(format!(
                "ref {} {} {} {}",
                flat(&r.from),
                flat(&r.file),
                flat(&r.to),
                r.resolved
            ));
        }
        out
    }

    /// docs/PROJECT-MAP-DESIGN.md §3.7 "The scanner": the listed files plus
    /// every header reached through each file's own include folders, both
    /// include forms, to closure — the same functions, calls and include
    /// edges as the folder form over the same files copied into one flat
    /// folder; the header nothing includes and the ledger's file never read.
    #[test]
    fn a_file_list_scans_like_the_folder_form_over_a_flat_copy() {
        let (t, ctx) = lzg("same");
        let (facts, notes) = CFrontend.scan_reporting(&ctx).expect("scan");
        assert_eq!(notes, ScanNotes::default());
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "src/include/lzg.h",
                "src/lib/checksum.c",
                "src/lib/encode.c",
                "src/lib/internal.h",
                "src/lib/version.c",
                "src/tools/lzg.c",
            ],
            "every header reached, nothing else"
        );
        let includes_of = |p: &str| {
            facts
                .files
                .iter()
                .find(|f| f.path == p)
                .map(|f| f.includes.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            includes_of("src/tools/lzg.c"),
            ["src/include/lzg.h"],
            "<lzg.h>"
        );
        assert_eq!(includes_of("src/lib/internal.h"), ["src/include/lzg.h"]);
        assert_eq!(includes_of("src/lib/version.c"), ["src/include/lzg.h"]);

        // The same files in one flat folder, their includes named plainly.
        let flat = tmp_root("flat");
        let _flat_guard = Lzg(flat.clone());
        for (rel, text) in LZG_FILES {
            let name = rel.rsplit('/').next().expect("name");
            let text = text
                .replace("\"../include/lzg.h\"", "\"lzg.h\"")
                .replace("<lzg.h>", "\"lzg.h\"");
            put(&flat, &format!("flat/{name}"), &text);
        }
        put(
            &flat,
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"lzg\"\nsource_dir = \"flat\"\n",
        );
        let folder = TargetContext::folder_form(
            flat.clone(),
            harness_core::config::TargetConfig::load(&flat).expect("folder config"),
        );
        let flat_facts = CFrontend.scan(&folder).expect("flat scan");
        assert_eq!(flattened(&facts), flattened(&flat_facts));
        assert!(facts
            .symbols
            .iter()
            .any(|s| s.name == "src/lib/encode.c::clamp"));
        drop(t);
    }

    /// A listed file or an include folder that leads under `migration/` or
    /// out of the project is refused by name, in one sentence — checked by
    /// the scanner itself, whatever the config's load saw.
    #[cfg(unix)]
    #[test]
    fn a_listed_file_under_the_ledger_or_a_folder_outside_the_root_is_refused() {
        use harness_core::config::{Form, TargetFile};
        let (_t, ctx) = lzg("refuse");
        let listing = |path: &str, dirs: &[&str]| {
            let mut ctx = ctx.clone();
            if let Form::FileList(list) = &mut ctx.config.target.form {
                list.files.push(TargetFile {
                    path: path.into(),
                    include_dirs: dirs.iter().map(|d| d.to_string()).collect(),
                });
            }
            CFrontend
                .scan_reporting(&ctx)
                .expect_err("refused")
                .to_string()
        };
        let err = listing("migration/tools/t-lzg/driver.c", &[]);
        assert!(
            err.contains("`migration/tools/t-lzg/driver.c` lies under migration/"),
            "{err}"
        );
        // Through a link inside the project.
        std::os::unix::fs::symlink(
            ctx.ledger.join("driver.c"),
            ctx.root.join("src/lib/sneaky.c"),
        )
        .expect("link");
        let err = listing("src/lib/sneaky.c", &[]);
        assert!(
            err.contains("`src/lib/sneaky.c` lies under migration/"),
            "{err}"
        );
        // A folder that leads outside the root.
        let outside = tmp_root("outside");
        let _outside_guard = Lzg(outside.clone());
        put(&outside, "secret.h", "int secret;\n");
        std::os::unix::fs::symlink(&outside, ctx.root.join("src/out")).expect("link");
        let err = listing("src/lib/version.c", &["src/out"]);
        assert!(
            err.contains("the include folder `src/out` of `src/lib/version.c` leads outside"),
            "{err}"
        );
        assert!(!err.contains('\n'), "{err:?}");
    }

    /// A walk error and a file that is not UTF-8 are notes of their paths,
    /// never a stop (docs/PROJECT-MAP-DESIGN.md §3.1 step 1).
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_file_and_an_unreadable_folder_are_noted_and_the_scan_goes_on() {
        use std::os::unix::fs::PermissionsExt;
        let t = TempTarget::new("odd", "[]");
        t.write("src/a.c", "int a(void) { return 0; }\n");
        std::fs::write(
            t.0.join("src/latin.c"),
            b"/* caf\xe9 */\nint latin(void) { return a(); }\n",
        )
        .expect("latin-1");
        t.write("src/locked/hidden.c", "int hidden(void) { return 2; }\n");
        let locked = t.0.join("src/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let readable = std::fs::read_dir(&locked).is_ok(); // root reads anything
        let target = TargetContext::load(&t.0).expect("target loads");
        let scanned = CFrontend.scan_reporting(&target);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let (facts, notes) = scanned.expect("the scan goes on");
        assert!(facts.symbols.iter().any(|s| s.name == "latin"));
        assert!(facts
            .refs
            .iter()
            .any(|r| r.from == "latin" && r.to == "a" && r.resolved));
        assert_eq!(notes.not_utf8, ["src/latin.c"]);
        if !readable {
            assert!(
                notes
                    .skipped
                    .iter()
                    .any(|(p, why)| p == &locked && why.starts_with("cannot be read")),
                "{notes:?}"
            );
            assert!(!facts.files.iter().any(|f| f.path.contains("hidden")));
        }
    }

    /// With `source_dir = "."` the ledger lies inside the scanned folder: it
    /// is pruned, so the ledger's model-written `driver.c` is never scanned.
    #[test]
    fn the_ledger_is_pruned_when_source_dir_is_the_root() {
        let root = tmp_root("dot");
        let _guard = Lzg(root.clone());
        put(&root, "a.c", "int a(void) { return 0; }\n");
        put(
            &root,
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \".\"\n",
        );
        put(
            &root,
            "migration/driver.c",
            "int model_written(void) { return 1; }\n",
        );
        put(
            &root,
            "migration/tools/t-x/x.c",
            "int other_ledger(void) { return 1; }\n",
        );
        let ctx = TargetContext::folder_form(
            root.clone(),
            harness_core::config::TargetConfig::load(&root).expect("config"),
        );
        let facts = CFrontend.scan(&ctx).expect("scan");
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.c"]);
    }

    /// docs/PROJECT-MAP-DESIGN.md §3.7 "The program digest and staleness":
    /// stale only where a scan would record otherwise, so a scan clears it.
    #[test]
    fn a_file_list_program_is_stale_where_a_scan_would_record_otherwise() {
        use harness_core::features::{program_digest_now, unrecorded_program_files, STALE_PROGRAM};
        let (_t, ctx) = lzg("stale");
        let facts = CFrontend.scan(&ctx).expect("scan");
        let first = program_digest_now(&ctx, &facts);
        assert!(first.starts_with("blake3:"), "{first}");
        assert_eq!(
            harness_core::features::program_paths(&ctx, &facts),
            [
                "src/include/lzg.h",
                "src/lib/checksum.c",
                "src/lib/encode.c",
                "src/lib/internal.h",
                "src/lib/version.c",
                "src/tools/lzg.c",
            ]
        );

        // Unrelated files: a README, a header beside the tool (its include
        // is angle-bracketed: its own folder is never searched), a header in
        // the include folder that no include names.
        put(&ctx.root, "README.md", "lzg\n");
        put(&ctx.root, "src/tools/notes.h", "int notes;\n");
        put(&ctx.root, "src/include/extra.h", "int extra;\n");
        assert_eq!(
            program_digest_now(&ctx, &facts),
            first,
            "no sign of stale facts"
        );

        // An edited reached header stales; a scan clears it.
        put(&ctx.root, "src/include/lzg.h", "int lzg_version(void);\n");
        assert_eq!(program_digest_now(&ctx, &facts), STALE_PROGRAM);
        let facts = CFrontend.scan(&ctx).expect("rescan");
        let second = program_digest_now(&ctx, &facts);
        assert!(second.starts_with("blake3:") && second != first, "{second}");

        // A new header beside a listed file, under a name it includes in
        // quotes, is found first: stale until a scan records it.
        put(&ctx.root, "src/lib/lzg.h", "int lzg_version(void);\n");
        assert_eq!(program_digest_now(&ctx, &facts), STALE_PROGRAM);
        // The new header is unrecorded, and version.c now reaches it.
        assert_eq!(
            unrecorded_program_files(&ctx, &facts),
            ["src/lib/lzg.h", "src/lib/version.c"]
        );
        let facts = CFrontend.scan(&ctx).expect("rescan");
        assert!(program_digest_now(&ctx, &facts).starts_with("blake3:"));
        let version = facts
            .files
            .iter()
            .find(|f| f.path == "src/lib/version.c")
            .expect("version.c");
        assert_eq!(version.includes, ["src/lib/lzg.h"]);

        // A listed file that vanished stales.
        std::fs::remove_file(ctx.root.join("src/lib/checksum.c")).expect("rm");
        assert_eq!(program_digest_now(&ctx, &facts), STALE_PROGRAM);
    }

    /// A mapped tool `t-x` over `files`, its `harness.toml` holding
    /// `files_toml` and `flags` (the configuration's), loaded as the CLI
    /// loads it.
    fn tool(
        tag: &str,
        files: &[(&str, &str)],
        files_toml: &str,
        flags: &str,
    ) -> (Lzg, TargetContext) {
        let root = tmp_root(tag);
        for (rel, text) in files {
            put(&root, rel, text);
        }
        let ctx = retool(&root, files_toml, flags);
        (Lzg(root), ctx)
    }

    /// `root`'s tool `t-x` rewritten with `files_toml` and `flags`, loaded.
    fn retool(root: &Path, files_toml: &str, flags: &str) -> TargetContext {
        let ledger = harness_core::config::tool_dir(root, "t-x");
        put(
            &ledger,
            "harness.toml",
            &format!(
                "schema_version = 2\n[target]\nname = \"x\"\nfiles = [{files_toml}]\n\
                 configuration = {{ name = \"make\", from = \"stated\", flags = [{flags}] }}\n"
            ),
        );
        let config =
            harness_core::config::TargetConfig::load_file(&ledger.join("harness.toml"), root)
                .expect("the file list loads");
        TargetContext {
            root: root.to_path_buf(),
            ledger,
            tool: Some("t-x".into()),
            config,
        }
    }

    fn includes_in(facts: &Facts, path: &str) -> Vec<String> {
        facts
            .files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.includes.clone())
            .unwrap_or_else(|| panic!("{path} not recorded: {:?}", facts.files))
    }

    fn fresh(ctx: &TargetContext, facts: &Facts) -> bool {
        harness_core::features::program_digest_now(ctx, facts).starts_with("blake3:")
            && harness_core::features::unrecorded_program_files(ctx, facts).is_empty()
    }

    /// Experiment e1 of the readers review (finding 1, the triage's
    /// decision 1): the configuration's `-I` and `-include` reach the scan
    /// in the compiler's order — before the file's own folders — so the
    /// facts name the headers the compile reads, the forced include and
    /// what it includes are each listed file's first includes, and an edit
    /// to any of them moves the program.
    #[test]
    fn the_configurations_path_flags_reach_the_scan_in_the_compilers_order() {
        let (_t, ctx) = tool(
            "e1",
            &[
                (
                    "src/main.c",
                    "#include \"x.h\"\n#include <y.h>\nint main(void) { return X + Y; }\n",
                ),
                ("a/x.h", "#define X 1\n"),
                ("a/y.h", "#define Y 1\n"),
                ("a/forced.h", "#include \"fh.h\"\n"),
                ("a/fh.h", "#define LZG_FAST 1\n"),
                ("b/x.h", "#define X 2\n"),
                ("b/y.h", "#define Y 2\n"),
            ],
            "{ path = \"src/main.c\", include_dirs = [\"b\"] }",
            "\"-Ia\", \"-includea/forced.h\"",
        );
        let (facts, notes) = CFrontend.scan_reporting(&ctx).expect("scan");
        assert_eq!(notes, ScanNotes::default());
        assert_eq!(
            includes_in(&facts, "src/main.c"),
            ["a/forced.h", "a/x.h", "a/y.h"]
        );
        assert_eq!(includes_in(&facts, "a/forced.h"), ["a/fh.h"]);
        assert!(!facts.files.iter().any(|f| f.path.starts_with("b/")));
        let first = harness_core::features::program_digest_now(&ctx, &facts);
        assert!(fresh(&ctx, &facts), "{first}");
        for edited in ["a/forced.h", "a/fh.h", "a/x.h", "a/y.h"] {
            let before = std::fs::read_to_string(ctx.root.join(edited)).expect("read");
            put(&ctx.root, edited, "#define LZG_FAST 0\n");
            assert_eq!(
                harness_core::features::program_digest_now(&ctx, &facts),
                harness_core::features::STALE_PROGRAM,
                "{edited}"
            );
            put(&ctx.root, edited, &before);
        }
        // The planner's closure, from the facts, holds what the compile reads.
        assert_eq!(
            facts.include_closure(&["src/main.c".to_string()]),
            ["a/fh.h", "a/forced.h", "a/x.h", "a/y.h", "src/main.c"]
        );

        // The search order itself, every kind at once: quoted is own
        // folder, -iquote, -I, the file's folders, -isystem; angle the same
        // without the own folder and -iquote.
        let ctx = retool(
            &ctx.root,
            "{ path = \"src/main.c\", include_dirs = [\"b\"] }",
            "\"-iquoteq\", \"-Ia\", \"-isystems\"",
        );
        let resolver = sources::Resolver::of(&ctx)
            .expect("resolver")
            .expect("a file list");
        assert_eq!(
            resolver.search_order("src/main.c", "src/main.c", true),
            ["src", "q", "a", "b", "s"]
        );
        assert_eq!(
            resolver.search_order("src/main.c", "a/forced.h", false),
            ["a", "b", "s"]
        );
        // Checked against the compiler where one is at hand: the files
        // `cc -M` reads, in the oracle's argument order, are the
        // resolver's.
        for (rel, text) in [
            ("q/x.h", "#define X 3\n"),
            ("src/x.h", "#define X 4\n"),
            ("s/z.h", "#define Z 1\n"),
            ("q/w.h", "#define W 0\n"),
            ("s/w.h", "#define W 1\n"),
        ] {
            put(&ctx.root, rel, text);
        }
        put(
            &ctx.root,
            "src/main.c",
            "#include \"x.h\"\n#include <y.h>\n#include <z.h>\n#include <w.h>\n\
             int main(void) { return X + Y + Z + W; }\n",
        );
        let found: Vec<String> = [
            ("x.h", true),
            ("y.h", false),
            ("z.h", false),
            ("w.h", false),
        ]
        .iter()
        .filter_map(|(name, quoted)| resolver.resolve("src/main.c", "src/main.c", name, *quoted))
        .collect();
        assert_eq!(found, ["src/x.h", "a/y.h", "s/z.h", "s/w.h"]);
        let cc = std::process::Command::new("cc")
            .current_dir(&ctx.root)
            .args(["-M", "-iquoteq", "-Ia", "-isystems", "-Ib", "src/main.c"])
            .output();
        if let Some(out) = cc.ok().filter(|o| o.status.success()) {
            let deps = String::from_utf8_lossy(&out.stdout).replace("\\\n", " ");
            for want in &found {
                assert!(deps.contains(want.as_str()), "{want} in {deps}");
            }
            for not in ["q/x.h", "a/x.h", "b/y.h", "q/w.h"] {
                assert!(!deps.contains(not), "{not} in {deps}");
            }
        }
    }

    /// The readers review, finding 2 (experiment e3): every case where the
    /// former parser and the staleness reader parted — includes inside a
    /// struct, a union, an enum or an initializer (the X-macro pattern), a
    /// continued `#inc\` line, `#/**/include`, trailing junk, `<sys//types.h>`
    /// — is recorded by a scan, and the facts are fresh right after it: one
    /// reader for both. The project map's [`file_facts`] reads the same.
    #[test]
    fn the_scanner_and_staleness_read_the_same_includes() {
        for (case, body, name) in [
            (
                "struct",
                "struct s {\n#include \"inc.h\"\n};\n",
                "inc/inc.h",
            ),
            ("union", "union u {\n#include \"inc.h\"\n};\n", "inc/inc.h"),
            ("enum", "enum op {\n#include \"inc.h\"\n};\n", "inc/inc.h"),
            (
                "initializer",
                "static const char *names[] = {\n#include \"inc.h\"\n};\n",
                "inc/inc.h",
            ),
            ("continued", "#inc\\\nlude \"inc.h\"\n", "inc/inc.h"),
            ("comment", "#/**/include \"inc.h\"\n", "inc/inc.h"),
            ("junk", "#include \"inc.h\" junk\n", "inc/inc.h"),
            ("slashes", "#include <sys//types.h>\n", "inc/sys/types.h"),
        ] {
            let source = format!("{body}int main(void) {{ return 0; }}\n");
            let (_t, ctx) = tool(
                &format!("e3-{case}"),
                &[
                    ("src/main.c", source.as_str()),
                    ("inc/inc.h", "X(a)\n"),
                    ("inc/sys/types.h", "typedef int t;\n"),
                ],
                "{ path = \"src/main.c\", include_dirs = [\"inc\"] }",
                "",
            );
            let facts = CFrontend.scan(&ctx).expect("scan");
            assert_eq!(includes_in(&facts, "src/main.c"), [name], "{case}");
            assert!(fresh(&ctx, &facts), "{case}: stale right after a scan");
            let read: Vec<(String, bool)> = file_facts(source.as_bytes())
                .expect("facts")
                .includes
                .into_iter()
                .map(|i| (i.name, i.quoted))
                .collect();
            assert_eq!(read, sources::include_names(source.as_bytes()), "{case}");
        }
    }

    /// The readers review, finding 3 (experiment e6): staleness compares the
    /// include a file resolves to now with the one its record names, both
    /// ways — a changed folder list stales, a recorded include that no
    /// longer resolves stales, and a scan clears each.
    #[test]
    fn staleness_compares_where_each_include_lands_with_the_record() {
        let files = [
            (
                "src/main.c",
                "#include \"x.h\"\nint main(void) { return X; }\n",
            ),
            (
                "src/other.c",
                "#include \"x.h\"\nint other(void) { return X; }\n",
            ),
            ("a/x.h", "#define X 1\n"),
            ("b/x.h", "#define X 2\n"),
        ];
        let listed = |main: &str| {
            format!(
                "{{ path = \"src/main.c\", include_dirs = [{main}] }}, \
                 {{ path = \"src/other.c\", include_dirs = [\"a\"] }}"
            )
        };
        let (_t, ctx) = tool("e6", &files, &listed("\"b\""), "");
        let facts = CFrontend.scan(&ctx).expect("scan");
        assert_eq!(includes_in(&facts, "src/main.c"), ["b/x.h"]);
        assert!(fresh(&ctx, &facts));

        // main.c's folders now say `a`: its record names b/x.h.
        let ctx = retool(&ctx.root, &listed("\"a\""), "");
        assert_eq!(
            harness_core::features::unrecorded_program_files(&ctx, &facts),
            ["src/main.c"]
        );
        let facts = CFrontend.scan(&ctx).expect("rescan");
        assert_eq!(includes_in(&facts, "src/main.c"), ["a/x.h"]);
        assert!(fresh(&ctx, &facts));

        // No folder at all: the recorded include no longer resolves.
        let ctx = retool(&ctx.root, &listed(""), "");
        assert_eq!(
            harness_core::features::unrecorded_program_files(&ctx, &facts),
            ["src/main.c"]
        );
        let facts = CFrontend.scan(&ctx).expect("rescan");
        assert!(includes_in(&facts, "src/main.c").is_empty());
        assert!(fresh(&ctx, &facts));
    }

    /// The readers review, finding 6: a header reached from two listed
    /// files whose folders resolve one of its includes to two files is an
    /// ambiguous include — noted, recorded for neither — never the union;
    /// both files it lands on are still recorded, and the facts are fresh.
    #[test]
    fn a_header_resolving_differently_under_two_files_is_an_ambiguous_include() {
        let (_t, ctx) = tool(
            "ambiguous",
            &[
                (
                    "src/a.c",
                    "#include \"common.h\"\nint a(void) { return CFG; }\n",
                ),
                (
                    "src/b.c",
                    "#include \"common.h\"\nint b(void) { return CFG; }\n",
                ),
                ("inc/common.h", "#include \"cfg.h\"\n#include \"same.h\"\n"),
                ("inc/same.h", "#define SAME 1\n"),
                ("d1/cfg.h", "#define CFG 1\n"),
                ("d2/cfg.h", "#define CFG 2\n"),
            ],
            "{ path = \"src/a.c\", include_dirs = [\"inc\", \"d1\"] }, \
             { path = \"src/b.c\", include_dirs = [\"inc\", \"d2\"] }",
            "",
        );
        let (facts, notes) = CFrontend.scan_reporting(&ctx).expect("scan");
        assert_eq!(includes_in(&facts, "inc/common.h"), ["inc/same.h"]);
        assert!(facts.files.iter().any(|f| f.path == "d1/cfg.h"));
        assert!(facts.files.iter().any(|f| f.path == "d2/cfg.h"));
        assert_eq!(notes.ambiguous.len(), 1, "{notes:?}");
        let line = &notes.ambiguous_lines()[0];
        assert!(
            line.contains(
                "inc/common.h's include \"cfg.h\" reaches d1/cfg.h when compiled for \
                 src/a.c but d2/cfg.h when compiled for src/b.c"
            ),
            "{line}"
        );
        assert!(fresh(&ctx, &facts));
    }

    /// The readers review, finding 4 (experiment e5): a file the scan cannot
    /// read is recorded as a fact (`unreadable`), so the facts are fresh
    /// right after the scan instead of stale forever — in the folder form
    /// and the file list alike.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_a_fact_and_never_a_permanent_staleness() {
        use std::os::unix::fs::PermissionsExt;
        let lock = |p: &Path, mode| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).expect("chmod")
        };
        // The folder form.
        let t = TempTarget::new("e5", "[]");
        t.write("src/a.c", "int a(void) { return 0; }\n");
        t.write("src/b.c", "int b(void) { return 1; }\n");
        let b = t.0.join("src/b.c");
        lock(&b, 0o000);
        if std::fs::read(&b).is_ok() {
            return; // run as root: nothing is unreadable
        }
        let target = TargetContext::load(&t.0).expect("target loads");
        let scanned = CFrontend.scan_reporting(&target);
        let digest = scanned
            .as_ref()
            .map(|(facts, _)| harness_core::features::program_digest_now(&target, facts))
            .ok();
        lock(&b, 0o644);
        let (facts, notes) = scanned.expect("the scan goes on");
        let record = facts
            .files
            .iter()
            .find(|f| f.path == "src/b.c")
            .expect("recorded");
        assert_eq!(record.hash, sources::UNREADABLE_HASH);
        assert_eq!(notes.unreadable, ["src/b.c"]);
        assert!(digest.expect("digest").starts_with("blake3:"));

        // The file list: an unreadable header.
        let (_t, ctx) = tool(
            "e5-list",
            &[
                (
                    "src/main.c",
                    "#include \"h.h\"\nint main(void) { return 0; }\n",
                ),
                ("src/h.h", "#define H 1\n"),
            ],
            "{ path = \"src/main.c\" }",
            "",
        );
        let h = ctx.root.join("src/h.h");
        lock(&h, 0o000);
        let facts = CFrontend.scan(&ctx).expect("scan");
        let was_fresh = fresh(&ctx, &facts);
        lock(&h, 0o644);
        assert_eq!(includes_in(&facts, "src/main.c"), ["src/h.h"]);
        assert!(was_fresh, "stale right after a scan");
        // Readable again: the record's hash no longer matches.
        assert_eq!(
            harness_core::features::program_digest_now(&ctx, &facts),
            harness_core::features::STALE_PROGRAM
        );
    }

    /// The readers review, finding 11: with `source_dir` itself a link, a
    /// top-level `.c` that links to another is named as the scan names it —
    /// under `source_dir` as written — so the facts are fresh after a scan.
    #[cfg(unix)]
    #[test]
    fn a_linked_source_dir_is_fresh_after_a_scan() {
        let root = tmp_root("linked-src");
        let _guard = Lzg(root.clone());
        put(&root, "real/a.c", "int a(void) { return 0; }\n");
        std::os::unix::fs::symlink(root.join("real/a.c"), root.join("real/b.c")).expect("link");
        std::os::unix::fs::symlink(root.join("real"), root.join("src")).expect("link");
        put(
            &root,
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n",
        );
        let ctx = TargetContext::folder_form(
            root.clone(),
            harness_core::config::TargetConfig::load(&root).expect("config"),
        );
        let facts = CFrontend.scan(&ctx).expect("scan");
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["src/a.c"]);
        assert!(harness_core::features::program_digest_now(&ctx, &facts).starts_with("blake3:"));
    }

    /// The readers review, finding 7: an include that lands under
    /// `migration/` or outside the project is a scan note naming it — never
    /// read, never recorded.
    #[test]
    fn an_include_landing_outside_the_project_is_a_note() {
        let outside = tmp_root("outside-inc");
        let _outside_guard = Lzg(outside.clone());
        put(&outside, "secret.h", "int secret;\n");
        let main = format!(
            "#include \"../migration/tools/t-x/model.h\"\n#include \"{}\"\n\
             int main(void) {{ return 0; }}\n",
            outside.join("secret.h").display()
        );
        let (_t, ctx) = tool(
            "outside",
            &[("src/main.c", main.as_str())],
            "{ path = \"src/main.c\" }",
            "",
        );
        put(&ctx.ledger, "model.h", "int model_written;\n");
        let (facts, notes) = CFrontend.scan_reporting(&ctx).expect("scan");
        assert!(includes_in(&facts, "src/main.c").is_empty());
        assert_eq!(facts.files.len(), 1);
        for (path, name) in [
            (
                ctx.ledger
                    .join("model.h")
                    .canonicalize()
                    .expect("canonical"),
                "../migration/tools/t-x/model.h".to_string(),
            ),
            (
                outside.join("secret.h"),
                outside.join("secret.h").display().to_string(),
            ),
        ] {
            assert!(
                notes.skipped.iter().any(|(p, why)| p == &path
                    && why.contains(&format!("src/main.c includes it as `{name}`"))
                    && why.contains("outside the project or under migration/")),
                "{path:?}: {notes:?}"
            );
        }
        assert!(fresh(&ctx, &facts));
    }
}
