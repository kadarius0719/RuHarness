//! One spawned `harness --json …` command (docs/TUI-DESIGN.md §4 "Child
//! process", §6): the child leads its own process group (a terminal hangup
//! reaches only the client's group, never kills the harness by the default
//! action), stdin is `/dev/null`, and stdout (the NDJSON events) and stderr
//! are drained by two reader threads into one channel, so neither pipe can
//! fill. A command is over only when BOTH readers hit EOF AND the child was
//! reaped — never on its `result` event alone (on a signal the CLI emits
//! `result` before it dies, holding the lock line). The child lives in a
//! [`ChildSlot`] shared with the client's signal path; it is only ever
//! signalled while `try_wait` says it has not been reaped, so a reused pid
//! is never hit.

use crate::events::{self, Event};
use std::ffi::OsString;
use std::io::BufReader;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The running child, shared with the signal path; its [`Running`] takes it
/// out once reaped. `std` keeps a reaped child's status, so `try_wait` on it
/// never touches the pid again.
pub type ChildSlot = Arc<Mutex<Option<Child>>>;

/// Longest stderr line kept, in bytes.
const MAX_STDERR_LINE_BYTES: usize = 64 * 1024;

/// What the reader threads report.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildMsg {
    /// A line of stdout, as an event.
    Event(Event),
    /// A line of stderr (lossy UTF-8).
    Stderr(String),
    /// A pipe hit EOF (or failed to read).
    Eof(Pipe),
}

/// Which pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipe {
    /// The events.
    Stdout,
    /// Human logs and errors.
    Stderr,
}

/// A spawned command.
#[derive(Debug)]
pub struct Running {
    argv: Vec<OsString>,
    pid: u32,
    slot: ChildSlot,
    rx: Receiver<ChildMsg>,
    stdout_eof: bool,
    stderr_eof: bool,
    status: Option<ExitStatus>,
}

fn lock(slot: &ChildSlot) -> std::sync::MutexGuard<'_, Option<Child>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Running {
    /// Spawn `argv` (the program first) in its own process group, into
    /// `slot` (which must be empty: one command at a time).
    pub fn spawn(argv: Vec<OsString>, slot: ChildSlot) -> std::io::Result<Running> {
        use std::os::unix::process::CommandExt;
        let Some((program, args)) = argv.split_first() else {
            return Err(std::io::Error::other("empty command line"));
        };
        let mut guard = lock(&slot);
        if guard.is_some() {
            return Err(std::io::Error::other("a command is already running"));
        }
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let pid = child.id();
        let (tx, rx) = channel();
        // Reader threads by `Builder`: a thread that cannot be created is an
        // error (the child's group is killed and reaped), never a panic
        // while the slot is locked and the child is not yet in it.
        let fail = |child: &mut Child, e: std::io::Error| {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.wait();
            e
        };
        if let Some(stdout) = child.stdout.take() {
            let tx = tx.clone();
            let spawned = std::thread::Builder::new().spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Ok(Some(line)) =
                    events::read_line_bounded(&mut reader, events::MAX_EVENT_LINE_BYTES)
                {
                    if tx.send(ChildMsg::Event(events::parse_line(&line))).is_err() {
                        return;
                    }
                }
                let _ = tx.send(ChildMsg::Eof(Pipe::Stdout));
            });
            if let Err(e) = spawned {
                return Err(fail(&mut child, e));
            }
        } else {
            let _ = tx.send(ChildMsg::Eof(Pipe::Stdout));
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = tx.clone();
            let spawned = std::thread::Builder::new().spawn(move || {
                let mut reader = BufReader::new(stderr);
                while let Ok(Some(line)) =
                    events::read_line_bounded(&mut reader, MAX_STDERR_LINE_BYTES)
                {
                    if tx.send(ChildMsg::Stderr(line)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(ChildMsg::Eof(Pipe::Stderr));
            });
            if let Err(e) = spawned {
                return Err(fail(&mut child, e));
            }
        } else {
            let _ = tx.send(ChildMsg::Eof(Pipe::Stderr));
        }
        *guard = Some(child);
        drop(guard);
        Ok(Running {
            argv,
            pid,
            slot,
            rx,
            stdout_eof: false,
            stderr_eof: false,
            status: None,
        })
    }

    /// The command line, program first.
    pub fn argv(&self) -> &[OsString] {
        &self.argv
    }

    /// The child's pid (also its process group id).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Every message the readers produced so far (never blocks).
    pub fn drain(&mut self) -> Vec<ChildMsg> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    match msg {
                        ChildMsg::Eof(Pipe::Stdout) => self.stdout_eof = true,
                        ChildMsg::Eof(Pipe::Stderr) => self.stderr_eof = true,
                        _ => {}
                    }
                    out.push(msg);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Both readers are gone; whatever they did not report
                    // is EOF.
                    self.stdout_eof = true;
                    self.stderr_eof = true;
                    break;
                }
            }
        }
        out
    }

    /// Reap the child if it has exited (never blocks); the status is kept.
    pub fn poll_exit(&mut self) -> std::io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            let mut guard = lock(&self.slot);
            if let Some(child) = guard.as_mut() {
                if let Some(status) = child.try_wait()? {
                    self.status = Some(status);
                    *guard = None;
                }
            }
        }
        Ok(self.status)
    }

    /// The exit status, once the command is OVER: both readers at EOF and
    /// the child reaped.
    pub fn finished(&self) -> Option<ExitStatus> {
        self.status.filter(|_| self.stdout_eof && self.stderr_eof)
    }

    /// Ask the command to cancel: `/bin/kill -INT -- -<pgid>` (the child's
    /// whole process group, which it leads) — only while the child has not
    /// been reaped. `Ok(false)` when it already exited.
    pub fn interrupt(&mut self) -> std::io::Result<bool> {
        interrupt(&self.slot)
    }
}

/// `/bin/kill -INT -- -<pgid>` of the child in `slot` — its whole process
/// group (the child leads it: a wrapper script's own children, such as the
/// real CLI under a `sh` that does not `exec`, are reached too, while the
/// CLI's sandboxed groups are its own to kill) — only while `try_wait` says
/// the leader has not been reaped (the slot's lock is held throughout, so
/// the pid cannot be reaped and reused in between). `Ok(false)` when there
/// is no live child.
pub fn interrupt(slot: &ChildSlot) -> std::io::Result<bool> {
    let mut guard = lock(slot);
    let Some(child) = guard.as_mut() else {
        return Ok(false);
    };
    if child.try_wait()?.is_some() {
        return Ok(false);
    }
    let status = Command::new("/bin/kill")
        .args(["-INT", "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

/// [`interrupt`] for a path that must never block (a panic hook, which may
/// run on a thread that holds the slot's lock): `Ok(false)` when the slot
/// is busy or empty.
pub fn try_interrupt(slot: &ChildSlot) -> std::io::Result<bool> {
    let mut guard = match slot.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
    };
    let Some(child) = guard.as_mut() else {
        return Ok(false);
    };
    if child.try_wait()?.is_some() {
        return Ok(false);
    }
    let status = Command::new("/bin/kill")
        .args(["-INT", "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

/// The client's signal path (docs/TUI-DESIGN.md §4 "The TUI's own
/// signals"): interrupt the running child, then wait at most `budget` for
/// it to end (the CLI's 250 ms courtesy budget plus its group kill). The
/// exit status when it ended in time; `None` when there was no child or it
/// is still running (it runs on to completion, its timeouts enforced).
pub fn interrupt_and_wait(slot: &ChildSlot, budget: Duration) -> Option<ExitStatus> {
    let _ = interrupt(slot);
    let deadline = Instant::now() + budget;
    loop {
        {
            let mut guard = lock(slot);
            if let Ok(Some(status)) = guard.as_mut()?.try_wait() {
                return Some(status);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn sh(script: &str) -> Vec<OsString> {
        ["/bin/sh", "-c", script].map(OsString::from).to_vec()
    }

    fn run_to_end(running: &mut Running) -> (Vec<ChildMsg>, ExitStatus) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut msgs = Vec::new();
        loop {
            msgs.extend(running.drain());
            running.poll_exit().unwrap();
            if let Some(status) = running.finished() {
                return (msgs, status);
            }
            assert!(Instant::now() < deadline, "the command never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn events_stderr_and_the_exit_arrive_and_the_slot_empties() {
        let slot = ChildSlot::default();
        let mut running = Running::spawn(
            sh(r#"echo '{"k":"message","text":"hi"}'; echo oops >&2; echo 'plain'; exit 3"#),
            slot.clone(),
        )
        .unwrap();
        let (msgs, status) = run_to_end(&mut running);
        assert_eq!(status.code(), Some(3));
        assert!(msgs.contains(&ChildMsg::Event(Event::Message { text: "hi".into() })));
        assert!(msgs.contains(&ChildMsg::Event(Event::NotJson {
            line: "plain".into()
        })));
        assert!(msgs.contains(&ChildMsg::Stderr("oops".into())));
        assert!(lock(&slot).is_none(), "the reaped child left the slot");
        assert!(!running.interrupt().unwrap(), "nothing left to signal");
    }

    /// §4: the command is over after EOF AND reaping — a `result` event,
    /// or even EOF, is not the end while the process lives.
    #[test]
    fn a_command_is_over_only_when_reaped_after_eof() {
        let slot = ChildSlot::default();
        let mut running = Running::spawn(
            sh(r#"echo '{"k":"result","exit":0}'; exec >&- 2>&-; sleep 1"#),
            slot,
        )
        .unwrap();
        let start = Instant::now();
        let mut saw_result_at = None;
        loop {
            for msg in running.drain() {
                if matches!(msg, ChildMsg::Event(Event::Result { .. })) {
                    saw_result_at = Some(start.elapsed());
                }
            }
            running.poll_exit().unwrap();
            if running.finished().is_some() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
        let over = start.elapsed();
        let saw = saw_result_at.expect("the result event");
        assert!(
            over >= Duration::from_millis(900) && over > saw,
            "over at {over:?}, result at {saw:?}"
        );
    }

    #[test]
    fn the_child_leads_its_own_process_group() {
        let slot = ChildSlot::default();
        let mut running = Running::spawn(sh("exec sleep 5"), slot.clone()).unwrap();
        let out = Command::new("ps")
            .args(["-o", "pgid=", "-p", &running.pid().to_string()])
            .output()
            .unwrap();
        let pgid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        assert_eq!(pgid, running.pid());
        assert!(running.interrupt().unwrap());
        let (_, status) = run_to_end(&mut running);
        assert_eq!(status.signal(), Some(2));
    }

    #[test]
    fn the_signal_path_interrupts_and_waits_within_its_budget() {
        // A child that exits on INT: reaped within the budget.
        let slot = ChildSlot::default();
        let mut running = Running::spawn(
            sh("trap 'exit 42' INT; while :; do sleep 0.05; done"),
            slot.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let status = interrupt_and_wait(&slot, Duration::from_secs(2)).expect("ended in time");
        assert_eq!(status.code(), Some(42));
        assert!(
            !interrupt(&slot).unwrap(),
            "a reaped child is never signalled"
        );
        // Its owner still sees the end.
        assert_eq!(run_to_end(&mut running).1.code(), Some(42));
        // A child that ignores INT: the path gives up after the budget.
        let slot = ChildSlot::default();
        let running = Running::spawn(
            sh("trap '' INT; while :; do sleep 0.05; done"),
            slot.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        assert_eq!(interrupt_and_wait(&slot, Duration::from_millis(300)), None);
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(300) && waited < Duration::from_secs(2));
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &running.pid().to_string()])
            .status();
        // No child at all.
        assert_eq!(
            interrupt_and_wait(&ChildSlot::default(), Duration::from_millis(50)),
            None
        );
    }

    /// A wrapper that does not `exec` (a script around the CLI): the
    /// interrupt reaches the process under it too, so nothing outlives the
    /// wrapper holding the pipes (MCP-DESIGN §R2 PROTO-3).
    #[test]
    fn the_interrupt_reaches_the_whole_process_group() {
        let slot = ChildSlot::default();
        let mut running = Running::spawn(sh("sleep 30; echo never"), slot).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let sleeper = loop {
            let out = Command::new("pgrep")
                .args(["-P", &running.pid().to_string()])
                .output()
                .unwrap();
            if let Some(pid) = String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|l| l.trim().parse::<u32>().ok())
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "no sleep under the shell");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(running.interrupt().unwrap());
        let (msgs, status) = run_to_end(&mut running);
        assert!(
            status.signal().is_some() || status.code() != Some(0),
            "{status:?}"
        );
        assert!(!msgs.contains(&ChildMsg::Stderr("never".into())));
        let alive = Command::new("/bin/kill")
            .args(["-0", &sleeper.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the sleep under the shell was interrupted too");
    }

    #[test]
    fn one_command_at_a_time() {
        let slot = ChildSlot::default();
        let mut first = Running::spawn(sh("exec sleep 5"), slot.clone()).unwrap();
        assert!(Running::spawn(sh("true"), slot.clone()).is_err());
        first.interrupt().unwrap();
        run_to_end(&mut first);
        let mut second = Running::spawn(sh("true"), slot).unwrap();
        assert_eq!(run_to_end(&mut second).1.code(), Some(0));
    }
}
