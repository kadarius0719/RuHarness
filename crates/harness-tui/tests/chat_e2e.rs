//! The chat end to end (docs/CHAT-PANE-DESIGN.md §9 "End to end"): the real
//! `harness-tui` under a pseudo-terminal (`script`), its chat runtime a fake
//! `claude` replaying a recording (`tests/fixtures/chat/replay.sh`), its acts
//! a fake `harness`:
//! - send → a read → a Migrate request → Review → Run → the outcome on the
//!   fake's stdin;
//! - every way out ends the chat's process group and removes its directory:
//!   quit, TERM, HUP, the hand edit's editor dying by TERM, New chat twice
//!   within 3 s, a runtime that stops reading its stdin, a grandchild in its
//!   group, a runtime stopped reading the terminal, a panic;
//! - a stale chat directory of a dead cockpit is swept when a chat starts.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RIGHT: &[u8] = b"\x1b[C";
const DOWN: &[u8] = b"\x1b[B";
const TAB: &[u8] = b"\t";
const ENTER: &[u8] = b"\r";
const CTRL_C: &[u8] = b"\x03";
const CTRL_N: &[u8] = b"\x0e";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chat")
}

fn alive(pid: u32) -> bool {
    // A zombie counts as gone: `ps` says Z.
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
    !stat.is_empty() && !stat.starts_with('Z')
}

fn wait_for<T>(what: &str, secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    // Every child inherits the test process's own adoption file, never the
    // person's (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adoption_file();
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
    // A copied ledger is adopted for this test process, as the person's
    // `--adopt` would (docs/PROJECT-MAP-DESIGN.md §3.7).
    if dst.join("migration").is_dir() {
        harness_core::adopt::testing::adopt(dst);
    }
}

fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Kills what the test started, whatever happens (the chat's processes are
/// the cockpit's to end — this only cleans up after a failure).
struct Cleanup {
    dir: PathBuf,
    pids: Vec<u32>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for pid in &self.pids {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .stderr(Stdio::null())
                .status();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One cockpit under a pty, with a fake chat runtime.
struct Run {
    tmp: PathBuf,
    log: PathBuf,
    script: Child,
    screen: Arc<Mutex<String>>,
    keys: ChildStdin,
    cleanup: Cleanup,
}

/// A scratch copy of zopfli whose u001 is planned again (its crate, verdicts
/// and attempts gone), for the chat's Migrate.
fn zopfli(tmp: &Path) -> PathBuf {
    let target = tmp.join("zopfli");
    copy_dir(&repo().join("targets/zopfli"), &target);
    let unit = target.join("migration/units/u001-katajainen");
    for d in ["attempts", "traces", "katajainen_rs"] {
        let _ = std::fs::remove_dir_all(unit.join(d));
    }
    for f in [
        "oracle-latest.json",
        "oracle-latest.md",
        "oracle-last-green.json",
    ] {
        let _ = std::fs::remove_file(unit.join(f));
    }
    let plan = target.join("migration/plan.toml");
    let text = std::fs::read_to_string(&plan).unwrap().replacen(
        "id = \"u001-katajainen\"\nstatus = \"verified\"",
        "id = \"u001-katajainen\"\nstatus = \"pending\"",
        1,
    );
    std::fs::write(&plan, text).unwrap();
    target.canonicalize().unwrap()
}

/// A hand-written recording (Python's separators, as the fake reads them):
/// initialize, the message, an `init`, the turn's end.
fn idle_recording(path: &Path) {
    let tools = [
        "mcp__harness__harness_status",
        "mcp__harness__harness_unit",
        "mcp__harness__harness_request",
        "mcp__harness__harness_migrate",
        "mcp__harness__harness_steer",
        "mcp__harness__harness_retry",
        "mcp__harness__harness_answer",
    ]
    .map(|t| format!("\"{t}\""))
    .join(", ");
    let lines = [
        r#"{"dir": "in", "msg": {"type": "control_request", "request_id": "r0", "request": {"subtype": "initialize", "hooks": null}}, "t": 0.0}"#.to_string(),
        r#"{"dir": "in", "msg": {"type": "user", "uuid": "old-uuid", "message": {"role": "user", "content": []}}, "t": 0.0}"#.to_string(),
        format!(
            r#"{{"dir": "out", "msg": {{"type": "system", "subtype": "init", "model": "claude-haiku-4-5-20251001", "permissionMode": "default", "claude_code_version": "2.1.274", "apiKeySource": "none", "tools": [{tools}], "mcp_servers": [{{"name": "harness", "status": "connected"}}]}}, "t": 0.0}}"#
        ),
        r#"{"dir": "out", "msg": {"type": "assistant", "message": {"id": "m1", "model": "claude-haiku-4-5-20251001", "content": [{"type": "text", "text": "HELLO-FROM-THE-FAKE"}]}}, "t": 0.0}"#.to_string(),
        r#"{"dir": "out", "msg": {"type": "result", "subtype": "success", "is_error": false, "total_cost_usd": 0.001}, "t": 0.0}"#.to_string(),
    ];
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
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

impl Run {
    /// Start a cockpit over `target` (else a copy of zopfli) whose `claude`
    /// runs `extra` then replays `recording` (else [`idle_recording`]).
    fn start(tag: &str, recording: Option<&Path>, extra: &str, env: &[(&str, &str)]) -> Run {
        Run::start_on(tag, recording, extra, env, false)
    }

    /// [`Run::start`] over a copy of the tractor case with a verified unit
    /// (`scalefactors`), or of zopfli.
    fn start_on(
        tag: &str,
        recording: Option<&Path>,
        extra: &str,
        env: &[(&str, &str)],
        scalefactors: bool,
    ) -> Run {
        let tmp = std::env::temp_dir().join(format!("hc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = tmp.canonicalize().unwrap();
        let cleanup = Cleanup {
            dir: tmp.clone(),
            pids: Vec::new(),
        };
        let target = if scalefactors {
            let t = tmp.join("case");
            copy_dir(
                &repo()
                    .join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib"),
                &t,
            );
            t.canonicalize().unwrap()
        } else {
            zopfli(&tmp)
        };
        let log = tmp.join("log");
        std::fs::create_dir_all(&log).unwrap();
        let rec = match recording {
            Some(r) => r.to_path_buf(),
            None => {
                let p = tmp.join("idle.jsonl");
                idle_recording(&p);
                p
            }
        };
        let claude = tmp.join("claude");
        executable(
            &claude,
            &format!(
                "#!/bin/sh\necho $$ >> '{log}/pids'\npwd > '{log}/cwd'\n{extra}\nexec /bin/sh '{replay}' '{rec}' '{log}' \"$@\"\n",
                log = log.display(),
                replay = fixtures().join("replay.sh").display(),
                rec = rec.display(),
            ),
        );
        // The fake harness: every act a green attempt.
        let harness = tmp.join("harness");
        executable(
            &harness,
            "#!/bin/sh\necho '{\"k\":\"attempt\",\"unit\":\"u001-katajainen\",\"id\":\"a-0123456789ab\",\"outcome\":\"green\",\"promotion\":\"not-promoted\"}'\necho '{\"k\":\"result\",\"exit\":0}'\nexit 0\n",
        );
        let mcp = PathBuf::from(env!("CARGO_BIN_EXE_harness-tui")).with_file_name("harness-mcp");
        let tui = env!("CARGO_BIN_EXE_harness-tui");
        let inner =
            format!(
            "stty rows 40 cols 140; exec '{tui}' --target '{}' --harness '{}' --chat-runtime '{}' \
             --harness-mcp '{}'",
            target.display(),
            harness.display(),
            claude.display(),
            if mcp.is_file() { mcp } else { PathBuf::from("/usr/bin/false") }.display(),
        );
        let mut cmd = Command::new("script");
        if cfg!(target_os = "macos") {
            cmd.args(["-q", "/dev/null", "/bin/sh", "-c", &inner]);
        } else {
            cmd.args(["-q", "-c", &format!("/bin/sh -c \"{inner}\""), "/dev/null"]);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut script = cmd
            .env("TMPDIR", &tmp)
            .env_remove("VISUAL")
            .env_remove("CLAUDECODE")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("script(1)");
        let screen = Arc::new(Mutex::new(String::new()));
        let sink = screen.clone();
        let mut out = script.stdout.take().unwrap();
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
        let keys = script.stdin.take().unwrap();
        let mut run = Run {
            tmp,
            log,
            script,
            screen,
            keys,
            cleanup,
        };
        run.cleanup.pids.push(run.script.id());
        run.wait_screen("the cockpit", "Files");
        run
    }

    fn press(&mut self, bytes: &[u8]) {
        self.keys.write_all(bytes).unwrap();
        self.keys.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.keys.write_all(c.to_string().as_bytes()).unwrap();
            self.keys.flush().unwrap();
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// What was written, escapes and whitespace removed — ratatui redraws
    /// only the cells that changed, so this may miss a letter already on
    /// screen: [`Self::screen`] reads the rendered screen too (review PRO-7).
    fn squeezed(&self) -> String {
        let raw = self.screen.lock().unwrap().clone();
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

    /// The screen as a terminal shows it now, whitespace removed.
    fn screen(&self) -> String {
        rendered(&self.screen.lock().unwrap(), 40, 140)
            .concat()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// On the screen now, or ever written whole.
    fn shows(&self, needle: &str) -> bool {
        let needle: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
        self.screen().contains(&needle) || self.squeezed().contains(&needle)
    }

    fn wait_screen(&self, what: &str, needle: &str) {
        let needle: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.shows(&needle) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the screen's tail: {}",
                self.squeezed()
                    .chars()
                    .rev()
                    .take(1500)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The cockpit's pid (the shell `script` runs execs it).
    fn tui_pid(&self) -> u32 {
        wait_for("the cockpit's pid", 10, || {
            let out = Command::new("pgrep")
                .args(["-P", &self.script.id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|l| l.trim().parse().ok())
        })
    }

    /// Focus the chat, send `text`, wait for the fake to start: its pid and
    /// its directory.
    fn chat(&mut self, text: &str) -> (u32, PathBuf) {
        let before = self.pids().len();
        if !self.shows("›type") {
            self.press(TAB);
            self.press(TAB);
        }
        self.type_text(text);
        self.press(ENTER);
        let pid = wait_for("the chat's runtime", 20, || {
            let pids = self.pids();
            (pids.len() > before).then(|| *pids.last().unwrap())
        });
        self.cleanup.pids.push(pid);
        let dir = wait_for("the chat's directory", 20, || {
            std::fs::read_to_string(self.log.join("cwd"))
                .ok()
                .map(|c| PathBuf::from(c.trim()))
                .filter(|d| d.exists())
        });
        (pid, dir)
    }

    fn pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.log.join("pids"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    /// Answer the open dialog's second button: armed after 300 ms quiet,
    /// then a move and Enter (the chat-dialog rules).
    fn confirm(&mut self) {
        std::thread::sleep(Duration::from_millis(700));
        self.press(RIGHT);
        self.press(ENTER);
    }

    fn ended(&mut self) {
        let _ = wait_for("the cockpit's end", 20, || {
            self.script.try_wait().ok().flatten()
        });
    }
}

/// The chat's processes and directory are gone.
fn gone(pids: &[u32], dirs: &[PathBuf]) {
    for &p in pids {
        wait_for(&format!("process {p} to end"), 10, || {
            (!alive(p)).then_some(())
        });
    }
    for d in dirs {
        wait_for(&format!("{} to go", d.display()), 10, || {
            (!d.exists()).then_some(())
        });
    }
}

/// §9: send → a read → a Migrate request → Review → Run (the fake harness)
/// → the outcome on the fake's stdin.
#[test]
fn a_request_is_reviewed_run_and_answered() {
    let mut r = Run::start("flow", Some(&fixtures().join("decline.jsonl")), "", &[]);
    let (pid, dir) = r.chat("Please migrate this unit.");
    r.wait_screen("the request", "Asks: Migrate u001-katajainen");
    std::thread::sleep(Duration::from_millis(1100));
    r.press(ENTER);
    r.wait_screen("its dialog", "The chat asks: Migrate u001-katajainen?");
    r.confirm();
    let stdin = wait_for("the outcome on the runtime's stdin", 30, || {
        let s = std::fs::read_to_string(r.log.join("stdin")).ok()?;
        s.contains("Ran by the cockpit after the person confirmed it")
            .then_some(s)
    });
    assert!(stdin.contains(r#"\"outcome\":\"green\""#), "{stdin}");
    assert!(
        stdin.contains(r#""behavior":"allow""#),
        "the read was allowed: {stdin}"
    );
    r.wait_screen("the model's reply", "The person declined");
    r.wait_screen("the turn's end", "of plan usage");
    let argv = std::fs::read_to_string(r.log.join("argv")).unwrap();
    for flag in [
        "--permission-prompt-tool",
        "--strict-mcp-config",
        "--no-session-persistence",
    ] {
        assert!(argv.lines().any(|l| l == flag), "{flag}: {argv}");
    }
    // Quit asks (a conversation exists).
    r.press(CTRL_C);
    r.wait_screen("the quit dialog", "The chat's conversation is not kept");
    r.confirm();
    r.ended();
    gone(&[pid], &[dir]);
}

/// §1.4: quit, TERM and HUP each end the chat's group — a grandchild
/// included — and remove its directory.
#[test]
fn quit_term_and_hup_end_the_chats_group() {
    for way in ["quit", "TERM", "HUP"] {
        let mut r = Run::start(
            &format!("way-{way}"),
            None,
            "export REPLAY_GRANDCHILD=1",
            &[],
        );
        let (pid, dir) = r.chat("hi");
        r.wait_screen("the reply", "HELLO-FROM-THE-FAKE");
        let grandchild: u32 = wait_for("the grandchild", 10, || {
            std::fs::read_to_string(r.log.join("grandchild"))
                .ok()?
                .trim()
                .parse()
                .ok()
        });
        r.cleanup.pids.push(grandchild);
        assert!(alive(grandchild));
        r.wait_screen("the turn's end", "of plan usage");
        match way {
            "quit" => {
                r.press(CTRL_C);
                r.wait_screen("the quit dialog", "conversation is not kept");
                r.confirm();
            }
            sig => {
                let tui = r.tui_pid();
                assert!(Command::new("/bin/kill")
                    .args([format!("-{sig}"), tui.to_string()])
                    .status()
                    .unwrap()
                    .success());
            }
        }
        r.ended();
        gone(&[pid, grandchild], &[dir]);
        drop(r);
    }
}

/// §1.2: New chat twice within 3 s — the older chat still ending is killed
/// first; every one ends, every directory goes.
#[test]
fn new_chat_twice_ends_both() {
    let mut r = Run::start("new-twice", None, "", &[]);
    let (p1, d1) = r.chat("one");
    r.wait_screen("the first reply", "HELLO-FROM-THE-FAKE");
    r.press(CTRL_N);
    r.wait_screen("the New chat dialog", "Start a new chat?");
    r.confirm();
    let (p2, d2) = r.chat("two");
    assert_ne!(p1, p2);
    r.press(CTRL_N);
    r.confirm();
    gone(&[p1], &[d1]);
    r.press(CTRL_C);
    r.confirm();
    r.ended();
    gone(&[p2], &[d2]);
}

/// §1.2, §1.4: a runtime that stops reading its stdin (its turn never
/// ends: the first Ctrl-C Stops, the second — a Stop on its way — asks to
/// quit) and one stopped reading the terminal never hold up the quit: TERM
/// after 1.5 s, KILL 300 ms later.
#[test]
fn a_deaf_or_stopped_runtime_is_ended_on_quit() {
    for (tag, extra) in [
        ("deaf", "exec /bin/sh -c 'read a; read b; sleep 300'"),
        ("tty", "export REPLAY_TTY=1"),
    ] {
        let mut r = Run::start(tag, None, extra, &[]);
        let (pid, dir) = r.chat("hi");
        if tag == "tty" {
            r.wait_screen("the turn's end", "of plan usage");
            // Stopped (SIGTTIN) reading the terminal.
            wait_for("the runtime stopped", 10, || {
                let out = Command::new("ps")
                    .args(["-o", "stat=", "-p", &pid.to_string()])
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .starts_with('T')
                    .then_some(())
            });
        } else {
            std::thread::sleep(Duration::from_millis(500));
            r.press(CTRL_C);
            r.wait_screen("the Stop", "stopping");
        }
        r.press(CTRL_C);
        r.wait_screen("the quit dialog", "conversation is not kept");
        r.confirm();
        let t0 = Instant::now();
        r.ended();
        gone(&[pid], &[dir]);
        assert!(
            t0.elapsed() < Duration::from_secs(6),
            "{tag}: {:?}",
            t0.elapsed()
        );
    }
}

/// §1.4: a panic ends the chat's group (the debug-only trigger).
#[test]
fn a_panic_ends_the_chats_group() {
    let mut r = Run::start(
        "panic",
        None,
        "",
        &[("HARNESS_TUI_TEST_PANIC", "chat-send")],
    );
    let (pid, dir) = r.chat("hi");
    r.wait_screen("the reply", "HELLO-FROM-THE-FAKE");
    // A second message while the chat runs: the trigger.
    r.type_text("again");
    r.press(ENTER);
    r.ended();
    gone(&[pid], &[dir]);
}

/// §1.4: the hand edit's editor dying by TERM takes the chat with it —
/// `finish_edit` ends through the signal thread's own routine.
#[test]
fn the_editors_term_ends_the_chat_too() {
    let tmp_editor = std::env::temp_dir().join(format!("hc-editor-{}.sh", std::process::id()));
    executable(&tmp_editor, "#!/bin/sh\necho EDITOR-RUNS\nsleep 60\n");
    let mut r = Run::start_on(
        "editor",
        None,
        "",
        &[("EDITOR", tmp_editor.to_str().unwrap())],
        true,
    );
    let (pid, dir) = r.chat("hi");
    r.wait_screen("the reply", "HELLO-FROM-THE-FAKE");
    // To the tree (a navigation key ends the typing guard), the unit, its
    // crate, and a hand edit of it.
    r.press(TAB);
    r.press(DOWN);
    r.press(b"J");
    r.press(RIGHT);
    r.press(DOWN);
    r.press(b"e");
    wait_for("the editor", 10, || r.shows("EDITOR-RUNS").then_some(()));
    let tui = r.tui_pid();
    assert!(Command::new("/bin/kill")
        .args(["-TERM", &tui.to_string()])
        .status()
        .unwrap()
        .success());
    r.ended();
    gone(&[pid], &[dir]);
    let _ = std::fs::remove_file(&tmp_editor);
}

/// §1.1, §R5: a chat directory a dead cockpit left is swept when a chat
/// starts; a live pid's and one of another mode are kept.
#[test]
fn a_stale_chat_directory_is_swept() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let mut r = Run::start("sweep", None, "", &[]);
    let mut c = Command::new("/usr/bin/true").spawn().unwrap();
    let dead = c.id();
    c.wait().unwrap();
    let make = |name: String, mode: u32| {
        let p = r.tmp.join(name);
        std::fs::DirBuilder::new().mode(mode).create(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    };
    let stale = make(format!("harness-tui-chat-{dead}-0123abcd"), 0o700);
    let open = make(format!("harness-tui-chat-{dead}-0123abce"), 0o755);
    let live = make(
        format!("harness-tui-chat-{}-0123abcf", std::process::id()),
        0o700,
    );
    let (pid, dir) = r.chat("hi");
    wait_for("the sweep", 10, || (!stale.exists()).then_some(()));
    assert!(open.exists() && live.exists());
    r.wait_screen("the turn's end", "of plan usage");
    r.press(CTRL_C);
    r.wait_screen("the quit dialog", "conversation is not kept");
    r.confirm();
    r.ended();
    gone(&[pid], &[dir]);
}
