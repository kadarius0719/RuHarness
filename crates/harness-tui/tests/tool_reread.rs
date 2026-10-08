//! The cockpit binary opened on one tool of a project of two
//! (docs/PROJECT-MAP-DESIGN.md §3.7): the title row names the tool, and a
//! re-read on the loader thread (`g`) opens that same tool — a read by the
//! lookup order alone would refuse ("2 mapped tools and no harness.toml of
//! its own"). Tests reload through `App::reload`, a separate path, so only
//! the real binary under a pseudo-terminal (`script`) proves the loader's.
//!
//! The two tools are written by hand: their `migration/` holds no results,
//! so no adoption is asked.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ROWS: usize = 40;
const COLS: usize = 140;

/// A directory removed on drop — also when the test fails.
struct TmpDir(PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Kills the pseudo-terminal's process group leader, whatever happens.
struct Reaper(std::process::Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn project() -> TmpDir {
    let dir = std::env::temp_dir().join(format!(
        "harness-tui-tool-reread-{}-{}",
        std::process::id(),
        harness_core::hash::random_hex(4)
    ));
    let put = |rel: &str, text: &str| {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put("src/a.c", "int a(void) { return 1; }\n");
    put("src/b.c", "int b(void) { return 2; }\n");
    for (id, file) in [("t-a", "src/a.c"), ("t-b", "src/b.c")] {
        put(
            &format!("migration/tools/{id}/harness.toml"),
            &format!(
                "schema_version = 2\n[target]\nname = \"{id}\"\nfiles = [\n\
                 {{ path = \"{file}\", include_dirs = [] }},\n]\n\
                 configuration = {{ name = \"make\", from = \"stated\", flags = [] }}\n"
            ),
        );
    }
    TmpDir(dir.canonicalize().unwrap())
}

/// What a terminal would show after `bytes` (cursor moves, erases, text),
/// whitespace removed: ratatui redraws only the cells that changed.
fn on_screen(bytes: &str) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut grid = vec![vec![' '; COLS]; ROWS];
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
                            r = n(0, 1).saturating_sub(1).min(ROWS - 1);
                            c = n(1, 1).saturating_sub(1).min(COLS - 1);
                        }
                        'A' => r = r.saturating_sub(n(0, 1)),
                        'B' => r = (r + n(0, 1)).min(ROWS - 1),
                        'C' => c = (c + n(0, 1)).min(COLS - 1),
                        'D' => c = c.saturating_sub(n(0, 1)),
                        'G' => c = n(0, 1).saturating_sub(1).min(COLS - 1),
                        'J' if nums.first() == Some(&2) || nums.first() == Some(&3) => {
                            grid = vec![vec![' '; COLS]; ROWS];
                        }
                        'J' => {
                            for cell in grid[r].iter_mut().skip(c) {
                                *cell = ' ';
                            }
                            for row in grid.iter_mut().skip(r + 1) {
                                *row = vec![' '; COLS];
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
            '\n' => r = (r + 1).min(ROWS - 1),
            ch if ch.is_control() => {}
            ch => {
                let w = ch.width().unwrap_or(0);
                if w == 0 {
                    continue;
                }
                if c < COLS {
                    grid[r][c] = ch;
                    if w == 2 && c + 1 < COLS {
                        grid[r][c + 1] = ' ';
                    }
                }
                c = (c + w).min(COLS);
            }
        }
    }
    grid.into_iter()
        .flatten()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Away from a terminal, a project of two tools opened without `--tool` is
/// refused as the command line refuses it: the tools named, exit 1 (not a
/// usage error); a folder with no target says the one sentence.
#[test]
fn two_tools_and_no_tool_exit_1_as_the_command_line() {
    harness_core::adopt::testing::adoption_file();
    let dir = project();
    let out = Command::new(env!("CARGO_BIN_EXE_harness-tui"))
        .args(["--target", dir.0.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("2 mapped tools") && stderr.contains("t-a, t-b"),
        "{stderr}"
    );
    let bare = dir.0.join("src");
    let out = Command::new(env!("CARGO_BIN_EXE_harness-tui"))
        .args(["--target", bare.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        format!(
            "harness-tui: {}",
            harness_core::Error::no_target_here(&bare)
        )
    );
}

#[test]
fn the_binarys_background_reread_opens_the_tool_it_was_started_on() {
    // Every child inherits the test process's own adoption file.
    harness_core::adopt::testing::adoption_file();
    let harness = Path::new(env!("CARGO_BIN_EXE_harness-tui")).with_file_name("harness");
    assert!(
        harness.is_file(),
        "{} is missing: build the CLI first (`cargo build -p harness-cli`)",
        harness.display()
    );
    let dir = project();
    let t = dir.0.to_str().unwrap();
    let tui = env!("CARGO_BIN_EXE_harness-tui");
    let inner = format!(
        "stty rows {ROWS} cols {COLS}; exec '{tui}' --target '{t}' --tool t-a --harness '{}'",
        harness.display()
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
    let mut child = script
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script(1)");
    let mut keys = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let reaper = Reaper(child);
    let screen = Arc::new(Mutex::new(String::new()));
    {
        let screen = screen.clone();
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
    let now = || on_screen(&screen.lock().unwrap());
    let wait = |what: &str, secs: u64, done: &dyn Fn(&str) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let shown = now();
            if done(&shown) {
                return shown;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the screen:\n{shown}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let first = wait("the cockpit to draw", 30, &|s| s.contains("Files"));
    // The title row names the tool open.
    assert!(first.contains("·toolt-a"), "{first}");
    // A re-read on the loader thread: the same tool ("re-read" said once
    // more than the key hint says it), or a refusal ("unreadable: …").
    let hints = first.matches("re-read").count();
    keys.write_all(b"g").unwrap();
    keys.flush().unwrap();
    let after = wait("the re-read", 30, &|s| {
        s.matches("re-read").count() > hints || s.contains("unreadable")
    });
    assert!(!after.contains("unreadable"), "{after}");
    keys.write_all(b"q").unwrap();
    let _ = keys.flush();
    drop(reaper);
}
