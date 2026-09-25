//! Reading a target off the UI thread (docs/COCKPIT-WRAPPER-DESIGN.md §6.3):
//! every read runs the [`preflight`](crate::preflight) first, then
//! [`Snapshot::load`] and the tree's walk ([`files::walk_tree`]: directory
//! entries and metadata only), and reads the writer lock's live holder; a
//! [`Loader`] runs them on its own thread, one at a
//! time, the latest request winning, so the cockpit keeps answering keys
//! (`x`, `q`, `Ctrl-C` above all) while a large or slow target is read.

use crate::files::{self, TreeWalk};
use crate::model::Snapshot;
use crate::preflight;
use harness_core::ledger::{Holder, Ledger};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// What one read of a target found.
#[derive(Debug, Clone)]
pub struct Read {
    /// The ledger.
    pub snapshot: Snapshot,
    /// The source tree.
    pub walk: TreeWalk,
    /// The writer lock's live holder, if any.
    pub holder: Option<Holder>,
    /// The target's effective migrate model (`[llm.migrate]` over `[llm]`),
    /// named in the model acts' dialogs — read here, never on the UI thread.
    pub migrate_model: String,
}

/// Read the target at `target`: the preflight, then the snapshot, the walk
/// and the lock holder. `Err` is a reason in words.
pub fn read(target: &Path) -> Result<Read, String> {
    preflight::preflight(target)?;
    let snapshot = Snapshot::load(target).map_err(|e| e.to_string())?;
    let ctx = harness_core::TargetContext::load(target).map_err(|e| e.to_string())?;
    // The tree lists only what lies inside the target (review SAFE-11): a
    // source_dir that leaves it is refused, as is one that resolves outside.
    // An empty source_dir is the root, as the scanner reads it; a missing
    // one lists nothing (the tree says why) — only one that leaves the
    // target is refused (review NEW-6/NEW-7).
    let source_dir = match ctx.config.target.source_dir.as_str() {
        "" => ".",
        dir => dir,
    };
    let clean = Path::new(source_dir).components().all(|c| {
        matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    });
    let outside = snapshot
        .root
        .join(source_dir)
        .canonicalize()
        .is_ok_and(|dir| !dir.starts_with(&snapshot.root));
    if !clean || outside {
        return Err(format!(
            "harness.toml's source_dir {source_dir:?} is not a directory inside the target"
        ));
    }
    let walk = files::walk_tree(&snapshot.root, source_dir, snapshot.facts.as_ref());
    let holder = harness_core::status::live_holder(&Ledger::new(&snapshot.root))
        .map_err(|e| e.to_string())?;
    let llm = &ctx.config.llm;
    let migrate_model = llm
        .migrate
        .as_ref()
        .and_then(|m| m.model.clone())
        .unwrap_or_else(|| llm.model.clone());
    Ok(Read {
        snapshot,
        walk,
        holder,
        migrate_model,
    })
}

/// What a load produced.
#[derive(Debug)]
pub struct Loaded {
    /// The request it answers (requests are numbered from 1).
    pub seq: u64,
    /// What it found, or why the target could not be read.
    pub result: Result<Read, String>,
}

/// The function a [`Loader`] runs ([`read`], or a test's stand-in).
pub type ReadFn = fn(&Path) -> Result<Read, String>;

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

    /// Reads started so far (the test waits on it instead of guessing).
    static STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn slow(target: &Path) -> Result<Read, String> {
        STARTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        // Two more while the first runs: one read answers both.
        let deadline = Instant::now() + Duration::from_secs(10);
        while STARTED.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the first read never started");
            std::thread::sleep(Duration::from_millis(5));
        }
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

    /// Review SAFE-11: the tree lists only what lies inside the target — a
    /// source_dir that leaves it, by path or by link, is refused.
    #[test]
    fn a_source_dir_outside_the_target_is_refused() {
        let dir = std::env::temp_dir().join(format!("harness-tui-srcdir-{}", std::process::id()));
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
        let config = dir.join("harness.toml");
        // Without include_dirs (they must lie inside source_dir).
        let text: String = std::fs::read_to_string(&config)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("include_dirs"))
            .map(|l| format!("{l}\n"))
            .collect();
        for bad in ["/", "..", "test_case/../.."] {
            std::fs::write(
                &config,
                text.replace(
                    "source_dir = \"test_case\"",
                    &format!("source_dir = {bad:?}"),
                ),
            )
            .unwrap();
            let err = read(&dir).unwrap_err();
            assert!(
                err.contains("not a directory inside the target"),
                "{bad}: {err}"
            );
        }
        std::fs::write(
            &config,
            text.replace("source_dir = \"test_case\"", "source_dir = \"out\""),
        )
        .unwrap();
        std::os::unix::fs::symlink("/usr", dir.join("out")).unwrap();
        assert!(read(&dir)
            .unwrap_err()
            .contains("not a directory inside the target"));
        // Second fix pass, NEW-6/NEW-7: an empty source_dir is the root; a
        // missing one reads, its tree saying why it is empty.
        std::fs::write(
            &config,
            text.replace("source_dir = \"test_case\"", "source_dir = \"\""),
        )
        .unwrap();
        let r = read(&dir).expect("an empty source_dir is the root");
        assert!(r.walk.listed.iter().any(|f| f == "test_case/src/lib.c"));
        std::fs::write(
            &config,
            text.replace("source_dir = \"test_case\"", "source_dir = \"gone\""),
        )
        .unwrap();
        let r = read(&dir).expect("a missing source_dir still reads");
        assert!(r.walk.listed.is_empty());
        assert_eq!(r.walk.errors.len(), 1, "{:?}", r.walk.errors);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
