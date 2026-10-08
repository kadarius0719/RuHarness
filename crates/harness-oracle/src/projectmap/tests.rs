//! Tests of the project map over one folder (docs/PROJECT-MAP-DESIGN.md §4,
//! "The walk and the map"): small made-up projects in temporary folders,
//! zopfli's `src/zopfli` and one benchmark case.

use super::*;
use crate::testutil::TempDir;

/// A made-up project: each `(relative path, bytes)` written under a fresh
/// temporary root.
fn project(tag: &str, files: &[(&str, &[u8])]) -> TempDir {
    let tmp = TempDir::new(tag);
    for (rel, bytes) in files {
        let path = tmp.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
        std::fs::write(&path, bytes).expect("write");
    }
    tmp
}

/// Map the whole of `root`, the fresh folder made under a temporary parent
/// of its own.
fn map(root: &Path) -> FolderMap {
    // The test process's own adoption file, never the person's.
    harness_core::adopt::testing::adoption_file();
    let parent = TempDir::new("map-parent");
    map_folder_in(root, Path::new("."), parent.path()).expect("the map runs")
}

fn file<'a>(map: &'a FolderMap, path: &str) -> &'a FileFacts {
    map.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is mapped: {:?}", map.files))
}

fn dirs(facts: &FileFacts) -> Vec<&str> {
    facts.include_dirs.iter().map(String::as_str).collect()
}

fn defined(facts: &FileFacts) -> Vec<&str> {
    facts.defined.iter().map(|d| d.name.as_str()).collect()
}

fn needed(facts: &FileFacts) -> Vec<&str> {
    facts.needed.iter().map(|n| n.name.as_str()).collect()
}

#[test]
fn an_angle_include_from_a_sibling_include_folder_is_found() {
    let tmp = project(
        "sibling",
        &[
            (
                "tool/main.c",
                b"#include <proj.h>\nint main(void) { return proj(); }\n",
            ),
            ("include/proj.h", b"int proj(void);\n"),
        ],
    );
    let map = map(tmp.path());
    let main = file(&map, "tool/main.c");
    assert_eq!(dirs(main), ["include"]);
    assert_eq!(main.includes, ["include/proj.h"]);
    assert_eq!(main.compiled, Some(Compiled::Ok), "{:?}", main.message);
    assert_eq!(defined(main), ["main"]);
    assert_eq!(main.defined[0].kind, "function");
    assert_eq!(needed(main), ["proj"]);
    assert!(main.ambiguous.is_empty());
    let header = file(&map, "include/proj.h");
    assert_eq!(header.kind, FileKind::H);
    assert_eq!(header.compiled, None);
}

#[test]
fn a_header_named_with_a_folder_part_is_found_by_whole_parts() {
    let tmp = project(
        "parts",
        &[
            (
                "tool/main.c",
                b"#include <proj/api.h>\nint main(void) { return api(); }\n",
            ),
            (
                "include/proj/api.h",
                b"#include \"detail.h\"\nint api(void);\n",
            ),
            ("include/proj/detail.h", b"#define DETAIL 1\n"),
            // Shares the last part only: never a candidate.
            ("other/notproj/api.h", b"#error wrong api.h\n"),
        ],
    );
    let map = map(tmp.path());
    let main = file(&map, "tool/main.c");
    assert_eq!(dirs(main), ["include"]);
    assert!(main.ambiguous.is_empty(), "{:?}", main.ambiguous);
    assert_eq!(main.compiled, Some(Compiled::Ok), "{:?}", main.message);
}

#[test]
fn a_quoted_include_beside_the_including_file_is_never_ambiguous() {
    let tmp = project(
        "beside",
        &[
            (
                "src/a.c",
                b"#include \"util.h\"\nint a(void) { return UTIL; }\n",
            ),
            ("src/util.h", b"#define UTIL 1\n"),
            ("lib/util.h", b"#error the other util.h\n"),
            // Not beside either: two project holders, so ambiguous.
            ("c.c", b"#include \"util.h\"\nint c(void) { return 0; }\n"),
        ],
    );
    let map = map(tmp.path());
    let a = file(&map, "src/a.c");
    assert!(a.ambiguous.is_empty(), "{:?}", a.ambiguous);
    assert!(a.include_dirs.is_empty());
    assert_eq!(a.includes, ["src/util.h"]);
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    let c = file(&map, "c.c");
    assert_eq!(c.ambiguous.len(), 1);
    assert_eq!(c.ambiguous[0].candidates[..2], ["lib/util.h", "src/util.h"]);
}

#[test]
fn two_folders_holding_config_h_make_an_ambiguous_include_with_the_one_used() {
    let tmp = project(
        "config",
        &[
            (
                "main.c",
                b"#include \"x.h\"\n#include \"config.h\"\nint m(void) { return X + CONFIG; }\n",
            ),
            ("a/x.h", b"#define X 1\n"),
            ("a/config.h", b"#define CONFIG 1\n"),
            ("b/config.h", b"#define CONFIG 2\n"),
        ],
    );
    let map = map(tmp.path());
    let main = file(&map, "main.c");
    // `a` is added for x.h alone; config.h picks no folder of its own.
    assert_eq!(dirs(main), ["a"]);
    assert_eq!(
        main.ambiguous,
        [Ambiguous {
            header: "config.h".into(),
            candidates: vec!["a/config.h".into(), "b/config.h".into()],
            used: Some("a/config.h".into()),
        }]
    );
    assert_eq!(main.compiled, Some(Compiled::Ok), "{:?}", main.message);
}

#[cfg(unix)]
#[test]
fn a_project_unistd_h_is_ambiguous_and_the_used_one_recorded() {
    let tmp = project(
        "unistd",
        &[
            (
                "a.c",
                b"#include <other.h>\n#include <unistd.h>\nint a(void) { return OTHER + PROJECT_UNISTD; }\n",
            ),
            (
                "b.c",
                b"#include <unistd.h>\nint b(void) { return (int)getpid(); }\n",
            ),
            ("compat/unistd.h", b"#define PROJECT_UNISTD 1\n"),
            ("compat/other.h", b"#define OTHER 1\n"),
        ],
    );
    let map = map(tmp.path());
    let a = file(&map, "a.c");
    assert_eq!(dirs(a), ["compat"]);
    let candidates = vec!["compat/unistd.h".to_string(), "system".to_string()];
    assert_eq!(
        a.ambiguous,
        [Ambiguous {
            header: "unistd.h".into(),
            candidates: candidates.clone(),
            used: Some("compat/unistd.h".into()),
        }]
    );
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    let b = file(&map, "b.c");
    assert!(b.include_dirs.is_empty());
    assert_eq!(
        b.ambiguous,
        [Ambiguous {
            header: "unistd.h".into(),
            candidates,
            used: Some("system".into()),
        }]
    );
    assert_eq!(b.compiled, Some(Compiled::Ok), "{:?}", b.message);
    assert!(
        !b.outside_includes,
        "the system's headers are the toolchain's"
    );
}

#[test]
fn a_file_that_does_not_compile_gets_a_closed_reason_and_the_walk_goes_on() {
    let tmp = project(
        "fails",
        &[
            ("bad.c", b"int x = ;\n"),
            ("missing.c", b"#include \"nope.h\"\nint m;\n"),
            ("good.c", b"int good(void) { return 1; }\n"),
        ],
    );
    let map = map(tmp.path());
    assert_eq!(
        file(&map, "bad.c").compiled,
        Some(Compiled::Failed {
            reason: Reason::Syntax,
            header: None,
            detail: None,
            at: Some("bad.c:1".into()),
        })
    );
    let missing = file(&map, "missing.c");
    assert_eq!(
        missing.compiled,
        Some(Compiled::Failed {
            reason: Reason::MissingHeader,
            header: Some("nope.h".into()),
            detail: None,
            at: Some("missing.c:1".into()),
        })
    );
    // The line shown once is scrubbed of the machine's paths.
    let shown = missing.message.as_deref().expect("a line to show");
    assert!(shown.contains("nope.h"), "{shown}");
    assert!(
        !shown.contains(tmp.path().to_str().expect("utf-8")),
        "{shown}"
    );
    assert_eq!(file(&map, "good.c").compiled, Some(Compiled::Ok));
    assert_eq!(defined(file(&map, "good.c")), ["good"]);
}

#[test]
fn a_c_file_over_the_cap_is_too_large_and_not_compiled() {
    let mut big = b"/*".to_vec();
    big.resize((MAX_SOURCE_BYTES + 16) as usize, b' ');
    big.extend_from_slice(b"*/\nint big(void) { return 1; }\n");
    let tmp = project(
        "large",
        &[
            ("big.c", &big),
            ("small.c", b"int small(void) { return 1; }\n"),
        ],
    );
    let map = map(tmp.path());
    let f = file(&map, "big.c");
    assert!(f.too_large);
    assert!(!f.parsed);
    assert_eq!(f.compiled, None);
    assert!(f.defined.is_empty());
    assert_eq!(f.bytes, big.len() as u64);
    // Over the cap: hashed over its size and its first 8 MiB, never read
    // whole.
    let mut head = format!("{}\n", big.len()).into_bytes();
    head.extend_from_slice(&big[..MAX_SOURCE_BYTES as usize]);
    assert_eq!(f.blake3, harness_core::hash::bytes_hash(&head));
    assert_ne!(f.blake3, harness_core::hash::bytes_hash(&big));
    assert_eq!(file(&map, "small.c").compiled, Some(Compiled::Ok));
    // A file under the cap keeps the plain hash of its bytes.
    assert_eq!(
        file(&map, "small.c").blake3,
        harness_core::hash::bytes_hash(b"int small(void) { return 1; }\n")
    );
}

/// A sparse header claiming 16 GiB (no blocks on disk) is hashed by its
/// size and head in well under a second, not streamed whole (12 s in the
/// security check; a terabyte would take minutes).
#[test]
fn a_sparse_giant_header_is_hashed_by_its_head() {
    let tmp = project("sparse", &[("a.c", b"int a(void) { return 1; }\n")]);
    let big = tmp.path().join("big.h");
    let f = std::fs::File::create(&big).expect("create");
    f.set_len(16 << 30).expect("a sparse file");
    drop(f);
    let started = std::time::Instant::now();
    let hash = map_hash(&big, 16 << 30).expect("hashed");
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(4), "{took:?}");
    let mut head = format!("{}\n", 16u64 << 30).into_bytes();
    head.resize(head.len() + MAX_SOURCE_BYTES as usize, 0);
    assert_eq!(hash, harness_core::hash::bytes_hash(&head));
    let map = map(tmp.path());
    let h = file(&map, "big.h");
    assert!(h.too_large);
    assert_eq!(h.blake3, hash);
}

#[test]
fn a_non_utf8_c_file_is_parsed_and_compiled() {
    let tmp = project(
        "latin1",
        &[("lat.c", b"/* caf\xe9 */\nint lat(void) { return 1; }\n")],
    );
    let map = map(tmp.path());
    let f = file(&map, "lat.c");
    assert!(f.not_utf8);
    assert!(f.parsed);
    assert_eq!(f.functions, 1);
    assert_eq!(f.compiled, Some(Compiled::Ok), "{:?}", f.message);
    assert_eq!(defined(f), ["lat"]);
}

#[test]
fn names_from_a_file_outside_the_root_are_withheld_and_counted() {
    let outside = TempDir::new("outside");
    let name = outside.path().join("name.txt");
    std::fs::write(&name, "leaked_name").expect("outside file");
    let src = format!(
        "int\n#include \"{}\"\n= 1;\nint kept(void) {{ return 2; }}\n",
        name.display()
    );
    let tmp = project("withheld", &[("a.c", src.as_bytes())]);
    let map = map(tmp.path());
    let a = file(&map, "a.c");
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    assert!(a.outside_includes);
    assert!(a.defined.is_empty() && a.needed.is_empty(), "{a:?}");
    assert_eq!(a.withheld_names, 2, "leaked_name and kept, counted");
}

/// A failed compile's `-MD` list is read too: one that read outside the
/// root keeps neither where it stopped nor the header it missed (either
/// can tell what exists outside); only the one bit stays.
#[test]
fn a_failed_compile_that_read_outside_keeps_no_place_or_header() {
    let outside = TempDir::new("outside-failed");
    let name = outside.path().join("probe.h");
    std::fs::write(&name, "int probed = 1;\n").expect("outside file");
    // (A compile stopped by a missing header writes no list — clang's fatal
    // error — so that case cannot be told; design §6 says so.)
    let a = format!("#include \"{}\"\n#error stop\n", name.display());
    let tmp = project(
        "outside-failed",
        &[("a.c", a.as_bytes()), ("c.c", b"\n#error stop\n")],
    );
    let map = map(tmp.path());
    let f = file(&map, "a.c");
    assert!(f.outside_includes);
    match &f.compiled {
        Some(Compiled::Failed { at, header, .. }) => {
            assert_eq!(at, &None);
            assert_eq!(header, &None);
        }
        other => panic!("{other:?}"),
    }
    // A failed compile that read nothing outside keeps its place.
    let c = file(&map, "c.c");
    assert!(!c.outside_includes);
    assert!(
        matches!(&c.compiled, Some(Compiled::Failed { at: Some(at), .. }) if at == "c.c:2"),
        "{:?}",
        c.compiled
    );
}

/// The parser's names are kept only for a `.c` that did not compile: the
/// one file the closure analysis asks them of.
#[test]
fn parser_facts_are_kept_only_for_a_c_file_that_did_not_compile() {
    let tmp = project(
        "parser-kept",
        &[
            ("ok.c", b"int ok(void) { return 1; }\n"),
            (
                "bad.c",
                b"int helper(void);\nint main(void) { return helper(); }\n#error x\n",
            ),
            ("h.h", b"int in_header(void);\n"),
        ],
    );
    let map = map(tmp.path());
    assert_eq!(map.parser.keys().collect::<Vec<_>>(), ["bad.c"]);
    let bad = &map.parser["bad.c"];
    assert!(bad.defines.contains("main") && bad.calls.contains("helper"));
}

#[test]
fn an_asm_labelled_odd_name_is_counted_never_kept() {
    let tmp = project(
        "odd",
        &[(
            "odd.c",
            b"int q __asm__(\"odd name\") = 1;\nint plain = 2;\n",
        )],
    );
    let map = map(tmp.path());
    let f = file(&map, "odd.c");
    assert_eq!(f.compiled, Some(Compiled::Ok), "{:?}", f.message);
    assert_eq!(f.odd_names, 1);
    assert_eq!(defined(f), ["plain"]);
    assert_eq!(f.defined[0].kind, "data");
}

#[cfg(target_os = "macos")]
#[test]
fn a_dollar_suffixed_libc_name_is_kept_by_its_base_name() {
    let tmp = project(
        "dollar",
        &[(
            "rp.c",
            b"#include <stdlib.h>\nchar *rp(char *b) { return realpath(\"a\", b); }\n",
        )],
    );
    let map = map(tmp.path());
    let f = file(&map, "rp.c");
    assert_eq!(f.compiled, Some(Compiled::Ok), "{:?}", f.message);
    assert!(needed(f).contains(&"realpath"), "{:?}", f.needed);
    assert!(f.needed.iter().all(|n| !n.name.contains('$')));
    assert_eq!(f.odd_names, 0);
}

#[test]
fn an_inc_file_the_compile_read_is_included_other() {
    let tmp = project(
        "inc",
        &[
            (
                "t.c",
                b"static const int t[] = {\n#include \"table.inc\"\n};\nint at(int i) { return t[i]; }\n",
            ),
            ("table.inc", b"1, 2, 3\n"),
        ],
    );
    let map = map(tmp.path());
    let f = file(&map, "t.c");
    assert_eq!(f.compiled, Some(Compiled::Ok), "{:?}", f.message);
    assert_eq!(f.included_other, ["table.inc"]);
    assert!(!f.outside_includes);
}

#[test]
fn a_root_that_is_or_holds_the_home_folder_is_refused() {
    let host = HostDirs::from_env().expect("HOME");
    for root in [host.home.as_path(), Path::new("/")] {
        let err = map_folder(root, Path::new("."), &MapOptions::default()).expect_err("refused");
        assert!(err.to_string().contains("home folder"), "{err}");
    }
}

#[test]
fn objects_and_dependency_lists_are_deleted_after_the_run() {
    let tmp = project(
        "deleted",
        &[
            ("a.c", b"int a(void) { return 1; }\n"),
            ("b.c", b"int b = ;\n"),
        ],
    );
    let parent = TempDir::new("deleted-parent");
    let map = map_folder_in(tmp.path(), Path::new("."), parent.path()).expect("maps");
    assert_eq!(file(&map, "a.c").compiled, Some(Compiled::Ok));
    let left: Vec<_> = std::fs::read_dir(parent.path())
        .expect("the parent")
        .map(|e| e.expect("entry").path())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
    let in_project: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("root")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(in_project.len(), 2, "nothing written into the project");
}

#[test]
fn a_dependency_list_is_read_as_make_writes_it() {
    let rule = "/f/0.o: /p/a\\ b.c /p/x.h \\\n  /p/c$$d.h /p/e\\#f.h\n";
    assert_eq!(
        dependency_paths(rule),
        ["/p/a b.c", "/p/x.h", "/p/c$d.h", "/p/e#f.h"]
    );
}

#[test]
fn the_system_folders_come_from_cc_dash_v() {
    let text = "ignored\n#include \"...\" search starts here:\n#include <...> search starts \
                here:\n /a/include\n /b/Frameworks (framework directory)\nEnd of search list.\n \
                /c\n";
    assert_eq!(
        system_include_dirs(text),
        [PathBuf::from("/a/include"), PathBuf::from("/b/Frameworks")]
    );
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn zopfli_maps_to_its_committed_target() {
    let root = repo().join("targets/zopfli");
    let config = harness_core::config::TargetConfig::load(&root).expect("harness.toml");
    let map = map_folder(
        &root,
        Path::new(&config.target.source_dir().unwrap()),
        &MapOptions::default(),
    )
    .expect("maps");
    assert_eq!(map.folder, "src/zopfli");
    let c: Vec<&FileFacts> = map.files.iter().filter(|f| f.kind == FileKind::C).collect();
    assert_eq!(c.len(), 13);
    for f in &c {
        assert_eq!(
            f.compiled,
            Some(Compiled::Ok),
            "{}: {:?}",
            f.path,
            f.message
        );
        assert!(!f.outside_includes, "{}", f.path);
    }
    for f in &map.files {
        assert_eq!(f.include_dirs, config.target.include_dirs(), "{}", f.path);
        assert!(f.ambiguous.is_empty(), "{}: {:?}", f.path, f.ambiguous);
    }
    // `-lm`'s symbols are among the needs.
    let needs: BTreeSet<&str> = c.iter().flat_map(|f| needed(f)).collect();
    assert!(needs.contains("log"), "{needs:?}");
    let defines: BTreeSet<&str> = c.iter().flat_map(|f| defined(f)).collect();
    assert!(defines.contains("main") && defines.contains("ZopfliCompress"));
    assert!(map
        .toolchain
        .system_include_dirs
        .iter()
        .all(|d| d.is_absolute()));
    assert!(!map.toolchain.system_include_dirs.is_empty());
}

#[test]
fn a_benchmark_case_maps_to_the_include_folder_bench_init_writes() {
    let root = repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/ima_decode_lib");
    let config = harness_core::config::TargetConfig::load(&root).expect("harness.toml");
    assert_eq!(config.target.include_dirs(), ["test_case/include"]);
    let map = map_folder(
        &root,
        Path::new(&config.target.source_dir().unwrap()),
        &MapOptions::default(),
    )
    .expect("maps");
    let lib = file(&map, "test_case/src/lib.c");
    assert_eq!(lib.include_dirs, config.target.include_dirs());
    assert!(lib.ambiguous.is_empty());
    assert_eq!(lib.compiled, Some(Compiled::Ok), "{:?}", lib.message);
    assert!(defined(lib).contains(&"ima_decode"));
}
