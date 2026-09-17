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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::facts::{Facts, FileRecord, RefRecord, SymbolRecord};
use harness_core::hash::file_hash;
use harness_core::traits::LanguageFrontend;
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
        let src_dir = target.root.join(&target.config.target.source_dir);
        let mut abs_files: Vec<PathBuf> = Vec::new();
        collect_source_files(&src_dir, &mut abs_files)?;

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
                .filter_map(|raw| resolve_include(rel, raw))
                .filter(|resolved| scanned.contains(resolved))
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

        Ok(Facts {
            frontend: self.name().to_string(),
            files: file_records,
            symbols,
            refs,
        })
    }
}

/// Recursively collect `*.c` and `*.h` files under `dir`.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            collect_source_files(&path, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("c") | Some("h")
        ) {
            out.push(path);
        }
    }
    Ok(())
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

/// Lexically resolve a quoted include `raw` against the directory of the
/// repo-relative `including` file. Returns `None` when the path escapes the
/// target root.
fn resolve_include(including: &str, raw: &str) -> Option<String> {
    let mut parts: Vec<&str> = including.split('/').collect();
    parts.pop(); // drop the file name, keeping its directory
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
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
}
