//! A piece that calls another (docs/BEYOND-LEAVES-DESIGN.md §7), through
//! the whole judge: the caller's test program validates with the callee's
//! C linked and only the caller's C mutated; the caller's Rust with an
//! `extern` block for the callee goes green; a Rust that names the callee
//! wrongly is a red `rust-build`, never an error; a plan that does not know
//! the callee is a red C side.

mod common;

use common::two;
use common::TempDir;
use harness_core::deps::DependencyKind;
use harness_core::traits::OracleStrategy;
use harness_core::Verdict;
use harness_oracle::{validate_driver, CAbiDifferential};

fn describe(verdict: &Verdict) -> String {
    verdict
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_callers_test_program_validates_with_the_callees_c_linked() {
    let tmp = TempDir::new("two-dv");
    let target = two::lay_out(tmp.path());
    let unit = two::unit(tmp.path(), "u-caller");
    let driver = tmp.path().join("migration/units/u-caller/driver.c");
    let v = validate_driver(&target, &unit, &driver).expect("validation runs");
    let detail: Vec<String> = v
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect();
    assert!(v.green, "{}", detail.join("\n"));
    assert!(
        v.checks[0]
            .detail
            .ends_with("links against 1 unit file(s) and 1 C file(s) of the pieces it calls"),
        "{}",
        v.checks[0].detail
    );
    // The record names the callee, as its C.
    assert_eq!(v.inputs.dependencies.len(), 1);
    assert_eq!(v.inputs.dependencies[0].unit, "u-callee");
    assert_eq!(v.inputs.dependencies[0].kind, DependencyKind::C);
    // Only the caller's own C is mutated.
    let mutants: Vec<_> = std::fs::read_dir(tmp.path().join("migration/build/u-caller/dv"))
        .expect("dv")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("mut-"))
        .collect();
    assert!(!mutants.is_empty());
    for m in &mutants {
        assert!(m.path().join("caller.c").exists(), "{:?}", m.path());
        assert!(!m.path().join("callee.c").exists(), "{:?}", m.path());
    }
}

#[test]
fn the_callers_rust_calling_the_callee_goes_green() {
    let tmp = TempDir::new("two-green");
    let target = two::lay_out(tmp.path());
    let unit = two::unit(tmp.path(), "u-caller");
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
    let deps = &verdict.inputs.dependencies;
    assert_eq!(deps.len(), 1, "{deps:?}");
    assert_eq!(deps[0].unit, "u-callee");
    assert_eq!(deps[0].kind, DependencyKind::C);
    assert!(deps[0].digest.starts_with("blake3:"));
}

#[test]
fn a_rust_side_that_forgets_the_callee_is_a_red_rust_build() {
    let tmp = TempDir::new("two-forgets");
    let target = two::lay_out(tmp.path());
    two::write_crate(tmp.path(), "u-caller", "caller_rs", two::CALLER_RS_FORGETS);
    let unit = two::unit(tmp.path(), "u-caller");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("a failed link is a verdict, not an error");
    assert!(!verdict.green);
    let last = verdict.checks.last().expect("checks");
    assert_eq!(last.name, "rust-build", "{}", describe(&verdict));
    assert!(!last.passed);
    assert!(
        last.detail
            .starts_with("the unit's Rust does not link into the test program: ")
            && last.detail.contains("callee_mixx"),
        "{}",
        last.detail
    );
}

#[test]
fn a_plan_that_does_not_know_the_callee_is_a_red_c_side() {
    let tmp = TempDir::new("two-unknown");
    let target = two::lay_out(tmp.path());
    two::set_plan(tmp.path(), "pending", "pending", "");
    // A Rust that computes the callee itself, so the import rule has
    // nothing to say: only the C side's link is left to fail.
    two::write_crate(
        tmp.path(),
        "u-caller",
        "caller_rs",
        "#[no_mangle]\npub extern \"C\" fn caller_sum(n: i32) -> i32 {\n    \
         let mut s: u32 = 0;\n    let mut i = 0;\n    while i < n {\n        \
         s = s.wrapping_add((i as u32).wrapping_mul(31).wrapping_add(7) ^ (i as u32));\n        \
         i += 1;\n    }\n    (s % 100000) as i32\n}\n",
    );
    let unit = two::unit(tmp.path(), "u-caller");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("a failed link is a verdict, not an error");
    assert!(!verdict.green);
    let last = verdict.checks.last().expect("checks");
    assert_eq!(last.name, "differential-driver", "{}", describe(&verdict));
    assert!(
        last.detail
            .starts_with(harness_core::verdict::DIFFERENTIAL_C_SIDE_LEAD_IN),
        "{}",
        last.detail
    );
    assert!(verdict.inputs.dependencies.is_empty());
}

/// Freshness (docs/BEYOND-LEAVES-DESIGN.md §4): the caller's verdict
/// recorded the callee as its C; once the callee's Rust is accepted, the
/// verdict is out of date for that reason alone, and a re-check tests the
/// caller against the callee's Rust — green, recorded as `rust`.
#[test]
fn accepting_the_callees_rust_makes_the_callers_verdict_out_of_date() {
    let tmp = TempDir::new("two-fresh");
    let target = two::lay_out(tmp.path());
    let ledger = harness_core::ledger::Ledger::of(&target);
    let facts = harness_core::Facts::load(&ledger.facts_path()).expect("facts");
    let report = |id: &str| {
        harness_core::status::unit_report(
            &target,
            &ledger,
            &facts,
            &two::unit(tmp.path(), id),
            None,
        )
        .expect("report")
    };
    let unit = two::unit(tmp.path(), "u-caller");
    let first = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(first.green, "{}", describe(&first));
    first
        .store(&ledger.verdict_latest_path("u-caller"))
        .expect("stored");
    two::set_plan(tmp.path(), "pending", "verified", "\"u-callee\"");
    assert!(report("u-caller").fresh_green());

    // The callee's Rust accepted.
    two::write_crate(tmp.path(), "u-callee", "callee_rs", two::CALLEE_RS);
    two::set_plan(tmp.path(), "verified", "verified", "\"u-callee\"");
    let stale = report("u-caller");
    assert_eq!(stale.verdict.stale, ["dependencies"]);
    assert!(stale
        .render_line()
        .contains("verdict=green (STALE: dependencies)"));

    // Re-checked: against the callee's Rust, green and fresh.
    let unit = two::unit(tmp.path(), "u-caller");
    let again = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(again.green, "{}", describe(&again));
    let deps = &again.inputs.dependencies;
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].kind, DependencyKind::Rust);
    assert!(tmp
        .path()
        .join("migration/units/u-callee/callee_rs/target/release/libcallee_rs.a")
        .exists());
    again
        .store(&ledger.verdict_latest_path("u-caller"))
        .expect("stored");
    assert!(report("u-caller").fresh_green());

    // The callee's Rust edited: out of date again.
    two::write_crate(
        tmp.path(),
        "u-callee",
        "callee_rs",
        &format!("{}// edited\n", two::CALLEE_RS),
    );
    assert_eq!(report("u-caller").verdict.stale, ["dependencies"]);
}

/// The whole program as it stands (docs/BEYOND-LEAVES-DESIGN.md §2, last
/// row): with the callee accepted, the caller's mixed program links both
/// staticlibs and runs like the C. Proof that the callee's Rust is the one
/// linked: an accepted callee that differs from its C only on large inputs
/// (which only the whole program reaches) turns the whole-program check
/// red, while the caller's own differential stays green.
#[test]
fn the_whole_program_links_every_accepted_piece() {
    let tmp = TempDir::new("two-whole");
    let target = two::lay_out(tmp.path());
    two::write_crate(tmp.path(), "u-callee", "callee_rs", two::CALLEE_RS);
    two::set_plan(tmp.path(), "verified", "pending", "\"u-callee\"");
    let unit = two::unit(tmp.path(), "u-caller");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    assert_eq!(
        verdict
            .checks
            .iter()
            .filter(|c| c.name.starts_with("whole-program:"))
            .count(),
        3
    );

    // The accepted callee off on large inputs only.
    two::write_crate(
        tmp.path(),
        "u-callee",
        "callee_rs",
        "#[no_mangle]\npub extern \"C\" fn callee_mix(a: i32) -> i32 {\n    \
         let r = (a as u32).wrapping_mul(31).wrapping_add(7) as i32;\n    \
         if a >= 1000 { r ^ 1 } else { r }\n}\n",
    );
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let check = |name: &str| {
        verdict
            .checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name}: {}", describe(&verdict)))
            .passed
    };
    assert!(check("differential-driver"), "{}", describe(&verdict));
    assert!(
        !check("whole-program:sample_text.txt"),
        "{}",
        describe(&verdict)
    );
    assert!(
        check("whole-program:sample_empty"),
        "{}",
        describe(&verdict)
    );
}

/// The import rule (docs/BEYOND-LEAVES-DESIGN.md §3): a Rust that calls a
/// function of the program outside the pieces it depends on fails
/// `capabilities`, by name, and is never linked.
#[test]
fn an_import_of_a_program_function_outside_the_dependencies_fails_capabilities() {
    let tmp = TempDir::new("two-import");
    let target = two::lay_out(tmp.path());
    two::write_crate(
        tmp.path(),
        "u-callee",
        "callee_rs",
        "extern \"C\" {\n    fn caller_sum(n: i32) -> i32;\n}\n\
         #[no_mangle]\npub extern \"C\" fn callee_mix(a: i32) -> i32 {\n    \
         if a == i32::MIN { unsafe { caller_sum(1) } } else {\n        \
         (a as u32).wrapping_mul(31).wrapping_add(7) as i32\n    }\n}\n",
    );
    let unit = two::unit(tmp.path(), "u-callee");
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let caps = verdict
        .checks
        .iter()
        .find(|c| c.name == "capabilities")
        .expect("capabilities ran");
    assert!(!caps.passed, "{}", describe(&verdict));
    assert_eq!(
        caps.detail,
        "calls caller_sum, which belongs to a piece it does not depend on"
    );
    assert_eq!(
        verdict.checks.last().map(|c| c.name.as_str()),
        Some("capabilities")
    );
}
