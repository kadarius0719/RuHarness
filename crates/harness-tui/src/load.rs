//! Reading a target off the UI thread (docs/COCKPIT-WRAPPER-DESIGN.md §6.3):
//! every read runs the [`preflight`](crate::preflight) first, then
//! [`Snapshot::load`]; a [`Loader`] runs them on its own thread, one at a
//! time, the latest request winning, so the cockpit keeps answering keys
//! (`x`, `q`, `Ctrl-C` above all) while a large or slow target is read.

use crate::model::Snapshot;
use crate::preflight;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Read the target at `target`: the preflight, then the snapshot. `Err` is
/// a reason in words.
pub fn read(target: &Path) -> Result<Snapshot, String> {
    preflight::preflight(target)?;
    Snapshot::load(target).map_err(|e| e.to_string())
}

/// What a load produced.
#[derive(Debug)]
pub struct Loaded {
    /// The request it answers (requests are numbered from 1).
    pub seq: u64,
    /// The snapshot, or why the target could not be read.
    pub result: Result<Snapshot, String>,
}

/// The function a [`Loader`] runs ([`read`], or a test's stand-in).
pub type ReadFn = fn(&Path) -> Result<Snapshot, String>;

/// A loader thread.
#[derive(Debug)]
pub struct Loader {
    requests: Sender<(u64, PathBuf)>,
    results: Receiver<Loaded>,
    requested: u64,
    received: u64,
}

impl Loader {
    /// Start the thread; it runs `read` for each request.
    pub fn spawn(read: ReadFn) -> std::io::Result<Loader> {
        let (requests, inbox) = mpsc::channel::<(u64, PathBuf)>();
        let (outbox, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("harness-tui-loader".into())
            .spawn(move || {
                while let Ok(mut next) = inbox.recv() {
                    // The latest request wins: whatever queued meanwhile is
                    // answered by one read.
                    while let Ok(later) = inbox.try_recv() {
                        next = later;
                    }
                    let (seq, target) = next;
                    let result = read(&target);
                    if outbox.send(Loaded { seq, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Loader {
            requests,
            results,
            requested: 0,
            received: 0,
        })
    }

    /// Ask for a read of `target`; its number.
    pub fn request(&mut self, target: &Path) -> u64 {
        self.requested += 1;
        // A dead thread shows as a load that never ends ("reading…").
        let _ = self.requests.send((self.requested, target.to_path_buf()));
        self.requested
    }

    /// The newest finished load, if any finished since the last call (older
    /// ones are dropped: the newest is the truth). Never blocks.
    pub fn poll(&mut self) -> Option<Loaded> {
        let mut newest = None;
        loop {
            match self.results.try_recv() {
                Ok(loaded) => {
                    self.received = self.received.max(loaded.seq);
                    newest = Some(loaded);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return newest,
            }
        }
    }

    /// A request is not answered yet.
    pub fn loading(&self) -> bool {
        self.received < self.requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn slow(target: &Path) -> Result<Snapshot, String> {
        std::thread::sleep(Duration::from_millis(300));
        Err(format!("slow {}", target.display()))
    }

    fn wait(loader: &mut Loader) -> Loaded {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(l) = loader.poll() {
                return l;
            }
            assert!(Instant::now() < deadline, "the load never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// §13 Freshness: a slow load keeps the caller answering; the latest
    /// request wins.
    #[test]
    fn a_slow_load_never_blocks_and_the_latest_request_wins() {
        let mut loader = Loader::spawn(slow).unwrap();
        let start = Instant::now();
        loader.request(Path::new("/a"));
        assert!(loader.poll().is_none());
        assert!(loader.loading());
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "request blocked"
        );
        // Two more while the first runs (it has started by now): one read
        // answers both.
        std::thread::sleep(Duration::from_millis(100));
        loader.request(Path::new("/b"));
        loader.request(Path::new("/c"));
        let first = wait(&mut loader);
        assert_eq!(first.seq, 1);
        let last = wait(&mut loader);
        assert_eq!(last.seq, 3, "/b is never read");
        assert_eq!(last.result.unwrap_err(), "slow /c");
        assert!(!loader.loading());
    }

    /// SAFE-6: the cockpit's read runs the preflight first — a target the
    /// preflight refuses is never loaded.
    #[test]
    fn a_read_runs_the_preflight_first() {
        let dir = std::env::temp_dir().join(format!("harness-tui-read-pf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let case = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib");
        assert!(std::process::Command::new("rsync")
            .args([
                "-a",
                "--exclude=target",
                "--exclude=build",
                "--exclude=.lock"
            ])
            .arg(format!("{}/", case.display()))
            .arg(&dir)
            .status()
            .unwrap()
            .success());
        assert!(read(&dir).is_ok(), "the copy reads");
        // A facts file past the cap (sparse): the loader alone would read
        // 64 MiB of zeros and report a parse error.
        std::fs::File::create(dir.join("migration/facts.jsonl"))
            .unwrap()
            .set_len(preflight::MAX_PROJECT_FILE_BYTES + 1)
            .unwrap();
        let err = read(&dir).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
