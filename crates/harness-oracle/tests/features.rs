//! End-to-end scenario checks (docs/FEATURES-DESIGN.md §4, §6) on a small
//! program with several behaviours: the person's features run on the all-C
//! program and on the mixed one; what the C side cannot run the same way
//! twice is a skip; what the Rust does is always a check.

mod common;

use common::{write, TempDir};
use harness_core::facts::FileRecord;
use harness_core::features::{FeatureSnapshot, INVALID_DIGEST};
use harness_core::traits::OracleStrategy;
use harness_core::{Facts, TargetContext, Unit, Verdict};
use harness_oracle::CAbiDifferential;
use std::path::Path;

const GOOD: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n";

const OFF_BY_ONE_ON_Z: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
if b == 122 { a.wrapping_add(b).wrapping_add(1) } else { a.wrapping_add(b) }\n}\n";

const PANICS_ON_Z: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
if b == 122 { panic!(\"z\") }\n    a.wrapping_add(b)\n}\n";

const MAIN: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include "unit.h"
#include "mul.h"
int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: %s -a|-m|-w|-t|-s FILE\n", argv[0]); return 1; }
  if (!strcmp(argv[1], "-a") && argc > 2) {
    FILE *f = fopen(argv[2], "rb"); int acc = 0, c;
    if (!f) { fprintf(stderr, "cannot open %s\n", argv[2]); return 2; }
    while ((c = fgetc(f)) != EOF) acc = unit_add(acc, c);
    fclose(f); printf("%d\n", acc); return 0;
  }
  if (!strcmp(argv[1], "-m") && argc > 2) {
    FILE *f = fopen(argv[2], "rb"); unsigned acc = 0; int c;
    if (!f) return 2;
    while ((c = fgetc(f)) != EOF) acc = mul_step(acc, c);
    fclose(f); printf("%u\n", acc); return 0;
  }
  if (!strcmp(argv[1], "-w") && argc > 2) {
    FILE *f = fopen(argv[2], "wb"); if (!f) return 3; fputs("x", f); fclose(f); return 0;
  }
  if (!strcmp(argv[1], "-t")) {
    struct timespec ts; clock_gettime(CLOCK_REALTIME, &ts); printf("%ld\n", (long)ts.tv_nsec); return 0;
  }
  if (!strcmp(argv[1], "-s")) { sleep(60); return 0; }
  fprintf(stderr, "unknown option %s\n", argv[1]);
  return 1;
}
"#;

const FEATURES: &str = r#"schema_version = 1

[[feature]]
id = "sum"
name = "Add up a file"

[[feature]]
id = "product"
name = "Hash a file"

[[feature]]
id = "usage"
name = "Say how to use it"

[[feature]]
id = "write"
name = "Write a file"

[[feature]]
id = "clock"
name = "Print the time"

[[scenario]]
feature = "sum"
id = "text"
args = ["-a", "{input}"]
input = "sample:text"

[[scenario]]
feature = "sum"
id = "missing"
args = ["-a", "nosuchfile"]

[[scenario]]
feature = "product"
id = "rand"
args = ["-m", "{input}"]
input = "sample:rand"

[[scenario]]
feature = "usage"
id = "none"

[[scenario]]
feature = "write"
id = "file"
args = ["-w", "out.txt"]

[[scenario]]
feature = "clock"
id = "now"
args = ["-t"]
"#;

/// A small program — `main.c` (options), `unit.c` (the unit: `unit_add`),
/// `mul.c` (another unit's code) — with its driver, facts, the unit crate
/// holding `lib_rs`, and `features` as `migration/features/features.toml`.
/// `extra` lands in `[oracle]`.
fn program(
    root: &Path,
    lib_rs: &str,
    features: Option<&str>,
    extra: &str,
) -> (TargetContext, Unit) {
    write(
        &root.join("harness.toml"),
        &format!(
            "schema_version = 1\n\n[target]\nname = \"tool\"\nsource_dir = \"src/tool\"\n\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n{extra}\n"
        ),
    );
    write(
        &root.join("src/tool/unit.h"),
        "int unit_add(int a, int b);\n",
    );
    write(
        &root.join("src/tool/unit.c"),
        "#include \"unit.h\"\nint unit_add(int a, int b) { return (int)((unsigned)a + (unsigned)b); }\n",
    );
    write(
        &root.join("src/tool/mul.h"),
        "unsigned mul_step(unsigned acc, int c);\n",
    );
    write(
        &root.join("src/tool/mul.c"),
        "#include \"mul.h\"\nunsigned mul_step(unsigned acc, int c) { return acc * 31u + (unsigned)c; }\n",
    );
    write(&root.join("src/tool/main.c"), MAIN);
    write(
        &root.join("migration/units/u-unit/driver.c"),
        "#include <stdio.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           for (int i = 0; i < 300; i++) printf(\"%d\\n\", unit_add(i * 7919, i - 150));\n\
           return 0;\n\
         }\n",
    );
    let crate_dir = root.join("migration/units/u-unit/unit_rs");
    write(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"unit_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\n",
    );
    write(&crate_dir.join("src/lib.rs"), lib_rs);
    if let Some(text) = features {
        write(&root.join("migration/features/features.toml"), text);
    }
    // The facts as a scan would write them: each file's hash, so the program
    // digest reads them as describing the tree.
    let rec = |path: &str, includes: &[&str]| FileRecord {
        path: path.into(),
        hash: harness_core::hash::file_hash(&root.join(path)).expect("hashed"),
        includes: includes.iter().map(|s| s.to_string()).collect(),
    };
    Facts {
        frontend: "test-inline".into(),
        files: vec![
            rec("src/tool/main.c", &["src/tool/mul.h", "src/tool/unit.h"]),
            rec("src/tool/mul.c", &["src/tool/mul.h"]),
            rec("src/tool/mul.h", &[]),
            rec("src/tool/unit.c", &["src/tool/unit.h"]),
            rec("src/tool/unit.h", &[]),
        ],
        ..Facts::default()
    }
    .store(&root.join("migration/facts.jsonl"))
    .expect("facts stored");
    let unit: Unit = toml::from_str(
        "id = \"u-unit\"\nstatus = \"pending\"\nfiles = [\"src/tool/unit.c\"]\n\
         symbols = [\"unit_add\"]\n\n[oracle]\nkind = \"c-abi-differential\"\n\
         driver = \"migration/units/u-unit/driver.c\"\nrust_crate = \"unit_rs\"\n\
         replaces = [\"src/tool/unit.c\"]\n",
    )
    .expect("unit parses");
    (TargetContext::load(root).expect("target loads"), unit)
}

fn check<'a>(verdict: &'a Verdict, name: &str) -> &'a harness_core::verdict::Check {
    verdict
        .checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}:\n{}", describe(verdict)))
}

fn describe(verdict: &Verdict) -> String {
    verdict
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .chain(
            verdict
                .inputs
                .features_skipped
                .iter()
                .map(|s| format!("skipped {s}")),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

fn feature_names(verdict: &Verdict) -> Vec<&str> {
    verdict
        .checks
        .iter()
        .filter(|c| c.name.starts_with("feature:"))
        .map(|c| c.name.as_str())
        .collect()
}

#[test]
fn every_scenario_becomes_a_check_or_a_recorded_skip() {
    let tmp = TempDir::new("feat-good");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    assert_eq!(
        feature_names(&verdict),
        [
            "feature:sum/text",
            "feature:sum/missing",
            "feature:product/rand",
            "feature:usage/none",
            "feature:write/file",
        ],
        "file order; the clock scenario is not a check"
    );
    assert_eq!(
        verdict.inputs.features_skipped,
        ["clock/now: c-side-unstable"],
        "{}",
        describe(&verdict)
    );
    // The usage line prints argv[0]: the same bytes on both sides.
    assert_eq!(
        check(&verdict, "feature:usage/none").detail,
        "exit 1; stdout empty; stderr 41 bytes identical" // "usage: $PROGDIR/tool -a|-m|-w|-t|-s FILE\n"
    );
    assert_eq!(
        check(&verdict, "feature:sum/missing").detail,
        "exit 2; stdout empty; stderr 23 bytes identical",
        "a file that does not exist is an easy scenario"
    );
    assert_eq!(
        check(&verdict, "feature:write/file").detail,
        "exit 0; stdout empty; stderr empty",
        "the file it writes lands in its own temp dir"
    );
    // Features after every other check, the digests recorded.
    let last_other = verdict
        .checks
        .iter()
        .rposition(|c| !c.name.starts_with("feature:"))
        .expect("other checks");
    assert!(verdict.checks[..=last_other]
        .iter()
        .all(|c| !c.name.starts_with("feature:")));
    let FeatureSnapshot::Valid { digest, .. } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    assert_eq!(verdict.inputs.features, digest);
    assert!(verdict.inputs.program.starts_with("blake3:"));
}

#[test]
fn what_the_rust_does_is_always_a_check_never_a_skip() {
    let tmp = TempDir::new("feat-bad");
    let (target, unit) = program(tmp.path(), OFF_BY_ONE_ON_Z, Some(FEATURES), "");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let sum = check(&verdict, "feature:sum/text");
    assert!(!sum.passed);
    assert!(
        sum.detail.starts_with("stdout differs (lens"),
        "{}",
        sum.detail
    );
    assert!(
        check(&verdict, "feature:product/rand").passed,
        "another unit's behaviour"
    );
    assert_eq!(
        verdict.inputs.features_skipped,
        ["clock/now: c-side-unstable"]
    );

    let tmp = TempDir::new("feat-panic");
    let (target, unit) = program(tmp.path(), PANICS_ON_Z, Some(FEATURES), "");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert_eq!(
        check(&verdict, "feature:sum/text").detail,
        "candidate run failed: signal 6",
        "{}",
        describe(&verdict)
    );
    assert_eq!(
        verdict.inputs.features_skipped,
        ["clock/now: c-side-unstable"]
    );
}

#[test]
fn a_c_side_that_times_out_is_a_skip_and_costs_one_run() {
    let tmp = TempDir::new("feat-slow");
    let features = "schema_version = 1\n[[feature]]\nid = \"slow\"\nname = \"Sleep\"\n\
                    [[scenario]]\nfeature = \"slow\"\nid = \"long\"\nargs = [\"-s\"]\n";
    let (target, unit) = program(tmp.path(), GOOD, Some(features), "timeout_secs = 20");
    let started = std::time::Instant::now();
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(
        verdict.green,
        "a skip is not a failure: {}",
        describe(&verdict)
    );
    assert_eq!(
        verdict.inputs.features_skipped,
        ["slow/long: c-side-timed-out"]
    );
    assert!(feature_names(&verdict).is_empty());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the mixed side and the second C run were not run"
    );
}

#[test]
fn an_invalid_file_runs_no_feature_and_says_so_in_the_verdict() {
    let tmp = TempDir::new("feat-invalid");
    let (target, unit) = program(tmp.path(), GOOD, Some("schema_version = 1\nnope = 1\n"), "");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    assert!(feature_names(&verdict).is_empty());
    assert_eq!(verdict.inputs.features, INVALID_DIGEST);
    assert!(verdict.inputs.program.starts_with("blake3:"));
    assert!(verdict.inputs.features_skipped.is_empty());
}

#[test]
fn the_link_decides_the_program_and_a_unit_outside_it_skips() {
    // Two main()s: the whole C program does not link — every scenario is a
    // C-side skip, never an error, never a failure.
    let tmp = TempDir::new("feat-twomain");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    write(
        &tmp.path().join("src/tool/second.c"),
        "int main(void) { return 0; }\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(feature_names(&verdict).is_empty());
    assert_eq!(verdict.inputs.features_skipped.len(), 6);
    assert!(verdict
        .inputs
        .features_skipped
        .iter()
        .all(|s| s.ends_with(": c-side-build-failed")));

    // A unit whose replaced file is not a top-level .c of the program.
    let tmp = TempDir::new("feat-outside");
    let (target, _) = program(tmp.path(), GOOD, Some(FEATURES), "");
    write(
        &tmp.path().join("src/tool/sub/unit.c"),
        "#include \"../unit.h\"\nint unit_add(int a, int b) { return (int)((unsigned)a + (unsigned)b); }\n",
    );
    let unit: Unit = toml::from_str(
        "id = \"u-unit\"\nstatus = \"pending\"\nfiles = [\"src/tool/sub/unit.c\"]\n\
         symbols = [\"unit_add\"]\n\n[oracle]\nkind = \"c-abi-differential\"\n\
         driver = \"migration/units/u-unit/driver.c\"\nrust_crate = \"unit_rs\"\n\
         replaces = [\"src/tool/sub/unit.c\"]\n",
    )
    .expect("unit parses");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(feature_names(&verdict).is_empty(), "{}", describe(&verdict));
    assert!(verdict
        .inputs
        .features_skipped
        .iter()
        .all(|s| s.ends_with(": not-in-program")));
}

/// Review O1: a top-level `.c` the build cannot take — an editor's dangling
/// lock link — is the tree's doing: every scenario a C-side skip, and the
/// verdict otherwise as without features.
#[test]
fn a_dangling_c_link_in_the_source_dir_is_a_skip_not_an_error() {
    let tmp = TempDir::new("feat-dangling");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    std::os::unix::fs::symlink(
        "beaumorton@host.1234:1",
        tmp.path().join("src/tool/.#main.c"),
    )
    .unwrap();
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    assert!(feature_names(&verdict).is_empty());
    assert_eq!(verdict.inputs.features_skipped.len(), 6);
    assert!(verdict
        .inputs
        .features_skipped
        .iter()
        .all(|s| s.ends_with(": c-side-build-failed")));
}

/// Review O2: a program file changed since the scan — its includes may be
/// ones the facts never saw — records the stale-facts digest, which no
/// status reads as current.
#[test]
fn a_program_changed_since_the_scan_is_never_current() {
    let tmp = TempDir::new("feat-stale-facts");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    let main = tmp.path().join("src/tool/main.c");
    let text = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, format!("{text}\n/* edited after the scan */\n")).unwrap();
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert_eq!(
        verdict.inputs.program,
        harness_core::features::STALE_PROGRAM
    );
    let facts = harness_core::Facts::load(&tmp.path().join("migration/facts.jsonl")).unwrap();
    let snapshot = FeatureSnapshot::load(&target);
    let now = harness_core::features::FeaturesNow::compute(&target, &facts, &snapshot).unwrap();
    assert_eq!(now.program, harness_core::features::STALE_PROGRAM);
    assert!(matches!(
        now.coverage(&verdict.inputs),
        harness_core::features::Coverage::Behind(r) if r.contains(&"program".to_string())
    ));
}

#[test]
fn a_mixed_program_that_does_not_link_fails_every_scenario() {
    // unit.c also defines a helper main.c needs; the unit's Rust exports only
    // unit_add (the symbol-set rule), so the mixed program cannot link.
    let tmp = TempDir::new("feat-nolink");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    write(
        &tmp.path().join("src/tool/unit.c"),
        "#include \"unit.h\"\nint unit_add(int a, int b) { return (int)((unsigned)a + (unsigned)b); }\n\
         int unit_helper(void) { return 7; }\n",
    );
    write(
        &tmp.path().join("src/tool/mul.c"),
        "#include \"mul.h\"\nint unit_helper(void);\n\
         unsigned mul_step(unsigned acc, int c) { return acc * 31u + (unsigned)c + (unsigned)unit_helper() * 0u; }\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let names = feature_names(&verdict);
    assert_eq!(names.len(), 6, "{}", describe(&verdict));
    for name in names {
        let c = check(&verdict, name);
        assert!(!c.passed);
        assert_eq!(c.detail, "the mixed program did not link");
    }
}

#[test]
fn without_a_features_file_nothing_changes() {
    let tmp = TempDir::new("feat-none");
    let (target, unit) = program(tmp.path(), GOOD, None, "");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green);
    assert!(feature_names(&verdict).is_empty());
    let json = serde_json::to_string(&verdict).expect("json");
    for key in ["\"features\"", "\"program\"", "\"features_skipped\""] {
        assert!(!json.contains(key), "{key} in {json}");
    }
}

#[test]
fn verify_with_uses_the_snapshot_it_is_given() {
    let tmp = TempDir::new("feat-with");
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "");
    let verdict = CAbiDifferential
        .verify_with(&target, &unit, &FeatureSnapshot::None)
        .expect("oracle runs");
    assert!(
        feature_names(&verdict).is_empty(),
        "a replay's None is honoured"
    );
    assert!(verdict.inputs.features.is_empty());
}

/// The map (docs/FEATURES-DESIGN.md §5): each scenario on a probed copy of
/// the C — which functions it ran, how it ended, whether it is stable.
struct Quiet(Vec<(String, usize, usize)>);

impl harness_oracle::MapProgress for Quiet {
    fn message(&mut self, _: &str) {}
    fn scenario(&mut self, r: &harness_core::features::ScenarioRecord, n: usize, of: usize) {
        self.0
            .push((format!("{}/{}", r.feature, r.scenario), n, of));
    }
}

fn with_symbols(root: &Path) -> Facts {
    // The facts of `program` plus its functions, as the scanner records them.
    let mut facts = Facts::load(&root.join("migration/facts.jsonl")).expect("facts");
    let sym = |file: &str, name: &str| harness_core::facts::SymbolRecord {
        name: name.into(),
        kind: "function".into(),
        file: file.into(),
        visibility: "public".into(),
        signature: String::new(),
        span: (1, 1),
    };
    facts.symbols = vec![
        sym("src/tool/main.c", "main"),
        sym("src/tool/mul.c", "mul_step"),
        sym("src/tool/unit.c", "unit_add"),
    ];
    facts
}

#[test]
fn the_map_says_which_functions_each_scenario_ran() {
    let tmp = TempDir::new("feat-map");
    let (target, _) = program(tmp.path(), GOOD, Some(FEATURES), "");
    let facts = with_symbols(tmp.path());
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let mut progress = Quiet(Vec::new());
    let map = harness_oracle::map_features(&target, &facts, &features, &digest, &mut progress)
        .expect("maps");
    assert_eq!(progress.0.len(), 6);
    assert_eq!(progress.0[0], ("sum/text".to_string(), 1, 6));
    let by = |id: &str| {
        map.scenarios
            .iter()
            .find(|r| format!("{}/{}", r.feature, r.scenario) == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    let names =
        |id: &str| -> Vec<String> { by(id).functions.iter().map(|(_, n)| n.clone()).collect() };
    assert_eq!(names("sum/text"), ["main", "unit_add"]);
    assert_eq!(names("product/rand"), ["main", "mul_step"]);
    assert_eq!(names("usage/none"), ["main"]);
    let sum = by("sum/text");
    assert_eq!(sum.end, "exit 0");
    assert!(sum.stable && sum.probe_agrees, "{sum:?}");
    assert_eq!(sum.noted, "complete");
    let usage = by("usage/none");
    assert_eq!(usage.end, "exit 1");
    assert_eq!(
        usage.stderr_head,
        "usage: $PROGDIR/tool -a|-m|-w|-t|-s FILE"
    );
    assert!(
        usage.probe_agrees,
        "argv[0] is the same path for plain and probed"
    );
    let clock = by("clock/now");
    assert!(
        !clock.stable,
        "the clock scenario's output differs between runs"
    );
    assert_eq!(map.inputs.features, digest);
    assert_eq!(map.inputs.platform, harness_core::features::platform());
    assert!(map.unwatched.is_empty());
    // Deterministic for its inputs, apart from the unstable scenario.
    let again =
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
            .expect("maps again");
    let stable =
        |m: &harness_core::features::FeatureMap| -> Vec<harness_core::features::ScenarioRecord> {
            m.scenarios
                .iter()
                .filter(|r| r.feature != "clock")
                .cloned()
                .collect()
        };
    assert_eq!(stable(&map), stable(&again));
    // The source tree is untouched: the probe works on a scratch copy.
    let main = std::fs::read_to_string(tmp.path().join("src/tool/main.c")).expect("main.c");
    assert!(!main.contains("__ruharness"));
}

/// Review M7 (third pass): the map refuses a program whose build reads a
/// file the scratch copy would not — an include leaving `source_dir`, a
/// folder linked into it — and maps one whose stray include is never
/// compiled.
#[test]
fn the_map_refuses_what_its_copy_would_build_differently() {
    let map = |root: &Path| {
        let target = TargetContext::load(root).unwrap();
        let facts = with_symbols(root);
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
    };
    let prepend = |root: &Path, text: &str| {
        let main = root.join("src/tool/main.c");
        let old = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{text}{old}")).unwrap();
    };
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";

    // An include that leaves source_dir.
    let tmp = TempDir::new("feat-map-leaves");
    program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/extra.h"), "#define EXTRA 1\n");
    prepend(tmp.path(), "#include \"../extra.h\"\n");
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("outside source_dir"), "{err}");

    // The same include, never compiled: mapped.
    let tmp = TempDir::new("feat-map-if0");
    program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/extra.h"), "#define EXTRA 1\n");
    prepend(
        tmp.path(),
        "#if 0\n#include \"../extra.h\"\n#endif\n/* #include \"../extra.h\" */\n",
    );
    write(
        &tmp.path().join("src/tool/tests/t.c"),
        "#include \"../../extra.h\"\n",
    );
    let mapped = map(tmp.path()).expect("maps");
    assert_eq!(mapped.scenarios.len(), 1);

    // A folder linked into source_dir: the copy holds one path per folder.
    let tmp = TempDir::new("feat-map-alias");
    program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/arch/types.h"), "#define T 1\n");
    std::os::unix::fs::symlink("arch", tmp.path().join("src/tool/sys")).unwrap();
    prepend(tmp.path(), "#include \"sys/types.h\"\n");
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("outside source_dir"), "{err}");
}

/// Review M3: a program that closes every inherited descriptor (the notes'
/// one too) keeps its notes, and never sees the probe in errno.
#[test]
fn a_program_that_closes_its_descriptors_keeps_its_notes_and_its_errno() {
    let tmp = TempDir::new("feat-map-closefrom");
    let features = "schema_version = 1\n[[feature]]\nid = \"closer\"\nname = \"Close all\"\n\
                    [[scenario]]\nfeature = \"closer\"\nid = \"x\"\nargs = [\"-q\"]\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/main.c"),
        "#include <errno.h>\n#include <stdio.h>\n#include <string.h>\n#include <unistd.h>\n\
         #include \"unit.h\"\n#include \"mul.h\"\n\
         static void report(void) { printf(\"%s\\n\", strerror(errno)); }\n\
         int main(int argc, char **argv) {\n\
           (void)argc; (void)argv;\n\
           for (int fd = 3; fd < 4096; fd++) close(fd);\n\
           FILE *f = fopen(\"nosuch\", \"rb\");\n\
           if (!f) report();\n\
           printf(\"%d %u\\n\", unit_add(1, 2), mul_step(0, 1));\n\
           return 0;\n\
         }\n",
    );
    let mut facts = with_symbols(tmp.path());
    facts.symbols.push(harness_core::facts::SymbolRecord {
        name: "src/tool/main.c::report".into(),
        kind: "function".into(),
        file: "src/tool/main.c".into(),
        visibility: "static".into(),
        signature: String::new(),
        span: (1, 1),
    });
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let map =
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
            .expect("maps");
    let r = &map.scenarios[0];
    assert_eq!(r.end, "exit 0");
    assert!(r.probe_agrees, "errno kept: {r:?}");
    assert_eq!(r.noted, "complete");
    let names: Vec<&str> = r.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(
        names,
        ["main", "src/tool/main.c::report", "mul_step", "unit_add"],
        "notes after the close are kept"
    );
}

#[test]
fn a_crash_keeps_the_notes_before_it() {
    let tmp = TempDir::new("feat-map-crash");
    let features = "schema_version = 1\n[[feature]]\nid = \"boom\"\nname = \"Crash\"\n\
                    [[scenario]]\nfeature = \"boom\"\nid = \"z\"\nargs = [\"-a\", \"{input}\"]\n\
                    input = \"sample:text\"\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    // unit_add aborts once it sees a 'z': the C side crashes after main and
    // unit_add have both run.
    write(
        &tmp.path().join("src/tool/unit.c"),
        "#include <stdlib.h>\n#include \"unit.h\"\n\
         int unit_add(int a, int b) { if (b == 122) abort(); return (int)((unsigned)a + (unsigned)b); }\n",
    );
    let facts = with_symbols(tmp.path());
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let map =
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
            .expect("maps");
    let r = &map.scenarios[0];
    assert_eq!(r.end, "signal 6");
    assert_eq!(r.noted, "complete");
    let names: Vec<&str> = r.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["main", "unit_add"]);
}
