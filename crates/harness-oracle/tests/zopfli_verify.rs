//! The real thing: `CAbiDifferential::verify` for unit u001 of the vendored
//! zopfli target, in a temp copy (the repo's own ledger is never mutated).
//! Must stay GREEN, under the sandbox where one exists, with the M3 checks
//! and evidence in place.

mod common;

use harness_core::traits::OracleStrategy;
use harness_core::{Plan, TargetContext};
use harness_oracle::{sandbox_mode, CAbiDifferential};
use std::path::Path;

#[test]
fn u001_katajainen_is_green_with_the_m3_trust_boundaries() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli");
    let tmp = common::TempDir::new("zopfli");
    common::copy_dir(&src, tmp.path());

    let target = TargetContext::load(tmp.path()).expect("harness.toml loads");
    assert!(
        target.config.oracle_allowlist().iter().any(|t| t == "nm"),
        "the zopfli target must allowlist nm for the symbol-set check"
    );
    let plan = Plan::load(&tmp.path().join("migration/plan.toml")).expect("plan loads");
    let unit = plan.unit("u001-katajainen").expect("u001 exists");

    let started = std::time::Instant::now();
    let verdict = CAbiDifferential.verify(&target, unit).expect("oracle runs");
    eprintln!(
        "u001 verify ({}) took {:.2?}",
        sandbox_mode(),
        started.elapsed()
    );

    let summary: Vec<String> = verdict
        .checks
        .iter()
        .map(|c| format!("[{}] {} — {}", c.passed, c.name, c.detail))
        .collect();
    assert!(verdict.green, "u001 must be green:\n{}", summary.join("\n"));

    // Check order: symbol-set right after the build, then the M0 checks.
    let names: Vec<&str> = verdict.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "symbol-set",
            "differential-driver",
            "whole-program:sample_text.txt",
            "whole-program:sample_rand.bin",
            "whole-program:sample_empty",
            "sanitizers",
        ]
    );
    // The symbol comparison is exact. On a platform with a constructor
    // scanner (macOS `nm -m`) the detail is nothing more; where none is
    // available (the zopfli target does not allowlist `objdump`, so Linux
    // cannot scan) the check records that fact but stays green.
    let symbol_detail = &verdict.checks[0].detail;
    assert!(
        symbol_detail.starts_with("1 exported symbol(s) match the unit's symbols exactly"),
        "{symbol_detail}"
    );
    if cfg!(target_os = "macos") {
        assert_eq!(
            symbol_detail,
            "1 exported symbol(s) match the unit's symbols exactly"
        );
    }
    // The differential still sees the full ~180KB driver output — far more
    // than a pipe buffer, so this also proves the capture cannot deadlock.
    let driver_bytes: usize = verdict.checks[1]
        .detail
        .strip_suffix(" bytes identical")
        .and_then(|n| n.parse().ok())
        .expect("differential detail reports the byte count");
    assert!(driver_bytes > 100_000, "{}", verdict.checks[1].detail);

    // Evidence: rustc, cc, and the sandbox mode actually applied.
    let toolchain = &verdict.inputs.toolchain;
    assert_eq!(toolchain.len(), 3, "{toolchain:?}");
    assert!(toolchain[0].starts_with("rustc "), "{toolchain:?}");
    assert_eq!(toolchain[2], format!("sandbox: {}", sandbox_mode()));
    if cfg!(target_os = "macos") {
        assert_eq!(sandbox_mode(), "sandbox-exec");
    }

    // The baseline was built inside the ledger build dir and cached by
    // `rustc -V`.
    let cache = tmp
        .path()
        .join("migration/build/symbol-baseline/unwind/symbols.txt");
    let text = std::fs::read_to_string(&cache).expect("baseline cache written");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("ruharness-symbol-baseline v2"));
    assert_eq!(lines.next(), Some(toolchain[0].as_str()));
    // The v2 cache persists the constructor-scan availability marker.
    assert!(
        text.lines().any(|l| l == "A 1" || l == "A 0"),
        "baseline cache records constructor-scan availability:\n{text}"
    );

    // Writes stayed where the contract says: build dir + the crate's target/.
    assert!(tmp
        .path()
        .join("migration/build/u001-katajainen/drv_rs.out")
        .exists());
    assert!(tmp
        .path()
        .join("migration/units/u001-katajainen/katajainen_rs/target/release")
        .is_dir());
}
