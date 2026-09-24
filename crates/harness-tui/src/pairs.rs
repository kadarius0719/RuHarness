//! Function pairs (docs/TUI-DESIGN.md §2): each plan symbol's C definition
//! beside the Rust that implements it — the exported shim and the logic
//! function the shim calls.
//!
//! - The C side is sliced from the tree by the `facts.jsonl` symbol span,
//!   ONLY when the file still has the hash the scan recorded (otherwise the
//!   span may point at other lines: the side says the facts are stale).
//! - The Rust side is found by parsing every `src/**/*.rs` of the crate with
//!   tree-sitter-rust: the shim is the function exported under the symbol's
//!   name (`#[no_mangle]`, or `#[export_name = "…"]`) in ANY file — the M0
//!   layout has an inline `mod ffi` in `lib.rs`; the logic callee is the
//!   first call in the shim's body that resolves, through the file's `use`
//!   aliases, to a non-shim function of the crate (preferring `logic.rs`).

use harness_core::facts::Facts;
use harness_core::hash;
use harness_core::plan::Unit;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Most Rust source files indexed per crate.
const MAX_RUST_FILES: usize = 256;
/// Largest file read (C or Rust).
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

/// A run of source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// Repo-relative (C) or crate-relative (Rust) path.
    pub file: String,
    /// 1-based line of `lines[0]`.
    pub first_line: usize,
    /// The raw lines (not yet display-filtered).
    pub lines: Vec<String>,
    /// The function's name as defined.
    pub name: String,
}

/// The C side of a pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CSide {
    /// The definition, sliced from a file the scan still describes.
    Source(SourceSpan),
    /// The file changed since the scan (or is gone): the span cannot be
    /// trusted — run `harness scan`.
    StaleFacts {
        /// The symbol's file.
        file: String,
    },
    /// The symbol has no record in `facts.jsonl`.
    NotInFacts,
}

/// Why the Rust side is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustNote {
    /// No crate to look in.
    NoCrate,
    /// No shim exports the symbol (or, for an internal symbol, no function
    /// of that name exists).
    NotFound,
    /// The shim exists but no call in it resolves to a crate function.
    LogicNotIdentified,
}

/// The Rust side of a pair.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RustSide {
    /// The exported shim.
    pub shim: Option<SourceSpan>,
    /// The logic function (for an internal symbol: the function of that name).
    pub logic: Option<SourceSpan>,
    /// What is missing, when something is.
    pub note: Option<RustNote>,
}

/// One plan symbol's C beside its Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionPair {
    /// The plan symbol (canonical id: `name`, or `<file>::<name>` internal).
    pub symbol: String,
    /// A public (exported) symbol.
    pub public: bool,
    /// The C side.
    pub c: CSide,
    /// The Rust side.
    pub rust: RustSide,
}

/// The pairs of `unit`, public symbols first, each group in plan order.
/// `crate_dir` is the Rust crate to look in (the unit crate or an attempt's
/// `candidate/`), `None` when there is none.
pub fn pairs(
    root: &Path,
    facts: &Facts,
    unit: &Unit,
    crate_dir: Option<&Path>,
) -> Vec<FunctionPair> {
    let index = crate_dir.map(CrateIndex::build);
    let exported: Vec<&str> = unit
        .symbols
        .iter()
        .filter(|s| !s.contains("::"))
        .map(String::as_str)
        .collect();
    let mut ordered: Vec<&String> = unit.symbols.iter().filter(|s| !s.contains("::")).collect();
    ordered.extend(unit.symbols.iter().filter(|s| s.contains("::")));
    ordered
        .into_iter()
        .map(|symbol| {
            let public = !symbol.contains("::");
            let rust = match &index {
                None => RustSide {
                    note: Some(RustNote::NoCrate),
                    ..RustSide::default()
                },
                Some(index) if public => index.public_side(symbol, &exported),
                Some(index) => index.internal_side(symbol.rsplit("::").next().unwrap_or(symbol)),
            };
            FunctionPair {
                symbol: symbol.clone(),
                public,
                c: c_side(root, facts, symbol),
                rust,
            }
        })
        .collect()
}

fn read_bounded(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_SOURCE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn c_side(root: &Path, facts: &Facts, symbol: &str) -> CSide {
    let Some(sym) = facts.symbols.iter().find(|s| s.name == symbol) else {
        return CSide::NotInFacts;
    };
    let stale = || CSide::StaleFacts {
        file: sym.file.clone(),
    };
    let Some(record) = facts.files.iter().find(|f| f.path == sym.file) else {
        return stale();
    };
    let path = root.join(&sym.file);
    match hash::file_hash(&path) {
        Ok(now) if now == record.hash => {}
        _ => return stale(),
    }
    let Some(text) = read_bounded(&path) else {
        return stale();
    };
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = (sym.span.0 as usize, sym.span.1 as usize);
    let start = start.max(1);
    let end = end.min(lines.len());
    if start > end {
        return stale();
    }
    let name = symbol.rsplit("::").next().unwrap_or(symbol).to_string();
    CSide::Source(SourceSpan {
        file: sym.file.clone(),
        first_line: start,
        lines: lines[start - 1..end]
            .iter()
            .map(|l| l.to_string())
            .collect(),
        name,
    })
}

/// One function definition of the crate.
#[derive(Debug, Clone)]
struct RustFn {
    name: String,
    /// The exported symbol, for a shim.
    export: Option<String>,
    file: String,
    start_row: usize,
    end_row: usize,
    /// Callee names in the body, in order (last path segment).
    calls: Vec<String>,
}

/// Every function of a crate, with each file's `use … as …` aliases.
struct CrateIndex {
    fns: Vec<RustFn>,
    /// (file, alias) → original name.
    aliases: BTreeMap<(String, String), String>,
    lines: BTreeMap<String, Vec<String>>,
}

impl CrateIndex {
    fn build(crate_dir: &Path) -> CrateIndex {
        let mut index = CrateIndex {
            fns: Vec::new(),
            aliases: BTreeMap::new(),
            lines: BTreeMap::new(),
        };
        let mut files = Vec::new();
        collect_rs(&crate_dir.join("src"), "src", &mut files);
        files.sort();
        files.truncate(MAX_RUST_FILES);
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return index;
        }
        for (rel, path) in files {
            let Some(text) = read_bounded(&path) else {
                continue;
            };
            let Some(tree) = parser.parse(&text, None) else {
                continue;
            };
            index.scan(&rel, &text, tree.root_node());
            index
                .lines
                .insert(rel, text.lines().map(str::to_string).collect());
        }
        index
    }

    fn scan(&mut self, file: &str, text: &str, root: tree_sitter::Node) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "function_item" => {
                    if let Some(f) = function(file, text, node) {
                        self.fns.push(f);
                    }
                }
                "use_as_clause" => {
                    if let (Some(path), Some(alias)) = (
                        node.child_by_field_name("path"),
                        node.child_by_field_name("alias"),
                    ) {
                        let original = last_segment(text, path);
                        let alias = node_text(text, alias).to_string();
                        self.aliases.insert((file.to_string(), alias), original);
                    }
                }
                _ => {}
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
    }

    /// A non-shim function named `name`, preferring `src/logic.rs`.
    fn logic_fn(&self, name: &str) -> Option<&RustFn> {
        let mut found: Vec<&RustFn> = self
            .fns
            .iter()
            .filter(|f| f.export.is_none() && f.name == name)
            .collect();
        found.sort_by_key(|f| (f.file != "src/logic.rs", f.file.clone(), f.start_row));
        found.into_iter().next()
    }

    fn span(&self, f: &RustFn) -> SourceSpan {
        let lines = self.lines.get(&f.file).map(Vec::as_slice).unwrap_or(&[]);
        let end = (f.end_row + 1).min(lines.len());
        let start = f.start_row.min(end);
        SourceSpan {
            file: f.file.clone(),
            first_line: start + 1,
            lines: lines[start..end].to_vec(),
            name: f.name.clone(),
        }
    }

    fn public_side(&self, symbol: &str, exported: &[&str]) -> RustSide {
        let Some(shim) = self
            .fns
            .iter()
            .find(|f| f.export.as_deref() == Some(symbol))
        else {
            return RustSide {
                note: Some(RustNote::NotFound),
                ..RustSide::default()
            };
        };
        let logic = shim
            .calls
            .iter()
            .filter_map(|call| {
                let original = self
                    .aliases
                    .get(&(shim.file.clone(), call.clone()))
                    .unwrap_or(call);
                // A call to another exported symbol's shim is not the logic.
                if exported.contains(&original.as_str()) && self.logic_fn(original).is_none() {
                    return None;
                }
                self.logic_fn(original)
            })
            .next()
            .or_else(|| self.logic_fn(symbol));
        RustSide {
            shim: Some(self.span(shim)),
            logic: logic.map(|f| self.span(f)),
            note: logic.is_none().then_some(RustNote::LogicNotIdentified),
        }
    }

    fn internal_side(&self, name: &str) -> RustSide {
        match self.logic_fn(name) {
            Some(f) => RustSide {
                shim: None,
                logic: Some(self.span(f)),
                note: None,
            },
            None => RustSide {
                note: Some(RustNote::NotFound),
                ..RustSide::default()
            },
        }
    }
}

fn collect_rs(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let child_rel = format!("{rel}/{name}");
        if meta.file_type().is_dir() {
            collect_rs(&entry.path(), &child_rel, out);
        } else if meta.file_type().is_file() && name.ends_with(".rs") {
            out.push((child_rel, entry.path()));
        }
    }
}

fn node_text<'t>(text: &'t str, node: tree_sitter::Node) -> &'t str {
    text.get(node.byte_range()).unwrap_or("")
}

/// The last segment of a path-like node (`a::b::c` → `c`).
fn last_segment(text: &str, node: tree_sitter::Node) -> String {
    match node.kind() {
        "scoped_identifier" => node
            .child_by_field_name("name")
            .map(|n| node_text(text, n).to_string())
            .unwrap_or_default(),
        _ => node_text(text, node).to_string(),
    }
}

/// A `function_item` as a [`RustFn`]: its export (from the attributes
/// before it) and the callees in its body.
fn function(file: &str, text: &str, node: tree_sitter::Node) -> Option<RustFn> {
    let name = node_text(text, node.child_by_field_name("name")?).to_string();
    let mut export = None;
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "attribute_item" => {
                let attr = node_text(text, p);
                if attr.contains("no_mangle") {
                    export = Some(name.clone());
                } else if let Some(rest) = attr.split("export_name").nth(1) {
                    if let Some(value) = rest.split('"').nth(1) {
                        export = Some(value.to_string());
                    }
                }
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        prev = p.prev_named_sibling();
    }
    let mut calls = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            if n.kind() == "call_expression" {
                if let Some(callee) = n.child_by_field_name("function") {
                    let callee = if callee.kind() == "generic_function" {
                        callee.child_by_field_name("function").unwrap_or(callee)
                    } else {
                        callee
                    };
                    if matches!(callee.kind(), "identifier" | "scoped_identifier") {
                        calls.push(last_segment(text, callee));
                    }
                }
            }
            let mut cursor = n.walk();
            let children: Vec<_> = n.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
    }
    Some(RustFn {
        name,
        export,
        file: file.to_string(),
        start_row: node.start_position().row,
        end_row: node.end_position().row,
        calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_with(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("harness-tui-pairs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn side(dir: &Path, symbol: &str, exported: &[&str]) -> RustSide {
        CrateIndex::build(dir).public_side(symbol, exported)
    }

    #[test]
    fn a_logic_path_call_resolves() {
        let dir = crate_with(
            "path",
            &[
                ("src/logic.rs", "pub fn add_impl(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n"),
                (
                    "src/ffi.rs",
                    "#[no_mangle]\npub unsafe extern \"C\" fn add(a: i32, b: i32) -> i32 {\n    crate::logic::add_impl(a, b)\n}\n",
                ),
            ],
        );
        let s = side(&dir, "add", &["add"]);
        assert_eq!(s.shim.as_ref().unwrap().first_line, 2);
        assert_eq!(s.logic.as_ref().unwrap().name, "add_impl");
        assert_eq!(s.logic.as_ref().unwrap().file, "src/logic.rs");
        assert_eq!(s.note, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bare_import_an_alias_and_a_call_to_another_shim() {
        // 024_struct_and_static's shape: `run` calls a renamed, bare-imported
        // logic fn; `driver` only calls the `run` shim.
        let dir = crate_with(
            "bare",
            &[
                ("src/logic.rs", "pub fn run_logic(x: i32) {}\npub fn dequantize(x: i32) -> i32 { x }\n"),
                (
                    "src/ffi.rs",
                    "use crate::logic::{run_logic, dequantize as dq_logic};\n\
                     #[no_mangle]\npub unsafe extern \"C\" fn run(x: i32) {\n    run_logic(x);\n}\n\
                     #[no_mangle]\npub unsafe extern \"C\" fn driver(x: i32) {\n    run(x);\n    run(x);\n}\n\
                     #[no_mangle]\npub extern \"C\" fn dequantize(x: i32) -> i32 {\n    dq_logic(x)\n}\n",
                ),
            ],
        );
        let exported = ["run", "driver", "dequantize"];
        assert_eq!(
            side(&dir, "run", &exported).logic.unwrap().name,
            "run_logic"
        );
        let driver = side(&dir, "driver", &exported);
        assert!(driver.shim.is_some());
        assert_eq!(driver.logic, None);
        assert_eq!(driver.note, Some(RustNote::LogicNotIdentified));
        assert_eq!(
            side(&dir, "dequantize", &exported).logic.unwrap().name,
            "dequantize"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_inline_ffi_module_and_export_name() {
        // The M0 layout: the shim in `mod ffi` inside lib.rs, calling super::.
        let dir = crate_with(
            "inline",
            &[(
                "src/lib.rs",
                "fn length_limited(a: u32) -> u32 { a }\n\
                 mod ffi {\n    #[no_mangle]\n    pub unsafe extern \"C\" fn ZopfliLen(a: u32) -> u32 {\n        super::length_limited(a)\n    }\n}\n\
                 #[export_name = \"renamed_sym\"]\npub extern \"C\" fn inner(a: u32) -> u32 { length_limited(a) }\n",
            )],
        );
        let s = side(&dir, "ZopfliLen", &["ZopfliLen", "renamed_sym"]);
        assert_eq!(s.shim.as_ref().unwrap().file, "src/lib.rs");
        assert_eq!(s.logic.as_ref().unwrap().name, "length_limited");
        let r = side(&dir, "renamed_sym", &["ZopfliLen", "renamed_sym"]);
        assert_eq!(r.shim.unwrap().name, "inner");
        assert_eq!(side(&dir, "missing", &[]).note, Some(RustNote::NotFound));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
