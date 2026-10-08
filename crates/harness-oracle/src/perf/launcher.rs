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
    find_compiler_in(&developer_folders())
}

/// [`find_compiler`] among `folders`, the selected one first.
fn find_compiler_in(folders: &[PathBuf]) -> Result<Compiler, Error> {
    let mut first_failure: Option<PathBuf> = None;
    for (i, dev) in folders.iter().enumerate() {
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

/// The launcher when its cache is current — never built here (`perf show`,
/// §3.9): `None` when there is none, or it would need building.
pub(crate) fn existing_launcher(host: &HostDirs) -> Option<Launcher> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let root = host.perf_cache.canonicalize().ok()?;
    refuse_cache(&root, host).is_none().then_some(())?;
    let compiler = find_compiler().ok()?;
    let dir = root.join(version_name(&compiler));
    let shared = lock_file(&dir.join(".lock"), false).ok()?;
    let (perfrun, perfgo) = load_built(&dir)?;
    Some(Launcher {
        perfrun,
        perfgo,
        _shared: shared,
    })
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
/// 3. cancelled() is checked first: cancelled → close the socket without
///    the go-ahead (the program never runs); else retake the lock (which
///    refuses once cancelled), register the program's group (killed first)
///    and write the go-ahead;
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
        // Output over the cap is judged first (§3.3 "Judging a run", step
        // 1), whatever else was seen: a stream's last bytes may reach the
        // cap only after perfrun's record was read in full.
        let seen = if overflow.load(Ordering::SeqCst) {
            Seen::Overflow
        } else {
            seen
        };
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
    #[cfg(test)]
    hooks::at(hooks::Stage::ChildLine, program_pid);
    #[cfg(test)]
    let held = hooks::hold_stdout().then(|| out_buf.lock());
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
    #[cfg(test)]
    drop(held);
    match got {
        ReadEnd::Done => {
            let text = String::from_utf8_lossy(&record).into_owned();
            // Unregister the program's group, then the bye: perfrun reaps it
            // only after that, so no later kill can reach a reused pid.
            drop(program_reg);
            #[cfg(test)]
            hooks::at(hooks::Stage::Bye, program_pid);
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

/// Where a test steps into [`run_measured`], at the moments the harness
/// side's order is about (§3.3 *The harness side*). Tests only; each hook
/// holds for the runs of the thread that set it.
#[cfg(test)]
mod hooks {
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Stage {
        /// perfrun's child line is read: the cancel check and the go-ahead
        /// come next.
        ChildLine,
        /// The record is read and the program's group unregistered: the bye
        /// comes next.
        Bye,
    }

    type Hook = Box<dyn FnMut(Stage, u32)>;

    thread_local! {
        static AT: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static HOLD_STDOUT: Cell<bool> = const { Cell::new(false) };
    }

    /// Call `hook` at each stage, with the program's pid.
    pub(super) fn set(hook: impl FnMut(Stage, u32) + 'static) {
        AT.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    }

    /// Hold the stdout buffer's lock from the go-ahead until the record is
    /// read: the stream's drain then meets the cap only after the record,
    /// as a drain thread kept off the CPU while the program exits would.
    pub(super) fn set_hold_stdout(hold: bool) {
        HOLD_STDOUT.with(|h| h.set(hold));
    }

    pub(super) fn at(stage: Stage, program_pid: u32) {
        AT.with(|h| {
            if let Some(hook) = h.borrow_mut().as_mut() {
                hook(stage, program_pid);
            }
        });
    }

    pub(super) fn hold_stdout() -> bool {
        HOLD_STDOUT.with(Cell::get)
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
            ("perf-launcher-2", PINNED_SOURCES),
            "perfrun.c or perfgo.c changed: bump PERF_LAUNCHER and PINNED_SOURCES"
        );
    }

    /// The sources' hash at [`harness_core::perf::PERF_LAUNCHER`].
    const PINNED_SOURCES: &str = "dafbfe6534c4eb17";

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

    /// A test run's shape; [`run`] is the usual one.
    struct Opts<'a> {
        deadline: u64,
        capture: bool,
        args: Vec<String>,
        /// The target root the profile denies reads under.
        target_root: &'a Path,
        /// The program the profile lets run, when not the one run.
        allowed: Option<&'a Path>,
        /// A profile of the test's own instead of the perf profile.
        profile: Option<&'a str>,
        max_output: usize,
    }

    fn opts(deadline: u64, capture: bool) -> Opts<'static> {
        Opts {
            deadline,
            capture,
            args: vec!["one".to_string()],
            target_root: Path::new("/nonexistent-target"),
            allowed: None,
            profile: None,
            max_output: 1024 * 1024,
        }
    }

    fn run(l: &Launcher, bin: &Path, deadline: u64, capture: bool) -> Measured {
        run_with(l, bin, &opts(deadline, capture))
    }

    fn run_with(l: &Launcher, bin: &Path, o: &Opts<'_>) -> Measured {
        let host = HostDirs::from_env().expect("host");
        let tmp = crate::testutil::TempDir::new("perf-run");
        let tmpdir = tmp.path().canonicalize().expect("tmp");
        let cwd = tmpdir.join("run");
        std::fs::create_dir(&cwd).expect("cwd");
        let profile = match o.profile {
            Some(p) => p.to_string(),
            None => crate::sandbox::render_perf_profile(&crate::sandbox::PerfSpec {
                host: &host,
                target_root: o.target_root,
                bin: o.allowed.unwrap_or(bin),
                perfgo: &l.perfgo.path,
                tmpdir: &tmpdir,
            })
            .expect("profile"),
        };
        run_measured(&RunSpec {
            launcher: l,
            profile: &profile,
            deadline_secs: o.deadline,
            allowance: Duration::from_secs(60),
            program: bin,
            name: "tool",
            args: &o.args,
            cwd: &cwd,
            tmpdir: &tmpdir,
            capture: o.capture,
            max_output: o.max_output,
        })
        .expect("runs")
    }

    /// The run's record, or a panic naming what was seen instead.
    fn record(m: &Measured) -> &Record {
        match &m.seen {
            Seen::Record(r) => r,
            other => panic!("{other:?}: {}", String::from_utf8_lossy(&m.stderr)),
        }
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

    /// perfrun only waits while the program runs (§3.3 step 5; §4 "perfrun
    /// stays idle meanwhile"): its own CPU time stays near zero while the
    /// program sleeps — a busy perfrun would take a fast core from the
    /// program and raise the load its record reports.
    #[test]
    fn perfrun_stays_idle_while_the_program_runs() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-idle");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let sleeper = program(
            &dir,
            "sleeper",
            "#include <unistd.h>\nint main(void) { sleep(3); return 0; }\n",
        );
        let l = Arc::new(l);
        // A reading counts only when taken while the program still sleeps:
        // perfrun gone, or the program already ended, means the reading came
        // too late on a busy Mac, and the run is tried again — three runs at
        // most. Every run's record is checked all the same.
        let mut used = None;
        for _ in 0..3 {
            let worker = {
                let l = Arc::clone(&l);
                let sleeper = sleeper.clone();
                std::thread::spawn(move || run(&l, &sleeper, 60, false))
            };
            let (program, perfrun) = pid_and_parent_of(&sleeper);
            std::thread::sleep(Duration::from_millis(1500));
            let reading = cpu_time(perfrun).filter(|_| alive(program));
            let m = worker.join().expect("joins");
            let r = record(&m);
            assert_eq!(
                (&r.status, r.end),
                (&Status::Ok, Some(End::Exit(0))),
                "{r:?}"
            );
            if reading.is_some() {
                used = reading;
                break;
            }
        }
        let used =
            used.expect("perfrun's CPU time was never read while its program slept, in 3 runs");
        assert!(
            used < Duration::from_millis(200),
            "perfrun used {used:?} of CPU while the program slept"
        );
    }

    /// perfrun takes the longest deadline the harness can ask for — the
    /// longest `[oracle] timeout_secs` plus step 1's extra minute — and no
    /// more.
    #[test]
    fn perfrun_takes_the_longest_deadline() {
        let max: u64 = PERFRUN_C
            .lines()
            .find_map(|l| l.strip_prefix("#define DEADLINE_MAX "))
            .expect("perfrun.c's DEADLINE_MAX")
            .trim()
            .parse()
            .expect("a number");
        assert_eq!(
            max,
            crate::MAX_TIMEOUT_SECS + crate::perf::measure::STEP1_EXTRA_SECS
        );
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-deadline");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let bin = program(&dir, "quick", "int main(void) { return 0; }\n");
        let r = record(&run(&l, &bin, max, true)).clone();
        assert_eq!((r.status, r.end), (Status::Ok, Some(End::Exit(0))));
        let m = run(&l, &bin, max + 1, true);
        assert!(
            matches!(&m.seen, Seen::NoRecord(w) if w.contains("bad deadline")),
            "{:?}",
            m.seen
        );
    }

    /// Every process in one `ps -axww -o <fields>` snapshot: each line's
    /// first `words` columns (none of them holds a space) and the rest of
    /// the line, the last column, which may.
    fn ps_rows(fields: &str, words: usize) -> Vec<(Vec<String>, String)> {
        let out = Command::new("ps")
            .args(["-axww", "-o", fields])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let mut rest = l.trim();
                let mut lead = Vec::with_capacity(words);
                for _ in 0..words {
                    let (word, after) = rest.split_once(char::is_whitespace)?;
                    lead.push(word.to_string());
                    rest = after.trim_start();
                }
                Some((lead, rest.to_string()))
            })
            .collect()
    }

    /// The pid of the process running `bin` once it runs, waiting up to 30
    /// seconds (a new binary's first exec can be slow on a busy Mac). Only
    /// a process this test process started counts — perfrun's child, or its
    /// own: another copy of these tests running at once on the machine (a
    /// second worktree) runs programs of the very same names.
    fn pid_of(bin: &Path) -> u32 {
        pid_and_parent_of(bin).0
    }

    /// [`pid_of`] and that process's parent, both from the one `ps`
    /// snapshot that found it: a second `ps` for the parent could come back
    /// empty if the process ended in between.
    fn pid_and_parent_of(bin: &Path) -> (u32, u32) {
        // The executable's own name (`ucomm`): perfrun's command line holds
        // the program's path too, and argv[0] is "tool".
        let name = bin
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            let rows = ps_rows("pid=,ppid=,ucomm=", 2);
            let parents: std::collections::HashMap<u32, u32> = rows
                .iter()
                .filter_map(|(w, _)| Some((w[0].parse().ok()?, w[1].parse().ok()?)))
                .collect();
            if let Some(pid) = rows
                .iter()
                .filter(|(_, comm)| *comm == name)
                .filter_map(|(w, _)| w[0].parse::<u32>().ok())
                .find(|&pid| descends_from_this_process(pid, &parents))
            {
                // The walk above found `pid`'s parent in this map.
                return (pid, parents[&pid]);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("{} never ran", bin.display());
    }

    /// Whether `pid` descends from this test process, by the parents of one
    /// `ps` snapshot. launchd (pid 1) ends the walk: an orphan it adopted is
    /// nobody's here.
    fn descends_from_this_process(pid: u32, parents: &std::collections::HashMap<u32, u32>) -> bool {
        let me = std::process::id();
        let mut at = pid;
        // A chain is never longer than the snapshot; the bound keeps a
        // malformed one from looping.
        for _ in 0..parents.len() {
            match parents.get(&at) {
                Some(&parent) if parent == me => return true,
                Some(&parent) if parent > 1 => at = parent,
                _ => return false,
            }
        }
        false
    }

    fn alive(pid: u32) -> bool {
        let stat = ps(pid, "stat");
        !stat.is_empty() && !stat.starts_with('Z')
    }

    /// Whether `pid` is gone or a zombie within `within`, polled: a killed
    /// process can take a moment to die on a busy Mac.
    fn dead_within(pid: u32, within: Duration) -> bool {
        let until = Instant::now() + within;
        loop {
            if !alive(pid) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// `ps -o <field>= -p <pid>`, trimmed: "" once the pid is gone.
    fn ps(pid: u32, field: &str) -> String {
        let out = Command::new("ps")
            .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn parent_of(pid: u32) -> u32 {
        ps(pid, "ppid").parse().expect("ppid")
    }

    /// The CPU time `pid` has used (`ps`'s `[hh:]mm:ss.hh`), or `None` once
    /// it is gone (`ps` prints nothing): the caller says what a gone
    /// process means. Anything else `ps` prints that is not a time panics.
    fn cpu_time(pid: u32) -> Option<Duration> {
        let text = ps(pid, "time");
        if text.is_empty() {
            return None;
        }
        let mut secs = 0.0;
        for part in text.split(':') {
            let v: f64 = part.parse().unwrap_or_else(|_| panic!("ps time {text:?}"));
            secs = secs * 60.0 + v;
        }
        Some(Duration::from_secs_f64(secs))
    }

    /// [`cpu_time`] of a process that is gone is `None`, not a panic: on a
    /// busy Mac the reading can come just after the process ended.
    #[test]
    fn cpu_time_of_a_gone_process_is_none() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let mut gone = Command::new("/usr/bin/true").spawn().expect("true");
        let pid = gone.id();
        gone.wait().expect("reaped");
        assert_eq!(cpu_time(pid), None);
        assert!(cpu_time(std::process::id()).is_some(), "this process runs");
    }

    /// [`pid_of`] takes only a process this test process started: a copy of
    /// the same executable that another run of these tests runs elsewhere
    /// on the machine is never taken for it, even while the decoy is the
    /// only one running. Like another worktree's program under its own
    /// perfrun, the decoy has a living parent that is not this process (a
    /// subshell launchd adopted), so rejecting only launchd's children is
    /// not enough to pass.
    #[test]
    fn pid_of_never_takes_another_run_s_program() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-pid-of");
        let (ours, theirs) = (tmp.path().join("ours"), tmp.path().join("theirs"));
        std::fs::create_dir(&ours).expect("ours");
        std::fs::create_dir(&theirs).expect("theirs");
        let bin = program(
            &ours,
            "lookalike",
            "#include <unistd.h>\nint main(void) { sleep(30); return 0; }\n",
        );
        let decoy = theirs.join("lookalike");
        std::fs::copy(&bin, &decoy).expect("the decoy");
        // A subshell starts the decoy, says its pid and waits for it; the
        // outer shell exits, so launchd adopts the subshell, which stays
        // alive as the decoy's parent. Only the first line is read: the
        // subshell and the decoy keep the pipe open.
        let mut outer = Command::new("/bin/sh")
            .arg("-c")
            .arg("( \"$0\" & echo $!; wait ) </dev/null 2>/dev/null & sleep 0.5")
            .arg(&decoy)
            .stdout(Stdio::piped())
            .spawn()
            .expect("sh");
        let mut first = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(outer.stdout.take().expect("stdout")),
            &mut first,
        )
        .expect("the decoy's pid");
        let decoy_pid: u32 = first.trim().parse().expect("the decoy's pid");
        // Until the outer shell is gone, the decoy still descends from us.
        outer.wait().expect("the outer shell ends");
        let until = Instant::now() + Duration::from_secs(30);
        while ps(decoy_pid, "ucomm") != "lookalike" {
            assert!(Instant::now() < until, "the decoy never ran");
            std::thread::sleep(Duration::from_millis(25));
        }
        let decoy_parent = parent_of(decoy_pid);
        // Ours starts a moment later, while pid_of already looks.
        let starter = {
            let bin = bin.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                Command::new(&bin).spawn()
            })
        };
        let found = pid_of(&bin);
        let mut started = starter.join().expect("joins").expect("ours starts");
        let _ = started.kill();
        let _ = started.wait();
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &decoy_pid.to_string()])
            .status();
        assert!(
            decoy_parent > 1 && decoy_parent != std::process::id(),
            "the decoy's parent ({decoy_parent}) lives and is not this process"
        );
        assert_eq!(
            found,
            started.id(),
            "pid_of took the decoy ({decoy_pid}) for this process's own"
        );
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
        let (pid, perfrun) = pid_and_parent_of(&spin);
        assert!(Command::new("/bin/kill")
            .args(["-KILL", &perfrun.to_string()])
            .status()
            .expect("kill")
            .success());
        let t = Instant::now();
        let m = worker.join().expect("joins");
        assert!(matches!(m.seen, Seen::NoRecord(_)), "{:?}", m.seen);
        assert!(t.elapsed() < Duration::from_secs(5), "answered at once");
        assert!(
            dead_within(pid, Duration::from_secs(5)),
            "the program died with its launcher"
        );
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

    /// [`run_with`], and the program's pid — the id of its group, which
    /// perfrun's child leads (§3.3 step 2) — from the harness's child line;
    /// `None` when there was none.
    fn run_with_group(l: &Launcher, bin: &Path, o: &Opts<'_>) -> (Measured, Option<u32>) {
        let group = std::rc::Rc::new(std::cell::Cell::new(None));
        {
            let group = std::rc::Rc::clone(&group);
            hooks::set(move |stage, pid| {
                if stage == hooks::Stage::ChildLine {
                    group.set(Some(pid));
                }
            });
        }
        let m = run_with(l, bin, o);
        (m, group.get())
    }

    /// What a run left running (not a zombie), as `pid command-line` lines:
    /// a process in the program's group `group` — the program cannot leave
    /// it, and whatever it starts joins it (§3.3 step 2) — or one whose
    /// argv[0] is `bin`'s exact path, a copy of itself the program started.
    /// Both are this run's own, never another run's of the same tests at
    /// once on the machine (a second worktree, programs of the very same
    /// names). Looked for again while anything is left, for up to `within`:
    /// a program killed at its run's end can take a moment to go on a busy
    /// Mac.
    fn left_running(group: u32, bin: &Path, within: Duration) -> Vec<String> {
        let path = bin.to_string_lossy();
        let until = Instant::now() + within;
        loop {
            let left: Vec<String> = ps_rows("pid=,pgid=,stat=,args=", 3)
                .into_iter()
                .filter(|(w, args)| {
                    let in_group = w[1].parse::<u32>().ok() == Some(group);
                    let started_as_bin = args
                        .strip_prefix(path.as_ref())
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '));
                    !w[2].starts_with('Z') && (in_group || started_as_bin)
                })
                .map(|(w, args)| format!("{} {args}", w[0]))
                .collect();
            if left.is_empty() || Instant::now() >= until {
                return left;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// [`left_running`] finds what a run left — anything in its group, a
    /// copy started by the program's exact path — and never a process of the
    /// same name that another run of these tests left elsewhere on the
    /// machine, nor one whose path only begins with the program's (a longer
    /// name beside it).
    #[test]
    fn only_a_run_s_own_leftovers_count() {
        if !cfg!(target_os = "macos") {
            return;
        }
        use std::os::unix::process::CommandExt;
        let tmp = crate::testutil::TempDir::new("perf-left");
        let (ours, theirs) = (tmp.path().join("ours"), tmp.path().join("theirs"));
        std::fs::create_dir(&ours).expect("ours");
        std::fs::create_dir(&theirs).expect("theirs");
        let bin = program(
            &ours,
            "leftover",
            "#include <unistd.h>\nint main(void) { sleep(30); return 0; }\n",
        );
        let decoy = theirs.join("leftover");
        std::fs::copy(&bin, &decoy).expect("the decoy");
        // Another run's leftover: the same name, its own folder and group.
        let mut other = Command::new(&decoy)
            .process_group(0)
            .spawn()
            .expect("the decoy");
        // Not a copy either: a program beside it whose path only begins with
        // the program's — the path must be the program's exact one.
        let longer = ours.join("leftover-old");
        std::fs::copy(&bin, &longer).expect("the longer name");
        let mut other_longer = Command::new(&longer)
            .process_group(0)
            .spawn()
            .expect("the longer name");
        // The program as perfrun runs it — argv[0] "tool", leading its group
        // — and something else it started, in that group.
        let mut program_run = Command::new(&bin)
            .arg0("tool")
            .process_group(0)
            .spawn()
            .expect("the program");
        let group = program_run.id();
        let mut started = Command::new("/bin/sleep")
            .arg("30")
            .process_group(i32::try_from(group).expect("a pid"))
            .spawn()
            .expect("sleep");
        let in_group = left_running(group, &bin, Duration::ZERO);
        for child in [&mut program_run, &mut started] {
            let _ = child.kill();
            let _ = child.wait();
        }
        // A copy started by the program's exact path, in a group of its own.
        let mut copy = Command::new(&bin)
            .process_group(0)
            .spawn()
            .expect("the copy");
        let by_path = left_running(group, &bin, Duration::ZERO);
        let _ = copy.kill();
        let _ = copy.wait();
        // Only the decoy and the longer name are left now.
        let none = left_running(group, &bin, Duration::from_secs(5));
        for child in [&mut other, &mut other_longer] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let has =
            |left: &[String], pid: u32| left.iter().any(|l| l.starts_with(&format!("{pid} ")));
        assert!(
            has(&in_group, group) && has(&in_group, started.id()),
            "{in_group:?}"
        );
        assert!(has(&by_path, copy.id()), "{by_path:?}");
        assert!(
            none.is_empty(),
            "a process not this run's was taken for this run's: {none:?}"
        );
    }

    /// Every way to start a process is killed on trying — a SIGKILL perfrun
    /// did not send — and nothing it would have started is left running.
    /// §3.12's "nothing the program starts outlives its run" rests on this
    /// rule on macOS: perfrun kills the program's group only on its
    /// deadline, a SIGTERM or the harness's end, never after a normal end.
    #[test]
    fn every_way_to_start_a_process_is_killed() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-spawns");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        // A started copy of the program would sleep, so a survivor shows.
        let head =
            "#include <spawn.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\
                    #include <unistd.h>\nextern char **environ;\nint main(int c, char **v) {\n\
                    if (c > 1 && strcmp(v[1], \"child\") == 0) { sleep(30); return 0; }\n";
        for (name, body) in [
            ("bysystem", "system(\"true\");"),
            ("bypopen", "FILE *f = popen(\"true\", \"r\"); if (f) pclose(f);"),
            (
                "byspawn",
                "pid_t p; char *a[] = { v[1], \"child\", 0 }; posix_spawn(&p, v[1], 0, 0, a, environ);",
            ),
            (
                "byvfork",
                "char *a[] = { v[1], \"child\", 0 }; if (vfork() == 0) { execv(v[1], a); _exit(0); }",
            ),
        ] {
            let bin = program(&dir, name, &format!("{head}{body}\nreturn 0; }}\n"));
            // Its own path, the one exec the profile allows.
            let o = Opts {
                args: vec![bin.to_string_lossy().into_owned()],
                ..opts(60, true)
            };
            let (m, group) = run_with_group(&l, &bin, &o);
            let r = record(&m);
            assert_eq!(
                (&r.status, r.end, r.killed),
                (&Status::Ok, Some(End::Signal(9)), false),
                "{name}: {r:?}"
            );
            assert_eq!(m.launcher_exit, Some(0), "{name}");
            // This run's own leftovers only: another worktree may be running
            // these very programs at this moment.
            let left = left_running(
                group.expect("the child line"),
                &bin,
                Duration::from_secs(5),
            );
            assert!(
                left.is_empty(),
                "{name}: a started copy outlived the run: {left:?}"
            );
        }
    }

    /// No signal leaves the sandbox — not to perfrun (the program's parent),
    /// not to the harness, both the person's own processes — only to the
    /// program itself (§3.4). Signal 0 is checked like any other.
    #[test]
    fn no_signal_leaves_the_sandbox() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-signal-out");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let bin = program(
            &dir,
            "signaller",
            "#include <errno.h>\n#include <signal.h>\n#include <stdio.h>\n#include <stdlib.h>\n\
             #include <unistd.h>\nstatic int try_kill(pid_t p) { return kill(p, 0) == 0 ? 0 : errno; }\n\
             int main(int c, char **v) { printf(\"self %d parent %d harness %d\\n\", \
             try_kill(getpid()), try_kill(getppid()), try_kill((pid_t)atoi(v[1]))); return 0; }\n",
        );
        let o = Opts {
            args: vec![std::process::id().to_string()],
            ..opts(60, true)
        };
        let m = run_with(&l, &bin, &o);
        assert_eq!(
            String::from_utf8_lossy(&m.stdout),
            "self 0 parent 1 harness 1\n",
            "{:?}",
            m.seen
        );
    }

    /// A run cannot read the other side's binary under the target, only
    /// its own (§3.4): the profile's target root is a real one here, with
    /// both sides' slots in it.
    #[test]
    fn a_run_cannot_read_the_other_side() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-sides");
        let target = tmp.path().join("t");
        let slots = target.join("migration/build/.perf/bin");
        let ours = slots.join("p000");
        let theirs = slots.join("p012");
        std::fs::create_dir_all(&ours).expect("p000");
        std::fs::create_dir_all(&theirs).expect("p012");
        let reader = program(
            &ours,
            "reader",
            "#include <errno.h>\n#include <fcntl.h>\n#include <stdio.h>\n\
             static int try_open(const char *p) { int fd = open(p, O_RDONLY); return fd >= 0 ? 0 : errno; }\n\
             int main(int c, char **v) { printf(\"other %d own %d\\n\", try_open(v[1]), try_open(v[2])); return 0; }\n",
        );
        let other = theirs.join("reader");
        std::fs::copy(&reader, &other).expect("the other side");
        let o = Opts {
            args: vec![
                other.to_string_lossy().into_owned(),
                reader.to_string_lossy().into_owned(),
            ],
            target_root: &target,
            ..opts(60, true)
        };
        let m = run_with(&l, &reader, &o);
        assert_eq!(
            String::from_utf8_lossy(&m.stdout),
            "other 1 own 0\n",
            "{:?}",
            m.seen
        );
    }

    /// Nothing is opened or started for the program by the system (§3.12):
    /// LaunchServices' open, Apple events and a launchd job are refused,
    /// and so is reaching the services that do them — while an ordinary
    /// service stays reachable. The sandbox is only asked: nothing opens.
    #[test]
    fn nothing_opens_or_starts_outside_the_sandbox() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-no-open");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let names = [
            "com.apple.coreservices.launchservicesd",
            "com.apple.CoreServices.coreservicesd",
            "com.apple.coreservices.appleevents",
            "com.apple.lsd.open",
            "com.apple.xpc.smd",
            "com.apple.xpc.loginitemregisterd",
            "com.apple.system.opendirectoryd.libinfo",
        ];
        let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
        let probe = program(
            &dir,
            "opener",
            &format!(
                "#include <servers/bootstrap.h>\n#include <stdio.h>\n#include <unistd.h>\n\
                 int sandbox_check(pid_t pid, const char *operation, int type, ...);\n\
                 int main(void) {{\n\
                 const char *ops[] = {{ \"lsopen\", \"appleevent-send\", \"job-creation\" }};\n\
                 for (int i = 0; i < 3; i++) printf(\"%s %d\\n\", ops[i], sandbox_check(getpid(), ops[i], 0));\n\
                 const char *names[] = {{ {} }};\n\
                 for (int i = 0; i < {}; i++) {{ mach_port_t p = MACH_PORT_NULL;\n\
                 printf(\"%s %d\\n\", names[i], bootstrap_look_up(bootstrap_port, names[i], &p) == BOOTSTRAP_NOT_PRIVILEGED); }}\n\
                 return 0; }}\n",
                quoted.join(", "),
                names.len()
            ),
        );
        let m = run(&l, &probe, 60, true);
        let mut expected = "lsopen 1\nappleevent-send 1\njob-creation 1\n".to_string();
        for n in names {
            // 1: refused by the sandbox; the last is an ordinary service.
            let refused = !n.starts_with("com.apple.system.");
            expected.push_str(&format!("{n} {}\n", u8::from(refused)));
        }
        assert_eq!(String::from_utf8_lossy(&m.stdout), expected, "{:?}", m.seen);
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
        assert!(
            dead_within(pid, Duration::from_secs(5)),
            "the program is dead"
        );
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

    /// perfrun started by hand as [`run_measured`] starts it, the go-ahead
    /// held back: the harness's end of the socket, perfrun, and its child
    /// (the program's pid once it runs).
    struct ByHand {
        ours: UnixStream,
        perfrun: std::process::Child,
        child: u32,
    }

    /// A fresh 0700 run folder, its `run/` working folder inside, and the
    /// perf profile for `bin` there.
    fn run_dir(l: &Launcher, bin: &Path) -> (crate::testutil::TempDir, PathBuf, String) {
        let tmp = crate::testutil::TempDir::new("perf-run");
        let cwd = tmp.path().join("run");
        std::fs::create_dir(&cwd).expect("cwd");
        let host = HostDirs::from_env().expect("host");
        let profile = crate::sandbox::render_perf_profile(&crate::sandbox::PerfSpec {
            host: &host,
            target_root: Path::new("/nonexistent-target"),
            bin,
            perfgo: &l.perfgo.path,
            tmpdir: tmp.path(),
        })
        .expect("profile");
        (tmp, cwd, profile)
    }

    fn by_hand(l: &Launcher, profile: &str, bin: &Path, cwd: &Path) -> ByHand {
        use std::os::unix::process::CommandExt;
        let (mut ours, theirs) = UnixStream::pair().expect("a socket pair");
        let perfrun = Command::new(&l.perfrun.path)
            .args(["run", profile])
            .arg(&l.perfgo.path)
            .arg("60")
            .arg(bin)
            .arg("tool")
            .current_dir(cwd)
            .env_clear()
            .env("TMPDIR", cwd.parent().expect("the run folder"))
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(theirs)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("perfrun starts");
        let mut line = Vec::new();
        let got = read_until(
            &mut ours,
            &mut line,
            CHILD_LINE_MAX,
            Instant::now() + Duration::from_secs(30),
            |b| b.ends_with(b"\n"),
            &|| false,
        );
        assert!(matches!(got, ReadEnd::Done), "{line:?}");
        let child = std::str::from_utf8(&line)
            .ok()
            .and_then(|l| l.strip_prefix("child "))
            .and_then(|l| l.trim_end().parse().ok())
            .expect("the child line");
        ByHand {
            ours,
            perfrun,
            child,
        }
    }

    impl ByHand {
        /// Wait until the child runs perfgo, which then waits for the
        /// go-ahead (a new binary's first exec can be slow on a busy Mac).
        fn wait_for_perfgo(&self) {
            for _ in 0..1200 {
                if ps(self.child, "ucomm") == "perfgo" {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("the child never ran perfgo");
        }

        /// The record, read to its end.
        fn record(&mut self) -> Record {
            let mut text = Vec::new();
            let got = read_until(
                &mut self.ours,
                &mut text,
                RECORD_MAX + 1,
                Instant::now() + Duration::from_secs(30),
                |b| b == b"end\n" || b.ends_with(b"\nend\n"),
                &|| false,
            );
            assert!(matches!(got, ReadEnd::Done), "{text:?}");
            parse_record(&String::from_utf8_lossy(&text), false).expect("a record")
        }
    }

    /// `child`'s exit code once it exits, within `within`.
    fn exits_within(child: &mut std::process::Child, within: Duration) -> Option<i32> {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if let Some(status) = child.try_wait().expect("try_wait") {
                return status.code();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = child.kill();
        panic!("perfrun did not exit within {within:?}");
    }

    /// A program that leaves the file `ran` in its working folder.
    const MARKER: &str =
        "#include <fcntl.h>\n#include <unistd.h>\nint main(void) { close(open(\"ran\", O_CREAT | O_WRONLY, 0600)); return 0; }\n";

    /// Run the test `name` alone in a new process of this test binary with
    /// `var` set to `dir` (a cancellation is for the whole process): its
    /// stdout, or a panic when it is not done within `within`.
    fn in_own_process(name: &str, var: &str, dir: &Path, within: Duration) -> String {
        let out = dir.join("child-out");
        let mut child = Command::new(std::env::current_exe().expect("exe"))
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(var, dir)
            .stdout(File::create(&out).expect("out"))
            .stderr(Stdio::inherit())
            .spawn()
            .expect("re-exec the test binary");
        let until = Instant::now() + within;
        while child.try_wait().expect("try_wait").is_none() {
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{name} was not done within {within:?}: {}",
                    std::fs::read_to_string(&out).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(&out).expect("the child's stdout")
    }

    /// The baseline is exact (§2, §3.3 step 4): an empty program run
    /// through the launcher counts about what it counts run alone —
    /// sandbox-exec's set-up and perfgo's start, several times an empty
    /// program's instructions, are left out — and work done before an exec
    /// of itself carries on into the count (§3.4).
    #[test]
    fn the_baseline_is_exact() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-baseline");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let empty = program(&dir, "empty", "int main(void) { return 0; }\n");
        let mut through: Vec<u64> = (0..3)
            .map(|_| {
                record(&run(&l, &empty, 60, false))
                    .instructions
                    .expect("counted")
            })
            .collect();
        through.sort_unstable();
        // `/usr/bin/time -l` counts the program run alone, where it can.
        let mut alone: Vec<u64> = (0..3)
            .filter_map(|_| {
                let out = Command::new("/usr/bin/time")
                    .arg("-l")
                    .arg(&empty)
                    .output()
                    .ok()?;
                String::from_utf8_lossy(&out.stderr).lines().find_map(|l| {
                    l.trim()
                        .strip_suffix("instructions retired")?
                        .trim()
                        .parse()
                        .ok()
                })
            })
            .collect();
        alone.sort_unstable();
        if let (Some(&through), Some(&alone)) = (through.get(1), alone.get(alone.len() / 2)) {
            assert!(
                through < alone * 2 && through * 2 > alone,
                "an empty program counts {through} through the launcher and {alone} alone"
            );
        }
        // A program that execs itself after its work: the work is counted.
        let again = program(
            &dir,
            "again",
            "#include <stdio.h>\n#include <string.h>\n#include <unistd.h>\nint main(int c, char **v) {\n\
             if (c > 2 && strcmp(v[1], \"first\") == 0) { volatile unsigned long x = 0;\n\
             for (unsigned long i = 0; i < 20000000UL; i++) x += i;\n\
             char *a[] = { v[0], \"second\", 0 }; execv(v[2], a); return 9; }\n\
             printf(\"%s %s\\n\", v[0], v[1]); return 0; }\n",
        );
        let o = Opts {
            args: vec!["first".into(), again.to_string_lossy().into_owned()],
            ..opts(60, true)
        };
        let m = run_with(&l, &again, &o);
        let r = record(&m);
        assert_eq!(
            (&r.status, r.end),
            (&Status::Ok, Some(End::Exit(0))),
            "{r:?}"
        );
        assert_eq!(m.stdout, b"tool second\n");
        assert!(r.instructions.is_some_and(|i| i > 20_000_000), "{r:?}");
    }

    /// A program the profile does not name never starts: the exec is
    /// refused, read as "never started" with its errno (§3.3 judging step 4).
    #[test]
    fn an_exec_the_profile_denies_never_starts() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-denied");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let named = program(&dir, "named", "int main(void) { return 0; }\n");
        let unnamed = program(&dir, "unnamed", MARKER);
        let o = Opts {
            allowed: Some(&named),
            ..opts(60, true)
        };
        let m = run_with(&l, &unnamed, &o);
        let r = record(&m);
        assert_eq!(r.status, Status::NeverStarted(Some(1)), "{r:?}");
    }

    /// Before the go-ahead (§3.3 step 4): a SIGTERM gives a `stopped`
    /// record within the harness's grace, the child kept unreaped until the
    /// bye (build note 7); the socket's end — the harness gone or cancelled
    /// — ends perfrun within the CLI's 250 ms. Either way the program never
    /// runs.
    #[test]
    fn before_the_go_ahead_the_program_never_runs() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-before-go");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let marker = program(&dir, "marker", MARKER);
        // A SIGTERM.
        let (_run, cwd, profile) = run_dir(&l, &marker);
        let mut h = by_hand(&l, &profile, &marker, &cwd);
        h.wait_for_perfgo();
        exec::terminate(h.perfrun.id());
        let t = Instant::now();
        let r = h.record();
        assert!(t.elapsed() < TERM_GRACE, "{:?}", t.elapsed());
        assert_eq!(
            (&r.status, r.killed, r.end),
            (&Status::Stopped, true, Some(End::Signal(9))),
            "{r:?}"
        );
        // The record says the child ended, so it is a zombie already: the
        // pause gives a perfrun that reaps too early the time to do it (a
        // busy Mac can only hide that, never fail a right perfrun).
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            ps(h.child, "stat").starts_with('Z'),
            "unreaped until the bye"
        );
        h.ours.write_all(b"B").expect("bye");
        assert_eq!(
            exits_within(&mut h.perfrun, Duration::from_secs(5)),
            Some(0)
        );
        assert_eq!(ps(h.child, "stat"), "", "reaped after the bye");
        assert!(!cwd.join("ran").exists(), "the program never ran");
        // The socket's end.
        let (_run, cwd, profile) = run_dir(&l, &marker);
        let h = by_hand(&l, &profile, &marker, &cwd);
        h.wait_for_perfgo();
        let ByHand {
            ours,
            mut perfrun,
            child,
        } = h;
        drop(ours);
        let t = Instant::now();
        assert_eq!(exits_within(&mut perfrun, Duration::from_secs(5)), Some(0));
        assert!(
            t.elapsed() < Duration::from_millis(250),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(ps(child, "stat"), "", "the child is gone");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!cwd.join("ran").exists(), "the program never ran");
    }

    /// After the go-ahead: the child stays unreaped from the record until
    /// the harness's bye (§3.3 step 7), so its pid can never be reused while
    /// the harness may still signal its group; and the socket's end — the
    /// harness killed while the program runs — kills the program (step 5).
    #[test]
    fn after_the_go_ahead_perfrun_holds_the_child_until_the_bye() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-after-go");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let marker = program(&dir, "marker", MARKER);
        let (_run, cwd, profile) = run_dir(&l, &marker);
        let mut h = by_hand(&l, &profile, &marker, &cwd);
        h.ours.write_all(b"G").expect("go");
        let r = h.record();
        assert_eq!(
            (&r.status, r.end),
            (&Status::Ok, Some(End::Exit(0))),
            "{r:?}"
        );
        assert!(cwd.join("ran").exists(), "the program ran");
        // A zombie already, as above: the pause is a wrong perfrun's chance
        // to reap it, not a wait for the child to die.
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            ps(h.child, "stat").starts_with('Z'),
            "unreaped until the bye"
        );
        h.ours.write_all(b"B").expect("bye");
        assert_eq!(
            exits_within(&mut h.perfrun, Duration::from_secs(5)),
            Some(0)
        );
        assert_eq!(ps(h.child, "stat"), "", "reaped after the bye");
        // The harness killed while its program runs.
        let spin = program(
            &dir,
            "spinhand",
            "#include <signal.h>\nint main(void) { signal(SIGTERM, SIG_IGN); for (;;) {} }\n",
        );
        let (_run, cwd, profile) = run_dir(&l, &spin);
        let ByHand {
            mut ours,
            mut perfrun,
            child,
        } = by_hand(&l, &profile, &spin, &cwd);
        ours.write_all(b"G").expect("go");
        assert_eq!(pid_of(&spin), child);
        drop(ours);
        assert_eq!(exits_within(&mut perfrun, Duration::from_secs(5)), Some(0));
        assert!(
            dead_within(child, Duration::from_secs(5)),
            "the program died with the harness"
        );
    }

    /// Run by [`a_cancel_before_the_go_ahead_never_starts_the_program`] in
    /// its own process; a no-op otherwise.
    #[test]
    fn cancel_before_go_child_body() {
        let Some(dir) = std::env::var_os("RUHARNESS_PERF_CANCEL_BEFORE_GO") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let (tmp, l) = test_launcher("perf-cancel-before-go");
        let marker = program(&dir, "marker", MARKER);
        let (run_tmp, cwd, profile) = run_dir(&l, &marker);
        let cancelled_at = std::rc::Rc::new(std::cell::Cell::new(None));
        {
            let cancelled_at = std::rc::Rc::clone(&cancelled_at);
            hooks::set(move |stage, _| {
                if stage == hooks::Stage::ChildLine {
                    cancelled_at.set(Some(Instant::now()));
                    exec::kill_live_process_groups();
                }
            });
        }
        let result = run_measured(&RunSpec {
            launcher: &l,
            profile: &profile,
            deadline_secs: 60,
            allowance: Duration::from_secs(60),
            program: &marker,
            name: "tool",
            args: &[],
            cwd: &cwd,
            tmpdir: run_tmp.path(),
            capture: false,
            max_output: 1024,
        });
        let took = cancelled_at.get().expect("the hook ran").elapsed();
        assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
        assert!(took < Duration::from_millis(250), "answered in {took:?}");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!cwd.join("ran").exists(), "the program never ran");
        drop((run_tmp, l, tmp));
        println!("cancel-before-go-ok");
        std::process::exit(0);
    }

    /// A cancel between perfrun's child line and the go-ahead leaves no
    /// program run, answered within the CLI's 250 ms (§3.3 *The harness
    /// side* step 3). A missing check would hang in the registry's lock,
    /// which a cancel keeps for good.
    #[test]
    fn a_cancel_before_the_go_ahead_never_starts_the_program() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-cancel-before-go-parent");
        let out = in_own_process(
            "perf::launcher::tests::cancel_before_go_child_body",
            "RUHARNESS_PERF_CANCEL_BEFORE_GO",
            tmp.path(),
            Duration::from_secs(120),
        );
        assert!(out.contains("cancel-before-go-ok"), "{out}");
    }

    /// Run by [`a_cancel_at_the_bye_never_signals_the_program`] in its own
    /// process; a no-op otherwise.
    #[test]
    fn cancel_at_bye_child_body() {
        let Some(dir) = std::env::var_os("RUHARNESS_PERF_CANCEL_AT_BYE") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let (tmp, l) = test_launcher("perf-cancel-at-bye");
        // The program leaves a sleeping child in its own group (this test's
        // profile allows the fork), says its pid, and exits.
        let leaver = program(
            &dir,
            "leaver",
            "#include <stdio.h>\n#include <unistd.h>\nint main(void) { pid_t p = fork();\n\
             if (p == 0) { sleep(30); return 0; }\n\
             FILE *f = fopen(\"grandchild\", \"w\"); fprintf(f, \"%d\\n\", (int)p); fclose(f); return 0; }\n",
        );
        let run_tmp = crate::testutil::TempDir::new("perf-run");
        let cwd = run_tmp.path().join("run");
        std::fs::create_dir(&cwd).expect("cwd");
        hooks::set(|stage, _| {
            if stage == hooks::Stage::Bye {
                exec::kill_live_process_groups();
            }
        });
        let result = run_measured(&RunSpec {
            launcher: &l,
            profile: "(version 1)(allow default)",
            deadline_secs: 60,
            allowance: Duration::from_secs(60),
            program: &leaver,
            name: "tool",
            args: &[],
            cwd: &cwd,
            tmpdir: run_tmp.path(),
            capture: false,
            max_output: 1024,
        });
        assert!(matches!(result, Err(Error::Interrupted)), "{result:?}");
        let grandchild: u32 = std::fs::read_to_string(cwd.join("grandchild"))
            .expect("the grandchild's pid")
            .trim()
            .parse()
            .expect("a pid");
        std::thread::sleep(Duration::from_millis(100));
        let spared = alive(grandchild);
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &grandchild.to_string()])
            .status();
        assert!(spared, "a cancel at the bye signalled the program's group");
        drop((run_tmp, l, tmp));
        println!("cancel-at-bye-ok");
        std::process::exit(0);
    }

    /// The program's group is unregistered before the bye (§3.3 *The
    /// harness side* step 4): a cancel just before it never signals the
    /// group, whose pid perfrun frees once it has the bye. A child the
    /// program left in its group (a profile that allows the fork) shows
    /// whether a signal came.
    #[test]
    fn a_cancel_at_the_bye_never_signals_the_program() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tmp = crate::testutil::TempDir::new("perf-cancel-at-bye-parent");
        let out = in_own_process(
            "perf::launcher::tests::cancel_at_bye_child_body",
            "RUHARNESS_PERF_CANCEL_AT_BYE",
            tmp.path(),
            Duration::from_secs(120),
        );
        assert!(out.contains("cancel-at-bye-ok"), "{out}");
    }

    /// Output over the cap ends the run at once (§3.3 *The harness side*
    /// step 6): the program — which ignores SIGTERM and SIGPIPE — is dead,
    /// and the answer never waits on the 2-second drain bound.
    #[test]
    fn an_overflow_ends_the_run_at_once() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-overflow");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let printer = program(
            &dir,
            "printer",
            "#include <signal.h>\n#include <unistd.h>\nint main(void) { signal(SIGTERM, SIG_IGN);\n\
             signal(SIGPIPE, SIG_IGN); char b[4096] = {0}; for (;;) { if (write(1, b, sizeof b) < 0) {} } }\n",
        );
        let o = Opts {
            max_output: 64 * 1024,
            ..opts(60, true)
        };
        // Timed the second time: a new binary's first exec can take
        // seconds on a busy Mac.
        for timed in [false, true] {
            let t = Instant::now();
            let (m, group) = run_with_group(&l, &printer, &o);
            assert!(matches!(m.seen, Seen::Overflow), "{:?}", m.seen);
            assert!(
                !timed || t.elapsed() < Duration::from_secs(2),
                "{:?}",
                t.elapsed()
            );
            let left = left_running(
                group.expect("the child line"),
                &printer,
                Duration::from_secs(5),
            );
            assert!(left.is_empty(), "the program is dead: {left:?}");
        }
    }

    /// A stream that reaches the cap only after perfrun's record was read
    /// in full is still output over the cap (§3.3 "Judging a run", step 1).
    #[test]
    fn an_overflow_seen_after_the_record_is_still_an_overflow() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-late-overflow");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let bin = program(
            &dir,
            "overprint",
            "#include <string.h>\n#include <unistd.h>\nint main(void) { char b[1100];\n\
             memset(b, 'x', sizeof b); if (write(1, b, sizeof b) < 0) return 1; return 0; }\n",
        );
        hooks::set_hold_stdout(true);
        let m = run_with(
            &l,
            &bin,
            &Opts {
                max_output: 1000,
                ..opts(60, true)
            },
        );
        hooks::set_hold_stdout(false);
        assert!(matches!(m.seen, Seen::Overflow), "{:?}", m.seen);
    }

    /// A child that ends before it is ready — here sandbox-exec refusing a
    /// broken profile at once, possibly before its exit watch is even added
    /// — reads `never-started no-ready` with the child's own end.
    #[test]
    fn a_child_that_ends_before_it_is_ready_never_started() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-early-end");
        let dir = tmp.path().join("progs");
        std::fs::create_dir(&dir).expect("dir");
        let bin = program(&dir, "early", "int main(void) { return 0; }\n");
        let m = run_with(
            &l,
            &bin,
            &Opts {
                profile: Some("("),
                ..opts(60, true)
            },
        );
        let r = record(&m);
        assert_eq!(
            (&r.status, r.end, r.killed),
            (&Status::NeverStarted(None), Some(End::Exit(65)), false),
            "{r:?}"
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

    /// Stale version folders go, but never one a perf run holds, and the
    /// newest other one stays: two worktrees in turn do not rebuild each
    /// other's launcher (build note 4).
    #[test]
    fn stale_launchers_go_but_never_one_in_use() {
        let tmp = crate::testutil::TempDir::new("perf-stale");
        let root = tmp.path();
        let current = root.join("perf-launcher-2-cccc");
        // Oldest first.
        let others = [
            "perf-launcher-1-aaaa",
            "perf-launcher-1-nolock",
            "perf-launcher-1-bbbb",
            "perf-launcher-1-dddd",
        ];
        for name in others.iter().chain(&["perf-launcher-2-cccc"]) {
            let d = root.join(name);
            std::fs::create_dir(&d).expect("folder");
            std::fs::write(d.join(HASHES_FILE), "perfrun x\nperfgo y\n").expect("hashes");
            if !name.ends_with("nolock") {
                std::fs::write(d.join(".lock"), "").expect("lock");
            }
        }
        std::fs::create_dir(root.join("other")).expect("not a launcher");
        let start = std::time::SystemTime::now() - Duration::from_secs(3600);
        for (i, name) in others.iter().enumerate() {
            File::open(root.join(name))
                .expect("open")
                .set_modified(start + Duration::from_secs(60 * i as u64))
                .expect("mtime");
        }
        let held = File::open(root.join("perf-launcher-1-aaaa/.lock")).expect("open");
        held.lock_shared().expect("a run holds it");
        remove_stale(root, &current);
        let left = |name: &str| root.join(name).exists();
        for kept in [
            "perf-launcher-2-cccc",
            "perf-launcher-1-dddd",
            "perf-launcher-1-aaaa",
            "other",
        ] {
            assert!(left(kept), "{kept} is kept");
        }
        for gone in ["perf-launcher-1-bbbb", "perf-launcher-1-nolock"] {
            assert!(!left(gone), "{gone} is removed");
        }
        drop(held);
        remove_stale(root, &current);
        assert!(!left("perf-launcher-1-aaaa"), "removed once free");
        assert!(left("perf-launcher-1-dddd") && left("perf-launcher-2-cccc"));
    }

    /// A version folder a perf run holds is never rebuilt under it — perf
    /// says so instead — and once free it is; stale folders go only then,
    /// under perf's lock, keeping the newest other and any held one.
    #[test]
    fn a_launcher_in_use_is_never_rebuilt() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (tmp, l) = test_launcher("perf-in-use");
        let owner = std::fs::metadata(tmp.path()).expect("meta").uid();
        let version = l.perfrun.path.parent().expect("version").to_path_buf();
        let root = version.parent().expect("root").to_path_buf();
        std::fs::write(version.join(HASHES_FILE), "junk\n").expect("needs rebuilding");
        let err = launcher_in(&root, owner, &mut |_: &str| {}).expect_err("refused");
        assert!(
            err.to_string().contains("in use by another perf run"),
            "{err}"
        );
        assert!(l.perfgo.path.exists(), "the folder in use is kept");
        // Other versions' folders: the newest kept, a held one kept.
        let start = std::time::SystemTime::now() - Duration::from_secs(3600);
        for (i, name) in [
            "perf-launcher-0-held",
            "perf-launcher-0-old",
            "perf-launcher-0-new",
        ]
        .iter()
        .enumerate()
        {
            let d = root.join(name);
            std::fs::create_dir(&d).expect("folder");
            std::fs::write(d.join(".lock"), "").expect("lock");
            File::open(&d)
                .expect("open")
                .set_modified(start + Duration::from_secs(60 * i as u64))
                .expect("mtime");
        }
        let held = File::open(root.join("perf-launcher-0-held/.lock")).expect("open");
        held.lock_shared().expect("a run holds it");
        drop(l);
        let mut said = Vec::new();
        let again = launcher_in(&root, owner, &mut |w: &str| said.push(w.to_string()))
            .expect("rebuilt once free");
        assert!(
            said.iter().any(|w| w == "building the launcher…"),
            "{said:?}"
        );
        assert!(again.check().is_ok());
        assert!(root.join("perf-launcher-0-held").exists());
        assert!(root.join("perf-launcher-0-new").exists());
        assert!(!root.join("perf-launcher-0-old").exists());
    }

    /// perf's lock is never held through a link (build note 4).
    #[test]
    fn a_lock_that_is_a_link_is_refused() {
        let tmp = crate::testutil::TempDir::new("perf-lock-link");
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::write(&elsewhere, "").expect("file");
        let lock = tmp.path().join(".lock");
        std::os::unix::fs::symlink(&elsewhere, &lock).expect("link");
        let err = lock_file(&lock, true).expect_err("refused");
        assert!(err.to_string().contains("perf's lock is a link"), "{err}");
    }

    /// A compiler in a developer folder the person owns is refused with its
    /// words (§3.2 step 1); when the selected folder fails and the Command
    /// Line Tools pass, they are used and the progress line says so (build
    /// note 9).
    #[test]
    fn a_compiler_the_person_owns_is_refused_with_its_words() {
        let tmp = crate::testutil::TempDir::new("perf-own-xcode");
        let dev = tmp.path().join("Developer");
        for file in [
            "usr/bin/clang",
            "usr/bin/ld",
            "usr/lib/libLTO.dylib",
            "SDKs/MacOSX.sdk/SDKSettings.json",
        ] {
            let path = dev.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(&path, "").expect("file");
        }
        let err = find_compiler_in(std::slice::from_ref(&dev)).expect_err("refused");
        assert_eq!(
            err.to_string(),
            Error::Invariant(format!(
                "your compiler at {} is not owned by the system — install Xcode or the \
                 Command Line Tools with Apple's installer",
                dev.join("usr/bin/clang").display()
            ))
            .to_string()
        );
        let clt = PathBuf::from(COMMAND_LINE_TOOLS);
        let clt_passes =
            layout(&clt).is_some_and(|paths| paths.iter().all(|p| root_owned(p).is_ok()));
        if cfg!(target_os = "macos") && clt_passes {
            let c = find_compiler_in(&[dev, clt]).expect("the Command Line Tools");
            assert_eq!(
                c.note.as_deref(),
                Some(
                    "the selected developer folder is not owned by the system — using the \
                     Command Line Tools at /Library/Developer/CommandLineTools"
                )
            );
        }
    }
}
