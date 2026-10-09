//! Ledger layout: where migration state lives inside a target repo
//! (docs/SCHEMAS.md). All paths derive from `<target root>/migration/`.
//! Also the ledger's writer lock (docs/CLI-HARDENING.md §1).

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Atomically replace `path` with `bytes`: write to a temp file in the same
/// directory, then rename over the target. An interrupted write can never
/// truncate or corrupt committed ledger evidence (docs/SCHEMAS.md).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| Error::Invariant(format!("{} has no parent dir", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ledger".into());
    // The temp file is created NEW (O_CREAT|O_EXCL: never through a planted
    // symlink or over an existing file) under an unpredictable name — the
    // ledger's directories are target-owned (§R4 CR-3).
    let (tmp, mut file) = new_temp_file(dir, &name)?;
    let written = file.write_all(bytes);
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::io(&tmp, e));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path, e)
    })
}

/// A new, exclusively created temp file `.<name>.tmp-<pid>-<random>` in
/// `dir` (O_CREAT|O_EXCL, so a planted file or symlink of that name is never
/// written through), retried under another name on a collision.
pub fn new_temp_file(dir: &Path, name: &str) -> Result<(PathBuf, std::fs::File), Error> {
    let mut last = None;
    for _ in 0..8 {
        let tmp = dir.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            crate::hash::random_hex(8)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(Error::io(&tmp, e)),
        }
    }
    Err(Error::io(
        dir,
        last.unwrap_or_else(|| std::io::Error::other("no free temp name")),
    ))
}

/// The bytes of `path` — a regular file (never read through a symlink) of at
/// most `max` bytes — read through a handle checked to be the very file
/// looked at: the ledger is target-owned, so a swap between the look and the
/// open is refused (`O_NOFOLLOW`, then the handle's identity), a FIFO never
/// blocks the open (`O_NONBLOCK`), and a file that grows is never read past
/// `max` (§R4 CR-11).
pub fn read_regular(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let refuse = |why: String| Error::Invariant(format!("{}: {why}", path.display()));
    let seen = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if !seen.file_type().is_file() {
        return Err(refuse("not a regular file (symlinks are refused)".into()));
    }
    if seen.len() > max {
        return Err(refuse(format!("longer than {max} bytes")));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| Error::io(path, e))?;
    let meta = file.metadata().map_err(|e| Error::io(path, e))?;
    if !meta.is_file() || meta.dev() != seen.dev() || meta.ino() != seen.ino() {
        return Err(refuse("the file changed while it was opened".into()));
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    if bytes.len() as u64 > max {
        return Err(refuse(format!("longer than {max} bytes")));
    }
    Ok(bytes)
}

/// Directory name of the ledger inside a target repo.
pub const MIGRATION_DIR: &str = "migration";

/// File name of the writer lock inside the ledger dir (gitignored).
pub const LOCK_FILE: &str = ".lock";

/// Path helpers over a ledger folder. Pure path arithmetic, no I/O.
#[derive(Debug, Clone)]
pub struct Ledger {
    root: PathBuf,
    dir: PathBuf,
}

impl Ledger {
    /// The ledger of a folder-form target: `<target_root>/migration/`.
    /// A target that was loaded uses [`Ledger::of`], which is right for a
    /// mapped tool too.
    pub fn new(target_root: impl Into<PathBuf>) -> Ledger {
        let root = target_root.into();
        Ledger {
            dir: root.join(MIGRATION_DIR),
            root,
        }
    }

    /// The ledger of a loaded target: its ledger folder
    /// (`migration/` or `migration/tools/<id>/`, docs/SCHEMAS.md "Ledger
    /// layout").
    pub fn of(ctx: &crate::config::TargetContext) -> Ledger {
        Ledger {
            root: ctx.root.clone(),
            dir: ctx.ledger.clone(),
        }
    }

    /// The ledger folder `dir` of the target at `root`.
    pub fn at(root: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Ledger {
        Ledger {
            root: root.into(),
            dir: dir.into(),
        }
    }

    /// The ledger of a loaded target over another spelling of its root (the
    /// canonical one a caller resolved): the same ledger folder, under
    /// `root`.
    pub fn of_under(ctx: &crate::config::TargetContext, root: impl Into<PathBuf>) -> Ledger {
        let root = root.into();
        Ledger {
            dir: root.join(ctx.ledger_rel()),
            root,
        }
    }

    /// The ledger folder: `<root>/migration/` or
    /// `<root>/migration/tools/<id>/`.
    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    /// The canonical committed fact file.
    pub fn facts_path(&self) -> PathBuf {
        self.dir().join("facts.jsonl")
    }

    /// The plan contract file.
    pub fn plan_path(&self) -> PathBuf {
        self.dir().join("plan.toml")
    }

    /// A unit's directory.
    pub fn unit_dir(&self, unit_id: &str) -> PathBuf {
        self.dir().join("units").join(unit_id)
    }

    /// A unit's latest verdict (green or red).
    pub fn verdict_latest_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-latest.json")
    }

    /// A unit's last green verdict (preserved across red re-runs).
    pub fn verdict_last_green_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-last-green.json")
    }

    /// The human rendering of the latest verdict.
    pub fn verdict_md_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("oracle-latest.md")
    }

    /// A unit's generated (or human-written) differential driver.
    pub fn driver_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("driver.c")
    }
    /// A unit's promoted driver self-validation record (M4).
    pub fn driver_validation_path(&self, unit_id: &str) -> PathBuf {
        self.unit_dir(unit_id).join("driver-validation.json")
    }
    /// Gitignored scratch/build directory for oracle runs.
    pub fn build_dir(&self) -> PathBuf {
        self.dir().join("build")
    }

    /// The writer lock file (gitignored, never deleted).
    pub fn lock_path(&self) -> PathBuf {
        self.dir().join(LOCK_FILE)
    }

    /// The target root this ledger belongs to.
    pub fn target_root(&self) -> &Path {
        &self.root
    }
}

// ---------- the writer lock (docs/CLI-HARDENING.md §1) ----------

/// Longest holder line read back.
const MAX_HOLDER_BYTES: u64 = 4096;
/// Longest `command` echoed from a holder line (it is target-tree text).
const MAX_COMMAND_BYTES: usize = 80;

/// What a lock holder wrote about itself: diagnostics only, never consulted
/// to decide staleness (the kernel releases the lock when the holder dies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    /// The holder's process id.
    pub pid: u32,
    /// The subcommand line it ran (printable, bounded).
    pub command: String,
    /// When it acquired the lock, RFC 3339 UTC (wall-clock is fine: the file
    /// is gitignored and non-canonical).
    pub started: String,
}

/// `describe_holder(Some(h))` = ``pid 4242, `migrate u-lib`, since 2026-…``;
/// `None` = "holder record not yet written". Used by [`Error::Locked`].
pub fn describe_holder(holder: &Option<Holder>) -> String {
    match holder {
        Some(h) => format!("pid {}, `{}`, since {}", h.pid, h.command, h.started),
        None => "holder record not yet written".to_string(),
    }
}

/// The ledger's one-writer lock: an advisory exclusive `flock(2)` on
/// `migration/.lock`, held on the open file description for the life of the
/// value and released by the kernel when the holder exits or dies — so a
/// crash never leaves a stale lock. The `File` is owned and never cloned
/// out; the path is never written through [`write_atomic`] (its rename
/// would swap the inode from under the lock).
#[derive(Debug)]
pub struct WriterLock {
    file: std::fs::File,
    path: PathBuf,
}

impl WriterLock {
    /// Take the lock for `command` (a short subcommand line for diagnostics).
    /// Non-blocking: another holder is [`Error::Locked`]. The ledger dir is
    /// created when absent; a symlink in place of the dir or the lock file,
    /// or a hard-linked lock file, is refused — the holder line is the
    /// harness's one in-place write into target-owned space, and it never
    /// truncates through a link (docs/CLI-HARDENING.md §1 "Open protocol").
    pub fn acquire(ledger: &Ledger, command: &str) -> Result<WriterLock, Error> {
        let dir = ledger.dir();
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                return Err(Error::Invariant(format!(
                    "{} is not a directory (symlinks are refused); refusing to lock the ledger",
                    dir.display()
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
            }
            Err(e) => return Err(Error::io(&dir, e)),
        }
        let path = ledger.lock_path();
        let file = open_lock_file(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                // A brand-new holder may not have written its line yet.
                let mut holder = read_holder(&path)?;
                if holder.is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    holder = read_holder(&path)?;
                }
                return Err(Error::Locked { holder });
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::io(&path, e)),
        }
        // Only under the lock: truncate and write the holder line.
        let holder = Holder {
            pid: std::process::id(),
            command: printable(command, MAX_COMMAND_BYTES),
            started: rfc3339_utc(unix_now()),
        };
        let mut line = serde_json::to_string(&holder)
            .map_err(|e| Error::Invariant(format!("serialize lock holder: {e}")))?;
        line.push('\n');
        let mut lock = WriterLock { file, path };
        lock.file.set_len(0).map_err(|e| Error::io(&lock.path, e))?;
        lock.file
            .write_all(line.as_bytes())
            .and_then(|()| lock.file.flush())
            .map_err(|e| Error::io(&lock.path, e))?;
        Ok(lock)
    }

    /// The current holder's record, by READING the lock file — never by
    /// locking (flock has no query, and a try-lock probe would knock over a
    /// real writer's fail-fast acquisition). `None` = no holder line: the
    /// last holder released cleanly, or nothing ever locked. `Some` = a live
    /// holder, or one that died without cleanup (its pid is reported).
    pub fn holder(ledger: &Ledger) -> Result<Option<Holder>, Error> {
        read_holder(&ledger.lock_path())
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        // Still exclusive here: no reader sees a torn line. A crash skips
        // this and the kernel frees the lock anyway; the stale line then
        // names the dead pid.
        let _ = self.file.set_len(0);
    }
}

// ---------- the project lock and the map's notice (docs/PROJECT-MAP-DESIGN.md §3.7) ----------

/// The project-level folder of the map, relative to the project root.
pub const MAP_DIR: &str = "migration/map";
/// The project map file, relative to the project root.
pub const PROJECT_MAP_FILE: &str = "migration/map/project-map.json";
/// The largest map file the notice reads.
const MAX_MAP_BYTES: u64 = 256 << 20;

impl WriterLock {
    /// The project lock, `migration/map/.lock`: the same lock as a ledger's,
    /// taken by `project map` (and later `accept`) and by `sync-runtime` for
    /// the shared root `AGENTS.md`/`CLAUDE.md`. `migration/` and
    /// `migration/map/` are made when absent; a link in place of either is
    /// refused. Another holder is [`Error::Locked`].
    pub fn acquire_project(root: &Path, command: &str) -> Result<WriterLock, Error> {
        let migration = root.join(MIGRATION_DIR);
        match std::fs::symlink_metadata(&migration) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                return Err(Error::Invariant(format!(
                    "{} is not a directory (symlinks are refused); refusing to lock the project",
                    migration.display()
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&migration).map_err(|e| Error::io(&migration, e))?;
            }
            Err(e) => return Err(Error::io(&migration, e)),
        }
        WriterLock::acquire(&Ledger::at(root, root.join(MAP_DIR)), command)
    }
}

/// The "project changed" notice of a mapped tool (docs/PROJECT-MAP-DESIGN.md
/// §3.7): for a tool whose `harness.toml` carries `map = {root_hash,
/// inputs_hash}`, the sentence the project map recorded for it in
/// `accepted_tools` (`{id, changed, says}`), so `state status`, the cockpit
/// and harness-mcp all say what `project map` said. Cheap: the map file is
/// read, nothing is hashed. `None` for a folder-form target, a hand-written
/// tool with no `map`, digests that match (accepted under this map), or a
/// record whose `changed` is `none` (only a file elsewhere in the project
/// changed). A map with no record for the tool (one written before records)
/// gives the general sentence. A notice, never staleness.
pub fn project_changed_notice(ctx: &crate::config::TargetContext) -> Option<String> {
    let stamp = ctx.config.target.file_list()?.map.as_ref()?;
    let path = ctx.root.join(PROJECT_MAP_FILE);
    #[derive(Deserialize)]
    struct Record {
        id: String,
        changed: String,
        says: String,
    }
    #[derive(Deserialize)]
    struct Digests {
        root_hash: String,
        inputs_hash: String,
        #[serde(default)]
        accepted_tools: Vec<Record>,
    }
    // `None`: no map file; `Some(None)`: one that cannot be read.
    let read = || -> Option<Option<Digests>> {
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.file_type().is_file() || meta.len() > MAX_MAP_BYTES {
            return Some(None);
        }
        let bytes = std::fs::read(&path).ok()?;
        Some(serde_json::from_slice::<Digests>(&bytes).ok())
    };
    let bare = |h: &str| {
        h.strip_prefix(crate::hash::HASH_PREFIX)
            .unwrap_or(h)
            .to_string()
    };
    // The map holds no record for the tool (written before records).
    let changed_since = || {
        "the project changed since this tool was accepted: run `harness project map`, then \
         `accept` again"
            .to_string()
    };
    match read() {
        None => Some(
            "no map written yet: run `harness project map` to see whether the project changed \
             since this tool was accepted"
                .into(),
        ),
        Some(None) => Some(format!(
            "the project map ({PROJECT_MAP_FILE}) could not be read: run `harness project map` \
             again"
        )),
        Some(Some(d))
            if bare(&d.root_hash) == bare(&stamp.root_hash)
                && bare(&d.inputs_hash) == bare(&stamp.inputs_hash) =>
        {
            None
        }
        Some(Some(d)) => match (ctx.tool.as_deref(), d.accepted_tools) {
            (Some(id), records) => match records.into_iter().find(|r| r.id == id) {
                Some(r) if r.changed == "none" => None,
                // The map file lives in the project: one line, filtered.
                Some(r) => Some(crate::text::safe_line(&r.says)),
                None => Some(changed_since()),
            },
            (None, _) => Some(changed_since()),
        },
    }
}

/// Open the lock file without ever following a link: a missing file is
/// created with `O_EXCL`; an existing one must be a regular, un-hard-linked
/// file both before (lstat) and after (fstat, same device and inode) the
/// open.
fn open_lock_file(path: &Path) -> Result<std::fs::File, Error> {
    for attempt in 0..2 {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(path)
                {
                    Ok(file) => return Ok(file),
                    // Lost a create race (or a link appeared): look again, once.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                        continue
                    }
                    Err(e) => return Err(Error::io(path, e)),
                }
            }
            Err(e) => return Err(Error::io(path, e)),
            Ok(before) => {
                if !is_plain_file(&before) {
                    return Err(not_plain(path));
                }
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|e| Error::io(path, e))?;
                let after = file.metadata().map_err(|e| Error::io(path, e))?;
                if !is_plain_file(&after) || !same_inode(&before, &after) {
                    return Err(not_plain(path));
                }
                return Ok(file);
            }
        }
    }
    Err(Error::Invariant(format!(
        "{} keeps changing underneath the harness; refusing to lock the ledger",
        path.display()
    )))
}

fn not_plain(path: &Path) -> Error {
    Error::Invariant(format!(
        "{} is not a regular file (symlinks and hard links are refused); refusing to lock the \
         ledger",
        path.display()
    ))
}

fn is_plain_file(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.file_type().is_file() && meta.nlink() == 1
    }
    #[cfg(not(unix))]
    {
        meta.file_type().is_file()
    }
}

fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (a.dev(), a.ino()) == (b.dev(), b.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        true
    }
}

/// The first line of the lock file, parsed; `None` for a missing or empty
/// file, or one whose line does not parse (a torn write in progress).
fn read_holder(path: &Path) -> Result<Option<Holder>, Error> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(not_plain(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(path, e)),
    }
    let file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut text = String::new();
    if file
        .take(MAX_HOLDER_BYTES)
        .read_to_string(&mut text)
        .is_err()
    {
        return Ok(None);
    }
    let Some(line) = text.lines().next() else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<Holder>(line).ok().map(|h| Holder {
        command: printable(&h.command, MAX_COMMAND_BYTES),
        ..h
    }))
}

/// `text` reduced to printable ASCII and cut to `max_bytes`.
fn printable(text: &str, max_bytes: usize) -> String {
    text.chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .take(max_bytes)
        .collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ` (civil-from-days,
/// proleptic Gregorian; no crate).
fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ruharness-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// §R4 CR-11: a target-owned file is read only as the regular file
    /// looked at, bounded — a symlink refused, a file over the bound refused
    /// (a FIFO: tests/core_tests.rs, since making one forks, and a fork in
    /// this binary can hold the lock tests' lock).
    #[test]
    fn read_regular_reads_only_a_bounded_regular_file() {
        let root = scratch("readregular");
        let file = root.join("f.json");
        std::fs::write(&file, "0123456789").unwrap();
        assert_eq!(read_regular(&file, 10).unwrap(), b"0123456789");
        let err = read_regular(&file, 9).unwrap_err().to_string();
        assert!(err.contains("longer than 9 bytes"), "{err}");
        let link = root.join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let err = read_regular(&link, 10).unwrap_err().to_string();
        assert!(err.contains("symlinks are refused"), "{err}");
        assert!(read_regular(&root.join("missing"), 10).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// §R4 CR-3: the temp file is created new under an unpredictable name —
    /// a symlink planted at the old predictable name (or any name) is never
    /// written through — and nothing is left behind.
    #[test]
    fn write_atomic_never_writes_through_a_planted_temp_name() {
        let root = scratch("atomic");
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "untouched").unwrap();
        let dir = root.join("ledger");
        std::fs::create_dir_all(&dir).unwrap();
        let planted = dir.join(format!(".attempt.json.tmp-{}", std::process::id()));
        std::os::unix::fs::symlink(&outside, &planted).unwrap();
        let path = dir.join("attempt.json");
        write_atomic(&path, b"new").unwrap();
        write_atomic(&path, b"newer").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "newer");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "untouched");
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let planted_name = planted.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(left, [planted_name, "attempt.json".to_string()]);
        let (a, _) = new_temp_file(&dir, "x").unwrap();
        let (b, _) = new_temp_file(&dir, "x").unwrap();
        assert_ne!(a, b, "unpredictable, never reused");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rfc3339_rendering() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_790_000_000), "2026-09-21T14:13:20Z");
    }

    /// The project lock is `migration/map/.lock`, made with its folders,
    /// apart from the ledger's own lock; a link in place of `migration/` is
    /// refused.
    #[test]
    fn the_project_lock_lives_in_the_map_folder_and_refuses_a_link() {
        let root = scratch("project");
        let lock = WriterLock::acquire_project(&root, "project map").unwrap();
        assert_eq!(lock.path(), root.join("migration/map/.lock"));
        assert!(matches!(
            WriterLock::acquire_project(&root, "sync-runtime"),
            Err(Error::Locked { holder: Some(_) })
        ));
        // The ledger's own lock is another lock.
        let ledger = WriterLock::acquire(&Ledger::new(&root), "scan").unwrap();
        drop((lock, ledger));
        WriterLock::acquire_project(&root, "project map").unwrap();

        let linked = scratch("project-link");
        std::fs::create_dir_all(linked.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(linked.join("elsewhere"), linked.join("migration")).unwrap();
        let err = WriterLock::acquire_project(&linked, "project map").unwrap_err();
        assert!(err.to_string().contains("symlinks are refused"), "{err}");
        assert!(!linked.join("elsewhere/map").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&linked);
    }

    #[test]
    fn the_lock_excludes_a_second_holder_in_the_same_process_and_releases_on_drop() {
        let root = scratch("excl");
        let ledger = Ledger::new(&root);
        assert!(WriterLock::holder(&ledger).unwrap().is_none());
        let first = WriterLock::acquire(&ledger, "migrate u-lib").unwrap();
        // flock contends per open file description — unlike fcntl, a second
        // open in the SAME process is refused too.
        match WriterLock::acquire(&ledger, "verify u-lib") {
            Err(Error::Locked { holder: Some(h) }) => {
                assert_eq!(h.pid, std::process::id());
                assert_eq!(h.command, "migrate u-lib");
                assert!(h.started.ends_with('Z'), "{}", h.started);
            }
            other => panic!("expected Locked with a holder, got {other:?}"),
        }
        let seen = WriterLock::holder(&ledger)
            .unwrap()
            .expect("holder visible");
        assert_eq!(seen.command, "migrate u-lib");
        drop(first);
        assert!(WriterLock::holder(&ledger).unwrap().is_none());
        let second = WriterLock::acquire(&ledger, "verify u-lib").unwrap();
        assert!(second.path().ends_with(".lock"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_reader_never_makes_a_writer_fail() {
        let root = scratch("reader");
        let ledger = Ledger::new(&root);
        // Warm up so the file exists.
        drop(WriterLock::acquire(&ledger, "scan").unwrap());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let ledger = ledger.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut reads = 0usize;
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = WriterLock::holder(&ledger).unwrap();
                    reads += 1;
                }
                reads
            })
        };
        for i in 0..1000 {
            let lock = WriterLock::acquire(&ledger, "verify u")
                .unwrap_or_else(|e| panic!("acquisition {i} failed: {e}"));
            drop(lock);
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(reader.join().unwrap() > 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_is_refused_and_never_truncated() {
        let root = scratch("symlink");
        let ledger = Ledger::new(&root);
        std::fs::create_dir(ledger.dir()).unwrap();
        let victim = ledger.dir().join("plan.toml");
        std::fs::write(&victim, "[unit]\n").unwrap();
        std::os::unix::fs::symlink(&victim, ledger.lock_path()).unwrap();
        let err = WriterLock::acquire(&ledger, "scan")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a regular file"), "{err}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "[unit]\n");
        assert!(WriterLock::holder(&ledger).is_err());
        // A symlinked ledger dir is refused before anything is opened.
        let root2 = scratch("symlink-dir");
        std::os::unix::fs::symlink(ledger.dir(), Ledger::new(&root2).dir()).unwrap();
        let err = WriterLock::acquire(&Ledger::new(&root2), "scan")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a directory"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_lock_file_is_refused_and_never_truncated() {
        let root = scratch("hardlink");
        let ledger = Ledger::new(&root);
        std::fs::create_dir(ledger.dir()).unwrap();
        let victim = ledger.dir().join("facts.jsonl");
        std::fs::write(&victim, "{\"k\":\"file\"}\n").unwrap();
        std::fs::hard_link(&victim, ledger.lock_path()).unwrap();
        let err = WriterLock::acquire(&ledger, "scan")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a regular file"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "{\"k\":\"file\"}\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_torn_or_foreign_holder_line_reads_as_none_and_is_bounded() {
        let root = scratch("torn");
        let ledger = Ledger::new(&root);
        std::fs::create_dir(ledger.dir()).unwrap();
        std::fs::write(ledger.lock_path(), "{\"pid\":1,\"comm").unwrap();
        assert!(WriterLock::holder(&ledger).unwrap().is_none());
        let long = "x".repeat(500);
        std::fs::write(
            ledger.lock_path(),
            format!("{{\"pid\":7,\"command\":\"{long}\\u0007\",\"started\":\"t\"}}\n"),
        )
        .unwrap();
        let h = WriterLock::holder(&ledger).unwrap().unwrap();
        assert_eq!(h.command.len(), MAX_COMMAND_BYTES);
        assert!(!h.command.contains('\u{7}'));
        let _ = std::fs::remove_dir_all(&root);
    }
}
