//! Tests of the analysis over hand-built facts (docs/PROJECT-MAP-DESIGN.md
//! §4, "The walk and the map"): no compiler, a fake linker.

use super::*;
use crate::projectmap::{Compiled, DefinedSymbol, NeededSymbol, Reason};

/// A compiled `.c` defining `defs` and needing `needs`. A definition is
/// `name` (a strong function) or `name:weak`, `name:common`, `name:data`;
/// a need is `name` or `name:weak`.
fn c(path: &str, defs: &[&str], needs: &[&str]) -> FileFacts {
    let mut defined: Vec<DefinedSymbol> = defs
        .iter()
        .map(|d| {
            let (name, tag) = d.split_once(':').unwrap_or((d, ""));
            DefinedSymbol {
                name: name.to_string(),
                kind: match tag {
                    "common" => "common",
                    "data" => "data",
                    _ => "function",
                },
                weak: tag == "weak",
            }
        })
        .collect();
    defined.sort_by(|a, b| a.name.cmp(&b.name));
    let mut needed: Vec<NeededSymbol> = needs
        .iter()
        .map(|n| {
            let (name, tag) = n.split_once(':').unwrap_or((n, ""));
            NeededSymbol {
                name: name.to_string(),
                weak: tag == "weak",
            }
        })
        .collect();
    needed.sort_by(|a, b| a.name.cmp(&b.name));
    FileFacts {
        path: path.to_string(),
        aliases: Vec::new(),
        kind: FileKind::C,
        bytes: 1,
        blake3: String::new(),
        parsed: true,
        too_large: false,
        not_utf8: false,
        functions: 0,
        includes: Vec::new(),
        include_dirs: Vec::new(),
        ambiguous: Vec::new(),
        compiled: Some(Compiled::Ok),
        outside_includes: false,
        included_other: Vec::new(),
        defined,
        needed,
        odd_names: 0,
        withheld_names: 0,
        message: None,
    }
}

/// A `.c` that did not compile (no symbols).
fn failed(path: &str) -> FileFacts {
    let mut f = c(path, &[], &[]);
    f.compiled = Some(Compiled::Failed {
        reason: Reason::Syntax,
        header: None,
        detail: None,
        at: None,
    });
    f
}

/// Symbols the fake system provides.
const SYSTEM: [&str; 5] = ["printf", "malloc", "free", "fopen", "log"];

/// Links when nothing is doubled and every unresolved name is the system's.
#[derive(Default)]
struct Fake {
    calls: usize,
}

impl Linker for Fake {
    fn link(&mut self, files: &[&FileFacts], unresolved: &[String]) -> Result<Linked, Error> {
        self.calls += 1;
        let missing: Vec<String> = unresolved
            .iter()
            .filter(|s| !SYSTEM.contains(&s.as_str()))
            .cloned()
            .collect();
        let doubled: Vec<String> = doubled(files).into_keys().collect();
        Ok(if missing.is_empty() && doubled.is_empty() {
            Linked::Ok
        } else {
            Linked::Failed { missing, doubled }
        })
    }
}

struct Run {
    files: Vec<FileFacts>,
    parser: BTreeMap<String, ParserFacts>,
    issues: Vec<WalkIssue>,
    accepted: Vec<(String, String)>,
}

impl Run {
    fn new(files: Vec<FileFacts>) -> Run {
        Run {
            files,
            parser: BTreeMap::new(),
            issues: Vec::new(),
            accepted: Vec::new(),
        }
    }

    fn input(&self) -> Input<'_> {
        Input {
            files: &self.files,
            parser: &self.parser,
            walk_issues: &self.issues,
            accepted: &self.accepted,
        }
    }

    fn bare(&self) -> Analysis {
        analyze(&self.input(), None).expect("no linker, no error")
    }

    fn linked(&self) -> (Analysis, usize) {
        let mut fake = Fake::default();
        let a = analyze(&self.input(), Some(&mut fake)).expect("the fake never fails");
        (a, fake.calls)
    }
}

fn closure<'a>(a: &'a Analysis, id: &str) -> &'a Closure {
    a.closures
        .iter()
        .find(|c| c.program == id)
        .unwrap_or_else(|| panic!("a closure for {id}: {:?}", a.closures))
}

fn program<'a>(a: &'a Analysis, id: &str) -> &'a Program {
    a.programs
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("program {id}: {:?}", a.programs))
}

/// Three `main()`s, a duplicate and a test.
fn three_mains() -> Vec<FileFacts> {
    vec![
        c("tools/a.c", &["main"], &["shared_fn", "x", "printf"]),
        c("tools/b.c", &["main"], &["y"]),
        c("tests/t.c", &["main"], &["shared_fn", "helper"]),
        c("lib/shared.c", &["shared_fn"], &[]),
        c("lib/y.c", &["y"], &[]),
        c("lib/x1.c", &["x"], &["helper"]),
        c("lib/x2.c", &["x"], &["nowhere"]),
        c("lib/helper.c", &["helper"], &[]),
    ]
}

#[test]
fn three_mains_with_a_duplicate_and_a_test() {
    let run = Run::new(three_mains());
    // Without linking: the duplicate is pending, nothing added for it.
    let bare = run.bare();
    let ids: Vec<&str> = bare.programs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["t-a", "t-b", "t-t"]);
    assert_eq!(program(&bare, "t-a").kind_guess, KindGuess::Tool);
    assert_eq!(program(&bare, "t-t").kind_guess, KindGuess::Test);
    assert_eq!(program(&bare, "t-a").index.as_deref(), Some("p2"));
    assert_eq!(program(&bare, "t-t").index.as_deref(), Some("p1"));
    let a = closure(&bare, "t-a");
    assert_eq!(a.files, ["lib/shared.c", "tools/a.c"]);
    assert!(a.incomplete);
    assert_eq!(a.duplicates.len(), 1);
    assert_eq!(a.duplicates[0].set, "d1");
    assert_eq!(a.duplicates[0].symbols, ["x"]);
    assert_eq!(a.outside, ["printf"]);
    // helper is shared only once the choice pulls it into t-a.
    let shared: Vec<&str> = bare.shared.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(shared, ["lib/shared.c"]);

    // Linking settles it: only x1 links.
    let (linked, _) = run.linked();
    let a = closure(&linked, "t-a");
    assert_eq!(
        a.files,
        ["lib/helper.c", "lib/shared.c", "lib/x1.c", "tools/a.c"]
    );
    assert!(!a.incomplete, "{:?}", a.incomplete_why);
    assert_eq!(a.linked, Some(Linked::Ok));
    let d = &a.duplicates[0];
    assert_eq!(
        d.choice,
        Some(Choice {
            keep: "d1.1".into(),
            by: "links"
        })
    );
    assert_eq!(d.links, ["d1.1"]);
    assert!(a.questions.is_empty());
    let shared: Vec<(&str, Vec<&str>)> = linked
        .shared
        .iter()
        .map(|s| {
            (
                s.file.as_str(),
                s.programs.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        shared,
        [
            ("lib/helper.c", vec!["t-a", "t-t"]),
            ("lib/shared.c", vec!["t-a", "t-t"])
        ]
    );
    // Neither alternative is a library.
    assert!(linked.libraries.is_empty(), "{:?}", linked.libraries);
}

#[test]
fn a_collision_nothing_needed_is_reported() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["f"]),
        c("f.c", &["f", "g"], &["h"]),
        c("h.c", &["h", "g"], &[]),
    ]);
    let a = run.bare();
    let cl = closure(&a, "t-main");
    assert_eq!(cl.files, ["f.c", "h.c", "main.c"]);
    assert_eq!(
        cl.collisions,
        [Collision {
            sym: "g".into(),
            definers: vec!["f.c".into(), "h.c".into()]
        }]
    );
    assert!(cl.duplicates.is_empty());
}

#[test]
fn a_definer_already_in_the_closure_meets_the_need() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["f", "s"]),
        c("f.c", &["f", "s"], &[]),
        c("other.c", &["s"], &[]),
    ]);
    let (a, _) = run.linked();
    let cl = closure(&a, "t-main");
    assert_eq!(cl.files, ["f.c", "main.c"]);
    assert!(cl.duplicates.is_empty());
    assert!(cl.questions.is_empty());
    assert!(!cl.incomplete);
}

#[test]
fn two_definers_pulled_in_for_other_symbols_collide() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["p", "q", "s"]),
        c("p.c", &["p", "s"], &[]),
        c("q.c", &["q", "s"], &[]),
    ]);
    let (a, _) = run.linked();
    let cl = closure(&a, "t-main");
    assert_eq!(cl.files, ["main.c", "p.c", "q.c"]);
    assert!(cl.duplicates.is_empty());
    assert_eq!(cl.collisions.len(), 1);
    assert_eq!(cl.collisions[0].sym, "s");
    assert_eq!(
        cl.linked,
        Some(Linked::Failed {
            missing: vec![],
            doubled: vec!["s".into()]
        })
    );
}

#[test]
fn weak_beside_strong_and_two_weak_are_no_duplicate() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["w", "v"]),
        c("w1.c", &["w:weak"], &[]),
        c("w2.c", &["w"], &[]),
        c("v1.c", &["v:weak"], &[]),
        c("v2.c", &["v:weak"], &[]),
    ]);
    let a = run.bare();
    let cl = closure(&a, "t-main");
    // The strong one; of two weak ones the first in path order.
    assert_eq!(cl.files, ["main.c", "v1.c", "w2.c"]);
    assert!(cl.duplicates.is_empty());
    assert!(cl.collisions.is_empty());
}

#[test]
fn common_symbols_are_merged() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["cvar", "a", "b"]),
        c("a.c", &["a", "cvar:common"], &[]),
        c("b.c", &["b", "cvar:common"], &[]),
        c("c.c", &["cvar:common"], &[]),
    ]);
    let a = run.bare();
    let cl = closure(&a, "t-main");
    assert_eq!(cl.files, ["a.c", "b.c", "main.c"]);
    assert!(cl.duplicates.is_empty());
    assert!(cl.collisions.is_empty());
    // A common beside a strong one: the strong one defines it.
    let run = Run::new(vec![
        c("main.c", &["main"], &["k"]),
        c("a.c", &["k:common"], &[]),
        c("b.c", &["k:data"], &[]),
    ]);
    assert_eq!(closure(&run.bare(), "t-main").files, ["b.c", "main.c"]);
}

#[test]
fn a_weak_need_is_a_need() {
    let run = Run::new(vec![
        c("main.c", &["main"], &["opt:weak", "gone:weak"]),
        c("opt.c", &["opt"], &[]),
    ]);
    let cl = run.bare();
    let cl = closure(&cl, "t-main");
    assert_eq!(cl.files, ["main.c", "opt.c"]);
    assert_eq!(cl.outside, ["gone"]);
}

#[test]
fn a_data_symbol_main_is_no_program() {
    let run = Run::new(vec![c("main.c", &["main:data"], &[])]);
    let a = run.bare();
    assert!(a.programs.is_empty());
    assert!(a.closures.is_empty());
    assert_eq!(a.libraries.len(), 1);
}

#[test]
fn the_lz4_shape_one_driver_ten_fuzzers_never_asked() {
    let mut files = vec![
        c(
            "ossfuzz/standaloneengine.c",
            &["main"],
            &[FUZZ_ENTRY, "fopen"],
        ),
        c("lib/lz4.c", &["LZ4_x"], &[]),
        c("examples/e1.c", &["main", "fatal"], &[]),
        c("tests/t1.c", &["main", "fatal"], &[]),
    ];
    for n in 0..10 {
        files.push(c(
            &format!("ossfuzz/f{n}.c"),
            &[FUZZ_ENTRY],
            &["LZ4_x", "malloc"],
        ));
    }
    let run = Run::new(files);
    let (a, _) = run.linked();
    let driver = program(&a, "t-standaloneengine");
    assert_eq!(driver.kind, ProgramKind::Driver);
    assert_eq!(driver.serves.len(), 10);
    assert_eq!(driver.index, None);
    let fuzz: Vec<&Program> = a
        .programs
        .iter()
        .filter(|p| p.kind == ProgramKind::Fuzz)
        .collect();
    assert_eq!(fuzz.len(), 10);
    assert!(fuzz.iter().all(|p| p.index.is_none()));
    assert!(fuzz.iter().all(|p| p.kind_guess == KindGuess::Test));
    // No closure for the driver; each fuzzer linked with it, never asked.
    assert!(a.closures.iter().all(|c| c.program != driver.id));
    assert_eq!(a.closures.len(), 12);
    for cl in &a.closures {
        assert!(cl.questions.is_empty());
        assert!(cl.duplicates.is_empty());
        assert_eq!(cl.linked, Some(Linked::Ok), "{}", cl.program);
    }
    assert_eq!(
        closure(&a, "t-f3").files,
        ["lib/lz4.c", "ossfuzz/f3.c"],
        "the driver is linked with, not part of the closure"
    );
    // The helpers both programs define: listed, not asked.
    assert_eq!(
        a.between_program_duplicates,
        [BetweenDuplicate {
            sym: "fatal".into(),
            definers: vec!["examples/e1.c".into(), "tests/t1.c".into()]
        }]
    );
    // A lone fuzzer with no driver is not linked.
    let run = Run::new(vec![
        c("fuzz/f.c", &[FUZZ_ENTRY], &[]),
        c("fuzz/g.c", &[FUZZ_ENTRY], &[]),
    ]);
    let (a, _) = run.linked();
    assert!(a.closures.iter().all(|c| c.linked.is_none()));
}

/// liblzg's shape: two definers of the same two functions in two
/// programs' closures, one needing a third file.
fn liblzg() -> Vec<FileFacts> {
    vec![
        c(
            "tools/unlzg.c",
            &["main"],
            &["LZG_Decode", "LZG_DecodedSize"],
        ),
        c(
            "tools/lzg.c",
            &["main"],
            &["LZG_Decode", "LZG_Encode", "printf"],
        ),
        c(
            "lib/decode.c",
            &["LZG_Decode", "LZG_DecodedSize"],
            &["_LZG_CalcChecksum"],
        ),
        c("lib/mini.c", &["LZG_Decode", "LZG_DecodedSize"], &[]),
        c("lib/checksum.c", &["_LZG_CalcChecksum"], &[]),
        c("lib/encode.c", &["LZG_Encode"], &["_LZG_CalcChecksum"]),
    ]
}

#[test]
fn the_liblzg_shape_both_choices_link_so_both_programs_are_held() {
    let run = Run::new(liblzg());
    let (a, _) = run.linked();
    for id in ["t-unlzg", "t-lzg"] {
        let cl = closure(&a, id);
        assert_eq!(cl.duplicates.len(), 1, "{id}");
        let d = &cl.duplicates[0];
        // One index for the set in both programs, each with its symbols.
        assert_eq!(d.set, "d1");
        assert_eq!(d.links, ["d1.1", "d1.2"]);
        assert_eq!(d.choice, None);
        assert_eq!(cl.questions, ["d1"]);
        assert_eq!(cl.linked, None);
        assert!(cl.incomplete);
    }
    let defs: Vec<&str> = closure(&a, "t-unlzg").duplicates[0]
        .definers
        .iter()
        .map(|d| d.path.as_str())
        .collect();
    assert_eq!(defs, ["lib/decode.c", "lib/mini.c"]);
    assert_eq!(
        closure(&a, "t-lzg").duplicates[0].symbols,
        ["LZG_Decode"],
        "each closure's own symbols"
    );
    // The alternatives are not libraries.
    assert!(a.libraries.is_empty(), "{:?}", a.libraries);
}

#[test]
fn a_choice_recomputes_the_closure_from_scratch() {
    let files = liblzg();
    let mut refs: Vec<&FileFacts> = files.iter().collect();
    refs.sort_by(|a, b| a.path.cmp(&b.path));
    let program: Vec<bool> = refs.iter().map(|f| f.path.starts_with("tools/")).collect();
    let project = Project::new(refs, program);
    let at = |p: &str| project.by_path[p];
    let start = at("tools/unlzg.c");
    let key = vec![at("lib/decode.c"), at("lib/mini.c")];
    let paths = |core: &Core| -> Vec<String> {
        core.files
            .iter()
            .map(|&f| project.files[f].path.clone())
            .collect()
    };
    let with = |keep: &str| project.closure(start, &BTreeMap::from([(key.clone(), at(keep))]));
    assert_eq!(
        paths(&with("lib/decode.c")),
        ["lib/checksum.c", "lib/decode.c", "tools/unlzg.c"]
    );
    assert_eq!(paths(&with("lib/mini.c")), ["lib/mini.c", "tools/unlzg.c"]);

    // Through the analysis: when only mini.c links for unlzg, checksum.c
    // is not kept with it, and decode.c is neither shared nor a library.
    let mut files = liblzg();
    files.retain(|f| f.path != "tools/lzg.c" && f.path != "lib/encode.c");
    // checksum.c needs a symbol nothing defines, so the decode.c choice fails.
    for f in &mut files {
        if f.path == "lib/checksum.c" {
            *f = c("lib/checksum.c", &["_LZG_CalcChecksum"], &["nowhere"]);
        }
    }
    let run = Run::new(files);
    let (a, _) = run.linked();
    let cl = closure(&a, "t-unlzg");
    assert_eq!(cl.files, ["lib/mini.c", "tools/unlzg.c"]);
    assert_eq!(cl.linked, Some(Linked::Ok));
    assert!(a.shared.is_empty());
    let lib_files: Vec<&str> = a
        .libraries
        .iter()
        .flat_map(|l| l.files.iter().map(String::as_str))
        .collect();
    assert!(!lib_files.contains(&"lib/decode.c"), "{lib_files:?}");
}

#[test]
fn a_program_file_is_never_pulled_into_another_closure() {
    let run = Run::new(vec![
        c("tools/a.c", &["main"], &["helper"]),
        c("tools/b.c", &["main", "helper"], &[]),
    ]);
    let (a, _) = run.linked();
    let cl = closure(&a, "t-a");
    assert_eq!(cl.files, ["tools/a.c"]);
    assert_eq!(
        cl.needs_from,
        [NeedsFrom {
            sym: "helper".into(),
            program: "t-b".into()
        }]
    );
    assert!(cl.outside.is_empty());
    assert_eq!(
        cl.linked,
        Some(Linked::Failed {
            missing: vec!["helper".into()],
            doubled: vec![]
        })
    );
}

#[test]
fn the_benchmark_shape_zero_programs_one_library() {
    let run = Run::new(vec![
        c("src/a.c", &["a_fn"], &["printf"]),
        c("src/b.c", &["b_fn"], &["a_fn"]),
        c("src/c.c", &["c_fn"], &["b_fn", "malloc"]),
    ]);
    let a = run.bare();
    assert!(a.programs.is_empty());
    assert_eq!(
        a.libraries,
        [Library {
            id: "l-a".into(),
            files: vec!["src/a.c".into(), "src/b.c".into(), "src/c.c".into()],
            needs_from_outside: vec![],
        }]
    );
}

#[test]
fn libraries_list_the_files_they_need_from_outside() {
    let run = Run::new(vec![
        c("tools/main.c", &["main"], &["core"]),
        c("lib/core.c", &["core"], &[]),
        c("lib/extra.c", &["extra"], &["core"]),
    ]);
    let a = run.bare();
    assert_eq!(
        a.libraries,
        [Library {
            id: "l-extra".into(),
            files: vec!["lib/extra.c".into()],
            needs_from_outside: vec!["lib/core.c".into()],
        }]
    );
}

#[test]
fn a_program_that_did_not_compile_is_listed_and_its_files_not_a_library() {
    let mut run = Run::new(vec![
        failed("tools/broken.c"),
        c("lib/helper.c", &["helper"], &["helper2"]),
        c("lib/helper2.c", &["helper2"], &[]),
        c("lib/other.c", &["other"], &[]),
    ]);
    run.parser.insert(
        "tools/broken.c".into(),
        ParserFacts {
            defines: BTreeSet::from(["main".into()]),
            calls: BTreeSet::from(["helper".into()]),
        },
    );
    let a = run.bare();
    assert_eq!(a.programs_not_compiled, ["tools/broken.c"]);
    assert!(a.programs.is_empty());
    let libs: Vec<&str> = a.libraries.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(libs, ["l-other"]);
}

#[test]
fn each_incomplete_case() {
    let base = || {
        vec![
            c("main.c", &["main"], &["frob", "printf"]),
            c("ok.c", &["ok"], &[]),
        ]
    };
    // Complete: every outside symbol, no unread file, no unreadable folder.
    let mut files = base();
    files[0] = c("main.c", &["main"], &["printf"]);
    let run = Run::new(files);
    assert!(!closure(&run.bare(), "t-main").incomplete);

    // may-be-defined-in: a parsed file that did not compile defines frob.
    let mut files = base();
    files.push(failed("lib/frob.c"));
    let mut run = Run::new(files);
    run.parser.insert(
        "lib/frob.c".into(),
        ParserFacts {
            defines: BTreeSet::from(["frob".into(), "unused".into()]),
            calls: BTreeSet::new(),
        },
    );
    let a = run.bare();
    let cl = closure(&a, "t-main");
    assert!(cl.incomplete);
    assert_eq!(
        cl.incomplete_why,
        [Incomplete {
            why: IncompleteWhy::MayBeDefinedIn,
            path: Some("lib/frob.c".into()),
            symbols: vec!["frob".into()],
        }]
    );

    // unread: a `.c` neither parsed nor compiled while outside symbols stay.
    let mut files = base();
    let mut big = c("big.c", &[], &[]);
    big.parsed = false;
    big.too_large = true;
    big.compiled = None;
    files.push(big.clone());
    let run = Run::new(files);
    let a = run.bare();
    assert_eq!(
        closure(&a, "t-main").incomplete_why,
        [Incomplete {
            why: IncompleteWhy::Unread,
            path: Some("big.c".into()),
            symbols: vec!["frob".into(), "printf".into()],
        }]
    );
    // ... but not when the closure has no outside symbol.
    let run = Run::new(vec![
        c("main.c", &["main"], &["ok"]),
        c("ok.c", &["ok"], &[]),
        big,
    ]);
    assert!(!closure(&run.bare(), "t-main").incomplete);

    // unreadable-folder: on every closure.
    let mut run = Run::new(vec![c("main.c", &["main"], &[])]);
    run.issues.push(WalkIssue {
        path: "secret".into(),
        why: "cannot be read: Permission denied (os error 13)".into(),
    });
    run.issues.push(WalkIssue {
        path: "link".into(),
        why: "a link that points nowhere".into(),
    });
    let a = run.bare();
    assert_eq!(
        closure(&a, "t-main").incomplete_why,
        [Incomplete {
            why: IncompleteWhy::UnreadableFolder,
            path: Some("secret".into()),
            symbols: vec![],
        }]
    );

    // pending: a duplicate with no linker.
    let run = Run::new(liblzg());
    let a = run.bare();
    assert_eq!(
        closure(&a, "t-unlzg").incomplete_why,
        [Incomplete {
            why: IncompleteWhy::Pending,
            path: None,
            symbols: vec!["LZG_Decode".into(), "LZG_DecodedSize".into()],
        }]
    );
}

#[test]
fn an_accepted_tool_keeps_its_id_in_the_analysis() {
    let mut run = Run::new(three_mains());
    run.accepted.push(("tests/t.c".into(), "t-my-test".into()));
    let a = run.bare();
    assert_eq!(program(&a, "t-my-test").path, "tests/t.c");
}

#[test]
fn indexes_once_per_project_with_nested_sets_under_their_choice() {
    let run = Run::new(vec![
        c("tools/p.c", &["main"], &["A"]),
        c("tools/q.c", &["main"], &["C"]),
        c("lib/a1.c", &["A"], &["B"]),
        c("lib/a2.c", &["A"], &[]),
        c("lib/b1.c", &["B"], &[]),
        c("lib/b2.c", &["B"], &[]),
        c("lib/c1.c", &["C"], &[]),
        c("lib/c2.c", &["C"], &[]),
    ]);
    let (a, _) = run.linked();
    let p = closure(&a, "t-p");
    let sets: Vec<(&str, Option<&str>)> = p
        .duplicates
        .iter()
        .map(|d| (d.set.as_str(), d.under.as_deref()))
        .collect();
    // {a1,a2} and {c1,c2} are reached directly: d1 and d2. {b1,b2} only
    // under a1's choice: numbered after them, under d1.1.
    assert_eq!(sets, [("d1", None), ("d3", Some("d1.1"))]);
    assert_eq!(closure(&a, "t-q").duplicates[0].set, "d2");
    assert_eq!(p.questions, ["d1", "d3"]);
    assert_eq!(p.duplicates[1].links, ["d3.1", "d3.2"]);
}

#[test]
fn linking_settles_exactly_one_holds_several_and_reports_none() {
    // None links: the fewest missing.
    let run = Run::new(vec![
        c("main.c", &["main"], &["x"]),
        c("x1.c", &["x"], &["m1", "m2"]),
        c("x2.c", &["x"], &["m3"]),
    ]);
    let (a, calls) = run.linked();
    assert_eq!(calls, 2);
    let cl = closure(&a, "t-main");
    assert_eq!(
        cl.linked,
        Some(Linked::Failed {
            missing: vec!["m3".into()],
            doubled: vec![]
        })
    );
    assert!(cl.questions.is_empty());
    assert_eq!(cl.duplicates[0].choice, None);
    assert!(cl.incomplete);
}

#[test]
fn over_the_limit_is_held_without_linking() {
    // Five definers in one set.
    let mut files = vec![c("main.c", &["main"], &["x"])];
    for n in 1..=5 {
        files.push(c(&format!("x{n}.c"), &["x"], &[]));
    }
    let (a, calls) = Run::new(files).linked();
    assert_eq!(calls, 0);
    assert_eq!(closure(&a, "t-main").questions, ["d1"]);
    // Five sets of two: 32 combinations, over 16.
    let mut files = vec![c("main.c", &["main"], &["s1", "s2", "s3", "s4", "s5"])];
    for s in 1..=5 {
        for d in 1..=2 {
            files.push(c(&format!("s{s}_{d}.c"), &[&format!("s{s}")], &[]));
        }
    }
    let (a, calls) = Run::new(files).linked();
    assert_eq!(calls, 0);
    let cl = closure(&a, "t-main");
    assert_eq!(cl.questions.len(), 5);
    assert_eq!(cl.linked, None);
    // Four sets of two (16 combinations) are within the limit.
    let mut files = vec![c("main.c", &["main"], &["s1", "s2", "s3", "s4"])];
    for s in 1..=4 {
        for d in 1..=2 {
            files.push(c(&format!("s{s}_{d}.c"), &[&format!("s{s}")], &[]));
        }
    }
    let (_, calls) = Run::new(files).linked();
    assert_eq!(calls, 16);
}

#[test]
fn a_c_file_included_by_another_is_recorded() {
    let mut lz4 = c("lib/lz4.c", &["LZ4_x"], &[]);
    lz4.includes = vec!["lib/lz4.h".into()];
    let mut hc = c("lib/lz4hc.c", &["LZ4_hc"], &[]);
    hc.includes = vec!["lib/lz4.c".into()];
    let mut h = c("lib/lz4all.h", &[], &[]);
    h.kind = FileKind::H;
    h.compiled = None;
    h.includes = vec!["lib/lz4.c".into()];
    let a = Run::new(vec![lz4, hc, h]).bare();
    assert_eq!(
        a.included_by,
        [IncludedBy {
            file: "lib/lz4.c".into(),
            by: vec!["lib/lz4all.h".into(), "lib/lz4hc.c".into()]
        }]
    );
}

#[test]
fn two_runs_give_the_same_output_in_any_input_order() {
    let mut files = three_mains();
    files.extend(liblzg());
    let first = Run::new(files.clone()).linked().0;
    files.reverse();
    let second = Run::new(files).linked().0;
    assert_eq!(first, second);
}
