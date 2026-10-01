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
/// the definition's id, number, `{` byte and body end.
type Insert = (usize, String, Option<(String, u32, usize, usize)>);

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
                inserts.push((at + 1, note(n), Some((id, n, at, end))));
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
        .map(|(id, n, at, end)| PlacedNote {
            id: id.clone(),
            n: *n,
            body: (at + shift(*at), end + shift(end - 1)),
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
        // A parameter named `naked` is no attribute, on gcc too.
        let p = probe_with(
            "int setmode(int naked) { return naked; }\n",
            &["setmode"],
            ProbeOptions {
                gcc: true,
                ..ProbeOptions::default()
            },
        );
        assert!(p.unwatched.is_empty(), "{p:?}");
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
            // two heads, one body
            // two heads, one body — the same name or not (another name would
            // take this body's note in the other build)
            "#if X\nstatic int s8(int a)\n#else\nstatic int s8(long a)\n#endif\n{ return (int)a; }\n",
            "#if X\nstatic int s9(int a)\n#else\nstatic int s9(int a)\n#endif\n{ return a; }\n",
        ];
        for src in conditional {
            let name = &src[src.find("s").unwrap()..][..2];
            let p = probe(src, &[name]);
            if !p.unwatched.is_empty() || p.notes.is_empty() {
                assert!(
                    p.unwatched.is_empty()
                        || p.unwatched == [(name.to_string(), NoNote::ConditionalBrace)]
                        || p.unwatched == [(name.to_string(), NoNote::Parser)],
                    "{src}: {p:?}"
                );
            }
            assert!(
                p.notes.is_empty(),
                "{src}: a note on one branch only: {}",
                text(&p)
            );
        }
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
