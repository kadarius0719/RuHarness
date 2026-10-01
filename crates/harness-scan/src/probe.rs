//! The features probe's source rewrite (docs/FEATURES-PROBE-REDESIGN.md
//! §3.1): a copy of a C file in which every function where a note can be
//! placed notes that it ran. Built on the scanner's own function
//! collection, so the ids are the facts' own; nothing but the insertions
//! changes — line numbers, `__LINE__` and `__func__` stay the original's.
//! Whether a note compiles is the compiler's to say (§3.4), not this rule's.

use crate::{collect_functions, FnDef, NoNote};
use harness_core::error::Error;

/// What to probe, and how (docs/FEATURES-PROBE-REDESIGN.md §3.1–§3.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeOptions<'a> {
    /// Canonical ids that get no note (the compile-and-retry pass took them
    /// out, with reasons of its own).
    pub skip: &'a [String],
    /// The compiler is gcc: rule 4 (a naked function) applies.
    pub gcc: bool,
    /// The listing copy: each noted body also carries `__ruharness_end_N`
    /// before its closing `}` (§3.2) — never compiled.
    pub end_tokens: bool,
}

/// One note the probe placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedNote {
    /// The definition's canonical id.
    pub id: String,
    /// The note's number.
    pub n: u32,
    /// The body's byte range in the probed copy, `{` to past `}`.
    pub body: (usize, usize),
    /// An `extern inline` definition (GNU's inline-only idiom): it may emit
    /// no symbol, so it explains none (fix pass 2's check).
    pub inline_only: bool,
}

/// A probed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    /// The file with a note after the opening `{` of every noted body.
    pub source: Vec<u8>,
    /// Definitions no note can go in, with the rule that says so, in source
    /// order, each id once (ids in [`ProbeOptions::skip`] are not listed).
    pub unwatched: Vec<(String, NoNote)>,
    /// The notes placed, in source order.
    pub notes: Vec<PlacedNote>,
}

/// An insertion into the copy: its position, its text, and — for a note —
/// the definition's id, number, `{` byte, body end and whether it is
/// `extern inline`.
type Insert = (usize, String, Option<(String, u32, usize, usize, bool)>);

/// Probe `source`, the file at `rel_path` (repo-relative, as the facts name
/// it). `index_of(canonical id)` is the note's number: the index of the
/// definition's `(file, id)` pair among the facts' distinct pairs. A
/// definition whose id `index_of` does not know is an error — the facts are
/// stale — never a guessed number.
pub fn probe_source(
    rel_path: &str,
    source: &[u8],
    index_of: &dyn Fn(&str) -> Option<u32>,
    options: ProbeOptions<'_>,
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
    let mut inserts: Vec<Insert> = Vec::new();
    let mut unwatched: Vec<(String, NoNote)> = Vec::new();
    for def in &defs {
        let id = def.canonical_id();
        let n = index_of(&id).ok_or_else(|| {
            Error::Invariant(format!(
                "{rel_path}: `{id}` is not in the facts — the facts are stale; scan again"
            ))
        })?;
        if options.skip.contains(&id) {
            continue;
        }
        // A misread twin inside another body never takes the note of a
        // definition of the same id the parser read whole (the map's check
        // for a compiled definition with no note covers the other way).
        if def.nested && defs.iter().any(|d| !d.nested && d.canonical_id() == id) {
            continue;
        }
        let placed = match def.note_at {
            Ok(_) if options.gcc && def.naked_head => Err(NoNote::Naked),
            other => other,
        };
        match placed {
            Ok((at, end)) => {
                let words: Vec<&str> = def
                    .signature
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .collect();
                let inline_only = words.contains(&"extern")
                    && words
                        .iter()
                        .any(|w| matches!(*w, "inline" | "__inline" | "__inline__"));
                inserts.push((at + 1, note(n), Some((id, n, at, end, inline_only))));
                if options.end_tokens && end > at + 1 && source.get(end - 1) == Some(&b'}') {
                    inserts.push((end - 1, format!(" __ruharness_end_{n} "), None));
                }
            }
            Err(why) if !unwatched.iter().any(|(u, _)| *u == id) => unwatched.push((id, why)),
            Err(_) => {}
        }
    }
    inserts.sort_by_key(|(at, _, _)| *at);
    // The copy's offset of an original byte: every insertion before it.
    let shift = |off: usize| -> usize {
        inserts
            .iter()
            .filter(|(at, _, _)| *at <= off)
            .map(|(_, text, _)| text.len())
            .sum::<usize>()
    };
    let notes: Vec<PlacedNote> = inserts
        .iter()
        .filter_map(|(_, _, placed)| placed.as_ref())
        .map(|(id, n, at, end, inline_only)| PlacedNote {
            id: id.clone(),
            n: *n,
            body: (at + shift(*at), end + shift(end - 1)),
            inline_only: *inline_only,
        })
        .collect();
    let mut out = Vec::with_capacity(source.len() + inserts.len() * 32);
    let mut from = 0;
    for (at, text, _) in &inserts {
        out.extend_from_slice(&source[from..*at]);
        out.extend_from_slice(text.as_bytes());
        from = *at;
    }
    out.extend_from_slice(&source[from..]);
    Ok(Probed {
        source: out,
        unwatched,
        notes,
    })
}

/// The note for id `n`: one store into the runtime's notes
/// (docs/FEATURES-PROBE-REDESIGN.md §3.6) — no call.
pub(crate) fn note(n: u32) -> String {
    format!("__ruharness_seen[{n}] = 1;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<'a>(pairs: &'a [&'a str]) -> impl Fn(&str) -> Option<u32> + 'a {
        move |id| pairs.iter().position(|p| *p == id).map(|i| i as u32)
    }

    fn probe_with(src: &str, known: &[&str], options: ProbeOptions<'_>) -> Probed {
        probe_source("src/a.c", src.as_bytes(), &ids(known), options).expect("probes")
    }

    fn probe(src: &str, known: &[&str]) -> Probed {
        probe_with(src, known, ProbeOptions::default())
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
        assert!(out.contains("{__ruharness_seen[0] = 1;\n  return x + 1;"));
        assert!(out.contains("int api(void) {__ruharness_seen[1] = 1; return helper(2); }"));
        assert!(p.unwatched.is_empty());
        // Removing the notes gives the original back, byte for byte.
        let stripped = out
            .replace("__ruharness_seen[0] = 1;", "")
            .replace("__ruharness_seen[1] = 1;", "");
        assert_eq!(stripped, src);
        // Each note's body range is the copy's `{ … }`.
        for note in &p.notes {
            let body = &out.as_bytes()[note.body.0..note.body.1];
            assert!(body.starts_with(b"{") && body.ends_with(b"}"), "{note:?}");
            assert!(String::from_utf8_lossy(body).contains(&super::note(note.n)));
        }
    }

    #[test]
    fn if_variants_share_an_id_and_k_and_r_definitions_are_watched() {
        let src = "#if A\nint f(void) { return 1; }\n#else\nint f(void) { return 2; }\n#endif\n\
                   int kr(a) int a; { return a; }\n";
        let p = probe(src, &["f", "kr"]);
        let out = text(&p);
        assert_eq!(out.matches("__ruharness_seen[0] = 1;").count(), 2, "{out}");
        assert!(
            out.contains("int kr(a) int a; {__ruharness_seen[1] = 1;"),
            "{out}"
        );
    }

    /// Rules 1 and 2 stay; what the removed rules used to guess — a parse
    /// error in the head, a leading pragma or macro, a naked function on
    /// clang — is now the compiler's to judge: the note is placed.
    #[test]
    fn only_the_hard_rules_keep_a_note_out() {
        let broken = "int good(void) { return 1; }\nint bad(int x,) { return x; }\n";
        let p = probe(broken, &["good", "bad"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        let src = "double f(double x) {\n#pragma STDC FENV_ACCESS ON\n  return x; }\n\
                   double g(double x) { FENV_ON\n  g2(x);\n  return x; }\n\
                   __attribute__((naked)) void h(void) { __asm__(\"ret\"); }\n\
                   int l(int x) { __label__ out; return x; }\n";
        let p = probe(src, &["f", "g", "h", "l"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        assert_eq!(p.notes.len(), 4);
        // Rule 4 is gcc's: a naked function stays unwatched there.
        let p = probe_with(
            src,
            &["f", "g", "h", "l"],
            ProbeOptions {
                gcc: true,
                ..ProbeOptions::default()
            },
        );
        assert_eq!(p.unwatched, [("h".to_string(), NoNote::Naked)]);
        // A parameter named `naked` is no attribute, on gcc too — nor a
        // K&R declaration of one (review: the mutation sweep).
        let gcc = ProbeOptions {
            gcc: true,
            ..ProbeOptions::default()
        };
        let p = probe_with(
            "int setmode(int naked) { return naked; }\n",
            &["setmode"],
            gcc,
        );
        assert!(p.unwatched.is_empty(), "{p:?}");
        let p = probe_with("int kr(naked) int naked; { return naked; }\n", &["kr"], gcc);
        assert!(p.unwatched.is_empty(), "{p:?}");
        // The reserved spelling is the same attribute.
        let p = probe_with(
            "__attribute__((__naked__)) void n(void) { __asm__(\"ret\"); }\n",
            &["n"],
            gcc,
        );
        assert_eq!(p.unwatched, [("n".to_string(), NoNote::Naked)], "{p:?}");
    }

    /// Premise re-run (sqlite3.c's winWrite): an `#if` whose branches each
    /// open a brace makes the parser run one body on over the definitions
    /// after it. They are recorded and unwatched (rule 1) — not lost — and
    /// a misread twin never takes the note of a definition read whole.
    #[test]
    fn definitions_a_misread_body_swallows_are_recorded_unwatched() {
        let src = "static int w(int rc) {\n#if defined(NEVER)\n  if (rc == 0) {\n#else\n  {\n#endif\n\
                   rc += 1;\n  }\n  return rc;\n}\n\
                   static int b(int x) { return x + 1; }\n\
                   #ifdef _WIN32\nint os_init(void) { return 1; }\n#endif\n\
                   static int d(int x) {\n  if (x) {\n    x++;\n#if defined(NEVER)\n  }\n#else\n  }\n#endif\n  return x;\n}\n\
                   int os_init(void) { return 0; }\n";
        let known = ["src/a.c::w", "src/a.c::b", "os_init", "src/a.c::d"];
        let p = probe(src, &known);
        let unwatched: Vec<&str> = p.unwatched.iter().map(|(id, _)| id.as_str()).collect();
        assert!(unwatched.contains(&"src/a.c::b"), "{p:?}");
        assert!(
            p.unwatched.iter().all(|(_, why)| *why == NoNote::Parser),
            "{p:?}"
        );
        assert!(
            !unwatched.contains(&"os_init"),
            "a misread twin demoted it: {p:?}"
        );
        assert!(p.notes.iter().any(|n| n.id == "os_init"), "{p:?}");
        // The misread body itself: its bounds are a guess.
        assert!(unwatched.contains(&"src/a.c::w"), "{p:?}");
    }

    /// Review (the mutation sweep): a head whose body a macro supplies runs
    /// on into the next definition's head, and the parser gives the first
    /// name the second body — no parse error. Such a definition is rule 1;
    /// heads with attributes, an `asm` label or a macro word after their
    /// parameters keep their notes.
    #[test]
    fn a_head_run_into_the_next_one_gets_no_note() {
        let p = probe(
            "int g(void) NOT_IMPLEMENTED\nint after(void) { return 1; }\n",
            &["g", "after"],
        );
        // Both heads recorded, both rule 1 (fix pass 1's check: the second
        // was lost from the facts).
        assert_eq!(
            p.unwatched,
            [
                ("g".to_string(), NoNote::Parser),
                ("after".to_string(), NoNote::Parser)
            ],
            "{p:?}"
        );
        assert!(p.notes.is_empty(), "{p:?}");
        // An annotation macro with arguments after the parameters is no
        // second head (fix pass 1's check), on the same line or the next.
        let fine = "int a(void) __attribute__((noinline)) { return 1; }\n\
                    void b(void) UNUSED_MACRO\n{ }\n\
                    int c(x) int x; { return x; }\n\
                    int d(void) ATTR(x) { return 1; }\n\
                    int e(void)\n  __acquires(lock)\n{ return 0; }\n\
                    int f(void) [[deprecated]] { return 1; }\n";
        let p = probe(fine, &["a", "b", "c", "d", "e", "f"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        assert_eq!(p.notes.len(), 6, "{p:?}");
    }

    /// The definitions the scanner records in `src` (as `src/a.c`).
    fn defs_of(src: &str) -> Vec<crate::FnDef> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("grammar");
        let tree = parser.parse(src, None).expect("parses");
        let mut defs = Vec::new();
        crate::collect_functions(tree.root_node(), src.as_bytes(), "src/a.c", &mut defs);
        defs
    }

    /// Fix pass 2's check: two heads run together in any layout — GNU's
    /// return type on its own line, one line, K&R, `static` on its own
    /// line, a `…_t` or a typedef'd type, a pointer, a parenthesized or
    /// nested name — are both recorded and both rule 1; the first head's
    /// note never lands in the second's body.
    #[test]
    fn two_heads_in_any_layout_are_both_recorded_and_unwatched() {
        let cases = [
            ("int g(void) NI\n\nint\nafter(void)\n{ return 1; }\n", false),
            ("int g(void) NI\n\nint\nafter()\n{ return 1; }\n", false),
            ("int g(void) NI int after(void) { return 1; }\n", false),
            ("int g(void) NI\nafter(x) int x; { return x; }\n", false),
            (
                "int g(void) NI\nstatic\nint after(void) { return 1; }\n",
                true,
            ),
            (
                "static int g(void) NI\n\nstatic int\nafter(int x, char *y)\n{ return x; }\n",
                true,
            ),
            (
                "int g(void) NI\nsize_t\nafter(size_t n, const char *s) { return 1; }\n",
                false,
            ),
            ("int g(void) NI\nchar *after(void) { return 0; }\n", false),
            ("int g(void) NI\nint (after)(void) { return 1; }\n", false),
            (
                "int g(void) NI\nvoid (*after(void))(int) { return 0; }\n",
                false,
            ),
            (
                "int g(void) NI\nmytype\nafter(int x)\n{ return x; }\n",
                false,
            ),
            // Fix pass 4's check: an empty parameter list after a typedef'd
            // or tag return type, or after an attribute.
            ("int g(void) NI\nCount\nafter()\n{ return 1; }\n", false),
            ("Count g(void) NI\n\nCount\nafter()\n{ return 1; }\n", false),
            (
                "int g(void) NI\nstatic Count after()\n{ return 1; }\n",
                true,
            ),
            ("int g(void) NI\nstruct s\nafter()\n{ return 1; }\n", false),
            ("int g(void) NI\nenum e after()\n{ return 1; }\n", false),
            (
                "int g(void) NOT_IMPL(-1)\nstatic Count\nafter()\n{ return 1; }\n",
                true,
            ),
            (
                "int g(void) NI\nint __attribute__((noinline))\nafter()\n{ return 1; }\n",
                false,
            ),
        ];
        for (src, second_static) in cases {
            let defs = defs_of(src);
            let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            assert_eq!(names, ["g", "after"], "{src}: {defs:?}");
            assert!(
                defs.iter().all(|d| d.note_at == Err(NoNote::Parser)),
                "{src}: {defs:?}"
            );
            assert_eq!(defs[1].is_static, second_static, "{src}");
            let g = if defs[0].is_static { "src/a.c::g" } else { "g" };
            let after = if second_static {
                "src/a.c::after"
            } else {
                "after"
            };
            let p = probe(src, &[g, after]);
            assert!(p.notes.is_empty(), "{src}: {p:?}");
        }
        // The second head's signature is a declaration.
        let defs = defs_of("int g(void) NI\nstatic\nint after(void) { return 1; }\n");
        assert_eq!(defs[1].signature, "static int after(void)");
        let defs = defs_of("int g(void) NI\nchar *after(void) { return 0; }\n");
        assert_eq!(defs[1].signature, "char *after(void)");
    }

    /// Fix pass 2's check: annotation macros keep the note — numbers, a
    /// single name, an address, nested calls — on the parameters' line or
    /// the next, after an annotation word or alone; no made-up function.
    #[test]
    fn annotations_after_the_parameters_keep_the_note() {
        for src in [
            "void say(const char *fmt, ...)\n    WARN_UNUSED PRINTF_LIKE(1, 2)\n{ }\n",
            "int e(void)\n  MACRO __acquires(lock)\n{ return 0; }\n",
            "int h(void) __releases(&l) __acquires(&l) { return 0; }\n",
            "int held(void) __must_hold(&lock) { return 0; }\n",
            "int n(char *p) NONNULL(1) { return 0; }\n",
            "int q(char *p)\n__nonnull((1))\n{ return 0; }\n",
            "int v(void)\nAPPLE_ARCHIVE_AVAILABLE(macos(11.0), ios(14.0))\n{ return 0; }\n",
            "int r(int i) __constant_range(i, 0, 3) { return i; }\n",
            // One annotation word, then an empty call: no second head.
            "int f(void) ATTR NAME() { return 1; }\n",
            "int f(void) __attribute__((cold)) NAME() { return 1; }\n",
        ] {
            let defs = defs_of(src);
            assert_eq!(defs.len(), 1, "{src}: {defs:?}");
            assert!(defs[0].note_at.is_ok(), "{src}: {defs:?}");
        }
    }

    /// Fix pass 2's check: a macro read as the declarator before the real
    /// name gives the definition the real name, rule 1 — never a function
    /// named after the macro, and two such definitions never share a note.
    #[test]
    fn a_macro_before_the_real_name_is_not_a_function() {
        let src = "static void * SIZED(size) alloc_a(int size) { return 0; }\n\
                   static void * SIZED(size) alloc_b(int size) { return 0; }\n\
                   static void\npg_attribute_unused()\nRT_DUMP_NODE(RT_NODE * node)\n{ }\n";
        let defs = defs_of(src);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["alloc_a", "alloc_b", "RT_DUMP_NODE"], "{defs:?}");
        assert!(
            defs.iter()
                .all(|d| d.note_at == Err(NoNote::Parser) && d.is_static),
            "{defs:?}"
        );
    }

    #[test]
    fn a_parameter_list_is_told_from_an_annotations_arguments() {
        for args in [
            "(void)",
            "(int size)",
            "(size_t n, const char *s)",
            "(RT_NODE * node)",
            "(struct s *p)",
            "(T buf[4])",
            "(unsigned)",
        ] {
            assert!(crate::decl_shaped(args), "{args}");
        }
        for args in [
            "()",
            "(1, 2)",
            "(lock)",
            "(&lock)",
            "((1))",
            "(macos(11.0), ios(14.0))",
            "(printf, 1, 2)",
            "(\"za\")",
            "(x)",
            "(...)",
        ] {
            assert!(!crate::decl_shaped(args), "{args}");
        }
    }

    /// Fix pass 2's check: a C23 attribute inside the declarator keeps the
    /// definition and its note; a parameter named `naked` is no attribute.
    #[test]
    fn a_c23_attribute_inside_the_declarator_keeps_the_note() {
        let src = "int f [[gnu::cold]] (void) { return 0; }\n\
                   static int c [[maybe_unused]] (int x) { return x; }\n\
                   int *d [[gnu::cold]] (void) { return 0; }\n\
                   int h(int naked) [[gnu::cold]] { return naked; }\n";
        let defs = defs_of(src);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["f", "c", "d", "h"], "{defs:?}");
        assert!(
            defs.iter().all(|d| d.note_at.is_ok() && !d.naked_head),
            "{defs:?}"
        );
    }

    /// Fix pass 3's check: a body macro with arguments before the next head
    /// (`NOT_IMPL(-1)`, `STUB(int)`) never takes the second head's name; a
    /// K&R second head with typedef'd types is caught by the parse error it
    /// leaves; a nested second head keeps its `static` and whole signature.
    #[test]
    fn the_real_second_head_is_named_in_every_layout() {
        for src in [
            "int g(void) NOT_IMPL(-1)\nint after(void) { return 1; }\n",
            "int g(void) NOT_IMPL(-1)\n\nstatic int\nafter(void)\n{ return 1; }\n",
            "int g(void) STUB(int)\nint after(void) { return 1; }\n",
            "int g(void) NI\nCount\nafter(x)\n\tCount x;\n{ return x; }\n",
            // Fix pass 4's check: the last call that qualifies by its
            // arguments, not the first (`STUB(int)` reads as parameters).
            "int g(void) STUB(int)\nmytype\nafter(int x)\n{ return x; }\n",
            // A K&R parameter of function type is declared, not a head.
            "int g(void) NI\nCount\nafter(cb)\n\tint cb(int);\n{ return cb(1); }\n",
            "int g(void) NI\nstatic Count\nafter(cb)\n\tint cb(int);\n{ return cb(1); }\n",
        ] {
            let defs = defs_of(src);
            let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            assert_eq!(names, ["g", "after"], "{src}: {defs:?}");
            assert!(
                defs.iter().all(|d| d.note_at == Err(NoNote::Parser)),
                "{src}: {defs:?}"
            );
            assert_eq!(defs[1].is_static, src.contains("static"), "{src}");
        }
        for (src, signature) in [
            (
                "int g(void) NI\nstatic int (after)(void) { return 1; }\n",
                "static int (after)(void)",
            ),
            (
                "int g(void) NI\nstatic void (*after(int x))(int) { return 0; }\n",
                "static void (*after(int x))(int)",
            ),
            (
                "int g(void) NI\nunsigned long (after)(void) { return 1; }\n",
                "unsigned long (after)(void)",
            ),
            // Fix pass 4's check: a typedef'd return type before a
            // parenthesized name.
            (
                "int g(void) NI\nstatic Count (after)(void) { return 1; }\n",
                "static Count (after)(void)",
            ),
            (
                "int g(void) NI\nstatic Count (*after(int x))(int) { return 0; }\n",
                "static Count (*after(int x))(int)",
            ),
            (
                "int g(void) NI\nsize_t (after)(void) { return 1; }\n",
                "size_t (after)(void)",
            ),
            (
                "int g(void) NI\nCount (after)(void) { return 1; }\n",
                "Count (after)(void)",
            ),
            (
                "int g(void) NOT_IMPL(-1)\nstatic Count (after)(void) { return 1; }\n",
                "static Count (after)(void)",
            ),
            // A head that is not nested ends at its call, whatever follows.
            (
                "int g(void) NI\nint after(void) ATTR\n{ return 1; }\n",
                "int after(void)",
            ),
            (
                "int g(void) NI\nint after(x) int x; { return x; }\n",
                "int after(x)",
            ),
        ] {
            let defs = defs_of(src);
            assert_eq!(defs.len(), 2, "{src}: {defs:?}");
            assert_eq!(defs[1].name, "after");
            assert_eq!(defs[1].signature, signature, "{src}");
            assert_eq!(defs[1].is_static, signature.starts_with("static"), "{src}");
        }
    }

    /// Fix pass 4's mutation check: a stray parse error between a head and
    /// its body, with no call to name, still makes the definition rule 1.
    #[test]
    fn a_stray_parse_error_before_the_body_is_rule_1() {
        let src = "int g(void) NI\nCount\nafter(x)\n\tOther y;\n{ return 0; }\n";
        let defs = defs_of(src);
        // The call's arguments are not named after it: no second head.
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["g"], "{defs:?}");
        assert!(
            defs.iter().all(|d| d.note_at == Err(NoNote::Parser)),
            "{defs:?}"
        );
    }

    /// Fix pass 3's check: an annotation naming the function's own parameters
    /// (`__sized_by(n * size)`) is no second head; a macro before the real
    /// name with attributes between (macOS's malloc headers) gives one static
    /// definition under the real name; a macro-made `PREFIX(name)(params)` is
    /// not recorded under the macro's name.
    #[test]
    fn annotations_and_name_macros_make_no_function() {
        for src in [
            "void *alloc_n(size_t n, size_t size) __sized_by(n * size) { return h(n); }\n",
            "void *alloc_n(size_t count, size_t size)\n    __sized_by(count*size)\n{ return h(count); }\n",
        ] {
            let defs = defs_of(src);
            assert_eq!(defs.len(), 1, "{src}: {defs:?}");
            assert_eq!(defs[0].name, "alloc_n");
            assert!(defs[0].note_at.is_ok(), "{src}: {defs:?}");
        }
        for src in [
            "static void * SIZED(size) __attribute__((always_inline)) alloc_a(int size) { return 0; }\n",
            "static void * __sized_by(count * size) my_calloc(int count, int size) { return 0; }\n",
        ] {
            let defs = defs_of(src);
            let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            assert_eq!(names.len(), 1, "{src}: {defs:?}");
            assert!(names[0] == "alloc_a" || names[0] == "my_calloc", "{src}: {defs:?}");
            assert!(defs[0].is_static, "{src}: {defs:?}");
        }
        let src = "static int PREFIX(scanLit)(int open, const char *p) { return 0; }\n\
                   static int PREFIX(scanRef)(int open, const char *p) { return 1; }\n";
        assert!(defs_of(src).is_empty(), "{:?}", defs_of(src));
    }

    /// Fix pass 1's check: a definition inside a body the parser read
    /// whole — a GNU nested function, a statement macro misread after an
    /// `#endif` — is no file-scope function; the body keeps its note.
    #[test]
    fn only_a_misread_body_holds_definitions() {
        for src in [
            "int outer(int x) {\n  int inner(int y) { return y + 1; }\n  return inner(x);\n}\n",
            "int k(int z) {\n  if (z) {\n    z = 1;\n  }\n#ifndef OMIT\n  else LOOP_MACRO(z) {\n    z = 2;\n  }\n#endif\n  return z;\n}\n",
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_c::LANGUAGE.into())
                .expect("grammar");
            let tree = parser.parse(src, None).expect("parses");
            let mut defs = Vec::new();
            crate::collect_functions(tree.root_node(), src.as_bytes(), "src/a.c", &mut defs);
            assert_eq!(defs.len(), 1, "{src}: {defs:?}");
            assert!(defs[0].note_at.is_ok(), "{src}: {defs:?}");
        }
        assert!(crate::body_misread(
            b"{\n#if A\n if (x) {\n#else\n {\n#endif\n y();\n}\n"
        ));
        assert!(crate::body_misread(
            b"{ if (x) {\n#if A\n }\n#else\n }\n#endif\n"
        ));
        assert!(!crate::body_misread(
            b"{\n#if A\n f(\"{\"); /* { */\n#else\n g();\n#endif\n}\n"
        ));
        assert!(!crate::body_misread(
            b"{\n#ifdef A\n if (x) { y(); }\n#else\n z();\n#endif\n}\n"
        ));
        // Fix pass 2's check: groups whose excess braces cancel inside the
        // body (a lock taken in one #if, released in a later one) are read
        // right — no descent, the body keeps its note.
        let balanced = "int k(int z) {\n#if USE_LOCK\n  if (lock()) {\n#else\n  {\n#endif\n\
                        int inner(int y) { return y + 1; }\n    z = inner(z);\n\
                        #if USE_LOCK\n    unlock(); }\n#else\n  }\n#endif\n  return z;\n}\n";
        assert!(!crate::body_misread(
            &balanced.as_bytes()[balanced.find('{').expect("body")..]
        ));
        // Fix pass 3's check: an `#else` that closes the function and opens
        // another is misread, though the first branch reads right.
        let mis2 = "int f(int x) {\n#ifdef A\n  if (x) {\n#else\n  return 0;\n}\n\
                    static int helper(int y) {\n  if (y) {\n#endif\n    g++;\n  }\n  return 0;\n}\n";
        assert!(crate::body_misread(
            &mis2.as_bytes()[mis2.find('{').expect("body")..]
        ));
        let names: Vec<String> = defs_of(mis2).into_iter().map(|d| d.name).collect();
        assert!(names.contains(&"helper".to_string()), "{names:?}");
        // Fix pass 4's check: the same shape with its closing group, which
        // the parser reads without an error: helper is found in f's body.
        let mis3 = "int f(int x) {\n#ifdef A\n  if (x) {\n#else\n  return 0; }\n\
                    static int helper(int y) { if (y) {\n#endif\n    g++;\n#ifdef A\n  }\n\
                    #else\n  }\n#endif\n  return 0;\n}\n";
        let defs3 = defs_of(mis3);
        let f = defs3.iter().find(|d| d.name == "f").expect("f");
        assert_eq!(f.note_at, Err(NoNote::Parser), "{defs3:?}");
        let helper = defs3.iter().find(|d| d.name == "helper").expect("helper");
        assert!(helper.nested && helper.is_static, "{defs3:?}");
        // A body the parser bounded wrongly for another reason (no `#if`:
        // mimalloc's `if mi_likely(x) {`) is no misread `#if` body.
        assert!(!crate::body_misread(b"{ if mi_likely(x) {\n y();\n }\n"));
        let defs = defs_of(balanced);
        assert_eq!(defs.len(), 1, "{defs:?}");
        assert!(defs[0].note_at.is_ok(), "{defs:?}");
    }

    /// Fix pass 4's check: a misread body is read in every combination of
    /// branches, not only branch k of every group: an `#ifndef A` beside an
    /// `#ifdef A` (branch 0 is the other build), or two macros of their own,
    /// where the build that closes the function takes branch 0 of one group
    /// and branch 1 of another.
    #[test]
    fn a_misread_body_is_read_in_every_combination_of_branches() {
        let inverted = "int f(int x) {\n#ifndef A\n  g += 0;\n#else\n  if (x > 5) {\n#endif\n\
                        #ifdef A\n  if (x) {\n#else\n  return 0; }\n\
                        static int helper(int y) { if (y) {\n#endif\n    g++;\n\
                        #ifdef A\n  }\n#else\n  }\n#endif\n#ifndef A\n  g--;\n#else\n  }\n#endif\n\
                        return 0;\n}\n";
        let two_macros = "int f(int x) {\n#ifdef A\n  g += 0;\n#else\n  if (x > 5) {\n#endif\n\
                          #ifdef B\n  if (x) {\n#else\n  return 0; }\n\
                          static int helper(int y) { if (y) {\n#endif\n    g++;\n\
                          #ifdef B\n  }\n#else\n  }\n#endif\n#ifdef A\n  g--;\n#else\n  }\n#endif\n\
                          return 0;\n}\n";
        for src in [inverted, two_macros] {
            assert!(
                crate::body_misread(&src.as_bytes()[src.find('{').expect("body")..]),
                "{src}"
            );
            let p = probe(src, &["f", "src/a.c::helper"]);
            assert!(
                p.unwatched
                    .contains(&("src/a.c::helper".to_string(), NoNote::Parser)),
                "{src}: {p:?}"
            );
            assert!(
                p.unwatched.contains(&("f".to_string(), NoNote::Parser)),
                "{src}: {p:?}"
            );
        }
        // Past the cap of combinations a body counts as misread.
        let mut many = String::from("{\n#if A\n if (x) {\n#else\n {\n#endif\n");
        for k in 0..13 {
            many.push_str(&format!("#if B{k}\n {{ }}\n#endif\n"));
        }
        many.push_str(" }\n}\n");
        assert!(crate::body_misread(many.as_bytes()));
    }

    /// Fix pass 4's check: a second head the parser folds behind a parse
    /// error (`**`, `* const *`, or one `*` after a typedef'd or struct
    /// return type) keeps its own storage class — never the first head's —
    /// and its signature starts at its own head.
    #[test]
    fn a_folded_second_head_keeps_its_own_static() {
        let cases = [
            (
                "int g(void) NI\nstatic char **after(void)\n{ return 0; }\n",
                "after",
                true,
                "static char **after(void)",
            ),
            (
                "int g(void) NI\nstatic Count *\nafter(int x)\n{ return h(); }\n",
                "after",
                true,
                "static Count * after(int x)",
            ),
            (
                "int g(void) NI\nstatic Count *after(int x)\n{ return h(); }\n",
                "after",
                true,
                "static Count *after(int x)",
            ),
            (
                "int g(void) NI\nstatic struct node *\nnext_node(struct node *n)\n{ return n; }\n",
                "next_node",
                true,
                "static struct node * next_node(struct node *n)",
            ),
            (
                "int g(void) NI\nstatic char * const *after(void)\n{ return 0; }\n",
                "after",
                true,
                "static char * const *after(void)",
            ),
            (
                "static int g(void) NI\nstruct node *\nnext_node(struct node *n)\n{ return n; }\n",
                "next_node",
                false,
                "struct node * next_node(struct node *n)",
            ),
            (
                "static int g(void) NI\nchar **after(void)\n{ return 0; }\n",
                "after",
                false,
                "char **after(void)",
            ),
            (
                "int g(void) NI\nCount *after(int x)\n{ return h(); }\n",
                "after",
                false,
                "Count *after(int x)",
            ),
            // An attribute macro is no first head: the `static` before it is
            // the definition's.
            (
                "static int M(x) **after(int x)\n{ return 0; }\n",
                "after",
                true,
                "static int M(x) **after(int x)",
            ),
        ];
        for (src, name, is_static, signature) in cases {
            let defs = defs_of(src);
            let d = defs
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("{src}: {defs:?}"));
            assert_eq!(d.is_static, is_static, "{src}: {defs:?}");
            assert_eq!(d.signature, signature, "{src}");
        }
    }

    /// The real-code re-run (sqlite3.c, 23 times): an `else if (` right
    /// after an `#ifndef` is read as a definition named `if`. A keyword is
    /// never a function's name — not in the facts, and the body it sits in
    /// keeps its note.
    #[test]
    fn a_keyword_is_never_a_function() {
        let src = "static int k(int z) {\n  if( z>0 ){\n    z = 1;\n  }\n#ifndef OMIT\n\
                   else if( z<0 ){\n    z = 2;\n  }\n#endif\n  return z;\n}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("grammar");
        let tree = parser.parse(src, None).expect("parses");
        let mut defs = Vec::new();
        crate::collect_functions(tree.root_node(), src.as_bytes(), "src/a.c", &mut defs);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["k"], "{defs:?}");
        let p = probe(src, &["src/a.c::k"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        assert_eq!(p.notes.len(), 1);
    }

    /// Rule 3 in each directive spelling, with the depth form: only a `{`
    /// in a group the head is not wholly in.
    #[test]
    fn a_brace_in_a_conditional_group_of_its_own_is_unwatched() {
        let conditional = [
            "int s1(int a)\n#if X\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            "int s2(int a)\n#ifdef TRACE\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            "int s3(int a)\n#ifndef FAST\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            "int s4(int a)\n#  ifdef TRACE\n{ return a; }\n#  else\n{ return -a; }\n#  endif\n",
            "int s5(int a)\n/* trace */ #ifdef TRACE\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            "int s6(int a)\n%:ifdef TRACE\n{ return a; }\n%:else\n{ return -a; }\n%:endif\n",
            "int s7(int a)\n#\\\nifdef TRACE\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            // Review (mutation sweep): a comment between `#` and the word,
            // and a CRLF line splice.
            "int s8(int a)\n#/*c*/ifdef TRACE\n{ return a; }\n#else\n{ return -a; }\n#endif\n",
            "int s9(int a)\r\n#\\\r\nifdef TRACE\r\n{ return a; }\r\n#else\r\n{ return -a; }\r\n#endif\r\n",
        ];
        for src in conditional {
            let name = &src[src.find("s").unwrap()..][..2];
            let p = probe(src, &[name]);
            assert_eq!(
                p.unwatched,
                [(name.to_string(), NoNote::ConditionalBrace)],
                "{src}: {p:?}"
            );
            assert!(
                p.notes.is_empty(),
                "{src}: a note on one branch only: {}",
                text(&p)
            );
        }
        // Two heads, one body (Cython's module init): the parser reads the
        // second head into the first's — never a note in a body one build
        // gives the other name.
        let src = "#if PY_MAJOR_VERSION >= 3\nPyMODINIT_FUNC PyInit_m(void)\n#else\n\
                   PyMODINIT_FUNC initm(void)\n#endif\n{ return 0; }\n";
        let p = probe(src, &["PyInit_m", "initm"]);
        assert_eq!(
            p.unwatched.len(),
            2,
            "both names recorded, unwatched: {p:?}"
        );
        assert!(p.notes.is_empty(), "{p:?}");
        // Wholly in the head: watched.
        let watched = [
            "int w1(\n#ifdef WIDE\n long a\n#else\n int a\n#endif\n) { return (int)a; }\n",
            "#ifdef _WIN32\n__declspec(dllexport)\n#endif\nint w2(int a) { return a; }\n",
            "int w4(void) // fixes #42\n{\n  return 0;\n}\n",
        ];
        for src in watched {
            let name = &src[src.find("w").unwrap()..][..2];
            let p = probe(src, &[name]);
            assert_eq!(p.notes.len(), 1, "{src}: {p:?}\n{}", text(&p));
        }
    }

    /// §4, rule 1, review (mutation sweep: the rule had no test): an
    /// `#ifdef` inside an initializer (signal-hook's extract.c) leaves the
    /// parser in an error that holds every later definition — their bodies'
    /// bounds are a guess, in either configuration.
    #[test]
    fn definitions_under_a_parse_error_are_rule_1() {
        let tail = "struct C cs[] = {\n#ifdef X\n    { 1, 2 },\n#endif\n    { 3, 4 },\n};\n\
                    int later(int x) { return x + cs[0].a; }\n\
                    int last(int x) { return x; }\n";
        for head in ["", "#define X 1\n"] {
            let src = format!("{head}struct C {{ int a; int b; }};\n{tail}");
            let p = probe(&src, &["later", "last"]);
            assert_eq!(
                p.unwatched,
                [
                    ("later".to_string(), NoNote::Parser),
                    ("last".to_string(), NoNote::Parser)
                ],
                "{head:?}: {p:?}"
            );
            assert!(p.notes.is_empty());
        }
        // An ERROR inside a definition's own head is not one around it.
        let p = probe(
            "int bad(int x,) { return x; }\nint good(void) { return 1; }\n",
            &["bad", "good"],
        );
        assert!(p.unwatched.is_empty(), "{p:?}");
    }

    #[test]
    fn the_scanner_reads_directives_as_the_preprocessor_does() {
        use crate::brace_is_conditional as c;
        assert!(c(b"f(int a)\n#ifdef X\n"));
        assert!(c(b"f(int a)\n  /* x */  #  if X\n"));
        assert!(c(b"f(int a)\n%:if X\n"));
        assert!(c(b"f(int a)\n#\\\nif X\n"));
        assert!(c(b"f(int a)\r#if X\r"));
        assert!(c(b"f(int a)\n#else\n"), "an outer group's #else");
        assert!(c(b"f(int a)\n#endif\n"), "an outer group's #endif");
        // Review (mutation sweep): the #elif forms, a comment after `#`, a
        // CRLF splice, and an open #if after a string holding `/*`.
        assert!(c(b"f(int a)\n#elif Y\n"));
        assert!(c(b"f(int a)\n#elifdef Y\n"));
        assert!(c(b"f(int a)\n#elifndef Y\n"));
        assert!(c(b"f(int a)\n#/*c*/ifdef X\n"));
        assert!(c(b"f(int a)\r\n#\\\r\nif X\r\n"));
        assert!(c(b"f(int a) __attribute__((section(\"/*\")))\n#if X\n"));
        assert!(!c(b"f(\n#if X\nint a\n#endif\n)\n"));
        assert!(!c(b"f(int a) /* #if X */\n"));
        assert!(!c(b"f(int a) // #if X\n"));
        assert!(!c(b"f(int a) /* a comment\n#if X\n*/\n"));
        assert!(!c(
            b"f(int a) __attribute__((section(\"/*\")))\n#if X\n#endif\n"
        ));
    }

    #[test]
    fn the_listing_copy_carries_end_tokens_and_the_skip_list_is_honoured() {
        let src = "int a(void) { return 1; }\nint b(void) {\n  return 2;\n}\n";
        let p = probe_with(
            src,
            &["a", "b"],
            ProbeOptions {
                end_tokens: true,
                ..ProbeOptions::default()
            },
        );
        let out = text(&p);
        assert!(
            out.contains("{__ruharness_seen[0] = 1; return 1;  __ruharness_end_0 }"),
            "{out}"
        );
        assert!(out.contains("return 2;\n __ruharness_end_1 }"), "{out}");
        assert_eq!(out.lines().count(), src.lines().count());
        let skip = vec!["a".to_string()];
        let p = probe_with(
            src,
            &["a", "b"],
            ProbeOptions {
                skip: &skip,
                ..ProbeOptions::default()
            },
        );
        assert_eq!(p.notes.len(), 1);
        assert_eq!(p.notes[0].id, "b");
        assert!(
            p.unwatched.is_empty(),
            "a skipped id is the caller's to list"
        );
    }

    #[test]
    fn two_functions_of_one_name_in_two_files_get_their_own_notes() {
        let known = ["src/a.c::helper", "src/b.c::helper"];
        let none = ProbeOptions::default();
        let a = probe_source(
            "src/a.c",
            b"static void helper(void) {}\n",
            &ids(&known),
            none,
        )
        .unwrap();
        let b = probe_source(
            "src/b.c",
            b"static void helper(void) {}\n",
            &ids(&known),
            none,
        )
        .unwrap();
        assert!(text(&a).contains("__ruharness_seen[0] = 1;"));
        assert!(text(&b).contains("__ruharness_seen[1] = 1;"));
    }

    #[test]
    fn a_definition_the_facts_do_not_know_is_an_error() {
        let err = probe_source(
            "src/a.c",
            b"int newer(void) { return 0; }\n",
            &|_: &str| None,
            ProbeOptions::default(),
        )
        .expect_err("stale facts");
        assert!(err.to_string().contains("the facts are stale"), "{err}");
    }

    #[test]
    fn macro_made_definitions_get_nothing() {
        // A macro that expands to a definition is invisible (the facts do
        // not know it either). The definition right after its invocation
        // keeps its own body and gets its note: whether that compiles is the
        // compiler's to say.
        let src =
            "#define DEF(n) int n(void) { return 0; }\nDEF(made)\nint real(void) { return 1; }\n";
        let p = probe(src, &["real"]);
        assert!(p.unwatched.is_empty(), "{p:?}");
        assert!(
            text(&p).contains("int real(void) {__ruharness_seen[0] = 1;"),
            "{}",
            text(&p)
        );
        let src =
            "#define DEF(n) int n(void) { return 0; }\nDEF(made);\nint real(void) { return 1; }\n";
        let p = probe(src, &["real"]);
        assert!(
            text(&p).contains("int real(void) {__ruharness_seen[0] = 1;"),
            "{}",
            text(&p)
        );
        assert_eq!(text(&p).matches("__ruharness_seen").count(), 1);
    }
}
