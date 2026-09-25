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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod interface;
mod lint;
mod mutate;

pub use interface::{
    is_c_identifier, parse_interface, InterfaceParam, InterfaceSig, MAX_INTERFACE_LINE,
};
pub use lint::{lint_driver, DRIVER_SYSTEM_INCLUDES};
pub use mutate::mutants;

use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::facts::{Facts, FileRecord, RefRecord, SymbolRecord};
use harness_core::hash::file_hash;
use harness_core::traits::LanguageFrontend;
use harness_core::walk;
use std::collections::BTreeSet;
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

impl CFrontend {
    /// [`LanguageFrontend::scan`], also returning the matching entries the
    /// walk left out because they are not regular files (a FIFO named `a.c`
    /// would block the scan forever): the caller reports them. A walk error
    /// stays fatal.
    pub fn scan_reporting(&self, target: &TargetContext) -> Result<(Facts, Vec<PathBuf>), Error> {
        let src_dir = target.root.join(&target.config.target.source_dir);
        let walked = walk::confined(&src_dir, &C_EXTENSIONS, walk::Limits::default());
        if let Some((path, why)) = walked.errors.first() {
            return Err(Error::io(path, std::io::Error::other(why.clone())));
        }
        let skipped: Vec<PathBuf> = walked.skipped.into_iter().map(|(p, _)| p).collect();
        let abs_files = walked.files;
        let source_rel = lexical_segments(&target.config.target.source_dir).unwrap_or_default();
        let include_dirs: Vec<Vec<String>> = target
            .config
            .target
            .include_dirs
            .iter()
            .filter_map(|d| lexical_segments(d))
            .collect();

        // (repo-relative path, absolute path), sorted by relative path.
        let mut files: Vec<(String, PathBuf)> = Vec::with_capacity(abs_files.len());
        for abs in abs_files {
            files.push((repo_relative(&target.root, &abs)?, abs));
        }
        files.sort();
        let scanned: BTreeSet<String> = files.iter().map(|(rel, _)| rel.clone()).collect();

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;

        let mut defs: Vec<FnDef> = Vec::new();
        let mut file_records: Vec<FileRecord> = Vec::with_capacity(files.len());
        for (rel, abs) in &files {
            let source = std::fs::read_to_string(abs).map_err(|e| Error::io(abs, e))?;
            let tree = parser
                .parse(&source, None)
                .ok_or_else(|| Error::parse(abs, "tree-sitter parse failed"))?;
            let root = tree.root_node();
            let src = source.as_bytes();

            let mut raw_includes: BTreeSet<String> = BTreeSet::new();
            collect_includes(root, src, &mut raw_includes);
            let includes: Vec<String> = raw_includes
                .iter()
                .filter_map(|raw| {
                    resolve_quoted_include(rel, raw, &include_dirs, &source_rel, &scanned)
                })
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect();

            file_records.push(FileRecord {
                path: rel.clone(),
                hash: file_hash(abs)?,
                includes,
            });
            collect_functions(root, src, rel, &mut defs);
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
            skipped,
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

/// Recursively collect the raw paths of quoted `#include "x"` directives.
fn collect_includes(node: tree_sitter::Node, src: &[u8], out: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include" {
            if let Some(path) = child.child_by_field_name("path") {
                // Quoted includes are string_literals; `<...>` system includes
                // are system_lib_strings and deliberately skipped.
                if path.kind() == "string_literal" {
                    let raw = text(path, src).trim_matches('"').to_string();
                    if !raw.is_empty() {
                        out.insert(raw);
                    }
                }
            }
        } else {
            collect_includes(child, src, out);
        }
    }
}

/// Recursively collect function definitions with their storage class,
/// signature, span, and direct-call names.
fn collect_functions(node: tree_sitter::Node, src: &[u8], file: &str, defs: &mut Vec<FnDef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            if let Some(name) = function_name(child, src) {
                let mut c = child.walk();
                let is_static = child
                    .children(&mut c)
                    .any(|n| n.kind() == "storage_class_specifier" && text(n, src) == "static");
                drop(c);
                let mut calls = BTreeSet::new();
                collect_calls(child, src, &mut calls);
                defs.push(FnDef {
                    name,
                    file: file.to_string(),
                    is_static,
                    signature: signature_of(child, src),
                    span: (
                        (child.start_position().row + 1) as u32,
                        (child.end_position().row + 1) as u32,
                    ),
                    calls,
                });
            }
        } else {
            collect_functions(child, src, file, defs);
        }
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

/// Find the identifier of a function definition by descending through
/// (possibly pointer-wrapped) declarators to the function_declarator.
fn function_name(def: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut node = def.child_by_field_name("declarator")?;
    loop {
        match node.kind() {
            "function_declarator" => {
                let decl = node.child_by_field_name("declarator")?;
                return if decl.kind() == "identifier" {
                    Some(text(decl, src).to_string())
                } else {
                    None
                };
            }
            "pointer_declarator" | "parenthesized_declarator" => {
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

    /// The real zopfli target vendored in this repository.
    fn zopfli_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli")
    }

    #[test]
    fn scans_real_zopfli_target() {
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

    /// docs/COCKPIT-WRAPPER-DESIGN.md §2.1: on the shared walk the facts are
    /// byte-identical to the committed ones (zopfli, the read_scalefactors
    /// case), and a synthetic copy with a symlink inside source_dir scans as
    /// it did before (checked by hand against the old walk when switching).
    #[test]
    fn the_shared_walk_keeps_the_committed_facts_byte_identical() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for rel in [
            "targets/zopfli",
            "targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib",
        ] {
            let root = repo.join(rel);
            let target = TargetContext::load(&root).expect("target loads");
            let facts = CFrontend.scan(&target).expect("scan");
            let committed =
                std::fs::read(root.join("migration/facts.jsonl")).expect("committed facts");
            assert!(
                facts.to_canonical_bytes().expect("bytes") == committed,
                "{rel}: the facts differ from the committed ones"
            );
        }
    }

    /// A FIFO (or any non-regular file) with a C name is skipped and
    /// reported — never read, so it can no longer hang a scan. A link to a
    /// file inside source_dir is scanned at its own path.
    #[cfg(unix)]
    #[test]
    fn a_fifo_named_like_c_is_skipped_and_an_inside_link_is_scanned() {
        let t = TempTarget::new("fifo", "[]");
        t.write("src/a.c", "int a(void) { return 0; }\n");
        std::os::unix::fs::symlink(t.0.join("src/a.c"), t.0.join("src/alias.c")).expect("symlink");
        assert!(std::process::Command::new("mkfifo")
            .arg(t.0.join("src/pipe.c"))
            .status()
            .expect("mkfifo")
            .success());
        let target = TargetContext::load(&t.0).expect("target loads");
        let (facts, skipped) = CFrontend.scan_reporting(&target).expect("scan terminates");
        let paths: Vec<&str> = facts.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.c", "src/alias.c"]);
        assert_eq!(skipped, vec![t.0.join("src/pipe.c")]);
    }
}
