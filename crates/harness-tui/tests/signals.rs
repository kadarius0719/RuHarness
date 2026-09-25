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

/// The arrow keys, as a terminal sends them.
const RIGHT: &[u8] = b"\x1b[C";
const DOWN: &[u8] = b"\x1b[B";

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

/// Kills whatever the test started that is still alive, whatever happens
/// (a pid that died long ago is left alone: it may have been reused).
struct Reaper(Vec<u32>);

impl Drop for Reaper {
    fn drop(&mut self) {
        for pid in self.0.iter().filter(|p| alive(**p)) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// A directory removed on drop — also when the test fails.
struct TmpDir(PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The pty's bytes as text, decoded across reads: a UTF-8 character split
/// between two reads is kept until it is whole (never a U+FFFD).
fn decode(carry: &mut Vec<u8>, chunk: &[u8]) -> String {
    carry.extend_from_slice(chunk);
    let upto = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => carry.len(),
    };
    let text = String::from_utf8_lossy(&carry[..upto]).into_owned();
    carry.drain(..upto);
    text
}

#[test]
fn a_hangup_cancels_the_running_harness_and_its_sandboxed_group() {
    let harness_path = harness_bin();
    let tmp = std::env::temp_dir().join(format!("harness-tui-signals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let _cleanup = TmpDir(tmp.clone());
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
            let mut carry = Vec::new();
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let text = decode(&mut carry, &buf[..n]);
                screen.lock().unwrap().push_str(&text);
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
    wait_for("the cockpit to draw", 30, || saw("Files").then_some(()));
    let tui_pid = wait_for("the cockpit process", 10, || {
        children_of(script.id()).into_iter().next()
    });
    reaper.0.push(tui_pid);
    // The first unit, open it, down to its crate, then its attempt; hand
    // edit.
    press(b"J");
    press(RIGHT);
    press(DOWN);
    press(DOWN);
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
    // The dialog arms (the screen is read as a terminal shows it).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !on_screen(&screen, 48, 160).contains("ready:→thenEnter,ory") {
        assert!(
            Instant::now() < deadline,
            "no armed dialog; the screen:\n{}",
            rendered(&screen.lock().unwrap(), 48, 160).join("\n")
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(on_screen(&screen, 48, 160).contains("Recordyourhandedit"));
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
    // …and they are the last word: nothing re-enabled after them.
    let after = screen.lock().unwrap()[before..].to_string();
    let last = |seq: &str| after.rfind(seq);
    assert!(
        last("\u{1b}[?25h") > last("\u{1b}[?25l"),
        "the cursor left hidden"
    );
    assert!(
        last("\u{1b}[?2004l") > last("\u{1b}[?2004h"),
        "bracketed paste left on"
    );
    assert!(
        last("\u{1b}[?1049l") > last("\u{1b}[?1049h"),
        "left in the alternate screen"
    );
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
    cockpit_then(args, env, None)
}

/// [`cockpit`]; with `stty_out`, the shell outlives the cockpit and records
/// the terminal's modes (`stty -a`) there once it is gone — the cockpit is
/// then the shell's child, not `script`'s.
fn cockpit_then(
    args: &str,
    env: &[(&str, &Path)],
    stty_out: Option<&Path>,
) -> (
    std::process::Child,
    Arc<Mutex<String>>,
    std::process::ChildStdin,
) {
    let tui = env!("CARGO_BIN_EXE_harness-tui");
    let inner = match stty_out {
        None => format!("stty rows 40 cols 140; exec '{tui}' {args}"),
        Some(out) => format!(
            "stty rows 40 cols 140; '{tui}' {args}; stty -a > '{}'",
            out.display()
        ),
    };
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
        let mut carry = Vec::new();
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            let text = decode(&mut carry, &buf[..n]);
            sink.lock().unwrap().push_str(&text);
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
    let _cleanup = TmpDir(tmp.clone());
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
        squeezed(&screen).contains("Files").then_some(())
    });
    let tui_pid = wait_for("the cockpit process", 10, || {
        children_of(script.id()).into_iter().next()
    });
    reaper.0.push(tui_pid);
    // The unit's crate.
    press(b"J");
    press(RIGHT);
    press(DOWN);
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

/// A scratch copy of the tractor case, removed on drop.
struct Case(PathBuf);

impl Case {
    fn new(tag: &str) -> Case {
        let dir = std::env::temp_dir().join(format!("harness-tui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(
            &repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib"),
            &dir.join("case"),
        );
        Case(dir)
    }

    fn target(&self) -> PathBuf {
        self.0.join("case").canonicalize().unwrap()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// docs/COCKPIT-WRAPPER-DESIGN.md §13: the keyboard end to end — Enter
/// opens the project's menu focused on its Next step (Scan, the C changed),
/// Enter opens the dialog, and once it is ready a move and Enter run it; the
/// activity panel says "Done" and the facts are rewritten.
#[test]
fn the_keyboard_scans_the_project_end_to_end() {
    let case = Case::new("e2e-scan");
    let target = case.target();
    let lib = target.join("test_case/src/lib.c");
    let c = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, format!("{c}/* edited outside */\n")).unwrap();
    let facts = target.join("migration/facts.jsonl");
    let before = std::fs::read_to_string(&facts).unwrap();
    let (mut script, screen, mut keys) = cockpit(
        &format!(
            "--target '{}' --harness '{}'",
            target.display(),
            harness_bin().display()
        ),
        &[],
    );
    let mut reaper = Reaper(vec![script.id()]);
    // What the screen shows now (the cockpit's pty is 40 × 140).
    let saw = |needle: &str| {
        on_screen(&screen, 40, 140).contains(&needle.split_whitespace().collect::<String>())
    };
    let mut press = |bytes: &[u8]| {
        keys.write_all(bytes).unwrap();
        keys.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    wait_for("the cockpit to draw", 30, || {
        saw("Next step: 1 file changed").then_some(())
    });
    let tui_pid = wait_for("the cockpit process", 10, || {
        children_of(script.id()).into_iter().next()
    });
    reaper.0.push(tui_pid);
    press(b"\r");
    wait_for("the menu", 10, || {
        saw("Enter choose · Esc close").then_some(())
    });
    press(b"\r");
    wait_for("the armed Scan dialog", 10, || {
        (saw("Scan the project?") && saw("ready: → then Enter, or y")).then_some(())
    });
    press(RIGHT);
    press(b"\r");
    wait_for("the scan to finish", 60, || {
        saw("Last: Scan the project — Done").then_some(())
    });
    let after = std::fs::read_to_string(&facts).unwrap();
    assert_ne!(before, after, "the facts are rewritten");
    press(b"q");
    wait_for("the cockpit to quit", 10, || {
        (!alive(tui_pid)).then_some(())
    });
    let status = script.wait().unwrap();
    assert!(status.success(), "{status:?}");
    drop(reaper);
}

/// SAFE-10: a TERM that lands right after the editor exits — while the
/// cockpit stages the edit and takes the terminal back — restores the shell
/// and names the kept edit.
#[test]
fn a_term_right_after_the_editor_restores_and_names_the_edit() {
    let case = Case::new("e2e-term");
    let target = case.target();
    let edits = case.0.join("edits");
    std::fs::create_dir_all(&edits).unwrap();
    let editor = case.0.join("editor.sh");
    // Saves, then asks for a TERM to its parent (the cockpit) a moment after
    // it has exited.
    std::fs::write(
        &editor,
        "#!/bin/sh\necho '// edited' >> \"$1\"\n(sleep 0.05; kill -TERM $PPID) &\nexit 0\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let stty = case.0.join("stty.txt");
    let (mut script, screen, mut keys) = cockpit_then(
        &format!("--target '{}' --harness /usr/bin/true", target.display()),
        &[("EDITOR", &editor), ("TMPDIR", &edits)],
        Some(&stty),
    );
    let mut reaper = Reaper(vec![script.id()]);
    let mut press = |bytes: &[u8]| {
        keys.write_all(bytes).unwrap();
        keys.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    wait_for("the cockpit to draw", 30, || {
        squeezed(&screen).contains("Files").then_some(())
    });
    let tui_pid = wait_for("the cockpit process", 10, || {
        let shell = children_of(script.id()).into_iter().next()?;
        children_of(shell).into_iter().next()
    });
    reaper.0.push(tui_pid);
    press(b"J");
    press(RIGHT);
    press(DOWN);
    let before = screen.lock().unwrap().len();
    press(b"e");
    wait_for("the cockpit to die by the TERM", 10, || {
        (!alive(tui_pid)).then_some(())
    });
    let _ = script.wait();
    wait_for("the kept-edit notice", 5, || {
        squeezed(&screen)
            .contains("ahandeditthatwasnotrecordediskeptin")
            .then_some(())
    });
    let after = screen.lock().unwrap()[before..].to_string();
    // The last word on the terminal is the restore: main screen, cursor
    // shown, bracketed paste off — after any re-enable.
    let last = |seq: &str| after.rfind(seq);
    assert!(
        last("\u{1b}[?1049l") > last("\u{1b}[?1049h"),
        "left in the alternate screen"
    );
    assert!(
        last("\u{1b}[?25h") > last("\u{1b}[?25l"),
        "the cursor left hidden"
    );
    assert!(
        last("\u{1b}[?2004l") > last("\u{1b}[?2004h"),
        "bracketed paste left on"
    );
    // Cooked mode again: the shell after it reads a canonical, echoing tty.
    let modes = wait_for("the shell's stty -a", 10, || {
        std::fs::read_to_string(&stty).ok()
    });
    let words: Vec<&str> = modes.split_whitespace().collect();
    assert!(
        words.contains(&"icanon") && words.contains(&"echo"),
        "{modes}"
    );
    let kept: Vec<PathBuf> = std::fs::read_dir(&edits)
        .unwrap()
        .map(|e| e.unwrap().path().join("edit/logic.rs"))
        .collect();
    assert!(
        kept.iter()
            .any(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains("// edited"))),
        "{kept:?}"
    );
    drop(reaper);
}

/// What a terminal would show after `bytes`: a small VT interpreter for the
/// sequences the cockpit's backend writes (cursor moves, erases, text; the
/// colours and modes are ignored), so a test reads the SCREEN — ratatui
/// redraws only the cells that changed, so the byte stream alone does not
/// hold the text a user sees.
fn rendered(bytes: &str, rows: usize, cols: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut grid = vec![vec![' '; cols]; rows];
    let (mut r, mut c) = (0usize, 0usize);
    let mut chars = bytes.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    let mut fin = ' ';
                    for p in chars.by_ref() {
                        if p.is_ascii_alphabetic() || p == '~' || p == '@' {
                            fin = p;
                            break;
                        }
                        params.push(p);
                    }
                    let nums: Vec<usize> = params
                        .trim_start_matches('?')
                        .split(';')
                        .map(|n| n.parse().unwrap_or(0))
                        .collect();
                    let n =
                        |i: usize, d: usize| nums.get(i).copied().filter(|v| *v > 0).unwrap_or(d);
                    match fin {
                        'H' | 'f' => {
                            r = n(0, 1).saturating_sub(1).min(rows - 1);
                            c = n(1, 1).saturating_sub(1).min(cols - 1);
                        }
                        'A' => r = r.saturating_sub(n(0, 1)),
                        'B' => r = (r + n(0, 1)).min(rows - 1),
                        'C' => c = (c + n(0, 1)).min(cols - 1),
                        'D' => c = c.saturating_sub(n(0, 1)),
                        'G' => c = n(0, 1).saturating_sub(1).min(cols - 1),
                        'J' if nums.first() == Some(&2) || nums.first() == Some(&3) => {
                            grid = vec![vec![' '; cols]; rows];
                        }
                        'J' => {
                            for cell in grid[r].iter_mut().skip(c) {
                                *cell = ' ';
                            }
                            for row in grid.iter_mut().skip(r + 1) {
                                *row = vec![' '; cols];
                            }
                        }
                        'K' => {
                            for cell in grid[r].iter_mut().skip(c) {
                                *cell = ' ';
                            }
                        }
                        _ => {}
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => c = 0,
            '\n' => r = (r + 1).min(rows - 1),
            ch if ch.is_control() => {}
            ch => {
                let w = ch.width().unwrap_or(0);
                if w == 0 {
                    continue;
                }
                if c < cols {
                    grid[r][c] = ch;
                    if w == 2 && c + 1 < cols {
                        grid[r][c + 1] = ' ';
                    }
                }
                c = (c + w).min(cols);
            }
        }
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

/// The screen now (see [`rendered`]), whitespace removed.
fn on_screen(screen: &Mutex<String>, rows: usize, cols: usize) -> String {
    rendered(&screen.lock().unwrap(), rows, cols)
        .concat()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}
