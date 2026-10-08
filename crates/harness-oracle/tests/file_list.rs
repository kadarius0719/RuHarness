//! A file-list target (`harness.toml` v2, docs/PROJECT-MAP-DESIGN.md §3.7)
//! through the public API, on a hand-written mapped tool: the
//! configuration's flags and each file's include folders reach every
//! compile — the driver builds, the boundary check's wrapper, the features
//! map's probed copy — and the mirror copies only the listed files and the
//! headers they reach, never anything under `migration/`.
//!
//! The facts are written by hand: the scanner's file-list read is another
//! part of step (c).

mod common;

use common::{write, TempDir};
use harness_core::facts::{FileRecord, SymbolRecord};
use harness_core::features::FeatureSnapshot;
use harness_core::traits::OracleStrategy;
use harness_core::{Facts, TargetContext, Unit, Verdict};
use harness_oracle::CAbiDifferential;
use std::path::Path;

const TOOL: &str = "t-pair";

/// The unit's header: it builds only under the configuration, whose
/// `-DPAIR_WIDE=4` also widens the struct (a 40-byte layout instead of 8).
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

/// Lay out the project and its mapped tool `t-pair`: `app/main.c` and
/// `lib/pair.c`, each with `inc/` as its include folder, built under
/// `flags`; the unit `u-pair` (boundary opted in), its driver, its crate,
/// the facts and a features file.
fn tool(root: &Path, flags: &str) -> (TargetContext, Unit) {
    let ledger = format!("migration/tools/{TOOL}");
    write(
        &root.join(&ledger).join("harness.toml"),
        &format!(
            "schema_version = 2\n\n[target]\nname = \"pair\"\n\
             files = [{{ path = \"app/main.c\", include_dirs = [\"inc\"] }}, \
             {{ path = \"lib/pair.c\", include_dirs = [\"inc\"] }}]\n\
             configuration = {{ name = \"make\", from = \"stated\", flags = [{flags}] }}\n\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n"
        ),
    );
    write(&root.join("inc/pair.h"), PAIR_H);
    write(&root.join("lib/pair.c"), PAIR_C);
    write(&root.join("app/main.c"), MAIN_C);
    let unit_dir = format!("{ledger}/units/u-pair");
    write(&root.join(&unit_dir).join("driver.c"), DRIVER_C);
    let crate_dir = root.join(&unit_dir).join("pair_rs");
    write(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"pair_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\n",
    );
    write(&crate_dir.join("src/lib.rs"), PAIR_RS);
    write(&root.join(&ledger).join("features/features.toml"), FEATURES);
    facts()
        .store(&root.join(&ledger).join("facts.jsonl"))
        .expect("facts stored");
    let unit: Unit = toml::from_str(&format!(
        "id = \"u-pair\"\nstatus = \"pending\"\nfiles = [\"lib/pair.c\"]\n\
         symbols = [\"pair_sum\"]\ninterface = [\"int pair_sum(const pair_t *p)\"]\n\n\
         [oracle]\nkind = \"c-abi-differential\"\nboundary = true\n\
         driver = \"{unit_dir}/driver.c\"\nrust_crate = \"pair_rs\"\n\
         replaces = [\"lib/pair.c\"]\n"
    ))
    .expect("unit parses");
    harness_core::adopt::testing::adopt(root);
    (
        TargetContext::open(root, Some(TOOL)).expect("tool loads"),
        unit,
    )
}

fn facts() -> Facts {
    let file = |path: &str, includes: &[&str]| FileRecord {
        path: path.into(),
        hash: String::new(),
        includes: includes.iter().map(|s| (*s).to_string()).collect(),
    };
    let sym = |file: &str, name: &str| SymbolRecord {
        name: name.into(),
        kind: "function".into(),
        file: file.into(),
        visibility: "public".into(),
        signature: String::new(),
        span: (1, 1),
    };
    Facts {
        frontend: "test-inline".into(),
        files: vec![
            file("app/main.c", &["inc/pair.h"]),
            file("inc/pair.h", &[]),
            file("lib/pair.c", &["inc/pair.h"]),
        ],
        symbols: vec![sym("app/main.c", "main"), sym("lib/pair.c", "pair_sum")],
        ..Facts::default()
    }
}

fn describe(verdict: &Verdict) -> String {
    verdict
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

struct Quiet;

impl harness_oracle::MapProgress for Quiet {
    fn message(&mut self, _: &str) {}
    fn scenario(&mut self, _: &harness_core::features::ScenarioRecord, _: usize, _: usize) {}
}

/// The configuration's `-D` (which the header requires, and which widens
/// the struct) is seen by the driver's builds — the C-linked driver prints
/// what the Rust of the widened layout prints — by the driver's own
/// compile (the shape gate), by the boundary check's wrapper, probes and
/// unit objects, and by the whole program the features step builds. Its
/// `-std=c89` reaches them too, and every check stays green: the
/// harness's own C under the configuration (the call wrapper) is C89, and
/// the guard and tracing-probe runtimes are built without the
/// configuration. Both listed files find `pair.h` through the one folder
/// they share, `inc/`; that each file is compiled with its own folders is
/// proven by the library test `a_units_files_compile_each_with_its_own_folders`.
#[test]
fn the_configuration_reaches_the_driver_build_and_the_boundary_check() {
    let tmp = TempDir::new("fl-verify");
    let (target, unit) = tool(tmp.path(), "\"-DPAIR_WIDE=4\", \"-O3\", \"-std=c89\"");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    // The verdict records the configuration it was built under.
    let entry = harness_core::status::configuration_entry(&target).expect("a file list");
    assert!(
        verdict.inputs.toolchain.contains(&entry),
        "{:?}",
        verdict.inputs.toolchain
    );
    let names: Vec<&str> = verdict.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "symbol-set",
            "capabilities",
            "driver-shape",
            "differential-driver",
            "whole-program",
            "sanitizers",
            "boundary",
            "feature:sum/plain",
        ],
        "{}",
        describe(&verdict)
    );
    // The driver's output is the widened layout's.
    let out = std::fs::read_to_string(
        tmp.path()
            .join(format!("migration/tools/{TOOL}/build/u-pair/drv_c.out")),
    )
    .expect("driver output kept");
    assert!(out.starts_with("43 40\n49 40\n"), "{out}");
}

/// The same tool with the configuration's flag removed: the header's
/// `#error` stops the driver's build — the flag is what made it build.
#[test]
fn without_the_flag_nothing_builds() {
    let tmp = TempDir::new("fl-noflag");
    let (target, unit) = tool(tmp.path(), "");
    let verdict = CAbiDifferential.verify(&target, &unit);
    let failed = match verdict {
        Err(e) => e.to_string(),
        Ok(v) => describe(&v),
    };
    assert!(
        failed.contains("built without the configuration"),
        "{failed}"
    );
}

/// The features map: the plain program and the probed copy (the
/// configuration's paths and every folder moved into the mirror) both build
/// under the configuration's `-D`, and the scenario ran both functions.
#[test]
fn the_configuration_reaches_the_features_maps_probed_build() {
    let tmp = TempDir::new("fl-map");
    let (target, _) = tool(tmp.path(), "\"-DPAIR_WIDE=4\", \"-Iinc\"");
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid features")
    };
    let map = harness_oracle::map_features(&target, &facts(), &features, &digest, &mut Quiet)
        .expect("maps");
    let sum = &map.scenarios[0];
    let ran: Vec<&str> = sum.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(ran, ["main", "pair_sum"], "{sum:?}");
    assert!(sum.stable && sum.probe_agrees, "{sum:?}");
    assert!(map.unwatched.is_empty(), "{:?}", map.unwatched);
    // The mirror holds the listed files and the header they reach — and
    // nothing of the ledger.
    let mirror = tmp
        .path()
        .join(format!("migration/tools/{TOOL}/build/.features/mirror"));
    assert!(mirror.join("inc/pair.h").is_file());
    assert!(mirror.join("lib/pair.c").is_file());
    assert!(!mirror.join("migration").exists());
}

/// The mirror reads each file as a regular file with a size cap: a
/// configuration `-include` that names a FIFO is refused by name, never
/// opened for a read that would wait forever for a writer.
#[test]
fn a_forced_include_that_is_a_fifo_is_refused_never_read() {
    let tmp = TempDir::new("fl-fifo");
    let (target, _) = tool(tmp.path(), "\"-DPAIR_WIDE=4\", \"-includeinc/fifo.h\"");
    let made = std::process::Command::new("mkfifo")
        .arg(tmp.path().join("inc/fifo.h"))
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid features")
    };
    let why = harness_oracle::map_features(&target, &facts(), &features, &digest, &mut Quiet)
        .expect_err("refused")
        .to_string();
    assert!(
        why.contains("fifo.h") && why.contains("not a regular file"),
        "{why}"
    );
}

/// Confinement, for the mirror: a listed file under `migration/` (reached
/// through a link the lexical check cannot see), an include folder that
/// leaves the project, and a header the facts say a listed file reaches
/// under `migration/` are each refused by name, before anything is built.
#[test]
fn the_mirror_refuses_the_ledger_and_folders_outside_the_root() {
    let tmp = TempDir::new("fl-confine");
    let root = tmp.path().join("project");
    let (target, _) = tool(&root, "\"-DPAIR_WIDE=4\"");
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid features")
    };
    let map = |target: &TargetContext, facts: &Facts| {
        harness_oracle::map_features(target, facts, &features, &digest, &mut Quiet)
            .expect_err("refused")
            .to_string()
    };
    let with = |files: &str| {
        let mut t = target.clone();
        let table: toml::Table = toml::from_str(&format!(
            "schema_version = 2\n[target]\nname = \"pair\"\nfiles = [{files}]\n\
             configuration = {{ name = \"make\", from = \"stated\", flags = [\"-DPAIR_WIDE=4\"] }}\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n"
        ))
        .expect("toml");
        t.config = harness_core::config::TargetConfig::from_table(table).expect("config");
        t
    };

    // A listed file that is a link into the ledger.
    write(
        &root.join(format!("migration/tools/{TOOL}/units/u-pair/evil.c")),
        "int evil(void) { return 1; }\n",
    );
    std::os::unix::fs::symlink(
        root.join(format!("migration/tools/{TOOL}/units/u-pair/evil.c")),
        root.join("lib/evil.c"),
    )
    .expect("link");
    let why = map(
        &with(
            "{ path = \"app/main.c\", include_dirs = [\"inc\"] }, \
             { path = \"lib/evil.c\", include_dirs = [\"inc\"] }",
        ),
        &facts(),
    );
    assert!(why.contains("migration/"), "{why}");

    // An include folder linked out of the project.
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    write(&outside.join("pair.h"), PAIR_H);
    std::os::unix::fs::symlink(&outside, root.join("ext")).expect("link");
    let why = map(
        &with(
            "{ path = \"app/main.c\", include_dirs = [\"ext\"] }, \
             { path = \"lib/pair.c\", include_dirs = [\"inc\"] }",
        ),
        &facts(),
    );
    assert!(why.contains("outside"), "{why}");

    // A header the facts place under the ledger.
    write(
        &root.join(format!("migration/tools/{TOOL}/stolen.h")),
        "#define STOLEN 1\n",
    );
    let mut reaching = facts();
    reaching.files[0]
        .includes
        .push(format!("migration/tools/{TOOL}/stolen.h"));
    let why = map(&target, &reaching);
    assert!(why.contains("migration/"), "{why}");
    assert!(
        !root
            .join(format!(
                "migration/tools/{TOOL}/build/.features/mirror/migration"
            ))
            .exists(),
        "nothing of the ledger copied"
    );
}
