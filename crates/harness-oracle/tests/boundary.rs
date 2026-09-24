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

/// The header declares `scan`, `pick` and `show` — NOT `helper` and `crc`
/// (the driver declares those itself, as corpus drivers do): the wrapper must
/// declare every symbol from its interface line. `crc`'s last parameter is
/// named `crc`: the wrapper must bind the real function where no parameter
/// can shadow it.
const HEADER_TYPES: &str = "#include <stdint.h>\n\
typedef struct { const uint8_t *buf; int pos, limit; } hdr_t;\n\
void scan(hdr_t *h, const uint8_t *tags, int n, int *out);\n\
int *pick(int *a, int *b, int which);\n\
void show(const char *s);\n\
int tail_sum(const int *p, int n);\n\
void first_nonzero(const int *p, int n, const int **out);\n\
int count(void);\n";

/// A second unit file whose only pointer use is a libc call: it makes no
/// load of its own, so its instrumented object references no callback.
const PRINT_C: &str = "#include <stdio.h>\n#include \"unit.h\"\n\
void show(const char *s) { printf(\"show %s\\n\", s); }\n";

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
int *pick(int *a, int *b, int which) { return which ? b : a; }\n\
int helper(int *p) { return *p + 1; }\n\
int crc(const uint8_t *d, int n, int crc) {\n\
    int i;\n\
    for (i = 0; i < n; i++) crc = (int)(((unsigned)crc * 31u) ^ d[i]);\n\
    return crc;\n\
}\n\
int tail_sum(const int *p, int n) {\n\
    int i, s = 0;\n\
    for (i = 1; i < n; i++) s += p[i];\n\
    return s;\n\
}\n\
void first_nonzero(const int *p, int n, const int **out) {\n\
    int i;\n\
    *out = 0;\n\
    for (i = 0; i < n; i++) if (p[i]) { *out = &p[i]; return; }\n\
}\n\
int count(void) { return 8; }\n";

/// The (already validated) driver: call 1 sets no tag, so the C never reads
/// `h` there; every call passes 16-element buffers with `n < 16`.
const DRIVER_C: &str = "#include <stdio.h>\n#include <stdint.h>\n#include \"unit.h\"\n\
int helper(int *p);\n\
int crc(const uint8_t *d, int n, int seed);\n\
int main(void) {\n\
    uint8_t tags[16]; int out[16]; hdr_t h; int i, k;\n\
    char msg[8] = \"literal\";\n\
    for (k = 0; k < count(); k++) {\n\
        const int *fnz;\n\
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
        printf(\"helper %d crc %d\\n\", helper(&x), crc(tags, n, k));\n\
        show(msg);\n\
        printf(\"tail %d\\n\", tail_sum(out, n));\n\
        first_nonzero(out, n, &fnz);\n\
        printf(\"fnz %d\\n\", fnz ? *fnz : -1);\n\
    }\n\
    return 0;\n\
}\n";

const HDR_RS: &str = "#[repr(C)]\npub struct Hdr {\n    pub buf: *const u8,\n    pub pos: i32,\n    pub limit: i32,\n}\n";

const PICK_RS: &str = "#[no_mangle]\npub unsafe extern \"C\" fn pick(a: *mut i32, b: *mut i32, which: i32) -> *mut i32 {\n    if which != 0 { b } else { a }\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn helper(p: *mut i32) -> i32 {\n    (*p).wrapping_add(1)\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn crc(d: *const u8, n: i32, mut crc: i32) -> i32 {\n    for i in 0..n.max(0) as usize {\n        crc = crc.wrapping_mul(31) ^ (*d.add(i) as i32);\n    }\n    crc\n}\n\
extern \"C\" {\n    fn printf(fmt: *const u8, ...) -> i32;\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn show(s: *const u8) {\n    printf(b\"show %s\\n\\0\".as_ptr(), s);\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn tail_sum(p: *const i32, n: i32) -> i32 {\n    let mut s = 0i32;\n    for i in 1..n.max(0) as usize {\n        s = s.wrapping_add(*p.add(i));\n    }\n    s\n}\n\
#[no_mangle]\npub unsafe extern \"C\" fn first_nonzero(p: *const i32, n: i32, out: *mut *const i32) {\n    *out = std::ptr::null();\n    for i in 0..n.max(0) as usize {\n        if *p.add(i) != 0 {\n            *out = p.add(i);\n            return;\n        }\n    }\n}\n\
#[no_mangle]\npub extern \"C\" fn count() -> i32 {\n    8\n}\n";

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

/// Reads `p[0]` in `tail_sum` (the C starts at 1) without using it: invisible
/// to the output and to the tail layout, caught only by the head layout.
fn below_window() -> String {
    faithful().replace(
        "    let mut s = 0i32;\n    for i in 1..n.max(0) as usize {",
        "    let mut s = 0i32;\n    if n > 1 { std::hint::black_box(*p); }\n    for i in 1..n.max(0) as usize {",
    )
}

/// Caches `h` in a static and reads it on the next call: a retained pointer.
fn retained_pointer() -> String {
    format!(
        "static mut LAST: *mut Hdr = std::ptr::null_mut();\n{}",
        faithful().replace(
            "    let mut acc = 0i32;\n",
            "    let last = *std::ptr::addr_of!(LAST);\n    if !last.is_null() { std::hint::black_box((*last).limit); }\n    \
             *std::ptr::addr_of_mut!(LAST) = h;\n    let mut acc = 0i32;\n",
        )
    )
}

/// Ends the process inside the last unit call (the ninth `count`, which
/// ends the driver's loop): the output is complete, the record is not.
fn exits_inside_a_call() -> String {
    format!(
        "extern \"C\" {{\n    fn exit(code: i32) -> !;\n}}\nstatic mut CALLS: u32 = 0;\n{}",
        faithful().replace(
            "#[no_mangle]\npub extern \"C\" fn count() -> i32 {\n    8\n}\n",
            "#[no_mangle]\npub unsafe extern \"C\" fn count() -> i32 {\n    *std::ptr::addr_of_mut!(CALLS) += 1;\n    \
             if *std::ptr::addr_of!(CALLS) == 9 { exit(0); }\n    8\n}\n",
        )
    )
}

/// Makes the driver loop one iteration less: fewer calls than the C made.
fn fewer_calls() -> String {
    faithful().replace(
        "#[no_mangle]\npub extern \"C\" fn count() -> i32 {\n    8\n}\n",
        "#[no_mangle]\npub extern \"C\" fn count() -> i32 {\n    7\n}\n",
    )
}

/// Installs its own fault handler (which opens the faulting page and
/// returns), reads the object the C never touches, then RESTORES the
/// runtime's handler before returning: only signal accounting can see it.
fn tamper_signal() -> String {
    format!(
        "#[repr(C)]\n#[derive(Clone, Copy)]\npub struct SigAction {{ pub handler: usize, pub mask: u32, pub flags: i32 }}\n\
         extern \"C\" {{\n    fn sigaction(sig: i32, act: *const SigAction, old: *mut SigAction) -> i32;\n    \
         fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;\n    fn getpagesize() -> i32;\n}}\n\
         static mut SAVED: [SigAction; 2] = [SigAction {{ handler: 0, mask: 0, flags: 0 }}; 2];\n\
         extern \"C\" fn open_page(_sig: i32, info: *mut u8, _uc: *mut u8) {{\n    unsafe {{\n        \
         let addr = *(info.add(24) as *const usize);\n        let pg = getpagesize() as usize;\n        \
         mprotect((addr / pg * pg) as *mut u8, pg, 3);\n    }}\n}}\n{}",
        faithful().replace(
            "    let mut acc = 0i32;\n",
            "    let mine = SigAction { handler: open_page as usize, mask: 0, flags: 0x40 };\n    \
             let saved = std::ptr::addr_of_mut!(SAVED) as *mut SigAction;\n    \
             sigaction(10, &mine, saved);\n    sigaction(11, &mine, saved.add(1));\n    \
             std::hint::black_box((*h).limit);\n    \
             sigaction(10, saved, std::ptr::null_mut());\n    sigaction(11, saved.add(1), std::ptr::null_mut());\n    \
             let mut acc = 0i32;\n",
        )
    )
}

/// Registers a Mach exception port for bad accesses and leaves it.
fn tamper_exception_port() -> String {
    format!(
        "extern \"C\" {{\n    static mach_task_self_: u32;\n    fn mach_port_allocate(task: u32, right: i32, name: *mut u32) -> i32;\n    \
         fn mach_port_insert_right(task: u32, name: u32, poly: u32, poly_type: u32) -> i32;\n    \
         fn task_set_exception_ports(task: u32, mask: u32, port: u32, behavior: i32, flavor: i32) -> i32;\n}}\n{}",
        faithful().replace(
            "    let mut acc = 0i32;\n",
            "    let task = *std::ptr::addr_of!(mach_task_self_);\n    let mut port = 0u32;\n    \
             mach_port_allocate(task, 1, &mut port);\n    mach_port_insert_right(task, port, port, 20);\n    \
             task_set_exception_ports(task, 2, port, 1, 0);\n    let mut acc = 0i32;\n",
        )
    )
}

/// Opens the page holding `h` (the object the C never touches in call 1),
/// reads it, and leaves the page open: no fault, no signal — only the
/// re-verification of the reservation's pages can see it.
fn tamper_protection() -> String {
    format!(
        "extern \"C\" {{\n    fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;\n    fn getpagesize() -> i32;\n}}\n{}",
        faithful().replace(
            "    let mut acc = 0i32;\n",
            "    let pg = getpagesize() as usize;\n    mprotect(((h as usize) / pg * pg) as *mut u8, pg, 3);\n    \
             std::hint::black_box((*h).limit);\n    let mut acc = 0i32;\n",
        )
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
    write(&root.join("src/print.c"), PRINT_C);
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
                path: "src/print.c".to_string(),
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
        "id = \"u-bnd\"\nstatus = \"pending\"\nfiles = [\"src/unit.c\", \"src/print.c\"]\n\
         symbols = [\"scan\", \"pick\", \"helper\", \"crc\", \"show\", \"tail_sum\", \
         \"first_nonzero\", \"count\"]\n\
         interface = [\"void scan(hdr_t *h, const uint8_t *tags, int n, int *out)\", \
         \"int *pick(int *a, int *b, int which)\", \"int helper(int *p)\", \
         \"int crc(const uint8_t *d, int n, int crc)\", \"void show(const char *s)\", \
         \"int tail_sum(const int *p, int n)\", \
         \"void first_nonzero(const int *p, int n, const int **out)\", \"int count(void)\"]\n\n\
         [oracle]\nkind = \"c-abi-differential\"\n{boundary}\
         driver = \"migration/units/u-bnd/driver.c\"\nrust_crate = \"bnd_rs\"\n\
         replaces = [\"src/unit.c\", \"src/print.c\"]\n"
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
    // Per iteration (after the `count` call that drives the loop): `scan`
    // (h, tags, out), `pick` (a, b), `helper` (x), `crc` (tags), `show`
    // (msg: only libc reads it, so its window is learned and widened),
    // `tail_sum` (out), `first_nonzero` (out, &fnz — the pointer it stores is
    // relocated back to the original, which the driver dereferences). `h` is
    // untouched in the first `scan` and `pick` never dereferences its
    // arguments (16 more untouched objects); `tags` (twice), `out` (three
    // times) are partially touched in every iteration (n < 16).
    assert!(
        check.detail.contains(
            "65 call(s), 88 guarded object(s) (17 untouched by the C, 40 partially touched, 8 widened), 0 argument(s) unshadowed; tail and head layouts clean; widened (object-level only): call 6: show.s, call 14: show.s, call 22: show.s, call 30: show.s, call 38: show.s, call 46: show.s +2 more"
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
        check.detail.starts_with("in call 2 of scan, the Rust touched the object passed as `h` (1 x 16 bytes) the C does not touch it in that call; tail layout."),
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
    let (check, report) = CAbiDifferential
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
    // The report exists once the windows do, whatever the outcome.
    assert!(report.is_some_and(|r| !r.is_vacuous()));
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
fn a_read_below_the_c_window_is_caught_by_the_head_layout() {
    let tmp = TempDir::new("bnd-below");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &below_window());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let check = boundary_check(&verdict).expect("boundary ran");
    assert!(!check.passed);
    assert!(
        check.detail.contains(
            "passed as `p` (16 x 4 bytes) below the C's window (elements [1, 2)); head layout"
        ),
        "{}",
        check.detail
    );
}

#[test]
fn a_pointer_retained_across_calls_is_red() {
    let tmp = TempDir::new("bnd-stale");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &retained_pointer());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    let check = boundary_check(&verdict).expect("boundary ran");
    assert!(!check.passed);
    assert!(
        check
            .detail
            .contains("the Rust touched the object passed as `h` to call ")
            && check.detail.contains(
                " of scan, which no longer exists: a pointer retained across calls (tail layout)"
            ),
        "{}",
        check.detail
    );
}

#[test]
fn ending_the_process_inside_a_call_is_a_candidate_crash_not_a_harness_error() {
    let tmp = TempDir::new("bnd-exit");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &exits_inside_a_call());
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs, never Err");
    let check = boundary_check(&verdict).expect("boundary ran");
    assert!(!check.passed);
    assert!(
        check.detail.starts_with("candidate run failed: the Rust ended the process inside call 65 of count, which the C never does (tail layout)"),
        "{}",
        check.detail
    );
}

#[test]
fn fewer_calls_than_the_c_made_is_a_changed_control_flow() {
    let tmp = TempDir::new("bnd-fewer");
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &fewer_calls());
    let (check, _) = CAbiDifferential
        .boundary_only(&target, &unit)
        .expect("check runs");
    assert!(!check.passed);
    assert!(
        check
            .detail
            .starts_with("the Rust changed the driver's control flow (call 58)"),
        "{}",
        check.detail
    );
}

/// Every fail-closed check of §B.R-1 has a candidate that only it can see.
#[test]
fn each_integrity_check_catches_its_own_tamper() {
    for (name, lib, what) in [
        ("signal", tamper_signal(), "(signal)"),
        (
            "exception-port",
            tamper_exception_port(),
            "(exception-port)",
        ),
        ("protection", tamper_protection(), "(protection)"),
    ] {
        let tmp = TempDir::new(&format!("bnd-tamper-{name}"));
        let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &lib);
        let (check, _) = CAbiDifferential
            .boundary_only(&target, &unit)
            .expect("check runs");
        assert!(!check.passed, "{name}: {}", check.detail);
        assert!(
            check
                .detail
                .contains(&format!("the guard was tampered with {what}")),
            "{name}: {}",
            check.detail
        );
    }
}

#[test]
fn a_late_red_check_keeps_the_boundary_check_from_running() {
    let tmp = TempDir::new("bnd-late");
    // The blind spot plus a wrong `helper`: differential-driver is red, and the
    // gate must hold there — not only at the early-returning checks.
    let lib = eager_header().replace("    (*p).wrapping_add(1)\n", "    (*p).wrapping_add(2)\n");
    assert_ne!(lib, eager_header());
    let (target, unit) = layout(tmp.path(), HEADER_TYPES, true, &lib);
    let verdict = CAbiDifferential
        .verify(&target, &unit)
        .expect("oracle runs");
    assert!(!verdict.green);
    let dd = verdict
        .checks
        .iter()
        .find(|c| c.name == "differential-driver")
        .expect("ran");
    assert!(!dd.passed, "{}", describe(&verdict));
    assert!(boundary_check(&verdict).is_none(), "{}", describe(&verdict));
    assert!(verdict
        .inputs
        .toolchain
        .iter()
        .any(|t| t.starts_with("boundary:")));
}

#[test]
fn an_early_red_records_the_opt_in_but_no_boundary_entry() {
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
