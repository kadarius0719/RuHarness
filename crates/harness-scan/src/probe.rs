//! The features probe's source rewrite (docs/FEATURES-DESIGN.md §5.3): a
//! copy of a C file in which every watchable function notes, the first time
//! it runs, that it ran. Built on the scanner's own function collection, so
//! the ids are the facts' own; nothing but the insertions changes — line
//! numbers, `__LINE__` and `__func__` stay the original's.

use crate::{collect_functions, FnDef};
use harness_core::error::Error;

/// A probed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    /// The file with a note after the opening `{` of every watched body.
    pub source: Vec<u8>,
    /// Canonical ids of the definitions the probe could not watch (under an
    /// ERROR node, a body that is not a real `{`-block, a directive between
    /// the declarator and the body), in source order, each once.
    pub unwatched: Vec<String>,
}

/// Probe `source`, the file at `rel_path` (repo-relative, as the facts name
/// it). `index_of(canonical id)` is the note's number: the index of the
/// definition's `(file, id)` pair among the facts' distinct pairs. A
/// definition whose id `index_of` does not know is an error — the facts are
/// stale — never a guessed number.
pub fn probe_source(
    rel_path: &str,
    source: &[u8],
    index_of: &dyn Fn(&str) -> Option<u32>,
) -> Result<Probed, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::parse(rel_path, "tree-sitter parse failed"))?;
    let mut defs: Vec<FnDef> = Vec::new();
    collect_functions(tree.root_node(), source, rel_path, &mut defs);
    let mut inserts: Vec<(usize, u32)> = Vec::new();
    let mut unwatched: Vec<String> = Vec::new();
    for def in &defs {
        let id = def.canonical_id();
        let n = index_of(&id).ok_or_else(|| {
            Error::Invariant(format!(
                "{rel_path}: `{id}` is not in the facts — the facts are stale; scan again"
            ))
        })?;
        match def.probe_at {
            Some(at) => inserts.push((at, n)),
            None if !unwatched.contains(&id) => unwatched.push(id),
            None => {}
        }
    }
    inserts.sort();
    let mut out = Vec::with_capacity(source.len() + inserts.len() * 64);
    let mut from = 0;
    for (at, n) in inserts {
        out.extend_from_slice(&source[from..=at]);
        out.extend_from_slice(note(n).as_bytes());
        from = at + 1;
    }
    out.extend_from_slice(&source[from..]);
    Ok(Probed {
        source: out,
        unwatched,
    })
}

/// The note for id `n`: a one-byte test, then the runtime's call —
/// parenthesised, so a function-like macro of the target cannot capture it.
fn note(n: u32) -> String {
    format!("if (!__ruharness_seen[{n}]) (__ruharness_probe)({n});")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<'a>(pairs: &'a [&'a str]) -> impl Fn(&str) -> Option<u32> + 'a {
        move |id| pairs.iter().position(|p| *p == id).map(|i| i as u32)
    }

    fn probe(src: &str, known: &[&str]) -> Probed {
        probe_source("src/a.c", src.as_bytes(), &ids(known)).expect("probes")
    }

    fn text(p: &Probed) -> String {
        String::from_utf8(p.source.clone()).expect("utf8")
    }

    #[test]
    fn a_note_goes_right_after_each_body_brace_and_nothing_else_moves() {
        let src = "#include <stdio.h>\nstatic int helper(int x)\n{\n  return x + 1;\n}\n\
                   int api(void) { return helper(2); }\n";
        let p = probe(src, &["src/a.c::helper", "api"]);
        let out = text(&p);
        assert_eq!(out.lines().count(), src.lines().count(), "no line moves");
        assert!(out.contains("{if (!__ruharness_seen[0]) (__ruharness_probe)(0);\n  return x + 1;"));
        assert!(out.contains(
            "int api(void) {if (!__ruharness_seen[1]) (__ruharness_probe)(1); return helper(2); }"
        ));
        assert!(p.unwatched.is_empty());
        // Removing the notes gives the original back, byte for byte.
        let stripped = out
            .replace("if (!__ruharness_seen[0]) (__ruharness_probe)(0);", "")
            .replace("if (!__ruharness_seen[1]) (__ruharness_probe)(1);", "");
        assert_eq!(stripped, src);
    }

    #[test]
    fn if_variants_share_an_id_and_k_and_r_definitions_are_watched() {
        let src = "#if A\nint f(void) { return 1; }\n#else\nint f(void) { return 2; }\n#endif\n\
                   int kr(a) int a; { return a; }\n";
        let p = probe(src, &["f", "kr"]);
        let out = text(&p);
        assert_eq!(out.matches("(__ruharness_probe)(0);").count(), 2, "{out}");
        assert!(
            out.contains("int kr(a) int a; {if (!__ruharness_seen[1])"),
            "{out}"
        );
    }

    #[test]
    fn a_brace_inside_if_or_an_error_is_unwatched_not_guessed() {
        let src = "int split(int a)\n#if X\n{ return a; }\n#else\n{ return -a; }\n#endif\n\
                   int ok(void) { return 0; }\n";
        let p = probe(src, &["split", "ok"]);
        assert_eq!(p.unwatched, ["split"], "{}", text(&p));
        assert!(text(&p).contains("int ok(void) {if (!__ruharness_seen[1])"));
        assert!(!text(&p).contains("(__ruharness_probe)(0)"));

        // An error in the head is unwatched; one in the body (a loop macro
        // the parser cannot read) is not in the note's way (review M9).
        let broken = "int good(void) { return 1; }\nint bad(int x,) { return x; }\n";
        let p = probe_source("src/a.c", broken.as_bytes(), &|_: &str| Some(0)).expect("probes");
        assert!(p.unwatched.contains(&"bad".to_string()), "{p:?}");
        let looped = "int total(int n) { int t = 0; FOREACH(i, n) { t += i; } return t; }\n";
        let p = probe(looped, &["total"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        assert!(text(&p).contains("int total(int n) {if (!__ruharness_seen[0])"));
        // A `#` in a comment between the head and the body is no directive.
        let allman = "int allman(void) // fixes #42\n{\n  return 0;\n}\n";
        let p = probe(allman, &["allman"]);
        assert!(p.unwatched.is_empty(), "{}", text(&p));
    }

    /// Review M2: a body that must open with a `#pragma`, and a naked
    /// function, take no note — unwatched, never a probed copy that fails
    /// to build.
    #[test]
    fn a_leading_pragma_or_a_naked_function_is_unwatched() {
        let src = "double f(double x) {\n#pragma STDC FENV_ACCESS ON\n  return x * 2; }\n\
                   double g(double x) {\n  /* fast */\n#pragma clang fp contract(fast)\n  return x; }\n\
                   __attribute__((naked)) void h(void) { __asm__(\"ret\"); }\n\
                   int k(void) {\n  int a = 1;\n#pragma unroll\n  return a; }\n";
        let p = probe(src, &["f", "g", "h", "k"]);
        assert_eq!(p.unwatched, ["f", "g", "h"], "{}", text(&p));
        assert!(text(&p).contains("int k(void) {if (!__ruharness_seen[3])"));
        // Fix check N1/M2: every spelling of a leading pragma, and a body
        // that opens with what the parser cannot read.
        let src = "double a(double x) {\n# pragma STDC FENV_ACCESS ON\n  return x; }\n\
                   double b(double x) {\n#ifdef FAST\n#pragma clang fp contract(fast)\n#endif\n  return x; }\n\
                   double c(double x) { _Pragma(\"clang fp contract(fast)\") return x; }\n\
                   double d(double x) { FP_FAST return x; }\n\
                   int e(int n) { int t = 0; FOREACH(i, n) { t += i; } return t; }\n";
        let p = probe(src, &["a", "b", "c", "d", "e"]);
        assert_eq!(p.unwatched, ["a", "b", "c", "d"], "{}", text(&p));
        // Fix check N4: `naked` only as a word of the head.
        let src = "static int naked_count(struct naked_list *l) { return 0; }\n\
                   int snaked(int x) { return x; }\n";
        let p = probe(src, &["src/a.c::naked_count", "snaked"]);
        assert!(p.unwatched.is_empty(), "{}", text(&p));
    }

    #[test]
    fn two_functions_of_one_name_in_two_files_get_their_own_notes() {
        let known = ["src/a.c::helper", "src/b.c::helper"];
        let a = probe_source("src/a.c", b"static void helper(void) {}\n", &ids(&known)).unwrap();
        let b = probe_source("src/b.c", b"static void helper(void) {}\n", &ids(&known)).unwrap();
        assert!(text(&a).contains("(__ruharness_probe)(0);"));
        assert!(text(&b).contains("(__ruharness_probe)(1);"));
    }

    #[test]
    fn a_definition_the_facts_do_not_know_is_an_error() {
        let err = probe_source("src/a.c", b"int newer(void) { return 0; }\n", &|_: &str| {
            None
        })
        .expect_err("stale facts");
        assert!(err.to_string().contains("the facts are stale"), "{err}");
    }

    #[test]
    fn macro_made_definitions_get_nothing() {
        // A macro that expands to a definition is invisible (the facts do
        // not know it either). The invocation also confuses the parser about
        // what follows it, so a definition right after it sits under an
        // ERROR node: unwatched, never guessed.
        let src =
            "#define DEF(n) int n(void) { return 0; }\nDEF(made)\nint real(void) { return 1; }\n";
        let p = probe(src, &["real"]);
        assert_eq!(p.unwatched, ["real"], "{}", text(&p));
        assert!(!text(&p).contains("__ruharness_probe"));
        let src =
            "#define DEF(n) int n(void) { return 0; }\nDEF(made);\nint real(void) { return 1; }\n";
        let p = probe(src, &["real"]);
        assert!(
            text(&p).contains("int real(void) {if (!__ruharness_seen[0])"),
            "{}",
            text(&p)
        );
        assert_eq!(text(&p).matches("__ruharness_probe").count(), 1);
    }
}
