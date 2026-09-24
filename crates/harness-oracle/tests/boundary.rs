//! Design B end to end (docs/ORACLE-HARDENING.md §B): the boundary check
//! through the public API, on a synthetic unit shaped like the blind spot it
//! exists for — a struct the C reads only on some path, a buffer the C reads
//! only up to `n`, a pointer return the driver dereferences. Every outcome
//! class has a candidate: byte-identity when a unit does not opt in, green
//! for a faithful translation, red for an object the C never touches and for
//! a read past the C's window, red for a candidate that alters how faults
//! are delivered, a C-side detail when the interface's types are not the
//! headers', and the gate that the check runs only after every other check
//! passed.

mod common;

use common::{write, TempDir};
use harness_core::facts::FileRecord;
use harness_core::traits::OracleStrategy;
use harness_core::verdict::BOUNDARY_C_SIDE_LEAD_IN;
use harness_core::{Facts, TargetContext, Unit, Verdict};
use harness_oracle::CAbiDifferential;
use std::path::Path;

const HEADER_TYPES: &str = "#include <stdint.h>\n\
typedef struct { const uint8_t *buf; int pos, limit; } hdr_t;\n\
void scan(hdr_t *h, const uint8_t *tags, int n, int *out);\n\
int *pick(int *a, int *b, int which);\n";

/// The unit's C: `h` is read only when some tag is set; `tags` only up to `n`.
const UNIT_C: &str = "#include \"unit.h\"\n\
void scan(hdr_t *h, const uint8_t *tags, int n, int *out) {\n\
    int i, acc = 0;\n\
    for (i = 0; i < n; i++) {\n\
        if (tags[i] == 0) { out[i] = 0; continue; }\n\
        acc += h->limit;\n\
        out[i] = acc + tags[i];\n\
    }\n\
}\n\
int *pick(int *a, int *b, int which) { return which ? b : a; }\n";

/// The (already validated) driver: call 1 sets no tag, so the C never reads
/// `h` there; every call passes 16-element buffers with `n < 16`.
const DRIVER_C: &str = "#include <stdio.h>\n#include <stdint.h>\n#include \"unit.h\"\n\
int main(void) {\n\
    uint8_t tags[16]; int out[16]; hdr_t h; int i, k;\n\
    for (k = 0; k < 8; k++) {\n\
        int n = 2 + k;\n\
        int x = k, y = -k;\n\
        for (i = 0; i < 16; i++) tags[i] = (uint8_t)(((k * 7 + i) % 3 == 0 && k != 0) ? i + 1 : 0);\n\
        for (i = 0; i < 16; i++) out[i] = -1;\n\
        h.buf = tags; h.pos = k; h.limit = 100 + k;\n\
        scan(&h, tags, n, out);\n\
        printf(\"case %d:\", k);\n\
        for (i = 0; i < n; i++) printf(\" %d\", out[i]);\n\
        printf(\"\\n\");\n\
        printf(\"pick %d\\n\", *pick(&x, &y, k & 1));\n\
    }\n\
    return 0;\n\
}\n";

const HDR_RS: &str = "#[repr(C)]\npub struct Hdr {\n    pub buf: *const u8,\n    pub pos: i32,\n    pub limit: i32,\n}\n";

const PICK_RS: &str = "#[no_mangle]\npub unsafe extern \"C\" fn pick(a: *mut i32, b: *mut i32, which: i32) -> *mut i32 {\n    if which != 0 { b } else { a }\n}\n";

/// Touches exactly what the C touches, on the same call.
fn faithful() -> String {
    format!(
        "{HDR_RS}#[no_mangle]\npub unsafe extern \"C\" fn scan(h: *mut Hdr, tags: *const u8, n: i32, out: *mut i32) {{\n    \
         let mut acc = 0i32;\n    for i in 0..n.max(0) as usize {{\n        let t = *tags.add(i);\n        \
         if t == 0 {{ *out.add(i) = 0; continue; }}\n        acc = acc.wrapping_add((*h).limit);\n        \
         *out.add(i) = acc.wrapping_add(t as i32);\n    }}\n}}\n{PICK_RS}"
    )
}

/// Reads `h` at entry — the blind spot: correct output, wrong footprint.
fn eager_header() -> String {
    faithful().replace(
        "    let mut acc = 0i32;\n",
        "    let limit = (*h).limit;\n    let mut acc = 0i32;\n    std::hint::black_box(limit);\n",
    )
}

/// Sums all 16 tags although the C reads only `n`.
fn eager_tags() -> String {
    faithful().replace(
        "    let mut acc = 0i32;\n",
        "    let all: &[u8] = std::slice::from_raw_parts(tags, 16);\n    \
         let sum: u32 = all.iter().map(|&b| b as u32).sum();\n    std::hint::black_box(sum);\n    let mut acc = 0i32;\n",
    )
}

/// Faithful, but installs its own SIGBUS handler and leaves it installed.
fn tampering() -> String {
    format!(
        "extern \"C\" {{\n    fn signal(sig: i32, handler: usize) -> usize;\n}}\n\
         extern \"C\" fn swallow(_sig: i32) {{}}\n{}",
        faithful().replace(
            "    let mut acc = 0i32;\n",
            "    signal(10, swallow as usize);\n    signal(11, swallow as usize);\n    let mut acc = 0i32;\n",
        )
    )
}

/// Lay out the target. `header` is the unit's header text; `opt_in` adds
/// `boundary = true` to the plan entry.
fn layout(root: &Path, header: &str, opt_in: bool, lib_rs: &str) -> (TargetContext, Unit) {
    write(
        &root.join("harness.toml"),
        "schema_version = 1\n\n[target]\nname = \"bnd\"\nsource_dir = \"src\"\n\n\
         [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
    );
    write(&root.join("src/unit.h"), header);
    write(&root.join("src/unit.c"), UNIT_C);
    write(&root.join("migration/units/u-bnd/driver.c"), DRIVER_C);
    let crate_dir = root.join("migration/units/u-bnd/bnd_rs");
    write(
        &crate_dir.join("Cargo.toml"),
        "[package]\nname = \"bnd_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\n",
    );
    write(&crate_dir.join("src/lib.rs"), lib_rs);
    let facts = Facts {
        frontend: "test-inline".to_string(),
        files: vec![
            FileRecord {
                path: "src/unit.c".to_string(),
                hash: String::new(),
                includes: vec!["src/unit.h".to_string()],
            },
            FileRecord {
                path: "src/unit.h".to_string(),
                hash: String::new(),
                includes: Vec::new(),
            },
        ],
        ..Facts::default()
    };
    facts
        .store(&root.join("migration/facts.jsonl"))
        .expect("facts stored");
    let boundary = if opt_in { "boundary = true\n" } else { "" };
    let unit: Unit = toml::from_str(&format!(
        "id = \"u-bnd\"\nstatus = \"pending\"\nfiles = [\"src/unit.c\"]\n\
         symbols = [\"scan\", \"pick\"]\n\
         interface = [\"void scan(hdr_t *h, const uint8_t *tags, int n, int *out)\", \
         \"int *pick(int *a, int *b, int which)\"]\n\n\
         [oracle]\nkind = \"c-abi-differential\"\n{boundary}\
         driver = \"migration/units/u-bnd/driver.c\"\nrust_crate = \"bnd_rs\"\n\
         replaces = [\"src/unit.c\"]\n"
    ))
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

fn boundary_check(verdict: &Verdict) -> Option<&harness_core::verdict::Check> {
    verdict.checks.iter().find(|c| c.name == "boundary")
}

#[test]
fn a_unit_that_does_not_opt_in_gets_no_boundary_check_at_all() {
    let tmp = TempDir::new("bnd-off");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, false, &eager_header());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    // The blind spot passes every other check, by construction.
    assert!(verdict.green, "{}", describe(&verdict));
    assert!(boundary_check(&verdict).is_none(), "{}", describe(&verdict));
    assert!(
        !verdict
            .inputs
            .toolchain
            .iter()
            .any(|t| t.starts_with("boundary:")),
        "{:?}",
        verdict.inputs.toolchain
    );
}

#[test]
fn a_faithful_candidate_is_green_and_the_verdict_records_the_runtime() {
    let tmp = TempDir::new("bnd-green");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &faithful());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(verdict.green, "{}", describe(&verdict));
    let check = verdict.checks.last().expect("checks");
    assert_eq!(check.name, "boundary", "{}", describe(&verdict));
    // 8 calls of `scan` (h, tags, out) and 8 of `pick` (a, b): `h` is untouched
    // in call 1 and `pick` never dereferences its arguments (16 more untouched
    // objects — the driver dereferences the RETURNED pointer, which the
    // runtime relocated back to the original); `tags` and `out` are partially
    // touched in every `scan` call (n < 16).
    assert!(
        check.detail.contains(
            "16 call(s), 40 guarded object(s) (17 untouched by the C, 16 partially touched, 0 widened), 0 argument(s) unshadowed"
        ),
        "{}",
        check.detail
    );
    let entry = verdict
        .inputs
        .toolchain
        .iter()
        .position(|t| t.starts_with("boundary: sancov+guard-pages rt="))
        .expect("toolchain entry");
    let observable = verdict
        .inputs
        .toolchain
        .iter()
        .position(|t| t == "observable: stdout+stderr")
        .expect("observable entry");
    assert!(entry < observable, "{:?}", verdict.inputs.toolchain);
}

#[test]
fn reading_an_object_the_c_never_touches_is_red_with_the_harness_wording() {
    let tmp = TempDir::new("bnd-untouched");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &eager_header());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green, "{}", describe(&verdict));
    let check = boundary_check(&verdict).expect("boundary ran");
    assert!(!check.passed);
    assert!(
        check.detail.starts_with("in call 1 of scan, the Rust touched the object passed as `h` (1 x 16 bytes) the C does not touch it in that call; tail layout."),
        "{}",
        check.detail
    );
    assert!(!check.detail.starts_with(BOUNDARY_C_SIDE_LEAD_IN));
    assert!(
        !check.detail.contains("run failed") && !check.detail.contains("timed out"),
        "{}",
        check.detail
    );
    // Every other check passed: the blind spot is invisible to them.
    assert!(
        verdict
            .checks
            .iter()
            .filter(|c| c.name != "boundary")
            .all(|c| c.passed),
        "{}",
        describe(&verdict)
    );
}

#[test]
fn reading_past_the_c_window_is_red() {
    let tmp = TempDir::new("bnd-above");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &eager_tags());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let check = boundary_check(&verdict).expect("boundary ran");
    assert!(!check.passed);
    assert!(
        check
            .detail
            .contains("passed as `tags` (16 x 1 bytes) above the C's window (elements [0, 2))"),
        "{}",
        check.detail
    );
}

#[test]
fn a_candidate_that_alters_fault_delivery_is_red_or_never_reaches_the_check() {
    let tmp = TempDir::new("bnd-tamper");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &tampering());
    // Through `verify`, the capabilities gate refuses `signal` to an opted-in
    // unit's candidate before anything is linked.
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let caps = verdict
        .checks
        .iter()
        .find(|c| c.name == "capabilities")
        .expect("capabilities");
    assert!(
        !caps.passed && caps.detail.contains("signal (signal)"),
        "{}",
        describe(&verdict)
    );
    assert!(boundary_check(&verdict).is_none(), "{}", describe(&verdict));
    // The runtime itself fails closed: the calibration entry point skips the
    // capabilities gate, and the leftover handler is detected after the call.
    let check = CAbiDifferential
        .boundary_only(&target, &unit)
        .expect("check runs");
    assert!(!check.passed);
    assert!(
        check
            .detail
            .contains("the guard was tampered with (handler)"),
        "{}",
        check.detail
    );
}

#[test]
fn interface_types_the_headers_do_not_declare_make_the_check_not_applicable() {
    let tmp = TempDir::new("bnd-cside");
    // unit.c defines `scan` through a private typedef alias; the scanner's
    // interface line names that alias, which the header (and so the driver,
    // which compiles fine) never declares.
    let unit_c = UNIT_C.replace(
        "#include \"unit.h\"\nvoid scan(hdr_t *h,",
        "#include \"unit.h\"\ntypedef hdr_t hdr_priv_t;\nvoid scan(hdr_priv_t *h,",
    );
    assert_ne!(unit_c, UNIT_C);
    let (target, mut unit) = layout(tmp.path(), HEADER_TYPES, true, &faithful());
    write(&tmp.path().join("src/unit.c"), &unit_c);
    unit.interface[0] = "void scan(hdr_priv_t *h, const uint8_t *tags, int n, int *out)".into();
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let check =
        boundary_check(&verdict).unwrap_or_else(|| panic!("boundary ran:\n{}", describe(&verdict)));
    assert!(!check.passed);
    assert!(
        check.detail.starts_with(BOUNDARY_C_SIDE_LEAD_IN)
            && check
                .detail
                .contains("the prototype of `scan` does not compile against the unit's headers"),
        "{}",
        check.detail
    );
    assert!(!verdict.green);
}

#[test]
fn the_check_runs_only_after_every_other_check_passed() {
    let tmp = TempDir::new("bnd-gate");
    let lib = format!(
        "{}\n#[no_mangle]\npub extern \"C\" fn helper_the_plan_never_heard_of() -> i32 {{\n    7\n}}\n",
        eager_header()
    );
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    assert_eq!(
        verdict.checks[0].name,
        "symbol-set",
        "{}",
        describe(&verdict)
    );
    assert!(boundary_check(&verdict).is_none(), "{}", describe(&verdict));
    // …and a red check earlier in the same run keeps recorded evidence intact:
    // only green runs grow a boundary entry.
    assert!(
        verdict
            .inputs
            .toolchain
            .iter()
            .any(|t| t.starts_with("boundary:")),
        "the toolchain entry records the opt-in whether or not the check ran: {:?}",
        verdict.inputs.toolchain
    );
}
