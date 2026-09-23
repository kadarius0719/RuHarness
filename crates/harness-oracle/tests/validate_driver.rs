//! `validate_driver` end to end against a tiny synthetic unit: a good driver
//! is green with mutation statistics; each way a driver can be wrong stops
//! the validation at the check that catches it.

mod common;

use common::{write, TempDir};
use harness_core::driver::DriverValidation;
use harness_core::facts::FileRecord;
use harness_core::{Facts, TargetContext, Unit};
use harness_oracle::validate_driver;
use std::path::{Path, PathBuf};

/// Two symbols; a file-scope table; every operator class the gate needs.
const UNIT_C: &str = "#include \"unit.h\"\n\
static const int weights[4] = {30, 50, 70, 110};\n\
int unit_score(int x, int y) {\n\
  int acc = 0;\n\
  for (int i = 0; i < 4; i++) {\n\
    if (x > i && y != 0) acc += weights[i] * x;\n\
    else acc -= (y << 1) ^ i;\n\
  }\n\
  return acc % 1000;\n\
}\n\
int unit_twice(int x) { return x * 2 + 1; }\n";

const GOOD_DRIVER: &str = "#include <stdio.h>\n#include \"unit.h\"\n\
int main(void) {\n\
  for (int x = 0; x <= 20; x++)\n\
    for (int y = 0; y <= 20; y++)\n\
      printf(\"%d %d %d\\n\", x, y, unit_score(x, y));\n\
  for (int x = -5; x <= 40; x++) printf(\"twice %d %d\\n\", x, unit_twice(x));\n\
  return 0;\n\
}\n";

/// A score unit target; returns (context, unit, root). Only `cc` and `nm`
/// are allowlisted: validation needs nothing else.
fn score_target(root: &Path) -> (TargetContext, Unit) {
    write(
        &root.join("harness.toml"),
        "schema_version = 1\n\n[target]\nname = \"score\"\nsource_dir = \"src/score\"\n\n\
         [oracle]\nallowlist = [\"cc\", \"nm\"]\n",
    );
    write(
        &root.join("src/score/unit.h"),
        "int unit_score(int x, int y);\nint unit_twice(int x);\n",
    );
    write(&root.join("src/score/unit.c"), UNIT_C);
    let facts = Facts {
        frontend: "test-inline".to_string(),
        files: vec![
            FileRecord {
                path: "src/score/unit.c".to_string(),
                hash: String::new(),
                includes: vec!["src/score/unit.h".to_string()],
            },
            FileRecord {
                path: "src/score/unit.h".to_string(),
                hash: String::new(),
                includes: Vec::new(),
            },
        ],
        ..Facts::default()
    };
    facts
        .store(&root.join("migration/facts.jsonl"))
        .expect("facts stored");
    let unit: Unit = toml::from_str(
        "id = \"u-score\"\nstatus = \"pending\"\nfiles = [\"src/score/unit.c\"]\n\
         symbols = [\"unit_score\", \"unit_twice\"]\n",
    )
    .expect("unit parses");
    (TargetContext::load(root).expect("target loads"), unit)
}

/// Write `driver` as an attempt's candidate driver and validate it.
fn validate(tag: &str, driver: &str) -> (TempDir, DriverValidation) {
    let tmp = TempDir::new(tag);
    let (target, unit) = score_target(tmp.path());
    let path: PathBuf = tmp
        .path()
        .join("migration/units/u-score/driver-attempts/d-000000000000/candidate/driver.c");
    write(&path, driver);
    let v = validate_driver(&target, &unit, &path).expect("validation runs");
    (tmp, v)
}

fn describe(v: &DriverValidation) -> String {
    v.checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

fn names(v: &DriverValidation) -> Vec<&str> {
    v.checks.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn a_good_driver_is_green_with_mutation_stats() {
    let (tmp, v) = validate("dv-good", GOOD_DRIVER);
    eprintln!("{}\n{:?}", describe(&v), v.mutation);
    assert!(v.green, "{}", describe(&v));
    assert_eq!(
        names(&v),
        [
            "driver-build",
            "driver-shape",
            "symbols-called",
            "determinism",
            "opt-levels",
            "sanitizers",
            "mutation",
        ]
    );
    let stats = v.mutation.as_ref().expect("mutation stats recorded");
    assert_eq!(stats.sites, 22, "{stats:?}");
    assert_eq!(stats.sampled, 22, "{stats:?}");
    assert!(stats.compiled >= 20, "{stats:?}");
    assert!(
        u64::from(stats.killed) * 1000 >= 600 * u64::from(stats.compiled),
        "{stats:?}"
    );
    // Record fields: schema, policy defaults, content-bound inputs.
    assert_eq!(v.schema, "ruharness-driver-validation");
    assert_eq!(v.unit, "u-score");
    assert_eq!(v.policy.max_mutants, 24);
    assert_eq!(v.policy.min_kill_permille, 600);
    assert!(v.inputs.unit_source.starts_with("blake3:"));
    assert!(v.inputs.driver.starts_with("blake3:"));
    assert!(v
        .inputs
        .toolchain
        .iter()
        .any(|t| t == "cflags: -ffp-contract=off"));
    assert!(v
        .inputs
        .toolchain
        .iter()
        .any(|t| t.starts_with("sandbox: ")));
    // Mutants were written under the unit build dir, never next to the unit.
    let dv = tmp.path().join("migration/build/u-score/dv");
    assert!(dv.join("mut-0/unit.c").exists());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("src/score/unit.c")).expect("unit.c"),
        UNIT_C,
        "the original C is never modified"
    );
    // Evidence is machine-independent.
    for check in &v.checks {
        assert!(!check.detail.contains("/Users/"), "{}", check.detail);
    }
}

#[test]
fn a_nondeterministic_driver_fails_determinism() {
    let driver = "#include <stdio.h>\n#include \"unit.h\"\n\
                  int main(void) {\n\
                    int local = unit_score(3, 4) + unit_twice(2);\n\
                    printf(\"%d %lu\\n\", local, (unsigned long)&local);\n\
                    return 0;\n\
                  }\n";
    let (_tmp, v) = validate("dv-nondet", driver);
    assert!(!v.green);
    assert_eq!(
        names(&v),
        [
            "driver-build",
            "driver-shape",
            "symbols-called",
            "determinism"
        ],
        "{}",
        describe(&v)
    );
    let det = &v.checks[3];
    assert!(!det.passed);
    assert!(
        det.detail.contains("printed something else"),
        "{}",
        det.detail
    );
    assert!(v.mutation.is_none());
}

#[test]
fn a_driver_that_skips_a_symbol_fails_symbols_called() {
    let driver = "#include <stdio.h>\n#include \"unit.h\"\n\
                  int main(void) {\n\
                    for (int x = 0; x < 10; x++) printf(\"%d\\n\", unit_score(x, x));\n\
                    return 0;\n\
                  }\n";
    let (_tmp, v) = validate("dv-skip", driver);
    assert!(!v.green);
    assert_eq!(
        names(&v),
        ["driver-build", "driver-shape", "symbols-called"],
        "{}",
        describe(&v)
    );
    assert_eq!(v.checks[2].detail, "the driver never calls: unit_twice");
}

#[test]
fn a_driver_calling_fopen_fails_driver_shape() {
    let driver = "#include <stdio.h>\n#include \"unit.h\"\n\
                  int main(void) {\n\
                    FILE* f = fopen(\"src/score/unit.c\", \"rb\");\n\
                    printf(\"%d %d %d\\n\", f != 0, unit_score(1, 2), unit_twice(3));\n\
                    return 0;\n\
                  }\n";
    let (_tmp, v) = validate("dv-fopen", driver);
    assert!(!v.green);
    assert_eq!(
        names(&v),
        ["driver-build", "driver-shape"],
        "{}",
        describe(&v)
    );
    assert!(
        v.checks[1].detail.contains("fopen"),
        "{}",
        v.checks[1].detail
    );
}

#[test]
fn a_weak_driver_fails_mutation_adequacy() {
    let driver = "#include <stdio.h>\n#include \"unit.h\"\n\
                  int main(void) {\n\
                    printf(\"%d %d\\n\", unit_score(0, 0), unit_twice(3));\n\
                    return 0;\n\
                  }\n";
    let (_tmp, v) = validate("dv-weak", driver);
    eprintln!("{}", describe(&v));
    assert!(!v.green, "{}", describe(&v));
    assert_eq!(v.checks.len(), 7, "{}", describe(&v));
    let mutation = v.checks.last().expect("mutation check");
    assert_eq!(mutation.name, "mutation");
    assert!(!mutation.passed, "{}", mutation.detail);
    let stats = v.mutation.as_ref().expect("stats");
    assert!(!stats.survivors.is_empty());
    // Survivors are harness-generated positions, never source bytes.
    assert!(stats
        .survivors
        .iter()
        .all(|s| s.file == "src/score/unit.c" && s.line > 0));
    assert!(stats
        .survivors
        .iter()
        .any(|s| s.operator == "table-element" && s.function.is_empty()));
}

#[test]
fn a_driver_that_does_not_compile_is_a_failed_check_not_an_error() {
    let driver = "#include \"unit.h\"\n\
                  int main(void) {\n\
                    printf(\"%d\\n\", unit_score(1, 2));\n\
                    return 0;\n\
                  }\n";
    let (_tmp, v) = validate("dv-nobuild", driver);
    assert!(!v.green);
    assert_eq!(names(&v), ["driver-build"]);
    let detail = &v.checks[0].detail;
    assert!(detail.contains("does not compile"), "{detail}");
    assert!(detail.contains("printf"), "{detail}");
    assert!(!detail.contains("/Users/"), "{detail}");
}

#[test]
fn a_driver_outside_the_target_root_is_refused() {
    let tmp = TempDir::new("dv-outside");
    let (target, unit) = score_target(tmp.path());
    let outside = TempDir::new("dv-outside-driver");
    let path = outside.path().join("driver.c");
    write(&path, GOOD_DRIVER);
    let err = validate_driver(&target, &unit, &path).expect_err("outside the root");
    assert!(
        matches!(err, harness_core::Error::InvalidPlan(_)),
        "{err:?}"
    );
}
