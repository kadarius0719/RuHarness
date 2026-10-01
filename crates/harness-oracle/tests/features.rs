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

/// Check of the third pass: a top-level `.c` that is not a regular file — a
/// FIFO would reach the build and hang until the timeout — is a C-side skip,
/// never a wait.
#[test]
fn a_fifo_named_like_a_c_file_is_a_skip_not_a_wait() {
    let tmp = TempDir::new("feat-fifo");
    // A wait on the FIFO would last the whole timeout (120 s); an ordinary
    // verify ends well before 100 s even on a loaded machine (a 20 s bound
    // measured the machine's load, not the wait).
    let (target, unit) = program(tmp.path(), GOOD, Some(FEATURES), "timeout_secs = 120");
    mkfifo(&tmp.path().join("src/tool/x.c"));
    let started = std::time::Instant::now();
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
    assert!(started.elapsed() < std::time::Duration::from_secs(100));
}

fn mkfifo(path: &Path) {
    let made = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
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

    // Under a folder whose name has a space (make escapes it: fix check 3
    // N8), the same include is still refused.
    let tmp = TempDir::new("feat-map-space");
    let spaced = tmp.path().join("sp ace");
    std::fs::create_dir_all(&spaced).unwrap();
    program(&spaced, GOOD, Some(features), "");
    write(&spaced.join("src/extra.h"), "#define EXTRA 1\n");
    prepend(&spaced, "#include \"../extra.h\"\n");
    let err = map(&spaced).expect_err("refused").to_string();
    assert!(err.contains("outside source_dir"), "{err}");
    // … and one that builds maps.
    let tmp = TempDir::new("feat-map-space-ok");
    let spaced = tmp.path().join("sp ace");
    std::fs::create_dir_all(&spaced).unwrap();
    program(&spaced, GOOD, Some(features), "");
    map(&spaced).expect("maps");

    // A folder linked into source_dir: the copy holds one path per folder.
    let tmp = TempDir::new("feat-map-alias");
    program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/arch/types.h"), "#define T 1\n");
    std::os::unix::fs::symlink("arch", tmp.path().join("src/tool/sys")).unwrap();
    prepend(tmp.path(), "#include \"sys/types.h\"\n");
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("outside source_dir"), "{err}");
}

/// Check of the third pass: the map refuses by name, before any build, a
/// top-level `.c` linked out of `source_dir` (the copy never holds it: the
/// include words would name the wrong cause) and one that is not a regular
/// file (a FIFO would hang the build).
#[test]
fn the_map_refuses_an_odd_c_file_by_name() {
    let map = |root: &Path| {
        let target = TargetContext::load(root).unwrap();
        let facts = with_symbols(root);
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
    };
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";

    let tmp = TempDir::new("feat-map-linked-c");
    program(tmp.path(), GOOD, Some(features), "timeout_secs = 20");
    write(
        &tmp.path().join("src/extra.c"),
        "int extra(void) { return 1; }\n",
    );
    std::os::unix::fs::symlink("../extra.c", tmp.path().join("src/tool/extra.c")).unwrap();
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("links outside source_dir (to src/extra.c)"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-fifo");
    program(tmp.path(), GOOD, Some(features), "timeout_secs = 20");
    mkfifo(&tmp.path().join("src/tool/x.c"));
    let started = std::time::Instant::now();
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("src/tool/x.c is not a regular file"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));

    let tmp = TempDir::new("feat-map-dir-c");
    program(tmp.path(), GOOD, Some(features), "timeout_secs = 20");
    std::fs::create_dir_all(tmp.path().join("src/tool/y.c")).unwrap();
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("src/tool/y.c is not a regular file"), "{err}");
}

/// Fix check 4: what the copy would read differently, refused by name — a
/// project file reached from a `system_header` (F1: `-MM` drops it), the
/// original read through `#include __FILE__` (F2), a `source_dir` reached
/// through a link (F5), a `.c` the copy holds under another path (F6).
#[test]
fn the_map_refuses_a_copy_that_reads_other_files() {
    let map = |root: &Path| {
        let target = TargetContext::load(root).unwrap();
        let facts = with_symbols(root);
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
    };
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let prepend = |root: &Path, text: &str| {
        let main = root.join("src/tool/main.c");
        let old = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{text}{old}")).unwrap();
    };

    let tmp = TempDir::new("feat-map-sysheader");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/compat.h"),
        "#pragma GCC system_header\n#include \"../include/stdio.h\"\n",
    );
    // The copy falls through to the system's `../include/stdio.h`.
    write(&tmp.path().join("src/include/stdio.h"), "#define EXTRA 1\n");
    prepend(tmp.path(), "#include \"compat.h\"\n");
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("src/tool/../include/stdio.h, which the scratch copy would not"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-file");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/mul.c"),
        "#ifndef AGAIN\n#define AGAIN\n#include __FILE__\n#else\n#include \"mul.h\"\n\
         unsigned mul_step(unsigned acc, int c) { return acc * 31u + (unsigned)c; }\n#endif\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.starts_with("the scratch copy would read src/tool/mul.c itself"),
        "the refusal's own words, not the include words (fix check 5 N1): {err}"
    );
    assert!(
        !tmp.path().join("migration/build/.features/plain").exists(),
        "refused before any build (fix check 5 N2)"
    );

    let tmp = TempDir::new("feat-map-linked-dir");
    program(tmp.path(), GOOD, Some(features), "");
    std::fs::rename(tmp.path().join("src/tool"), tmp.path().join("src/real")).unwrap();
    std::os::unix::fs::symlink("real", tmp.path().join("src/tool")).unwrap();
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("resolves to src/real (through a link"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-alias-c");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/zdir/x.c"),
        "int x_c(void) { return 1; }\n",
    );
    std::os::unix::fs::symlink("zdir", tmp.path().join("src/tool/a_link")).unwrap();
    std::os::unix::fs::symlink("zdir/x.c", tmp.path().join("src/tool/b.c")).unwrap();
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("src/tool/zdir/x.c is reached through a link"),
        "{err}"
    );
}

/// Fix check 5: the copy's reads are compared per compile and in order — a
/// folder linked into `source_dir` reached by both its names, in two files
/// (M1) or in one, and a file linked to another that `#pragma once` sees as
/// one (L1); a name with a newline is refused (L2); an absolute `source_dir`
/// inside the root maps (N3).
#[test]
fn the_map_compares_each_compile_in_order() {
    let map = |root: &Path| {
        let target = TargetContext::load(root).unwrap();
        let facts = with_symbols(root);
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
    };
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let prepend = |root: &Path, file: &str, text: &str| {
        let path = root.join("src/tool").join(file);
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}{old}")).unwrap();
    };
    // `sys` links to `proj`; the copy's `sys/un.h` falls through to the
    // system's.
    let aliased = |name: &str| {
        let tmp = TempDir::new(name);
        program(tmp.path(), GOOD, Some(features), "");
        write(&tmp.path().join("src/tool/proj/un.h"), "#define PROJ_T 7\n");
        std::os::unix::fs::symlink("proj", tmp.path().join("src/tool/sys")).unwrap();
        tmp
    };

    let tmp = aliased("feat-map-cross-tu");
    prepend(tmp.path(), "main.c", "#include \"sys/un.h\"\n");
    prepend(tmp.path(), "unit.c", "#include \"proj/un.h\"\n");
    // Per compile, the files alone already tell (M1).
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("the program reads src/tool/sys/un.h, which the scratch copy would not"),
        "{err}"
    );

    // In one compile the names tell too (fix check 6: the name the
    // compiler used, not only the file).
    let tmp = aliased("feat-map-same-tu");
    prepend(
        tmp.path(),
        "main.c",
        "#include \"sys/un.h\"\n#include \"proj/un.h\"\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("the program reads src/tool/sys/un.h, which the scratch copy would not"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-pragma-once");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/y.h"),
        "#pragma once\n#ifdef SEEN_Y\n#define TWICE\n#endif\n#define SEEN_Y\n",
    );
    std::os::unix::fs::symlink("y.h", tmp.path().join("src/tool/x.h")).unwrap();
    prepend(tmp.path(), "main.c", "#include \"y.h\"\n#include \"x.h\"\n");
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("the program's header #2 is")
            && err.contains("would be src/tool/x.h (depth 1)"),
        "{err}"
    );

    // A name with a control character stays out of the copy (a Finder
    // `Icon\r` is no reason to refuse; fix check 6 L1); a `.c` with one is
    // refused by name.
    let tmp = TempDir::new("feat-map-newline");
    program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/Icon\r"), "");
    map(tmp.path()).expect("an unread Icon\\r maps");
    // A tab the program reads: `-M` leaves it unescaped, so both lists
    // would split the name alike (`…/a` and `b.h`, each resolving) — the
    // copy never holds it, and its compile fails instead.
    let tabbed = TempDir::new("feat-map-tab");
    program(tabbed.path(), GOOD, Some(features), "");
    write(&tabbed.path().join("src/tool/a\tb.h"), "#define AB 1\n");
    write(&tabbed.path().join("src/tool/a/keep.h"), "");
    write(&tabbed.path().join("b.h"), "");
    let main = tabbed.path().join("src/tool/main.c");
    let old = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, format!("#include \"a\tb.h\"\n{old}")).unwrap();
    let err = map(tabbed.path()).expect_err("refused").to_string();
    assert!(err.contains("the scratch copy cannot find"), "{err}");
    write(
        &tmp.path().join("src/tool/a\nb.c"),
        "int ab(void) { return 1; }\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("has a control character in its name"), "{err}");

    let tmp = TempDir::new("feat-map-absolute");
    program(tmp.path(), GOOD, Some(features), "");
    let toml = tmp.path().join("harness.toml");
    let text = std::fs::read_to_string(&toml).unwrap().replace(
        "source_dir = \"src/tool\"",
        &format!(
            "source_dir = {:?}",
            tmp.path().join("src/tool").display().to_string()
        ),
    );
    std::fs::write(&toml, text).unwrap();
    map(tmp.path()).expect("an absolute source_dir inside the root maps");
}

/// Fix check 6: a lookup through a folder linked into `source_dir` differs
/// by the name the compiler used (M2: `__has_include`), and the headers
/// entered by their depth (M1); a listing that fails says the compiler's own
/// words (L2); a target folder with '=' in its path is refused (L4).
#[test]
fn the_map_compares_names_and_depths() {
    let map = |root: &Path| {
        let target = TargetContext::load(root).unwrap();
        let facts = with_symbols(root);
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
    };
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let prepend = |root: &Path, text: &str| {
        let path = root.join("src/tool/main.c");
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}{old}")).unwrap();
    };

    let tmp = TempDir::new("feat-map-has-include");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/proj/feat.h"),
        "#ifndef FEAT_H\n#define FEAT_H\n#endif\n",
    );
    std::os::unix::fs::symlink("proj", tmp.path().join("src/tool/sys")).unwrap();
    prepend(
        tmp.path(),
        "#include \"proj/feat.h\"\n#if __has_include(\"sys/feat.h\")\n#define RUN 1\n#endif\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.contains("the program reads src/tool/sys/feat.h, which the scratch copy would not"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-depth");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/arch/types.h"),
        "#ifndef ARCH_TYPES_H\n#define ARCH_TYPES_H\n#define PROJ_T 7\n#endif\n",
    );
    std::fs::create_dir_all(tmp.path().join("src/tool/lib")).unwrap();
    std::os::unix::fs::symlink("../arch", tmp.path().join("src/tool/lib/sys")).unwrap();
    write(
        &tmp.path().join("src/tool/lib/a.h"),
        "#include \"sys/types.h\"\n#ifdef PROJ_T\n#define RUN 1\n#else\n#define RUN 0\n#endif\n",
    );
    prepend(
        tmp.path(),
        "#include <sys/types.h>\n#include \"lib/a.h\"\n#include \"arch/types.h\"\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(err.contains("src/tool/lib/sys/types.h"), "{err}");

    // A lookup from the mirror for what the map builds finds nothing: it
    // all lives in a random folder outside the target
    // (docs/FEATURES-PROBE-REDESIGN.md §3.4 "Order").
    for looked_up in ["../../../fnprobe.c", "../../../plain", "../../../fnprobe.h"] {
        let tmp = TempDir::new("feat-map-copy-only");
        program(tmp.path(), GOOD, Some(features), "");
        prepend(
            tmp.path(),
            &format!("#if __has_include(\"{looked_up}\")\n#error found\n#endif\n"),
        );
        map(tmp.path()).expect("the lookup finds nothing, in the program and the copy alike");
    }

    let tmp = TempDir::new("feat-map-words");
    program(tmp.path(), GOOD, Some(features), "");
    prepend(
        tmp.path(),
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#include \"missing.h\"\n",
    );
    let err = map(tmp.path()).expect_err("refused").to_string();
    assert!(
        err.starts_with("the C program does not build")
            && err.contains("'missing.h' file not found"),
        "{err}"
    );

    let tmp = TempDir::new("feat-map-eq");
    let root = tmp.path().join("a=b");
    std::fs::create_dir_all(&root).unwrap();
    program(&root, GOOD, Some(features), "");
    let err = map(&root).expect_err("refused").to_string();
    assert!(err.contains("has '=' in its path"), "{err}");
}

/// Review M3: a program that closes every inherited descriptor (the notes'
/// one too) keeps its notes, and never sees the probe in errno.
/// docs/FEATURES-PROBE-REDESIGN.md §3.6: the runtime's setup reads the
/// notes file from `TMPDIR`; a program whose own constructor changes
/// `TMPDIR` first leaves the attach byte unset — "the probe's setup did not
/// run", never a complete record where nothing ran. A program that lists its
/// temp dir sees the same entries plain and probed (the notes file is in
/// every run's), and one that defines `strlen` is not recorded as running it
/// (the note calls nothing).
#[test]
fn the_runtime_attaches_or_says_it_did_not() {
    let features = "schema_version = 1\n[[feature]]\nid = \"f\"\nname = \"F\"\n\
                    [[scenario]]\nfeature = \"f\"\nid = \"x\"\nargs = [\"-q\"]\n";
    let map_of = |root: &Path, main: &str, extra: &[&str]| {
        let (target, _) = program(root, GOOD, Some(features), "");
        write(&root.join("src/tool/main.c"), main);
        let mut facts = with_symbols(root);
        for name in extra {
            facts.symbols.push(harness_core::facts::SymbolRecord {
                name: (*name).into(),
                kind: "function".into(),
                file: "src/tool/main.c".into(),
                visibility: "public".into(),
                signature: String::new(),
                span: (1, 1),
            });
        }
        let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
            panic!("valid")
        };
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
            .expect("maps")
    };

    // The runtime is linked first: its setup runs before the program's own
    // constructors, so one that changes TMPDIR comes too late to matter.
    let tmp = TempDir::new("feat-map-tmpdir-moved");
    let map = map_of(
        tmp.path(),
        "#include <stdlib.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
         __attribute__((constructor)) static void away(void) { setenv(\"TMPDIR\", \"/nonexistent\", 1); }\n\
         int main(void) { return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1); }\n",
        &["src/tool/main.c::away"],
    );
    let r = &map.scenarios[0];
    assert_eq!(r.noted, "complete", "{r:?}");
    let names: Vec<&str> = r.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["main", "src/tool/main.c::away", "unit_add"]);

    let tmp = TempDir::new("feat-map-lists-tmpdir");
    let map = map_of(
        tmp.path(),
        "#include <dirent.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
         int main(void) {\n\
           DIR *d = opendir(getenv(\"TMPDIR\"));\n\
           int n = 0; struct dirent *e;\n\
           while (d && (e = readdir(d))) n++;\n\
           printf(\"%d entries %d\\n\", n, unit_add(1, 2));\n\
           return 0;\n\
         }\n",
        &[],
    );
    let r = &map.scenarios[0];
    assert!(r.probe_agrees, "the same entries plain and probed: {r:?}");
    assert_eq!(r.noted, "complete");

    let tmp = TempDir::new("feat-map-own-strlen");
    let map = map_of(
        tmp.path(),
        "#include <stddef.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
         size_t strlen(const char *s) { size_t n = 0; while (s[n]) n++; return n; }\n\
         int main(void) { return unit_add(1, 2) == 3 ? 0 : 1; }\n",
        &["strlen"],
    );
    let r = &map.scenarios[0];
    assert_eq!(r.noted, "complete", "{r:?}");
    let names: Vec<&str> = r.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["main", "unit_add"], "strlen never ran");
}

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

/// docs/FEATURES-PROBE-REDESIGN.md §3.6, the runtime itself: with the notes
/// file in `TMPDIR`, setup maps it, merges the notes made before it and sets
/// the attach byte; a second image adds its notes; without `TMPDIR` the
/// file stays all zeros — read as "the probe's setup did not run".
#[test]
fn the_runtime_merges_and_marks_that_it_attached() {
    let tmp = TempDir::new("fnprobe-runtime");
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.c");
    let main = tmp.path().join("main.c");
    write(
        &main,
        "#include <stdlib.h>\n#include <unistd.h>\n\
         extern unsigned char *volatile __ruharness_seen;\n\
         __attribute__((constructor)) static void early(void) { __ruharness_seen[0] = 1; }\n\
         static void second(void) { __ruharness_seen[2] = 1; }\n\
         int main(int argc, char **argv) {\n\
           __ruharness_seen[1] = 1;\n\
           if (argc > 1) { second(); return 0; }\n\
           char *again[] = { argv[0], \"again\", 0 };\n\
           pid_t p = fork();\n\
           if (p == 0) { execv(argv[0], again); _exit(9); }\n\
           int st; while (wait(&st) < 0) {}\n\
           return 0;\n\
         }\n",
    );
    let bin = tmp.path().join("probed");
    let built = std::process::Command::new("cc")
        .args(["-O2", "-DRUHARNESS_FNPROBE_N=4", "-include"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.h"))
        .arg("-o")
        .arg(&bin)
        // The program's object first: its constructor's note lands in the
        // early array before setup, so the merge is what keeps it (with the
        // runtime first, setup ran first and nothing needed merging — the
        // fix pass's mutation check found the merge untested).
        .arg(&main)
        .arg(&runtime)
        .status()
        .expect("cc runs");
    assert!(built.success());
    let dir = tmp.path().join("t");
    std::fs::create_dir_all(&dir).unwrap();
    let notes = dir.join(".ruharness-notes");
    std::fs::write(&notes, [0u8; 5]).unwrap();
    let ran = std::process::Command::new(&bin)
        .env_clear()
        .env("TMPDIR", &dir)
        .status()
        .expect("runs");
    assert!(ran.success());
    assert_eq!(
        std::fs::read(&notes).unwrap(),
        [1, 1, 1, 0, 1],
        "the constructor's note merged, the child image's note added, attached"
    );
    std::fs::write(&notes, [0u8; 5]).unwrap();
    let ran = std::process::Command::new(&bin)
        .env_clear()
        .status()
        .expect("runs");
    assert!(ran.success());
    assert_eq!(std::fs::read(&notes).unwrap(), [0u8; 5], "not attached");
}

/// docs/FEATURES-PROBE-REDESIGN.md §3.3: a probed file read as data —
/// `#embed` in any spelling, `__has_embed`, a `__has_include` lookup — goes
/// back unprobed and the map succeeds; a note turned into text by a macro
/// is taken out; a probed header both included and embedded is refused by
/// the same-code check; a target under a folder with non-ASCII names maps.
#[test]
fn the_copy_is_the_same_program_or_the_map_says_why() {
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let map_with = |root: &Path, extra: &[(&str, &str)]| {
        let target = TargetContext::load(root).unwrap();
        let mut facts = with_symbols(root);
        for (file, name) in extra {
            facts.symbols.push(harness_core::facts::SymbolRecord {
                name: (*name).into(),
                kind: "function".into(),
                file: (*file).into(),
                visibility: "public".into(),
                signature: String::new(),
                span: (1, 1),
            });
        }
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
    for spelling in [
        "#embed \"unit.c\"",
        "# embed \"unit.c\"",
        "%:embed \"unit.c\"",
        "#/*c*/embed \"unit.c\"",
    ] {
        let tmp = TempDir::new("feat-map-embed");
        program(tmp.path(), GOOD, Some(features), "");
        prepend(
            tmp.path(),
            &format!("static const unsigned char unit_src[] = {{\n{spelling}\n}};\n"),
        );
        let map = map_with(tmp.path(), &[]).expect("maps");
        assert!(
            map.unwatched
                .contains(&("src/tool/unit.c".to_string(), "unit_add".to_string())),
            "{spelling}: {:?}",
            map.unwatched
        );
    }
    let tmp = TempDir::new("feat-map-has-embed");
    program(tmp.path(), GOOD, Some(features), "");
    prepend(
        tmp.path(),
        "#if __has_embed(\"unit.c\")\n#define HAS 1\n#endif\n",
    );
    let map = map_with(tmp.path(), &[]).expect("maps");
    assert!(map
        .unwatched
        .contains(&("src/tool/unit.c".to_string(), "unit_add".to_string())));
    let tmp = TempDir::new("feat-map-has-include-only");
    program(tmp.path(), GOOD, Some(features), "");
    prepend(
        tmp.path(),
        "#if __has_include(\"mul.c\")\n#define HAS 1\n#endif\n",
    );
    let map = map_with(tmp.path(), &[]).expect("maps");
    assert!(map
        .unwatched
        .contains(&("src/tool/mul.c".to_string(), "mul_step".to_string())));

    // A note a macro turns into text: taken out, the program's text kept.
    let tmp = TempDir::new("feat-map-stringized");
    program(tmp.path(), GOOD, Some(features), "");
    prepend(
        tmp.path(),
        "#define STR(...) #__VA_ARGS__\n#define SHADER static const char frag[] = STR\n\
         SHADER(\nvoid shade(void) { color = vec4(1.0); }\n);\n",
    );
    let map = map_with(tmp.path(), &[("src/tool/main.c", "shade")]).expect("maps");
    assert!(
        map.unwatched
            .contains(&("src/tool/main.c".to_string(), "shade".to_string())),
        "{:?}",
        map.unwatched
    );

    // A probed header both included and embedded: the embedded bytes would
    // carry notes — refused by the same-code check, by name.
    let tmp = TempDir::new("feat-map-include-and-embed");
    program(tmp.path(), GOOD, Some(features), "");
    write(
        &tmp.path().join("src/tool/twice.h"),
        "static inline int twice(int x) { return 2 * x; }\n",
    );
    prepend(
        tmp.path(),
        "#include \"twice.h\"\nstatic const unsigned char twice_src[] = {\n#embed \"twice.h\"\n};\n",
    );
    let err = map_with(
        tmp.path(),
        &[("src/tool/twice.h", "src/tool/twice.h::twice")],
    )
    .expect_err("refused")
    .to_string();
    assert!(err.contains("is not the same program near"), "{err}");

    // A target under folders named with non-ASCII characters maps.
    let tmp = TempDir::new("feat-map-unicode");
    let odd = tmp.path().join("café\u{a0}dir");
    std::fs::create_dir_all(&odd).unwrap();
    program(&odd, GOOD, Some(features), "");
    map_with(&odd, &[]).expect("maps");
}

/// Writes `main.c` (with `extra` files), records `functions` of main.c in
/// the facts beside the fixture's, and maps it.
fn map_program(
    name: &str,
    main: &str,
    extra: &[(&str, &str)],
    functions: &[(&str, &str)],
) -> (
    TempDir,
    Result<harness_core::features::FeatureMap, harness_core::error::Error>,
) {
    let tmp = TempDir::new(name);
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/main.c"), main);
    for (path, text) in extra {
        write(&tmp.path().join(path), text);
    }
    let mut facts = with_symbols(tmp.path());
    for (file, id) in functions {
        facts.symbols.push(harness_core::facts::SymbolRecord {
            name: (*id).into(),
            kind: "function".into(),
            file: (*file).into(),
            visibility: "public".into(),
            signature: String::new(),
            span: (1, 1),
        });
    }
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let map =
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()));
    (tmp, map)
}

fn unwatched_ids(map: &harness_core::features::FeatureMap) -> Vec<&str> {
    map.unwatched.iter().map(|(_, id)| id.as_str()).collect()
}

/// docs/FEATURES-PROBE-REDESIGN.md §3.4: the compiler decides — a note it
/// rejects is taken out of exactly the function its error lands in, and
/// the map succeeds; the functions whose notes compile stay watched.
#[test]
fn the_compiler_decides_which_notes_stay() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's pragma errors: skipped on this compiler");
        return;
    }
    let main = "#include \"unit.h\"\n#include \"mul.h\"\n\
                #define FENV_ON _Pragma(\"STDC FENV_ACCESS ON\")\n\
                #define DO_PRAGMA(x) _Pragma(#x)\n\
                #define FENV(x) DO_PRAGMA(STDC FENV_ACCESS x)\n\
                double a(double x) {\n#pragma STDC FENV_ACCESS ON\n  return x * 2; }\n\
                double b(double x) { FENV_ON\n  return x; }\n\
                double c(double x) {\n  FENV(ON)\n  x = x + 1;\n  return x; }\n\
                int l(int x) { __label__ out; if (x) goto out; return 0; out: return 1; }\n\
                int ok(int x) { return x + 1; }\n\
                int main(void) { return (int)a(1) + (int)b(1) + (int)c(1) + l(0) + ok(0) + unit_add(1, 2) - 11 - (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program(
        "feat-map-compiler-decides",
        main,
        &[],
        &[
            ("src/tool/main.c", "a"),
            ("src/tool/main.c", "b"),
            ("src/tool/main.c", "c"),
            ("src/tool/main.c", "l"),
            ("src/tool/main.c", "ok"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        unwatched_ids(&map),
        ["a", "b", "c", "l"],
        "{:?}",
        map.unwatched
    );
    // Each with its reason: the compiler's own words.
    for r in &map.unwatched_reasons {
        assert_eq!(r.kind, "compile", "{r:?}");
        assert!(!r.detail.is_empty() && r.detail.len() <= 160, "{r:?}");
    }
    assert_eq!(map.unwatched_reasons.len(), 4);
    assert!(map
        .unwatched_reasons
        .iter()
        .any(|r| r.id == "a" && r.detail.contains("FENV_ACCESS")));
    let r = &map.scenarios[0];
    assert_eq!(r.noted, "complete", "{r:?}");
    let ran: Vec<&str> = r.functions.iter().map(|(_, n)| n.as_str()).collect();
    assert!(ran.contains(&"ok") && ran.contains(&"main"), "{ran:?}");
}

/// Two hundred rejected notes across a header and the file — well over the
/// runner's 8 KiB error excerpt — are all read: at most two rounds, exactly
/// those unwatched.
#[test]
fn every_error_line_is_read() {
    if cfg!(not(target_os = "macos")) {
        return;
    }
    let mut header = String::from("#define FENV_ON _Pragma(\"STDC FENV_ACCESS ON\")\n");
    let mut main = String::from("#include \"unit.h\"\n#include \"mul.h\"\n#include \"many.h\"\n");
    let mut functions = Vec::new();
    let mut names = Vec::new();
    for i in 0..100 {
        header.push_str(&format!(
            "static inline double h{i}(double x) {{ FENV_ON\n  return x; }}\n"
        ));
        names.push(format!("src/tool/many.h::h{i}"));
    }
    for i in 0..100 {
        main.push_str(&format!(
            "double m{i}(double x) {{ FENV_ON\n  return x + h{i}(x); }}\n"
        ));
        names.push(format!("m{i}"));
    }
    main.push_str(
        "int main(void) { return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1) + (int)m0(1); }\n",
    );
    for n in &names {
        let file = if n.starts_with("src/tool/many.h") {
            "src/tool/many.h"
        } else {
            "src/tool/main.c"
        };
        functions.push((file, n.as_str()));
    }
    let (_tmp, map) = map_program(
        "feat-map-many-errors",
        &main,
        &[("src/tool/many.h", &header)],
        &functions,
    );
    let map = map.expect("maps");
    let mut unwatched: Vec<&str> = unwatched_ids(&map);
    unwatched.sort();
    let mut expected: Vec<&str> = names.iter().map(String::as_str).collect();
    expected.sort();
    assert_eq!(unwatched, expected);
    assert_eq!(map.scenarios[0].noted, "complete");
}

/// Placement by byte: ten functions on one line, a tab and a multibyte
/// character before the error's column, only the last one rejected; a
/// bison-shaped `#line`; an error in an included `.inc` placed through the
/// include chain.
#[test]
fn an_error_lands_in_the_function_that_holds_it() {
    if cfg!(not(target_os = "macos")) {
        return;
    }
    let mut line = String::from("\t/* é */ ");
    let mut functions = Vec::new();
    let names: Vec<String> = (0..10).map(|i| format!("f{i}")).collect();
    for (i, n) in names.iter().enumerate() {
        if i == 9 {
            line.push_str(&format!(
                "int {n}(int x) {{ __label__ out; if (x) goto out; return 0; out: return 1; }} "
            ));
        } else {
            line.push_str(&format!("int {n}(int x) {{ return x + {i}; }} "));
        }
    }
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{line}\n\
         #line 40 \"gen.y\"\n\
         double g(double x) {{\n#pragma STDC FENV_ACCESS ON\n  return x; }}\n\
         double inc(double x) {{\n#include \"fenv.inc\"\n  return x; }}\n\
         int main(void) {{ return f0(0) + f9(0) + (int)g(0) + (int)inc(0) + unit_add(1, 2) - 3 - (int)mul_step(0, 0); }}\n"
    );
    for n in &names {
        functions.push(("src/tool/main.c", n.as_str()));
    }
    functions.push(("src/tool/main.c", "g"));
    functions.push(("src/tool/main.c", "inc"));
    let (_tmp, map) = map_program(
        "feat-map-placement",
        &main,
        &[("src/tool/fenv.inc", "#pragma STDC FENV_ACCESS ON\n")],
        &functions,
    );
    let map = map.expect("maps");
    assert_eq!(
        unwatched_ids(&map),
        ["f9", "g", "inc"],
        "{:?}",
        map.unwatched
    );
}

/// docs/FEATURES-PROBE-REDESIGN.md §3.5: a program that defines a name the
/// probe's runtime uses before `main` is refused by name; an absolute
/// include of a header without notes is the same file either way and maps.
#[test]
fn the_runtimes_names_and_function_less_headers() {
    let main = "#include <unistd.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
                int close(int fd) { (void)fd; return 0; }\n\
                int main(void) { return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program(
        "feat-map-own-close",
        main,
        &[],
        &[("src/tool/main.c", "close")],
    );
    let err = map.expect_err("refused").to_string();
    assert!(err.contains("the program defines close()"), "{err}");

    let tmp = TempDir::new("feat-map-absolute-header");
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/consts.h"), "#define ANSWER 42\n");
    let consts = tmp.path().join("src/tool/consts.h").canonicalize().unwrap();
    let main = tmp.path().join("src/tool/main.c");
    let old = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, format!("#include \"{}\"\n{old}", consts.display())).unwrap();
    let facts = with_symbols(tmp.path());
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
        .expect("an absolute include of a header without notes maps");
}

// ---- The compiler-guided probe's code review, its first fix pass ----

/// A progress that keeps its messages.
struct Loud(Vec<String>);

impl harness_oracle::MapProgress for Loud {
    fn message(&mut self, m: &str) {
        self.0.push(m.to_string());
    }
    fn scenario(&mut self, _: &harness_core::features::ScenarioRecord, _: usize, _: usize) {}
}

/// [`map_program`] under `bounds`, keeping the progress messages.
fn map_program_bounded(
    name: &str,
    main: &str,
    extra: &[(&str, &str)],
    functions: &[(&str, &str)],
    bounds: harness_oracle::MapBounds,
) -> (
    TempDir,
    Result<harness_core::features::FeatureMap, harness_core::error::Error>,
    Vec<String>,
) {
    let tmp = TempDir::new(name);
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    write(&tmp.path().join("src/tool/main.c"), main);
    for (path, text) in extra {
        write(&tmp.path().join(path), text);
    }
    let mut facts = with_symbols(tmp.path());
    for (file, id) in functions {
        facts.symbols.push(harness_core::facts::SymbolRecord {
            name: (*id).into(),
            kind: "function".into(),
            file: (*file).into(),
            visibility: "public".into(),
            signature: String::new(),
            span: (1, 1),
        });
    }
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let mut loud = Loud(Vec::new());
    let map = harness_oracle::with_map_bounds(bounds, || {
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut loud)
    });
    (tmp, map, loud.0)
}

const DESIGN_BOUNDS: harness_oracle::MapBounds = harness_oracle::MapBounds {
    placed_rounds: 8,
    file_compiles: 64,
    pass_compiles: 400,
};

fn kind_of<'a>(map: &'a harness_core::features::FeatureMap, id: &str) -> Option<&'a str> {
    map.unwatched_reasons
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.kind.as_str())
}

fn ran(map: &harness_core::features::FeatureMap) -> Vec<&str> {
    map.scenarios[0]
        .functions
        .iter()
        .map(|(_, n)| n.as_str())
        .collect()
}

/// Review: an `#if` sibling the parser cannot see shares its id — the
/// visible definition sits in the branch the build skips, so the compiled
/// one carries no note. It is unwatched with its reason, never "not run".
#[test]
fn a_compiled_definition_the_parser_cannot_read_is_unwatched() {
    let shapes = [
        (
            "macro",
            "#define DEFINE_ADD(T) T add(T a, T b) { return a + b; }\n\
             #ifndef PORTABLE\nDEFINE_ADD(int)\n#else\nint add(int a, int b) { return a + b; }\n#endif\n",
            "add",
            "add(1, 2)",
        ),
        (
            "paren",
            "#if 0\nint add(int x, int y) { return x - y; }\n#else\nint (add)(int x, int y) { return x + y; }\n#endif\n",
            "add",
            "add(1, 2)",
        ),
        (
            "static",
            "#if 0\nstatic int sadd(int x, int y) { return x - y; }\n#else\n__attribute__((noinline)) static int (sadd)(int x, int y) { return x + y; }\n#endif\n\
             __attribute__((noinline)) int add2(int x, int y) { return sadd(x, y); }\n",
            "src/tool/main.c::sadd",
            "add2(1, 2)",
        ),
    ];
    for (name, defs, id, call) in shapes {
        let main = format!(
            "#include \"unit.h\"\n#include \"mul.h\"\n{defs}\
             __attribute__((noinline)) int use(void) {{ return {call}; }}\n\
             int main(void) {{ return unit_add(1, 2) == 3 ? use() - 3 : (int)mul_step(0, 1); }}\n"
        );
        let mut functions = vec![("src/tool/main.c", id), ("src/tool/main.c", "use")];
        if name == "static" {
            functions.push(("src/tool/main.c", "add2"));
        }
        let (_tmp, map) = map_program(
            &format!("feat-review-hidden-{name}"),
            &main,
            &[],
            &functions,
        );
        let map = map.expect("maps");
        let reason = map.unwatched_reasons.iter().find(|r| r.id == id);
        assert!(
            reason.is_some_and(|r| r.kind == "parser" && r.detail.contains("another #if branch")),
            "{name}: {:?}",
            map.unwatched_reasons
        );
        assert!(ran(&map).contains(&"use"), "{name}: {:?}", ran(&map));
    }
    // Both readable: the compiled one carries the note, and it ran.
    let main = "#include \"unit.h\"\n#include \"mul.h\"\n\
                #if 0\nint add(int x, int y) { return x - y; }\n#else\nint add(int x, int y) { return x + y; }\n#endif\n\
                int main(void) { return unit_add(1, 2) == 3 ? add(1, 2) - 3 : (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program(
        "feat-review-hidden-control",
        main,
        &[],
        &[("src/tool/main.c", "add")],
    );
    let map = map.expect("maps");
    assert!(map.unwatched.is_empty(), "{:?}", map.unwatched_reasons);
    assert!(ran(&map).contains(&"add"));
}

/// Review: a GNU raw string with a lone `"` on the line before a stringized
/// body put the old scanner out of step — the note in the string read as
/// code, the probed program was another program. The tokenizer keeps the
/// literal whole: the function is unwatched, the map records what ran.
#[test]
fn a_raw_string_never_hides_a_stringized_note() {
    let main = "#include <stdio.h>\n#include <string.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
                #define STR(...) #__VA_ARGS__\n\
                #define SHADER(...) static const char *pre = R\"(\")\"; static const char frag[] = STR(__VA_ARGS__)\n\
                SHADER(\nvoid shade(void) { color = vec4(1.0); }\n);\n\
                void longer(void) { puts(\"x\"); } void shorter(void) { puts(\"x\"); }\n\
                int main(void) { (void)pre; if (unit_add(1, 2) != 3) return (int)mul_step(0, 1);\n\
                if (strlen(frag) > 45) longer(); else shorter(); return 0; }\n";
    let (_tmp, map) = map_program(
        "feat-review-raw-string",
        main,
        &[],
        &[
            ("src/tool/main.c", "shade"),
            ("src/tool/main.c", "longer"),
            ("src/tool/main.c", "shorter"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "shade"),
        Some("stringized"),
        "{:?}",
        map.unwatched_reasons
    );
    let ran = ran(&map);
    assert!(
        ran.contains(&"shorter") && !ran.contains(&"longer"),
        "{ran:?}"
    );
}

/// Review: a Latin-1 byte on a function's line made the same-code check
/// refuse an ordinary program (its end token's spaces were kept).
#[test]
fn a_latin1_line_maps() {
    let main = b"#include <stdio.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
                 int greet(void) { return puts(\"caf\xE9\"); }\n\
                 int main(void) { return unit_add(1, 2) == 3 ? greet() < 0 : (int)mul_step(0, 1); }\n";
    let tmp = TempDir::new("feat-review-latin1");
    let features = "schema_version = 1\n[[feature]]\nid = \"use\"\nname = \"Usage\"\n\
                    [[scenario]]\nfeature = \"use\"\nid = \"none\"\nargs = []\n";
    let (target, _) = program(tmp.path(), GOOD, Some(features), "");
    std::fs::write(tmp.path().join("src/tool/main.c"), main).unwrap();
    let mut facts = with_symbols(tmp.path());
    facts.symbols.push(harness_core::facts::SymbolRecord {
        name: "greet".into(),
        kind: "function".into(),
        file: "src/tool/main.c".into(),
        visibility: "public".into(),
        signature: String::new(),
        span: (1, 1),
    });
    let FeatureSnapshot::Valid { features, digest } = FeatureSnapshot::load(&target) else {
        panic!("valid")
    };
    let map =
        harness_oracle::map_features(&target, &facts, &features, &digest, &mut Quiet(Vec::new()))
            .expect("maps");
    assert!(map.unwatched.is_empty(), "{:?}", map.unwatched_reasons);
    assert!(ran(&map).contains(&"greet"));
}

/// Review: an `.incbin` whose directive is split over two string literals
/// on two lines read the probed copy's bytes.
#[test]
fn an_incbin_split_across_lines_unprobes_the_file_it_reads() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("Mach-O symbol names: skipped here");
        return;
    }
    let main = "#include \"unit.h\"\n#include \"mul.h\"\n\
                __asm__(\".data\\n.globl _blob\\n_blob:\\n.inc\"\n\
                \"bin \\\"unit.c\\\"\\n.globl _blob_end\\n_blob_end:\\n.text\\n\");\n\
                extern const char blob[], blob_end[];\n\
                int main(void) { return unit_add(1, 2) == 3 ? (blob_end - blob > 100000) : (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program("feat-review-incbin-split", main, &[], &[]);
    let map = map.expect("maps");
    let reason = map.unwatched_reasons.iter().find(|r| r.id == "unit_add");
    assert!(
        reason.is_some_and(|r| r.kind == "data" && r.detail.contains(".incbin")),
        "{:?}",
        map.unwatched_reasons
    );
}

/// Review: §3.4 step 6's pass bound counts the compiles beyond each file's
/// first — files whose notes all compile never reach it.
#[test]
fn the_pass_bound_counts_only_compiles_beyond_the_first() {
    let files: Vec<(String, String)> = (0..3)
        .map(|i| {
            (
                format!("src/tool/z{i}.c"),
                format!("int zf{i}(int x) {{ return x + {i}; }}\n"),
            )
        })
        .collect();
    let extra: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let ids: Vec<(String, String)> = (0..3)
        .map(|i| (format!("src/tool/z{i}.c"), format!("zf{i}")))
        .collect();
    let ids: Vec<(&str, &str)> = ids.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let bounds = harness_oracle::MapBounds {
        pass_compiles: 1,
        ..DESIGN_BOUNDS
    };
    let (_tmp, map, _) = map_program_bounded(
        "feat-review-pass-first",
        "#include \"unit.h\"\n#include \"mul.h\"\nint main(void) { return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1); }\n",
        &extra,
        &ids,
        bounds,
    );
    let map = map.expect("maps");
    assert!(map.unwatched.is_empty(), "{:?}", map.unwatched_reasons);
}

/// Review: past the pass bound, only the probed files no settled compile
/// has checked go back unprobed — a header an earlier file compiled with
/// its notes keeps them.
#[test]
fn past_the_pass_bound_checked_headers_keep_their_notes() {
    let shared = "static inline int sh(int x) { return x * 2; }\n";
    let a = "#include \"shared.h\"\n\
             int bad(int x) { __label__ out; if (x) goto out; return 0; out: return 1; }\n\
             int a_use(int x) { return sh(x) + bad(x); }\n";
    let b = "#include \"shared.h\"\nint b_use(int x) { return sh(x) + 1; }\n";
    let main = "#include \"unit.h\"\n#include \"mul.h\"\nint a_use(int); int b_use(int);\n\
                int main(void) { return unit_add(1, 2) == 3 ? a_use(0) + b_use(0) - 1 : (int)mul_step(0, 1); }\n";
    let bounds = harness_oracle::MapBounds {
        pass_compiles: 1,
        ..DESIGN_BOUNDS
    };
    let (_tmp, map, _) = map_program_bounded(
        "feat-review-pass-checked",
        main,
        &[
            ("src/tool/shared.h", shared),
            ("src/tool/a.c", a),
            ("src/tool/b.c", b),
        ],
        &[
            ("src/tool/shared.h", "src/tool/shared.h::sh"),
            ("src/tool/a.c", "bad"),
            ("src/tool/a.c", "a_use"),
            ("src/tool/b.c", "b_use"),
        ],
        bounds,
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "bad"),
        Some("compile"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(kind_of(&map, "b_use"), Some("not-checked"));
    assert_eq!(
        kind_of(&map, "src/tool/shared.h::sh"),
        None,
        "{:?}",
        map.unwatched_reasons
    );
    assert!(
        ran(&map).contains(&"src/tool/shared.h::sh"),
        "{:?}",
        ran(&map)
    );
}

/// `m` pairs: g{i}'s note tips w{i} (which carries no note: its brace sits
/// under `#if`) out of being inlined, and w{i}'s `__builtin_constant_p`
/// guard then calls an `error`-attributed function — an error in a body
/// with no note, which only a search finds (the review's exp/gen6.sh; the
/// sizes are clang 21's inlining threshold).
fn tipping_pairs(m: usize) -> (String, Vec<(String, String)>) {
    let mut s = String::from(
        "__attribute__((error(\"not constant\"))) extern void bad_size(void);\n\
         volatile int sink; volatile int sink2;\n",
    );
    let mut ids = Vec::new();
    for p in 0..m {
        s.push_str(&format!("static void g{p}(int n) {{ sink = n; }}\n"));
        s.push_str(&format!(
            "static int w{p}(int n)\n#if 1\n{{\n#endif\n  g{p}(n);\n"
        ));
        for i in 0..23 {
            s.push_str(&format!("  sink = n * {i} + sink;\n"));
        }
        for _ in 0..4 {
            s.push_str("  sink2 = 1;\n");
        }
        s.push_str("  if (!__builtin_constant_p(n)) bad_size();\n  return n; }\n");
        s.push_str(&format!(
            "int f{p}(void)\n#if 1\n{{\n#endif\n  return w{p}(5); }}\n"
        ));
        s.push_str(&format!(
            "int h{p}(void)\n#if 1\n{{\n#endif\n  return w{p}(6); }}\n"
        ));
        ids.push((
            "src/tool/main.c".to_string(),
            format!("src/tool/main.c::g{p}"),
        ));
    }
    (s, ids)
}

/// Review: a search is not a placed round — two notes each found by a
/// search stay within one placed round, and main keeps its note.
#[test]
fn searches_are_not_placed_rounds() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let (body, ids) = tipping_pairs(2);
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{body}\
         int main(void) {{ return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1) + f0() + h1(); }}\n"
    );
    let ids: Vec<(&str, &str)> = ids.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let bounds = harness_oracle::MapBounds {
        placed_rounds: 1,
        ..DESIGN_BOUNDS
    };
    let (_tmp, map, _) = map_program_bounded("feat-review-search-rounds", &main, &[], &ids, bounds);
    let map = map.expect("maps");
    assert_eq!(kind_of(&map, "src/tool/main.c::g0"), Some("elimination"));
    assert_eq!(kind_of(&map, "src/tool/main.c::g1"), Some("elimination"));
    assert_eq!(kind_of(&map, "main"), None, "{:?}", map.unwatched_reasons);
}

/// Review: a search a bound stops has found nothing — the file goes back
/// unprobed ("file-limit"), no note blamed as "elimination".
#[test]
fn a_search_cut_by_the_bound_blames_no_note() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let (body, mut ids) = tipping_pairs(1);
    let mut trivial = String::new();
    for i in 0..20 {
        trivial.push_str(&format!("int t{i}(int x) {{ return x + {i}; }}\n"));
        ids.push(("src/tool/main.c".to_string(), format!("t{i}")));
    }
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{trivial}{body}\
         int main(void) {{ return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1) + f0() + h0(); }}\n"
    );
    let ids: Vec<(&str, &str)> = ids.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let bounds = harness_oracle::MapBounds {
        file_compiles: 4,
        ..DESIGN_BOUNDS
    };
    let (_tmp, map, _) = map_program_bounded("feat-review-search-cut", &main, &[], &ids, bounds);
    let map = map.expect("maps");
    assert!(
        !map.unwatched_reasons
            .iter()
            .any(|r| r.kind == "elimination"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(kind_of(&map, "src/tool/main.c::g0"), Some("file-limit"));
    assert_eq!(kind_of(&map, "t0"), Some("file-limit"));
}

/// Review: the restore pass's compiles count — a.c's one search costs
/// five compiles beyond its first (the syntax check, every note out, the
/// compile after, the note put back, and out again), so a pass bound of
/// five leaves b.c unchecked and six does not.
#[test]
fn the_restore_pass_compiles_are_counted() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let (body, _) = tipping_pairs(1);
    let a = format!("{body}int a_use(void) {{ return f0() + h0(); }}\n");
    let b = "int b_use(int x) { return x + 1; }\n";
    let main = "#include \"unit.h\"\n#include \"mul.h\"\nint a_use(void); int b_use(int);\n\
                int main(void) { return unit_add(1, 2) == 3 ? a_use() + b_use(0) - 12 : (int)mul_step(0, 1); }\n";
    for (bound, b_kind) in [(5, Some("not-checked")), (6, None)] {
        let bounds = harness_oracle::MapBounds {
            pass_compiles: bound,
            ..DESIGN_BOUNDS
        };
        let (_tmp, map, _) = map_program_bounded(
            &format!("feat-review-restore-{bound}"),
            main,
            &[("src/tool/a.c", &a), ("src/tool/b.c", b)],
            &[
                ("src/tool/a.c", "src/tool/a.c::g0"),
                ("src/tool/a.c", "a_use"),
                ("src/tool/b.c", "b_use"),
            ],
            bounds,
        );
        let map = map.expect("maps");
        assert_eq!(kind_of(&map, "src/tool/a.c::g0"), Some("elimination"));
        assert_eq!(
            kind_of(&map, "b_use"),
            b_kind,
            "bound {bound}: {:?}",
            map.unwatched_reasons
        );
    }
}

/// §4's per-file round bound. The design's fixture (`#pragma clang
/// diagnostic fatal`) is silenced by the probed compile's `-w`; here a
/// parse error in `d` hides a code-generation error in `w` (its note tips
/// it out of being inlined, so an `error`-attributed call stays), so the
/// file needs two placed rounds: one round allowed sends it back unprobed,
/// two map it.
#[test]
fn the_per_file_round_bound_unprobes_the_file() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let w = tipping_w(25, 0, false).replace(
        "extern void bad_size(void);",
        "__attribute__((error(\"not constant\"))) extern void bad_size(void);",
    );
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{w}\
         int d(int x) {{ __label__ out; if (x) goto out; return 0; out: return 1; }}\n\
         int main(void) {{ return unit_add(1, 2) == 3 ? d(0) : (int)mul_step(0, 1) + f() + f2(); }}\n"
    );
    let ids = [
        ("src/tool/main.c", "src/tool/main.c::w"),
        ("src/tool/main.c", "f"),
        ("src/tool/main.c", "f2"),
        ("src/tool/main.c", "d"),
    ];
    for (rounds, w_kind) in [(1, "file-limit"), (2, "compile")] {
        let bounds = harness_oracle::MapBounds {
            placed_rounds: rounds,
            ..DESIGN_BOUNDS
        };
        let (_tmp, map, _) = map_program_bounded(
            &format!("feat-review-round-bound-{rounds}"),
            &main,
            &[],
            &ids,
            bounds,
        );
        let map = map.expect("maps");
        assert_eq!(
            kind_of(&map, "d"),
            Some("compile"),
            "{:?}",
            map.unwatched_reasons
        );
        assert_eq!(
            kind_of(&map, "src/tool/main.c::w"),
            Some(w_kind),
            "{rounds} round(s): {:?}",
            map.unwatched_reasons
        );
        let main_kind = if rounds == 1 {
            Some("file-limit")
        } else {
            None
        };
        assert_eq!(kind_of(&map, "main"), main_kind);
    }
}

/// A static `w` whose inlining into two constant callers is tipped by a
/// note: `k` big statements and `j` small ones (the review's exp/gen3.sh);
/// with `g`, `g`'s note (inlined into `w`) tips it and `w` carries none.
fn tipping_w(k: usize, j: usize, with_g: bool) -> String {
    let mut s =
        String::from("extern void bad_size(void);\nvolatile int sink; volatile int sink2;\n");
    if with_g {
        s.push_str(
            "static void g(int n) { sink = n; }\nstatic int w(int n)\n#if 1\n{\n#endif\n  g(n);\n",
        );
    } else {
        s.push_str("static int w(int n) {\n");
    }
    for i in 0..k {
        s.push_str(&format!("  sink = n * {i} + sink;\n"));
    }
    for _ in 0..j {
        s.push_str("  sink2 = 1;\n");
    }
    s.push_str("  if (!__builtin_constant_p(n)) bad_size();\n  return n; }\n");
    s.push_str("int f(void) { return w(5); }\nint f2(void) { return w(6); }\n");
    s
}

/// Review: link rule (b) when the function the linker names carries no
/// note — the search over the named object's notes, the relink its test,
/// finds `g`'s; the callers keep theirs.
#[test]
fn link_rule_b_falls_back_to_a_search() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{}\
         int main(void) {{ return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1) + f() + f2(); }}\n",
        tipping_w(23, 4, true)
    );
    let (_tmp, map) = map_program(
        "feat-review-link-search",
        &main,
        &[],
        &[
            ("src/tool/main.c", "src/tool/main.c::g"),
            ("src/tool/main.c", "f"),
            ("src/tool/main.c", "f2"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "src/tool/main.c::g"),
        Some("link"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(kind_of(&map, "f"), None);
    assert_eq!(kind_of(&map, "f2"), None);
}

/// Review: link rule (b) takes the note of the function the linker names
/// in the object it names — a static of the same name in another file
/// keeps its own.
#[test]
fn link_rule_b_names_one_function() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{}int other(int);\n\
         int main(void) {{ return unit_add(1, 2) == 3 ? other(0) : (int)mul_step(0, 1) + f() + f2(); }}\n",
        tipping_w(25, 0, false)
    );
    let other = "static int w(int x) { return x + 1; }\nint other(int x) { return w(x) - 1; }\n";
    let (_tmp, map) = map_program(
        "feat-review-link-b-one",
        &main,
        &[("src/tool/other.c", other)],
        &[
            ("src/tool/main.c", "src/tool/main.c::w"),
            ("src/tool/main.c", "f"),
            ("src/tool/main.c", "f2"),
            ("src/tool/other.c", "src/tool/other.c::w"),
            ("src/tool/other.c", "other"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "src/tool/main.c::w"),
        Some("link"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(
        kind_of(&map, "src/tool/other.c::w"),
        None,
        "{:?}",
        map.unwatched_reasons
    );
}

/// Review and §4: the C99 `inline` link case — rule (a) takes the inline
/// function's own note; a static of the same name elsewhere keeps its; and
/// when a callee's note (inlined into it) still tips it, the search finds
/// that note, the callers keeping theirs.
#[test]
fn the_c99_inline_link_case() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("clang's inlining threshold: skipped here");
        return;
    }
    let mut w = String::from("volatile int sink; volatile int sink2;\ninline int w(int n) {\n");
    for i in 0..35 {
        w.push_str(&format!("  sink = n * {i} + sink;\n"));
    }
    w.push_str("  return n; }\nint f(void) { return w(5); }\nint f2(void) { return w(6); }\n");
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{w}int other(int);\n\
         int main(void) {{ return unit_add(1, 2) == 3 ? other(0) : (int)mul_step(0, 1) + f() + f2(); }}\n"
    );
    let other = "static int w(int x) { return x + 1; }\nint other(int x) { return w(x) - 1; }\n";
    let (_tmp, map) = map_program(
        "feat-review-c99-inline",
        &main,
        &[("src/tool/other.c", other)],
        &[
            ("src/tool/main.c", "w"),
            ("src/tool/main.c", "f"),
            ("src/tool/main.c", "f2"),
            ("src/tool/other.c", "src/tool/other.c::w"),
            ("src/tool/other.c", "other"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "w"),
        Some("link"),
        "{:?}",
        map.unwatched_reasons
    );
    // The detail is the name; the words say the rest once.
    assert!(
        map.unwatched_reasons
            .iter()
            .any(|r| r.id == "w" && r.detail == "w is undefined"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(
        kind_of(&map, "src/tool/other.c::w"),
        None,
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(kind_of(&map, "f"), None);

    let mut w = String::from(
        "volatile int sink; volatile int sink2;\nstatic void g(int n) { sink = n; }\ninline int w(int n) {\n  g(n);\n",
    );
    for i in 0..33 {
        w.push_str(&format!("  sink = n * {i} + sink;\n"));
    }
    for _ in 0..4 {
        w.push_str("  sink2 = 1;\n");
    }
    w.push_str("  return n; }\nint f(void) { return w(5); }\nint f2(void) { return w(6); }\n");
    let main = format!(
        "#include \"unit.h\"\n#include \"mul.h\"\n{w}\
         int main(void) {{ return unit_add(1, 2) == 3 ? 0 : (int)mul_step(0, 1) + f() + f2(); }}\n"
    );
    let (_tmp, map) = map_program(
        "feat-review-c99-callee",
        &main,
        &[],
        &[
            ("src/tool/main.c", "src/tool/main.c::g"),
            ("src/tool/main.c", "w"),
            ("src/tool/main.c", "f"),
            ("src/tool/main.c", "f2"),
        ],
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "w"),
        Some("link"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(kind_of(&map, "src/tool/main.c::g"), Some("link"));
    assert_eq!(kind_of(&map, "f"), None);
    assert_eq!(kind_of(&map, "f2"), None);
}

/// Review: a program's own static `close` is never what the runtime's
/// import binds to; a program's `environ` is refused in words for a
/// variable.
#[test]
fn the_runtimes_names_are_external_definitions() {
    let main = "#include \"unit.h\"\n#include \"mul.h\"\n\
                __attribute__((noinline)) static int close(int fd) { return fd + 1; }\n\
                int main(int argc, char **argv) { (void)argv; return unit_add(1, 2) == 3 ? close(-argc) : (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program(
        "feat-review-static-close",
        main,
        &[],
        &[("src/tool/main.c", "src/tool/main.c::close")],
    );
    let map = map.expect("a static close maps");
    assert!(
        ran(&map).contains(&"src/tool/main.c::close"),
        "{:?}",
        ran(&map)
    );

    let main = "#include \"unit.h\"\n#include \"mul.h\"\nchar **environ = 0;\n\
                int main(void) { return unit_add(1, 2) == 3 ? (environ != 0) : (int)mul_step(0, 1); }\n";
    let (_tmp, map) = map_program("feat-review-environ", main, &[], &[]);
    let err = map.expect_err("refused").to_string();
    assert!(err.contains("the program defines environ,"), "{err}");
}

/// §3.4 step 7, and review: a header's note rejected only in a later file
/// makes the earlier file that entered it compile again — and the progress
/// says "again" instead of restarting at round 1 unexplained.
#[test]
fn a_header_rejected_in_a_later_file_compiles_the_earlier_one_again() {
    let h = "static inline int hf(int x) { HF_BODY }\n";
    let a =
        "#define HF_BODY return x + 1;\n#include \"hf.h\"\nint a_use(int x) { return hf(x); }\n";
    let b = "#define HF_BODY __label__ out; if (x) goto out; return 0; out: return 1;\n\
             #include \"hf.h\"\nint b_use(int x) { return hf(x); }\n";
    let main = "#include \"unit.h\"\n#include \"mul.h\"\nint a_use(int); int b_use(int);\n\
                int main(void) { return unit_add(1, 2) == 3 ? a_use(0) - 1 + b_use(0) : (int)mul_step(0, 1); }\n";
    let (_tmp, map, messages) = map_program_bounded(
        "feat-review-again",
        main,
        &[
            ("src/tool/hf.h", h),
            ("src/tool/a.c", a),
            ("src/tool/b.c", b),
        ],
        &[
            ("src/tool/hf.h", "src/tool/hf.h::hf"),
            ("src/tool/a.c", "a_use"),
            ("src/tool/b.c", "b_use"),
        ],
        DESIGN_BOUNDS,
    );
    let map = map.expect("maps");
    assert_eq!(
        kind_of(&map, "src/tool/hf.h::hf"),
        Some("compile"),
        "{:?}",
        map.unwatched_reasons
    );
    assert_eq!(map.scenarios[0].noted, "complete");
    assert!(
        messages
            .iter()
            .any(|m| m.ends_with("src/tool/a.c (again, round 1)")),
        "{messages:?}"
    );
}

/// Review (token comparison): a raw string spanning lines holds text the
/// parser reads as a function; its note lands inside the string. The
/// whole text is one literal to the tokenizer, so the note is seen there —
/// the function is unwatched and the map records what ran.
#[test]
fn a_function_shaped_line_in_a_raw_string_never_changes_the_program() {
    let main = "#include <stdio.h>\n#include <string.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
                static const char *tmpl = R\"(\nvoid g(void);\nint f(void) { return 1;}\n)\";\n\
                void longer(void) { puts(\"x\"); }\nvoid shorter(void) { puts(\"x\"); }\n\
                int main(void) { if (unit_add(1, 2) != 3) return (int)mul_step(0, 1);\n\
                if (strlen(tmpl) > 50) longer(); else shorter(); return 0; }\n";
    let (_tmp, map) = map_program(
        "feat-review-raw-template",
        main,
        &[],
        &[
            ("src/tool/main.c", "f"),
            ("src/tool/main.c", "longer"),
            ("src/tool/main.c", "shorter"),
        ],
    );
    let map = map.expect("maps");
    let ran = ran(&map);
    assert!(
        ran.contains(&"shorter") && !ran.contains(&"longer"),
        "{ran:?}"
    );
    assert!(kind_of(&map, "f").is_some(), "{:?}", map.unwatched_reasons);
}

/// Review (token comparison): an odd quote inside a raw string on a body's
/// line left the end token in (the map refused an ordinary program), or
/// took a later note for a stringized one.
#[test]
fn an_odd_quote_in_a_raw_string_maps() {
    let main = "#include <stdio.h>\n#include \"unit.h\"\n#include \"mul.h\"\n\
                static const char *quote(void) { return R\"(\")\"; }\n\
                static const char *q = R\"(\")\"; static int f(void) { return q[0]; }\n\
                int main(void) { if (unit_add(1, 2) != 3) return (int)mul_step(0, 1);\n\
                printf(\"%s %d\\n\", quote(), f()); return 0; }\n";
    let (_tmp, map) = map_program(
        "feat-review-raw-quote",
        main,
        &[],
        &[
            ("src/tool/main.c", "src/tool/main.c::quote"),
            ("src/tool/main.c", "src/tool/main.c::f"),
        ],
    );
    let map = map.expect("maps");
    assert!(map.unwatched.is_empty(), "{:?}", map.unwatched_reasons);
    let ran = ran(&map);
    assert!(
        ran.contains(&"src/tool/main.c::quote") && ran.contains(&"src/tool/main.c::f"),
        "{ran:?}"
    );
}

/// §4 (§3.6), review: a constructor that forks before the runtime's setup —
/// both images run setup, map the one notes file and merge; neither wipes
/// the other's notes.
#[test]
fn a_constructor_that_forks_before_setup_keeps_both_images_notes() {
    if cfg!(not(target_os = "macos")) {
        eprintln!("constructor order by link order: checked on Mach-O");
        return;
    }
    let tmp = TempDir::new("fnprobe-fork-early");
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.c");
    let main = tmp.path().join("main.c");
    write(
        &main,
        "#include <sys/wait.h>\n#include <unistd.h>\n\
         extern unsigned char *volatile __ruharness_seen;\n\
         static pid_t kid = -1;\n\
         __attribute__((constructor)) static void early(void) { kid = fork(); }\n\
         int main(void) {\n\
           if (kid == 0) { __ruharness_seen[1] = 1; return 0; }\n\
           __ruharness_seen[0] = 1;\n\
           int st; while (waitpid(kid, &st, 0) < 0) {}\n\
           return 0;\n\
         }\n",
    );
    let bin = tmp.path().join("probed");
    // The program's object first: its constructor runs before setup.
    let built = std::process::Command::new("cc")
        .args(["-O2", "-DRUHARNESS_FNPROBE_N=4", "-include"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.h"))
        .arg("-o")
        .arg(&bin)
        .arg(&main)
        .arg(&runtime)
        .status()
        .expect("cc runs");
    assert!(built.success());
    let dir = tmp.path().join("t");
    std::fs::create_dir_all(&dir).unwrap();
    let notes = dir.join(".ruharness-notes");
    std::fs::write(&notes, [0u8; 5]).unwrap();
    let ran = std::process::Command::new(&bin)
        .env_clear()
        .env("TMPDIR", &dir)
        .status()
        .expect("runs");
    assert!(ran.success());
    assert_eq!(std::fs::read(&notes).unwrap(), [1, 1, 0, 0, 1]);
}

/// §4 (§3.6), review: setup points the notes at the mapping before it
/// merges the early array — a note a thread makes during setup lands in
/// the mapping or in the array the merge then reads. A runtime that merged
/// first and switched after would drop a note made between the two; a race
/// test of it is flaky either way, so the order itself is pinned here.
#[test]
fn setup_switches_to_the_mapping_before_it_merges() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fnprobe/fnprobe.c"),
    )
    .unwrap();
    let switch = source
        .find("__ruharness_seen = m;")
        .expect("setup points the notes at the mapping");
    let merge = source
        .find("if (ruharness_early[i])")
        .expect("setup merges the early notes");
    assert!(switch < merge, "the merge runs before the switch");
}
