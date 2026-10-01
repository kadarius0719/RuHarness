//! The person's workloads: `migration/perf/workloads.toml`
//! (`ruharness-perf-workloads` v1, docs/PERF-DESIGN.md §3.1), its strict
//! reader — every refusal names the line, the column and the workload — the
//! starter, the digest, and the one confined, bounded read of an input that
//! perf and the cockpit share.

use crate::error::Error;
use crate::hash::HASH_PREFIX;
use std::path::{Path, PathBuf};

/// Version of the workloads file this build reads.
pub const WORKLOADS_SCHEMA_VERSION: i64 = 1;
/// File name of the person's workloads, in `migration/perf/`.
pub const WORKLOADS_FILE: &str = "workloads.toml";
/// Largest workloads file read.
pub const MAX_WORKLOADS_BYTES: u64 = 64 * 1024;
/// Most workloads in one file.
pub const MAX_WORKLOADS: usize = 16;
/// Most arguments of one workload.
pub const MAX_ARGS: usize = 8;
/// Longest argument, in bytes.
pub const MAX_ARG_BYTES: usize = 256;
/// Longest workload id, in bytes.
pub const MAX_ID_BYTES: usize = 24;
/// Largest input file.
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
/// Fewest runs a side.
pub const MIN_RUNS: u32 = 5;
/// Most runs a side.
pub const MAX_RUNS: u32 = 31;
/// Runs a side when the workload does not say.
pub const DEFAULT_RUNS: u32 = 15;
/// The argument that stands for the input file.
pub const INPUT_ARG: &str = "{input}";

/// `migration/perf/workloads.toml` under `root`.
pub fn workloads_path(root: &Path) -> PathBuf {
    super::perf_dir(root).join(WORKLOADS_FILE)
}

/// One workload: a command line and at most one input file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workload {
    /// `^[a-z0-9][a-z0-9-]{0,23}$`, unique in the file.
    pub id: String,
    /// The program's arguments; `{input}` stands for the input, once, as a
    /// whole argument.
    pub args: Vec<String>,
    /// The input file, target-relative, as written.
    pub input: Option<String>,
    /// Runs a side, 5 to 31.
    pub runs: u32,
}

/// A validated workloads file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workloads {
    /// The workloads, in file order.
    pub workloads: Vec<Workload>,
}

impl Workloads {
    /// The workload with `id`.
    pub fn get(&self, id: &str) -> Option<&Workload> {
        self.workloads.iter().find(|w| w.id == id)
    }
}

/// A refusal of the workloads file: where, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadsError {
    /// 1-based line.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// The rule, naming the workload when there is one.
    pub message: String,
}

impl std::fmt::Display for WorkloadsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{WORKLOADS_FILE} line {}, column {}: {}",
            crate::ledger::MIGRATION_DIR,
            super::PERF_DIR,
            self.line,
            self.column,
            self.message
        )
    }
}

/// The workloads file as perf and the cockpit find it (§3.1 States).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkloadsState {
    /// No file.
    NoFile,
    /// A file with no workload — what the starter is.
    NoWorkload,
    /// A file that does not validate.
    Invalid(WorkloadsError),
    /// A file with at least one workload.
    Ready(Workloads),
}

impl WorkloadsState {
    /// Why perf cannot measure, in the CLI's and the cockpit's words, or
    /// `None` when it can.
    pub fn blocker(&self) -> Option<String> {
        match self {
            WorkloadsState::NoFile => {
                Some("write your workloads file first — harness perf init gives a starter".into())
            }
            WorkloadsState::NoWorkload => Some(format!(
                "add a [[workload]] to {}/{}/{WORKLOADS_FILE}",
                crate::ledger::MIGRATION_DIR,
                super::PERF_DIR
            )),
            WorkloadsState::Invalid(e) => Some(format!("{e} — fix it, or Edit the workloads file")),
            WorkloadsState::Ready(_) => None,
        }
    }
}

/// Load and validate `migration/perf/workloads.toml` under `root`. An I/O
/// failure, a link in place of the folder or the file, or a file over the
/// cap is an error; a newer `schema_version` an [`Error::SchemaTooNew`].
pub fn load(root: &Path) -> Result<WorkloadsState, Error> {
    let dir = super::perf_dir(root);
    match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(WorkloadsState::NoFile),
        Err(e) => return Err(Error::io(&dir, e)),
        Ok(m) if !m.file_type().is_dir() => {
            return Err(Error::InvalidPlan(format!(
                "{}: must be a directory (a link is refused)",
                dir.display()
            )))
        }
        Ok(_) => {}
    }
    let path = workloads_path(root);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(WorkloadsState::NoFile),
        Err(e) => return Err(Error::io(&path, e)),
        Ok(_) => {}
    }
    let bytes = crate::ledger::read_regular(&path, MAX_WORKLOADS_BYTES)?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(WorkloadsState::Invalid(WorkloadsError {
            line: 1,
            column: 1,
            message: "not UTF-8 text".into(),
        }));
    };
    match parse(text, &path) {
        Ok(w) if w.workloads.is_empty() => Ok(WorkloadsState::NoWorkload),
        Ok(w) => Ok(WorkloadsState::Ready(w)),
        Err(ParseError::Rule(e)) => Ok(WorkloadsState::Invalid(e)),
        Err(ParseError::TooNew(e)) => Err(e),
    }
}

/// Why a parse stopped.
#[derive(Debug)]
pub enum ParseError {
    /// A rule of §3.1, with its place.
    Rule(WorkloadsError),
    /// A newer `schema_version`.
    TooNew(Error),
}

type Spanned<'a> = toml::Spanned<toml::de::DeValue<'a>>;

fn at(text: &str, span: std::ops::Range<usize>, message: String) -> ParseError {
    let (line, column) = line_column(text, span.start);
    ParseError::Rule(WorkloadsError {
        line,
        column,
        message,
    })
}

/// 1-based line and column (in characters) of byte `offset` in `text`.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

fn integer(v: &toml::de::DeValue<'_>) -> Option<i64> {
    let i = v.as_integer()?;
    i64::from_str_radix(i.as_str(), i.radix()).ok()
}

/// Validate the text of a workloads file (see [`load`]); `path` names it in
/// a too-new error.
pub fn parse(text: &str, path: &Path) -> Result<Workloads, ParseError> {
    let doc = toml::de::DeTable::parse(text).map_err(|e| {
        let span = e.span().unwrap_or(0..0);
        at(
            text,
            span,
            format!("not valid TOML: {}", crate::features::one_line(e.message())),
        )
    })?;
    let table = doc.get_ref();
    let mut version: Option<&Spanned> = None;
    let mut list: Option<&Spanned> = None;
    let mut unknown: Option<(&str, std::ops::Range<usize>)> = None;
    for (key, value) in table.iter() {
        match key.get_ref().as_ref() {
            "schema_version" => version = Some(value),
            "workload" => list = Some(value),
            other => {
                if unknown.is_none() {
                    unknown = Some((other, key.span()));
                }
            }
        }
    }
    // A newer file first, whatever its keys: a key this harness does not
    // know is what a newer version adds, so it is told to upgrade, never to
    // fix the file.
    if let Some(n) = version
        .and_then(|v| integer(v.get_ref()))
        .filter(|n| *n > WORKLOADS_SCHEMA_VERSION)
    {
        return Err(ParseError::TooNew(Error::SchemaTooNew {
            path: path.to_path_buf(),
            found: n as u64,
            supported: WORKLOADS_SCHEMA_VERSION as u64,
        }));
    }
    if let Some((other, span)) = unknown {
        return Err(at(
            text,
            span,
            format!("unknown key {other:?} (the keys are schema_version and [[workload]])"),
        ));
    }
    match version {
        None => return Err(at(text, 0..0, "`schema_version = 1` is missing".into())),
        Some(v) => match integer(v.get_ref()) {
            Some(n) if n == WORKLOADS_SCHEMA_VERSION => {}
            _ => return Err(at(text, v.span(), "schema_version must be 1".into())),
        },
    }
    let items: Vec<&Spanned> = match list {
        None => Vec::new(),
        Some(v) => match v.get_ref().as_array() {
            Some(items) => items.iter().collect(),
            None => {
                return Err(at(
                    text,
                    v.span(),
                    "`workload` must be written as [[workload]] tables".into(),
                ))
            }
        },
    };
    if items.len() > MAX_WORKLOADS {
        return Err(at(
            text,
            items[MAX_WORKLOADS].span(),
            format!("{} workloads; at most {MAX_WORKLOADS}", items.len()),
        ));
    }
    let mut out: Vec<Workload> = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let Some(t) = item.get_ref().as_table() else {
            return Err(at(
                text,
                item.span(),
                format!("workload[{i}] must be a table ([[workload]])"),
            ));
        };
        let w = parse_workload(text, item.span(), t, i, &out)?;
        out.push(w);
    }
    Ok(Workloads { workloads: out })
}

fn parse_workload(
    text: &str,
    span: std::ops::Range<usize>,
    t: &toml::de::DeTable<'_>,
    i: usize,
    earlier: &[Workload],
) -> Result<Workload, ParseError> {
    let mut id: Option<&Spanned> = None;
    let mut args: Option<&Spanned> = None;
    let mut input: Option<&Spanned> = None;
    let mut runs: Option<&Spanned> = None;
    // The workload's name in every refusal once its id is known.
    let what = |id: Option<&str>| match id {
        Some(id) => format!("workload \"{id}\""),
        None => format!("workload[{i}]"),
    };
    let id_text = t
        .iter()
        .find(|(k, _)| k.get_ref().as_ref() == "id")
        .and_then(|(_, v)| v.get_ref().as_str())
        .filter(|s| is_id(s));
    for (key, value) in t.iter() {
        match key.get_ref().as_ref() {
            "id" => id = Some(value),
            "args" => args = Some(value),
            "input" => input = Some(value),
            "runs" => runs = Some(value),
            other => {
                return Err(at(
                    text,
                    key.span(),
                    format!(
                        "{}: unknown key {other:?} (a workload has id, args, input and runs)",
                        what(id_text)
                    ),
                ))
            }
        }
    }
    let Some(id_value) = id else {
        return Err(at(text, span, format!("{}: `id` is missing", what(None))));
    };
    let Some(id) = id_value.get_ref().as_str() else {
        return Err(at(
            text,
            id_value.span(),
            format!("{}: `id` must be a string", what(None)),
        ));
    };
    if !is_id(id) {
        return Err(at(
            text,
            id_value.span(),
            format!(
                "{}: id {id:?} is not allowed — 1 to 24 of a-z, 0-9 and -, starting with a \
                 letter or digit",
                what(None)
            ),
        ));
    }
    let what = what(Some(id));
    if earlier.iter().any(|w| w.id == id) {
        return Err(at(
            text,
            id_value.span(),
            format!("{what}: the id is used twice"),
        ));
    }
    let input = match input {
        None => None,
        Some(v) => {
            let Some(s) = v.get_ref().as_str() else {
                return Err(at(
                    text,
                    v.span(),
                    format!("{what}: `input` must be a string"),
                ));
            };
            if let Some(why) = input_problem(s) {
                return Err(at(
                    text,
                    v.span(),
                    format!("{what}: input {s:?} is not allowed — {why}"),
                ));
            }
            Some(s.to_string())
        }
    };
    let mut list: Vec<String> = Vec::new();
    if let Some(v) = args {
        let Some(items) = v.get_ref().as_array() else {
            return Err(at(
                text,
                v.span(),
                format!("{what}: `args` must be an array of strings"),
            ));
        };
        if items.len() > MAX_ARGS {
            return Err(at(
                text,
                v.span(),
                format!("{what}: {} arguments; at most {MAX_ARGS}", items.len()),
            ));
        }
        for (j, item) in items.iter().enumerate() {
            let Some(arg) = item.get_ref().as_str() else {
                return Err(at(
                    text,
                    item.span(),
                    format!("{what}: args[{j}] must be a string"),
                ));
            };
            if arg.len() > MAX_ARG_BYTES {
                return Err(at(
                    text,
                    item.span(),
                    format!(
                        "{what}: args[{j}] is {} bytes; at most {MAX_ARG_BYTES}",
                        arg.len()
                    ),
                ));
            }
            if arg.contains('\0') {
                return Err(at(
                    text,
                    item.span(),
                    format!("{what}: args[{j}] cannot hold a NUL"),
                ));
            }
            if arg != INPUT_ARG && arg.contains(INPUT_ARG) {
                return Err(at(
                    text,
                    item.span(),
                    format!(
                        "{what}: \"{INPUT_ARG}\" is a whole argument (\"{INPUT_ARG}\", never \
                         inside another, like \"--file={INPUT_ARG}\")"
                    ),
                ));
            }
            if arg == INPUT_ARG && input.is_none() {
                return Err(at(
                    text,
                    item.span(),
                    format!("{what}: \"{INPUT_ARG}\" needs an input (input = \"bench/big.txt\")"),
                ));
            }
            if arg == INPUT_ARG && list.iter().any(|a| a == INPUT_ARG) {
                return Err(at(
                    text,
                    item.span(),
                    format!("{what}: \"{INPUT_ARG}\" may appear only once"),
                ));
            }
            list.push(arg.to_string());
        }
    }
    if input.is_some() && !list.iter().any(|a| a == INPUT_ARG) {
        let place = args.map_or(span.clone(), |v| v.span());
        return Err(at(
            text,
            place,
            format!("{what}: it has an input, so one argument must be \"{INPUT_ARG}\""),
        ));
    }
    let runs = match runs {
        None => DEFAULT_RUNS,
        Some(v) => match integer(v.get_ref()) {
            Some(n) if (MIN_RUNS as i64..=MAX_RUNS as i64).contains(&n) => n as u32,
            _ => {
                return Err(at(
                    text,
                    v.span(),
                    format!("{what}: runs must be a whole number from {MIN_RUNS} to {MAX_RUNS}"),
                ))
            }
        },
    };
    Ok(Workload {
        id: id.to_string(),
        args: list,
        input,
        runs,
    })
}

/// Whether `s` is a workload id: `^[a-z0-9][a-z0-9-]{0,23}$`.
pub fn is_id(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && s.len() <= MAX_ID_BYTES
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

/// Why an input path as written is not allowed, or `None` when it is:
/// relative, no control character, every component non-empty and not
/// starting with `.` or `-`, not under `migration/`.
pub fn input_problem(input: &str) -> Option<&'static str> {
    if input.is_empty() {
        return Some("it is empty");
    }
    if input.starts_with('/') {
        return Some("it must be relative to the project");
    }
    if input.chars().any(char::is_control) {
        return Some("it cannot hold control characters");
    }
    let mut parts = input.split('/');
    if input.split('/').any(str::is_empty) {
        return Some("it has an empty part (\"//\" or a trailing \"/\")");
    }
    if input
        .split('/')
        .any(|p| p.starts_with('.') || p.starts_with('-'))
    {
        return Some("no part of it may start with \".\" or \"-\"");
    }
    if parts.next() == Some(crate::ledger::MIGRATION_DIR) {
        return Some("it cannot be under migration/");
    }
    None
}

/// Why perf cannot use a workload's input (§3.1, outcome `input-unusable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputUnusable {
    /// Not there.
    Missing,
    /// The file itself is a link.
    Link,
    /// A linked folder leads outside the project.
    Outside,
    /// A linked folder leads into `.git`.
    IntoGit,
    /// A folder, a pipe or a device.
    NotAFile,
    /// Over 64 MiB.
    TooLarge,
    /// Under the ledger folder (through a linked folder).
    UnderMigration,
    /// Its permissions refuse the read.
    PermissionDenied,
    /// Another read failure.
    Unreadable,
}

impl InputUnusable {
    /// Every reason, in a fixed order.
    pub const ALL: [InputUnusable; 9] = [
        InputUnusable::Missing,
        InputUnusable::Link,
        InputUnusable::Outside,
        InputUnusable::IntoGit,
        InputUnusable::NotAFile,
        InputUnusable::TooLarge,
        InputUnusable::UnderMigration,
        InputUnusable::PermissionDenied,
        InputUnusable::Unreadable,
    ];

    /// The closed value stored on a row.
    pub fn token(self) -> &'static str {
        match self {
            InputUnusable::Missing => "missing",
            InputUnusable::Link => "link",
            InputUnusable::Outside => "outside",
            InputUnusable::IntoGit => "into-git",
            InputUnusable::NotAFile => "not-a-file",
            InputUnusable::TooLarge => "too-large",
            InputUnusable::UnderMigration => "under-migration",
            InputUnusable::PermissionDenied => "permission-denied",
            InputUnusable::Unreadable => "unreadable",
        }
    }

    /// The reason a stored token names.
    pub fn from_token(token: &str) -> Option<InputUnusable> {
        InputUnusable::ALL.into_iter().find(|r| r.token() == token)
    }

    /// The words, the same in the CLI and the cockpit, for `input` as
    /// written.
    pub fn words(self, input: &str) -> String {
        let input = crate::text::safe_line(input);
        match self {
            InputUnusable::Missing => {
                format!("{input} is not here — put the file back or remove the workload")
            }
            InputUnusable::Link => format!("{input} is a link — copy the file in instead"),
            InputUnusable::Outside => format!(
                "{input} leads outside the project through a linked folder — copy the file in"
            ),
            InputUnusable::IntoGit => {
                format!("{input} leads into .git through a linked folder — copy the file in")
            }
            InputUnusable::NotAFile => {
                format!("{input} is a folder (or a pipe, or a device) — name a file")
            }
            InputUnusable::TooLarge => format!("{input} is over 64 MiB — use a smaller input"),
            InputUnusable::UnderMigration => format!("{input} is under migration/ — move it"),
            InputUnusable::PermissionDenied => {
                format!("{input} cannot be read (permission denied) — fix its permissions")
            }
            InputUnusable::Unreadable => {
                format!("{input} cannot be read — check the file and measure again")
            }
        }
    }
}

/// Read a workload's input once (§3.1): `rel` as written, under `root`; its
/// canonical path inside the canonical `root`, outside the canonical
/// ledger folder and any `.git`; a regular file of at most 64 MiB, read
/// through a handle checked to be the file looked at, never past the cap.
/// Every run's copy and the digest come from these bytes.
pub fn read_input(root: &Path, rel: &str) -> Result<Vec<u8>, InputUnusable> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let io = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => InputUnusable::Missing,
        std::io::ErrorKind::PermissionDenied => InputUnusable::PermissionDenied,
        _ => InputUnusable::Unreadable,
    };
    if input_problem(rel).is_some() {
        return Err(InputUnusable::Unreadable);
    }
    let written = root.join(rel);
    let seen = std::fs::symlink_metadata(&written).map_err(io)?;
    if seen.file_type().is_symlink() {
        return Err(InputUnusable::Link);
    }
    let canonical_root = root.canonicalize().map_err(io)?;
    let canonical = written.canonicalize().map_err(io)?;
    let Ok(inside) = canonical.strip_prefix(&canonical_root) else {
        return Err(InputUnusable::Outside);
    };
    if inside.components().any(|c| c.as_os_str() == ".git") {
        return Err(InputUnusable::IntoGit);
    }
    let ledger = canonical_root.join(crate::ledger::MIGRATION_DIR);
    if canonical.starts_with(&ledger) {
        return Err(InputUnusable::UnderMigration);
    }
    let meta = std::fs::symlink_metadata(&canonical).map_err(io)?;
    if !meta.file_type().is_file() {
        return Err(InputUnusable::NotAFile);
    }
    if meta.len() > MAX_INPUT_BYTES {
        return Err(InputUnusable::TooLarge);
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(&canonical)
        .map_err(io)?;
    let opened = file.metadata().map_err(io)?;
    if !opened.is_file() || opened.dev() != meta.dev() || opened.ino() != meta.ino() {
        return Err(InputUnusable::Unreadable);
    }
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(InputUnusable::TooLarge);
    }
    Ok(bytes)
}

/// The workload's digest (§3.1): blake3 over its id, its arguments, its
/// input's name and bytes — not `runs` (a row records the n it used).
/// `input` is the bytes [`read_input`] gave, when the workload has one.
pub fn digest(workload: &Workload, input: Option<&[u8]>) -> String {
    let mut h = blake3::Hasher::new();
    let mut field = |tag: &[u8], bytes: &[u8]| {
        h.update(tag);
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    field(b"id", workload.id.as_bytes());
    for arg in &workload.args {
        field(b"arg", arg.as_bytes());
    }
    if let Some(name) = &workload.input {
        field(b"input-name", name.as_bytes());
        field(b"input-bytes", input.unwrap_or_default());
    }
    format!("{HASH_PREFIX}{}", h.finalize().to_hex())
}

/// The starter `harness perf init` writes: no workload yet, and what to
/// write (§3.1).
pub const STARTER: &str = r#"schema_version = 1
# Workloads: runs of the whole program long enough to time — half a second or more of the C.
# Use the options and an input your program is really used with. Put the input file in the
# project (a real file, not a link; at most 64 MiB; not under migration/ or .git; no part of
# its path starting with "." or "-") and commit it, or a fresh clone cannot measure.
# perf compares what the program prints and how it ends, not files it writes.
# A program that starts other programs cannot be measured.
#
# [[workload]]
# id = "big-text"
# args = ["{input}"]          # your program's own options; "{input}" once, as its own argument
# input = "bench/big.txt"
# runs = 15                   # 5 to 31
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> WorkloadsError {
        match parse(text, Path::new("w.toml")) {
            Err(ParseError::Rule(e)) => e,
            other => panic!("expected a refusal of {text:?}, got {other:?}"),
        }
    }

    #[test]
    fn a_valid_file_reads_with_defaults() {
        let text = "schema_version = 1\n[[workload]]\nid = \"big-text\"\nargs = [\"-c\", \
                    \"{input}\"]\ninput = \"bench/big.txt\"\n\n[[workload]]\nid = \"none\"\n";
        let w = parse(text, Path::new("w.toml")).expect("valid");
        assert_eq!(w.workloads.len(), 2);
        assert_eq!(w.workloads[0].runs, DEFAULT_RUNS);
        assert_eq!(w.workloads[0].input.as_deref(), Some("bench/big.txt"));
        assert!(w.workloads[1].args.is_empty() && w.workloads[1].input.is_none());
        assert_eq!(w.get("none").map(|w| w.id.as_str()), Some("none"));
    }

    #[test]
    fn the_starter_has_no_workload() {
        let w = parse(STARTER, Path::new("w.toml")).expect("the starter parses");
        assert!(w.workloads.is_empty());
        // Uncommented, its example is a valid workload.
        let example: String = STARTER
            .lines()
            .map(|l| {
                l.strip_prefix("# ")
                    .filter(|l| {
                        !l.starts_with(char::is_uppercase)
                            && !l.is_empty()
                            && (l.starts_with('[') || l.contains(" = "))
                    })
                    .unwrap_or(l)
            })
            .map(|l| format!("{l}\n"))
            .collect();
        let w = parse(&example, Path::new("w.toml")).expect("the example parses");
        assert_eq!(w.workloads.len(), 1, "{example}");
        assert_eq!(w.workloads[0].runs, 15);
    }

    #[test]
    fn every_refusal_names_its_line_and_column_and_workload() {
        let e = rule("schema_version = 1\n[[workload]]\nid = \"a\"\n  colour = 3\n");
        assert_eq!((e.line, e.column), (4, 3), "{e}");
        assert!(
            e.message.contains("workload \"a\"") && e.message.contains("colour"),
            "{e}"
        );
        let e = rule("schema_version = 1\n[[workload]]\nid = \"a\"\nruns = 4\n");
        assert_eq!((e.line, e.column), (4, 8), "{e}");
        assert!(e.message.contains("5 to 31"), "{e}");
        let e = rule("schema_version = 1\n[[workload]]\nid = \"a\"\n[[workload]]\nid = \"a\"\n");
        assert_eq!(e.line, 5, "{e}");
        assert!(e.message.contains("used twice"), "{e}");
        let e = rule("schema_version = 1\nextra = 1\n");
        assert_eq!((e.line, e.column), (2, 1), "{e}");
        let e = rule("schema_version = 1\n[[workload]\n");
        assert!(e.message.starts_with("not valid TOML"), "{e}");
        assert_eq!(
            e.to_string().split(':').next(),
            Some(
                format!(
                    "migration/perf/workloads.toml line {}, column {}",
                    e.line, e.column
                )
                .as_str()
            )
        );
        // A newer schema is its own error, not a rule — also when it has a
        // key or a table this harness does not know, before or after the
        // version.
        for newer in [
            "schema_version = 2\n",
            "schema_version = 2\nprofile = 1\n",
            "profile = 1\nschema_version = 2\n",
            "schema_version = 2\n[settings]\nwarm = true\n",
            "schema_version = 2\n[[workload]]\nid = \"a\"\nwarmup = 3\n",
        ] {
            assert!(
                matches!(
                    parse(newer, Path::new("w.toml")),
                    Err(ParseError::TooNew(_))
                ),
                "{newer:?}"
            );
        }
        // A misspelt version is named where it is.
        let e = rule("schema_versoin = 1\n");
        assert_eq!((e.line, e.column), (1, 1), "{e}");
        assert!(e.message.contains("unknown key \"schema_versoin\""), "{e}");
        assert!(rule("[[workload]]\nid = \"a\"\n")
            .message
            .contains("schema_version"));
    }

    #[test]
    fn the_input_rules() {
        let with = |args: &str, input: &str| {
            format!("schema_version = 1\n[[workload]]\nid = \"w\"\nargs = {args}\n{input}")
        };
        // {input} once, whole, and only with an input.
        assert!(rule(&with("[\"--file={input}\"]", "input = \"a.txt\"\n"))
            .message
            .contains("whole argument"));
        assert!(rule(&with("[\"{input}\"]", ""))
            .message
            .contains("needs an input"));
        assert!(
            rule(&with("[\"{input}\", \"{input}\"]", "input = \"a.txt\"\n"))
                .message
                .contains("only once")
        );
        assert!(rule(&with("[\"-x\"]", "input = \"a.txt\"\n"))
            .message
            .contains("one argument must be"));
        // As written.
        for (input, why) in [
            ("/etc/passwd", "relative"),
            ("migration/x.txt", "migration/"),
            ("bench/.hidden", "start with"),
            ("-rf", "start with"),
            ("a/../b", "start with"),
            ("a//b", "empty part"),
            ("a\tb", "control"),
        ] {
            let e = rule(&with("[\"{input}\"]", &format!("input = {input:?}\n")));
            assert!(e.message.contains(why), "{input}: {e}");
        }
        // Limits.
        let nine = format!("[{}]", ["\"a\""; 9].join(", "));
        assert!(rule(&with(&nine, "")).message.contains("at most 8"));
        let long = format!("[\"{}\"]", "a".repeat(257));
        assert!(rule(&with(&long, "")).message.contains("at most 256"));
        let many: String = (0..17)
            .map(|i| format!("[[workload]]\nid = \"w{i}\"\n"))
            .collect();
        assert!(rule(&format!("schema_version = 1\n{many}"))
            .message
            .contains("at most 16"));
    }

    #[test]
    fn the_states_and_their_words() {
        let dir = std::env::temp_dir().join(format!("perf-w-{}", crate::hash::random_hex(6)));
        std::fs::create_dir_all(&dir).expect("dir");
        assert_eq!(load(&dir).expect("load"), WorkloadsState::NoFile);
        assert!(WorkloadsState::NoFile
            .blocker()
            .expect("words")
            .contains("harness perf init"));
        std::fs::create_dir_all(super::super::perf_dir(&dir)).expect("perf dir");
        std::fs::write(workloads_path(&dir), STARTER).expect("write");
        let state = load(&dir).expect("load");
        assert_eq!(state, WorkloadsState::NoWorkload);
        assert!(state
            .blocker()
            .expect("words")
            .contains("add a [[workload]]"));
        std::fs::write(
            workloads_path(&dir),
            "schema_version = 1\n[[workload]]\nid = \"A\"\n",
        )
        .expect("write");
        let state = load(&dir).expect("load");
        let words = state.blocker().expect("words");
        assert!(
            words.starts_with("migration/perf/workloads.toml line 3, column 6:")
                && words.ends_with("— fix it, or Edit the workloads file"),
            "{words}"
        );
        // A newer file with a key of its own: upgrade the harness, not "fix
        // it".
        std::fs::write(workloads_path(&dir), "schema_version = 2\nprofile = 1\n").expect("write");
        assert!(matches!(
            load(&dir),
            Err(Error::SchemaTooNew { found: 2, .. })
        ));
        std::fs::write(
            workloads_path(&dir),
            "schema_version = 1\n[[workload]]\nid = \"a\"\n",
        )
        .expect("write");
        assert!(matches!(
            load(&dir).expect("load"),
            WorkloadsState::Ready(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_input_is_read_once_confined_and_bounded() {
        let base = std::env::temp_dir().join(format!("perf-i-{}", crate::hash::random_hex(6)));
        let root = base.join("t");
        std::fs::create_dir_all(root.join("bench")).expect("dir");
        std::fs::create_dir_all(root.join(".git")).expect("git");
        std::fs::create_dir_all(root.join("migration")).expect("ledger");
        std::fs::create_dir_all(base.join("outside")).expect("outside");
        std::fs::write(root.join("bench/big.txt"), b"hello").expect("write");
        std::fs::write(base.join("outside/x.txt"), b"x").expect("write");
        std::fs::write(root.join(".git/config"), b"x").expect("write");
        std::fs::write(root.join("migration/y.txt"), b"y").expect("write");
        std::os::unix::fs::symlink(root.join("bench/big.txt"), root.join("bench/link.txt"))
            .expect("link");
        std::os::unix::fs::symlink(base.join("outside"), root.join("away")).expect("link");
        std::os::unix::fs::symlink(root.join(".git"), root.join("g")).expect("link");
        std::os::unix::fs::symlink(root.join("migration"), root.join("m")).expect("link");
        assert_eq!(read_input(&root, "bench/big.txt"), Ok(b"hello".to_vec()));
        assert_eq!(
            read_input(&root, "bench/gone.txt"),
            Err(InputUnusable::Missing)
        );
        assert_eq!(
            read_input(&root, "bench/link.txt"),
            Err(InputUnusable::Link)
        );
        assert_eq!(read_input(&root, "away/x.txt"), Err(InputUnusable::Outside));
        assert_eq!(read_input(&root, "g/config"), Err(InputUnusable::IntoGit));
        assert_eq!(
            read_input(&root, "m/y.txt"),
            Err(InputUnusable::UnderMigration)
        );
        assert_eq!(read_input(&root, "bench"), Err(InputUnusable::NotAFile));
        // Each reason has its own words and token.
        for r in InputUnusable::ALL {
            assert_eq!(InputUnusable::from_token(r.token()), Some(r));
            assert!(r.words("bench/big.txt").starts_with("bench/big.txt "));
        }
        assert_eq!(
            InputUnusable::Missing.words("bench/big.txt"),
            "bench/big.txt is not here — put the file back or remove the workload"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_digest_covers_id_args_and_input_but_not_runs() {
        let w = Workload {
            id: "a".into(),
            args: vec!["{input}".into()],
            input: Some("x.txt".into()),
            runs: 15,
        };
        let d = digest(&w, Some(b"one"));
        assert_eq!(
            digest(
                &Workload {
                    runs: 31,
                    ..w.clone()
                },
                Some(b"one")
            ),
            d
        );
        assert_ne!(digest(&w, Some(b"two")), d);
        assert_ne!(
            digest(
                &Workload {
                    id: "b".into(),
                    ..w.clone()
                },
                Some(b"one")
            ),
            d
        );
        assert_ne!(
            digest(
                &Workload {
                    input: Some("y.txt".into()),
                    ..w.clone()
                },
                Some(b"one")
            ),
            d
        );
        // Framed: moving bytes between fields changes it.
        let ab = Workload {
            id: "w".into(),
            args: vec!["ab".into()],
            input: None,
            runs: 15,
        };
        let a_b = Workload {
            args: vec!["a".into(), "b".into()],
            ..ab.clone()
        };
        assert_ne!(digest(&ab, None), digest(&a_b, None));
    }
}
