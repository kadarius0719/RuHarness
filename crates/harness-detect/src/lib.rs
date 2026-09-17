//! RuHarness built-in hazard detectors (docs/SCHEMAS.md "M2 additions").
//!
//! One suite at M2: [`CTreeSitterSuite`], a tree-sitter based C detector
//! implementing [`harness_core::traits::Detector`]. Each source file is
//! parsed once and all category visitors run over the shared tree; the
//! `nonlocal`, `concurrency` and `alloc` categories are driven by the
//! scanner facts instead of the tree. The `macros` category alone is a
//! lexical scan over the raw bytes: tree-sitter-c mis-parses `#define`
//! bodies that mix line continuations with comments (zopfli's
//! ZOPFLI_APPEND_DATA yields ERROR/MISSING nodes with truncated ranges),
//! so macro extents cannot be trusted to the tree.
//!
//! Findings are a pure function of (tree, facts, suite): deterministic,
//! plan-independent, with content-keyed ids via
//! [`harness_core::observer::finding_id`]. The `detector` field of each
//! finding is the category *group* name (`macros`, `nonlocal`, …) so ids
//! survive suite growth.
//!
//! # Identity bytes per group
//!
//! The id rule in docs/SCHEMAS.md hashes "the exact spanned source bytes";
//! each group defines what those bytes are, and the rule is stated again
//! next to every detector below:
//!
//! | group | identity bytes |
//! |---|---|
//! | `macros` | the definition text from `#` through the newline ending its last physical line |
//! | `nonlocal`, `concurrency` | callee name ‖ NUL ‖ caller canonical symbol id (facts refs carry no byte span) |
//! | `alloc` | the function's canonical symbol id (one finding per function) |
//! | `variadic` | the `function_definition` node's bytes |
//! | `layout` | the `union_specifier` / `field_declaration` node's bytes |
//! | `fn-pointer` | decl: the `type_definition` / `parameter_declaration` / `field_declaration` node's bytes; arg: the identifier argument's bytes |
//! | `global` | the `declaration` node's bytes |
//!
//! `span` is display data everywhere: 1-based physical lines, end-inclusive.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::facts::{Facts, SymbolRecord};
use harness_core::hash::file_hash;
use harness_core::observer::{finding_id, Finding};
use harness_core::traits::Detector;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The built-in C detector suite (`c-treesitter-v1`).
///
/// Walks the target's `source_dir` for `*.c` / `*.h` files (the same walk as
/// the scanner), parses each once, and emits findings for the eight category
/// groups: `macros`, `nonlocal`, `concurrency`, `variadic`, `layout`,
/// `fn-pointer`, `global`, `alloc`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CTreeSitterSuite;

const SEV_HIGH: &str = "high";
const SEV_MEDIUM: &str = "medium";
const SEV_LOW: &str = "low";
const SEV_INFO: &str = "info";

const SETJMP_NAMES: [&str; 6] = [
    "setjmp",
    "longjmp",
    "_setjmp",
    "_longjmp",
    "sigsetjmp",
    "siglongjmp",
];
const SIGNAL_NAMES: [&str; 3] = ["signal", "sigaction", "raise"];
/// Threading/atomics API families matched by prefix (POSIX threads, C11
/// `threads.h` mutex/thread/condition/once families, atomics, semaphores).
const THREADING_PREFIXES: [&str; 7] = [
    "pthread_", "mtx_", "thrd_", "cnd_", "once_", "atomic_", "sem_",
];
/// Threading API entry points matched by exact name (no shared prefix).
const THREADING_NAMES: [&str; 1] = ["call_once"];
const ALLOC_NAMES: [&str; 4] = ["malloc", "calloc", "realloc", "strdup"];
const VA_NAMES: [&str; 3] = ["va_start", "va_arg", "va_copy"];
/// Declarator kinds that introduce a function type (named and unnamed).
const FN_DECLARATOR_KINDS: [&str; 2] = ["function_declarator", "abstract_function_declarator"];

/// A finding before occurrence assignment and id computation.
struct RawFinding {
    detector: &'static str,
    category: &'static str,
    severity: &'static str,
    blocker: bool,
    human_mandatory: bool,
    /// 1-based (start_line, end_line) — display data.
    span: (u32, u32),
    /// Byte-order key within the file (occurrence is counted in this order).
    order: u64,
    /// The exact bytes keyed into the content id.
    spanned: Vec<u8>,
    message: String,
    evidence: String,
}

/// A parsed source file shared by all tree visitors.
struct ParsedFile {
    rel: String,
    source: String,
    tree: tree_sitter::Tree,
    hash: String,
    is_c: bool,
}

impl Detector for CTreeSitterSuite {
    fn name(&self) -> &'static str {
        "c-treesitter-v1"
    }

    fn detect(&self, target: &TargetContext, facts: &Facts) -> Result<Vec<Finding>, Error> {
        let src_dir = target.root.join(&target.config.target.source_dir);
        let mut abs_files: Vec<PathBuf> = Vec::new();
        collect_source_files(&src_dir, &mut abs_files)?;
        let mut files: Vec<(String, PathBuf)> = Vec::with_capacity(abs_files.len());
        for abs in abs_files {
            files.push((repo_relative(&target.root, &abs)?, abs));
        }
        files.sort();

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;

        let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files.len());
        for (rel, abs) in &files {
            let source = std::fs::read_to_string(abs).map_err(|e| Error::io(abs, e))?;
            let tree = parser
                .parse(&source, None)
                .ok_or_else(|| Error::parse(abs, "tree-sitter parse failed"))?;
            parsed.push(ParsedFile {
                rel: rel.clone(),
                source,
                tree,
                hash: file_hash(abs)?,
                is_c: rel.ends_with(".c"),
            });
        }

        // Facts-side lookup tables.
        let mut publics: BTreeSet<&str> = BTreeSet::new();
        let mut statics: BTreeSet<(&str, &str)> = BTreeSet::new();
        let mut symbol_by_name: BTreeMap<&str, &SymbolRecord> = BTreeMap::new();
        for s in &facts.symbols {
            symbol_by_name.insert(s.name.as_str(), s);
            match s.name.rsplit_once("::") {
                Some((file, short)) => {
                    statics.insert((file, short));
                }
                None => {
                    publics.insert(s.name.as_str());
                }
            }
        }
        let mut refs_by_from: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for r in &facts.refs {
            refs_by_from
                .entry(r.from.as_str())
                .or_default()
                .insert(r.to.as_str());
        }

        // Pass A: function-typedef names across all files (used by the
        // fn-pointer parameter/field checks; typedefs usually live in
        // headers). Direct shapes first, then aliases of known names
        // (`typedef cmp_t chained_t;`) to a fixpoint so chains resolve
        // regardless of file order.
        let mut fn_typedefs: BTreeSet<String> = BTreeSet::new();
        for p in &parsed {
            collect_fn_typedef_names(p.tree.root_node(), p.source.as_bytes(), &mut fn_typedefs);
        }
        loop {
            let mut grew = false;
            for p in &parsed {
                grew |= collect_fn_typedef_aliases(
                    p.tree.root_node(),
                    p.source.as_bytes(),
                    &mut fn_typedefs,
                );
            }
            if !grew {
                break;
            }
        }

        // Pass B: pointer-return shape of every function definition, keyed
        // by canonical symbol id (used by the alloc check; decided from the
        // declarator chain, never from the display signature).
        let mut returns_pointer_by_symbol: BTreeMap<String, bool> = BTreeMap::new();
        for p in &parsed {
            collect_pointer_returns(
                p.tree.root_node(),
                p.source.as_bytes(),
                &p.rel,
                &mut returns_pointer_by_symbol,
            );
        }

        // Per-file raw findings.
        let mut raw_by_file: BTreeMap<String, Vec<RawFinding>> = files
            .iter()
            .map(|(rel, _)| (rel.clone(), Vec::new()))
            .collect();
        let hash_by_file: BTreeMap<&str, &str> = parsed
            .iter()
            .map(|p| (p.rel.as_str(), p.hash.as_str()))
            .collect();

        for p in &parsed {
            let ctx = FileCtx {
                rel: &p.rel,
                src: p.source.as_bytes(),
                is_c: p.is_c,
                fn_typedefs: &fn_typedefs,
                publics: &publics,
                statics: &statics,
                refs_by_from: &refs_by_from,
            };
            let raws = raw_by_file
                .get_mut(&p.rel)
                .ok_or_else(|| Error::Invariant(format!("missing raw bucket for {}", p.rel)))?;
            scan_macros(&p.source, raws);
            visit(p.tree.root_node(), &ctx, false, raws);
        }

        // Facts-driven categories: nonlocal + concurrency (refs), alloc
        // (symbols). Identity bytes for nonlocal/concurrency are
        // `callee ‖ NUL ‖ caller canonical id`: a ref has no byte span, and
        // this key is stable across line shifts and body edits.
        for r in &facts.refs {
            let hit: Option<(&'static str, &'static str, String)> =
                if !r.resolved && SETJMP_NAMES.contains(&r.to.as_str()) {
                    Some((
                        "nonlocal",
                        "setjmp-longjmp",
                        format!(
                            "call to {}: nonlocal control flow has no safe Rust equivalent",
                            r.to
                        ),
                    ))
                } else if !r.resolved && SIGNAL_NAMES.contains(&r.to.as_str()) {
                    Some((
                        "nonlocal",
                        "signal-handler",
                        format!(
                            "call to {}: signal-handler semantics need a human migration decision",
                            r.to
                        ),
                    ))
                } else if THREADING_PREFIXES.iter().any(|p| r.to.starts_with(p))
                    || THREADING_NAMES.contains(&r.to.as_str())
                {
                    Some((
                        "concurrency",
                        "threading-api",
                        format!(
                            "call to {}: threading/atomics API needs a human concurrency model",
                            r.to
                        ),
                    ))
                } else {
                    None
                };
            let Some((detector, category, message)) = hit else {
                continue;
            };
            let symbol = symbol_by_name.get(r.from.as_str()).ok_or_else(|| {
                Error::Invariant(format!("facts ref from unknown symbol `{}`", r.from))
            })?;
            let raws = raw_by_file.get_mut(&symbol.file).ok_or_else(|| {
                Error::Invariant(format!(
                    "facts reference `{}` in {} which is not under source_dir — re-run `harness scan`",
                    r.from, symbol.file
                ))
            })?;
            raws.push(RawFinding {
                detector,
                category,
                severity: SEV_HIGH,
                blocker: true,
                human_mandatory: true,
                span: symbol.span,
                order: u64::from(symbol.span.0),
                // The ref has no byte span; key the id on callee + caller
                // canonical id so it is stable across line shifts.
                spanned: format!("{}\0{}", r.to, r.from).into_bytes(),
                message: clamp_message(message),
                evidence: excerpt(&symbol.signature),
            });
        }

        // `alloc` group. Identity bytes are the function's canonical symbol
        // id: one finding per function, stable across edits to its body.
        for s in &facts.symbols {
            if s.kind != "function" {
                continue;
            }
            let empty = BTreeSet::new();
            let tos = refs_by_from.get(s.name.as_str()).unwrap_or(&empty);
            let allocs: Vec<&str> = ALLOC_NAMES
                .iter()
                .copied()
                .filter(|n| tos.contains(n))
                .collect();
            let frees = tos.contains("free");
            // Tree first (pass B); the display signature is only a fallback
            // for symbols whose definition the tree walk did not see.
            let returns_pointer = returns_pointer_by_symbol
                .get(s.name.as_str())
                .copied()
                .unwrap_or_else(|| signature_returns_pointer(&s.signature));
            if !((!allocs.is_empty() && returns_pointer) || frees) {
                continue;
            }
            let mut detail: Vec<String> = Vec::new();
            if !allocs.is_empty() && returns_pointer {
                detail.push(format!("{} + pointer return", allocs.join("/")));
            }
            if frees {
                detail.push("calls free".to_string());
            }
            let short = s.name.rsplit("::").next().unwrap_or(&s.name);
            let raws = raw_by_file.get_mut(&s.file).ok_or_else(|| {
                Error::Invariant(format!(
                    "facts symbol `{}` in {} which is not under source_dir — re-run `harness scan`",
                    s.name, s.file
                ))
            })?;
            raws.push(RawFinding {
                detector: "alloc",
                category: "alloc-ownership",
                severity: SEV_INFO,
                blocker: false,
                human_mandatory: false,
                span: s.span,
                order: u64::from(s.span.0),
                spanned: s.name.clone().into_bytes(),
                message: clamp_message(format!(
                    "{short}: heap ownership crosses the function boundary ({})",
                    detail.join(", ")
                )),
                evidence: excerpt(&s.signature),
            });
        }

        // Occurrence assignment (0-based index among identical
        // (detector, category, spanned-bytes) per file, counted in byte
        // order), then id computation and final canonical-ish ordering.
        let mut out: Vec<Finding> = Vec::new();
        for (rel, mut raws) in raw_by_file {
            let file_hash = hash_by_file
                .get(rel.as_str())
                .ok_or_else(|| Error::Invariant(format!("no hash recorded for {rel}")))?
                .to_string();
            raws.sort_by(|a, b| {
                a.order
                    .cmp(&b.order)
                    .then_with(|| a.detector.cmp(b.detector))
                    .then_with(|| a.category.cmp(b.category))
                    .then_with(|| a.spanned.cmp(&b.spanned))
            });
            let mut counts: BTreeMap<(&str, &str, Vec<u8>), u32> = BTreeMap::new();
            for raw in raws {
                let count = counts
                    .entry((raw.detector, raw.category, raw.spanned.clone()))
                    .or_insert(0);
                let occurrence = *count;
                *count += 1;
                out.push(Finding {
                    id: finding_id(raw.detector, raw.category, &rel, &raw.spanned, occurrence),
                    detector: raw.detector.to_string(),
                    category: raw.category.to_string(),
                    severity: raw.severity.to_string(),
                    blocker: raw.blocker,
                    human_mandatory: raw.human_mandatory,
                    file: rel.clone(),
                    file_hash: file_hash.clone(),
                    span: raw.span,
                    occurrence,
                    message: raw.message,
                    evidence: raw.evidence,
                });
            }
        }
        out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }
}

/// Shared per-file context for the tree visitors.
struct FileCtx<'a> {
    rel: &'a str,
    src: &'a [u8],
    is_c: bool,
    fn_typedefs: &'a BTreeSet<String>,
    publics: &'a BTreeSet<&'a str>,
    statics: &'a BTreeSet<(&'a str, &'a str)>,
    refs_by_from: &'a BTreeMap<&'a str, BTreeSet<&'a str>>,
}

/// One pre-order walk running every tree-driven category visitor. Every
/// tree-driven finding's identity bytes are the flagged node's exact byte
/// range (see [`tree_finding`]); the node kind per category is listed in
/// the crate docs.
fn visit(node: tree_sitter::Node, ctx: &FileCtx, in_function: bool, out: &mut Vec<RawFinding>) {
    match node.kind() {
        "union_specifier" => {
            if node.child_by_field_name("body").is_some() {
                out.push(tree_finding(
                    node,
                    ctx,
                    "layout",
                    "union-decl",
                    SEV_HIGH,
                    false,
                    true,
                    "union declaration: layout and type-punning semantics need human review".into(),
                ));
            }
        }
        "field_declaration" => {
            let mut cursor = node.walk();
            let has_bitfield = node
                .children(&mut cursor)
                .any(|c| c.kind() == "bitfield_clause");
            if has_bitfield {
                out.push(tree_finding(
                    node,
                    ctx,
                    "layout",
                    "bitfield",
                    SEV_HIGH,
                    false,
                    true,
                    "struct field uses bit-field layout; Rust has no ABI-compatible bit-field"
                        .into(),
                ));
            }
            // fn-pointer (field): `int (*read)(void *);` or a field of a
            // known function typedef type, inside any struct/union body —
            // typedef'd or not. The field is the finding; a typedef around
            // the struct does not fire again for it.
            if declarator_function(node).is_some() || type_is_fn_typedef(node, ctx) {
                out.push(tree_finding(
                    node,
                    ctx,
                    "fn-pointer",
                    "function-pointer-decl",
                    SEV_MEDIUM,
                    false,
                    false,
                    "struct/union field of function-pointer type: indirect call edge invisible to scanner"
                        .into(),
                ));
            }
        }
        "function_definition" => {
            variadic_finding(node, ctx, out);
            let mut cursor = node.walk();
            let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
            for child in children {
                visit(child, ctx, true, out);
            }
            return;
        }
        "type_definition" => {
            // Only the typedef's own declarator counts: a function pointer
            // inside a struct body in its type is that field's finding.
            if let Some(fd) = declarator_function(node) {
                let name = declared_type_name(fd, ctx.src).unwrap_or_default();
                out.push(tree_finding(
                    node,
                    ctx,
                    "fn-pointer",
                    "function-pointer-decl",
                    SEV_MEDIUM,
                    false,
                    false,
                    clamp_message(format!(
                        "typedef {name} declares a function type: indirect calls are invisible to the scanner"
                    )),
                ));
            } else if type_is_fn_typedef(node, ctx) {
                let name = alias_type_name(node, ctx.src).unwrap_or_default();
                out.push(tree_finding(
                    node,
                    ctx,
                    "fn-pointer",
                    "function-pointer-decl",
                    SEV_MEDIUM,
                    false,
                    false,
                    clamp_message(format!(
                        "typedef {name} aliases a function type: indirect calls are invisible to the scanner"
                    )),
                ));
            }
        }
        "parameter_declaration" => {
            // Named (`void (*cb)(int)`) and unnamed (`void (*)(int)`)
            // function-pointer parameters, plus parameters of a known
            // function typedef type (including chained aliases).
            let fn_type = declarator_function(node).is_some();
            let typedef_type = type_is_fn_typedef(node, ctx);
            if fn_type || typedef_type {
                out.push(tree_finding(
                    node,
                    ctx,
                    "fn-pointer",
                    "function-pointer-decl",
                    SEV_MEDIUM,
                    false,
                    false,
                    "parameter of function or function-pointer type: indirect call edge invisible to scanner"
                        .into(),
                ));
            }
        }
        "call_expression" => fn_pointer_args(node, ctx, out),
        "declaration" => {
            if ctx.is_c && !in_function {
                global_finding(node, ctx, out);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    for child in children {
        visit(child, ctx, in_function, out);
    }
}

/// `macros` group: lexical scan for function-like `#define`s. One finding per
/// definition at its most severe matching category. Works on raw bytes (with
/// backslash continuations joined) because tree-sitter-c cannot represent
/// continuation-plus-comment macro bodies. Identity bytes are the whole
/// definition from `#` through the newline ending its last physical line;
/// the span is that first line through that last line, end-inclusive, like
/// every other detector.
fn scan_macros(source: &str, out: &mut Vec<RawFinding>) {
    let bytes = source.as_bytes();
    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    if line_starts.last() == Some(&bytes.len()) {
        line_starts.pop(); // no phantom line after a trailing newline
    }
    let line_end = |li: usize| -> usize { line_starts.get(li + 1).copied().unwrap_or(bytes.len()) };

    let mut in_comment = false;
    let mut li = 0;
    while li < line_starts.len() {
        let line = &source[line_starts[li]..line_end(li)];
        let was_in_comment = in_comment;
        in_comment = comment_state_after(line, in_comment);
        if was_in_comment {
            li += 1;
            continue;
        }
        let trimmed = line.trim_start();
        let Some(name) = function_like_define_name(trimmed) else {
            li += 1;
            continue;
        };
        // Extent: this line plus every backslash-continued successor.
        let mut last = li;
        while last < line_starts.len() && continues(&source[line_starts[last]..line_end(last)]) {
            last += 1;
        }
        let last = last.min(line_starts.len() - 1);
        let def_start = line_starts[li] + (line.len() - trimmed.len());
        let def_end = line_end(last);
        for l in (li + 1)..=last {
            in_comment = comment_state_after(&source[line_starts[l]..line_end(l)], in_comment);
        }
        let text = &source[def_start..def_end];
        let (params, body) = split_macro_params_body(text, &name);
        push_macro_finding(
            &name,
            params,
            body,
            ((li + 1) as u32, (last + 1) as u32),
            def_start as u64,
            text,
            out,
        );
        li = last + 1;
    }
}

/// The macro name when `line` (already left-trimmed) is a function-like
/// `#define` — an identifier immediately followed by `(`.
fn function_like_define_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("define")?;
    let after_ws = rest.trim_start();
    if after_ws.len() == rest.len() {
        return None; // `#definefoo`
    }
    let name_len = after_ws
        .char_indices()
        .take_while(|(i, c)| {
            if *i == 0 {
                c.is_ascii_alphabetic() || *c == '_'
            } else {
                c.is_ascii_alphanumeric() || *c == '_'
            }
        })
        .count();
    if name_len == 0 || !after_ws[name_len..].starts_with('(') {
        return None;
    }
    Some(after_ws[..name_len].to_string())
}

/// Split a joined macro definition into its parameter list text and
/// replacement body (paren-balance from the `(` after the name).
fn split_macro_params_body<'a>(text: &'a str, name: &str) -> (&'a str, &'a str) {
    let Some(name_pos) = text.find(name) else {
        return ("", "");
    };
    let open = name_pos + name.len();
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (&text[open..=i], &text[i + 1..]);
                }
            }
            _ => {}
        }
    }
    (&text[open..], "")
}

/// Block-comment state after scanning one physical line (line comments end
/// the scan; string literals are not tracked — sufficient for directives).
fn comment_state_after(line: &str, mut in_comment: bool) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if in_comment {
            if b[i] == b'*' && b[i + 1] == b'/' {
                in_comment = false;
                i += 2;
            } else {
                i += 1;
            }
        } else if b[i] == b'/' && b[i + 1] == b'*' {
            in_comment = true;
            i += 2;
        } else if b[i] == b'/' && b[i + 1] == b'/' {
            break;
        } else {
            i += 1;
        }
    }
    in_comment
}

/// Whether a physical line's content ends with a `\` continuation.
fn continues(line: &str) -> bool {
    line.trim_end_matches(['\n', '\r']).ends_with('\\')
}

/// Classify one function-like macro and push its finding.
fn push_macro_finding(
    name: &str,
    params: &str,
    body: &str,
    span: (u32, u32),
    order: u64,
    text: &str,
    out: &mut Vec<RawFinding>,
) {
    let (category, severity, message) = if body.contains('#') {
        // `##` token pasting or a stringizing `#` — both defeat translation.
        (
            "macro-token-pasting",
            SEV_HIGH,
            format!(
                "macro {name} uses token pasting or stringizing; no mechanical Rust equivalent"
            ),
        )
    } else if body.contains('{') || body.contains(';') {
        let has_alloc = ["malloc", "realloc", "free"]
            .iter()
            .any(|t| contains_token(body, t));
        if has_alloc {
            (
                "macro-statement-body",
                SEV_HIGH,
                format!("function-like macro {name} with statement body and embedded allocation"),
            )
        } else {
            (
                "macro-statement-body",
                SEV_MEDIUM,
                format!("function-like macro {name} with statement body"),
            )
        }
    } else if params.contains("...") {
        (
            "macro-variadic",
            SEV_MEDIUM,
            format!("variadic macro {name}: argument packs have no direct Rust equivalent"),
        )
    } else {
        (
            "macro-function-like",
            SEV_LOW,
            format!("function-like macro {name}: expansion semantics must be preserved"),
        )
    };
    out.push(RawFinding {
        detector: "macros",
        category,
        severity,
        blocker: false,
        human_mandatory: false,
        span,
        order,
        spanned: text.as_bytes().to_vec(),
        message: clamp_message(message),
        evidence: excerpt(text),
    });
}

/// `variadic` group: one finding per function that has `...` parameters or
/// references `va_start`/`va_arg`/`va_copy` (folded, never separate).
/// Identity bytes are the `function_definition` node's bytes.
fn variadic_finding(def: tree_sitter::Node, ctx: &FileCtx, out: &mut Vec<RawFinding>) {
    let Some((fd, _)) = function_declarator_of(def) else {
        return;
    };
    let has_ellipsis = fd.child_by_field_name("parameters").is_some_and(|params| {
        let mut cursor = params.walk();
        let found = params
            .children(&mut cursor)
            .any(|c| c.kind() == "variadic_parameter");
        found
    });
    let uses_va = function_name(def, ctx.src).is_some_and(|name| {
        let canonical = if is_static_definition(def, ctx.src) {
            format!("{}::{}", ctx.rel, name)
        } else {
            name
        };
        ctx.refs_by_from
            .get(canonical.as_str())
            .is_some_and(|tos| VA_NAMES.iter().any(|v| tos.contains(v)))
    });
    if has_ellipsis || uses_va {
        out.push(tree_finding(
            def,
            ctx,
            "variadic",
            "variadic-function",
            SEV_MEDIUM,
            false,
            false,
            "variadic function: C varargs need an explicit Rust redesign".into(),
        ));
    }
}

/// `fn-pointer` (arg): bare-identifier call arguments naming project
/// functions. Identity bytes are the identifier argument's bytes.
fn fn_pointer_args(call: tree_sitter::Node, ctx: &FileCtx, out: &mut Vec<RawFinding>) {
    let Some(args) = call.child_by_field_name("arguments") else {
        return;
    };
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        if arg.kind() != "identifier" {
            continue;
        }
        let name = text(arg, ctx.src);
        if !(ctx.publics.contains(name) || ctx.statics.contains(&(ctx.rel, name))) {
            continue;
        }
        out.push(RawFinding {
            detector: "fn-pointer",
            category: "function-pointer-arg",
            severity: SEV_MEDIUM,
            blocker: false,
            human_mandatory: false,
            span: node_span(arg),
            order: arg.start_byte() as u64,
            spanned: ctx.src[arg.start_byte()..arg.end_byte()].to_vec(),
            message: clamp_message(format!(
                "function {name} passed as a value argument: forms an indirect call edge"
            )),
            evidence: excerpt(text(call, ctx.src)),
        });
    }
}

/// `global` group: mutable file-scope objects in `.c` files. Identity bytes
/// are the `declaration` node's bytes (one finding per declaration).
///
/// Fires when any declarator defines a mutable object: `extern` without an
/// initializer only declares (quiet), `extern` with one defines (fires).
/// Const-ness is decided at the declarator level binding the name (see
/// [`object_constness`]): a declaration-level `const` reaches the object
/// only through array/paren layers, never through a pointer layer, so
/// `const char *p` fires while `char *const p` and `static const T t[N]`
/// are quiet.
fn global_finding(node: tree_sitter::Node, ctx: &FileCtx, out: &mut Vec<RawFinding>) {
    let mut top_const = false;
    let mut is_extern = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_qualifier" if text(child, ctx.src) == "const" => top_const = true,
            "storage_class_specifier" => match text(child, ctx.src) {
                "typedef" => return,
                "extern" => is_extern = true,
                _ => {}
            },
            _ => {}
        }
    }
    drop(cursor);
    let mut cursor = node.walk();
    let defines_mutable = node
        .children_by_field_name("declarator", &mut cursor)
        .any(|d| {
            let is_definition = !is_extern || d.kind() == "init_declarator";
            is_definition && object_constness(d, top_const, ctx.src) == Some(false)
        });
    if defines_mutable {
        out.push(tree_finding(
            node,
            ctx,
            "global",
            "global-mutable",
            SEV_HIGH,
            false,
            false,
            "mutable file-scope global: needs a Rust ownership/synchronization decision".into(),
        ));
    }
}

/// Which declarator layer binds the declared name's type.
#[derive(Clone, Copy)]
enum Binding {
    /// No pointer/function layer: the declaration-level qualifiers apply.
    Top,
    /// A pointer layer, with its own `const` qualifier state.
    Pointer { is_const: bool },
    /// A function layer with nothing but parentheses below it: a prototype.
    Function,
}

/// Whether a declarator declares an object (variable) and, if so, whether
/// that object is itself const: `Some(is_const)` for objects, `None` for
/// function prototypes and unrecognized shapes.
///
/// Walks top-down; the layer visited last before the identifier is the one
/// binding the name, and it decides. A `pointer_declarator` decides with its
/// own qualifier (`*const`), so `top_const` (the declaration-level `const`)
/// reaches the object only when no pointer layer is on the path — array
/// and parenthesized layers pass it through, which keeps `static const T
/// table[N]` const and makes `const char *p` a mutable pointer.
fn object_constness(mut node: tree_sitter::Node, top_const: bool, src: &[u8]) -> Option<bool> {
    let mut binding = Binding::Top;
    loop {
        match node.kind() {
            "identifier" => {
                return match binding {
                    Binding::Top => Some(top_const),
                    Binding::Pointer { is_const } => Some(is_const),
                    Binding::Function => None,
                }
            }
            "function_declarator" => {
                binding = Binding::Function;
                node = inner_declarator(node)?;
            }
            "pointer_declarator" => {
                binding = Binding::Pointer {
                    is_const: has_const_qualifier(node, src),
                };
                node = inner_declarator(node)?;
            }
            "init_declarator" | "array_declarator" | "parenthesized_declarator" => {
                node = inner_declarator(node)?;
            }
            _ => return None,
        }
    }
}

/// Whether a declarator node carries its own `const` type qualifier
/// (`*const` on a `pointer_declarator`).
fn has_const_qualifier(node: tree_sitter::Node, src: &[u8]) -> bool {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .any(|c| c.kind() == "type_qualifier" && text(c, src) == "const");
    found
}

/// Collect names declared by function-type typedefs (both `typedef R F(A)`
/// and `typedef R (*F)(A)` shapes).
fn collect_fn_typedef_names(node: tree_sitter::Node, src: &[u8], out: &mut BTreeSet<String>) {
    if node.kind() == "type_definition" {
        if let Some(fd) = declarator_function(node) {
            if let Some(name) = declared_type_name(fd, src) {
                out.insert(name);
            }
        }
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    for child in children {
        collect_fn_typedef_names(child, src, out);
    }
}

/// Record typedef aliases of already-known function typedef names
/// (`typedef cmp_t chained_t;`, `typedef cmp_t *cmp_ptr_t;`). Returns
/// whether `known` grew, so the caller can iterate to a fixpoint.
fn collect_fn_typedef_aliases(
    node: tree_sitter::Node,
    src: &[u8],
    known: &mut BTreeSet<String>,
) -> bool {
    let mut grew = false;
    if node.kind() == "type_definition" {
        let base_is_fn = node
            .child_by_field_name("type")
            .filter(|t| t.kind() == "type_identifier")
            .is_some_and(|t| known.contains(text(t, src)));
        if base_is_fn {
            if let Some(name) = alias_type_name(node, src) {
                grew |= known.insert(name);
            }
        }
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    for child in children {
        grew |= collect_fn_typedef_aliases(child, src, known);
    }
    grew
}

/// The `type_identifier` a non-function typedef declares, unwrapping
/// pointer and parenthesized layers (`typedef T *P;` names `P`).
fn alias_type_name(type_definition: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut node = type_definition.child_by_field_name("declarator")?;
    loop {
        match node.kind() {
            "type_identifier" => return Some(text(node, src).to_string()),
            "parenthesized_declarator" | "pointer_declarator" => {
                node = inner_declarator(node)?;
            }
            _ => return None,
        }
    }
}

/// Whether a declaration-like node's `type` is a known function typedef
/// name (direct or chained alias).
fn type_is_fn_typedef(node: tree_sitter::Node, ctx: &FileCtx) -> bool {
    node.child_by_field_name("type")
        .filter(|t| t.kind() == "type_identifier")
        .is_some_and(|t| ctx.fn_typedefs.contains(text(t, ctx.src)))
}

/// The function declarator (named or abstract) reached through one of
/// `node`'s own `declarator` fields, if any. Only the declarator subtree is
/// searched — never the `type` — so a function pointer inside a struct body
/// belongs to that field, not to the enclosing typedef or declaration.
fn declarator_function(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = node.walk();
    let declarators: Vec<tree_sitter::Node> = node
        .children_by_field_name("declarator", &mut cursor)
        .collect();
    drop(cursor);
    declarators.into_iter().find_map(|d| {
        if FN_DECLARATOR_KINDS.contains(&d.kind()) {
            Some(d)
        } else {
            first_descendant_of_kinds(d, &FN_DECLARATOR_KINDS)
        }
    })
}

/// Record, for every function definition the scanner would also record
/// (pointer/paren layers over `function_declarator(identifier)`), whether
/// it returns a pointer: a `pointer_declarator` sits between the
/// definition and its function declarator. Keyed by canonical symbol id.
fn collect_pointer_returns(
    node: tree_sitter::Node,
    src: &[u8],
    rel: &str,
    out: &mut BTreeMap<String, bool>,
) {
    if node.kind() == "function_definition" {
        if let (Some(name), Some((_, returns_pointer))) =
            (function_name(node, src), function_declarator_of(node))
        {
            let canonical = if is_static_definition(node, src) {
                format!("{rel}::{name}")
            } else {
                name
            };
            out.insert(canonical, returns_pointer);
        }
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    for child in children {
        collect_pointer_returns(child, src, rel, out);
    }
}

/// Fallback pointer-return test on a display signature, for symbols whose
/// definition the tree walk did not see: block comments and
/// `__attribute__((...))` specifiers are stripped first so their
/// parentheses cannot mask the return type (`__attribute__((malloc)) char
/// *adup(int)`).
fn signature_returns_pointer(signature: &str) -> bool {
    let stripped = strip_attributes(&strip_block_comments(signature));
    stripped
        .split('(')
        .next()
        .is_some_and(|before| before.contains('*'))
}

/// Remove every `/* ... */` comment (unterminated: to end of text).
fn strip_block_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        rest = match rest[open + 2..].find("*/") {
            Some(close) => &rest[open + 2 + close + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Remove every `__attribute__ (( ... ))` specifier with its balanced
/// parenthesized argument list.
fn strip_attributes(s: &str) -> String {
    const KEYWORD: &str = "__attribute__";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find(KEYWORD) {
        out.push_str(&rest[..at]);
        let after = &rest[at + KEYWORD.len()..];
        let args = after.trim_start();
        let Some(close) = balanced_paren_end(args) else {
            // Malformed: drop the keyword alone and continue.
            rest = after;
            continue;
        };
        rest = &args[close..];
    }
    out.push_str(rest);
    out
}

/// When `s` starts with `(`, the byte index just past its matching `)`.
fn balanced_paren_end(s: &str) -> Option<usize> {
    if !s.starts_with('(') {
        return None;
    }
    let mut depth = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `type_identifier` a typedef'd function declarator declares, unwrapping
/// parentheses and pointers (`(*F)` shapes).
fn declared_type_name(function_declarator: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut node = function_declarator.child_by_field_name("declarator")?;
    loop {
        match node.kind() {
            "type_identifier" => return Some(text(node, src).to_string()),
            "parenthesized_declarator" | "pointer_declarator" => {
                node = inner_declarator(node)?;
            }
            _ => return None,
        }
    }
}

/// The function declarator of a definition, unwrapping pointer/paren layers,
/// plus whether a pointer layer was crossed (the function returns a pointer).
fn function_declarator_of(def: tree_sitter::Node) -> Option<(tree_sitter::Node, bool)> {
    let mut node = def.child_by_field_name("declarator")?;
    let mut through_pointer = false;
    loop {
        match node.kind() {
            "function_declarator" => return Some((node, through_pointer)),
            "pointer_declarator" => {
                through_pointer = true;
                node = inner_declarator(node)?;
            }
            "parenthesized_declarator" => {
                node = inner_declarator(node)?;
            }
            _ => return None,
        }
    }
}

/// The raw source name of a function definition, when its declarator chain
/// bottoms out in a plain identifier.
fn function_name(def: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let decl = function_declarator_of(def)?
        .0
        .child_by_field_name("declarator")?;
    (decl.kind() == "identifier").then(|| text(decl, src).to_string())
}

/// Whether a function definition carries the `static` storage class.
fn is_static_definition(def: tree_sitter::Node, src: &[u8]) -> bool {
    let mut cursor = def.walk();
    let found = def
        .children(&mut cursor)
        .any(|n| n.kind() == "storage_class_specifier" && text(n, src) == "static");
    found
}

/// A declarator's inner declarator: the `declarator` field when present,
/// otherwise the first named child (parenthesized declarators are unfielded).
fn inner_declarator(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    if let Some(d) = node.child_by_field_name("declarator") {
        return Some(d);
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment");
    found
}

/// First pre-order descendant strictly below `node` whose kind is in `kinds`.
fn first_descendant_of_kinds<'a>(
    node: tree_sitter::Node<'a>,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    drop(cursor);
    for child in children {
        if kinds.contains(&child.kind()) {
            return Some(child);
        }
        if let Some(found) = first_descendant_of_kinds(child, kinds) {
            return Some(found);
        }
    }
    None
}

/// Build a raw finding whose identity bytes are the flagged node's exact
/// byte range.
#[allow(clippy::too_many_arguments)]
fn tree_finding(
    node: tree_sitter::Node,
    ctx: &FileCtx,
    detector: &'static str,
    category: &'static str,
    severity: &'static str,
    blocker: bool,
    human_mandatory: bool,
    message: String,
) -> RawFinding {
    RawFinding {
        detector,
        category,
        severity,
        blocker,
        human_mandatory,
        span: node_span(node),
        order: node.start_byte() as u64,
        spanned: ctx.src[node.start_byte()..node.end_byte()].to_vec(),
        message,
        evidence: excerpt(text(node, ctx.src)),
    }
}

fn node_span(node: tree_sitter::Node) -> (u32, u32) {
    (
        (node.start_position().row + 1) as u32,
        (node.end_position().row + 1) as u32,
    )
}

fn text<'a>(node: tree_sitter::Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

/// Whitespace-collapsed single-line source excerpt, ≤200 chars.
fn excerpt(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(200).collect()
}

/// Clamp a message to the ~120-char schema guidance.
fn clamp_message(message: String) -> String {
    if message.chars().count() <= 120 {
        message
    } else {
        message.chars().take(120).collect()
    }
}

/// Whether `haystack` contains `token` as a whole word (ASCII identifier
/// boundaries on both sides).
fn contains_token(haystack: &str, token: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(token) {
        let i = start + pos;
        let end = i + token.len();
        let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        start = i + 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Recursively collect `*.c` and `*.h` files under `dir` (the same walk as
/// the scanner; harness-detect deliberately does not depend on harness-scan).
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
