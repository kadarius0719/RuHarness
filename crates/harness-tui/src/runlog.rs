//! The run log (docs/RUN-LOG-DESIGN.md): a file the cockpit appends to as
//! things happen — every command it ran with its lines, every line the chat
//! pane showed, what the cockpit told the model, every dialog's answer,
//! every notice — and the same events kept in memory for the Log view.
//!
//! The file is `<target>/migration/build/cockpit.log`: the one exception to
//! "the cockpit never writes the ledger" (docs/COCKPIT-WRAPPER-DESIGN.md
//! §0) — a diary in the folder every ledger's `.gitignore` names,
//! append-only, never read back by the cockpit or by any harness command.
//! It is never written through a link: the folder and the file are opened
//! with `O_NOFOLLOW`, owner-only, and a symbolic link or a hard-linked file
//! in their place is refused (the Log view's footer says why). One JSON
//! object per line, escaped like the CLI's `--json` stream
//! ([`harness_core::text::json_safe`]), so `tail -f` shows nothing a
//! terminal acts on. A writer thread owns the file: the screen's thread
//! never waits on a disk.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// The header line's `schema`.
pub const SCHEMA: &str = "ruharness-cockpit-log";
/// The header line's `schema_version`.
pub const SCHEMA_VERSION: u64 = 1;
/// The file's name, under the ledger's `build/` (the adoption check knows
/// it as a diary, never a result).
pub const FILE: &str = harness_core::ledger::COCKPIT_LOG_FILE;
/// The ledger's scratch folder (gitignored).
pub const BUILD_DIR: &str = "build";
/// Events kept in memory for the view (older ones are in the file).
pub const MAX_EVENTS: usize = 5_000;
/// A `raw` longer than this is cut with ` … (N more bytes)`.
pub const MAX_RAW_BYTES: usize = 4_000;

/// What kind of event a row is (`k` in the file; an open set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The cockpit's own: it ended, the log could not be written.
    Cockpit,
    /// A command: started, each line, ended.
    Command,
    /// The chat: its lines, its process, its replies, what it was told.
    Chat,
    /// A dialog answered.
    Dialog,
    /// The notice row's text changed.
    Notice,
}

impl Kind {
    /// The `k` value.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Cockpit => "cockpit",
            Kind::Command => "command",
            Kind::Chat => "chat",
            Kind::Dialog => "dialog",
            Kind::Notice => "notice",
        }
    }
}

/// How the view draws a row (the screen's tones; the file has none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Ordinary.
    #[default]
    Plain,
    /// Background: a read, a raw line, the cockpit's own rows.
    Dim,
    /// A dialog's row.
    Label,
    /// Went well.
    Good,
    /// Went badly.
    Bad,
    /// Worth a look.
    Warn,
    /// A notice, a pause.
    Notice,
    /// The person's words.
    You,
    /// The cockpit's line in the chat.
    Cockpit,
}

/// One event, as the view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// When it happened (the writer's wall clock).
    pub t: SystemTime,
    /// What kind.
    pub k: Kind,
    /// The unit it is about, when it has one.
    pub piece: Option<String>,
    /// The plain sentence.
    pub words: String,
    /// The command line, the output line, the record, the tool call — cut
    /// at [`MAX_RAW_BYTES`].
    pub raw: Option<String>,
    /// How to draw it.
    pub tone: Tone,
}

#[derive(Serialize)]
struct Line<'a> {
    k: &'static str,
    t: String,
    pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    piece: Option<&'a str>,
    words: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<&'a str>,
}

#[derive(Serialize)]
struct Header<'a> {
    k: &'static str,
    schema: &'static str,
    schema_version: u64,
    t: String,
    pid: u32,
    harness: &'static str,
    target: &'a str,
}

/// Where the lines go.
#[derive(Debug)]
enum Sink {
    /// No ledger folder yet (a target before its first map): the lines
    /// wait, and are written once it exists.
    Waiting(Vec<String>),
    /// The writer thread.
    Open {
        tx: Sender<String>,
        thread: JoinHandle<()>,
    },
    /// Closed on purpose (before an adoption, on the way out): the lines
    /// wait for [`RunLog::reopen`]; nothing opens the file meanwhile.
    Closed(Vec<String>),
    /// Refused, or a write failed: nothing more is written
    /// ([`RunLog::not_written`] says why).
    Off,
}

/// The session's log: its file and its events.
#[derive(Debug)]
pub struct RunLog {
    /// The last [`MAX_EVENTS`] events, oldest first.
    pub events: VecDeque<Event>,
    /// Events dropped from memory (they are in the file).
    pub dropped: usize,
    /// Why the file is not written, when it is not.
    pub not_written: Option<String>,
    root: PathBuf,
    pid: u32,
    sink: Sink,
    /// A write that failed, reported by the writer thread.
    failure: Arc<Mutex<Option<String>>>,
    /// A notice the app has not said yet (a refusal, a failed write).
    unsaid: Option<String>,
    /// The local clock's offset from UTC in seconds, once asked
    /// (`None`: not asked; `Some(None)`: unknown, shown in UTC).
    offset: Option<Option<i32>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `secs` since the epoch with `millis`, as RFC 3339 UTC with milliseconds.
pub fn rfc3339_ms(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let base = harness_core::ledger::rfc3339_utc(d.as_secs());
    format!("{}.{:03}Z", base.trim_end_matches('Z'), d.subsec_millis())
}

/// `raw` cut at [`MAX_RAW_BYTES`] (on a character boundary) with the count
/// of what was cut; shorter ones as they are.
pub fn cap_raw(raw: &str) -> String {
    if raw.len() <= MAX_RAW_BYTES {
        return raw.to_string();
    }
    let mut cut = MAX_RAW_BYTES;
    while !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{} … ({} more bytes)", &raw[..cut], raw.len() - cut)
}

fn plain_file(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.file_type().is_file() && meta.nlink() == 1
}

fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// The file, opened for appending and never through a link; `Ok(None)`
/// when the target has no ledger folder yet.
fn open_file(root: &Path) -> Result<Option<std::fs::File>, String> {
    use std::fs::{symlink_metadata, DirBuilder, OpenOptions};
    use std::io::ErrorKind::NotFound;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let migration = root.join(harness_core::ledger::MIGRATION_DIR);
    match symlink_metadata(&migration) {
        Err(e) if e.kind() == NotFound => return Ok(None),
        Ok(m) if m.file_type().is_dir() => {}
        Ok(_) => return Err("migration/ is not a directory (symlinks are refused)".into()),
        Err(e) => return Err(format!("migration/: {e}")),
    }
    let build = migration.join(BUILD_DIR);
    match symlink_metadata(&build) {
        Err(e) if e.kind() == NotFound => {
            DirBuilder::new()
                .mode(0o700)
                .create(&build)
                .map_err(|e| format!("migration/build/ could not be made: {e}"))?;
        }
        Ok(m) if m.file_type().is_dir() => {}
        Ok(_) => return Err("migration/build is not a directory (symlinks are refused)".into()),
        Err(e) => return Err(format!("migration/build/: {e}")),
    }
    let path = build.join(FILE);
    let refuse = || {
        format!(
            "migration/build/{FILE} is not a regular file (symlinks and hard links are refused)"
        )
    };
    let before = match symlink_metadata(&path) {
        Err(e) if e.kind() == NotFound => None,
        Ok(m) if plain_file(&m) => Some(m),
        Ok(_) => return Err(refuse()),
        Err(e) => return Err(format!("migration/build/{FILE}: {e}")),
    };
    let mut opts = OpenOptions::new();
    opts.append(true).custom_flags(libc::O_NOFOLLOW);
    if before.is_none() {
        opts.create_new(true).mode(0o600);
    }
    let file = opts.open(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            refuse()
        } else {
            format!("migration/build/{FILE} could not be opened: {e}")
        }
    })?;
    let after = file
        .metadata()
        .map_err(|e| format!("migration/build/{FILE}: {e}"))?;
    if !plain_file(&after) || before.as_ref().is_some_and(|b| !same_inode(b, &after)) {
        return Err(refuse());
    }
    Ok(Some(file))
}

impl RunLog {
    /// The log of `root`'s ledger for this cockpit (`pid`): the file opened
    /// (or refused, or waited for) and the header written.
    pub fn open(root: &Path, pid: u32) -> RunLog {
        let mut log = RunLog {
            events: VecDeque::new(),
            dropped: 0,
            not_written: None,
            root: root.to_path_buf(),
            pid,
            sink: Sink::Waiting(Vec::new()),
            failure: Arc::new(Mutex::new(None)),
            unsaid: None,
            offset: None,
        };
        log.try_open();
        log
    }

    /// Open the file when the ledger folder exists; refuse links; start the
    /// writer; write the header and the lines that waited.
    fn try_open(&mut self) {
        let held = match &mut self.sink {
            Sink::Waiting(held) => std::mem::take(held),
            _ => return,
        };
        let file = match open_file(&self.root) {
            Ok(Some(file)) => file,
            Ok(None) => {
                self.sink = Sink::Waiting(held);
                return;
            }
            Err(why) => {
                self.refused(why, held);
                return;
            }
        };
        let (tx, rx) = channel::<String>();
        let failure = Arc::clone(&self.failure);
        let spawned = std::thread::Builder::new()
            .name("cockpit-log".into())
            .spawn(move || {
                let mut file = file;
                for line in rx {
                    if let Err(e) = file.write_all(line.as_bytes()) {
                        *lock(&failure) = Some(e.to_string());
                        break;
                    }
                }
            });
        let thread = match spawned {
            Ok(t) => t,
            Err(e) => {
                self.refused(format!("the log's writer thread: {e}"), held);
                return;
            }
        };
        let header = Header {
            k: "header",
            schema: SCHEMA,
            schema_version: SCHEMA_VERSION,
            t: rfc3339_ms(SystemTime::now()),
            pid: self.pid,
            harness: env!("CARGO_PKG_VERSION"),
            target: &self.root.display().to_string(),
        };
        let mut lines = vec![to_line(&header)];
        lines.extend(held);
        for line in lines {
            let _ = tx.send(line);
        }
        self.sink = Sink::Open { tx, thread };
    }

    fn refused(&mut self, why: String, _held: Vec<String>) {
        self.unsaid = Some(format!(
            "the log is not written: {why}; the Log view still shows this session"
        ));
        self.not_written = Some(why);
        self.sink = Sink::Off;
    }

    /// One event: stamped now, written (or held), kept for the view.
    pub fn push(
        &mut self,
        k: Kind,
        piece: Option<&str>,
        words: impl Into<String>,
        raw: Option<&str>,
        tone: Tone,
    ) {
        self.push_at(SystemTime::now(), k, piece, words, raw, tone);
    }

    /// [`RunLog::push`] at a given time (tests).
    pub fn push_at(
        &mut self,
        t: SystemTime,
        k: Kind,
        piece: Option<&str>,
        words: impl Into<String>,
        raw: Option<&str>,
        tone: Tone,
    ) {
        let words = words.into();
        let raw = raw.map(cap_raw);
        let line = to_line(&Line {
            k: k.key(),
            t: rfc3339_ms(t),
            pid: self.pid,
            piece,
            words: &words,
            raw: raw.as_deref(),
        });
        self.write(line);
        self.events.push_back(Event {
            t,
            k,
            piece: piece.map(str::to_string),
            words,
            raw,
            tone,
        });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
            self.dropped += 1;
        }
    }

    fn write(&mut self, line: String) {
        if matches!(self.sink, Sink::Waiting(_)) {
            // The ledger folder may exist now (the first map made it).
            self.try_open();
        }
        match &mut self.sink {
            Sink::Waiting(held) | Sink::Closed(held) => held.push(line),
            Sink::Open { tx, .. } => {
                if tx.send(line).is_err() {
                    self.failed("the log's writer stopped".into());
                }
            }
            Sink::Off => {}
        }
    }

    fn failed(&mut self, why: String) {
        self.unsaid = Some(format!(
            "the log could not be written: {why}; the Log view still shows this session"
        ));
        self.not_written = Some(why);
        if let Sink::Open { tx, thread } = std::mem::replace(&mut self.sink, Sink::Off) {
            drop(tx);
            let _ = thread.join();
        }
    }

    /// Each loop pass: a write that failed is said once (the notice's
    /// words), and the file is left alone from then on.
    pub fn poll(&mut self) -> Option<String> {
        let failed = lock(&self.failure).take();
        if let Some(why) = failed {
            self.failed(why);
        }
        self.unsaid.take()
    }

    /// Close the file (before an adoption deletes `build/`): every line
    /// queued is written first. Lines pushed until [`RunLog::reopen`] wait.
    pub fn close(&mut self) {
        match std::mem::replace(&mut self.sink, Sink::Closed(Vec::new())) {
            Sink::Open { tx, thread } => {
                drop(tx);
                let _ = thread.join();
            }
            Sink::Waiting(held) | Sink::Closed(held) => self.sink = Sink::Closed(held),
            Sink::Off => self.sink = Sink::Off,
        }
    }

    /// Open the file again (after an adoption), with a new header; the link
    /// checks are made again, a refusal lifted or renewed.
    pub fn reopen(&mut self) {
        self.close();
        self.not_written = None;
        let held = match std::mem::replace(&mut self.sink, Sink::Off) {
            Sink::Waiting(held) | Sink::Closed(held) => held,
            _ => Vec::new(),
        };
        self.sink = Sink::Waiting(held);
        self.try_open();
    }

    /// The file is being written.
    pub fn writing(&self) -> bool {
        matches!(self.sink, Sink::Open { .. })
    }

    /// The Log view's last row: where the file is, or why it is not written.
    pub fn footer_words(&self) -> String {
        match (&self.not_written, &self.sink) {
            (Some(why), _) => {
                format!("not written: {why} — this view shows this session only")
            }
            (None, Sink::Waiting(_)) => {
                "no ledger folder yet: the log file starts with the first map — this view shows \
                 this session"
                    .to_string()
            }
            (None, _) => {
                format!("migration/{BUILD_DIR}/{FILE} — copy this file when reporting a problem")
            }
        }
    }

    /// The local clock's offset from UTC, asked of the system once (`date
    /// +%z`: no unsafe code and no new crate); `None` when it cannot be
    /// read — the view then shows UTC and says so.
    pub fn offset(&mut self) -> Option<i32> {
        if let Some(o) = self.offset {
            return o;
        }
        let o = local_offset();
        self.offset = Some(o);
        o
    }

    /// Tests and the view's goldens: a fixed offset.
    pub fn set_offset(&mut self, offset: Option<i32>) {
        self.offset = Some(offset);
    }

    /// `t` as `HH:MM:SS` in the local zone (UTC when it is not known).
    pub fn clock(&mut self, t: SystemTime) -> String {
        hms(t, self.offset())
    }

    #[cfg(test)]
    fn fail_for_test(&self, why: &str) {
        *lock(&self.failure) = Some(why.to_string());
    }
}

/// `t` as `HH:MM:SS`, `offset` seconds east of UTC (none: UTC).
pub fn hms(t: SystemTime, offset: Option<i32>) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (secs + i64::from(offset.unwrap_or(0))).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn to_line(v: &impl Serialize) -> String {
    let json = serde_json::to_string(v).unwrap_or_else(|e| {
        format!(r#"{{"k":"cockpit","words":"a log line could not be written: {e}"}}"#)
    });
    let mut line = harness_core::text::json_safe(&json);
    line.push('\n');
    line
}

/// `date +%z` as seconds east of UTC.
fn local_offset() -> Option<i32> {
    let out = std::process::Command::new("date")
        .arg("+%z")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    let (sign, digits) = text.split_at_checked(1)?;
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[..2].parse().ok()?;
    let m: i32 = digits[2..].parse().ok()?;
    let secs = h * 3600 + m * 60;
    match sign {
        "+" => Some(secs),
        "-" => Some(-secs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TmpDir;
    use serde_json::Value;
    use std::time::Duration;

    fn lines(root: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(root.join("migration/build").join(FILE)).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect()
    }

    fn settle(log: &mut RunLog) {
        // The writer thread runs behind: close flushes it, reopen starts it
        // again with a header — so the tests read a closed file instead.
        log.close();
    }

    #[test]
    fn a_push_is_one_escaped_json_line_after_the_header() {
        let tmp = TmpDir::new("runlog-push");
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        let mut log = RunLog::open(&tmp.0, 4242);
        assert!(log.writing());
        assert_eq!(
            log.footer_words(),
            "migration/build/cockpit.log — copy this file when reporting a problem"
        );
        let t = UNIX_EPOCH + Duration::from_millis(1_762_535_553_418);
        log.push_at(
            t,
            Kind::Command,
            Some("u-lz4"),
            "Make u-lz4's test program — started (allowed by you)",
            Some("harness --json gen-driver u-lz4"),
            Tone::Plain,
        );
        log.push_at(
            t,
            Kind::Chat,
            None,
            "a line with a C1 control \u{9b} and a bidi mark \u{202E} and DEL \u{7f}",
            None,
            Tone::Dim,
        );
        settle(&mut log);
        let raw_text = std::fs::read_to_string(tmp.0.join("migration/build").join(FILE)).unwrap();
        assert!(
            raw_text.contains("\\u009b")
                && raw_text.contains("\\u202e")
                && raw_text.contains("\\u007f"),
            "{raw_text}"
        );
        assert!(
            !raw_text.contains('\u{9b}') && !raw_text.contains('\u{202E}'),
            "{raw_text}"
        );
        let got = lines(&tmp.0);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0]["k"], "header");
        assert_eq!(got[0]["schema"], SCHEMA);
        assert_eq!(got[0]["schema_version"], SCHEMA_VERSION);
        assert_eq!(got[0]["pid"], 4242);
        assert_eq!(got[0]["target"], tmp.0.display().to_string());
        assert_eq!(got[1]["k"], "command");
        assert_eq!(got[1]["t"], "2025-11-07T17:12:33.418Z");
        assert_eq!(got[1]["piece"], "u-lz4");
        assert_eq!(got[1]["raw"], "harness --json gen-driver u-lz4");
        assert_eq!(got[1]["pid"], 4242);
        assert!(got[2].get("piece").is_none() && got[2].get("raw").is_none());
        assert_eq!(
            got[2]["words"],
            "a line with a C1 control \u{9b} and a bidi mark \u{202E} and DEL \u{7f}"
        );
        // The field order is the design's.
        assert!(raw_text
            .lines()
            .nth(1)
            .unwrap()
            .starts_with(r#"{"k":"command","t":"#));
        // Owner-only, a regular file.
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(tmp.0.join("migration/build").join(FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let dir = std::fs::metadata(tmp.0.join("migration/build")).unwrap();
        assert_eq!(dir.permissions().mode() & 0o777, 0o700);
        assert_eq!(log.events.len(), 2);
    }

    #[test]
    fn raw_is_cut_at_the_limit_and_words_never() {
        let big = "x".repeat(MAX_RAW_BYTES + 100);
        let cut = cap_raw(&big);
        assert!(
            cut.ends_with(" … (100 more bytes)"),
            "{}",
            &cut[cut.len() - 40..]
        );
        assert_eq!(cut.len(), MAX_RAW_BYTES + " … (100 more bytes)".len());
        // Cut on a character boundary.
        let wide = "é".repeat(MAX_RAW_BYTES);
        let cut = cap_raw(&wide);
        assert!(cut.contains(" … ("));
        assert_eq!(cap_raw("short"), "short");
        let tmp = TmpDir::new("runlog-raw");
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        let mut log = RunLog::open(&tmp.0, 1);
        log.push(Kind::Chat, None, big.clone(), Some(&big), Tone::Plain);
        settle(&mut log);
        let got = lines(&tmp.0);
        assert_eq!(got[1]["words"].as_str().unwrap().len(), big.len());
        assert!(got[1]["raw"]
            .as_str()
            .unwrap()
            .ends_with("(100 more bytes)"));
    }

    #[test]
    fn links_are_refused_and_said_why() {
        use std::os::unix::fs::symlink;
        // A linked build/.
        let tmp = TmpDir::new("runlog-link-dir");
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        std::fs::create_dir_all(tmp.0.join("elsewhere")).unwrap();
        symlink(tmp.0.join("elsewhere"), tmp.0.join("migration/build")).unwrap();
        let mut log = RunLog::open(&tmp.0, 1);
        assert_eq!(
            log.not_written.as_deref(),
            Some("migration/build is not a directory (symlinks are refused)")
        );
        assert!(log
            .poll()
            .unwrap()
            .starts_with("the log is not written: migration/build is not"));
        assert!(log.poll().is_none(), "said once");
        log.push(Kind::Chat, None, "kept in memory", None, Tone::Plain);
        assert_eq!(log.events.len(), 1);
        assert!(std::fs::read_dir(tmp.0.join("elsewhere"))
            .unwrap()
            .next()
            .is_none());
        assert!(log
            .footer_words()
            .starts_with("not written: migration/build is not a directory"));
        // A linked file.
        let tmp = TmpDir::new("runlog-link-file");
        std::fs::create_dir_all(tmp.0.join("migration/build")).unwrap();
        std::fs::write(tmp.0.join("victim"), "").unwrap();
        symlink(
            tmp.0.join("victim"),
            tmp.0.join("migration/build").join(FILE),
        )
        .unwrap();
        let log = RunLog::open(&tmp.0, 1);
        assert_eq!(
            log.not_written.as_deref(),
            Some("migration/build/cockpit.log is not a regular file (symlinks and hard links are refused)")
        );
        assert_eq!(std::fs::read_to_string(tmp.0.join("victim")).unwrap(), "");
        // A hard-linked file.
        let tmp = TmpDir::new("runlog-hard-link");
        std::fs::create_dir_all(tmp.0.join("migration/build")).unwrap();
        std::fs::write(tmp.0.join("victim"), "").unwrap();
        std::fs::hard_link(
            tmp.0.join("victim"),
            tmp.0.join("migration/build").join(FILE),
        )
        .unwrap();
        let log = RunLog::open(&tmp.0, 1);
        assert!(log
            .not_written
            .as_deref()
            .unwrap()
            .contains("hard links are refused"));
        // A linked migration/.
        let tmp = TmpDir::new("runlog-link-migration");
        std::fs::create_dir_all(tmp.0.join("elsewhere")).unwrap();
        symlink(tmp.0.join("elsewhere"), tmp.0.join("migration")).unwrap();
        let log = RunLog::open(&tmp.0, 1);
        assert_eq!(
            log.not_written.as_deref(),
            Some("migration/ is not a directory (symlinks are refused)")
        );
    }

    #[test]
    fn a_failed_write_is_said_once_and_the_events_go_on() {
        let tmp = TmpDir::new("runlog-fail");
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        let mut log = RunLog::open(&tmp.0, 1);
        log.push(Kind::Chat, None, "before", None, Tone::Plain);
        log.fail_for_test("No space left on device (os error 28)");
        let said = log.poll().unwrap();
        assert_eq!(
            said,
            "the log could not be written: No space left on device (os error 28); the Log view \
             still shows this session"
        );
        assert!(log.poll().is_none());
        assert!(!log.writing());
        log.push(Kind::Chat, None, "after", None, Tone::Plain);
        assert_eq!(log.events.len(), 2);
        assert!(log.footer_words().contains("No space left on device"));
    }

    #[test]
    fn memory_keeps_the_last_events_and_counts_the_rest() {
        let tmp = TmpDir::new("runlog-cap");
        let mut log = RunLog::open(&tmp.0, 1);
        for i in 0..(MAX_EVENTS + 7) {
            log.push(Kind::Notice, None, format!("n{i}"), None, Tone::Notice);
        }
        assert_eq!(log.events.len(), MAX_EVENTS);
        assert_eq!(log.dropped, 7);
        assert_eq!(log.events[0].words, "n7");
    }

    #[test]
    fn a_target_without_a_ledger_waits_for_its_first_map() {
        let tmp = TmpDir::new("runlog-wait");
        let mut log = RunLog::open(&tmp.0, 9);
        assert!(!log.writing() && log.not_written.is_none());
        assert!(log.footer_words().starts_with("no ledger folder yet"));
        log.push(Kind::Notice, None, "held", None, Tone::Notice);
        assert!(
            !tmp.0.join("migration").exists(),
            "the cockpit never makes the ledger"
        );
        // The first map made the folder: the next push opens the file and
        // writes the header, then what waited.
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        log.push(Kind::Notice, None, "written", None, Tone::Notice);
        assert!(log.writing());
        settle(&mut log);
        let got = lines(&tmp.0);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0]["k"], "header");
        assert_eq!(got[1]["words"], "held");
        assert_eq!(got[2]["words"], "written");
    }

    #[test]
    fn close_then_reopen_writes_a_second_header_and_loses_nothing() {
        let tmp = TmpDir::new("runlog-reopen");
        std::fs::create_dir_all(tmp.0.join("migration")).unwrap();
        let mut log = RunLog::open(&tmp.0, 1);
        log.push(Kind::Chat, None, "one", None, Tone::Plain);
        log.close();
        // The adoption deletes build/ meanwhile.
        std::fs::remove_dir_all(tmp.0.join("migration/build")).unwrap();
        log.push(Kind::Chat, None, "between", None, Tone::Plain);
        assert!(
            !tmp.0.join("migration/build").exists(),
            "closed: nothing opens the file"
        );
        log.reopen();
        log.push(Kind::Chat, None, "two", None, Tone::Plain);
        settle(&mut log);
        let got = lines(&tmp.0);
        let words: Vec<&str> = got.iter().map(|l| l["k"].as_str().unwrap()).collect();
        assert_eq!(words, ["header", "chat", "chat"]);
        assert_eq!(got[1]["words"], "between");
        assert_eq!(got[2]["words"], "two");
        assert_eq!(log.events.len(), 3, "the view keeps every event");
    }

    #[test]
    fn the_clock_is_local_when_known_and_utc_otherwise() {
        let tmp = TmpDir::new("runlog-clock");
        let mut log = RunLog::open(&tmp.0, 1);
        let t = UNIX_EPOCH + Duration::from_secs(1_762_535_553);
        log.set_offset(None);
        assert_eq!(log.clock(t), "17:12:33");
        log.set_offset(Some(3600));
        assert_eq!(log.clock(t), "18:12:33");
        log.set_offset(Some(-8 * 3600));
        assert_eq!(log.clock(t), "09:12:33");
        assert_eq!(
            rfc3339_ms(t + Duration::from_millis(7)),
            "2025-11-07T17:12:33.007Z"
        );
    }
}
