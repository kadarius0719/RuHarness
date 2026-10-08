//! The link checks over tiny C projects (docs/PROJECT-MAP-DESIGN.md §4):
//! mapped and linked with the real `cc` under the map's runner, as the
//! project map's own tests run it (sandboxed where this computer has one).

use super::*;
use crate::projectmap::closure::{Choice, Closure};
use crate::projectmap::map_folder_in;
use crate::testutil::TempDir;
use std::collections::BTreeMap;

fn project(tag: &str, files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new(tag);
    for (rel, text) in files {
        let path = tmp.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
        std::fs::write(&path, text).expect("write");
    }
    tmp
}

/// Map `root` and analyse it with the real linker.
fn analyse(root: &Path) -> Analysis {
    let parent = TempDir::new("link-parent");
    let map = map_folder_in(root, Path::new("."), parent.path()).expect("the map runs");
    let parser = BTreeMap::new();
    let input = Input {
        files: &map.files,
        parser: &parser,
        walk_issues: &map.walk_issues,
        accepted: &[],
    };
    let mut linker = CcLinker::new_in(root, &[], parent.path()).expect("a linker");
    closure::analyze(&input, Some(&mut linker)).expect("the analysis runs")
}

fn closure<'a>(a: &'a Analysis, id: &str) -> &'a Closure {
    a.closures
        .iter()
        .find(|c| c.program == id)
        .unwrap_or_else(|| panic!("a closure for {id}: {:?}", a.closures))
}

const MAIN: &str = "int pick(void);\nint main(void) { return pick(); }\n";

#[test]
fn exactly_one_choice_links() {
    let tmp = project(
        "link-one",
        &[
            ("main.c", MAIN),
            (
                "a/pick.c",
                "int absent_one(void);\nint pick(void) { return absent_one(); }\n",
            ),
            ("b/pick.c", "int pick(void) { return 0; }\n"),
        ],
    );
    let a = analyse(tmp.path());
    let cl = closure(&a, "t-main");
    assert_eq!(cl.linked, Some(Linked::Ok));
    assert_eq!(cl.files, ["b/pick.c", "main.c"]);
    let d = &cl.duplicates[0];
    assert_eq!(
        d.choice,
        Some(Choice {
            keep: "d1.2".into(),
            by: "links"
        })
    );
    assert!(cl.questions.is_empty());
    assert!(!cl.incomplete, "{:?}", cl.incomplete_why);
}

#[test]
fn when_no_choice_links_the_fewest_missing_are_kept() {
    let tmp = project(
        "link-none",
        &[
            ("main.c", MAIN),
            (
                "a/pick.c",
                "int absent_one(void);\nint pick(void) { return absent_one(); }\n",
            ),
            (
                "b/pick.c",
                "int absent_two(void); int absent_three(void);\n\
                 int pick(void) { return absent_two() + absent_three(); }\n",
            ),
        ],
    );
    let a = analyse(tmp.path());
    let cl = closure(&a, "t-main");
    assert_eq!(
        cl.linked,
        Some(Linked::Failed {
            missing: vec!["absent_one".into()],
            doubled: vec![]
        })
    );
    assert!(cl.questions.is_empty());
    assert_eq!(cl.duplicates[0].choice, None);
}

#[test]
fn several_choices_linking_hold_the_program() {
    let tmp = project(
        "link-several",
        &[
            ("main.c", MAIN),
            ("a/pick.c", "int pick(void) { return 1; }\n"),
            ("b/pick.c", "int pick(void) { return 0; }\n"),
        ],
    );
    let a = analyse(tmp.path());
    let cl = closure(&a, "t-main");
    assert_eq!(cl.linked, None);
    assert_eq!(cl.questions, ["d1"]);
    assert_eq!(cl.duplicates[0].links, ["d1.1", "d1.2"]);
    assert_eq!(cl.files, ["main.c"]);
}

#[test]
fn a_libm_name_links_with_the_guessed_library() {
    let tmp = project(
        "link-libm",
        &[(
            "main.c",
            "#include <math.h>\n#include <stdio.h>\n\
             int main(int argc, char **argv) { (void)argv; printf(\"%f\\n\", log((double)argc)); \
             return 0; }\n",
        )],
    );
    let a = analyse(tmp.path());
    let cl = closure(&a, "t-main");
    assert!(cl.outside.iter().any(|s| s == "log"), "{:?}", cl.outside);
    assert_eq!(cl.linked, Some(Linked::Ok));
}

#[test]
fn the_libraries_are_guessed_from_the_names() {
    let names = |n: &[&str]| -> Vec<String> { n.iter().map(|s| (*s).to_string()).collect() };
    assert_eq!(guess_libs(&names(&["log", "printf"]), false, true), ["-lm"]);
    assert_eq!(guess_libs(&names(&["sqrtf"]), false, true), ["-lm"]);
    assert_eq!(guess_libs(&names(&["logical"]), false, true), [""; 0]);
    assert_eq!(guess_libs(&names(&["deflate"]), true, true), ["-lz"]);
    assert_eq!(guess_libs(&names(&["deflate"]), false, true), [""; 0]);
    assert_eq!(
        guess_libs(&names(&["pthread_create"]), false, false),
        ["-lpthread"]
    );
    assert_eq!(
        guess_libs(&names(&["pthread_create"]), false, true),
        [""; 0]
    );
}

#[test]
fn linked_comes_from_the_facts_and_the_exit_status() {
    // Two files of one closure define `dup`: doubled, from the facts.
    // `helper` only another program defines: missing, found by probing,
    // while `printf`, which the system provides, is not.
    let tmp = project(
        "link-facts",
        &[
            (
                "tools/a.c",
                "#include <stdio.h>\nint helper(void); int f(void); int g(void);\n\
                 int main(void) { printf(\"undefined symbol _decoy\\n\"); \
                 return helper() + f() + g(); }\n",
            ),
            (
                "tools/b.c",
                "int helper(void) { return 0; }\nint main(void) { return helper(); }\n",
            ),
            ("lib/f.c", "int dup = 1;\nint f(void) { return dup; }\n"),
            ("lib/g.c", "int dup = 2;\nint g(void) { return dup; }\n"),
        ],
    );
    let a = analyse(tmp.path());
    let cl = closure(&a, "t-a");
    assert_eq!(cl.collisions.len(), 1);
    assert_eq!(
        cl.linked,
        Some(Linked::Failed {
            missing: vec!["helper".into()],
            doubled: vec!["dup".into()]
        })
    );
    assert_eq!(closure(&a, "t-b").linked, Some(Linked::Ok));
}

#[test]
fn a_fuzzer_links_with_the_project_driver() {
    let tmp = project(
        "link-fuzz",
        &[
            (
                "ossfuzz/driver.c",
                "#include <stddef.h>\n#include <stdint.h>\n\
                 int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n);\n\
                 int main(void) { return LLVMFuzzerTestOneInput(0, 0); }\n",
            ),
            (
                "ossfuzz/f1.c",
                "#include <stddef.h>\n#include <stdint.h>\n\
                 int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n) { (void)d; return (int)n; }\n",
            ),
            (
                "ossfuzz/f2.c",
                "#include <stddef.h>\n#include <stdint.h>\n\
                 int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n) { (void)n; return d != 0; }\n",
            ),
        ],
    );
    let a = analyse(tmp.path());
    assert_eq!(a.closures.len(), 2, "no closure for the driver");
    for cl in &a.closures {
        assert_eq!(cl.linked, Some(Linked::Ok), "{}", cl.program);
        assert!(cl.questions.is_empty());
    }
}
