//! Deterministic call-graph scan of the vendored zopfli sources (M0 embryo
//! of the future `harness-scan` frontend). No LLM involvement (§2.3).
//!
//! Known, deliberate gap (recorded in DECISIONS.md): calls made through
//! function pointers (e.g. qsort comparators) are invisible here — that is
//! detector material for M2, not a scan feature.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug)]
struct FnDef {
    file: String,
    is_static: bool,
    /// Names appearing as direct callees inside this function's body.
    calls: BTreeSet<String>,
}

pub fn run(root: &Path) -> Result<(), String> {
    let src_dir = root.join("targets/zopfli/src/zopfli");
    let mut files: Vec<_> = std::fs::read_dir(&src_dir)
        .map_err(|e| format!("reading {}: {e}", src_dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("c") | Some("h")
            )
        })
        .collect();
    files.sort();

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| format!("tree-sitter language version mismatch: {e}"))?;

    let mut defs: BTreeMap<String, FnDef> = BTreeMap::new();
    for path in &files {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| format!("parse failed for {}", path.display()))?;
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        collect_functions(tree.root_node(), source.as_bytes(), &file_name, &mut defs);
    }

    // A unit for a public (non-static) function F is the transitive closure
    // of project-defined callees. F is a *leaf unit* if that closure contains
    // no other public project function — only same-unit statics and externals.
    let mut leaves: Vec<(String, usize, BTreeSet<String>)> = Vec::new();
    for (name, _) in defs.iter().filter(|(_, d)| !d.is_static) {
        let mut closure: BTreeSet<String> = BTreeSet::new();
        let mut externals: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![name.clone()];
        let mut blocked = false;
        while let Some(f) = stack.pop() {
            if !closure.insert(f.clone()) {
                continue;
            }
            if let Some(d) = defs.get(&f) {
                if f != *name && !d.is_static {
                    blocked = true; // depends on another public project symbol
                }
                for callee in &d.calls {
                    if defs.contains_key(callee) {
                        stack.push(callee.clone());
                    } else {
                        externals.insert(callee.clone());
                    }
                }
            }
        }
        if !blocked {
            leaves.push((name.clone(), closure.len(), externals));
        }
    }
    leaves.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));

    println!(
        "scan: {} functions across {} files; {} public leaf unit(s):",
        defs.len(),
        files.len(),
        leaves.len()
    );
    for (name, size, externals) in &leaves {
        let file = &defs[name].file;
        let ext: Vec<_> = externals.iter().cloned().collect();
        println!(
            "  {name}  [{file}]  closure={size}  externals={}",
            ext.join(",")
        );
    }
    println!("scan: M0 unit selection: ZopfliLengthLimitedCodeLengths (u001-katajainen)");
    Ok(())
}

/// Recursively collect function definitions, their storage class, and the
/// direct-call names inside their bodies.
fn collect_functions(
    node: tree_sitter::Node,
    src: &[u8],
    file: &str,
    defs: &mut BTreeMap<String, FnDef>,
) {
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
                defs.insert(
                    name,
                    FnDef {
                        file: file.to_string(),
                        is_static,
                        calls,
                    },
                );
            }
        } else {
            collect_functions(child, src, file, defs);
        }
    }
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
