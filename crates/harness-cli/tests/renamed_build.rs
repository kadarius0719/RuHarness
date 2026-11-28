//! A build whose function names the plan step could not match to the
//! source's (docs/SCHEMAS.md "A build whose names the harness cannot
//! follow", the lz4 run of 2026-11-27): `harness gen-driver` and `harness
//! migrate` refuse the piece, and every piece that calls it, in one
//! sentence before anything is asked or written — on a temp copy of the
//! vendored zopfli target whose plan gives `u-hash` an unexplained
//! `build_only`.

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
    if dst.join("migration").is_dir() {
        harness_core::adopt::testing::adopt(dst);
    }
}

struct Run {
    code: i32,
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
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn zopfli_with_unfollowed_names(tag: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!("harness-renamed-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli"),
        &t,
    );
    let plan = t.join("migration/plan.toml");
    let text = std::fs::read_to_string(&plan).unwrap();
    assert_eq!(text.matches("id = \"u-hash\"\n").count(), 1);
    std::fs::write(
        &plan,
        text.replace(
            "id = \"u-hash\"\n",
            "id = \"u-hash\"\nbuild_only = [\"NS_ZopfliResetHash\"]\n",
        ),
    )
    .unwrap();
    t
}

#[test]
fn a_build_the_harness_cannot_follow_is_refused_before_any_question() {
    let t = zopfli_with_unfollowed_names("refuse");
    let target = t.to_str().unwrap();
    let r = harness(&[
        "gen-driver",
        "u-hash",
        "--target",
        target,
        "--provider=external",
        "--model=m",
    ]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stderr.contains(
            "error: u-hash's build gives its functions names the harness cannot follow yet (1 \
             name the source does not have), so no question is posed"
        ),
        "{}",
        r.stderr
    );
    assert!(!r.stderr.contains("invalid plan"), "{}", r.stderr);
    // A piece that calls it, transitively: u-lz77 calls u-hash.
    let r = harness(&[
        "migrate",
        "u-lz77",
        "--target",
        target,
        "--provider=external",
        "--model=m",
    ]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(
        r.stderr.contains(
            "error: u-lz77 calls u-hash, whose build gives its functions names the harness \
             cannot follow yet (1 name the source does not have), so no question is posed"
        ),
        "{}",
        r.stderr
    );
    // Nothing was written for either: no hand-off, no try.
    for unit in ["u-hash", "u-lz77"] {
        let dir = t.join("migration/units").join(unit);
        for sub in ["traces", "driver-traces", "attempts", "driver-attempts"] {
            assert!(!dir.join(sub).exists(), "{unit}/{sub}");
        }
    }
    let _ = std::fs::remove_dir_all(&t);
}
