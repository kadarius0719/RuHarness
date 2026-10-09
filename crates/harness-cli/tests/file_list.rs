//! A file-list target (`harness.toml` v2, docs/PROJECT-MAP-DESIGN.md §3.7)
//! through the binary, on hand-written mapped tools: a liblzg-shaped tool
//! runs `scan`, `plan`, `detect`, `features init`, `state status` and
//! `sync-runtime`, every write under the tool's ledger; a tiny tool with a
//! hand-written unit crate verifies green with the configuration's `-D`
//! reaching the build, and maps its features.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "harness-cli-file-list-{tag}-{}-{}",
        std::process::id(),
        harness_core::hash::random_hex(4)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness(args: &[&str]) -> Run {
    let file = harness_core::adopt::testing::adoption_file();
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .env(harness_core::adopt::ADOPTED_ENV, file)
        .output()
        .expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every file under `dir`, relative to it, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The liblzg shape (harness-scan's fixture): a library folder whose header
/// includes `"../include/lzg.h"`, a tool including `<lzg.h>` through
/// `src/include`, a header nothing includes, and a C file the tool does not
/// list.
const LZG_FILES: &[(&str, &str)] = &[
    (
        "src/lib/checksum.c",
        "#include \"internal.h\"\n\
         unsigned lzg_checksum(const unsigned char *p, unsigned n) {\n\
         \x20   unsigned s = 0;\n\
         \x20   while (n--) s += *p++;\n\
         \x20   return s;\n}\n",
    ),
    (
        "src/lib/encode.c",
        "#include \"internal.h\"\nint lzg_level = 3;\n\
         static int clamp(int x) { return lzg_min(x, 9); }\n\
         unsigned lzg_encode(const unsigned char *p, unsigned n) {\n\
         \x20   return lzg_checksum(p, n) + (unsigned)clamp(lzg_level);\n}\n",
    ),
    (
        "src/lib/version.c",
        "#include \"lzg.h\"\nint lzg_version(void) { return LZG_VERSION; }\n",
    ),
    (
        "src/lib/internal.h",
        "#include \"../include/lzg.h\"\n\
         static inline int lzg_min(int a, int b) { return a < b ? a : b; }\n",
    ),
    (
        "src/include/lzg.h",
        "#define LZG_VERSION 0x010304\n\
         unsigned lzg_checksum(const unsigned char *p, unsigned n);\n\
         unsigned lzg_encode(const unsigned char *p, unsigned n);\n\
         int lzg_version(void);\n",
    ),
    (
        "src/tools/lzg.c",
        "#include <stdio.h>\n#include <lzg.h>\n\
         int main(void) {\n\
         \x20   printf(\"%d\\n\", lzg_version());\n\
         \x20   return (int)lzg_encode(0, 0);\n}\n",
    ),
    (
        "src/include/unused.h",
        "static inline int never_read(void) { return 0; }\n",
    ),
    ("src/other/decode.c", "int lzg_decode(void) { return 1; }\n"),
];

const LZG_TOML: &str = "schema_version = 2\n[target]\nname = \"lzg\"\nfiles = [\n\
    { path = \"src/lib/checksum.c\", include_dirs = [\"src/include\"] },\n\
    { path = \"src/lib/encode.c\", include_dirs = [\"src/include\"] },\n\
    { path = \"src/lib/version.c\", include_dirs = [\"src/include\"] },\n\
    { path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] },\n]\n\
    configuration = { name = \"make\", from = \"stated\", flags = [] }\n";

fn lzg(tag: &str) -> PathBuf {
    let root = tmp(tag);
    for (rel, text) in LZG_FILES {
        put(&root, rel, text);
    }
    put(&root, "migration/tools/t-lzg/harness.toml", LZG_TOML);
    root
}

/// The unit holding `file` in a plan's text, and its `source_hash`.
fn source_hash(plan: &str, file: &str) -> String {
    let block = plan
        .split("[[unit]]")
        .find(|b| b.contains(&format!("files = [\"{file}\"]")))
        .unwrap_or_else(|| panic!("no unit holds {file}: {plan}"));
    block
        .lines()
        .find_map(|l| l.strip_prefix("source_hash = "))
        .unwrap()
        .to_string()
}

#[test]
fn a_file_list_tool_runs_every_reading_command_inside_its_ledger() {
    let root = lzg("lzg");
    let target = root.to_str().unwrap();
    let tool = root.join("migration/tools/t-lzg");
    let run = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend(["--target", target, "--tool", "t-lzg"]);
        let r = harness(&argv);
        assert_eq!(r.code, 0, "{args:?}: {}\n{}", r.stdout, r.stderr);
        // A hand-written tool (no `map`, no `picks`) gets no "what
        // changed" notice anywhere.
        for words in ["changed since this tool", "project changed"] {
            assert!(
                !r.stdout.contains(words) && !r.stderr.contains(words),
                "{args:?}: {}\n{}",
                r.stdout,
                r.stderr
            );
        }
        r
    };
    let r = harness(&["scan", "--target", target, "--tool", "t-lzg"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    run(&["plan"]);
    run(&["detect"]);
    run(&["features", "init"]);
    let status = run(&["state", "status"]);
    assert!(status.stdout.contains("u-encode"), "{}", status.stdout);
    run(&["sync-runtime"]);

    // The facts hold the listed files and the headers they reach, through
    // both include forms — not the unlisted ones.
    let facts = std::fs::read_to_string(tool.join("facts.jsonl")).unwrap();
    for f in [
        "src/lib/encode.c",
        "src/tools/lzg.c",
        "src/lib/internal.h",
        "src/include/lzg.h",
    ] {
        assert!(facts.contains(&format!("\"{f}\"")), "{f}: {facts}");
    }
    assert!(!facts.contains("unused.h") && !facts.contains("decode.c"));
    // One unit per listed file with a public symbol, the tool's main among
    // them.
    let plan = std::fs::read_to_string(tool.join("plan.toml")).unwrap();
    for id in ["u-checksum", "u-encode", "u-version", "u-lzg"] {
        assert!(plan.contains(&format!("id = \"{id}\"")), "{id}: {plan}");
    }
    assert!(!plan.contains("decode"), "{plan}");

    // Every write lies under the tool's ledger: the root's migration/ holds
    // the adoption token, its ignore rules and tools/ only, and the shared AGENTS.md names
    // the tool's ledger.
    let mut top: Vec<String> = std::fs::read_dir(root.join("migration"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    // (`map/` holds only the project lock `sync-runtime` takes.)
    assert_eq!(
        top,
        [".gitignore", ".ruharness-adopted", "map", "tools"],
        "{top:?}"
    );
    let map: Vec<String> = std::fs::read_dir(root.join("migration/map"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(map, [".lock"], "{map:?}");
    let written = files_under(&tool);
    for f in [
        "facts.jsonl",
        "plan.toml",
        "features/features.toml",
        "harness.toml",
    ] {
        assert!(written.iter().any(|w| w == f), "{f}: {written:?}");
    }
    assert!(
        written.iter().any(|w| w.starts_with("observer/")),
        "{written:?}"
    );
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("`migration/tools/t-lzg/plan.toml`"),
        "{agents}"
    );
    let r = run(&["sync-runtime", "--check"]);
    assert_eq!(r.code, 0);

    // The plan's source_hash covers the include closure the facts record:
    // the tool's unit moves with the header it reaches by `<lzg.h>`, and
    // with nothing it does not reach.
    let before = source_hash(&plan, "src/tools/lzg.c");
    put(
        &root,
        "src/include/unused.h",
        "static inline int never_read(void) { return 1; }\n",
    );
    put(
        &root,
        "src/other/decode.c",
        "int lzg_decode(void) { return 2; }\n",
    );
    run(&["scan"]);
    run(&["plan"]);
    let plan = std::fs::read_to_string(tool.join("plan.toml")).unwrap();
    assert_eq!(source_hash(&plan, "src/tools/lzg.c"), before);
    let text = std::fs::read_to_string(root.join("src/include/lzg.h")).unwrap();
    put(&root, "src/include/lzg.h", &format!("{text}/* edited */\n"));
    // Stale facts are refused by plan until a scan: the header is the
    // tool's.
    let r = harness(&["plan", "--target", target, "--tool", "t-lzg"]);
    assert_eq!(r.code, 1, "{}\n{}", r.stdout, r.stderr);
    run(&["scan"]);
    run(&["plan"]);
    let plan = std::fs::read_to_string(tool.join("plan.toml")).unwrap();
    assert_ne!(source_hash(&plan, "src/tools/lzg.c"), before);
    let _ = std::fs::remove_dir_all(&root);
}

/// The unit's header builds only under the configuration, whose
/// `-DPAIR_WIDE=4` also widens the struct (harness-oracle's `t-pair` test).
const PAIR_H: &str = "#ifndef PAIR_H\n#define PAIR_H\n\
#ifndef PAIR_WIDE\n#error \"built without the configuration's -DPAIR_WIDE\"\n#endif\n\
typedef struct { long long pad[PAIR_WIDE]; int a; int b; } pair_t;\n\
int pair_sum(const pair_t *p);\n#endif\n";

const PAIR_C: &str = "#include \"pair.h\"\n\
int pair_sum(const pair_t *p) { return p->a + p->b + (int)sizeof(pair_t); }\n";

const MAIN_C: &str = "#include <stdio.h>\n#include \"pair.h\"\n\
int main(void) {\n    pair_t p = {{0}, 2, 3};\n    printf(\"%d\\n\", pair_sum(&p));\n    return 0;\n}\n";

const DRIVER_C: &str = "#include <stdio.h>\n#include \"pair.h\"\n\
int main(void) {\n    pair_t p = {{0}, 0, 0};\n    int i;\n\
    for (i = 0; i < 50; i++) {\n        p.a = i * 7;\n        p.b = 3 - i;\n\
        printf(\"%d %d\\n\", pair_sum(&p), (int)sizeof(pair_t));\n    }\n    return 0;\n}\n";

/// The Rust of the widened layout: right only when every C side was built
/// with the configuration's `-D`.
const PAIR_RS: &str =
    "#[repr(C)]\npub struct Pair {\n    pad: [i64; 4],\n    a: i32,\n    b: i32,\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn pair_sum(p: *const Pair) -> i32 {\n    \
(*p).a.wrapping_add((*p).b).wrapping_add(40)\n}\n";

const FEATURES: &str = "schema_version = 1\n\n[[feature]]\nid = \"sum\"\nname = \"Add a pair\"\n\n\
[[scenario]]\nfeature = \"sum\"\nid = \"plain\"\n";

const PAIR_LEDGER: &str = "migration/tools/t-pair";

/// The project `app/main.c` + `lib/pair.c` (each finding `pair.h` in `inc/`)
/// as the tool `t-pair` built under `flags`, with `u-pair`'s driver, its
/// hand-written crate and a features file.
fn pair(tag: &str, flags: &str) -> PathBuf {
    let root = tmp(tag);
    put(
        &root,
        &format!("{PAIR_LEDGER}/harness.toml"),
        &format!(
            "schema_version = 2\n\n[target]\nname = \"pair\"\n\
             files = [{{ path = \"app/main.c\", include_dirs = [\"inc\"] }}, \
             {{ path = \"lib/pair.c\", include_dirs = [\"inc\"] }}]\n\
             configuration = {{ name = \"make\", from = \"stated\", flags = [{flags}] }}\n\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n"
        ),
    );
    put(&root, "inc/pair.h", PAIR_H);
    put(&root, "lib/pair.c", PAIR_C);
    put(&root, "app/main.c", MAIN_C);
    let unit_dir = format!("{PAIR_LEDGER}/units/u-pair");
    put(&root, &format!("{unit_dir}/driver.c"), DRIVER_C);
    put(
        &root,
        &format!("{unit_dir}/pair_rs/Cargo.toml"),
        "[package]\nname = \"pair_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\n",
    );
    put(&root, &format!("{unit_dir}/pair_rs/src/lib.rs"), PAIR_RS);
    put(
        &root,
        &format!("{PAIR_LEDGER}/features/features.toml"),
        FEATURES,
    );
    root
}

/// Give `u-pair` its oracle table in the planned `plan.toml`.
fn wire_unit(root: &Path) {
    let path = root.join(PAIR_LEDGER).join("plan.toml");
    let plan = std::fs::read_to_string(&path).unwrap();
    let mut blocks: Vec<String> = plan.split("[[unit]]").map(str::to_string).collect();
    let at = blocks
        .iter()
        .position(|b| b.contains("id = \"u-pair\""))
        .unwrap_or_else(|| panic!("no u-pair: {plan}"));
    blocks[at] = format!(
        "{}\n\n[unit.oracle]\nkind = \"c-abi-differential\"\nboundary = true\n\
         driver = \"{PAIR_LEDGER}/units/u-pair/driver.c\"\nrust_crate = \"pair_rs\"\n\
         replaces = [\"lib/pair.c\"]\n\n",
        blocks[at].trim_end()
    );
    std::fs::write(&path, blocks.join("[[unit]]")).unwrap();
}

/// `verify` and `features map` on a tool through the CLI: the
/// configuration's `-DPAIR_WIDE=4` reaches the builds (the header refuses
/// to compile without it, and the Rust is right only for the widened
/// layout), the verdict lands under the tool's ledger, green.
#[test]
fn a_file_list_tool_verifies_green_through_the_cli() {
    let root = pair("verify", "\"-DPAIR_WIDE=4\"");
    let target = root.to_str().unwrap();
    let ledger = root.join(PAIR_LEDGER);
    let run = |args: &[&str], code: i32| {
        let mut argv = args.to_vec();
        argv.extend(["--target", target, "--tool", "t-pair"]);
        let r = harness(&argv);
        assert_eq!(r.code, code, "{args:?}: {}\n{}", r.stdout, r.stderr);
        r
    };
    // The test wrote a unit's driver, crate and features into the tool's
    // ledger — results, not just a hand-written harness.toml — so the first
    // command adopts them.
    let r = harness(&["--adopt", "scan", "--target", target, "--tool", "t-pair"]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    run(&["plan"], 0);
    wire_unit(&root);
    let r = run(&["features", "map"], 0);
    assert!(r.stdout.contains("sum"), "{}", r.stdout);
    assert!(ledger.join("features/map.json").is_file());
    let r = run(&["verify", "u-pair", "--allow-unsandboxed"], 0);
    assert!(r.stdout.contains("GREEN"), "{}\n{}", r.stdout, r.stderr);
    let verdict = std::fs::read_to_string(ledger.join("units/u-pair/oracle-latest.json")).unwrap();
    assert!(verdict.contains("\"green\": true"), "{verdict}");
    for check in ["differential-driver", "boundary", "feature:sum/plain"] {
        assert!(verdict.contains(check), "{check}: {verdict}");
    }
    // No [oracle.whole_program]: the screen says the check did not run and
    // how to turn it on; the recorded verdict keeps its passed check.
    assert!(
        r.stdout.contains(
            "verify: [SKIP] whole-program — not run: not configured for this target (add \
             [oracle.whole_program] args = [...] to harness.toml)"
        ),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("[PASS] whole-program"), "{}", r.stdout);
    assert!(
        verdict.contains(
            "\"name\": \"whole-program\",\n      \"passed\": true,\n      \"detail\": \"not \
             configured for this target\""
        ),
        "{verdict}"
    );
    assert!(!root.join("migration/units").exists());
    let plan = std::fs::read_to_string(ledger.join("plan.toml")).unwrap();
    assert!(plan.contains("status = \"verified\""), "{plan}");
    let _ = std::fs::remove_dir_all(&root);
}
