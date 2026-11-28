//! `harness verify --behind` (docs/SCHEMAS.md "CLI contract") on a temp
//! copy of the vendored zopfli target with two pieces in Rust (u001, and
//! u-util through `harness verify`): nothing behind without a features
//! file; both behind after one is written, re-checked in plan order with
//! their "(1 of 2)" lines; a red piece stops the sweep with verify's own
//! exit code; and a piece together with `--behind`, or neither, refused.

use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_dir(src: &Path, dst: &Path) {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "build" || name_str == "target" || name_str == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
    // A copied ledger is adopted for this test process, as the person's
    // `--adopt` would.
    if dst.join("migration").is_dir() {
        harness_core::adopt::testing::adopt(dst);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn harness(args: &[&str]) -> Run {
    harness_core::adopt::testing::adoption_file();
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .output()
        .expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(
        text.matches(from).count(),
        1,
        "{}: {from:?}",
        path.display()
    );
    std::fs::write(path, text.replace(from, to)).unwrap();
}

/// zopfli's util.c (`ZopfliInitOptions`) in Rust: the same as the C.
const UTIL_RS: &str = r#"//! zopfli's util.c in Rust (a test's second piece).

/// zopfli's `ZopfliOptions`, field for field.
#[repr(C)]
pub struct ZopfliOptions {
    verbose: i32,
    verbose_more: i32,
    numiterations: i32,
    blocksplitting: i32,
    blocksplittinglast: i32,
    blocksplittingmax: i32,
}

/// # Safety
/// `options` points at a writable `ZopfliOptions`.
#[no_mangle]
pub unsafe extern "C" fn ZopfliInitOptions(options: *mut ZopfliOptions) {
    let o = &mut *options;
    o.verbose = 0;
    o.verbose_more = 0;
    o.numiterations = 15;
    o.blocksplitting = 1;
    o.blocksplittinglast = 0;
    o.blocksplittingmax = 15;
}
"#;

/// u-util's differential driver.
const UTIL_DRIVER: &str = "#include <stdio.h>\n#include \"zopfli.h\"\n\n\
int main(void) {\n  ZopfliOptions o = {0};\n  ZopfliInitOptions(&o);\n  \
printf(\"%d %d %d %d %d %d\\n\", o.verbose, o.verbose_more, o.numiterations,\n         \
o.blocksplitting, o.blocksplittinglast, o.blocksplittingmax);\n  return 0;\n}\n";

/// One quick feature: the help.
const FEATURES: &str = "schema_version = 1\n\n[[feature]]\nid = \"help\"\n\
name = \"Show the help\"\n\n[[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n";

/// The zopfli copy with no features file and two pieces in Rust: u001 (as
/// vendored) and u-util, verified here.
fn two_pieces(tag: &str) -> PathBuf {
    let t = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("verify-behind-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli"),
        &t,
    );
    std::fs::remove_dir_all(t.join("migration/features")).unwrap();
    let unit = t.join("migration/units/u-util");
    std::fs::create_dir_all(unit.join("util_rs/src")).unwrap();
    std::fs::write(
        unit.join("util_rs/Cargo.toml"),
        "[package]\nname = \"util_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(unit.join("util_rs/src/lib.rs"), UTIL_RS).unwrap();
    std::fs::write(unit.join("driver.c"), UTIL_DRIVER).unwrap();
    let ends = "symbols = [\"ZopfliInitOptions\"]\n\
                interface = [\"void ZopfliInitOptions(ZopfliOptions* options)\"]\n\
                depends_on = []\ntest_strategy = \"\"\ndone_criteria = \"\"\n";
    edit(
        &t.join("migration/plan.toml"),
        ends,
        &format!(
            "{ends}\n[unit.oracle]\nkind = \"c-abi-differential\"\n\
             driver = \"migration/units/u-util/driver.c\"\nrust_crate = \"util_rs\"\n\
             replaces = [\"src/zopfli/util.c\"]\n"
        ),
    );
    let target = t.to_str().unwrap();
    let r = harness(&["scan", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let r = harness(&["verify", "u-util", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    t
}

fn write_features(t: &Path, text: &str) {
    std::fs::create_dir_all(t.join("migration/features")).unwrap();
    std::fs::write(t.join("migration/features/features.toml"), text).unwrap();
}

/// The `status:` line of `unit`.
fn status_line(target: &str, unit: &str) -> String {
    let r = harness(&["state", "status", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    r.stdout
        .lines()
        .find(|l| l.starts_with(&format!("status: {unit} [")))
        .unwrap_or_else(|| panic!("{}", r.stdout))
        .to_string()
}

const NOTHING: &str =
    "verify: every piece in Rust is checked on the program as it stands — nothing to do\n";

#[test]
fn behind_re_checks_every_piece_in_plan_order_and_stops_at_a_red_one() {
    let t = two_pieces("sweep");
    let target = t.to_str().unwrap();

    // No features file: nothing is behind; the events stream stays valid.
    let r = harness(&["verify", "--behind", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.stdout, NOTHING, "{}", r.stderr);
    let r = harness(&["--json", "verify", "--behind", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let events: Vec<serde_json::Value> = r
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l:?}: {e}")))
        .collect();
    assert_eq!(events.first().unwrap()["k"], "header", "{}", r.stdout);
    assert_eq!(events.last().unwrap()["k"], "result", "{}", r.stdout);
    assert!(events
        .iter()
        .any(|e| e["k"] == "message" && e["text"] == NOTHING.trim_end()));

    // A features file: both verdicts are behind, both re-checked in plan
    // order (u001 comes first), each as `verify <unit>` would.
    write_features(&t, FEATURES);
    assert!(
        status_line(target, "u-util").contains("features=behind("),
        "{}",
        status_line(target, "u-util")
    );
    let r = harness(&["verify", "--behind", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    let first = r
        .stdout
        .find("verify: u001-katajainen (1 of 2)\n")
        .unwrap_or_else(|| panic!("{}", r.stdout));
    let second = r
        .stdout
        .find("verify: u-util (2 of 2)\n")
        .unwrap_or_else(|| panic!("{}", r.stdout));
    assert!(first < second, "{}", r.stdout);
    let green_one = r
        .stdout
        .find("verify: u001-katajainen GREEN — status set to verified\n")
        .unwrap_or_else(|| panic!("{}", r.stdout));
    assert!(first < green_one && green_one < second, "{}", r.stdout);
    assert!(
        r.stdout.contains("[PASS] feature:help/flag"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.ends_with(
            "verify: u-util GREEN — status set to verified\n\
             verify: 2 pieces re-checked on the program as it stands — all green\n"
        ),
        "{}",
        r.stdout
    );
    for unit in ["u001-katajainen", "u-util"] {
        let line = status_line(target, unit);
        assert!(line.ends_with("features=current"), "{line}");
    }
    // Now nothing is behind.
    let r = harness(&["verify", "--behind", "--target", target]);
    assert_eq!(r.code, 0, "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.stdout, NOTHING);

    // The features change (both behind again) and u001's Rust no longer
    // builds: the sweep stops at it, red, and u-util is not re-checked.
    write_features(
        &t,
        &format!(
            "{FEATURES}\n[[scenario]]\nfeature = \"help\"\nid = \"long\"\nargs = [\"--help\"]\n"
        ),
    );
    let lib = t.join("migration/units/u001-katajainen/katajainen_rs/src/lib.rs");
    let mut text = std::fs::read_to_string(&lib).unwrap();
    text.push_str("\nthis does not build\n");
    std::fs::write(&lib, text).unwrap();
    let r = harness(&["verify", "--behind", "--target", target]);
    assert_eq!(r.code, 10, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("verify: u001-katajainen (1 of 2)\n"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("(2 of 2)"), "{}", r.stdout);
    assert!(
        r.stdout.ends_with(
            "verify: u001-katajainen RED — status demoted verified -> in-progress\n\
             verify: stopped at u001-katajainen — 1 piece not re-checked\n"
        ),
        "{}",
        r.stdout
    );
    let line = status_line(target, "u-util");
    assert!(line.contains("features=behind(changed"), "{line}");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn verify_takes_a_piece_or_behind_exactly_one() {
    let r = harness(&["verify", "u-util", "--behind"]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(
            "give one piece to verify, or --behind for every piece not yet checked on the \
             program as it stands — not both"
        ),
        "{}",
        r.stderr
    );
    let r = harness(&["verify"]);
    assert_eq!(r.code, 2, "{}\n{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(
            "say which piece to verify (harness verify <UNIT>), or --behind for every piece \
             not yet checked on the program as it stands"
        ),
        "{}",
        r.stderr
    );
    assert!(r.stdout.is_empty(), "{}", r.stdout);
}
