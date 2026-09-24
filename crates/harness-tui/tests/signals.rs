//! The cockpit's own signal path end to end (docs/TUI-DESIGN.md §4 "The
//! TUI's own signals", §7): the real `harness-tui` under a pseudo-terminal
//! (`script`), driven by keys through a hand edit whose oracle run spins —
//! then a hangup to the cockpit's process group. The spawned `harness` leads
//! its own group, so the hangup never reaches it; the cockpit must INT it,
//! and the harness must kill its sandboxed groups (the spinning C driver)
//! before dying by that SIGINT. Had the hangup reached the harness (its
//! SIGHUP handler is off under pipes), it would have died by the default
//! action and left the driver spinning.
//!
//! The `harness` binary is the workspace build next to `harness-tui`
//! (`cargo test --workspace` builds it).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn harness_bin() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_harness-tui")).with_file_name("harness");
    assert!(
        path.is_file(),
        "{} is missing: build the CLI first (`cargo build -p harness-cli`; \
         `cargo test --workspace` does)",
        path.display()
    );
    path
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "build" || name == "target" || name == ".git" || name == ".lock" {
            continue;
        }
        let (from, to) = (entry.path(), dst.join(&name));
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn harness(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(harness_bin()).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn children_of(pid: u32) -> Vec<u32> {
    let out = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn comm_of(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait_for<T>(what: &str, secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Kills whatever the test started, whatever happens.
struct Reaper(Vec<u32>);

impl Drop for Reaper {
    fn drop(&mut self) {
        for pid in &self.0 {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[test]
fn a_hangup_cancels_the_running_harness_and_its_sandboxed_group() {
    let harness_path = harness_bin();
    let tmp = std::env::temp_dir().join(format!("harness-tui-signals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let target_dir = tmp.join("zopfli");
    copy_dir(&repo().join("targets/zopfli"), &target_dir);
    let target = target_dir.canonicalize().unwrap();
    let t = target.to_str().unwrap();
    let unit = "u001-katajainen";
    let unit_dir = target.join("migration/units").join(unit);
    let _ = std::fs::remove_dir_all(unit_dir.join("traces"));
    let _ = std::fs::remove_dir_all(unit_dir.join("attempts"));
    for cmd in ["scan", "plan"] {
        let (code, out, err) = harness(&[cmd, "--target", t]);
        assert_eq!(code, 0, "{out}\n{err}");
    }
    // A green attempt through the hand-off: a crate in the executor layout.
    let migrate = [
        "migrate",
        "--allow-unsandboxed",
        unit,
        "--target",
        t,
        "--no-promote",
    ];
    let (code, out, err) = harness(&migrate);
    assert_eq!(code, 1, "{out}\n{err}");
    let traces = unit_dir.join("traces");
    let request = std::fs::read_dir(&traces)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".request.json"))
        .expect("the pending request");
    let logic = include_str!("../../harness-cli/tests/fixtures/katajainen_logic.rs");
    let ffi = include_str!("../../harness-cli/tests/fixtures/katajainen_ffi.rs");
    let text = format!(
        "src/logic.rs\n```rust\n{logic}```\nsrc/ffi.rs\n```rust\n{ffi}```\nRUHARNESS_END_OF_OUTPUT\n"
    );
    std::fs::write(
        request.to_string_lossy().replace(".request.json", ".response.json"),
        serde_json::json!({"text": text, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"})
            .to_string(),
    )
    .unwrap();
    let (code, out, err) = harness(&migrate);
    assert_eq!(code, 0, "{out}\n{err}");
    // From now on the oracle's C driver spins until the harness kills it.
    std::fs::write(
        unit_dir.join("driver.c"),
        "int main(void) { volatile unsigned long x = 0; for (;;) { x++; } }\n",
    )
    .unwrap();
    // The "editor": appends a comment to logic.rs.
    let editor = tmp.join("editor.sh");
    std::fs::write(
        &editor,
        "#!/bin/sh\necho '// edited in the cockpit' >> \"$1\"\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // The cockpit under a pty, sized so it draws.
    let tui = env!("CARGO_BIN_EXE_harness-tui");
    let inner = format!(
        "stty rows 48 cols 160; exec '{tui}' --target '{t}' --harness '{}' --allow-unsandboxed",
        harness_path.display()
    );
    let mut script = if cfg!(target_os = "macos") {
        let mut c = Command::new("script");
        c.args(["-q", "/dev/null", "/bin/sh", "-c", &inner]);
        c
    } else {
        let mut c = Command::new("script");
        c.args(["-q", "-c", &format!("/bin/sh -c \"{inner}\""), "/dev/null"]);
        c
    };
    let mut script = script
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script(1)");
    let mut reaper = Reaper(vec![script.id()]);
    let screen = Arc::new(Mutex::new(String::new()));
    {
        let screen = screen.clone();
        let mut stdout = script.stdout.take().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                screen
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });
    }
    // The screen as text: escapes stripped (ratatui moves the cursor over
    // blank cells instead of writing spaces, so matching ignores whitespace).
    let plain = || {
        let raw = screen.lock().unwrap().clone();
        let mut plain = String::new();
        let mut esc = false;
        for c in raw.chars() {
            match (esc, c) {
                (false, '\u{1b}') => esc = true,
                (true, c) if c.is_ascii_alphabetic() || c == '~' => esc = false,
                (true, _) => {}
                (false, c) => plain.push(c),
            }
        }
        plain
    };
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let saw = |needle: &str| squeeze(&plain()).contains(&squeeze(needle));
    let tail = || {
        let plain = plain();
        let n = plain.chars().count();
        plain
            .chars()
            .skip(n.saturating_sub(3000))
            .collect::<String>()
    };
    let mut keys = script.stdin.take().unwrap();
    let mut press = |bytes: &[u8]| {
        keys.write_all(bytes).unwrap();
        keys.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    wait_for("the cockpit to draw", 30, || saw("units").then_some(()));
    let tui_pid = wait_for("the cockpit process", 10, || {
        children_of(script.id()).into_iter().next()
    });
    reaper.0.push(tui_pid);
    // Rail focus, the attempt, show it, hand edit.
    press(b"\t");
    press(b"j");
    press(b"\r");
    press(b"e");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !saw("a note for the human attempt") {
        assert!(
            Instant::now() < deadline,
            "no hand-edit note prompt; the screen ended with:\n{}",
            tail()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    press(b"\r");
    wait_for("the confirmation", 10, || saw("run this?").then_some(()));
    press(b"y");
    // The override runs: the harness is the cockpit's child, the spinning
    // C driver the harness's.
    let harness_pid = wait_for("the spawned harness", 30, || {
        children_of(tui_pid)
            .into_iter()
            .find(|p| comm_of(*p).ends_with("harness"))
    });
    reaper.0.push(harness_pid);
    let drivers = wait_for("the spinning driver", 240, || {
        let d: Vec<u32> = children_of(harness_pid)
            .into_iter()
            .filter(|p| comm_of(*p).ends_with("drv_c"))
            .collect();
        (!d.is_empty()).then_some(d)
    });
    reaper.0.extend(&drivers);
    // The harness leads its own group: the hangup cannot reach it.
    let pgid = |pid: u32| {
        let out = Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(pgid(harness_pid), harness_pid.to_string());
    assert_ne!(pgid(tui_pid), pgid(harness_pid));
    // Hang up the cockpit's group (a closed terminal).
    let before = screen.lock().unwrap().len();
    assert!(Command::new("/bin/kill")
        .args(["-HUP", &format!("-{}", pgid(tui_pid))])
        .status()
        .unwrap()
        .success());
    wait_for("the cockpit to die", 10, || (!alive(tui_pid)).then_some(()));
    wait_for("the harness to die", 10, || {
        (!alive(harness_pid)).then_some(())
    });
    for d in &drivers {
        wait_for("the driver group to die", 10, || (!alive(*d)).then_some(()));
    }
    // The cockpit left the terminal usable (cursor shown, bracketed paste
    // off) and named the hand edit it kept (the override was interrupted
    // before recording it).
    wait_for("the restore sequences", 5, || {
        let after = screen.lock().unwrap()[before..].to_string();
        (after.contains("\u{1b}[?25h") && after.contains("\u{1b}[?2004l")).then_some(())
    });
    wait_for("the kept-edit notice", 5, || {
        saw("a hand edit that was not recorded is kept in").then_some(())
    });
    // The harness died by the signal (no clean release): its holder line
    // stays, naming it.
    let holder = std::fs::read_to_string(target.join("migration/.lock")).unwrap();
    assert!(
        holder.contains(&format!("\"pid\":{harness_pid}")),
        "the holder line: {holder:?}"
    );
    let _ = script.wait();
    drop(reaper);
    let _ = std::fs::remove_dir_all(&tmp);
    // The cockpit's own temp dir of the interrupted edit.
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("harness-tui-edit-{tui_pid}-")) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// The cockpit under a pty (`script`), sized so it draws, with `env` added
/// (and `VISUAL` removed): the process, what it wrote, and its keyboard.
fn cockpit(
    args: &str,
    env: &[(&str, &Path)],
) -> (
    std::process::Child,
    Arc<Mutex<String>>,
    std::process::ChildStdin,
) {
    let tui = env!("CARGO_BIN_EXE_harness-tui");
    let inner = format!("stty rows 40 cols 140; exec '{tui}' {args}");
    let mut cmd = Command::new("script");
    if cfg!(target_os = "macos") {
        cmd.args(["-q", "/dev/null", "/bin/sh", "-c", &inner]);
    } else {
        cmd.args(["-q", "-c", &format!("/bin/sh -c \"{inner}\""), "/dev/null"]);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .env_remove("VISUAL")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script(1)");
    let screen = Arc::new(Mutex::new(String::new()));
    let sink = screen.clone();
    let mut out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let keys = child.stdin.take().unwrap();
    (child, screen, keys)
}

/// `screen` as text, escapes and whitespace removed (see the first test).
fn squeezed(screen: &Mutex<String>) -> String {
    let raw = screen.lock().unwrap().clone();
    let mut plain = String::new();
    let mut esc = false;
    for c in raw.chars() {
        match (esc, c) {
            (false, '\u{1b}') => esc = true,
            (true, c) if c.is_ascii_alphabetic() || c == '~' => esc = false,
            (true, _) => {}
            (false, c) if !c.is_whitespace() => plain.push(c),
            _ => {}
        }
    }
    plain
}

/// Review PROC-4 / STATE-4 and the verification round's EDIT-SIGNALS-NEW-1:
/// an editor that saves then exits non-zero keeps the edit; a TERM to the
/// cockpit while the editor runs is forwarded to the editor, which leaves
/// its swap file — kept, never removed — and the cockpit dies by the TERM
/// only after the editor is gone, naming every kept edit.
#[test]
fn a_terminated_edit_is_never_lost() {
    let tmp = std::env::temp_dir().join(format!("harness-tui-editsig-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let target = tmp.join("case");
    copy_dir(
        &repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib"),
        &target,
    );
    let edits = tmp.join("edits");
    std::fs::create_dir_all(&edits).unwrap();
    let (counter, ready) = (tmp.join("counter"), tmp.join("ready"));
    let editor = tmp.join("editor.sh");
    std::fs::write(
        &editor,
        r#"#!/bin/sh
n=$(cat "$COUNTER" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$COUNTER"
if [ "$n" = 1 ]; then echo '// saved, then aborted' >> "$1"; exit 1; fi
trap 'echo buffer > "$(dirname "$1")/.logic.rs.swp"; exit 1' TERM
echo "$$" > "$READY"
while :; do sleep 0.1; done
"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (mut script, screen, mut keys) = cockpit(
        &format!("--target '{}' --harness /usr/bin/true", target.display()),
        &[
            ("EDITOR", &editor),
            ("COUNTER", &counter),
            ("READY", &ready),
            ("TMPDIR", &edits),
        ],
    );
    let mut reaper = Reaper(vec![script.id()]);
    let mut press = |bytes: &[u8]| {
        keys.write_all(bytes).unwrap();
        keys.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    wait_for("the cockpit to draw", 30, || {
        squeezed(&screen).contains("units").then_some(())
    });
    let tui_pid = wait_for("the cockpit process", 10, || {
        children_of(script.id()).into_iter().next()
    });
    reaper.0.push(tui_pid);
    // 1. Saved, then the editor exits 1: kept, E offers it.
    press(b"e");
    wait_for("the abort notice", 20, || {
        squeezed(&screen)
            .contains("theeditiskept—Eoffersit")
            .then_some(())
    });
    // 2. The editor runs; a TERM to the cockpit alone.
    press(b"e");
    let editor_pid: u32 = wait_for("the second editor", 20, || {
        std::fs::read_to_string(&ready).ok()?.trim().parse().ok()
    });
    reaper.0.push(editor_pid);
    assert!(Command::new("/bin/kill")
        .args(["-TERM", &tui_pid.to_string()])
        .status()
        .unwrap()
        .success());
    wait_for("the editor to end (forwarded TERM)", 10, || {
        (!alive(editor_pid)).then_some(())
    });
    wait_for("the cockpit to die", 10, || (!alive(tui_pid)).then_some(()));
    let status = script.wait().unwrap();
    let _ = status;
    // Both edits are on disk and named.
    let dirs: Vec<PathBuf> = std::fs::read_dir(&edits)
        .unwrap()
        .map(|e| e.unwrap().path().join("edit"))
        .collect();
    assert_eq!(dirs.len(), 2, "{dirs:?}");
    assert!(dirs
        .iter()
        .any(|d| std::fs::read_to_string(d.join("logic.rs"))
            .is_ok_and(|t| t.contains("// saved, then aborted"))));
    assert!(
        dirs.iter().any(|d| d.join(".logic.rs.swp").is_file()),
        "the swap file kept"
    );
    let said = squeezed(&screen);
    assert_eq!(
        said.matches("ahandeditthatwasnotrecordediskeptin").count(),
        2,
        "both named on the way out"
    );
    drop(reaper);
    let _ = std::fs::remove_dir_all(&tmp);
}
