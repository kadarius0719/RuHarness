//! A scripted MCP client over the real `harness-mcp` binary's stdio.

#![allow(dead_code)]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn server_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_harness-mcp"))
}

/// The CLI next to the server (`cargo test --workspace` builds it) — and
/// never an older build than its sources: harness-mcp does not depend on
/// harness-cli, so `cargo test -p harness-mcp` alone would drive a stale
/// binary without this check (§R2 TESTS-8).
pub fn harness_bin() -> PathBuf {
    let path = server_bin().with_file_name("harness");
    let built = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or_else(|_| {
            panic!(
                "{} is missing: build the CLI first (`cargo build -p harness-cli`; \
                 `cargo test --workspace` does)",
                path.display()
            )
        });
    fn newest(dir: &Path, newest_seen: &mut Option<(std::time::SystemTime, PathBuf)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                newest(&p, newest_seen);
            } else if let Ok(t) = entry.metadata().and_then(|m| m.modified()) {
                if newest_seen.as_ref().is_none_or(|(n, _)| t > *n) {
                    *newest_seen = Some((t, p));
                }
            }
        }
    }
    let mut seen = None;
    for krate in [
        "harness-cli",
        "harness-core",
        "harness-llm",
        "harness-oracle",
        "harness-scan",
        "harness-detect",
    ] {
        newest(&repo().join("crates").join(krate).join("src"), &mut seen);
    }
    if let Some((t, source)) = seen {
        assert!(
            built >= t,
            "{} is older than {}: rebuild it (`cargo build -p harness-cli`)",
            path.display(),
            source.display()
        );
    }
    path
}

/// A temp directory removed on drop — also when a test fails.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("harness-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn copy_dir(src: &Path, dst: &Path) {
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

pub fn children_of(pid: u32) -> Vec<u32> {
    let out = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

pub fn comm_of(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn wait_for<T>(what: &str, secs: u64, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Kills whatever the test started, whatever happens — each pid only while
/// it still runs the program it was recorded with (never a reused pid).
#[derive(Default)]
pub struct Reaper(Vec<(u32, String)>);

impl Reaper {
    pub fn add(&mut self, pid: u32) {
        self.0.push((pid, comm_of(pid)));
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        for (pid, comm) in &self.0 {
            if !comm.is_empty() && alive(*pid) && comm_of(*pid) == *comm {
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", &pid.to_string()])
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

/// The server, its stdin, and every stdout line parsed.
pub struct Client {
    pub child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Result<Value, String>>,
    pub stderr: Arc<Mutex<String>>,
    /// Messages received but not yet taken.
    pending: Vec<Value>,
}

impl Client {
    pub fn start(args: &[&str]) -> Client {
        let mut child = Command::new(server_bin())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                // Every stdout line must be one JSON-RPC 2.0 message.
                let parsed = serde_json::from_str::<Value>(&line)
                    .map_err(|e| format!("not JSON ({e}): {line}"))
                    .and_then(|v| {
                        if v["jsonrpc"] == "2.0" {
                            Ok(v)
                        } else {
                            Err(format!("not JSON-RPC 2.0: {line}"))
                        }
                    });
                if tx.send(parsed).is_err() {
                    return;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        {
            let stderr = stderr.clone();
            let mut pipe = child.stderr.take().unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    stderr
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            });
        }
        Client {
            stdin: child.stdin.take(),
            child,
            rx,
            stderr,
            pending: Vec::new(),
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    pub fn send(&mut self, message: Value) {
        self.raw(&message.to_string());
    }

    pub fn request(&mut self, id: Value, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    pub fn call(&mut self, id: u64, tool: &str, arguments: Value, token: Option<&str>) {
        let mut params = json!({"name": tool, "arguments": arguments});
        if let Some(token) = token {
            params["_meta"] = json!({"progressToken": token});
        }
        self.request(json!(id), "tools/call", params);
    }

    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    fn recv(&mut self, deadline: Instant) -> Option<Value> {
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match self.rx.recv_timeout(left) {
            Ok(Ok(v)) => Some(v),
            Ok(Err(bad)) => panic!("{bad}"),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// The response to `id`, and every other message that arrived first.
    pub fn response(&mut self, id: &Value, secs: u64) -> (Value, Vec<Value>) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut before = Vec::new();
        loop {
            let Some(msg) = self.recv(deadline) else {
                panic!(
                    "no response to {id} within {secs} s; got {before:?}; stderr:\n{}",
                    self.stderr.lock().unwrap()
                );
            };
            if msg.get("id") == Some(id) && msg.get("method").is_none() {
                return (msg, before);
            }
            before.push(msg);
        }
    }

    /// Every message that arrives within `ms`.
    pub fn drain(&mut self, ms: u64) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut out = Vec::new();
        while let Some(msg) = self.recv(deadline) {
            out.push(msg);
        }
        out
    }

    pub fn initialize(&mut self) -> Value {
        self.request(
            json!(0),
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        );
        let (init, _) = self.response(&json!(0), 30);
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    pub fn wait_exit(&mut self, secs: u64) -> ExitStatus {
        wait_for("the server to exit", secs, || {
            self.child.try_wait().unwrap()
        })
    }
}

impl Drop for Client {
    /// SIGTERM first — the server's own shutdown interrupts its act (a
    /// SIGKILL would orphan the harness, which leads its own group) — then
    /// SIGKILL after 3 s.
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &self.child.id().to_string()])
                .stderr(Stdio::null())
                .status();
            let deadline = Instant::now() + Duration::from_secs(3);
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A tool result's `structuredContent`, checked against its text channel.
pub fn structured(response: &Value) -> Value {
    let result = &response["result"];
    let text: Value = serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text content: {response}")),
    )
    .unwrap();
    assert_eq!(
        text, result["structuredContent"],
        "both channels carry the same JSON"
    );
    result["structuredContent"].clone()
}

pub fn is_error(response: &Value) -> bool {
    response["result"]["isError"] == true
}
