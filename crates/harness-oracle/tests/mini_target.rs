//! End-to-end `verify` runs against a tiny synthetic target, exercising the
//! M3 trust boundaries through the public API only: the symbol-set check
//! (green, extra export, libc shadowing), timeouts of built binaries as
//! failed checks, a missing crate as a red build, and config refusal.

mod common;

use common::{write, TempDir};
use harness_core::facts::FileRecord;
use harness_core::traits::OracleStrategy;
use harness_core::{Facts, TargetContext, Unit, Verdict};
use harness_oracle::{sandbox_mode, CAbiDifferential};
use std::path::Path;

const GOOD_LIB: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n";

/// Lay out a complete mini target: one replaceable C unit, a whole program
/// honouring the `-c <file>` convention, a driver, facts, and a unit crate
/// whose `src/lib.rs` is `lib_rs`.
fn mini_target(root: &Path, oracle_extra: &str, lib_rs: &str) -> (TargetContext, Unit) {
    write(
        &root.join("harness.toml"),
        &format!(
            "schema_version = 1\n\n[target]\nname = \"mini\"\nsource_dir = \"src/mini\"\n\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n{oracle_extra}\n"
        ),
    );
    write(
        &root.join("src/mini/unit.h"),
        "int unit_add(int a, int b);\n",
    );
    write(
        &root.join("src/mini/unit.c"),
        "#include \"unit.h\"\nint unit_add(int a, int b) { return (int)((unsigned)a + (unsigned)b); }\n",
    );
    write(
        &root.join("src/mini/main.c"),
        "#include <stdio.h>\n#include \"unit.h\"\n\
         int main(int argc, char** argv) {\n\
           int acc = 0, c;\n\
           FILE* f = argc > 2 ? fopen(argv[2], \"rb\") : NULL;\n\
           if (!f) return 2;\n\
           while ((c = fgetc(f)) != EOF) acc = unit_add(acc, c);\n\
           fclose(f);\n\
           printf(\"%d\\n\", acc);\n\
           return 0;\n\
         }\n",
    );
    write(
        &root.join("migration/units/u-mini/driver.c"),
        "#include <stdio.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           for (int i = 0; i < 1000; i++) printf(\"%d\\n\", unit_add(i * 7919, i - 500));\n\
           printf(\"%d\\n\", unit_add(-77777, 1));\n\
           return 0;\n\
         }\n",
    );
    let crate_dir = root.join("migration/units/u-mini/mini_rs");
    write(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"mini_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\n",
    );
    write(&crate_dir.join("src/lib.rs"), lib_rs);

    let facts = Facts {
        frontend: "test-inline".to_string(),
        files: vec![
            FileRecord {
                path: "src/mini/unit.c".to_string(),
                hash: String::new(),
                includes: vec!["src/mini/unit.h".to_string()],
            },
            FileRecord {
                path: "src/mini/unit.h".to_string(),
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
        "id = \"u-mini\"\nstatus = \"pending\"\nfiles = [\"src/mini/unit.c\"]\n\
         symbols = [\"unit_add\"]\n\n[oracle]\nkind = \"c-abi-differential\"\n\
         driver = \"migration/units/u-mini/driver.c\"\nrust_crate = \"mini_rs\"\n\
         replaces = [\"src/mini/unit.c\"]\n",
    )
    .expect("unit parses");
    (TargetContext::load(root).expect("target loads"), unit)
}

fn describe(verdict: &Verdict) -> String {
    verdict
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_faithful_candidate_is_green_and_records_the_sandbox_mode() {
    let tmp = TempDir::new("mini-green");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    assert_eq!(verdict.checks[0].name, "symbol-set");
    assert_eq!(verdict.checks.len(), 6, "{}", describe(&verdict));
    assert_eq!(
        verdict.inputs.toolchain.last().map(String::as_str),
        Some(format!("sandbox: {}", sandbox_mode()).as_str())
    );
    // The candidate sets panic = "abort", so the abort baseline was used.
    assert!(tmp
        .path()
        .join("migration/build/symbol-baseline/abort/symbols.txt")
        .exists());

    // Content-bound and repeatable: a second run yields the same verdict.
    let again = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert_eq!(
        serde_json_like(&verdict),
        serde_json_like(&again),
        "verdicts must be deterministic"
    );
}

/// Field-by-field rendering (the test crate has no serde_json dependency).
fn serde_json_like(v: &Verdict) -> String {
    format!(
        "{} {} {:?} {:?} {:?} {:?} {:?} {}",
        v.unit,
        v.green,
        v.inputs.unit_source,
        v.inputs.driver,
        v.inputs.rust_crate,
        v.inputs.replaces,
        v.inputs.toolchain,
        describe(v)
    )
}

#[test]
fn an_extra_export_is_red_and_nothing_of_the_candidate_runs() {
    let tmp = TempDir::new("mini-extra");
    let lib = format!(
        "{GOOD_LIB}\n#[no_mangle]\npub extern \"C\" fn helper_the_plan_never_heard_of() -> i32 {{\n    7\n}}\n"
    );
    let (target, unit) = mini_target(tmp.path(), "", &lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    assert_eq!(verdict.checks.len(), 1, "{}", describe(&verdict));
    let check = &verdict.checks[0];
    assert_eq!(check.name, "symbol-set");
    assert!(
        check
            .detail
            .contains("unexpected: helper_the_plan_never_heard_of"),
        "{}",
        check.detail
    );
    // Short-circuit: the candidate was never linked, let alone run.
    assert!(!tmp.path().join("migration/build/u-mini/drv_rs").exists());
}

/// The attack the check exists for: a candidate that shadows `printf` so the
/// Rust-linked driver prints whatever the attacker likes. Behaviourally it
/// would be caught or not depending on the forgery; structurally it can never
/// go green.
#[test]
fn a_candidate_shadowing_libc_cannot_go_green() {
    let tmp = TempDir::new("mini-shadow");
    let lib = format!(
        "{GOOD_LIB}\n#[export_name = \"printf\"]\npub extern \"C\" fn forged(_fmt: *const u8) -> i32 {{\n    0\n}}\n"
    );
    let (target, unit) = mini_target(tmp.path(), "", &lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    assert_eq!(verdict.checks[0].name, "symbol-set");
    assert!(
        verdict.checks[0].detail.contains("unexpected: printf"),
        "{}",
        verdict.checks[0].detail
    );
}

#[test]
fn a_candidate_missing_the_units_symbol_is_red_not_a_link_error() {
    let tmp = TempDir::new("mini-missing");
    let lib = "#[no_mangle]\npub extern \"C\" fn unit_ad(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n";
    let (target, unit) = mini_target(tmp.path(), "", lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let detail = &verdict.checks[0].detail;
    assert!(detail.contains("unexpected: unit_ad"), "{detail}");
    assert!(detail.contains("missing: unit_add"), "{detail}");
}

/// A candidate that hangs on one driver input: the C side finishes, the
/// candidate side is killed at the deadline, and the verdict says so. (The
/// trigger is negative, which the whole program's running byte sum never is,
/// so only the driver run hangs.)
#[test]
fn a_hanging_candidate_is_a_failed_check_not_a_hung_harness() {
    let tmp = TempDir::new("mini-hang");
    let lib = "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
               while a == -77777 {\n        std::hint::spin_loop();\n    }\n    a.wrapping_add(b)\n}\n";
    let (target, unit) = mini_target(tmp.path(), "timeout_secs = 10", lib);
    let started = std::time::Instant::now();
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let diff = verdict
        .checks
        .iter()
        .find(|c| c.name == "differential-driver")
        .expect("differential check present");
    assert!(!diff.passed);
    assert_eq!(diff.detail, "candidate run failed: timed out after 10s");
    // Only the driver hits the hanging input; the rest still ran and passed.
    assert!(
        verdict
            .checks
            .iter()
            .filter(|c| c.name != "differential-driver")
            .all(|c| c.passed),
        "{}",
        describe(&verdict)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
}

#[test]
fn a_candidate_that_does_not_compile_is_a_red_rust_build() {
    let tmp = TempDir::new("mini-nobuild");
    let (target, unit) = mini_target(tmp.path(), "", "pub fn broken( -> i32 { 1 }\n");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    assert_eq!(verdict.checks.len(), 1);
    assert_eq!(verdict.checks[0].name, "rust-build");
    assert_eq!(
        verdict.inputs.toolchain.last().map(String::as_str),
        Some(format!("sandbox: {}", sandbox_mode()).as_str())
    );
}

/// A failing build's detail is machine-independent: the absolute paths cargo
/// and rustc print (manifest path, target dir, the target root) are scrubbed
/// to placeholders, while the rustc diagnostic and the relative `src/` path it
/// points at survive — so the committed red verdict is reproducible.
#[test]
fn a_failing_build_detail_is_machine_independent() {
    let tmp = TempDir::new("mini-scrub");
    let (target, unit) = mini_target(tmp.path(), "", "pub fn broken( -> i32 { 1 }\n");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let detail = &verdict.checks[0].detail;
    assert_eq!(verdict.checks[0].name, "rust-build");
    // No machine paths leak into committed evidence.
    assert!(!detail.contains("/Users/"), "{detail}");
    assert!(!detail.contains("/home/"), "{detail}");
    // The target root is folded to its placeholder …
    assert!(detail.contains("<target>"), "{detail}");
    // … while the actual rustc diagnostic and its relative source path remain.
    assert!(detail.contains("src/lib.rs"), "{detail}");
    assert!(detail.contains("error"), "{detail}");
}

#[test]
fn a_unit_without_a_crate_yet_is_a_red_rust_build() {
    let tmp = TempDir::new("mini-nocrate");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    std::fs::remove_dir_all(tmp.path().join("migration/units/u-mini/mini_rs")).expect("rm crate");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    assert_eq!(verdict.checks[0].name, "rust-build");
    assert!(
        verdict.checks[0].detail.contains("does not exist"),
        "{}",
        verdict.checks[0].detail
    );
}

#[test]
fn hostile_config_is_refused_before_anything_is_built() {
    let tmp = TempDir::new("mini-hostile");
    let (target, unit) = mini_target(
        tmp.path(),
        "extra_link_args = [\"-lm\", \"-Wl,-rpath,/tmp/evil\"]",
        GOOD_LIB,
    );
    let err = CAbiDifferential
        .verify(&target, &unit)
        .expect_err("hostile link arg");
    assert!(
        matches!(err, harness_core::Error::InvalidPlan(_)),
        "{err:?}"
    );
    assert!(err.to_string().contains("-Wl,-rpath,/tmp/evil"), "{err}");
    assert!(!tmp
        .path()
        .join("migration/units/u-mini/mini_rs/target")
        .exists());
    assert!(!tmp.path().join("migration/build").exists());
}

/// Under the sandbox, a built binary may `exec` nothing but itself: a driver
/// that shells out is denied on both sides, so the run stays green (the denial
/// is identical) while proving the process-exec restriction is in force.
#[test]
fn built_binaries_cannot_exec_other_programs() {
    if sandbox_mode() != "sandbox-exec" {
        return;
    }
    let tmp = TempDir::new("mini-noexec");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        "#include <stdio.h>\n#include <stdlib.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           int rc = system(\"/bin/echo forged > /dev/null 2>&1\");\n\
           printf(\"exec: %s\\n\", rc == 0 ? \"ALLOWED\" : \"denied\");\n\
           printf(\"%d\\n\", unit_add(2, 3));\n\
           return 0;\n\
         }\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    let out = std::fs::read_to_string(tmp.path().join("migration/build/u-mini/drv_rs.out"))
        .expect("driver output kept");
    assert_eq!(out, "exec: denied\n5\n", "a built binary was able to exec");
}

/// Under the sandbox, candidate code cannot write outside temp — not even
/// into the unit's own build dir — and cannot read the user's home.
#[test]
fn candidate_code_is_confined_by_the_sandbox() {
    if sandbox_mode() != "sandbox-exec" {
        return;
    }
    let tmp = TempDir::new("mini-confined");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    // Outside every temp dir: the workspace's own build output directory.
    let exe_dir = std::env::current_exe()
        .expect("test exe")
        .parent()
        .expect("exe dir")
        .canonicalize()
        .expect("exe dir resolves");
    if exe_dir.starts_with("/private/tmp") || exe_dir.starts_with("/private/var/folders") {
        return;
    }
    let probe = exe_dir.join(format!("ruharness-confined-probe-{}", std::process::id()));
    let home = std::env::var("HOME").expect("HOME set");
    // A driver that tries to escape, and reports what happened on stdout —
    // identically on both sides, so the differential itself stays green.
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        &format!(
            "#include <stdio.h>\n#include <dirent.h>\n#include \"unit.h\"\n\
             int main(void) {{\n\
               FILE* f = fopen(\"{}\", \"w\");\n\
               printf(\"stray write: %s\\n\", f ? \"ALLOWED\" : \"denied\");\n\
               if (f) fclose(f);\n\
               DIR* d = opendir(\"{home}\");\n\
               printf(\"home read: %s\\n\", d ? \"ALLOWED\" : \"denied\");\n\
               if (d) closedir(d);\n\
               printf(\"%d\\n\", unit_add(2, 3));\n\
               return 0;\n\
             }}\n",
            probe.display()
        ),
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let created = probe.exists();
    let _ = std::fs::remove_file(&probe);
    assert!(verdict.green, "{}", describe(&verdict));
    assert!(!created, "a built binary wrote outside temp");
    let out = std::fs::read_to_string(tmp.path().join("migration/build/u-mini/drv_rs.out"))
        .expect("driver output kept");
    assert_eq!(out, "stray write: denied\nhome read: denied\n5\n");
}
