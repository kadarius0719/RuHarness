//! The cockpit driver (devtools/cockpit-drive): a checker who points
//! `RUHARNESS_ADOPTED` at a scratch file must have the cockpit it drives
//! adopt into that file, never into the person's own.

use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn the_driver_passes_the_adoption_file_through() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = repo.join("devtools/cockpit-drive/cockpit.py");
    let work = std::env::temp_dir().join(format!(
        "harness-tui-cockpit-drive-{}-{}",
        std::process::id(),
        harness_core::hash::random_hex(4)
    ));
    std::fs::create_dir_all(&work).unwrap();
    let adopted = work.join("scratch-adopted.toml");
    // The driven program prints its environment: what the cockpit would get.
    let child = std::process::Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(&work)
        .args(["24", "200", "--", "/usr/bin/env"])
        .env("RUHARNESS_ADOPTED", &adopted)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        eprintln!("skipped: no python3 to run the driver");
        return;
    };
    let want = format!("RUHARNESS_ADOPTED={}", adopted.display());
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = String::new();
    while Instant::now() < deadline {
        seen = String::from_utf8_lossy(&std::fs::read(work.join("raw.bin")).unwrap_or_default())
            .into_owned();
        if seen.contains(&want) && seen.contains("TERM=") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&work);
    assert!(
        seen.contains(&want),
        "the driven program's environment: {seen}"
    );
}
