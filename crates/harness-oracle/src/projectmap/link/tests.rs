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
    harness_core::adopt::testing::adoption_file();
    let parent = TempDir::new("link-parent");
    let map = map_folder_in(root, Path::new("."), parent.path()).expect("the map runs");
    let parser = BTreeMap::new();
    let input = Input {
        files: &map.files,
        parser: &parser,
        walk_issues: &map.walk_issues,
        accepted: &[],
    };
    let setup = LinkSetup {
        walked: map.files.iter().map(|f| f.path.as_str()).collect(),
        system_headers: &map.configuration.system_headers,
        deadline: map.deadline,
    };
    let mut linker = CcLinker::new_in(root, setup, parent.path()).expect("a linker");
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
            doubled: vec![],
            not_checked: vec![],
            not_compiled: vec![]
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
            doubled: vec!["dup".into()],
            not_checked: vec![],
            not_compiled: vec![]
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

/// Map `root` and link it with `parent` as the fresh folder's parent,
/// keeping the linker to look into after the analysis.
fn analyse_keeping<'a>(
    map: &'a crate::projectmap::FolderMap,
    parser: &'a BTreeMap<String, closure::ParserFacts>,
    parent: &Path,
) -> (Analysis, CcLinker<'a>) {
    let input = Input {
        files: &map.files,
        parser,
        walk_issues: &map.walk_issues,
        accepted: &[],
    };
    let setup = LinkSetup {
        walked: map.files.iter().map(|f| f.path.as_str()).collect(),
        system_headers: &map.configuration.system_headers,
        deadline: map.deadline,
    };
    let mut linker = CcLinker::new_in(&map.root, setup, parent).expect("a linker");
    let a = closure::analyze(&input, Some(&mut linker)).expect("the analysis runs");
    (a, linker)
}

/// The link check compiles each file exactly as the map did: a guess's
/// per-file `compile_commands.json` flags (here a `-D` every file needs)
/// reach the link's compile too, so both choices link and the set is held.
#[test]
fn the_link_compiles_with_each_files_own_flags() {
    const GUARD: &str = "#ifndef HAVE_CONFIG_H\n#error no config\n#endif\n";
    let main = format!("{GUARD}int decode(void);\nint main(void) {{ return decode(); }}\n");
    let dec = format!("{GUARD}int decode(void) {{ return 0; }}\n");
    let mini = format!("{GUARD}int decode(void) {{ return 1; }}\n");
    let tmp = project(
        "link-entry-flags",
        &[
            ("main.c", &main),
            ("src/dec.c", &dec),
            ("src/mini.c", &mini),
        ],
    );
    let root = tmp.path();
    let entries: Vec<serde_json::Value> = ["main.c", "src/dec.c", "src/mini.c"]
        .iter()
        .map(|f| {
            serde_json::json!({
                "directory": root.to_str().expect("utf-8"),
                "file": f,
                "arguments": ["cc", "-DHAVE_CONFIG_H", "-c", f],
            })
        })
        .collect();
    std::fs::write(
        root.join("compile_commands.json"),
        serde_json::to_string(&entries).expect("json"),
    )
    .expect("write");
    let a = analyse(root);
    let cl = closure(&a, "t-main");
    assert_eq!(cl.questions, ["d1"], "{cl:?}");
    assert_eq!(cl.duplicates[0].links, ["d1.1", "d1.2"]);
}

/// `system_headers`: the map compiles a folder holding a project
/// `unistd.h` with `-idirafter` so the system's is found first; the link's
/// compile must too, or it reaches the project's (an `#error`) and fails.
#[test]
fn the_link_passes_system_header_folders_after_the_system() {
    let tmp = project(
        "link-sysh",
        &[
            (
                "main.c",
                "#include <unistd.h>\n#include <compat_extra.h>\n\
                 int main(void) { return EXTRA + (int)getpid() * 0; }\n",
            ),
            (
                "compat/unistd.h",
                "#error the project's unistd.h was used\n",
            ),
            ("compat/compat_extra.h", "#define EXTRA 0\n"),
            (
                "migration/map/config.toml",
                "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []\n\
                 system_headers = [\"unistd.h\"]\n",
            ),
        ],
    );
    let a = analyse(tmp.path());
    assert_eq!(closure(&a, "t-main").linked, Some(Linked::Ok));
}

/// Past the probe budget, a name not yet decided is "not checked", never
/// "missing": the system's own names never show up as missing.
#[test]
fn names_past_the_probe_budget_are_not_checked_never_missing() {
    let mut src = String::from("#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n");
    for n in 1..=70 {
        src.push_str(&format!("int miss_{n}(void);\n"));
    }
    src.push_str("int main(int argc, char **argv) {\n  int t = 0;\n");
    for n in 1..=70 {
        src.push_str(&format!("  t += miss_{n}();\n"));
    }
    src.push_str(
        "  puts(argv[0]); t += (int)strlen(argv[0]); t += atoi(argv[0]);\n\
         t += (int)strtol(argv[0], 0, 10); t += rand(); t += putchar('x');\n\
         t += (int)(strchr(argv[0], 'a') != 0); t += (int)(strrchr(argv[0], 'b') != 0);\n\
         return t + argc;\n}\n",
    );
    let tmp = project("link-probes", &[("main.c", &src)]);
    let a = analyse(tmp.path());
    let Some(Linked::Failed {
        missing,
        not_checked,
        ..
    }) = &closure(&a, "t-main").linked
    else {
        panic!("it does not link: {:?}", closure(&a, "t-main").linked);
    };
    assert!(
        missing.iter().all(|m| m.starts_with("miss_")),
        "only the project's names are missing: {missing:?}"
    );
    for n in 1..=70 {
        let name = format!("miss_{n}");
        assert!(
            missing.contains(&name) || not_checked.contains(&name),
            "{name} is missing or not checked"
        );
    }
    assert!(!not_checked.is_empty(), "the budget ran out: {missing:?}");
}

/// An object over 64 MiB is refused by the link's compile (here the file
/// grew after the map compiled it): "a file did not compile for the link",
/// never a bare failure.
#[test]
fn an_object_over_the_cap_did_not_compile_for_the_link() {
    let tmp = project(
        "link-big-object",
        &[
            ("main.c", "int f(void);\nint main(void) { return f(); }\n"),
            ("f.c", "int f(void) { return 0; }\n"),
        ],
    );
    harness_core::adopt::testing::adoption_file();
    let parent = TempDir::new("link-big-parent");
    let map = map_folder_in(tmp.path(), Path::new("."), parent.path()).expect("the map runs");
    std::fs::write(
        tmp.path().join("f.c"),
        "char big[65 << 20] = {1};\nint f(void) { return big[0]; }\n",
    )
    .expect("grow f.c");
    let parser = BTreeMap::new();
    let (a, _) = analyse_keeping(&map, &parser, parent.path());
    assert_eq!(
        closure(&a, "t-main").linked,
        Some(Linked::Failed {
            missing: vec![],
            doubled: vec![],
            not_checked: vec![],
            not_compiled: vec!["f.c".into()],
        })
    );
}

/// Objects are deleted once no closure still to be linked needs them, and
/// nothing built is ever run (a constructor and `main` that would write a
/// marker leave none).
#[test]
fn objects_go_after_their_last_link_and_nothing_built_runs() {
    let tmp = project(
        "link-release",
        &[("lib/shared.c", "int shared(void) { return 0; }\n")],
    );
    let root = tmp.path().to_path_buf();
    let dir = root.to_str().expect("utf-8").to_string();
    for p in ["a", "b"] {
        let src = format!(
            "#include <stdio.h>\n\
             static void mark(const char *p) {{ FILE *f = fopen(p, \"w\"); if (f) fclose(f); }}\n\
             __attribute__((constructor)) static void ctor(void) {{ mark(\"{dir}/ctor-{p}\"); }}\n\
             int shared(void);\n\
             int main(void) {{ mark(\"{dir}/main-{p}\"); return shared(); }}\n"
        );
        std::fs::create_dir_all(root.join("tools")).expect("tools");
        std::fs::write(root.join(format!("tools/{p}.c")), src).expect("write");
    }
    harness_core::adopt::testing::adoption_file();
    let parent = TempDir::new("link-release-parent");
    let map = map_folder_in(&root, Path::new("."), parent.path()).expect("the map runs");
    let parser = BTreeMap::new();
    let (a, linker) = analyse_keeping(&map, &parser, parent.path());
    for id in ["t-a", "t-b"] {
        assert_eq!(closure(&a, id).linked, Some(Linked::Ok), "{id}");
    }
    assert!(
        linker.objects.values().all(Option::is_none),
        "every object released: {:?}",
        linker.objects
    );
    let objects_left = std::fs::read_dir(&linker.fresh.0)
        .expect("the fresh folder")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('o'))
        .count();
    assert_eq!(objects_left, 0);
    for m in ["ctor-a", "ctor-b", "main-a", "main-b"] {
        assert!(!root.join(m).exists(), "{m}: nothing built is run");
    }
}

/// On Apple, an object's own `.linker_option` never picks a library: a
/// program calling zlib's `compress` with `-lz` hidden in its object is
/// missing `compress`, as it is without the line.
#[cfg(target_vendor = "apple")]
#[test]
fn an_objects_linker_option_never_picks_a_library() {
    let tmp = project(
        "link-autolink",
        &[(
            "main.c",
            "__asm__(\".linker_option \\\"-lz\\\"\");\n\
             int compress(unsigned char *, unsigned long *, const unsigned char *, unsigned long);\n\
             int main(void) { return compress(0, 0, 0, 0); }\n",
        )],
    );
    let a = analyse(tmp.path());
    assert!(
        matches!(
            &closure(&a, "t-main").linked,
            Some(Linked::Failed { missing, .. }) if missing == &["compress"]
        ),
        "{:?}",
        closure(&a, "t-main").linked
    );
}
