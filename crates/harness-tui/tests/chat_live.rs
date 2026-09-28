//! The chat's live test (docs/CHAT-PANE-DESIGN.md §9 "Live test"): ignored
//! unless `RUHARNESS_LIVE_CHAT=1`, run by hand at each build's end. The real
//! cockpit under a pty, your own `claude` on PATH (on haiku: a few US
//! cents of plan usage a run), the real `harness` next to the cockpit,
//! sandboxed, on a zopfli copy whose u001 is planned again:
//! - a Migrate asked in chat, confirmed; its hand-off read and answered in
//!   chat and continued under the permission; GREEN; the outcome told;
//! - the `init` checks passed (the title names the model), the runtime's
//!   environment is the design's (§1.1), its inbox socket in its own
//!   directory and none added to the shared listing;
//! - quit ends it within the design's bounds; an unknown `--chat-model` is
//!   said in words.
//!
//! `RUHARNESS_LIVE_CHAT_HOST=plain` runs it as from a plain terminal (a
//! minimal environment, no Claude Code session variables); otherwise it
//! inherits this process's environment (from inside a Claude Code session,
//! `CLAUDECODE` is set). Run both.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn live() -> bool {
    std::env::var("RUHARNESS_LIVE_CHAT").as_deref() == Ok("1")
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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

struct Live {
    script: std::process::Child,
    screen: Arc<Mutex<String>>,
    keys: std::process::ChildStdin,
    tmp: PathBuf,
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.script.kill();
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
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

impl Live {
    fn start(tag: &str, extra_args: &str) -> Live {
        let tmp = std::env::temp_dir().join(format!("hl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = tmp.canonicalize().unwrap();
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
        let tui = PathBuf::from(env!("CARGO_BIN_EXE_harness-tui"));
        let harness = tui.with_file_name("harness");
        let mcp = tui.with_file_name("harness-mcp");
        assert!(
            harness.is_file() && mcp.is_file(),
            "build the workspace first"
        );
        // Your own claude, wrapped: its environment, argv and pid recorded
        // first (macOS `ps` does not show another process's environment).
        let real = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("claude"))
            .find(|p| p.is_file())
            .expect("claude on PATH");
        let wrapper = tmp.join("claude");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nenv > '{t}/runtime-env'\necho $$ > '{t}/runtime-pid'\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{t}/runtime-argv'\nexec '{}' \"$@\"\n",
                real.display(),
                t = tmp.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let inner = format!(
            "stty rows 45 cols 150; exec '{}' --target '{}' --harness '{}' --harness-mcp '{}' \
             --chat-runtime '{}' {extra_args}",
            tui.display(),
            target.display(),
            harness.display(),
            mcp.display(),
            wrapper.display()
        );
        let mut cmd = Command::new("script");
        cmd.args(["-q", "/dev/null", "/bin/sh", "-c", &inner]);
        if std::env::var("RUHARNESS_LIVE_CHAT_HOST").as_deref() == Ok("plain") {
            cmd.env_clear();
            for k in ["HOME", "PATH", "USER", "LOGNAME", "SHELL", "LANG"] {
                if let Some(v) = std::env::var_os(k) {
                    cmd.env(k, v);
                }
            }
        }
        let mut script = cmd
            .env("TERM", "xterm-256color")
            .env("TMPDIR", &tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
        Live {
            script,
            screen,
            keys,
            tmp,
        }
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
        rendered(&self.screen.lock().unwrap(), 45, 150)
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

    fn wait(&self, what: &str, needle: &str, secs: u64) {
        let needle: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
        let deadline = Instant::now() + Duration::from_secs(secs);
        while !self.shows(&needle) {
            if Instant::now() >= deadline {
                let s = self.squeezed();
                let tail: String = s
                    .chars()
                    .skip(s.chars().count().saturating_sub(3000))
                    .collect();
                panic!("timed out waiting for {what}: …{tail}");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn confirm(&mut self) {
        std::thread::sleep(Duration::from_millis(800));
        self.press(b"\x1b[C");
        self.press(b"\r");
    }

    /// The chat's runtime's pid (the wrapper recorded it, then exec'd).
    fn runtime_pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.tmp.join("runtime-pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}

fn cc_socks() -> Vec<String> {
    std::fs::read_dir("/tmp/cc-socks")
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "live: RUHARNESS_LIVE_CHAT=1 (a real model call through your own claude)"]
fn a_live_migration_through_the_chat() {
    if !live() {
        return;
    }
    let socks_before = cc_socks();
    let mut l = Live::start("round", "--chat-model haiku");
    l.wait("the cockpit", "Files", 30);
    l.press(b"\t");
    l.press(b"\t");
    l.type_text("Please migrate u001-katajainen.");
    l.press(b"\r");
    l.wait("the model's init", "haiku", 60);
    let pid = (0..50)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(100));
            l.runtime_pid()
        })
        .expect("the runtime");
    // Its environment (§1.1): no Claude Code session variable of a host.
    let env = std::fs::read_to_string(l.tmp.join("runtime-env")).unwrap();
    eprintln!(
        "the runtime's environment: {:?}",
        env.lines()
            .map(|l| l.split('=').next().unwrap_or_default())
            .collect::<Vec<_>>()
    );
    for host in [
        "CLAUDECODE=",
        "CLAUDE_CODE_SESSION_ID=",
        "MCP_CONNECTION_NONBLOCKING=",
        "CLAUDE_CODE_MESSAGING_SOCKET=",
        "CLAUDE_CODE_ENTRYPOINT=",
    ] {
        assert!(
            !env.lines().any(|l| l.starts_with(host)),
            "{host} reached the runtime"
        );
    }
    assert!(env.lines().any(|l| l == "TERM=dumb"), "{env}");
    // Its inbox socket is in its own directory.
    let argv = std::fs::read_to_string(l.tmp.join("runtime-argv")).unwrap();
    let args: Vec<&str> = argv.lines().collect();
    let socket = args
        .iter()
        .position(|a| *a == "--messaging-socket-path")
        .map(|i| PathBuf::from(args[i + 1]))
        .expect("the socket's path");
    assert!(args.contains(&"--no-session-persistence"));
    assert!(!args.contains(&"--bare"));
    assert!(
        socket.exists(),
        "the inbox socket is there while the chat runs"
    );
    assert!(socket.file_name().is_some_and(|n| n == "inbox.sock"));
    assert!(
        socket
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("harness-tui-chat-"),
        "{}",
        socket.display()
    );
    l.wait("the request", "Asks: Migrate u001-katajainen", 120);
    std::thread::sleep(Duration::from_millis(1100));
    l.press(b"\r");
    l.wait("its dialog", "The chat asks: Migrate u001-katajainen?", 20);
    l.confirm();
    l.wait("the hand-off", "awaiting the chat's answer", 180);
    l.wait("the continuation", "continued, as you agreed", 600);
    l.wait("the verdict", "GREEN", 600);
    l.wait("the turn's end", "of plan usage", 300);
    let added: Vec<String> = cc_socks()
        .into_iter()
        .filter(|s| !socks_before.contains(s))
        .collect();
    eprintln!(
        "new entries in /tmp/cc-socks during the run (other sessions may add some): {added:?}"
    );
    // Quit, timed.
    let t0 = Instant::now();
    l.press(b"\x03");
    l.wait("the quit dialog", "conversation is not kept", 10);
    l.confirm();
    let _ = l.script.wait();
    assert!(t0.elapsed() < Duration::from_secs(8), "{:?}", t0.elapsed());
    std::thread::sleep(Duration::from_millis(300));
    let gone = Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true);
    assert!(gone, "the runtime outlived the cockpit");
    assert!(
        !socket.parent().unwrap().exists(),
        "the chat's directory is removed"
    );
}

#[test]
#[ignore = "live: RUHARNESS_LIVE_CHAT=1"]
fn an_unknown_chat_model_is_said_in_words() {
    if !live() {
        return;
    }
    let mut l = Live::start("model", "--chat-model no-such-model-xyz");
    l.wait("the cockpit", "Files", 30);
    l.press(b"\t");
    l.press(b"\t");
    l.type_text("hi");
    l.press(b"\r");
    l.wait("the words", "unknown model no-such-model-xyz", 90);
}

/// A harness server that fails to start ends the chat, in words (§1.2).
#[test]
#[ignore = "live: RUHARNESS_LIVE_CHAT=1"]
fn a_failed_harness_server_ends_the_chat() {
    if !live() {
        return;
    }
    let mut l = Live::start("failed", "--chat-model haiku --harness-mcp /usr/bin/false");
    l.wait("the cockpit", "Files", 30);
    l.press(b"\t");
    l.press(b"\t");
    l.type_text("hi");
    l.press(b"\r");
    l.wait("the words", "the harness tools did not start", 90);
}

/// TERM to the cockpit mid-turn: the runtime and harness-mcp gone, the
/// chat's directory removed (§1.4, §9); New chat before it ends the older
/// chat.
#[test]
#[ignore = "live: RUHARNESS_LIVE_CHAT=1"]
fn new_chat_then_term_mid_turn() {
    if !live() {
        return;
    }
    let mut l = Live::start("term", "--chat-model haiku");
    l.wait("the cockpit", "Files", 30);
    l.press(b"\t");
    l.press(b"\t");
    l.type_text("Say hello in one short line.");
    l.press(b"\r");
    l.wait("the reply's end", "of plan usage", 120);
    let first = l.runtime_pid().expect("the first runtime");
    l.press(b"\x0e");
    l.wait("the New chat dialog", "Start a new chat?", 10);
    l.confirm();
    std::thread::sleep(Duration::from_secs(4));
    let alive = |pid: u32| {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    assert!(!alive(first), "the first chat ended after New chat");
    l.type_text("Write eight paragraphs about the history of compilers.");
    l.press(b"\r");
    l.wait("a second runtime", "thinking", 60);
    let second = (0..100)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(100));
            l.runtime_pid().filter(|p| *p != first)
        })
        .expect("the second runtime");
    let mcp: Vec<u32> = {
        let out = Command::new("pgrep")
            .args(["-P", &second.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|x| x.trim().parse().ok())
            .collect()
    };
    let socket_dir = {
        let argv = std::fs::read_to_string(l.tmp.join("runtime-argv")).unwrap();
        let args: Vec<&str> = argv.lines().collect();
        let i = args
            .iter()
            .position(|a| *a == "--messaging-socket-path")
            .unwrap();
        PathBuf::from(args[i + 1]).parent().unwrap().to_path_buf()
    };
    std::thread::sleep(Duration::from_secs(3));
    let tui = {
        let out = Command::new("pgrep")
            .args(["-P", &l.script.id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|x| x.trim().parse::<u32>().ok())
            .unwrap()
    };
    assert!(Command::new("/bin/kill")
        .args(["-TERM", &tui.to_string()])
        .status()
        .unwrap()
        .success());
    let _ = l.script.wait();
    std::thread::sleep(Duration::from_millis(800));
    assert!(!alive(second), "the runtime outlived the cockpit");
    for p in &mcp {
        assert!(!alive(*p), "harness-mcp {p} outlived the cockpit");
    }
    assert!(!socket_dir.exists(), "the chat's directory is removed");
}
