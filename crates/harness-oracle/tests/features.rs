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
    let rec = |path: &str, includes: &[&str]| FileRecord {
        path: path.into(),
        hash: String::new(),
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
