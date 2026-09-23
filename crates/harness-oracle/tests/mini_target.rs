//! End-to-end `verify` runs against a tiny synthetic target, exercising the
//! M3/M4 trust boundaries through the public API only: the symbol-set check
//! (green, extra export, libc shadowing), the capabilities and driver-shape
//! gates, run confinement, the opt-in whole-program check, include dirs,
//! timeouts of built binaries as failed checks, a missing crate as a red
//! build, and config refusal.

mod common;

use common::{write, TempDir};
use harness_core::facts::FileRecord;
use harness_core::traits::OracleStrategy;
use harness_core::{Facts, TargetContext, Unit, Verdict};
use harness_oracle::{sandbox_mode, CAbiDifferential};
use std::path::Path;

const GOOD_LIB: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n";

/// Lay out a complete mini target (see [`mini`]) with the whole-program
/// check configured as `-c <sample>`.
fn mini_target(root: &Path, oracle_extra: &str, lib_rs: &str) -> (TargetContext, Unit) {
    mini(root, "", oracle_extra, true, lib_rs)
}

/// Lay out a complete mini target: one replaceable C unit, a whole program
/// honouring the `-c <file>` convention, a driver, facts, and a unit crate
/// whose `src/lib.rs` is `lib_rs`. `target_extra` lands in `[target]`,
/// `oracle_extra` in `[oracle]`; `whole_program` adds
/// `[oracle.whole_program] args = ["-c"]`.
fn mini(
    root: &Path,
    target_extra: &str,
    oracle_extra: &str,
    whole_program: bool,
    lib_rs: &str,
) -> (TargetContext, Unit) {
    let wp = if whole_program {
        "\n[oracle.whole_program]\nargs = [\"-c\"]\n"
    } else {
        ""
    };
    write(
        &root.join("harness.toml"),
        &format!(
            "schema_version = 1\n\n[target]\nname = \"mini\"\nsource_dir = \"src/mini\"\n\
             {target_extra}\n\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n{oracle_extra}\n{wp}"
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
    let names: Vec<&str> = verdict.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "symbol-set",
            "capabilities",
            "driver-shape",
            "differential-driver",
            "whole-program:sample_text.txt",
            "whole-program:sample_rand.bin",
            "whole-program:sample_empty",
            "sanitizers",
        ]
    );
    let toolchain = &verdict.inputs.toolchain;
    assert_eq!(
        toolchain[toolchain.len() - 3],
        format!("sandbox: {}", sandbox_mode())
    );
    assert_eq!(toolchain[toolchain.len() - 2], "cflags: -ffp-contract=off");
    assert_eq!(
        toolchain.last().map(String::as_str),
        Some("observable: stdout+stderr")
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

/// A driver reporting part of the unit's behavior on stderr (M4:
/// `014_pow_subfunction` does): the differential oracle compares both
/// streams, so a candidate that is right on every stdout observation but
/// wrong on the stderr one is red. Under the stdout-only compare it was green.
#[test]
fn a_candidate_wrong_only_on_stderr_is_red() {
    const STDERR_DRIVER: &str = "#include <stdio.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           for (int i = 0; i < 100; i++) printf(\"%d\\n\", unit_add(i, i));\n\
           fprintf(stderr, \"edge %d\\n\", unit_add(-77777, 1));\n\
           return 0;\n\
         }\n";
    const WRONG_ON_EDGE: &str = "#[no_mangle]\n\
pub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
if a == -77777 { 0 } else { a.wrapping_add(b) }\n}\n";

    let tmp = TempDir::new("mini-stderr-red");
    let (target, unit) = mini(tmp.path(), "", "", false, WRONG_ON_EDGE);
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        STDERR_DRIVER,
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green, "{}", describe(&verdict));
    let diff = verdict
        .checks
        .iter()
        .find(|c| c.name == "differential-driver")
        .expect("differential check ran");
    assert!(!diff.passed, "{}", describe(&verdict));
    assert!(
        diff.detail.starts_with("stdout identical (")
            && diff
                .detail
                .contains("; stderr differs (lens 12 vs 7, first diff at byte 5)"),
        "{}",
        diff.detail
    );
    let build = tmp.path().join("migration/build/u-mini");
    assert_eq!(
        std::fs::read(build.join("drv_c.err")).unwrap(),
        b"edge -77776\n"
    );
    assert_eq!(
        std::fs::read(build.join("drv_rs.err")).unwrap(),
        b"edge 0\n"
    );

    // The faithful candidate is green, and the detail says stderr was compared.
    let tmp = TempDir::new("mini-stderr-green");
    let (target, unit) = mini(tmp.path(), "", "", false, GOOD_LIB);
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        STDERR_DRIVER,
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    let diff = verdict
        .checks
        .iter()
        .find(|c| c.name == "differential-driver")
        .expect("differential check ran");
    assert!(
        diff.detail
            .ends_with(" bytes identical (stderr: 12 bytes identical)"),
        "{}",
        diff.detail
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
    assert!(
        verdict
            .inputs
            .toolchain
            .contains(&format!("sandbox: {}", sandbox_mode())),
        "{:?}",
        verdict.inputs.toolchain
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

/// A driver that shells out (`system`) or opens files is refused by the
/// `driver-shape` gate (R1) before anything is linked: `system`/`fopen` are
/// not on the driver libc allowlist. (That a built binary could not exec or
/// read the target root anyway is proven by the run-confinement tests.)
#[test]
fn a_driver_reaching_for_the_os_fails_driver_shape_and_nothing_is_linked() {
    let tmp = TempDir::new("mini-shape");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        "#include <stdio.h>\n#include <stdlib.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           FILE* f = fopen(\"migration/build/u-mini/drv_c.out\", \"rb\");\n\
           int rc = system(\"/bin/echo forged\");\n\
           printf(\"%d %d\\n\", f != 0, rc);\n\
           printf(\"%d\\n\", unit_add(2, 3));\n\
           return 0;\n\
         }\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let names: Vec<&str> = verdict.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["symbol-set", "capabilities", "driver-shape"]);
    let detail = &verdict.checks[2].detail;
    assert!(!verdict.checks[2].passed);
    assert!(detail.contains("fopen"), "{detail}");
    assert!(detail.contains("system"), "{detail}");
    assert!(!detail.contains("/Users/"), "{detail}");
    let build = tmp.path().join("migration/build/u-mini");
    assert!(!build.join("drv_c").exists(), "nothing may be linked");
    assert!(!build.join("drv_rs").exists(), "nothing may be linked");
}

/// A driver that takes a unit symbol's address (the lint half of the gate).
#[test]
fn a_driver_taking_a_unit_symbols_address_fails_driver_shape() {
    let tmp = TempDir::new("mini-addr");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    write(
        &tmp.path().join("migration/units/u-mini/driver.c"),
        "#include <stdio.h>\n#include \"unit.h\"\n\
         int main(void) {\n\
           int (*volatile fp)(int, int) = unit_add;\n\
           printf(\"%d\\n\", fp(2, 3));\n\
           return 0;\n\
         }\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let shape = verdict
        .checks
        .iter()
        .find(|c| c.name == "driver-shape")
        .expect("driver-shape ran");
    assert!(
        shape
            .detail
            .contains("unit symbol `unit_add` may only be called"),
        "{}",
        shape.detail
    );
}

/// R2: a candidate whose own code reads a file (here: the C side's pinned
/// output) is red at `capabilities` — the C unit does no I/O — and is never
/// linked or run.
#[test]
fn a_candidate_reading_files_fails_capabilities_and_is_never_run() {
    let tmp = TempDir::new("mini-caps");
    let lib = "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
               let n = std::fs::read(\"migration/build/u-mini/drv_c.out\").map(|v| v.len()).unwrap_or(0);\n    \
               a.wrapping_add(b).wrapping_add(n as i32 & 0)\n}\n";
    let (target, unit) = mini_target(tmp.path(), "", lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let names: Vec<&str> = verdict.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["symbol-set", "capabilities"],
        "{}",
        describe(&verdict)
    );
    assert!(verdict.checks[0].passed);
    let detail = &verdict.checks[1].detail;
    assert!(detail.contains("_ZN3std2fs"), "{detail}");
    assert!(detail.contains("(fs)"), "{detail}");
    assert!(!tmp.path().join("migration/build/u-mini/drv_rs").exists());
}

/// R1 run confinement, end to end: the whole program (target-owned C, never
/// linted) tries to read the C side's pinned `drv_c.out` and the harness
/// config inside the target root, to read the home directory, to write
/// outside its temp dir and to exec — and exits non-zero if ANY of it works.
/// The verdict is green only because every attempt was denied, while the
/// program still read its one listed input (the sample) and wrote its TMPDIR.
#[test]
fn built_binaries_are_confined_to_their_inputs_and_tmpdir() {
    if sandbox_mode() != "sandbox-exec" {
        return;
    }
    let tmp = TempDir::new("mini-confined");
    let (target, unit) = mini_target(tmp.path(), "", GOOD_LIB);
    let probe = tmp.path().join("migration/build/stray-write-probe");
    let home = std::env::var("HOME").expect("HOME set");
    write(
        &tmp.path().join("src/mini/main.c"),
        &format!(
            "#include <stdio.h>\n#include <stdlib.h>\n#include <dirent.h>\n#include <unistd.h>\n\
             #include \"unit.h\"\n\
             int main(int argc, char** argv) {{\n\
               int acc = 0, c;\n\
               if (fopen(\"migration/build/u-mini/drv_c.out\", \"rb\")) return 7;\n\
               if (fopen(\"harness.toml\", \"rb\")) return 8;\n\
               if (opendir(\"{home}\")) return 9;\n\
               if (fopen(\"{probe}\", \"wb\")) return 10;\n\
               if (system(\"/usr/bin/true\") == 0) return 11;\n\
               const char* t = getenv(\"TMPDIR\");\n\
               char path[4096];\n\
               snprintf(path, sizeof path, \"%s/scratch\", t ? t : \"/nonexistent\");\n\
               FILE* s = fopen(path, \"wb\");\n\
               if (!s) return 12;\n\
               fclose(s);\n\
               FILE* f = argc > 2 ? fopen(argv[2], \"rb\") : NULL;\n\
               if (!f) return 2;\n\
               while ((c = fgetc(f)) != EOF) acc = unit_add(acc, c);\n\
               fclose(f);\n\
               printf(\"%d\\n\", acc);\n\
               return 0;\n\
             }}\n",
            probe = probe.display()
        ),
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!probe.exists(), "a built binary wrote outside its TMPDIR");
    assert!(verdict.green, "{}", describe(&verdict));
    // The C side's output the candidate would have liked to replay exists —
    // it just was not readable.
    assert!(tmp.path().join("migration/build/u-mini/drv_c.out").exists());
}

/// R8: without `[oracle.whole_program]` the verdict carries exactly one,
/// passed, explicit `whole-program` check, and no whole program is built.
#[test]
fn the_whole_program_check_is_opt_in() {
    let tmp = TempDir::new("mini-nowp");
    let (target, unit) = mini(tmp.path(), "", "", false, GOOD_LIB);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    let wp: Vec<_> = verdict
        .checks
        .iter()
        .filter(|c| c.name.starts_with("whole-program"))
        .collect();
    assert_eq!(wp.len(), 1, "{}", describe(&verdict));
    assert_eq!(wp[0].name, "whole-program");
    assert!(wp[0].passed);
    assert_eq!(wp[0].detail, "not configured for this target");
    assert!(!tmp.path().join("migration/build/u-mini/whole_c").exists());
}

/// R8: `args` holds flags only (≤ 4); anything else is refused before any
/// subprocess runs.
#[test]
fn hostile_whole_program_args_are_refused_before_anything_runs() {
    for (args, offender) in [
        ("[\"-c\", \"/etc/passwd\"]", "/etc/passwd"),
        ("[\"--out=/tmp/x\"]", "--out=/tmp/x"),
        ("[\"-c\", \"-v\", \"-q\", \"-x\", \"-y\"]", "at most 4"),
        ("[\"-\"]", "\"-\""),
        ("[\"---x\"]", "---x"),
        ("\"-c\"", "array"),
    ] {
        let tmp = TempDir::new("mini-wphostile");
        let (target, unit) = mini(
            tmp.path(),
            "",
            &format!("\n[oracle.whole_program]\nargs = {args}\n"),
            false,
            GOOD_LIB,
        );
        let err = CAbiDifferential
            .verify(&target, &unit)
            .expect_err("hostile whole_program args");
        assert!(
            matches!(err, harness_core::Error::InvalidPlan(_)),
            "{args}: {err:?}"
        );
        assert!(err.to_string().contains(offender), "{args}: {err}");
        assert!(!tmp.path().join("migration/build").exists(), "{args}");
    }
}

/// `[target] include_dirs` reach every compile as `-I` after the source dir:
/// the unit header includes a type header that lives only in the include
/// dir. Without the config the driver does not even compile on its own (a
/// red driver-shape); with it the verdict is green.
#[test]
fn include_dirs_reach_every_compile() {
    let layout = |root: &Path| {
        write(
            &root.join("src/mini/unit.h"),
            "#include \"types.h\"\nunit_int unit_add(unit_int a, unit_int b);\n",
        );
        write(
            &root.join("src/mini/include/types.h"),
            "typedef int unit_int;\n",
        );
        let facts = Facts {
            frontend: "test-inline".to_string(),
            files: vec![
                FileRecord {
                    path: "src/mini/include/types.h".to_string(),
                    hash: String::new(),
                    includes: Vec::new(),
                },
                FileRecord {
                    path: "src/mini/unit.c".to_string(),
                    hash: String::new(),
                    includes: vec!["src/mini/unit.h".to_string()],
                },
                FileRecord {
                    path: "src/mini/unit.h".to_string(),
                    hash: String::new(),
                    includes: vec!["src/mini/include/types.h".to_string()],
                },
            ],
            ..Facts::default()
        };
        facts
            .store(&root.join("migration/facts.jsonl"))
            .expect("facts stored");
    };

    let without = TempDir::new("mini-noinc");
    let (target, unit) = mini(without.path(), "", "", true, GOOD_LIB);
    layout(without.path());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let shape = verdict.checks.last().expect("a check");
    assert_eq!(shape.name, "driver-shape", "{}", describe(&verdict));
    assert!(
        shape.detail.contains("does not compile on its own"),
        "{}",
        shape.detail
    );
    assert!(shape.detail.contains("types.h"), "{}", shape.detail);

    let with = TempDir::new("mini-inc");
    let (target, unit) = mini(
        with.path(),
        "include_dirs = [\"src/mini/include\"]",
        "",
        true,
        GOOD_LIB,
    );
    layout(with.path());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
}
