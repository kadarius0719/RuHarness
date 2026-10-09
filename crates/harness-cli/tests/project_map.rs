//! `harness project map` through the binary (docs/PROJECT-MAP-DESIGN.md
//! §3.8, §5 step a): zopfli's folder, the refusals and their exit codes, and
//! a file name holding a newline in both output modes. Every child runs with
//! the test process's own adoption file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, canonical temporary folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-project-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir.canonicalize().unwrap())
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness_env(args: &[&str], env: &[(&str, &Path)]) -> Run {
    let file = harness_core::adopt::testing::adoption_file();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harness"));
    cmd.args(args).env(harness_core::adopt::ADOPTED_ENV, file);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn harness(args: &[&str]) -> Run {
    harness_env(args, &[])
}

/// Copy `src` into `dst`, leaving out the names in `skip` (relative to
/// `src`).
fn copy_dir(src: &Path, dst: &Path, skip: &[&str]) {
    fn walk(src: &Path, dst: &Path, rel: &str, skip: &[&str]) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let here = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if skip.contains(&here.as_str()) {
                continue;
            }
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&entry.path(), &dst.join(&name), &here, skip);
            } else if kind.is_file() {
                std::fs::copy(entry.path(), dst.join(&name)).unwrap();
            }
        }
    }
    walk(src, dst, "", skip);
}

/// The map file as JSON.
fn map_json(root: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(root.join("migration/map/project-map.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn map_bytes(root: &Path) -> Vec<u8> {
    std::fs::read(root.join("migration/map/project-map.json")).unwrap()
}

fn events(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l:?}: {e}")))
        .collect()
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn zopfli_without_its_harness_toml_is_one_program_of_thirteen_files() {
    let tmp = Tmp::new("zopfli");
    copy_dir(
        &repo().join("targets/zopfli"),
        &tmp.0,
        &["harness.toml", "migration/build", "migration/.lock"],
    );
    harness_core::adopt::testing::adopt(&tmp.0);
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"plain\"\nfrom = \"stated\"\nflags = []\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let map = map_json(&tmp.0);
    // One `main` program.
    let programs = map["programs"].as_array().unwrap();
    assert_eq!(programs.len(), 1, "{programs:?}");
    assert_eq!(programs[0]["path"], "src/zopfli/zopfli_bin.c");
    assert_eq!(programs[0]["kind"], "main");
    // Its closure: the 13 files of the committed `source_dir`, which the
    // committed target compiles with no include folders of its own.
    let closure = &map["closures"][0];
    let files = strings(&closure["files"]);
    let committed: Vec<String> = {
        let mut v: Vec<String> = std::fs::read_dir(repo().join("targets/zopfli/src/zopfli"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".c"))
            .map(|n| format!("src/zopfli/{n}"))
            .collect();
        v.sort();
        v
    };
    assert_eq!(files.len(), 13);
    assert_eq!(files, committed);
    assert_eq!(closure["linked"], "ok");
    assert_eq!(closure["incomplete"], false);
    for f in map["files"].as_array().unwrap() {
        if files.contains(&f["path"].as_str().unwrap().to_string()) {
            assert_eq!(f["include_dirs"], serde_json::json!([]), "{f}");
        }
    }
    assert!(
        run.stdout.contains("guessed libraries: -lm"),
        "{}",
        run.stdout
    );
    // zopflipng's C++ counted, never read; the harness's own folder pruned.
    assert!(map["set_aside"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["folder"] == "src/zopflipng" && s["lang"] == "c++"));
    assert!(run
        .stdout
        .contains("set aside in src/zopflipng: 2 c++ file(s), not read"));
    assert!(map["skipped_folders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["path"] == "migration"));
    assert!(run
        .stdout
        .contains("skipped folder: migration (the harness's own files)"));
    assert!(map["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| !f["path"].as_str().unwrap().starts_with("migration/")));
    assert_eq!(map["configuration"]["source"], "stated");
}

#[test]
fn a_benchmark_case_is_one_library_and_no_program() {
    let tmp = Tmp::new("bench");
    copy_dir(
        &repo().join("targets/tractor/cases/Public-Tests/B01_organic/crc16_lib/test_case"),
        &tmp.0,
        &[],
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let map = map_json(&tmp.0);
    assert_eq!(map["programs"], serde_json::json!([]));
    assert_eq!(map["closures"], serde_json::json!([]));
    let libraries = map["libraries"].as_array().unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0]["id"], "l-lib");
    assert_eq!(strings(&libraries[0]["files"]), ["src/lib.c"]);
    // The include folder `bench init` writes for the case.
    let lib = map["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/lib.c")
        .unwrap();
    assert_eq!(strings(&lib["include_dirs"]), ["include"]);
    assert!(run.stdout.contains("programs: 0"), "{}", run.stdout);
    assert!(
        run.stdout.contains("library l-lib: src/ lib.c"),
        "{}",
        run.stdout
    );
    // No program: nothing about what linking proves.
    assert!(!run.stdout.contains("what the link check proves"));
}

#[test]
fn a_newline_in_a_file_name_never_reaches_the_output_raw() {
    let tmp = Tmp::new("newline");
    tmp.write("a\nb.c", "int main(void) { return 0; }\n");
    tmp.write("ok.c", "int ok(void) { return 2; }\n");

    let run = harness(&["project", "map", "--target", tmp.arg(), "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let events = events(&run.stdout);
    let files: Vec<&serde_json::Value> =
        events.iter().filter(|e| e["k"] == "project-file").collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "a\nb.c");
    assert_eq!(files[0]["compiled"], true);
    assert_eq!(files[0]["defined"], 1);
    assert!(run.stdout.contains(r#""path":"a\nb.c""#), "{}", run.stdout);
    let program = events
        .iter()
        .find(|e| e["k"] == "project-program")
        .expect("a program event");
    assert_eq!(program["path"], "a\nb.c");
    assert!(run.stdout.lines().all(|l| !l.starts_with("b.c")));
    // Stored raw in the map file.
    assert_eq!(map_json(&tmp.0)["programs"][0]["path"], "a\nb.c");

    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains(" — a?b.c (main;"), "{}", run.stdout);
    assert!(
        run.stdout.lines().all(|l| !l.starts_with("b.c")),
        "{}",
        run.stdout
    );
}

#[test]
fn a_home_folder_root_is_refused() {
    let home = Tmp::new("home");
    home.write("a.c", "int a;\n");
    let run = harness_env(
        &["project", "map", "--target", home.arg()],
        &[("HOME", home.0.as_path())],
    );
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("is your home folder, which the map never reads"),
        "{}",
        run.stderr
    );
    assert!(!run.stdout.contains("compiled"), "{}", run.stdout);
}

#[test]
fn a_folder_without_c_is_refused() {
    let tmp = Tmp::new("empty");
    tmp.write("README", "no C here\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains("no C files (.c or .h) were found"),
        "{}",
        run.stderr
    );
    // Refused untouched: no migration/, no lock, no token, no record.
    assert!(!tmp.0.join("migration").exists());
    let adopted =
        std::fs::read_to_string(harness_core::adopt::testing::adoption_file()).unwrap_or_default();
    assert!(!adopted.contains(tmp.arg()), "{adopted}");
}

/// A refusal after the project lock (here `--configuration` naming none)
/// on a folder that had no `migration/` removes what the run made.
#[test]
fn a_refused_first_map_leaves_no_migration_folder() {
    let tmp = Tmp::new("refused-first");
    tmp.write("a.c", "int a(void) { return 1; }\n");
    let run = harness(&[
        "project",
        "map",
        "--target",
        tmp.arg(),
        "--configuration",
        "meson",
    ]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains("names no configuration"),
        "{}",
        run.stderr
    );
    assert!(!tmp.0.join("migration").exists(), "{}", run.stderr);
}

#[test]
fn a_ledger_made_elsewhere_is_refused_before_anything_compiles() {
    let tmp = Tmp::new("ledger");
    tmp.write("a.c", "int a;\n");
    tmp.write("migration/facts.jsonl", "");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("this folder already holds migration results made elsewhere"),
        "{}",
        run.stderr
    );
    assert!(!run.stdout.contains("compiled"), "{}", run.stdout);
}

#[test]
fn a_usage_error_exits_2() {
    let run = harness(&["project", "map", "--no-such-flag"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
}

#[test]
fn several_configurations_need_a_name_and_the_one_named_is_shown() {
    let tmp = Tmp::new("configs");
    tmp.write(
        "a.c",
        "#ifndef HAVE_CONFIG_H\n#error no config\n#endif\nint a(void) { return 1; }\n",
    );
    tmp.write("tools/png.cpp", "int x;\n");
    tmp.write("Makefile", "all:\n");
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\"-DHAVE_CONFIG_H\"]\n\n\
         [[configuration]]\nname = \"cmake\"\nfrom = \"cmake\"\nflags = []\n",
    );
    // A `migration/` folder holding only the person's config.toml still
    // asks for adoption once on this computer.
    harness_core::adopt::testing::adopt(&tmp.0);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains(
            "holds several configurations (make, cmake); pick one with --configuration NAME"
        ),
        "{}",
        run.stderr
    );
    let run = harness(&[
        "project",
        "map",
        "--target",
        tmp.arg(),
        "--configuration",
        "make",
    ]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    for says in [
        "configuration: make, from make (stated in config.toml), flags -DHAVE_CONFIG_H",
        "library l-a: ./ a.c",
        "build files: Makefile",
        "set aside in tools: 1 c++ file(s), not read",
    ] {
        assert!(run.stdout.contains(says), "{says}: {}", run.stdout);
    }
    let run = harness(&[
        "project",
        "map",
        "--target",
        tmp.arg(),
        "--configuration",
        "meson",
    ]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("--configuration meson names no configuration"),
        "{}",
        run.stderr
    );
}

/// A made-up project with three `main()`s (a tool at the root, one under
/// `tools/`, a test under `tests/`), a shared file, a duplicate both of
/// whose definers link, and a README.
fn three_mains(tmp: &Tmp) {
    tmp.write(
        "lib/shared.h",
        "int shared_add(int a, int b);\ndouble shared_root(double x);\n",
    );
    tmp.write(
        "lib/shared.c",
        "#include <math.h>\n#include \"shared.h\"\nint shared_add(int a, int b) { return a + b; }\n\
         double shared_root(double x) { return sqrt(x); }\n",
    );
    tmp.write("lib/fast.c", "int pick(int x) { return x + 1; }\n");
    tmp.write("lib/slow.c", "int pick(int x) { return x + 2; }\n");
    tmp.write(
        "alpha.c",
        "#include <stdio.h>\n#include \"lib/shared.h\"\n\
         int main(void) { printf(\"%d\\n\", shared_add(1, 2)); return 0; }\n",
    );
    tmp.write(
        "tools/beta.c",
        "#include <stdio.h>\n#include \"../lib/shared.h\"\nint pick(int x);\n\
         int main(void) { printf(\"%f %d\\n\", shared_root(4.0), pick(1)); return 0; }\n",
    );
    tmp.write(
        "tests/test_shared.c",
        "#include \"shared.h\"\nint main(void) { return shared_add(1, 1) == 2 ? 0 : 1; }\n",
    );
    tmp.write("README", "a made-up project\n");
}

#[test]
fn three_mains_are_mapped_written_and_shown() {
    let tmp = Tmp::new("three");
    three_mains(&tmp);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let map = map_json(&tmp.0);
    assert_eq!(map["schema"], "ruharness-project-map");
    assert_eq!(map["schema_version"], 1);
    let ids: Vec<&str> = map["programs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["t-alpha", "t-beta", "t-test_shared"]);
    let program = |id: &str| {
        map["programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(program("t-alpha")["index"], "p1");
    assert_eq!(program("t-test_shared")["index"], "p2");
    assert_eq!(program("t-beta")["index"], "p3");
    assert_eq!(program("t-test_shared")["kind_guess"], "test");
    assert_eq!(program("t-alpha")["kind_guess"], "tool");
    let closure = |id: &str| {
        map["closures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["program"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        strings(&closure("t-alpha")["files"]),
        ["alpha.c", "lib/shared.c"]
    );
    assert_eq!(closure("t-alpha")["linked"], "ok");
    assert_eq!(strings(&closure("t-alpha")["outside"]), ["printf"]);
    // The duplicate both of whose definers link: held, a question.
    let beta = closure("t-beta");
    assert_eq!(beta["incomplete"], true);
    assert_eq!(strings(&beta["questions"]), ["d1"]);
    assert_eq!(beta["duplicates"][0]["definers"][0]["path"], "lib/fast.c");
    assert_eq!(beta["duplicates"][0]["definers"][1]["path"], "lib/slow.c");
    assert!(beta.get("linked").is_none());
    assert_eq!(map["shared"][0]["file"], "lib/shared.c");
    assert_eq!(
        strings(&map["shared"][0]["programs"]),
        ["t-alpha", "t-beta", "t-test_shared"]
    );
    // No source text: the README and the C bodies never appear.
    let text = String::from_utf8(map_bytes(&tmp.0)).unwrap();
    assert!(!text.contains("return a + b") && !text.contains("made-up"));

    for says in [
        "the harness's own flags on every compile: -O2",
        "programs: 3",
        "  p1 t-alpha — alpha.c (main; kind guess from its folder: tool)",
        "      files: ./ alpha.c; lib/ shared.c",
        "      outside symbols: printf; guessed libraries: none",
        "      link check: linked",
        "  p2 t-test_shared — tests/test_shared.c (main; kind guess from its folder: test)",
        "      link check: not linked while d1 is open",
        "      duplicate set d1 (pick): held, linking cannot tell d1.1 lib/fast.c from d1.2 \
         lib/slow.c apart, so the choice is yours",
        "what the link check proves:",
        "shared file: lib/shared.c (in t-alpha, t-beta, t-test_shared)",
        // A guess: the next step is to state the configuration, the lines
        // of config.toml shown and `ask --build` named; accept is not.
        "project map: wrote migration/map/project-map.json and migration/.gitignore (3 \
         program(s), 0 libraries; the project's own files were not changed); the configuration \
         is a guess, so nothing can be accepted yet: next, state the build in \
         migration/map/config.toml, for example\n  [[configuration]]\n  name = \"plain\"\n  \
         from = \"stated\"\n  flags = []  # the -I and -D flags the build passes, each joined, \
         like \"-Isrc/include\"\nthen run `harness project map` again (or have a model propose \
         one: `harness project ask --build`)\n",
    ] {
        assert!(run.stdout.contains(says), "{says}\n{}", run.stdout);
    }
    assert!(!run.stdout.contains("project accept"), "{}", run.stdout);
    // The person's choice is never suggested.
    assert!(!run.stdout.contains("--keep d1=d1.1"), "{}", run.stdout);
    assert!(!run.stdout.contains("--keep d1=d1.2"), "{}", run.stdout);
    assert_eq!(run.stdout.matches("what the link check proves").count(), 1);
    // The alternatives are not "unreached".
    assert!(!run.stdout.contains("unreached"), "{}", run.stdout);

    // `--json`: the events beside the existing ones.
    let run = harness(&["project", "map", "--target", tmp.arg(), "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let events = events(&run.stdout);
    let kinds = |k: &str| events.iter().filter(|e| e["k"] == k).count();
    assert_eq!(kinds("project-file"), 7);
    assert_eq!(kinds("project-build"), 1);
    assert_eq!(kinds("project-program"), 3);
    assert_eq!(kinds("project-link"), 2);
    let beta = events
        .iter()
        .find(|e| e["k"] == "project-program" && e["id"] == "t-beta")
        .unwrap();
    assert_eq!(
        beta,
        &serde_json::json!({"k": "project-program", "id": "t-beta", "path": "tools/beta.c",
            "kind": "main", "kind_guess": "tool", "files": ["lib/shared.c", "tools/beta.c"],
            "outside": ["printf"], "incomplete": true, "held": ["d1"]})
    );
    let link = events
        .iter()
        .find(|e| e["k"] == "project-link" && e["id"] == "t-alpha")
        .unwrap();
    assert_eq!(link["ok"], true);

    // Stated: accept is the next step, the held choices named, none
    // suggested.
    harness_core::adopt::testing::adopt(&tmp.0);
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"plain\"\nfrom = \"stated\"\nflags = [\"-O2\"]\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    for says in [
        "project map: wrote migration/map/project-map.json (3 program(s), 0 libraries; the \
         project's own files were not changed); next, make a program or library a tool with \
         `harness project accept <id>`",
        "the held choices (d1) are yours to make",
        "--keep <set>=<index or path>",
        // `-O` is recorded only.
        "flags -O2; -O2 is recorded only, never applied (every compile keeps the harness's own)",
    ] {
        assert!(run.stdout.contains(says), "{says}\n{}", run.stdout);
    }
    assert!(!run.stdout.contains("--keep d1=d1.1"), "{}", run.stdout);
}

#[test]
fn a_program_that_does_not_link_says_what_is_missing() {
    let tmp = Tmp::new("nolink");
    tmp.write(
        "main.c",
        "int nowhere(int);\nint main(void) { return nowhere(1); }\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg(), "--json"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let link = events(&run.stdout)
        .into_iter()
        .find(|e| e["k"] == "project-link")
        .unwrap();
    assert_eq!(
        link,
        serde_json::json!({"k": "project-link", "id": "t-main", "missing": ["nowhere"],
            "doubled": []})
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert!(
        run.stdout
            .contains("link check: did not link; missing nowhere"),
        "{}",
        run.stdout
    );
}

#[test]
fn the_map_is_the_same_bytes_twice_and_its_hashes_move_with_their_inputs() {
    let tmp = Tmp::new("bytes");
    three_mains(&tmp);
    let map = || {
        let run = harness(&["project", "map", "--target", tmp.arg()]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        map_bytes(&tmp.0)
    };
    let first = map();
    assert_eq!(first, map(), "two runs, two different files");
    let hashes = |bytes: &[u8]| {
        let v: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        (
            v["root_hash"].as_str().unwrap().to_string(),
            v["inputs_hash"].as_str().unwrap().to_string(),
        )
    };
    let (root0, inputs0) = hashes(&first);
    // A README changes nothing.
    tmp.write("README", "edited\n");
    assert_eq!(hashes(&map()), (root0.clone(), inputs0.clone()));
    // A header moves root_hash, not inputs_hash.
    tmp.write(
        "lib/shared.h",
        "int shared_add(int a, int b);\ndouble shared_root(double x); /* edited */\n",
    );
    let (root1, inputs1) = hashes(&map());
    assert_ne!(root1, root0);
    assert_eq!(inputs1, inputs0);
    // A configuration moves inputs_hash, not root_hash.
    let config = |flags: &str| {
        tmp.write(
            "migration/map/config.toml",
            &format!("[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [{flags}]\n"),
        );
    };
    config("\"-DWIDE\"");
    let after = map();
    let (root2, inputs2) = hashes(&after);
    assert_eq!(root2, root1);
    assert_ne!(inputs2, inputs1);
    let v: serde_json::Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(strings(&v["configuration"]["flags"]), ["-DWIDE"]);
    assert_eq!(v["configuration"]["source"], "stated");
    // Flags keep their order, and the order counts.
    config("\"-DB\", \"-DA\"");
    let ba = map();
    let v: serde_json::Value = serde_json::from_slice(&ba).unwrap();
    assert_eq!(strings(&v["configuration"]["flags"]), ["-DB", "-DA"]);
    config("\"-DA\", \"-DB\"");
    assert_ne!(hashes(&ba).1, hashes(&map()).1);
}

#[test]
fn past_a_cap_the_file_facts_are_written_with_no_programs_and_exit_1() {
    let tmp = Tmp::new("cap");
    tmp.write("main.c", "int main(void) { return 0; }\n");
    // 33 folders deep: past the walk's depth cap of 32.
    let deep = (0..33)
        .map(|i| format!("d{i}"))
        .collect::<Vec<_>>()
        .join("/");
    tmp.write(&format!("{deep}/deep.c"), "int deep(void) { return 1; }\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("the map stopped at its depth limit of 32 folders deep"),
        "{}",
        run.stderr
    );
    // No summary when refusing.
    assert!(!run.stdout.contains("programs:"), "{}", run.stdout);
    let map = map_json(&tmp.0);
    assert_eq!(map["limits_hit"][0]["limit"], "depth");
    assert_eq!(map["programs"], serde_json::json!([]));
    assert_eq!(map["closures"], serde_json::json!([]));
    assert_eq!(map["files"][0]["path"], "main.c");
    assert_eq!(map["files"][0]["compiled"], "ok");
}

#[test]
fn the_project_lock_held_elsewhere_refuses_the_map() {
    let tmp = Tmp::new("lock");
    tmp.write("main.c", "int main(void) { return 0; }\n");
    // The first map makes the ledger here; then this test process holds
    // the project lock while the binary asks for it.
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let held = harness_core::ledger::WriterLock::acquire_project(&tmp.0, "a test").unwrap();
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("ledger is locked by another harness command"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("`a test`"), "{}", run.stderr);
    drop(held);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

#[test]
fn the_gitignore_is_written_by_the_first_map_and_never_overwritten() {
    let tmp = Tmp::new("ignore");
    tmp.write("main.c", "int main(void) { return 0; }\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let path = tmp.0.join("migration/.gitignore");
    let text = std::fs::read_to_string(&path).unwrap();
    for name in [
        "build/",
        ".lock",
        "traces/",
        ".promote-*/",
        ".*.prev/",
        ".replay-*/",
        "target/",
        ".ruharness-adopted",
        "map/.lock",
        "map/project-map.reply.json",
    ] {
        assert!(text.lines().any(|l| l == name), "{name}: {text}");
    }
    std::fs::write(&path, "the person's own\n").unwrap();
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "the person's own\n"
    );
    assert!(
        run.stdout
            .contains("project map: wrote migration/map/project-map.json ("),
        "{}",
        run.stdout
    );
}

/// A file-list tool over `alpha.c` and `lib/shared.c`, with `map` when
/// given.
fn write_tool(root: &Path, id: &str, map: Option<(&str, &str)>) {
    let mut text = String::from(
        "schema_version = 2\n[target]\nname = \"alpha\"\nfiles = [{ path = \"alpha.c\", \
         include_dirs = [] }, { path = \"lib/shared.c\", include_dirs = [] }]\n\
         configuration = { name = \"guessed\", from = \"stated\", flags = [] }\n",
    );
    if let Some((root_hash, inputs_hash)) = map {
        text.push_str(&format!(
            "map = {{ root_hash = \"{root_hash}\", inputs_hash = \"{inputs_hash}\" }}\n"
        ));
    }
    let dir = harness_core::config::tool_dir(root, id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("harness.toml"), text).unwrap();
}

#[test]
fn state_status_says_when_the_project_changed_since_a_tool_was_accepted() {
    let tmp = Tmp::new("notice");
    three_mains(&tmp);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let map = map_json(&tmp.0);
    let (root_hash, inputs_hash) = (
        map["root_hash"].as_str().unwrap().to_string(),
        map["inputs_hash"].as_str().unwrap().to_string(),
    );
    write_tool(&tmp.0, "t-alpha", Some((&root_hash, &inputs_hash)));
    write_tool(&tmp.0, "t-hand", None);
    let status = |tool: &str| {
        let run = harness(&["state", "status", "--target", tmp.arg(), "--tool", tool]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        run.stdout
    };
    const CHANGED: &str = "its own files changed since it was accepted (lib/shared.h); its \
                           closure, configuration and link are the same: scan it to read them \
                           (`harness scan --tool t-alpha`); accepting it again only clears this \
                           note";
    // The digests match: no notice.
    let out = status("t-alpha");
    assert!(
        !out.contains("changed") && !out.contains("elsewhere"),
        "{out}"
    );
    // A file elsewhere changes and the map is made again: the map says
    // there is nothing to do, and state status says nothing.
    tmp.write(
        "tools/beta.c",
        "#include <stdio.h>\n#include \"../lib/shared.h\"\nint pick(int x);\n\
         int main(void) { printf(\"%f %d\\n\", shared_root(5.0), pick(1)); return 0; }\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(
            "accepted tool t-alpha: a file elsewhere in the project changed; nothing to do for \
             this tool"
        ),
        "{}",
        run.stdout
    );
    let record = &map_json(&tmp.0)["accepted_tools"];
    assert_eq!(
        record,
        &serde_json::json!([{"id": "t-alpha", "changed": "none",
            "says": "a file elsewhere in the project changed; nothing to do for this tool"}])
    );
    let out = status("t-alpha");
    assert!(
        !out.contains("changed") && !out.contains("elsewhere"),
        "{out}"
    );
    // A header it includes changes and the map is made again: the notice
    // says the same sentence as the map, read from the map's record.
    tmp.write("lib/shared.h", "int shared_add(int a, int b);\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(&format!(
            "accepted tool t-alpha changed since it was accepted: {CHANGED}"
        )),
        "{}",
        run.stdout
    );
    assert_eq!(map_json(&tmp.0)["accepted_tools"][0]["changed"], "files");
    let out = status("t-alpha");
    assert!(out.contains(&format!("status: {CHANGED}")), "{out}");
    let run = harness(&[
        "state",
        "status",
        "--target",
        tmp.arg(),
        "--tool",
        "t-alpha",
        "--json",
    ]);
    assert!(
        events(&run.stdout)
            .iter()
            .any(|e| e["k"] == "project-notice" && e["says"] == CHANGED),
        "{}",
        run.stdout
    );
    // A hand-written tool with no `map`: never a notice.
    let out = status("t-hand");
    assert!(
        !out.contains("changed") && !out.contains("elsewhere"),
        "{out}"
    );
    // A map written before the records: the general sentence.
    let path = tmp.0.join("migration/map/project-map.json");
    let mut m = map_json(&tmp.0);
    m.as_object_mut().unwrap().remove("accepted_tools");
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    let out = status("t-alpha");
    assert!(
        out.contains(
            "status: the project changed since this tool was accepted: run `harness project \
             map`, then `accept` again"
        ),
        "{out}"
    );
    // No map file: said so.
    std::fs::remove_file(tmp.0.join("migration/map/project-map.json")).unwrap();
    let out = status("t-alpha");
    assert!(out.contains("status: no map written yet"), "{out}");
    let out = status("t-hand");
    assert!(!out.contains("no map"), "{out}");
}

#[test]
fn sync_runtime_takes_the_project_lock() {
    let tmp = Tmp::new("sync-lock");
    three_mains(&tmp);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    write_tool(&tmp.0, "t-hand", None);
    let held = harness_core::ledger::WriterLock::acquire_project(&tmp.0, "a test").unwrap();
    let run = harness(&["sync-runtime", "--target", tmp.arg(), "--tool", "t-hand"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr
            .contains("ledger is locked by another harness command (pid"),
        "{}",
        run.stderr
    );
    drop(held);
    // Free again: it goes on to its own next step (no facts yet).
    let run = harness(&["sync-runtime", "--target", tmp.arg(), "--tool", "t-hand"]);
    assert!(!run.stderr.contains("locked"), "{}", run.stderr);
}

#[test]
fn per_file_flags_that_differ_inside_a_program_keep_the_configuration_a_guess() {
    let tmp = Tmp::new("differ");
    tmp.write(
        "main.c",
        "int helper(void);\nint main(void) { return helper(); }\n",
    );
    tmp.write("helper.c", "int helper(void) { return 0; }\n");
    let dir = tmp.arg().replace('\\', "\\\\");
    tmp.write(
        "compile_commands.json",
        &format!(
            "[{{\"directory\": \"{dir}\", \"file\": \"main.c\", \"arguments\": [\"cc\", \"-DA\", \
             \"-c\", \"main.c\"]}}, {{\"directory\": \"{dir}\", \"file\": \"helper.c\", \
             \"arguments\": [\"cc\", \"-DB\", \"-c\", \"helper.c\"]}}]"
        ),
    );
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"cc\"\nfrom = \"compile_commands\"\nflags = []\n",
    );
    harness_core::adopt::testing::adopt(&tmp.0);
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let map = map_json(&tmp.0);
    let closure = &map["closures"][0];
    assert!(closure.get("flags").is_none(), "{closure}");
    assert_eq!(
        closure["flags_differ"],
        serde_json::json!([{"path": "helper.c", "flags": ["-DB"]},
            {"path": "main.c", "flags": ["-DA"]}])
    );
    assert_eq!(map["configuration"]["source"], "guessed");
    assert_eq!(map["build_evidence"]["compile_commands"], "present");
    assert!(
        run.stdout
            .contains("flags differ between its files, so the configuration stays a guess"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("configuration: cc, from compile_commands (still a guess:"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_compile_stopped_by_an_error_directive_or_a_missing_header_is_named() {
    let tmp = Tmp::new("errors");
    tmp.write(
        "inc/pair.h",
        "#ifndef PAIR_WIDE\n#error PAIR_WIDE is needed\n#endif\n",
    );
    tmp.write(
        "a.c",
        "#include \"inc/pair.h\"\nint a(void) { return 1; }\n",
    );
    tmp.write(
        "b.c",
        "#include \"proj/missing.h\"\nint b(void) { return 2; }\n",
    );
    tmp.write(
        "c.c",
        "#include \"/nonexistent-ruharness/x.h\"\nint c(void) { return 3; }\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    for says in [
        "did not compile: a.c — stopped at an #error directive at inc/pair.h:2",
        "did not compile: b.c — a header was not found (proj/missing.h) at b.c:1",
        "did not compile: c.c — a header was not found at c.c:1",
    ] {
        assert!(run.stdout.contains(says), "{says}\n{}", run.stdout);
    }
    let map = map_json(&tmp.0);
    let file = |p: &str| {
        map["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == p)
            .unwrap()
            .clone()
    };
    assert_eq!(
        file("a.c")["compiled"],
        serde_json::json!({"reason": "syntax", "at": "inc/pair.h:2"})
    );
    assert_eq!(
        file("b.c")["compiled"],
        serde_json::json!({"reason": "missing-header", "header": "proj/missing.h", "at": "b.c:1"})
    );
    // An absolute name can be a machine path: never stored.
    assert_eq!(
        file("c.c")["compiled"],
        serde_json::json!({"reason": "missing-header", "at": "c.c:1"})
    );
    let text = String::from_utf8(map_bytes(&tmp.0)).unwrap();
    assert!(!text.contains("nonexistent-ruharness"));
    assert!(!text.contains("PAIR_WIDE is needed"));
}

/// A `config.toml` the download shipped is proposed, not the person's: the
/// screen says it came with the project and the source stays a guess,
/// run after run, until `--adopt` states it.
#[test]
fn a_shipped_config_toml_is_proposed_until_the_person_states_it() {
    let tmp = Tmp::new("shipped-config");
    tmp.write("a.c", "int a(void) { return 1; }\n");
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\n\
         flags = [\"-DSHIPPED_BY_THE_DOWNLOAD\"]\n",
    );
    for _ in 0..2 {
        let run = harness(&["project", "map", "--target", tmp.arg()]);
        assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
        assert!(
            run.stdout.contains(
                "configuration: make, from make, flags -DSHIPPED_BY_THE_DOWNLOAD; it came with \
                 the project, so it is proposed, not yours yet: run once with --adopt to state it"
            ),
            "{}",
            run.stdout
        );
        let map = map_json(&tmp.0);
        assert_eq!(map["configuration"]["source"], "guessed");
        assert_eq!(map["configuration"]["proposed"], true);
    }
    let run = harness(&["project", "map", "--target", tmp.arg(), "--adopt"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("configuration: make, from make (stated in config.toml)"),
        "{}",
        run.stdout
    );
    let map = map_json(&tmp.0);
    assert_eq!(map["configuration"]["source"], "stated");
    assert!(map["configuration"].get("proposed").is_none());

    // The person's own edit states it too.
    let other = Tmp::new("shipped-config-edit");
    other.write("a.c", "int a(void) { return 1; }\n");
    other.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []\n",
    );
    let run = harness(&["project", "map", "--target", other.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(map_json(&other.0)["configuration"]["source"], "guessed");
    other.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\"-DMINE\"]\n",
    );
    let run = harness(&["project", "map", "--target", other.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert_eq!(map_json(&other.0)["configuration"]["source"], "stated");
}

/// A `.c` a header pulls in as text is shown as such, warned about before
/// it is offered as a library, and never a duplicate between programs.
#[test]
fn a_c_file_a_header_includes_is_warned_about_on_the_screen() {
    let tmp = Tmp::new("text-include");
    tmp.write("impl.c", "int impl(void) { return 1; }\n");
    tmp.write("all.h", "#include \"impl.c\"\n");
    tmp.write(
        "main.c",
        "#include \"all.h\"\nint main(void) { return impl(); }\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(
            "warning: impl.c is included as text by all.h: moving it to Rust leaves them \
             compiling its C text, so it is no library of its own"
        ),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout
            .contains("defined in two programs' files that never meet"),
        "{}",
        run.stdout
    );
}

/// The program count names the fuzzers and drivers among the programs.
#[test]
fn the_program_count_names_fuzzers_and_drivers() {
    let tmp = Tmp::new("fuzz-count");
    let fuzzer = "#include <stddef.h>\n#include <stdint.h>\n\
                  int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n) { (void)d; return (int)n; }\n";
    tmp.write("fuzz/f1.c", fuzzer);
    tmp.write("fuzz/f2.c", fuzzer);
    tmp.write(
        "fuzz/driver.c",
        "#include <stddef.h>\n#include <stdint.h>\n\
         int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n);\n\
         int main(void) { return LLVMFuzzerTestOneInput(0, 0); }\n",
    );
    tmp.write("tool.c", "int main(void) { return 0; }\n");
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("programs: 4 (2 fuzzers, 1 driver)"),
        "{}",
        run.stdout
    );
}

/// The compiler's and runtime's own names (`__stderrp` on Apple) are
/// folded into a count after the project's outside symbols.
#[cfg(target_vendor = "apple")]
#[test]
fn runtime_names_are_folded_into_a_count() {
    let tmp = Tmp::new("runtime-names");
    tmp.write(
        "main.c",
        "#include <stdio.h>\nint main(void) { fprintf(stderr, \"x\\n\"); return 0; }\n",
    );
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            // At -O2 the compiler turns this fprintf into fwrite.
            .contains("outside symbols: fwrite, and 1 compiler or runtime name;"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("__stderrp"), "{}", run.stdout);
}
