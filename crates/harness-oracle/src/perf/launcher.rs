//! The launcher (docs/PERF-DESIGN.md §3.2 step 1, §3.3; build notes 1–9):
//! `perfrun` and `perfgo`, built from their embedded sources into a private
//! cache outside the target, with a compiler found through root-owned paths
//! — locked, hash-checked before each run — and the harness's side of one
//! measured run: the control socket, the go-ahead and the bye.

use super::build::Hashed;
use crate::exec::{self, OTHER, PROGRAM_FIRST};
use crate::sandbox::HostDirs;
use harness_core::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// perfrun's source, embedded.
pub(crate) const PERFRUN_C: &str = include_str!("perfrun.c");
/// perfgo's source, embedded.
pub(crate) const PERFGO_C: &str = include_str!("perfgo.c");

/// The longest record perfrun writes, beyond the child line.
pub(crate) const RECORD_MAX: usize = 4096;
/// The longest `child <pid>` line.
const CHILD_LINE_MAX: usize = 64;
/// How long perfrun gets after a SIGTERM before its group is killed.
const TERM_GRACE: Duration = Duration::from_millis(250);
/// The harness's margin over perfrun's own deadline.
const HARNESS_MARGIN: Duration = Duration::from_secs(15);

// ---- The compiler ------------------------------------------------------

/// The compiler the launcher is built with, found through root-owned paths
/// only (§3.2 step 1, build note 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Compiler {
    pub clang: PathBuf,
    pub ld: PathBuf,
    pub sdk: PathBuf,
    /// `clang --version`'s first line.
    pub version: String,
    /// Said in the progress line when the selected developer folder failed
    /// the check and the Command Line Tools were used.
    pub note: Option<String>,
}

const XCODE_SELECT_LINK: &str = "/var/db/xcode_select_link";
const XCODE_DEVELOPER: &str = "/Applications/Xcode.app/Contents/Developer";
const COMMAND_LINE_TOOLS: &str = "/Library/Developer/CommandLineTools";

/// A developer folder's (clang, ld, libLTO, SDK): Xcode's layout or the
/// Command Line Tools'.
fn layout(dev: &Path) -> Option<[PathBuf; 4]> {
    let xcode = dev.join("Toolchains/XcodeDefault.xctoolchain/usr");
    if xcode.join("bin/clang").exists() {
        return Some([
            xcode.join("bin/clang"),
            xcode.join("bin/ld"),
            xcode.join("lib/libLTO.dylib"),
            dev.join("Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk"),
        ]);
    }
    let clt = dev.join("usr");
    if clt.join("bin/clang").exists() {
        return Some([
            clt.join("bin/clang"),
            clt.join("bin/ld"),
            clt.join("lib/libLTO.dylib"),
            dev.join("SDKs/MacOSX.sdk"),
        ]);
    }
    None
}

/// `path`'s canonical form when every component of it is owned by root with
/// the "other" write bit clear (a group write only for wheel or admin —
/// `/Applications` is root:admin, which no tool profile can write); else the
/// first component that is not.
pub(crate) fn root_owned(path: &Path) -> Result<PathBuf, PathBuf> {
    let canonical = path.canonicalize().map_err(|_| path.to_path_buf())?;
    for p in canonical.ancestors() {
        let Ok(m) = std::fs::symlink_metadata(p) else {
            return Err(p.to_path_buf());
        };
        let group_ok = m.mode() & 0o020 == 0 || matches!(m.gid(), 0 | 80);
        if m.uid() != 0 || m.mode() & 0o002 != 0 || !group_ok {
            return Err(p.to_path_buf());
        }
    }
    Ok(canonical)
}

/// The developer folders to try, the selected one first: the target of the
/// root-owned `/var/db/xcode_select_link`, else Xcode's, then the Command
/// Line Tools.
fn developer_folders() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let link_root_owned = std::fs::symlink_metadata(XCODE_SELECT_LINK)
        .is_ok_and(|m| m.uid() == 0 && m.file_type().is_symlink());
    match std::fs::read_link(XCODE_SELECT_LINK) {
        Ok(target) if link_root_owned => out.push(target),
        _ if Path::new(XCODE_DEVELOPER).is_dir() => out.push(PathBuf::from(XCODE_DEVELOPER)),
        _ => {}
    }
    if !out.iter().any(|p| p == Path::new(COMMAND_LINE_TOOLS)) {
        out.push(PathBuf::from(COMMAND_LINE_TOOLS));
    }
    out
}

/// Find the launcher's compiler; `Err` in the words of §3.2 step 1.
pub(crate) fn find_compiler() -> Result<Compiler, Error> {
    let mut first_failure: Option<PathBuf> = None;
    for (i, dev) in developer_folders().iter().enumerate() {
        let Some(paths) = layout(dev) else {
            continue;
        };
        let mut owned = Vec::with_capacity(4);
        let mut failed = None;
        for p in &paths {
            match root_owned(p) {
                Ok(c) => owned.push(c),
                Err(at) => {
                    failed = Some(at);
                    break;
                }
            }
        }
        if let Some(at) = failed {
            first_failure.get_or_insert(at);
            continue;
        }
        let clang = owned[0].clone();
        if clang == Path::new("/usr/bin/clang") || clang == Path::new("/usr/bin/cc") {
            first_failure.get_or_insert(clang);
            continue;
        }
        let version = clang_version(&clang)?;
        let note = (i > 0 && first_failure.is_some()).then(|| {
            format!(
                "the selected developer folder is not owned by the system — using the Command \
                 Line Tools at {}",
                dev.display()
            )
        });
        return Ok(Compiler {
            clang,
            ld: owned[1].clone(),
            sdk: owned[3].clone(),
            version,
            note,
        });
    }
    let at = first_failure.unwrap_or_else(|| PathBuf::from(COMMAND_LINE_TOOLS));
    Err(Error::Invariant(format!(
        "your compiler at {} is not owned by the system — install Xcode or the Command Line \
         Tools with Apple's installer",
        at.display()
    )))
}

fn clang_version(clang: &Path) -> Result<String, Error> {
    let out = Command::new(clang)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::io(clang, e))?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
}

// ---- The cache ---------------------------------------------------------

/// A built, checked launcher, held with a shared lock for the run.
#[derive(Debug)]
pub(crate) struct Launcher {
    pub perfrun: Hashed,
    pub perfgo: Hashed,
    /// The shared lock on the version folder's `.lock`, held while it lives.
    _shared: File,
}

impl Launcher {
    /// Refuse when either binary changed since it was built.
    pub(crate) fn check(&self) -> Result<(), Error> {
        self.perfrun.check()?;
        self.perfgo.check()
    }
}

/// Why the cache folder cannot be used: a build tool may write it.
fn refuse_cache(root: &Path, host: &HostDirs) -> Option<String> {
    let temp_roots = [Path::new("/private/tmp"), Path::new("/private/var/folders")];
    if temp_roots.iter().any(|t| root.starts_with(t)) {
        return Some(
            "your home folder is inside a temporary folder, which every build tool may write — \
             perf cannot keep its launcher there"
                .into(),
        );
    }
    if host.tmpdir.as_deref().is_some_and(|t| root.starts_with(t)) {
        return Some(
            "your TMPDIR holds perf's launcher folder, which every build tool may write — set \
             TMPDIR elsewhere"
                .into(),
        );
    }
    if host
        .cargo_home
        .as_deref()
        .is_some_and(|c| root.starts_with(c))
    {
        return Some(
            "your CARGO_HOME holds perf's launcher folder, which every crate build may write — \
             set CARGO_HOME elsewhere"
                .into(),
        );
    }
    None
}

/// A private folder (0700), made when missing; a link or a folder owned by
/// someone else is refused.
fn private_dir(path: &Path, owner: u32) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .or_else(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| Error::io(path, e))?;
            private_dir_check(path, owner)
        }
        Err(e) => Err(Error::io(path, e)),
        Ok(_) => private_dir_check(path, owner),
    }
}

fn private_dir_check(path: &Path, owner: u32) -> Result<(), Error> {
    let m = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if m.file_type().is_symlink() || !m.is_dir() || m.uid() != owner || m.mode() & 0o077 != 0 {
        return Err(Error::Invariant(format!(
            "{}: perf's launcher folder must be a private folder of yours (no link)",
            path.display()
        )));
    }
    Ok(())
}

/// Lock `path` (made when missing), shared or exclusive; after the lock the
/// locked file must still be the one at `path`, a regular file (build note
/// 4) — a link or a swapped file is retried, never held.
fn lock_file(path: &Path, exclusive: bool) -> Result<File, Error> {
    for _ in 0..100 {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        if exclusive {
            file.lock().map_err(|e| Error::io(path, e))?;
        } else {
            file.lock_shared().map_err(|e| Error::io(path, e))?;
        }
        let held = file.metadata().map_err(|e| Error::io(path, e))?;
        match std::fs::symlink_metadata(path) {
            Ok(now) if now.is_file() && now.dev() == held.dev() && now.ino() == held.ino() => {
                return Ok(file)
            }
            Ok(now) if now.file_type().is_symlink() => {
                return Err(Error::Invariant(format!(
                    "{}: perf's lock is a link — remove it",
                    path.display()
                )))
            }
            _ => continue,
        }
    }
    Err(Error::Invariant(format!(
        "{}: could not hold perf's lock",
        path.display()
    )))
}

/// The version folder's name: [`harness_core::perf::PERF_LAUNCHER`] and
/// the blake3 of the sources and the compiler.
fn version_name(compiler: &Compiler) -> String {
    let mut framed = Vec::new();
    for part in [
        harness_core::perf::PERF_LAUNCHER,
        PERFRUN_C,
        PERFGO_C,
        compiler.clang.to_string_lossy().as_ref(),
        compiler.ld.to_string_lossy().as_ref(),
        compiler.sdk.to_string_lossy().as_ref(),
        compiler.version.as_str(),
    ] {
        framed.extend_from_slice(&(part.len() as u64).to_le_bytes());
        framed.extend_from_slice(part.as_bytes());
    }
    let digest = harness_core::hash::bytes_hash(&framed);
    let hex = digest.trim_start_matches(harness_core::hash::HASH_PREFIX);
    format!("{}-{}", harness_core::perf::PERF_LAUNCHER, &hex[..16])
}

const HASHES_FILE: &str = "hashes";

/// The launcher, built when its cache is stale (`progress` says "building
/// the launcher…" then), checked, and held with a shared lock.
pub(crate) fn launcher(host: &HostDirs, progress: &mut dyn FnMut(&str)) -> Result<Launcher, Error> {
    if !cfg!(target_os = "macos") {
        return Err(Error::Invariant(
            "perf runs on macOS only for now — the Linux launcher is not built yet".into(),
        ));
    }
    let owner = std::fs::metadata(&host.home)
        .map_err(|e| Error::io(&host.home, e))?
        .uid();
    let root = host.perf_cache.clone();
    if let Some(words) = refuse_cache(&root, host) {
        return Err(Error::Invariant(words));
    }
    let caches = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Error::Invariant("perf's launcher folder has no parent".into()))?;
    std::fs::create_dir_all(caches).map_err(|e| Error::io(caches, e))?;
    private_dir(root.parent().unwrap_or(&root), owner)?;
    private_dir(&root, owner)?;
    let canonical = root.canonicalize().map_err(|e| Error::io(&root, e))?;
    if let Some(words) = refuse_cache(&canonical, host) {
        return Err(Error::Invariant(words));
    }
    launcher_in(&canonical, owner, progress)
}

/// [`launcher`] in the resolved, private cache root `canonical` (the
/// refusal of §3.2 already passed).
pub(crate) fn launcher_in(
    canonical: &Path,
    owner: u32,
    progress: &mut dyn FnMut(&str),
) -> Result<Launcher, Error> {
    let compiler = find_compiler()?;
    if let Some(note) = &compiler.note {
        progress(note);
    }
    let exclusive = lock_file(&canonical.join(".lock"), true)?;
    let dir = canonical.join(version_name(&compiler));
    let current = load_built(&dir);
    let (perfrun, perfgo) = match current {
        Some(built) => built,
        None => {
            progress("building the launcher…");
            if dir.exists() {
                // Only when no run holds it.
                if let Ok(lock) = File::open(dir.join(".lock")) {
                    if lock.try_lock().is_err() {
                        return Err(Error::Invariant(
                            "perf's launcher is in use by another perf run and needs \
                             rebuilding — wait for that run to finish"
                                .into(),
                        ));
                    }
                }
                std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
            }
            build(&dir, &compiler, owner)?
        }
    };
    let shared = lock_file(&dir.join(".lock"), false)?;
    remove_stale(canonical, &dir);
    drop(exclusive);
    let launcher = Launcher {
        perfrun,
        perfgo,
        _shared: shared,
    };
    launcher.check()?;
    Ok(launcher)
}

/// The built binaries when the folder holds both and their hashes match.
fn load_built(dir: &Path) -> Option<(Hashed, Hashed)> {
    let text = std::fs::read_to_string(dir.join(HASHES_FILE)).ok()?;
    let mut perfrun = None;
    let mut perfgo = None;
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("perfrun", h)) => perfrun = Some(h.to_string()),
            Some(("perfgo", h)) => perfgo = Some(h.to_string()),
            _ => return None,
        }
    }
    let pr = Hashed {
        path: dir.join("perfrun"),
        digest: perfrun?,
    };
    let pg = Hashed {
        path: dir.join("perfgo"),
        digest: perfgo?,
    };
    (pr.check().is_ok() && pg.check().is_ok()).then_some((pr, pg))
}

/// Build both binaries into `dir` (0700) with a cleared environment and
/// `TMPDIR` a fresh 0700 folder inside it.
fn build(dir: &Path, compiler: &Compiler, owner: u32) -> Result<(Hashed, Hashed), Error> {
    private_dir(dir, owner)?;
    let tmp = dir.join("tmp");
    private_dir(&tmp, owner)?;
    let mut out = Vec::new();
    for (name, source) in [("perfrun", PERFRUN_C), ("perfgo", PERFGO_C)] {
        let src = tmp.join(format!("{name}.c"));
        std::fs::write(&src, source).map_err(|e| Error::io(&src, e))?;
        let bin = dir.join(name);
        let mut cmd = Command::new(&compiler.clang);
        cmd.arg("-isysroot")
            .arg(&compiler.sdk)
            .arg(format!("-fuse-ld={}", compiler.ld.display()))
            .args(["-O2", "-Wall", "-o"])
            .arg(&bin)
            .arg(&src)
            .env_clear()
            .env("TMPDIR", &tmp)
            .current_dir(&tmp)
            .stdin(Stdio::null());
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let shown = format!("{} {name}.c", compiler.clang.display());
        let result = exec::run_with_timeout(
            cmd,
            &shown,
            Duration::from_secs(120),
            1024 * 1024,
            true,
            exec::Wait::Tool,
        )?;
        let ok = matches!(&result.end, exec::ChildEnd::Exited(s) if s.success());
        if !ok {
            let first: Vec<&str> = std::str::from_utf8(&result.stderr)
                .unwrap_or("")
                .lines()
                .take(5)
                .collect();
            return Err(Error::Invariant(format!(
                "perf's launcher does not build: {}",
                first.join(" / ")
            )));
        }
        out.push(Hashed::new(&bin)?);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    let hashes = format!("perfrun {}\nperfgo {}\n", out[0].digest, out[1].digest);
    harness_core::ledger::write_atomic(&dir.join(HASHES_FILE), hashes.as_bytes())?;
    Ok((out[0].clone(), out[1].clone()))
}

/// Remove stale version folders under the cache root, keeping the current
/// one and the newest other (two worktrees in turn do not rebuild each
/// other); only a folder whose own `.lock` takes an exclusive lock (no run
/// holds it). Best effort.
fn remove_stale(root: &Path, current: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut others: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p != current && p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("perf-launcher-"))
        })
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    others.sort();
    others.reverse();
    for (_, stale) in others.into_iter().skip(1) {
        let Ok(lock) = File::open(stale.join(".lock")) else {
            let _ = std::fs::remove_dir_all(&stale);
            continue;
        };
        if lock.try_lock().is_ok() {
            let _ = std::fs::remove_dir_all(&stale);
        }
    }
}

// ---- The record --------------------------------------------------------

/// The computer as perfrun reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Facts {
    pub cpu: String,
    pub os: String,
    pub build: String,
    pub arch: String,
    pub fast_cores: u32,
    pub two_kinds: bool,
}

/// How a run ended, as perfrun saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Ok,
    Timeout,
    Stopped,
    /// The exec failed (`Some(errno)`), or perfgo never said it was ready.
    NeverStarted(Option<i32>),
    /// perfrun's own failure, in its words.
    Launcher(String),
}

/// How the program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Exit(i32),
    Signal(i32),
}

impl End {
    /// `exit N` or `signal N`.
    pub(crate) fn token(self) -> String {
        match self {
            End::Exit(n) => format!("exit {n}"),
            End::Signal(n) => format!("signal {n}"),
        }
    }
}

/// One run's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub status: Status,
    pub end: Option<End>,
    /// perfrun sent the kill.
    pub killed: bool,
    pub instructions: Option<u64>,
    pub cycles: Option<u64>,
    pub p_instructions: Option<u64>,
    pub p_cycles: Option<u64>,
    pub cpu_us: Option<u64>,
    pub memory: Option<u64>,
    pub wall_us: Option<u64>,
    pub load: Option<u32>,
    pub facts: Facts,
}

const RECORD_KEYS: &[&str] = &[
    "status",
    "ended",
    "killed",
    "instructions",
    "cycles",
    "p_instructions",
    "p_cycles",
    "cpu_us",
    "memory",
    "wall_us",
    "load",
    "cpu",
    "os",
    "build",
    "arch",
    "fast_cores",
    "two_kinds",
];

const FACT_KEYS: &[&str] = &["cpu", "os", "build", "arch", "fast_cores", "two_kinds"];

/// Read a record strictly (§3.3 step 7): `key value` lines, each known key
/// once, every number bounded, `end` last and nothing after it. `facts_only`
/// reads `perfrun facts`' output (no status).
pub(crate) fn parse_record(text: &str, facts_only: bool) -> Result<Record, String> {
    if text.len() > RECORD_MAX + 1 {
        return Err("the record is over 4 KiB".into());
    }
    let body = text.strip_suffix("end\n").ok_or("the record has no end")?;
    let mut seen: Vec<&str> = Vec::new();
    let mut r = Record {
        status: Status::Ok,
        end: None,
        killed: false,
        instructions: None,
        cycles: None,
        p_instructions: None,
        p_cycles: None,
        cpu_us: None,
        memory: None,
        wall_us: None,
        load: None,
        facts: Facts::default(),
    };
    let allowed = if facts_only { FACT_KEYS } else { RECORD_KEYS };
    for line in body.lines() {
        let (key, value) = line
            .split_once(' ')
            .ok_or_else(|| format!("a bad line {line:?}"))?;
        if !allowed.contains(&key) {
            return Err(format!("an unknown key {key:?}"));
        }
        if seen.contains(&key) {
            return Err(format!("{key} twice"));
        }
        seen.push(key);
        let number = |max: u64| -> Result<u64, String> {
            value
                .parse::<u64>()
                .ok()
                .filter(|v| *v <= max && v.to_string() == value)
                .ok_or_else(|| format!("{key} {value:?} is not a number"))
        };
        let text = || -> Result<String, String> {
            if value.is_empty()
                || value.len() > 120
                || !value.bytes().all(|b| (0x20..0x7f).contains(&b))
            {
                return Err(format!("{key} is not plain text"));
            }
            Ok(value.to_string())
        };
        match key {
            "status" => {
                r.status = match value {
                    "ok" => Status::Ok,
                    "timeout" => Status::Timeout,
                    "stopped" => Status::Stopped,
                    "never-started no-ready" => Status::NeverStarted(None),
                    v if v.starts_with("never-started ") => {
                        let errno = v["never-started ".len()..]
                            .parse::<i32>()
                            .ok()
                            .filter(|e| (1..=255).contains(e))
                            .ok_or("a bad errno")?;
                        Status::NeverStarted(Some(errno))
                    }
                    v if v.starts_with("launcher ") => {
                        Status::Launcher(v["launcher ".len()..].to_string())
                    }
                    _ => return Err(format!("a status {value:?}")),
                }
            }
            "ended" => {
                r.end = Some(match value.split_once(' ') {
                    Some(("exit", n)) => {
                        End::Exit(n.parse::<u8>().map_err(|_| "a bad exit")? as i32)
                    }
                    Some(("signal", n)) => {
                        let s = n.parse::<u8>().map_err(|_| "a bad signal")?;
                        if !(1..=64).contains(&s) {
                            return Err("a bad signal".into());
                        }
                        End::Signal(s as i32)
                    }
                    _ => return Err(format!("ended {value:?}")),
                })
            }
            "killed" => r.killed = number(1)? == 1,
            "instructions" => r.instructions = Some(number(u64::MAX)?),
            "cycles" => r.cycles = Some(number(u64::MAX)?),
            "p_instructions" => r.p_instructions = Some(number(u64::MAX)?),
            "p_cycles" => r.p_cycles = Some(number(u64::MAX)?),
            "cpu_us" => r.cpu_us = Some(number(u64::MAX)?),
            "memory" => r.memory = Some(number(u64::MAX)?),
            "wall_us" => r.wall_us = Some(number(u64::MAX)?),
            "load" => r.load = Some(number(1_000_000)? as u32),
            "cpu" => r.facts.cpu = text()?,
            "os" => r.facts.os = text()?,
            "build" => r.facts.build = text()?,
            "arch" => r.facts.arch = text()?,
            "fast_cores" => r.facts.fast_cores = number(4096)? as u32,
            "two_kinds" => r.facts.two_kinds = number(1)? == 1,
            _ => unreachable!("the key is one of the allowed"),
        }
    }
    if !facts_only && !seen.contains(&"status") {
        return Err("the record has no status".into());
    }
    if r.p_instructions.is_some() != r.p_cycles.is_some() {
        return Err("the performance-core counts are both present or both absent".into());
    }
    Ok(r)
}

/// `perfrun facts`, read by its own rule (no child line, no bye).
pub(crate) fn computer_facts(launcher: &Launcher) -> Result<Facts, Error> {
    launcher.check()?;
    let out = Command::new(&launcher.perfrun.path)
        .arg("facts")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::io(&launcher.perfrun.path, e))?;
    let text = String::from_utf8(out.stdout)
        .map_err(|_| Error::Invariant("perfrun facts: not text".into()))?;
    parse_record(&text, true)
        .map(|r| r.facts)
        .map_err(|why| Error::Invariant(format!("perfrun facts: {why}")))
}

// ---- One measured run --------------------------------------------------

/// One run's spec.
pub(crate) struct RunSpec<'a> {
    pub launcher: &'a Launcher,
    /// The perf profile (`sandbox::render_perf_profile`).
    pub profile: &'a str,
    /// perfrun's own deadline, in seconds.
    pub deadline_secs: u64,
    /// The bound before the go-ahead, and the extra after it (§3.3).
    pub allowance: Duration,
    pub program: &'a Path,
    /// argv[0].
    pub name: &'a str,
    pub args: &'a [String],
    /// The run's working folder.
    pub cwd: &'a Path,
    /// The run's TMPDIR.
    pub tmpdir: &'a Path,
    /// Capture stdout and stderr (step 1), else `/dev/null` (timed runs).
    pub capture: bool,
    pub max_output: usize,
}

/// How the harness saw a run end.
#[derive(Debug)]
pub(crate) enum Seen {
    /// perfrun's record, complete.
    Record(Box<Record>),
    /// perfrun ended without a complete record (or never said its child).
    NoRecord(String),
    /// The harness's own timer fired.
    TimedOut,
    /// A stream passed the capture cap.
    Overflow,
}

/// A measured run: what the harness saw, perfrun's exit, the streams.
#[derive(Debug)]
pub(crate) struct Measured {
    pub seen: Seen,
    /// perfrun's own exit code (`None`: killed or not reaped).
    pub launcher_exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Read from the socket until `stop` says the bytes are complete, the
/// `limit` passes, `deadline` passes, or `interrupt` asks; polling in short
/// steps so an overflow or a cancellation is seen.
enum ReadEnd {
    Done,
    Eof,
    TooLong,
    Deadline,
    Interrupt,
}

fn read_until(
    sock: &mut UnixStream,
    buf: &mut Vec<u8>,
    limit: usize,
    deadline: Instant,
    stop: impl Fn(&[u8]) -> bool,
    interrupt: &dyn Fn() -> bool,
) -> ReadEnd {
    let mut byte = [0u8; 1];
    loop {
        if stop(buf) {
            return ReadEnd::Done;
        }
        if buf.len() > limit {
            return ReadEnd::TooLong;
        }
        if interrupt() {
            return ReadEnd::Interrupt;
        }
        let now = Instant::now();
        if now >= deadline {
            return ReadEnd::Deadline;
        }
        let step = (deadline - now).min(Duration::from_millis(50));
        let _ = sock.set_read_timeout(Some(step.max(Duration::from_millis(1))));
        // One byte at a time: a second record's bytes never join this one.
        match sock.read(&mut byte) {
            Ok(0) => return ReadEnd::Eof,
            Ok(n) => buf.extend_from_slice(&byte[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return ReadEnd::Eof,
        }
    }
}

/// Run `spec.program` once through perfrun (§3.3 *The harness side*):
/// 1. spawn perfrun under the registry's lock and register its group;
/// 2. outside the lock, read its `child <pid>` line;
/// 3. under the lock again: cancelled → close the socket without the
///    go-ahead (the program never runs); else register the program's group
///    (killed first) and write the go-ahead;
/// 4. read the record to `end`, unregister the program's group, then write
///    the bye — the program is reaped only after it;
/// 5. perfrun ends without a complete record → kill the program's group at
///    once;
/// 6. on the harness's own timeout, an overflow or a cancellation: SIGTERM
///    to perfrun, a 250 ms grace, then SIGKILL to the program's group and
///    perfrun's.
pub(crate) fn run_measured(spec: &RunSpec<'_>) -> Result<Measured, Error> {
    spec.launcher.check()?;
    let (mut ours, theirs) =
        UnixStream::pair().map_err(|e| Error::Invariant(format!("perf: a socket pair: {e}")))?;
    let mut cmd = Command::new(&spec.launcher.perfrun.path);
    cmd.arg("run")
        .arg(spec.profile)
        .arg(&spec.launcher.perfgo.path)
        .arg(spec.deadline_secs.max(1).to_string())
        .arg(spec.program)
        .arg(spec.name)
        .args(spec.args)
        .current_dir(spec.cwd)
        .env_clear()
        .env("TMPDIR", spec.tmpdir);
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd.stdin(Stdio::from(std::os::fd::OwnedFd::from(theirs)));
    if spec.capture {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    // 1. Spawn under the lock.
    let (mut child, launcher_reg) = {
        let mut live = exec::live_lock()?;
        let child = cmd
            .spawn()
            .map_err(|e| Error::Invariant(format!("perf: starting perfrun: {e}")))?;
        let reg = live.register(child.id(), OTHER);
        (child, reg)
    };
    drop(cmd);
    let perfrun_pid = child.id();
    let overflow = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let out_buf = Arc::new(Mutex::new(Vec::new()));
    let err_buf = Arc::new(Mutex::new(Vec::new()));
    let mut readers = 0;
    if let Some(p) = child.stdout.take() {
        exec::drain(p, &out_buf, spec.max_output, &overflow, done_tx.clone());
        readers += 1;
    }
    if let Some(p) = child.stderr.take() {
        exec::drain(p, &err_buf, spec.max_output, &overflow, done_tx.clone());
        readers += 1;
    }
    drop(done_tx);
    let finish = |child: &mut std::process::Child, seen: Seen| -> Measured {
        let status = child.wait().ok();
        let drain_deadline = Instant::now() + Duration::from_secs(2);
        for _ in 0..readers {
            let left = drain_deadline.saturating_duration_since(Instant::now());
            if done_rx.recv_timeout(left).is_err() {
                break;
            }
        }
        Measured {
            seen,
            launcher_exit: status.and_then(|s| s.code()),
            stdout: exec::take(&out_buf),
            stderr: exec::take(&err_buf),
        }
    };
    let interrupt = || exec::cancelled();
    // 2. The child line, outside the lock.
    let mut line = Vec::new();
    let line_deadline = Instant::now() + spec.allowance;
    let got = read_until(
        &mut ours,
        &mut line,
        CHILD_LINE_MAX,
        line_deadline,
        |b| b.ends_with(b"\n"),
        &interrupt,
    );
    let program_pid: Option<u32> = match got {
        ReadEnd::Done => std::str::from_utf8(&line)
            .ok()
            .and_then(|l| l.strip_prefix("child "))
            .and_then(|l| l.trim_end().parse::<u32>().ok())
            .filter(|p| *p > 1),
        _ => None,
    };
    let Some(program_pid) = program_pid else {
        // No child line: perfrun failed before its fork (its record, if any,
        // follows) or is stalled.
        let mut rest = line;
        let _ = read_until(
            &mut ours,
            &mut rest,
            RECORD_MAX + 1,
            Instant::now() + TERM_GRACE,
            |b| b.ends_with(b"end\n"),
            &interrupt,
        );
        exec::terminate(perfrun_pid);
        std::thread::sleep(Duration::from_millis(20));
        exec::kill_process_group(perfrun_pid);
        drop(ours);
        if exec::cancelled() {
            let _ = child.wait();
            return Err(Error::Interrupted);
        }
        let words = String::from_utf8_lossy(&rest)
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        let m = finish(
            &mut child,
            Seen::NoRecord(format!("no child line: {words}")),
        );
        drop(launcher_reg);
        return Ok(m);
    };
    // 3. The go-ahead, under the lock — or a cancel that never starts it.
    if exec::cancelled() {
        drop(ours);
        let _ = child.wait();
        return Err(Error::Interrupted);
    }
    let program_reg = {
        let mut live = exec::live_lock()?;
        let reg = live.register(program_pid, PROGRAM_FIRST);
        let _ = ours.write_all(b"G");
        reg
    };
    // 4. The record.
    let harness_deadline =
        Instant::now() + Duration::from_secs(spec.deadline_secs) + HARNESS_MARGIN + spec.allowance;
    let mut record = Vec::new();
    let overflowed = || overflow.load(Ordering::SeqCst) || exec::cancelled();
    let got = read_until(
        &mut ours,
        &mut record,
        RECORD_MAX + 1,
        harness_deadline,
        |b| b == b"end\n" || b.ends_with(b"\nend\n"),
        &overflowed,
    );
    match got {
        ReadEnd::Done => {
            let text = String::from_utf8_lossy(&record).into_owned();
            // Unregister the program's group, then the bye: perfrun reaps it
            // only after that, so no later kill can reach a reused pid.
            drop(program_reg);
            let _ = ours.write_all(b"B");
            let seen = match parse_record(&text, false) {
                Ok(r) => Seen::Record(Box::new(r)),
                Err(why) => Seen::NoRecord(why),
            };
            // perfrun exits at once after the bye; bound the wait.
            let wait_until = Instant::now() + Duration::from_secs(5);
            while Instant::now() < wait_until {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if child.try_wait().ok().flatten().is_none() {
                exec::kill_process_group(perfrun_pid);
            }
            drop(ours);
            let m = finish(&mut child, seen);
            drop(launcher_reg);
            if exec::cancelled() {
                return Err(Error::Interrupted);
            }
            Ok(m)
        }
        ReadEnd::Eof | ReadEnd::TooLong => {
            // 5. perfrun ended (or broke the record): its program's pid is
            // still reserved — kill its group now.
            exec::kill_process_group(program_pid);
            drop(program_reg);
            exec::kill_process_group(perfrun_pid);
            drop(ours);
            let why = if matches!(got, ReadEnd::TooLong) {
                "the record is over 4 KiB"
            } else {
                "perfrun ended before its record was complete"
            };
            let m = finish(&mut child, Seen::NoRecord(why.into()));
            drop(launcher_reg);
            if exec::cancelled() {
                return Err(Error::Interrupted);
            }
            Ok(m)
        }
        ReadEnd::Deadline | ReadEnd::Interrupt => {
            // 6. SIGTERM, a short grace, then SIGKILL — the program's group
            // first, then perfrun's.
            exec::terminate(perfrun_pid);
            let mut rest = record;
            let _ = read_until(
                &mut ours,
                &mut rest,
                RECORD_MAX + 1,
                Instant::now() + TERM_GRACE,
                |b| b.ends_with(b"end\n"),
                &|| false,
            );
            exec::kill_process_group(program_pid);
            drop(program_reg);
            exec::kill_process_group(perfrun_pid);
            drop(ours);
            let seen = if overflow.load(Ordering::SeqCst) {
                Seen::Overflow
            } else {
                Seen::TimedOut
            };
            let m = finish(&mut child, seen);
            drop(launcher_reg);
            if exec::cancelled() {
                return Err(Error::Interrupted);
            }
            Ok(m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sources_are_pinned_to_the_launcher_version() {
        // A change to either source must change PERF_LAUNCHER (a new cache
        // folder); this pins the sources' hash — update both together.
        let digest = harness_core::hash::bytes_hash(format!("{PERFRUN_C}\0{PERFGO_C}").as_bytes());
        let h = digest.trim_start_matches(harness_core::hash::HASH_PREFIX);
        assert_eq!(
            (harness_core::perf::PERF_LAUNCHER, &h[..16]),
            ("perf-launcher-1", PINNED_SOURCES),
            "perfrun.c or perfgo.c changed: bump PERF_LAUNCHER and PINNED_SOURCES"
        );
    }

    /// The sources' hash at [`harness_core::perf::PERF_LAUNCHER`].
    const PINNED_SOURCES: &str = "ad34c300de52107d";

    #[test]
    fn a_record_is_read_strictly() {
        let good = "status ok\nended exit 3\nkilled 0\ninstructions 4510482775\ncycles 905494051\n\
                    p_instructions 4507580577\np_cycles 903879602\ncpu_us 243807\nmemory 917816\n\
                    wall_us 464573\nload 292\ncpu Apple M3\nos 26.5.2\nbuild 25F84\narch arm64\n\
                    fast_cores 4\ntwo_kinds 1\nend\n";
        let r = parse_record(good, false).expect("reads");
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.end, Some(End::Exit(3)));
        assert_eq!(r.instructions, Some(4_510_482_775));
        assert_eq!(r.facts.cpu, "Apple M3");
        assert!(r.facts.two_kinds);
        for bad in [
            good.replace("end\n", ""),
            good.replace("killed 0\n", "killed 0\nkilled 0\n"),
            good.replace("load 292\n", "load 292\nweather sunny\n"),
            good.replace("cycles 905494051\n", "cycles -1\n"),
            good.replace("ended exit 3\n", "ended exit 300\n"),
            good.replace("p_cycles 903879602\n", ""),
            format!("{good}status ok\nend\n"),
            good.replace("status ok\n", ""),
            good.replace("cpu Apple M3\n", "cpu Apple\u{1b}M3\n"),
            format!("{}{good}", "x ".repeat(3000)),
        ] {
            assert!(parse_record(&bad, false).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse_record(
                "status never-started no-ready\nended exit 127\nkilled 0\nend\n",
                false
            )
            .expect("reads")
            .status,
            Status::NeverStarted(None)
        );
        assert_eq!(
            parse_record(
                "status never-started 2\nended exit 127\nkilled 0\nend\n",
                false
            )
            .expect("reads")
            .status,
            Status::NeverStarted(Some(2))
        );
        // `perfrun facts` by its own rule: no status, no counters.
        let facts =
            "cpu Apple M3\nos 26.5.2\nbuild 25F84\narch arm64\nfast_cores 4\ntwo_kinds 1\nend\n";
        assert_eq!(
            parse_record(facts, true).expect("reads").facts.fast_cores,
            4
        );
        assert!(
            parse_record(good, true).is_err(),
            "a run record is not facts"
        );
    }

    /// A private cache root for a test (the real refusal of a temp folder is
    /// tested on its own).
    fn test_launcher(tag: &str) -> (crate::testutil::TempDir, Launcher) {
        let tmp = crate::testutil::TempDir::new(tag);
        let root = tmp.path().join("cache");
        let owner = std::fs::metadata(tmp.path()).expect("meta").uid();
        private_dir(&root, owner).expect("root");
        let canonical = root.canonicalize().expect("canonical");
        let mut said = Vec::new();
        let l = launcher_in(&canonical, owner, &mut |w: &str| said.push(w.to_string()))
            .expect("the launcher builds");
        assert!(
            said.iter().any(|w| w == "building the launcher…"),
            "{said:?}"
        );
        (tmp, l)
    }

    /// A C program built for a run, in `dir`.
    fn program(dir: &Path, name: &str, source: &str) -> PathBuf {
        let src = dir.join(format!("{name}.c"));
        std::fs::write(&src, source).expect("source");
        let bin = dir.join(name);
        let ok = Command::new("cc")
            .args(["-O0", "-o"])
            .arg(&bin)
            .arg(&src)
            .status()
            .expect("cc");
        assert!(ok.success());
        bin.canonicalize().expect("canonical")
    }

    fn run(l: &Launcher, bin: &Path, deadline: u64, capture: bool) -> Measured {
        let host = HostDirs::from_env().expect("host");
        let tmp = crate::testutil::TempDir::new("perf-run");
        let tmpdir = tmp.path().canonicalize().expect("tmp");
        let cwd = tmpdir.join("run");
        std::fs::create_dir(&cwd).expect("cwd");
        let profile = crate::sandbox::render_perf_profile(&crate::sandbox::PerfSpec {
            host: &host,
            target_root: Path::new("/nonexistent-target"),
            bin,
            perfgo: &l.perfgo.path,
            tmpdir: &tmpdir,
        })
        .expect("profile");
        run_measured(&RunSpec {
            launcher: l,
            profile: &profile,
            deadline_secs: deadline,
            allowance: Duration::from_secs(60),
            program: bin,
            name: "tool",
            args: &["one".to_string()],
            cwd: &cwd,
            tmpdir: &tmpdir,
            capture,
            max_output: 1024 * 1024,
        })
        .expect("runs")
    }

    #[test]
    fn the_launcher_measures_ends_and_kills() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-launcher");
        // The cache is reused: no rebuild, the same binaries.
        let again = launcher_in(
            l.perfrun
                .path
                .parent()
                .and_then(Path::parent)
                .expect("root"),
            std::fs::metadata(tmp.path()).expect("m").uid(),
            &mut |w: &str| {
                assert_ne!(
                    w, "building the launcher…",
                    "a current cache is not rebuilt"
                )
            },
        )
        .expect("again");
        assert_eq!(again.perfrun, l.perfrun);
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        // argv[0], the streams through the launcher, an exit code that is the
        // program's own (65, 71 and 125 are never "never started").
        for code in [0, 3, 65, 71, 125] {
            let bin = program(
                &dir,
                &format!("ends{code}"),
                &format!(
                    "#include <stdio.h>\nint main(int c, char **v) {{ volatile unsigned long x = 0; \
                     for (unsigned long i = 0; i < 20000000UL; i++) x += i; printf(\"%s %s\\n\", v[0], v[1]); \
                     fprintf(stderr, \"err\\n\"); return {code}; }}\n"
                ),
            );
            let m = run(&l, &bin, 60, true);
            let Seen::Record(r) = &m.seen else {
                panic!("{code}: {:?}", m.seen)
            };
            assert_eq!(r.status, Status::Ok, "{code}");
            assert_eq!(r.end, Some(End::Exit(code)), "{code}");
            assert!(!r.killed);
            assert!(r.instructions.is_some_and(|i| i > 20_000_000), "{r:?}");
            assert!(
                r.cpu_us.is_some() && r.wall_us.is_some() && r.memory.is_some(),
                "{r:?}"
            );
            assert_eq!(m.stdout, b"tool one\n");
            assert_eq!(m.stderr, b"err\n");
            assert_eq!(m.launcher_exit, Some(0));
        }
        // A missing program never started.
        let m = run(&l, &dir.join("missing"), 60, true);
        let Seen::Record(r) = &m.seen else {
            panic!("{:?}", m.seen)
        };
        assert_eq!(r.status, Status::NeverStarted(Some(2)), "{r:?}");
        // perfrun's own deadline: a spinner that ignores SIGTERM and SIGXCPU.
        let spin = program(
            &dir,
            "spin",
            "#include <signal.h>\nint main(void) { signal(SIGTERM, SIG_IGN); signal(SIGXCPU, SIG_IGN); for (;;) {} }\n",
        );
        let t = Instant::now();
        let m = run(&l, &spin, 2, false);
        let Seen::Record(r) = &m.seen else {
            panic!("{:?}", m.seen)
        };
        assert_eq!(r.status, Status::Timeout);
        assert!(r.killed && r.end == Some(End::Signal(9)), "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(10));
        // A fork is killed by the profile — a SIGKILL perfrun did not send.
        let forker = program(
            &dir,
            "forker",
            "#include <unistd.h>\nint main(void) { fork(); return 0; }\n",
        );
        let m = run(&l, &forker, 60, true);
        let Seen::Record(r) = &m.seen else {
            panic!("{:?}", m.seen)
        };
        assert_eq!((r.end, r.killed), (Some(End::Signal(9)), false), "{r:?}");
        // A changed binary is refused by its hash.
        std::fs::write(&l.perfgo.path, b"changed").expect("write");
        assert!(l.check().is_err());
    }

    /// The pid of the process running `bin`, once it runs.
    fn pid_of(bin: &Path) -> u32 {
        for _ in 0..400 {
            // The executable's own name (`ucomm`): perfrun's command line
            // holds the program's path too, and argv[0] is "tool".
            let name = bin
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            let out = Command::new("ps")
                .args(["-axo", "pid=,ucomm="])
                .output()
                .expect("ps");
            if let Some(pid) = String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
                let (pid, comm) = l.trim().split_once(' ')?;
                (comm.trim() == name)
                    .then(|| pid.parse::<u32>().ok())
                    .flatten()
            }) {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("{} never ran", bin.display());
    }

    fn alive(pid: u32) -> bool {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !stat.is_empty() && !stat.starts_with('Z')
    }

    #[test]
    fn a_launcher_killed_alone_takes_its_program_with_it() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-kill9");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let spin = program(
            &dir,
            "spin9",
            "#include <signal.h>\nint main(void) { signal(SIGTERM, SIG_IGN); for (;;) {} }\n",
        );
        let l = Arc::new(l);
        let worker = {
            let l = Arc::clone(&l);
            let spin = spin.clone();
            std::thread::spawn(move || run(&l, &spin, 60, false))
        };
        let pid = pid_of(&spin);
        let parent = Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        let perfrun: u32 = String::from_utf8_lossy(&parent.stdout)
            .trim()
            .parse()
            .expect("ppid");
        assert!(Command::new("/bin/kill")
            .args(["-KILL", &perfrun.to_string()])
            .status()
            .expect("kill")
            .success());
        let t = Instant::now();
        let m = worker.join().expect("joins");
        assert!(matches!(m.seen, Seen::NoRecord(_)), "{:?}", m.seen);
        assert!(t.elapsed() < Duration::from_secs(5), "answered at once");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!alive(pid), "the program died with its launcher");
    }

    #[test]
    fn the_program_sees_default_signal_dispositions() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-signals");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let bin = program(
            &dir,
            "dispositions",
            "#include <signal.h>\n#include <stdio.h>\nint main(void) {\n\
             struct sigaction a; int dfl = 1; int s;\n\
             for (s = 1; s < NSIG; s++) { if (s == SIGKILL || s == SIGSTOP) continue;\n\
             if (sigaction(s, 0, &a) == 0 && a.sa_handler != SIG_DFL) dfl = 0; }\n\
             sigset_t m; sigprocmask(SIG_BLOCK, 0, &m); int empty = 1;\n\
             for (s = 1; s < NSIG; s++) if (sigismember(&m, s)) empty = 0;\n\
             printf(\"dfl %d empty %d\\n\", dfl, empty); return 0; }\n",
        );
        let m = run(&l, &bin, 60, true);
        assert_eq!(m.stdout, b"dfl 1 empty 1\n", "{:?}", m.seen);
    }

    /// Run by [`a_cancel_while_the_program_runs_leaves_nothing`] in its own
    /// process (a cancellation is for the whole process); a no-op otherwise.
    #[test]
    fn cancel_perf_child_body() {
        let Some(dir) = std::env::var_os("RUHARNESS_PERF_CANCEL_TEST") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let (_tmp, l) = test_launcher("perf-cancel");
        let spin = program(
            &dir,
            "spincancel",
            "#include <signal.h>\nint main(void) { signal(SIGTERM, SIG_IGN); for (;;) {} }\n",
        );
        let l = Arc::new(l);
        let worker = {
            let l = Arc::clone(&l);
            let spin = spin.clone();
            std::thread::spawn(move || {
                let host = HostDirs::from_env().expect("host");
                let tmp = crate::testutil::TempDir::new("perf-run");
                let tmpdir = tmp.path().canonicalize().expect("tmp");
                let profile = crate::sandbox::render_perf_profile(&crate::sandbox::PerfSpec {
                    host: &host,
                    target_root: Path::new("/nonexistent-target"),
                    bin: &spin,
                    perfgo: &l.perfgo.path,
                    tmpdir: &tmpdir,
                })
                .expect("profile");
                run_measured(&RunSpec {
                    launcher: &l,
                    profile: &profile,
                    deadline_secs: 60,
                    allowance: Duration::from_secs(60),
                    program: &spin,
                    name: "tool",
                    args: &[],
                    cwd: &tmpdir,
                    tmpdir: &tmpdir,
                    capture: false,
                    max_output: 1024,
                })
            })
        };
        let pid = pid_of(&spin);
        println!("program={pid}");
        exec::kill_live_process_groups();
        let result = worker.join().expect("joins");
        assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!alive(pid), "the program is dead");
        println!("cancel-ok");
        std::process::exit(0);
    }

    #[test]
    fn a_cancel_while_the_program_runs_leaves_nothing() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-cancel-parent");
        let out = Command::new(std::env::current_exe().expect("exe"))
            .args([
                "--exact",
                "perf::launcher::tests::cancel_perf_child_body",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RUHARNESS_PERF_CANCEL_TEST", tmp.path())
            .output()
            .expect("re-exec the test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("cancel-ok"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn a_cache_a_build_tool_may_write_is_refused() {
        let mut host = HostDirs::from_env().expect("host");
        let default_tmpdir = host.tmpdir.clone();
        host.perf_cache = PathBuf::from("/private/var/folders/xy/T/ruharness/perf");
        assert!(refuse_cache(&host.perf_cache, &host)
            .expect("refused")
            .contains("temporary folder"));
        // A TMPDIR under the home folder holding the cache.
        host.tmpdir = Some(host.home.clone());
        host.perf_cache = host.home.join("Library/Caches/ruharness/perf");
        assert!(refuse_cache(&host.perf_cache, &host)
            .expect("refused")
            .contains("TMPDIR"));
        // The default TMPDIR under /var/folders is accepted.
        host.tmpdir = default_tmpdir;
        host.cargo_home = None;
        assert_eq!(refuse_cache(&host.perf_cache, &host), None);
        host.cargo_home = Some(host.home.join("Library"));
        assert!(refuse_cache(&host.perf_cache, &host)
            .expect("refused")
            .contains("CARGO_HOME"));
    }

    /// The compiler is found through root-owned paths only — never `xcrun`'s
    /// per-user cache, never a shim (build note 9).
    #[test]
    fn the_compiler_is_root_owned() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let c = find_compiler().expect("a compiler on this Mac");
        for p in [&c.clang, &c.ld, &c.sdk] {
            assert!(root_owned(p).is_ok(), "{}", p.display());
        }
        assert_ne!(c.clang, Path::new("/usr/bin/clang"));
        assert!(c.version.contains("clang"), "{}", c.version);
    }

    #[test]
    fn root_owned_paths() {
        assert!(root_owned(Path::new("/usr/bin/true")).is_ok());
        let mine = std::env::temp_dir();
        assert!(
            root_owned(&mine).is_err(),
            "a temp folder of the user's is not root's"
        );
    }
}
