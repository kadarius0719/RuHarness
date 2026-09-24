//! The `capabilities` check (docs/M4-DESIGN.md §R2 "Capability parity"): a
//! translated pure function has no business reading files, the environment
//! or the clock — and if it could, it could read a held-out vector (or the
//! build dir's `drv_c.out`) at verify or score time and replay it.
//!
//! The check looks at the undefined symbols of the archive members that
//! belong to the candidate crate ITSELF (member names start with the crate
//! name followed by cargo's `-<hash>`; std's own members legitimately
//! reference everything and are not the candidate's choice). It rejects:
//!
//! - std APIs of the fs / env / process / net / os / thread / time modules
//!   (legacy mangling `_ZN3std2fs…` & co., v0 `…_3std2fs…`), except the
//!   thread-LOCAL storage support (`std::thread::local`, which `thread_local!`
//!   needs for C global state and which grants no capability);
//! - the libc entry points of the same capabilities plus `dl*`/`syscall`;
//!
//! UNLESS the C unit itself uses that capability class (derived from its
//! unresolved call refs in facts: a unit calling `fopen` may translate to
//! `std::fs`). It also rejects `asm!`/`global_asm!`/`naked_asm!` anywhere in
//! the candidate's `src/` (the task names `ffi.rs`; every file is scanned,
//! since `global_asm!` needs no `unsafe` and could hide in `logic.rs`).

use crate::exec::Runner;
use crate::symbols::normalize;
use harness_core::error::Error;
use harness_core::facts::Facts;
use harness_core::verdict::Check;
use harness_core::Unit;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Name of the check in verdicts.
pub(crate) const CHECK_NAME: &str = "capabilities";

/// How many offending names a failure detail lists.
const DETAIL_LIMIT: usize = 10;

/// A capability class: the std module it maps to (legacy-mangled path
/// component, `None` = no std module) and its libc entry points (names as C
/// sees them; `$…` variant suffixes are stripped before comparing).
struct Class {
    name: &'static str,
    std_module: Option<&'static str>,
    libc: &'static [&'static str],
}

/// The capability classes a candidate may only reach when its C unit does.
const CLASSES: &[Class] = &[
    Class {
        name: "fs",
        std_module: Some("2fs"),
        libc: &[
            "open",
            "openat",
            "open64",
            "openat64",
            "__open_2",
            "__openat_2",
            "creat",
            "fopen",
            "fopen64",
            "freopen",
            "freopen64",
            "fdopen",
            "stat",
            "lstat",
            "fstat",
            "stat64",
            "lstat64",
            "fstat64",
            "fstatat",
            "fstatat64",
            "statx",
            "__xstat",
            "__lxstat",
            "__fxstat",
            "__xstat64",
            "__lxstat64",
            "__fxstat64",
            "opendir",
            "fdopendir",
            "readdir",
            "readdir64",
            "readdir_r",
            "access",
            "faccessat",
            "realpath",
            "readlink",
            "readlinkat",
            "getcwd",
            "chdir",
            "mkdir",
            "unlink",
            "rename",
            "truncate",
        ],
    },
    Class {
        name: "env",
        std_module: Some("3env"),
        libc: &[
            "getenv",
            "secure_getenv",
            "setenv",
            "unsetenv",
            "putenv",
            "environ",
            "_NSGetEnviron",
            "_NSGetArgv",
            "_NSGetArgc",
        ],
    },
    Class {
        name: "process",
        std_module: Some("7process"),
        libc: &[
            "fork",
            "vfork",
            "clone",
            "execve",
            "execv",
            "execvp",
            "execvpe",
            "execl",
            "execlp",
            "execle",
            "fexecve",
            "posix_spawn",
            "posix_spawnp",
            "system",
            "popen",
            // Process control: at score time the candidate runs inside the
            // corpus runner's process tree, so signalling or tracing it (or
            // its parent) must be as unreachable as spawning.
            "kill",
            "killpg",
            "raise",
            "ptrace",
            "getppid",
            "pthread_kill",
            "_exit",
            "_Exit",
        ],
    },
    Class {
        name: "net",
        std_module: Some("3net"),
        libc: &[
            "socket",
            "connect",
            "bind",
            "listen",
            "accept",
            "getaddrinfo",
            "gethostbyname",
        ],
    },
    Class {
        name: "os",
        std_module: Some("2os"),
        libc: &[],
    },
    Class {
        name: "thread",
        std_module: Some("6thread"),
        libc: &["pthread_create"],
    },
    Class {
        name: "time",
        std_module: Some("4time"),
        libc: &[
            "time",
            "clock",
            "clock_gettime",
            "clock_gettime_nsec_np",
            "gettimeofday",
            "mach_absolute_time",
            "mach_continuous_time",
        ],
    },
    Class {
        name: "dl",
        std_module: None,
        libc: &["dlopen", "dlsym", "dlvsym"],
    },
    Class {
        name: "syscall",
        std_module: None,
        libc: &["syscall"],
    },
    // Fault interception (design B, docs/ORACLE-HARDENING.md §B.8): signal
    // dispositions, Mach exception ports and the raw messaging/thread
    // primitives that reach them. Split out of `process` so a boundary-checked
    // unit's candidate can be refused this class whatever its C uses.
    Class {
        name: "signal",
        std_module: None,
        libc: &[
            "sigaction",
            "__sigaction",
            "signal",
            "bsd_signal",
            "sigvec",
            "sigset",
            "sigaltstack",
            "sigprocmask",
            "pthread_sigmask",
            "task_set_exception_ports",
            "thread_set_exception_ports",
            "task_swap_exception_ports",
            "thread_swap_exception_ports",
            "mach_msg",
            "mach_msg_overwrite",
            "mach_msg2",
            "mach_msg_trap",
            "mach_msg2_trap",
            "mach_port_allocate",
            "mach_port_insert_right",
            "mach_port_construct",
            "thread_create",
            "thread_create_running",
            "thread_set_state",
            "thread_resume",
        ],
    },
    // Mapping and protecting memory (design B, docs/ORACLE-HARDENING.md
    // §B.8): the boundary check's guard pages prove nothing if the candidate
    // can re-protect them. Its own class, not `os`, which fs/process/net
    // admit implicitly.
    Class {
        name: "mem",
        std_module: None,
        libc: &[
            "mmap",
            "munmap",
            "mprotect",
            "madvise",
            "minherit",
            "mremap",
            "mlock",
            "munlock",
            "mach_vm_protect",
            "mach_vm_allocate",
            "mach_vm_deallocate",
            "mach_vm_map",
            "mach_vm_remap",
            "vm_protect",
            "vm_allocate",
            "vm_deallocate",
            "vm_map",
            "vm_remap",
            "mach_vm_remap_new",
            "vm_remap_new",
            "vm_copy",
            "mach_make_memory_entry",
            "mach_make_memory_entry_64",
            "mach_task_self",
            "mach_task_self_",
            "task_for_pid",
        ],
    },
];

/// `std::thread` sub-module that is storage, not a capability.
const THREAD_LOCAL_MODULE: &str = "6thread5local";

/// The capability classes the C unit itself uses: its files' unresolved
/// call refs mapped through [`CLASSES`]. Using any of fs/process/net also
/// admits `os` (the `std::os::unix::…` extension traits those need). A unit
/// opted into the boundary check (`[unit.oracle] boundary = true`) is never
/// granted `mem` or `signal`, whatever its C uses: the guard pages and the
/// runtime's fault handler are the check (docs/ORACLE-HARDENING.md §B.8).
pub(crate) fn unit_classes(facts: &Facts, unit: &Unit) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for r in facts
        .refs
        .iter()
        .filter(|r| !r.resolved && unit.files.contains(&r.file))
    {
        if let Some(class) = libc_class(&r.to) {
            out.insert(class);
        }
    }
    if ["fs", "process", "net"].iter().any(|c| out.contains(c)) {
        out.insert("os");
    }
    if unit.oracle_param_bool("boundary") == Some(true) {
        out.remove("mem");
        out.remove("signal");
    }
    out
}

/// The `_kernelrpc_*` Mach traps and the `*_trap` entry points reach the
/// kernel without the named stubs: they are `signal`-class for every candidate.
fn is_mach_trap(name: &str) -> bool {
    name.starts_with("_kernelrpc_") || (name.starts_with("mach_") && name.ends_with("_trap"))
}

/// The boundary runtime's own entry points and the coverage callbacks
/// (docs/ORACLE-HARDENING.md §B.8): no candidate may reference them.
fn is_runtime_name(name: &str) -> bool {
    name.starts_with("ruharness_") || name.starts_with("__sanitizer_cov_")
}

/// The class of a libc name (after stripping a `$…` variant suffix).
fn libc_class(name: &str) -> Option<&'static str> {
    let base = name.split('$').next().unwrap_or(name);
    CLASSES
        .iter()
        .find(|c| c.libc.contains(&base))
        .map(|c| c.name)
}

/// The class of a Rust-mangled std path, if it is a capability module.
fn std_class(normalized: &str) -> Option<&'static str> {
    let legacy = normalized.strip_prefix("_ZN3std");
    for class in CLASSES {
        let Some(module) = class.std_module else {
            continue;
        };
        let hit = match legacy {
            Some(rest) => {
                rest.starts_with(module)
                    && !(class.name == "thread" && rest.starts_with(THREAD_LOCAL_MODULE))
            }
            None => {
                normalized.starts_with("_R")
                    && normalized.contains(&format!("_3std{module}"))
                    && !(class.name == "thread"
                        && normalized.contains(&format!("_3std{THREAD_LOCAL_MODULE}")))
            }
        };
        if hit {
            return Some(class.name);
        }
    }
    None
}

/// `(archive member, undefined name)` pairs from `nm -u` of a staticlib.
pub(crate) fn undefined_by_member(nm_u_output: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut member = String::new();
    for line in nm_u_output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(header) = trimmed.strip_suffix(':') {
            // `member.o:` (macOS, GNU) or `lib.a(member.o):` (some nm).
            member = match (header.rfind('('), header.strip_suffix(')')) {
                (Some(open), Some(inner)) => inner[open + 1..].to_string(),
                _ => header.to_string(),
            };
            continue;
        }
        if let Some(name) = trimmed.split_whitespace().last() {
            out.push((member.clone(), name.to_string()));
        }
    }
    out
}

/// The `asm!`/`global_asm!`/`naked_asm!` macro uses in Rust source: an
/// identifier token (raw `r#` prefix allowed) followed by `!`, outside
/// comments, strings and char literals. Returns the offending macro names.
pub(crate) fn asm_macros(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            // Rust block comments nest.
            let mut depth = 0usize;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b'\'' {
            // A char literal ('x', '\n', '\u{..}') — or a lifetime ('a).
            if b.get(i + 2) == Some(&b'\'') {
                i += 3;
                continue;
            }
            if b.get(i + 1) == Some(&b'\\') {
                i += 2;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let mut ident = &src[start..i];
            // Raw strings: r"…", r#"…"#, br#"…"#.
            if matches!(ident, "r" | "br") && matches!(b.get(i), Some(b'"') | Some(b'#')) {
                let mut hashes = 0;
                let mut j = i;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) == Some(&b'"') {
                    let close: Vec<u8> = std::iter::once(b'"')
                        .chain(std::iter::repeat_n(b'#', hashes))
                        .collect();
                    j += 1;
                    while j < b.len() && !b[j..].starts_with(&close) {
                        j += 1;
                    }
                    i = (j + close.len()).min(b.len());
                    continue;
                }
                if ident == "r" && hashes == 1 {
                    // Raw identifier `r#asm`.
                    let s2 = j;
                    let mut k = j;
                    while k < b.len() && (b[k].is_ascii_alphanumeric() || b[k] == b'_') {
                        k += 1;
                    }
                    ident = &src[s2..k];
                    i = k;
                }
            }
            if matches!(ident, "asm" | "global_asm" | "naked_asm") {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                // `asm !=` is a comparison, not a macro call.
                if b.get(j) == Some(&b'!') && b.get(j + 1) != Some(&b'=') {
                    out.push(format!("{ident}!"));
                } else {
                    // Any other mention names the item — e.g. a renaming
                    // import `use core::arch::asm as z;` then `z!(…)`, which
                    // the macro-call form alone would miss (M4 review).
                    out.push(format!("{ident} (named outside a macro call)"));
                }
            }
            if ident == "arch" {
                // `core::arch` / `std::arch` (or any `…::arch` path): the
                // inline-assembly and intrinsics module, reachable under a
                // renamed import.
                let mut k = start;
                while k > 0 && b[k - 1].is_ascii_whitespace() {
                    k -= 1;
                }
                if k >= 2 && &b[k - 2..k] == b"::" {
                    out.push("`::arch` path".to_string());
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Run the check for the candidate staticlib `staticlib` (built from
/// `crate_dir`), allowing the capability classes the C unit uses.
pub(crate) fn capabilities_check(
    runner: &Runner,
    staticlib: &Path,
    crate_dir: &Path,
    allowed: &BTreeSet<&'static str>,
) -> Result<Check, Error> {
    let crate_name = staticlib
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("lib"))
        .ok_or_else(|| {
            Error::Invariant(format!(
                "cannot derive the crate name from {}",
                staticlib.display()
            ))
        })?
        .to_string();
    let lib_str = staticlib
        .to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", staticlib.display())))?;
    let out = runner.tool(&["nm".to_string(), "-u".to_string(), lib_str.to_string()])?;
    let pairs = undefined_by_member(&String::from_utf8_lossy(&out));
    let own_prefix = format!("{crate_name}-");
    let macos = cfg!(target_os = "macos");

    let mut own_members = BTreeSet::new();
    let mut offending: BTreeMap<String, &'static str> = BTreeMap::new();
    for (member, raw) in &pairs {
        if !member.starts_with(&own_prefix) {
            continue;
        }
        own_members.insert(member.clone());
        let name = normalize(raw, macos);
        if is_runtime_name(name) {
            offending.insert(name.to_string(), "boundary-runtime");
            continue;
        }
        let class = std_class(name)
            .or_else(|| libc_class(name))
            .or_else(|| is_mach_trap(name).then_some("signal"));
        if let Some(class) = class {
            if !allowed.contains(class) {
                offending.insert(name.to_string(), class);
            }
        }
    }

    let mut asm: Vec<String> = Vec::new();
    rust_sources(&crate_dir.join("src"), &mut |rel, text| {
        for m in asm_macros(text) {
            asm.push(format!("src/{rel} uses `{m}`"));
        }
    })?;

    let mut problems: Vec<String> = Vec::new();
    if own_members.is_empty() {
        problems.push(format!(
            "no archive member of crate `{crate_name}` found in the staticlib — cannot attribute \
             its references"
        ));
    }
    if !offending.is_empty() {
        let total = offending.len();
        let mut listed: Vec<String> = offending
            .iter()
            .take(DETAIL_LIMIT)
            .map(|(name, class)| format!("{name} ({class})"))
            .collect();
        if total > DETAIL_LIMIT {
            listed.push(format!("+{} more", total - DETAIL_LIMIT));
        }
        problems.push(format!(
            "the candidate references capabilities its C unit does not use: {}",
            listed.join(", ")
        ));
    }
    problems.extend(asm);

    let allowed_note = if allowed.is_empty() {
        "none".to_string()
    } else {
        allowed.iter().copied().collect::<Vec<_>>().join(", ")
    };
    Ok(if problems.is_empty() {
        Check {
            name: CHECK_NAME.into(),
            passed: true,
            detail: format!("no capability beyond the C unit's (allowed: {allowed_note}); no asm"),
        }
    } else {
        Check {
            name: CHECK_NAME.into(),
            passed: false,
            detail: problems.join("; "),
        }
    })
}

/// Call `visit(relative path, text)` for every `.rs` file under `dir`,
/// sorted, following no symlink out of `dir`.
fn rust_sources(dir: &Path, visit: &mut dyn FnMut(&str, &str)) -> Result<(), Error> {
    let root = dir.canonicalize().map_err(|e| Error::io(dir, e))?;
    let mut stack = vec![root.clone()];
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| Error::io(&d, e))? {
            let path = entry.map_err(|e| Error::io(&d, e))?.path();
            let Ok(canon) = path.canonicalize() else {
                continue;
            };
            if !canon.starts_with(&root) {
                continue;
            }
            if canon.is_dir() {
                if canon != d {
                    stack.push(canon);
                }
            } else if canon.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(canon);
            }
        }
    }
    files.sort();
    files.dedup();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|e| Error::io(&file, e))?;
        let rel = file
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        visit(&rel, &text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Regression (M4 security review): a renaming import evaded the
    /// literal `asm!` scan and compiled under the candidate scaffolding.
    #[test]
    fn renamed_inline_assembly_is_caught() {
        let src = "use core::arch::asm as z;\n\
                   #[no_mangle] pub extern \"C\" fn probe() -> u64 {\n\
                   let mut out: u64 = 0; unsafe { z!(\"mov {0}, 7\", out(reg) out); } out }\n";
        let hits = asm_macros(src);
        assert!(hits.iter().any(|h| h.contains("::arch")), "{hits:?}");
        assert!(hits.iter().any(|h| h.starts_with("asm ")), "{hits:?}");
        let glob = "use std::arch::*;\n";
        assert!(!asm_macros(glob).is_empty());
        // Innocent code stays clean (comments and strings are skipped).
        assert!(asm_macros("// core::arch::asm\nlet s = \"asm!\"; let arch_x = 1;\n").is_empty());
    }

    use super::*;
    use crate::testutil::{TempDir, ToolBench};

    #[test]
    fn std_and_libc_classes() {
        assert_eq!(
            std_class("_ZN3std2fs4read5inner17h0123456789abcdefE"),
            Some("fs")
        );
        assert_eq!(
            std_class("_ZN3std3env7_var_os17h0123456789abcdefE"),
            Some("env")
        );
        assert_eq!(
            std_class("_ZN3std4time7Instant3now17h0123456789abcdefE"),
            Some("time")
        );
        assert_eq!(
            std_class("_ZN3std7process4exit17h0123456789abcdefE"),
            Some("process")
        );
        assert_eq!(
            std_class("_ZN3std6thread5sleep17h0123456789abcdefE"),
            Some("thread")
        );
        assert_eq!(std_class("_RNvNtCs1234_3std2fs4read"), Some("fs"));
        // Thread-local storage support is not a capability.
        assert_eq!(
            std_class("_ZN3std6thread5local18panic_access_error17h0123456789abcdefE"),
            None
        );
        assert_eq!(
            std_class("_ZN3std3sys3pal4unix4sync5mutex5Mutex4lock17h0123456789abcdefE"),
            None
        );
        assert_eq!(
            std_class("_ZN4core9panicking5panic17h0123456789abcdefE"),
            None
        );
        assert_eq!(libc_class("fopen"), Some("fs"));
        assert_eq!(libc_class("open$NOCANCEL"), Some("fs"));
        assert_eq!(libc_class("mach_absolute_time"), Some("time"));
        assert_eq!(libc_class("dlsym"), Some("dl"));
        assert_eq!(libc_class("malloc"), None);
        assert_eq!(libc_class("close"), None);
    }

    #[test]
    fn unit_classes_come_from_its_own_unresolved_calls() {
        let facts: Facts = Facts {
            frontend: "t".into(),
            refs: vec![
                harness_core::facts::RefRecord {
                    from: "f".into(),
                    file: "src/u.c".into(),
                    to: "fopen".into(),
                    refkind: "call".into(),
                    resolved: false,
                },
                harness_core::facts::RefRecord {
                    from: "g".into(),
                    file: "src/other.c".into(),
                    to: "time".into(),
                    refkind: "call".into(),
                    resolved: false,
                },
                harness_core::facts::RefRecord {
                    from: "f".into(),
                    file: "src/u.c".into(),
                    to: "malloc".into(),
                    refkind: "call".into(),
                    resolved: false,
                },
            ],
            ..Facts::default()
        };
        let unit: Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"src/u.c\"]\nsymbols = [\"f\"]\n",
        )
        .expect("unit");
        let classes = unit_classes(&facts, &unit);
        assert_eq!(classes.into_iter().collect::<Vec<_>>(), vec!["fs", "os"]);
    }

    #[test]
    fn nm_u_members_are_attributed() {
        let out = "\nfs_rs-f863.fs_rs.c17c-cgu.0.rcgu.o:\n__ZN3std2fs4read5inner17hebd695181b114834E\n_close\n\n\
                   std-8676.std.aa3e-cgu.0.rcgu.o:\n                 U _open\n\nlibx.a(y.o):\n_z\n";
        let pairs = undefined_by_member(out);
        assert_eq!(
            pairs,
            vec![
                (
                    "fs_rs-f863.fs_rs.c17c-cgu.0.rcgu.o".to_string(),
                    "__ZN3std2fs4read5inner17hebd695181b114834E".to_string()
                ),
                (
                    "fs_rs-f863.fs_rs.c17c-cgu.0.rcgu.o".to_string(),
                    "_close".to_string()
                ),
                (
                    "std-8676.std.aa3e-cgu.0.rcgu.o".to_string(),
                    "_open".to_string()
                ),
                ("y.o".to_string(), "_z".to_string()),
            ]
        );
    }

    #[test]
    fn asm_macro_scan_ignores_comments_strings_and_lookalikes() {
        let src = "// asm!(\"x\")\n/* global_asm! /* nested */ asm! */\nlet s = \"asm!\";\n\
                   let r = r#\"naked_asm!\"#;\nlet c = '!';\nfn asm_helper() {}\n\
                   fn f<'a>(x: &'a u8) {}\n";
        assert!(asm_macros(src).is_empty(), "{:?}", asm_macros(src));
        // Stricter since the M4 security review: ANY bare `asm`-family
        // identifier is rejected (a renaming import is indistinguishable from
        // a variable at the token level), even `let asm = 1; asm != 2`.
        assert_eq!(asm_macros("let asm = 1; let y = asm != 2;\n").len(), 2);
        assert_eq!(
            asm_macros("core::arch::asm!(\"nop\");"),
            vec!["`::arch` path", "asm!"]
        );
        assert_eq!(asm_macros("global_asm ! (\"\");"), vec!["global_asm!"]);
        assert_eq!(asm_macros("r#naked_asm!(\"\")"), vec!["naked_asm!"]);
    }

    /// The executor layout (harness-owned lib.rs, model-written logic.rs +
    /// ffi.rs) with `logic` as given.
    fn candidate(bench: &ToolBench, name: &str, logic: &str, ffi: &str) -> std::path::PathBuf {
        let dir = bench.root().join("fixtures").join(name);
        std::fs::create_dir_all(dir.join("src")).expect("dirs");
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n\
                 [profile.release]\npanic = \"abort\"\n"
            ),
        )
        .expect("manifest");
        std::fs::write(
            dir.join("src/lib.rs"),
            "#![deny(unsafe_code)]\n#[forbid(unsafe_code)]\nmod logic;\n#[allow(unsafe_code)]\nmod ffi;\n",
        )
        .expect("lib.rs");
        std::fs::write(dir.join("src/logic.rs"), logic).expect("logic.rs");
        std::fs::write(dir.join("src/ffi.rs"), ffi).expect("ffi.rs");
        dir.canonicalize().expect("canonical")
    }

    const FFI: &str = "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    crate::logic::add(a, b)\n}\n";

    /// Real toolchain: a candidate that reads a file in `logic.rs` is red; a
    /// pure one (thread-local state included) is green; the same file read is
    /// allowed when the C unit itself uses `fs`.
    #[test]
    fn synthetic_candidates_through_the_real_check() {
        let bench = ToolBench::new("caps");
        let none = BTreeSet::new();

        let clean = candidate(
            &bench,
            "clean_rs",
            "use std::cell::RefCell;\nthread_local! { static SEEN: RefCell<Vec<i32>> = RefCell::new(Vec::new()); }\n\
             pub fn add(a: i32, b: i32) -> i32 {\n    SEEN.with(|s| s.borrow_mut().push(a));\n    a.wrapping_add(b)\n}\n",
            FFI,
        );
        let lib = bench.build(&clean);
        let check = capabilities_check(bench.runner(), &lib, &clean, &none).expect("runs");
        assert!(check.passed, "{}", check.detail);

        let reads = candidate(
            &bench,
            "reads_rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    let n = std::fs::read(\"migration/build/u/drv_c.out\").map(|v| v.len()).unwrap_or(0) as i32;\n    a.wrapping_add(b).wrapping_add(n)\n}\n",
            FFI,
        );
        let lib = bench.build(&reads);
        let check = capabilities_check(bench.runner(), &lib, &reads, &none).expect("runs");
        assert!(!check.passed, "{}", check.detail);
        assert!(check.detail.contains("_ZN3std2fs"), "{}", check.detail);
        assert!(check.detail.contains("(fs)"), "{}", check.detail);

        let fs_ok: BTreeSet<&'static str> = ["fs", "os"].into_iter().collect();
        let check = capabilities_check(bench.runner(), &lib, &reads, &fs_ok).expect("runs");
        assert!(check.passed, "{}", check.detail);

        let clock = candidate(
            &bench,
            "clock_rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    let t = std::time::Instant::now().elapsed().as_nanos() as i32;\n    a.wrapping_add(b).wrapping_add(t & 0)\n}\n",
            FFI,
        );
        let lib = bench.build(&clock);
        let check = capabilities_check(bench.runner(), &lib, &clock, &fs_ok).expect("runs");
        assert!(!check.passed, "{}", check.detail);
        assert!(check.detail.contains("(time)"), "{}", check.detail);
    }

    /// Design B §B.8: a candidate that re-protects memory (here `mprotect`
    /// declared in its own shim) could unguard the boundary check's pages, so
    /// it is red unless the C unit itself maps or protects memory.
    #[test]
    fn a_candidate_that_reprotects_memory_is_red() {
        let bench = ToolBench::new("caps-mem");
        let ffi =
            "extern \"C\" {\n    fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;\n}\n\
                   #[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
                   let r = unsafe { mprotect(std::ptr::null_mut(), 0, 3) };\n    \
                   crate::logic::add(a, b).wrapping_add(r & 0)\n}\n";
        let dir = candidate(
            &bench,
            "unguard_rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n",
            ffi,
        );
        let lib = bench.build(&dir);
        let none = BTreeSet::new();
        let check = capabilities_check(bench.runner(), &lib, &dir, &none).expect("runs");
        assert!(!check.passed, "{}", check.detail);
        assert!(check.detail.contains("mprotect (mem)"), "{}", check.detail);
        let mem_ok: BTreeSet<&'static str> = ["mem"].into_iter().collect();
        let check = capabilities_check(bench.runner(), &lib, &dir, &mem_ok).expect("runs");
        assert!(check.passed, "{}", check.detail);
    }

    /// Design B §B.8: the runtime's entry points and the coverage callbacks
    /// are never a candidate's to reference, and a boundary-checked unit's
    /// candidate is never granted `mem` or `signal` even when its C uses them.
    #[test]
    fn runtime_names_are_always_red_and_boundary_units_never_get_signal() {
        let bench = ToolBench::new("caps-rt");
        let ffi = "extern \"C\" {\n    fn ruharness_exit();\n}\n\
                   #[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 {\n    \
                   unsafe { ruharness_exit() };\n    crate::logic::add(a, b)\n}\n";
        let dir = candidate(
            &bench,
            "rt_rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n",
            ffi,
        );
        let lib = bench.build(&dir);
        let all: BTreeSet<&'static str> = CLASSES.iter().map(|c| c.name).collect();
        let check = capabilities_check(bench.runner(), &lib, &dir, &all).expect("runs");
        assert!(!check.passed, "{}", check.detail);
        assert!(
            check.detail.contains("ruharness_exit (boundary-runtime)"),
            "{}",
            check.detail
        );

        let facts = Facts {
            frontend: "t".into(),
            refs: vec![harness_core::facts::RefRecord {
                file: "u.c".into(),
                from: "f".into(),
                to: "sigaction".into(),
                resolved: false,
                refkind: "call".into(),
            }],
            ..Facts::default()
        };
        let plain: Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"u.c\"]\nsymbols = [\"f\"]\n",
        )
        .expect("unit");
        assert!(unit_classes(&facts, &plain).contains("signal"));
        let opted: Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"u.c\"]\nsymbols = [\"f\"]\n\
             [oracle]\nkind = \"c-abi-differential\"\nboundary = true\n",
        )
        .expect("unit");
        assert!(!unit_classes(&facts, &opted).contains("signal"));
        assert!(is_mach_trap("_kernelrpc_mach_vm_protect_trap") && is_mach_trap("mach_msg2_trap"));
        assert!(!is_mach_trap("mach_absolute_time"));
    }

    #[test]
    fn asm_in_the_shim_is_red_without_building() {
        let tmp = TempDir::new("caps-asm");
        std::fs::create_dir_all(tmp.path().join("src")).expect("src");
        std::fs::write(
            tmp.path().join("src/ffi.rs"),
            "core::arch::global_asm!(\".globl _x\");\n",
        )
        .expect("ffi.rs");
        let mut found = Vec::new();
        rust_sources(&tmp.path().join("src"), &mut |rel, text| {
            for m in asm_macros(text) {
                found.push(format!("{rel}:{m}"));
            }
        })
        .expect("walks");
        assert_eq!(
            found,
            vec![
                "ffi.rs:`::arch` path".to_string(),
                "ffi.rs:global_asm!".to_string()
            ]
        );
    }
}
