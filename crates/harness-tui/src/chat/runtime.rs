//! The chat's runtime child (docs/CHAT-PANE-DESIGN.md §1.1, §1.2, §1.4):
//! the person's installed, unmodified `claude`, run headless in its own
//! process group, in a fresh private directory, with an environment the
//! cockpit chose and the exact command line of §1.1.
//!
//! - stdin is a pipe fed by a writer thread (a runtime that stops reading
//!   never blocks the UI); stdout lines (bounded at [`MAX_LINE_BYTES`], a
//!   longer one reported cut, never truncated into something parseable) and
//!   stderr are drained by two reader threads into one channel.
//! - Every chat process sits in [`Procs`], which the signal path, the panic
//!   hook and a dying hand edit reach too. The leader is reaped only after
//!   its whole group was sent SIGKILL — so a group is only ever signalled
//!   while its leader is unreaped (a zombie still reserves the id) and no
//!   grandchild (harness-mcp, anything it started) outlives the chat.
//! - The directory is removed once the process is reaped; directories of
//!   dead cockpits are swept at the next start (§R5: only this user's 0700
//!   directories, "gone" only from `kill -0` saying so under `LC_ALL=C`).

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Longest stdout line kept (a 512 KiB answer, JSON-escaped, rides in its
/// `tool_use` line and in its `can_use_tool` line).
pub const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;
/// Longest stderr line kept.
const MAX_STDERR_BYTES: usize = 16 * 1024;
/// Stderr lines kept for "why it ended".
const STDERR_TAIL: usize = 12;
/// New chat: SIGTERM to the old group this long after stdin closed…
pub const END_TERM_AFTER: Duration = Duration::from_secs(2);
/// …and SIGKILL this long after.
pub const END_KILL_AFTER: Duration = Duration::from_secs(3);
/// Quit: how long the cockpit waits for a graceful end after the terminal
/// is restored, before SIGTERM (SIGKILL [`BOUNDED_KILL_AFTER`] later).
pub const QUIT_WAIT: Duration = Duration::from_millis(1500);
/// The bounded end: SIGKILL this long after SIGTERM.
pub const BOUNDED_KILL_AFTER: Duration = Duration::from_millis(300);
/// The directory's name prefix.
pub const DIR_PREFIX: &str = "harness-tui-chat-";
/// The most stale directories one sweep looks at.
const SWEEP_MAX: usize = 64;
/// `sun_path` holds 104 bytes on macOS (108 on Linux), the NUL included.
const SOCKET_PATH_MAX: usize = 103;

/// The two binaries, resolved at start (absolute, canonical: the argv
/// shown is what runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binaries {
    /// Claude Code.
    pub claude: PathBuf,
    /// harness-mcp, attached with `--cockpit`.
    pub mcp: PathBuf,
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `name` on `path` (absolute entries only), canonical.
fn on_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|p| executable(p))
        .and_then(|p| p.canonicalize().ok())
}

fn given(flag: &str, path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .ok()
        .filter(|p| executable(p))
        .ok_or_else(|| format!("{flag} {}: not an executable file", path.display()))
}

/// Resolve both binaries, like `harness` (docs/CHAT-PANE-DESIGN.md §1.1):
/// `claude` from `runtime`, else PATH; harness-mcp from `mcp`, else next
/// to `exe`, else PATH. `Err` says in words what is missing.
pub fn resolve(
    runtime: Option<&Path>,
    mcp: Option<&Path>,
    exe: Option<&Path>,
    path: Option<&OsStr>,
) -> Result<Binaries, String> {
    let claude = match runtime {
        Some(p) => given("--chat-runtime", p)?,
        None => on_path("claude", path)
            .ok_or("claude not found: install Claude Code or pass --chat-runtime")?,
    };
    let mcp = match mcp {
        Some(p) => given("--harness-mcp", p)?,
        None => exe
            .and_then(Path::parent)
            .map(|d| d.join("harness-mcp"))
            .filter(|p| executable(p))
            .and_then(|p| p.canonicalize().ok())
            .or_else(|| on_path("harness-mcp", path))
            .ok_or("harness-mcp not found: build it beside harness-tui or pass --harness-mcp")?,
    };
    Ok(Binaries { claude, mcp })
}

/// The allowlist inside a Claude Code session (the spikes' own).
const ALLOW: &[&str] = &["HOME", "PATH", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG"];
/// The person's own sign-in and provider variables (passed inside a Claude
/// Code session, where everything else is the host's).
const SIGN_IN: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_CLIENT_CERT",
    "CLAUDE_CODE_CLIENT_KEY",
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "CLOUD_ML_REGION",
    "GOOGLE_APPLICATION_CREDENTIALS",
];
/// The credentials an `ANTHROPIC_BASE_URL` goes with.
const ANTHROPIC_CREDENTIALS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
];
/// What a Claude Code session sets for its children: exact names…
const SESSION_NAMES: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_EXECPATH",
];
/// …and prefixes.
const SESSION_PREFIXES: &[&str] = &[
    "CLAUDE_CODE_SESSION_",
    "CLAUDE_CODE_HOST_",
    "CLAUDE_CODE_MESSAGING_",
    "CLAUDE_CODE_SDK_",
    "CLAUDE_CODE_DESKTOP_",
    "CLAUDE_AGENT_",
    "MCP_",
];

fn inside_claude_code(vars: &[(OsString, OsString)]) -> bool {
    vars.iter().any(|(k, v)| k == "CLAUDECODE" && !v.is_empty())
}

fn get<'a>(vars: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    vars.iter()
        .find(|(k, v)| k == name && !v.is_empty())
        .map(|(_, v)| v)
}

/// The runtime's environment, from the cockpit's `vars` (§1.1): inside a
/// Claude Code session (`CLAUDECODE` set), the allowlist and `LC_*`, and
/// the person's sign-in variables — an `ANTHROPIC_BASE_URL` only with an
/// Anthropic credential beside it (alone it is the host's plumbing: never an
/// endpoint without its credentials, never credentials without theirs);
/// outside one, everything but what a Claude Code session sets for its
/// children. `TERM=dumb` either way.
pub fn environment(vars: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = if inside_claude_code(vars) {
        let endpoint_ok = ANTHROPIC_CREDENTIALS.iter().any(|c| get(vars, c).is_some());
        vars.iter()
            .filter(|(k, _)| {
                let k = k.to_string_lossy();
                ALLOW.contains(&k.as_ref())
                    || k.starts_with("LC_")
                    || k.starts_with("AWS_")
                    || (SIGN_IN.contains(&k.as_ref()) && (k != "ANTHROPIC_BASE_URL" || endpoint_ok))
            })
            .cloned()
            .collect()
    } else {
        vars.iter()
            .filter(|(k, _)| {
                let k = k.to_string_lossy();
                !SESSION_NAMES.contains(&k.as_ref())
                    && !SESSION_PREFIXES.iter().any(|p| k.starts_with(p))
            })
            .cloned()
            .collect()
    };
    out.retain(|(k, _)| k != "TERM");
    out.push(("TERM".into(), "dumb".into()));
    out
}

/// What the runtime will sign in with, in words, from its environment —
/// said before the first message is sent (§1.1).
pub fn sign_in_words(env: &[(OsString, OsString)]) -> String {
    let set = |n: &str| get(env, n).is_some();
    let mut words = if set("CLAUDE_CODE_USE_BEDROCK") {
        "Amazon Bedrock".to_string()
    } else if set("CLAUDE_CODE_USE_VERTEX") {
        "Google Vertex AI".to_string()
    } else if set("CLAUDE_CODE_USE_FOUNDRY") {
        "Microsoft Foundry".to_string()
    } else if set("ANTHROPIC_API_KEY") {
        "the API key in ANTHROPIC_API_KEY — billed to that key".to_string()
    } else if set("ANTHROPIC_AUTH_TOKEN") {
        "the token in ANTHROPIC_AUTH_TOKEN".to_string()
    } else if set("CLAUDE_CODE_OAUTH_TOKEN") {
        "Claude Code's OAuth token (CLAUDE_CODE_OAUTH_TOKEN)".to_string()
    } else {
        "your Claude subscription".to_string()
    };
    if let Some(url) = get(env, "ANTHROPIC_BASE_URL") {
        words.push_str(&format!(" at {}", url.to_string_lossy()));
    }
    words
}

/// Four random bytes as hex (from the system's randomness; the process id
/// and the time if it cannot be read).
fn random_hex() -> String {
    let mut bytes = [0u8; 4];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        bytes = (t ^ std::process::id().rotate_left(16)).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A fresh private directory for one chat process: its working directory
/// (no instruction file of the target's is ever loaded) and the home of its
/// inbox socket. `harness-tui-chat-<pid>-<8 hex>`, mode 0700, never an
/// existing one; under `temp`, or `/tmp` when the socket path would not fit.
pub fn create_dir(temp: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let name = format!("{DIR_PREFIX}{}-{}", std::process::id(), random_hex());
    let base = if temp.join(&name).join("inbox.sock").as_os_str().len() <= SOCKET_PATH_MAX {
        temp.to_path_buf()
    } else {
        PathBuf::from("/tmp")
    };
    let dir = base.join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// The pid a chat directory's name carries.
fn dir_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(DIR_PREFIX)?;
    let (pid, hex) = rest.split_once('-')?;
    let ok = hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    if !ok || pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

/// Whether process `pid` is gone: only when `kill -0` (in the C locale)
/// fails saying "No such process". A failed probe, EPERM or any other
/// answer: alive (§R5).
pub fn pid_gone(pid: u32) -> bool {
    let out = Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            !o.status.success() && String::from_utf8_lossy(&o.stderr).contains("No such process")
        }
        Err(_) => false,
    }
}

/// Remove the chat directories dead cockpits left under `temp` (a
/// SIGKILLed cockpit leaves its own; its runtime ends on stdin EOF): only
/// directories — never a link — owned by `uid`, mode exactly 0700, whose
/// pid is gone; at most [`SWEEP_MAX`] looked at. `own` is never touched.
/// Returns how many were removed.
pub fn sweep(temp: &Path, uid: u32, own: &Path) -> usize {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(entries) = std::fs::read_dir(temp) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries
        .flatten()
        .take(4096)
        .filter(|e| e.file_name().to_string_lossy().starts_with(DIR_PREFIX))
        .take(SWEEP_MAX)
    {
        let path = entry.path();
        if path == own {
            continue;
        }
        let Some(pid) = dir_pid(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let ours = meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == uid
            && meta.permissions().mode() & 0o777 == 0o700;
        if ours
            && pid != std::process::id()
            && pid_gone(pid)
            && std::fs::remove_dir_all(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// The runtime's command line (§1.1), the program first.
pub fn argv(
    bins: &Binaries,
    dir: &Path,
    target: &Path,
    brief: &str,
    model: Option<&str>,
) -> Vec<OsString> {
    let config = serde_json::json!({"mcpServers": {"harness": {
        "command": bins.mcp.to_string_lossy(),
        "args": ["--cockpit", "--target", target.to_string_lossy()],
    }}})
    .to_string();
    let mut argv: Vec<OsString> = vec![bins.claude.clone().into()];
    for a in [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        "default",
        "--tools",
        "",
        "--restricted",
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--strict-mcp-config",
        "--mcp-config",
    ] {
        argv.push(a.into());
    }
    argv.push(config.into());
    argv.push("--no-session-persistence".into());
    argv.push("--messaging-socket-path".into());
    argv.push(dir.join("inbox.sock").into());
    argv.push("--append-system-prompt".into());
    argv.push(brief.into());
    if let Some(m) = model {
        argv.push("--model".into());
        argv.push(m.into());
    }
    argv
}

/// What the reader threads report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// A stdout line (lossy UTF-8).
    Line(String),
    /// A stdout line longer than [`MAX_LINE_BYTES`]: dropped whole.
    Cut,
    /// A stderr line.
    Stderr(String),
    /// A pipe hit EOF (or failed).
    Eof(bool),
}

/// One line of at most `cap` bytes: `(bytes, cut)` — a longer line is read
/// to its end and reported cut, its bytes dropped.
pub fn read_bounded(
    reader: &mut impl BufRead,
    cap: usize,
) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    let mut line = Vec::new();
    let mut cut = false;
    let mut seen = false;
    loop {
        let (used, done) = {
            let buf = match reader.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if buf.is_empty() {
                if !seen {
                    return Ok(None);
                }
                break;
            }
            seen = true;
            let (part, used, done) = match buf.iter().position(|b| *b == b'\n') {
                Some(i) => (&buf[..i], i + 1, true),
                None => (buf, buf.len(), false),
            };
            if !cut {
                if line.len() + part.len() > cap {
                    cut = true;
                    line = Vec::new();
                } else {
                    line.extend_from_slice(part);
                }
            }
            (used, done)
        };
        reader.consume(used);
        if done {
            break;
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some((line, cut)))
}

/// A chat process as the signal path sees it.
#[derive(Debug)]
pub struct Proc {
    /// Its generation.
    pub gen: u64,
    /// Its pid — its process group's id (it leads it).
    pub pid: u32,
    child: Child,
    /// Its private directory.
    pub dir: PathBuf,
    /// SIGKILL was sent to its group: it may be reaped.
    killed: bool,
}

/// Every chat process not yet reaped: the live one and the ending ones
/// (§1.2's two slots), shared with the signal path, the panic hook and a
/// dying hand edit.
pub type Procs = Arc<Mutex<Vec<Proc>>>;

fn lock(procs: &Procs) -> std::sync::MutexGuard<'_, Vec<Proc>> {
    procs.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `/bin/kill -<sig> -- -<pid>`: the group of an unreaped leader. `true`
/// when the signal was sent (a group already empty counts: nothing left
/// to end).
fn kill_group(pid: u32, sig: &str) -> bool {
    Command::new("/bin/kill")
        .args([format!("-{sig}"), "--".into(), format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Whether the leader `pid` has exited (a zombie, or gone): `ps` says so.
/// A failed probe says nothing (alive).
pub fn leader_exited(pid: u32) -> bool {
    let Ok(out) = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
    out.status.success() && stat.starts_with('Z')
}

/// Signal the group of chat `gen` with `sig` (`TERM`, `KILL`) — it is in
/// `procs`, so unreaped.
pub fn signal(procs: &Procs, gen: u64, sig: &str) {
    let mut list = lock(procs);
    if let Some(p) = list.iter_mut().find(|p| p.gen == gen) {
        // `killed` only once the KILL was really sent (review PRO-10).
        if kill_group(p.pid, sig) && sig == "KILL" {
            p.killed = true;
        }
    }
}

/// Reap chat `gen` once its group was sent SIGKILL, and remove its
/// directory: `true` once it is gone from `procs` (or was never there).
pub fn reap(procs: &Procs, gen: u64) -> bool {
    let mut list = lock(procs);
    let Some(i) = list.iter().position(|p| p.gen == gen) else {
        return true;
    };
    if !list[i].killed {
        if !kill_group(list[i].pid, "KILL") {
            return false;
        }
        list[i].killed = true;
    }
    match list[i].child.try_wait() {
        Ok(Some(_)) | Err(_) => {
            let p = list.remove(i);
            let _ = std::fs::remove_dir_all(&p.dir);
            true
        }
        Ok(None) => false,
    }
}

/// The bounded end of every chat (the signal path, the panic hook, a dying
/// hand edit — §1.4): SIGTERM to each group now; [`kill_all`] 300 ms later.
pub fn term_all(procs: &Procs) {
    for p in lock(procs).iter() {
        kill_group(p.pid, "TERM");
    }
}

/// SIGKILL to each group, reap what can be reaped (briefly), remove the
/// directories.
pub fn kill_all(procs: &Procs) {
    let mut list = lock(procs);
    for p in list.iter_mut() {
        p.killed = kill_group(p.pid, "KILL");
    }
    let deadline = Instant::now() + Duration::from_millis(200);
    while !list.is_empty() {
        list.retain_mut(|p| match p.child.try_wait() {
            Ok(None) => true,
            _ => {
                let _ = std::fs::remove_dir_all(&p.dir);
                false
            }
        });
        if list.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // What could not be reaped in time is killed; its directory goes too.
    for p in list.iter() {
        let _ = std::fs::remove_dir_all(&p.dir);
    }
}

/// [`term_all`] for the panic hook, which may run on a thread that holds
/// the lock: nothing when it is busy.
pub fn try_term_all(procs: &Procs) -> bool {
    match procs.try_lock() {
        Ok(list) => {
            for p in list.iter() {
                kill_group(p.pid, "TERM");
            }
            true
        }
        Err(std::sync::TryLockError::Poisoned(list)) => {
            for p in list.into_inner().iter() {
                kill_group(p.pid, "TERM");
            }
            true
        }
        Err(std::sync::TryLockError::WouldBlock) => false,
    }
}

/// [`kill_all`] for the panic hook: nothing when the lock is busy.
pub fn try_kill_all(procs: &Procs) -> bool {
    let mut list = match procs.try_lock() {
        Ok(list) => list,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return false,
    };
    // What is reaped here leaves the registry: a later end never signals
    // a reaped leader's group (review PRO-8).
    list.retain_mut(|p| {
        p.killed = kill_group(p.pid, "KILL");
        match p.child.try_wait() {
            Ok(None) => true,
            _ => {
                let _ = std::fs::remove_dir_all(&p.dir);
                false
            }
        }
    });
    true
}

/// The live handle of one chat process (the loop's; its [`Proc`] is in
/// [`Procs`]).
#[derive(Debug)]
pub struct Runtime {
    /// Its generation.
    pub gen: u64,
    /// Its pid (and group).
    pub pid: u32,
    /// Its directory.
    pub dir: PathBuf,
    /// The command line it runs, the program first.
    pub argv: Vec<OsString>,
    rx: Receiver<Out>,
    tx: Option<Sender<String>>,
    stdout_eof: bool,
    stderr_eof: bool,
    /// The last stderr lines.
    pub stderr_tail: VecDeque<String>,
    procs: Procs,
}

impl Runtime {
    /// Start `argv` in `dir` (created by [`create_dir`]) with exactly `env`,
    /// in its own process group, into `procs` as generation `gen`.
    pub fn spawn(
        gen: u64,
        argv: Vec<OsString>,
        dir: PathBuf,
        env: &[(OsString, OsString)],
        procs: &Procs,
    ) -> std::io::Result<Runtime> {
        use std::os::unix::process::CommandExt;
        let Some((program, args)) = argv.split_first() else {
            return Err(std::io::Error::other("empty command line"));
        };
        // Held across the spawn: the signal path sees the child as soon as
        // it exists (it waits for the lock at worst).
        let mut list = lock(procs);
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&dir)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let pid = child.id();
        let (tx, rx) = channel();
        let fail = |child: &mut Child, e: std::io::Error| {
            kill_group(child.id(), "KILL");
            let _ = child.wait();
            e
        };
        if let Some(stdout) = child.stdout.take() {
            let tx = tx.clone();
            let spawned = std::thread::Builder::new().spawn(move || {
                let mut r = BufReader::new(stdout);
                while let Ok(Some((line, cut))) = read_bounded(&mut r, MAX_LINE_BYTES) {
                    let msg = if cut {
                        Out::Cut
                    } else {
                        Out::Line(String::from_utf8_lossy(&line).into_owned())
                    };
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Out::Eof(true));
            });
            if let Err(e) = spawned {
                return Err(fail(&mut child, e));
            }
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = tx.clone();
            let spawned = std::thread::Builder::new().spawn(move || {
                let mut r = BufReader::new(stderr);
                while let Ok(Some((line, _))) = read_bounded(&mut r, MAX_STDERR_BYTES) {
                    if tx
                        .send(Out::Stderr(String::from_utf8_lossy(&line).into_owned()))
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = tx.send(Out::Eof(false));
            });
            if let Err(e) = spawned {
                return Err(fail(&mut child, e));
            }
        }
        let (wtx, wrx) = channel::<String>();
        if let Some(mut stdin) = child.stdin.take() {
            let spawned = std::thread::Builder::new().spawn(move || {
                // One line per message; a runtime that stopped reading
                // blocks this thread, never the loop. Closed when the
                // sender is dropped.
                while let Ok(line) = wrx.recv() {
                    if stdin
                        .write_all(line.as_bytes())
                        .and_then(|()| stdin.write_all(b"\n"))
                        .and_then(|()| stdin.flush())
                        .is_err()
                    {
                        return;
                    }
                }
            });
            if let Err(e) = spawned {
                return Err(fail(&mut child, e));
            }
        }
        list.push(Proc {
            gen,
            pid,
            child,
            dir: dir.clone(),
            killed: false,
        });
        drop(list);
        Ok(Runtime {
            gen,
            pid,
            dir,
            argv,
            rx,
            tx: Some(wtx),
            stdout_eof: false,
            stderr_eof: false,
            stderr_tail: VecDeque::new(),
            procs: procs.clone(),
        })
    }

    /// Write one line to its stdin (through the writer thread): `false`
    /// once stdin is closed.
    pub fn send(&self, line: String) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.send(line).is_ok())
    }

    /// Close its stdin (the writer ends once it wrote what is queued).
    pub fn close_stdin(&mut self) {
        self.tx = None;
    }

    /// Stdin is still open.
    pub fn open(&self) -> bool {
        self.tx.is_some()
    }

    /// Everything the readers reported so far (never blocks); stderr lines
    /// are kept in [`Runtime::stderr_tail`] too.
    pub fn drain(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    match &msg {
                        Out::Eof(true) => self.stdout_eof = true,
                        Out::Eof(false) => self.stderr_eof = true,
                        Out::Stderr(line) => {
                            self.stderr_tail.push_back(line.clone());
                            if self.stderr_tail.len() > STDERR_TAIL {
                                self.stderr_tail.pop_front();
                            }
                        }
                        _ => {}
                    }
                    out.push(msg);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.stdout_eof = true;
                    self.stderr_eof = true;
                    break;
                }
            }
        }
        out
    }

    /// Both pipes are at EOF: the process closed them (it is ending).
    pub fn eof(&self) -> bool {
        self.stdout_eof && self.stderr_eof
    }

    /// Signal its group (`TERM`, `KILL`).
    pub fn signal(&self, sig: &str) {
        signal(&self.procs, self.gen, sig);
    }

    /// Kill what is left of its group and reap it, removing its directory:
    /// `true` once gone.
    pub fn reap(&self) -> bool {
        reap(&self.procs, self.gen)
    }
}

/// A chat on its way out (§1.4): stdin closed; SIGTERM to its group at
/// `term_at`, SIGKILL at `kill_at` — or at once when both pipes reached
/// EOF (the process is ending: its group's stragglers go with it) — then
/// reaped and its directory removed.
#[derive(Debug)]
pub struct Ending {
    /// The process.
    pub runtime: Runtime,
    /// When SIGTERM goes to its group.
    pub term_at: Instant,
    /// When SIGKILL goes to its group.
    pub kill_at: Instant,
    termed: bool,
}

impl Ending {
    /// End `runtime` gracefully from `now`: its stdin closed (after an
    /// `interrupt`, sent by the caller when a turn ran).
    pub fn new(
        mut runtime: Runtime,
        now: Instant,
        term_after: Duration,
        kill_after: Duration,
    ) -> Ending {
        runtime.close_stdin();
        Ending {
            runtime,
            term_at: now + term_after,
            kill_at: now + kill_after,
            termed: false,
        }
    }

    /// One pass of the loop: `true` once it is reaped and its directory
    /// removed.
    pub fn step(&mut self, now: Instant) -> bool {
        let _ = self.runtime.drain();
        if self.runtime.eof() || now >= self.kill_at {
            return self.runtime.reap();
        }
        if now >= self.term_at && !self.termed {
            self.termed = true;
            self.runtime.signal("TERM");
        }
        false
    }

    /// End it now: SIGKILL to its group and reap (New chat while this one
    /// is still ending, §1.2).
    pub fn kill_now(&mut self) -> bool {
        self.runtime.signal("KILL");
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if self.runtime.reap() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    fn names(env: &[(OsString, OsString)]) -> Vec<String> {
        let mut n: Vec<String> = env
            .iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    }

    /// Mutation-checked rule (§1.1): inside a Claude Code session only the
    /// allowlist and the person's sign-in pass — the host's endpoint alone
    /// never; outside one, everything but the session's variables.
    #[test]
    fn the_environment_follows_the_rules() {
        let host = vars(&[
            ("CLAUDECODE", "1"),
            ("HOME", "/h"),
            ("PATH", "/bin"),
            ("LANG", "C.UTF-8"),
            ("LC_CTYPE", "UTF-8"),
            ("TERM", "xterm"),
            ("ANTHROPIC_BASE_URL", "http://host-proxy"),
            ("CLAUDE_CODE_SESSION_ID", "s"),
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/x"),
            ("MCP_CONNECTION_NONBLOCKING", "1"),
            ("DISABLE_AUTOUPDATER", "1"),
            ("CLAUDE_CONFIG_DIR", "/c"),
            ("AWS_PROFILE", "p"),
        ]);
        let env = environment(&host);
        assert_eq!(
            names(&env),
            [
                "AWS_PROFILE",
                "CLAUDE_CONFIG_DIR",
                "HOME",
                "LANG",
                "LC_CTYPE",
                "PATH",
                "TERM"
            ]
        );
        assert!(env.contains(&("TERM".into(), "dumb".into())));
        assert_eq!(sign_in_words(&env), "your Claude subscription");
        // The endpoint with its credential: both pass, and are named.
        let mut with_key = host.clone();
        with_key.push(("ANTHROPIC_AUTH_TOKEN".into(), "t".into()));
        let env = environment(&with_key);
        assert!(names(&env).contains(&"ANTHROPIC_BASE_URL".to_string()));
        assert!(names(&env).contains(&"ANTHROPIC_AUTH_TOKEN".to_string()));
        assert_eq!(
            sign_in_words(&env),
            "the token in ANTHROPIC_AUTH_TOKEN at http://host-proxy"
        );
        // Outside a session: the person's own environment, less the
        // session's variables.
        let mut outside = host.clone();
        outside.retain(|(k, _)| k != "CLAUDECODE");
        outside.push(("CLAUDE_PID".into(), "7".into()));
        outside.push(("CLAUDE_CODE_ENTRYPOINT".into(), "sdk".into()));
        let env = environment(&outside);
        let n = names(&env);
        for gone in [
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "MCP_CONNECTION_NONBLOCKING",
            "CLAUDE_PID",
            "CLAUDE_CODE_ENTRYPOINT",
        ] {
            assert!(!n.contains(&gone.to_string()), "{gone}");
        }
        for kept in ["DISABLE_AUTOUPDATER", "ANTHROPIC_BASE_URL", "HOME"] {
            assert!(n.contains(&kept.to_string()), "{kept}");
        }
        assert_eq!(env.iter().filter(|(k, _)| k == "TERM").count(), 1);
        assert_eq!(
            sign_in_words(&vars(&[("ANTHROPIC_API_KEY", "k")])),
            "the API key in ANTHROPIC_API_KEY — billed to that key"
        );
        assert_eq!(
            sign_in_words(&vars(&[("CLAUDE_CODE_USE_BEDROCK", "1")])),
            "Amazon Bedrock"
        );
    }

    #[test]
    fn the_command_line_is_the_designs() {
        let bins = Binaries {
            claude: "/b/claude".into(),
            mcp: "/b/harness-mcp".into(),
        };
        let argv = argv(
            &bins,
            Path::new("/t/d"),
            Path::new("/repo/t"),
            "BRIEF",
            Some("haiku"),
        );
        let s: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(s[0], "/b/claude");
        let at = |flag: &str| s.iter().position(|a| a == flag).map(|i| s[i + 1].clone());
        assert_eq!(at("--tools").as_deref(), Some(""));
        assert_eq!(at("--setting-sources").as_deref(), Some(""));
        assert_eq!(at("--permission-prompt-tool").as_deref(), Some("stdio"));
        assert_eq!(at("--permission-mode").as_deref(), Some("default"));
        assert_eq!(
            at("--messaging-socket-path").as_deref(),
            Some("/t/d/inbox.sock")
        );
        assert_eq!(at("--append-system-prompt").as_deref(), Some("BRIEF"));
        assert_eq!(at("--model").as_deref(), Some("haiku"));
        for flag in [
            "--restricted",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--no-session-persistence",
            "--include-partial-messages",
            "--verbose",
        ] {
            assert!(s.contains(&flag.to_string()), "{flag}");
        }
        assert!(!s.contains(&"--bare".to_string()));
        let config: serde_json::Value = serde_json::from_str(&at("--mcp-config").unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["harness"]["command"], "/b/harness-mcp");
        assert_eq!(
            config["mcpServers"]["harness"]["args"],
            serde_json::json!(["--cockpit", "--target", "/repo/t"])
        );
        let no_model = super::argv(&bins, Path::new("/d"), Path::new("/t"), "b", None);
        assert!(!no_model.contains(&OsString::from("--model")));
    }

    #[test]
    fn binaries_resolve_absolute_and_canonical() {
        let tmp = crate::testutil::TmpDir::new("chat-resolve");
        let bin = tmp.0.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["claude", "harness-mcp", "harness-tui"] {
            let p = bin.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = OsString::from(format!("relative:{}", bin.display()));
        let b = resolve(None, None, Some(&bin.join("harness-tui")), Some(&path)).unwrap();
        assert_eq!(b.claude, bin.join("claude").canonicalize().unwrap());
        assert_eq!(b.mcp, bin.join("harness-mcp").canonicalize().unwrap());
        let err = resolve(None, None, None, Some(OsStr::new("relative"))).unwrap_err();
        assert!(err.starts_with("claude not found"), "{err}");
        let err = resolve(Some(Path::new("/no/such")), None, None, None).unwrap_err();
        assert!(err.starts_with("--chat-runtime"), "{err}");
    }

    #[test]
    fn a_long_line_is_reported_cut_never_truncated() {
        let data = format!("{}\nshort\n{}", "x".repeat(50), "tail-no-newline");
        let mut r = std::io::BufReader::with_capacity(8, data.as_bytes());
        assert_eq!(read_bounded(&mut r, 10).unwrap(), Some((Vec::new(), true)));
        assert_eq!(
            read_bounded(&mut r, 10).unwrap(),
            Some((b"short".to_vec(), false))
        );
        assert_eq!(
            read_bounded(&mut r, 100).unwrap(),
            Some((b"tail-no-newline".to_vec(), false))
        );
        assert_eq!(read_bounded(&mut r, 10).unwrap(), None);
    }

    #[test]
    fn the_directory_is_private_and_its_name_bounded() {
        use std::os::unix::fs::MetadataExt;
        let tmp = crate::testutil::TmpDir::new("chat-dir");
        let dir = create_dir(&tmp.0).unwrap();
        let meta = std::fs::metadata(&dir).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o700);
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(dir_pid(&name), Some(std::process::id()));
        assert!(dir.join("inbox.sock").as_os_str().len() <= SOCKET_PATH_MAX);
        // A base too long for the socket path: /tmp.
        let deep = tmp.0.join("d".repeat(90));
        std::fs::create_dir_all(&deep).unwrap();
        let fallback = create_dir(&deep).unwrap();
        assert!(fallback.starts_with("/tmp"), "{}", fallback.display());
        let _ = std::fs::remove_dir_all(&fallback);
        assert_eq!(dir_pid("harness-tui-chat-12-0123abcd"), Some(12));
        for bad in [
            "harness-tui-chat-12-0123abc",
            "harness-tui-chat--0123abcd",
            "harness-tui-chat-1x-0123abcd",
            "harness-tui-chat-12-0123abcz",
        ] {
            assert_eq!(dir_pid(bad), None, "{bad}");
        }
    }

    /// Mutation-checked rule (§R5, carried to the chat): the sweep removes
    /// only this user's 0700 directories whose pid is gone — never a link,
    /// another mode, a live pid, or its own.
    #[test]
    fn the_sweep_removes_only_dead_private_directories() {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let tmp = crate::testutil::TmpDir::new("chat-sweep");
        let uid = std::fs::metadata(&tmp.0).unwrap().uid();
        // A pid that is gone: a child we reaped.
        let mut c = Command::new("/usr/bin/true").spawn().unwrap();
        let dead = c.id();
        c.wait().unwrap();
        let make = |name: String, mode: u32| {
            let p = tmp.0.join(name);
            std::fs::DirBuilder::new().mode(mode).create(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            std::fs::write(p.join("inbox.sock"), "").unwrap();
            p
        };
        let stale = make(format!("{DIR_PREFIX}{dead}-0123abcd"), 0o700);
        let open = make(format!("{DIR_PREFIX}{dead}-0123abce"), 0o755);
        let live = make(
            format!("{DIR_PREFIX}{}-0123abcf", std::process::id()),
            0o700,
        );
        let own = make(format!("{DIR_PREFIX}{dead}-00000000"), 0o700);
        // Another live pid (pid 1: its probe fails with EPERM — alive).
        let other = make(format!("{DIR_PREFIX}1-0123abd2"), 0o700);
        let odd = make(format!("{DIR_PREFIX}{dead}-xyz"), 0o700);
        let target = make("elsewhere".into(), 0o700);
        let link = tmp.0.join(format!("{DIR_PREFIX}{dead}-0123abd0"));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(sweep(&tmp.0, uid, &own), 1);
        assert!(!stale.exists());
        for kept in [&open, &live, &own, &odd, &target, &other] {
            assert!(kept.exists(), "{}", kept.display());
        }
        assert!(
            link.symlink_metadata().is_ok(),
            "a link is never followed or removed"
        );
        assert!(target.join("inbox.sock").exists());
        // Another user's directory is never touched.
        let stale2 = make(format!("{DIR_PREFIX}{dead}-0123abd1"), 0o700);
        assert_eq!(sweep(&tmp.0, uid.wrapping_add(1), &own), 0);
        assert!(stale2.exists());
        assert!(!pid_gone(std::process::id()));
        assert!(pid_gone(dead));
        // A probe that fails for another reason (EPERM: pid 1 is not ours)
        // says nothing: alive (§R5).
        assert!(!pid_gone(1));
    }

    fn sh(script: &str) -> Vec<OsString> {
        ["/bin/sh", "-c", script].map(OsString::from).to_vec()
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn alive(pid: u32) -> bool {
        !pid_gone(pid)
    }

    /// A line in, a line out, EOF on stdin ends it; its group, with a
    /// grandchild, is gone and its directory removed once reaped.
    #[test]
    fn a_runtime_talks_ends_and_takes_its_group_along() {
        let tmp = crate::testutil::TmpDir::new("chat-proc");
        let dir = create_dir(&tmp.0).unwrap();
        let procs = Procs::default();
        let pidfile = tmp.0.join("grandchild");
        let script = format!(
            "sleep 30 & echo $! > '{}'; read line; echo \"got $line\"; echo oops >&2; \
             read rest; exit 0",
            pidfile.display()
        );
        let env = vars(&[("PATH", "/usr/bin:/bin")]);
        let mut rt = Runtime::spawn(1, sh(&script), dir.clone(), &env, &procs).unwrap();
        assert!(rt.send("hello".into()));
        let mut got = Vec::new();
        wait_until("the echo", || {
            got.extend(rt.drain());
            got.contains(&Out::Line("got hello".into()))
        });
        let grandchild: u32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(grandchild));
        let mut ending = Ending::new(rt, Instant::now(), END_TERM_AFTER, END_KILL_AFTER);
        wait_until("the end", || ending.step(Instant::now()));
        assert!(!dir.exists(), "the directory is removed");
        assert!(lock(&procs).is_empty());
        wait_until("the grandchild's end", || !alive(grandchild));
    }

    /// A runtime that ignores TERM and never closes its pipes, and one that
    /// is stopped: SIGKILL ends both, bounded.
    #[test]
    fn a_stubborn_or_stopped_runtime_is_killed() {
        let tmp = crate::testutil::TmpDir::new("chat-stubborn");
        let procs = Procs::default();
        let env = vars(&[("PATH", "/usr/bin:/bin")]);
        let t0 = Instant::now();
        let stubborn = Runtime::spawn(
            1,
            sh("trap '' TERM; while :; do sleep 0.05; done"),
            create_dir(&tmp.0).unwrap(),
            &env,
            &procs,
        )
        .unwrap();
        let mut e = Ending::new(
            stubborn,
            t0,
            Duration::from_millis(100),
            Duration::from_millis(300),
        );
        wait_until("the stubborn end", || e.step(Instant::now()));
        assert!(t0.elapsed() >= Duration::from_millis(300));
        let stopped = Runtime::spawn(
            2,
            sh("kill -STOP $$; sleep 30"),
            create_dir(&tmp.0).unwrap(),
            &env,
            &procs,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let mut e = Ending::new(
            stopped,
            Instant::now(),
            Duration::ZERO,
            Duration::from_secs(60),
        );
        assert!(e.kill_now(), "a stopped process dies by KILL");
        assert!(lock(&procs).is_empty());
    }

    /// The bounded end (the signal path): TERM, then KILL, the directories
    /// removed — whatever the processes do.
    #[test]
    fn the_bounded_end_kills_every_chat() {
        let tmp = crate::testutil::TmpDir::new("chat-bounded");
        let procs = Procs::default();
        let env = vars(&[("PATH", "/usr/bin:/bin")]);
        let mut pids = Vec::new();
        let mut dirs = Vec::new();
        for gen in 1..=2 {
            let dir = create_dir(&tmp.0).unwrap();
            let rt = Runtime::spawn(gen, sh("trap '' TERM; sleep 30"), dir.clone(), &env, &procs)
                .unwrap();
            pids.push(rt.pid);
            dirs.push(dir);
            std::mem::forget(rt);
        }
        term_all(&procs);
        std::thread::sleep(BOUNDED_KILL_AFTER);
        kill_all(&procs);
        assert!(lock(&procs).is_empty());
        for d in &dirs {
            assert!(!d.exists());
        }
        for p in pids {
            wait_until("gone", || !alive(p));
        }
    }

    /// Review PRO-8, PRO-11: what the panic path reaps leaves the registry;
    /// a leader that exited is seen as such while it is a zombie.
    #[test]
    fn a_reaped_entry_leaves_and_a_zombie_leader_is_seen() {
        let tmp = crate::testutil::TmpDir::new("chat-zombie");
        let procs = Procs::default();
        let env = vars(&[("PATH", "/usr/bin:/bin")]);
        let rt =
            Runtime::spawn(1, sh("exit 0"), create_dir(&tmp.0).unwrap(), &env, &procs).unwrap();
        wait_until("a zombie", || leader_exited(rt.pid));
        assert!(!leader_exited(std::process::id()));
        assert!(try_kill_all(&procs));
        assert!(lock(&procs).is_empty(), "reaped entries leave the registry");
    }

    /// A runtime that stops reading its stdin never blocks the sender.
    #[test]
    fn the_writer_never_blocks_the_loop() {
        let tmp = crate::testutil::TmpDir::new("chat-writer");
        let procs = Procs::default();
        let env = vars(&[("PATH", "/usr/bin:/bin")]);
        let rt =
            Runtime::spawn(1, sh("sleep 30"), create_dir(&tmp.0).unwrap(), &env, &procs).unwrap();
        let t0 = Instant::now();
        for _ in 0..64 {
            rt.send("x".repeat(64 * 1024));
        }
        assert!(t0.elapsed() < Duration::from_secs(2), "sending blocked");
        let mut e = Ending::new(
            rt,
            Instant::now(),
            Duration::ZERO,
            Duration::from_millis(50),
        );
        wait_until("the end", || e.step(Instant::now()));
    }
}
