//! Tests of step (b), part 1 (docs/PROJECT-MAP-DESIGN.md §4): the
//! configuration file, `compile_commands.json` read as evidence, the
//! compile's flags, the caps and the set-aside counts.

use super::config::{self, ConfigSource};
use super::evidence::{split_command, CompileCommands};
use super::*;
use crate::testutil::TempDir;
use harness_core::config::ConfigurationFrom;

fn project(tag: &str, files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new(tag);
    for (rel, text) in files {
        let path = tmp.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
        std::fs::write(&path, text).expect("write");
    }
    tmp
}

fn map_with(root: &Path, options: &MapOptions) -> Result<FolderMap, Error> {
    // The test process's own adoption file, never the person's.
    harness_core::adopt::testing::adoption_file();
    let parent = TempDir::new("map-parent-b");
    map_in(root, Path::new("."), options, parent.path())
}

/// The person's own `config.toml`: the root recorded on this computer as
/// adopted (`--adopt`), so its configuration is stated, not proposed.
fn stated(root: &Path) {
    harness_core::adopt::testing::adopt(root);
}

fn map_default(root: &Path) -> FolderMap {
    map_with(root, &MapOptions::default()).expect("the map runs")
}

fn file<'a>(map: &'a FolderMap, path: &str) -> &'a FileFacts {
    map.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is mapped"))
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
    std::fs::write(path, text).expect("write");
}

/// A `compile_commands.json` from `(directory, file, arguments)` entries.
fn compile_commands(entries: &[(&Path, &str, &[&str])]) -> String {
    let list: Vec<serde_json::Value> = entries
        .iter()
        .map(|(dir, file, args)| {
            serde_json::json!({
                "directory": dir.to_str().expect("utf-8"),
                "file": file,
                "arguments": args,
                "output": "/tmp/never-used.o",
            })
        })
        .collect();
    serde_json::to_string(&list).expect("json")
}

fn ignored<'a>(map: &'a FolderMap, flag: &str) -> &'a str {
    map.evidence
        .ignored_flags
        .iter()
        .find(|f| f.flag == flag)
        .map(|f| f.why.as_str())
        .unwrap_or_else(|| panic!("{flag} is named: {:?}", map.evidence.ignored_flags))
}

const NEEDS_CONFIG: &str =
    "#ifndef HAVE_CONFIG_H\n#error no config\n#endif\nint a(void) { return 1; }\n";

#[test]
fn a_two_argument_isystem_and_a_relative_include_folder_are_read_against_directory() {
    let tmp = project(
        "cc-forms",
        &[
            (
                "src/a.c",
                "#include <v.h>\n#include \"api.h\"\nint a(void) { return V + API; }\n",
            ),
            ("vendor/inc/v.h", "#define V 1\n"),
            ("src/include/api.h", "#define API 2\n"),
        ],
    );
    let root = tmp.path();
    let build = root.join("build");
    let vendor = root.join("vendor/inc");
    write(
        root,
        "build/compile_commands.json",
        &compile_commands(&[(
            &build,
            "../src/a.c",
            &[
                "cc",
                "-isystem",
                vendor.to_str().expect("utf-8"),
                "-I",
                "../src/include",
                "-I../src/include",
                "-c",
                "../src/a.c",
            ],
        )]),
    );
    let map = map_default(root);
    assert_eq!(
        map.evidence.compile_commands,
        CompileCommands::Present {
            path: "build/compile_commands.json".into()
        }
    );
    let a = file(&map, "src/a.c");
    assert_eq!(
        a.flags,
        ["-isystemvendor/inc", "-Isrc/include", "-Isrc/include"]
    );
    assert_eq!(map.evidence.ignored_flag_count, 0, "{:?}", map.evidence);
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    assert_eq!(map.configuration.source, ConfigSource::Guessed);
    assert_eq!(map.configuration.from, ConfigurationFrom::CompileCommands);
}

#[test]
fn a_compile_commands_flag_outside_the_grammar_is_refused_by_name() {
    let tmp = project(
        "cc-refused",
        &[
            ("a.c", "int a(void) { return 1; }\n"),
            ("opts.txt", "-fplugin=evil.so\n"),
            (
                "evil-cc",
                "#!/bin/sh\ntouch \"$(dirname \"$0\")/ran-the-entry-compiler\"\n",
            ),
        ],
    );
    let root = tmp.path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.join("evil-cc"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    let evil = root.join("evil-cc");
    let ld = format!("-fuse-ld={}", root.join("ld").display());
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[
            (
                root,
                "a.c",
                &[
                    evil.to_str().expect("utf-8"),
                    "@opts.txt",
                    "-I",
                    "@opts.txt",
                    "-I@opts.txt",
                    "-B/x",
                    "-fplugin=x.so",
                    &ld,
                    "-Xclang",
                    "-load",
                    "-o",
                    "out.o",
                    "-MF",
                    "dep.d",
                    "-include",
                    "/etc/hosts",
                    "-DKEPT",
                    "-c",
                    "a.c",
                ],
            ),
            // A directory outside the root: the whole entry is ignored.
            (Path::new("/"), "a.c", &["cc", "-DOUTSIDE", "-c", "a.c"]),
        ]),
    );
    let map = map_default(root);
    for (flag, says) in [
        ("@opts.txt", "is not one the harness passes"),
        ("-I @opts.txt", "starting with `@`"),
        ("-I@opts.txt", "starting with `@`"),
        ("-B/x", "is not one the harness passes"),
        ("-fplugin=x.so", "is not one the harness passes"),
        (ld.as_str(), "is not one the harness passes"),
        ("-Xclang -load", "is not one the harness passes"),
        ("-include /etc/hosts", "outside the project"),
    ] {
        let why = ignored(&map, flag);
        assert!(why.contains(says), "{flag}: {why}");
    }
    // The build's bookkeeping (each entry's own `-o`, the `-M` family) is
    // dropped silently, as `-c` is: never named, never kept.
    assert!(
        map.evidence
            .ignored_flags
            .iter()
            .all(|f| !f.flag.starts_with("-o") && !f.flag.starts_with("-M")),
        "{:?}",
        map.evidence.ignored_flags
    );
    assert_eq!(map.evidence.ignored_entries, 1);
    let a = file(&map, "a.c");
    assert_eq!(a.flags, ["-DKEPT"]);
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    assert!(
        !root.join("ran-the-entry-compiler").exists(),
        "the entry's own compiler never runs"
    );
    assert!(!root.join("out.o").exists() && !root.join("dep.d").exists());
}

#[test]
fn a_positive_define_from_compile_commands_makes_a_file_compile() {
    let files = [("a.c", NEEDS_CONFIG)];
    let without = project("cc-config-no", &files);
    let a = map_default(without.path());
    assert!(
        matches!(
            file(&a, "a.c").compiled,
            Some(Compiled::Failed {
                reason: Reason::Syntax,
                ..
            })
        ),
        "{:?}",
        file(&a, "a.c").compiled
    );
    let with = project("cc-config-yes", &files);
    let root = with.path();
    // `command`, split by shell rules: quotes and a backslash.
    let entry = serde_json::json!([{
        "directory": root.to_str().expect("utf-8"),
        "file": "a.c",
        "command": "/usr/bin/cc '-DHAVE_CONFIG_H' -DNAME=\"a b\" -DX=1\\ 2 -c a.c",
    }]);
    write(root, "compile_commands.json", &entry.to_string());
    let map = map_default(root);
    let a = file(&map, "a.c");
    assert_eq!(a.flags, ["-DHAVE_CONFIG_H", "-DNAME=a b", "-DX=1 2"]);
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
}

#[test]
fn entries_that_differ_for_one_file_give_a_flags_differ_fact_and_stay_guessed() {
    let tmp = project(
        "cc-differ",
        &[
            ("a.c", "int a(void) { return 1; }\n"),
            ("b.c", "int b(void) { return 2; }\n"),
            ("missing-dir/.keep", ""),
        ],
    );
    let root = tmp.path();
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[
            (root, "a.c", &["cc", "-DA", "-c", "a.c"]),
            (root, "a.c", &["cc", "-DB", "-c", "a.c"]),
            (root, "b.c", &["cc", "-DONLY_B", "-c", "b.c"]),
            (root, "gone.c", &["cc", "-c", "gone.c"]),
        ]),
    );
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"cdb\"\nfrom = \"compile_commands\"\nflags = []\n",
    );
    stated(root);
    let map = map_default(root);
    assert_eq!(map.evidence.flags_differ.len(), 1);
    assert_eq!(map.evidence.flags_differ[0].path, "a.c");
    assert_eq!(
        map.evidence.flags_differ[0].flags,
        [vec!["-DA".to_string()], vec!["-DB".to_string()]]
    );
    assert_eq!(map.evidence.unfound_entries, ["gone.c"]);
    assert_eq!(map.configuration.source, ConfigSource::Guessed);
    // Each file takes its own entry's flags.
    assert_eq!(file(&map, "a.c").flags, ["-DA"]);
    assert_eq!(file(&map, "b.c").flags, ["-DONLY_B"]);

    // Without the second entry, the stated compile_commands source holds.
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[(root, "a.c", &["cc", "-DA", "-c", "a.c"])]),
    );
    let map = map_default(root);
    assert!(map.evidence.flags_differ.is_empty());
    assert_eq!(map.configuration.source, ConfigSource::CompileCommands);
}

#[test]
fn two_configurations_and_no_name_are_refused_naming_them() {
    let tmp = project("two-configs", &[("a.c", NEEDS_CONFIG)]);
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\"-DHAVE_CONFIG_H\"]\n\n\
         [[configuration]]\nname = \"cmake\"\nfrom = \"cmake\"\nflags = []\n",
    );
    stated(root);
    let err = map_with(root, &MapOptions::default())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("several configurations (make, cmake)") && err.contains("--configuration"),
        "{err}"
    );
    let err = map_with(
        root,
        &MapOptions {
            configuration: Some("meson".into()),
            ..MapOptions::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("--configuration meson names no configuration"),
        "{err}"
    );
    assert!(err.contains("make, cmake"), "{err}");
    let map = map_with(
        root,
        &MapOptions {
            configuration: Some("make".into()),
            ..MapOptions::default()
        },
    )
    .expect("maps under make");
    assert_eq!(map.configuration.name, "make");
    assert_eq!(map.configuration.source, ConfigSource::Stated);
    assert_eq!(map.configuration.flags, ["-DHAVE_CONFIG_H"]);
    assert_eq!(file(&map, "a.c").compiled, Some(Compiled::Ok));
}

#[test]
fn system_headers_pass_the_project_folder_after_the_system() {
    let tmp = project(
        "sys-headers",
        &[
            (
                "a.c",
                "#include <other.h>\n#include <unistd.h>\nint a(void) { return OTHER + (int)getpid(); }\n",
            ),
            ("compat/other.h", "#define OTHER 1\n"),
            ("compat/unistd.h", "#error the project's unistd.h\n"),
        ],
    );
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"mine\"\nfrom = \"stated\"\nflags = []\n\
         system_headers = [\"unistd.h\"]\n",
    );
    let map = map_default(root);
    let a = file(&map, "a.c");
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    assert_eq!(a.ambiguous.len(), 1);
    assert_eq!(a.ambiguous[0].used.as_deref(), Some("system"));
    let walked: BTreeSet<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
    let argv = compile_argv(
        &map.root,
        &walked,
        &map.configuration.system_headers,
        a,
        &a.flags,
        Path::new("/o"),
        Path::new("/d"),
    )
    .expect("argv");
    let compat = map.root.join("compat").display().to_string();
    let at = argv
        .iter()
        .position(|x| x == "-idirafter")
        .expect("-idirafter");
    assert_eq!(argv[at + 1], compat);
    assert!(!argv.contains(&format!("-I{compat}")), "{argv:?}");
    // Without the configuration, the project's header shadows the system's.
    std::fs::remove_file(root.join(config::CONFIG_FILE)).expect("rm");
    let map = map_default(root);
    assert!(file(&map, "a.c").compiled != Some(Compiled::Ok));
}

/// A configuration's `-idirafter<dir>` is in the grammar (fix pass F): the
/// map compiles with it as a two-argument option after the system's folders,
/// and that folder settles the ambiguous include it holds.
#[test]
fn an_idirafter_flag_compiles_after_the_system_and_settles_its_include() {
    let tmp = project(
        "idirafter",
        &[
            (
                "a.c",
                "#include <other.h>\n#include <unistd.h>\nint main(void) { return OTHER + (int)getpid(); }\n",
            ),
            ("compat/other.h", "#define OTHER 1\n"),
            ("compat/unistd.h", "#error the project's unistd.h\n"),
        ],
    );
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"mine\"\nfrom = \"stated\"\nflags = [\"-idiraftercompat\"]\n",
    );
    let map = map_default(root);
    let a = file(&map, "a.c");
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    assert_eq!(a.ambiguous.len(), 1);
    assert_eq!(a.ambiguous[0].used.as_deref(), Some("system"));
    let walked: BTreeSet<&str> = map.files.iter().map(|f| f.path.as_str()).collect();
    let argv = compile_argv(
        &map.root,
        &walked,
        &map.configuration.system_headers,
        a,
        &a.flags,
        Path::new("/o"),
        Path::new("/d"),
    )
    .expect("argv: the grammar's path flags are the map's");
    let compat = map.root.join("compat").display().to_string();
    let at = argv
        .iter()
        .position(|x| x == "-idirafter")
        .expect("-idirafter as two arguments");
    assert_eq!(argv[at + 1], compat);
    // The folder the person named settles the ambiguous include.
    let mut map = map;
    let analysis = super::mapfile::analyze(&mut map).expect("analysis");
    let rendered = super::mapfile::render(&map, analysis.as_ref()).expect("render");
    let closure = rendered
        .closures
        .iter()
        .find(|c| c.program == "t-a")
        .expect("one program, t-a");
    assert!(
        closure.ambiguous_unsettled.is_empty(),
        "the -idirafter folder settles the include: {:?}",
        closure.ambiguous_unsettled
    );
}

#[test]
fn an_optimisation_level_is_recorded_and_never_applied() {
    let tmp = project(
        "o0",
        &[(
            "a.c",
            "#ifndef __OPTIMIZE__\n#error compiled without optimisation\n#endif\nint a(void) { return 1; }\n",
        )],
    );
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"debug\"\nfrom = \"stated\"\nflags = [\"-O0\", \"-DX\"]\n",
    );
    let map = map_default(root);
    assert_eq!(map.configuration.flags, ["-O0", "-DX"]);
    let a = file(&map, "a.c");
    assert_eq!(a.flags, ["-O0", "-DX"]);
    assert_eq!(a.compiled, Some(Compiled::Ok), "{:?}", a.message);
    let argv = compile_argv(
        &map.root,
        &BTreeSet::new(),
        &[],
        a,
        &a.flags,
        Path::new("/o"),
        Path::new("/d"),
    )
    .expect("argv");
    assert!(argv.contains(&"-O2".to_string()), "{argv:?}");
    assert!(!argv.contains(&"-O0".to_string()), "{argv:?}");
    assert!(argv.contains(&"-DX".to_string()), "{argv:?}");
}

#[test]
fn the_name_cap_and_the_budget_stop_the_compiles_and_say_so() {
    let files = [
        (
            "a.c",
            "int a1(void) { return 1; }\nint a2(void) { return 2; }\n",
        ),
        ("b.c", "int b(void) { return 3; }\n"),
    ];
    let tmp = project("caps", &files);
    let root = tmp.path();
    let map = map_with(
        root,
        &MapOptions {
            limits: Limits {
                max_symbol_names: 1,
                ..Limits::default()
            },
            ..MapOptions::default()
        },
    )
    .expect("maps");
    assert_eq!(map.limits_hit.len(), 1);
    assert_eq!(map.limits_hit[0].limit, "symbols");
    assert!(!map.closures_possible());
    assert_eq!(map.files.len(), 2, "the facts are written");
    assert_eq!(file(&map, "a.c").compiled, Some(Compiled::Ok));
    assert_eq!(file(&map, "b.c").compiled, None, "not reached");

    let map = map_with(
        root,
        &MapOptions {
            limits: Limits {
                budget: Duration::ZERO,
                ..Limits::default()
            },
            ..MapOptions::default()
        },
    )
    .expect("maps");
    // The budget reaches the hash loop first: nothing past it is visited.
    assert_eq!(map.limits_hit[0].limit, "budget");
    assert!(!map.closures_possible());
    assert!(map.files.is_empty(), "{:?}", map.files);

    assert!(map_default(root).closures_possible());
}

/// The deadline reaches the link checks too: a map whose time ran out
/// during them records the budget limit and has no closures.
#[test]
fn the_budget_stops_the_link_checks() {
    let tmp = project(
        "budget-link",
        &[
            ("main.c", "int f(void);\nint main(void) { return f(); }\n"),
            ("f.c", "int f(void) { return 0; }\n"),
        ],
    );
    let mut map = map_default(tmp.path());
    assert!(map.closures_possible());
    map.deadline = std::time::Instant::now();
    let analysis = mapfile::analyze(&mut map).expect("the analysis runs");
    assert!(analysis.is_none());
    assert_eq!(map.limits_hit.len(), 1);
    assert_eq!(map.limits_hit[0].limit, "budget");
    assert!(
        map.limits_hit[0].at.contains("link checks"),
        "{:?}",
        map.limits_hit
    );

    // With time left, the same map links.
    let mut map = map_default(tmp.path());
    let analysis = mapfile::analyze(&mut map)
        .expect("the analysis runs")
        .expect("closures");
    assert_eq!(analysis.closures[0].linked, Some(closure::Linked::Ok));
}

/// Over the name caps: a name over 4 KiB is an odd name, and the total
/// bytes of kept names is a limit beside the count of distinct names.
#[test]
fn a_long_symbol_name_is_odd_and_name_bytes_have_a_cap() {
    let long = "x".repeat(MAX_NAME_BYTES + 1);
    let a_src = format!("int {long}(void) {{ return 1; }}\nint a(void) {{ return 2; }}\n");
    let tmp = project(
        "name-bytes",
        &[
            ("a.c", a_src.as_str()),
            ("b.c", "int b(void) { return 3; }\n"),
        ],
    );
    let root = tmp.path();
    let map = map_default(root);
    let a = file(&map, "a.c");
    assert_eq!(a.odd_names, 1, "{:?}", a.defined);
    assert!(a.defined.iter().all(|d| d.name.len() <= MAX_NAME_BYTES));

    let map = map_with(
        root,
        &MapOptions {
            limits: Limits {
                max_name_bytes: 0,
                ..Limits::default()
            },
            ..MapOptions::default()
        },
    )
    .expect("maps");
    assert_eq!(map.limits_hit.len(), 1, "{:?}", map.limits_hit);
    assert_eq!(map.limits_hit[0].limit, "symbols");
    assert!(map.limits_hit[0].at.contains("MiB of symbol names"));
    assert_eq!(file(&map, "b.c").compiled, None, "not reached");
}

#[test]
fn set_aside_files_are_counted_per_folder_and_build_files_found() {
    let tmp = project(
        "set-aside",
        &[
            ("src/a.c", "int a(void) { return 1; }\n"),
            ("src/png.cpp", "int x;\n"),
            ("src/more.cc", "int y;\n"),
            ("asm/fast.S", ".text\n"),
            ("lib/libz.a", "!<arch>\n"),
            ("Makefile", "all:\n"),
            ("sub/rules.mk", "x = 1\n"),
            ("meson.build", "project('x')\n"),
            (".hidden/skip.cpp", "int z;\n"),
        ],
    );
    let map = map_default(tmp.path());
    let got: Vec<(&str, &str, usize)> = map
        .set_aside
        .iter()
        .map(|s| (s.folder.as_str(), s.lang, s.count))
        .collect();
    assert_eq!(
        got,
        [
            ("asm", "assembly", 1),
            ("lib", "prebuilt", 1),
            ("src", "c++", 2)
        ]
    );
    assert!(map.others_complete);
    assert_eq!(
        map.evidence.build_files,
        ["Makefile", "meson.build", "sub/rules.mk"]
    );
    assert_eq!(map.evidence.compile_commands, CompileCommands::Absent);
}

#[test]
fn the_configuration_digest_changes_with_flag_order() {
    let ab = config::digest(
        "make",
        ConfigurationFrom::Make,
        &["-DA".into(), "-DB".into()],
        &[],
    );
    let ba = config::digest(
        "make",
        ConfigurationFrom::Make,
        &["-DB".into(), "-DA".into()],
        &[],
    );
    assert_ne!(ab, ba);
    assert!(ab.starts_with("blake3:") && ab.len() == 71);
    assert_eq!(
        ab,
        harness_core::hash::bytes_hash(br#"{"flags":["-DA","-DB"],"from":"make","name":"make"}"#)
    );
    assert_ne!(
        ab,
        config::digest(
            "make",
            ConfigurationFrom::Meson,
            &["-DA".into(), "-DB".into()],
            &[]
        )
    );
}

/// `system_headers` changes the compile (`-idirafter`) and what settles an
/// ambiguous include, so it moves the digest — and through it the map's
/// `inputs_hash` — while a configuration without it keeps its old digest.
#[test]
fn system_headers_move_the_configuration_digest() {
    let flags = ["-DA".to_string()];
    let none = config::digest("make", ConfigurationFrom::Make, &flags, &[]);
    let one = config::digest(
        "make",
        ConfigurationFrom::Make,
        &flags,
        &["stdio.h".to_string()],
    );
    assert_ne!(none, one);
    assert_eq!(
        one,
        harness_core::hash::bytes_hash(
            br#"{"flags":["-DA"],"from":"make","name":"make","system_headers":["stdio.h"]}"#
        )
    );

    // Through the map: adding `system_headers` to config.toml moves the
    // configuration's digest and the inputs hash.
    let tmp = project("sysh-digest", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []\n",
    );
    let before = map_default(root);
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []\n\
         system_headers = [\"stdio.h\"]\n",
    );
    let after = map_default(root);
    assert_ne!(before.configuration.digest, after.configuration.digest);
    let inputs = |m: &FolderMap| {
        let file = mapfile::render(m, None).expect("render");
        file.inputs_hash
    };
    assert_ne!(inputs(&before), inputs(&after));
}

#[test]
fn the_same_flags_from_config_toml_and_harness_toml_are_refused_alike() {
    let tmp = project("alike", &[("src/a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    for bad in [
        "-fuse-ld=/x",
        "-B/x",
        "-fplugin=x.so",
        "-Xclang",
        "-o",
        "-MF",
        "@args",
        "-I@f",
        "-I../outside",
        "-includemigration/x.h",
        "-DFOO BAR",
    ] {
        write(
            root,
            config::CONFIG_FILE,
            &format!("[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [{bad:?}]\n"),
        );
        let ours = config::read_entries(root).unwrap_err().to_string();
        write(
            root,
            "harness.toml",
            &format!(
                "schema_version = 2\n[target]\nname = \"x\"\n\
                 files = [{{ path = \"src/a.c\" }}]\n\
                 configuration = {{ name = \"make\", from = \"make\", flags = [{bad:?}] }}\n"
            ),
        );
        let theirs = harness_core::config::TargetConfig::load(root)
            .unwrap_err()
            .to_string();
        let sentence = harness_core::config::flags::check_flag(bad).unwrap_err();
        assert!(ours.contains(&sentence), "{bad}: {ours}");
        assert!(theirs.contains(&sentence), "{bad}: {theirs}");
        // And the map refuses before anything compiles.
        assert!(map_with(root, &MapOptions::default()).is_err(), "{bad}");
    }
}

#[test]
fn a_command_splits_by_shell_rules_without_expansion() {
    assert_eq!(
        split_command(r#"cc -DA="x y" 'it''s' a\ b $HOME *.c "q\"\$\n" -c"#).expect("splits"),
        ["cc", "-DA=x y", "its", "a b", "$HOME", "*.c", "q\"$\\n", "-c"]
    );
    assert_eq!(split_command("a ''  b").expect("splits"), ["a", "", "b"]);
    assert_eq!(split_command("a\\\nb").expect("splits"), ["ab"]);
    assert!(split_command("cc 'open").is_err());
    assert!(split_command("cc \"open").is_err());
}

#[test]
fn a_configuration_file_with_a_bad_entry_is_refused() {
    let tmp = project("bad-config", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    for (text, says) in [
        (
            "[[configuration]]\nname = \"a b\"\nfrom = \"make\"\nflags = []\n",
            "configuration name",
        ),
        (
            "[[configuration]]\nname = \"x\"\nfrom = \"ninja\"\nflags = []\n",
            "unknown variant",
        ),
        (
            "[[configuration]]\nname = \"x\"\nfrom = \"make\"\nflags = []\nextra = 1\n",
            "unknown field",
        ),
        (
            "[[configuration]]\nname = \"x\"\nfrom = \"make\"\nflags = []\n\
             system_headers = [\"../etc/passwd\"]\n",
            "system header",
        ),
        (
            "[[configuration]]\nname = \"x\"\nfrom = \"make\"\nflags = []\n\
             [[configuration]]\nname = \"x\"\nfrom = \"cmake\"\nflags = []\n",
            "two [[configuration]] entries are named `x`",
        ),
    ] {
        write(root, config::CONFIG_FILE, text);
        let err = config::read_entries(root).unwrap_err().to_string();
        assert!(err.contains(says), "{text}: {err}");
    }
}

/// A set's definers are written in path order by their numbers: `d1.2`
/// before `d1.10`.
#[test]
fn definers_are_written_in_order_of_their_numbers() {
    let mut files: Vec<(String, String)> = vec![(
        "main.c".into(),
        "int x(void);\nint main(void) { return x(); }\n".into(),
    )];
    for n in 1..=12 {
        files.push((format!("x/f{n:02}.c"), "int x(void) { return 0; }\n".into()));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let tmp = project("definer-order", &files);
    let mut map = map_default(tmp.path());
    let analysis = mapfile::analyze(&mut map).expect("analysis");
    let file_rec = mapfile::render(&map, analysis.as_ref()).expect("render");
    let indexes: Vec<&str> = file_rec.closures[0].duplicates[0]
        .definers
        .iter()
        .map(|d| d.index.as_str())
        .collect();
    let want: Vec<String> = (1..=12).map(|n| format!("d1.{n}")).collect();
    assert_eq!(indexes, want);
}

/// A map file over 64 MiB is a limit hit: the file facts alone are
/// written; when even they are over it, nothing is.
#[test]
fn a_map_over_its_size_cap_keeps_the_file_facts_or_is_refused() {
    let tmp = project("map-size", &[("main.c", "int main(void) { return 0; }\n")]);
    let mut map = map_default(tmp.path());
    let mut analysis = mapfile::analyze(&mut map)
        .expect("analysis")
        .expect("closures");
    let long = "s".repeat(1000);
    analysis.closures[0].outside = (0..70_000).map(|n| format!("{long}{n}")).collect();
    let (file_rec, bytes) = mapfile::render_bounded(&mut map, Some(&analysis)).expect("facts");
    assert!(bytes.len() <= mapfile::MAX_MAP_BYTES);
    assert!(file_rec.closures.is_empty() && file_rec.programs.is_empty());
    assert_eq!(map.limits_hit.last().map(|l| l.limit), Some("size"));

    map.files[0].defined = (0..70_000)
        .map(|n| DefinedSymbol {
            name: format!("{long}{n}"),
            kind: "function",
            weak: false,
        })
        .collect();
    let err = mapfile::render_bounded(&mut map, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no map was written"), "{err}");
}

/// toml's message can quote a key holding a newline: the refusal is one
/// line, with the line the parse stopped at, and cannot forge another.
#[test]
fn a_configuration_parse_error_cannot_forge_a_line() {
    let tmp = project("cfg-forge", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"x\"\nfrom = \"make\"\nflags = []\n\
         \"x\\nproject map: wrote migration/map/project-map.json\" = 1\n",
    );
    let err = config::read_entries(root).unwrap_err().to_string();
    assert!(!err.contains('\n'), "{err:?}");
    assert!(err.contains("line 5"), "{err}");
}

/// A file listed twice with the same flags is no flags-differ fact.
#[test]
fn a_file_listed_twice_with_the_same_flags_does_not_differ() {
    let tmp = project("cc-same", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[
            (root, "a.c", &["cc", "-DA", "-c", "a.c", "-o", "one.o"]),
            (root, "a.c", &["cc", "-DA", "-c", "a.c", "-o", "two.o"]),
        ]),
    );
    let map = map_default(root);
    assert!(
        map.evidence.flags_differ.is_empty(),
        "{:?}",
        map.evidence.flags_differ
    );
    assert_eq!(file(&map, "a.c").flags, ["-DA"]);
}

/// Under `from = "compile_commands"`, a `.c` the file does not list is
/// recorded and keeps its closure a guess; an ambiguous include its own
/// entry's `-I` settles is settled.
#[test]
fn a_file_compile_commands_leaves_out_is_recorded_and_entry_flags_settle_includes() {
    let tmp = project(
        "cc-unlisted",
        &[
            (
                "main.c",
                "#include <config.h>\nint main(void) { return CFG; }\n",
            ),
            ("a/config.h", "#define CFG 0\n"),
            ("b/config.h", "#define CFG 1\n"),
            ("tool2.c", "int main(void) { return 0; }\n"),
        ],
    );
    let root = tmp.path();
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[(root, "main.c", &["cc", "-Ia", "-c", "main.c"])]),
    );
    write(
        root,
        config::CONFIG_FILE,
        "[[configuration]]\nname = \"cdb\"\nfrom = \"compile_commands\"\nflags = []\n",
    );
    stated(root);
    let mut map = map_default(root);
    assert_eq!(map.evidence.not_in_compile_commands, ["tool2.c"]);
    assert_eq!(map.configuration.source, ConfigSource::CompileCommands);
    let analysis = mapfile::analyze(&mut map).expect("analysis");
    let file_rec = mapfile::render(&map, analysis.as_ref()).expect("render");
    assert_eq!(file_rec.build_evidence.not_in_compile_commands, ["tool2.c"]);
    // tool2.c's closure holds an unlisted file: the source stays a guess.
    assert_eq!(file_rec.configuration.source, "guessed");
    // main.c's `-Ia` (its entry's) settles `<config.h>`.
    let main = file_rec
        .closures
        .iter()
        .find(|c| c.program == "t-main")
        .expect("t-main");
    assert!(
        main.ambiguous_unsettled.is_empty(),
        "{:?}",
        main.ambiguous_unsettled
    );
}

/// The other `compile_commands.json` files one level down are named, not
/// read.
#[test]
fn other_compile_commands_files_are_named_as_also_found() {
    let tmp = project("cc-also", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    for dir in ["aaa", "build"] {
        write(
            root,
            &format!("{dir}/compile_commands.json"),
            &compile_commands(&[(root, "a.c", &["cc", "-c", "a.c"])]),
        );
    }
    let map = map_default(root);
    assert_eq!(
        map.evidence.compile_commands,
        CompileCommands::Present {
            path: "aaa/compile_commands.json".into()
        }
    );
    assert_eq!(map.evidence.also_found, ["build/compile_commands.json"]);
}

/// `compile_commands.json` is read into typed entries with caps: 64 flags
/// an entry (the rest counted as ignored), 50 000 entries (the rest counted
/// as ignored entries), and an entry of any other shape ignored, not held.
#[test]
fn compile_commands_entries_and_flags_are_capped() {
    let tmp = project("cc-caps", &[("a.c", "int a(void) { return 1; }\n")]);
    let root = tmp.path();
    let mut args: Vec<String> = vec!["cc".into()];
    for n in 0..100 {
        args.push(format!("-DX{n}"));
    }
    args.extend(["-c".to_string(), "a.c".to_string()]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    write(
        root,
        "compile_commands.json",
        &compile_commands(&[(root, "a.c", &args)]),
    );
    let map = map_default(root);
    assert_eq!(file(&map, "a.c").flags.len(), evidence::MAX_ENTRY_FLAGS);
    assert_eq!(
        map.evidence.ignored_flag_count,
        100 - evidence::MAX_ENTRY_FLAGS
    );

    // Past 50 000 entries the rest are counted, never read; `{"":0}` is an
    // entry of no use, ignored.
    let mut text = String::from("[");
    for _ in 0..evidence::MAX_ENTRIES + 3 {
        text.push_str("{\"\":0},");
    }
    text.push_str("{\"\":0}]");
    write(root, "compile_commands.json", &text);
    let map = map_default(root);
    assert_eq!(map.evidence.ignored_entries, evidence::MAX_ENTRIES + 4);
}
