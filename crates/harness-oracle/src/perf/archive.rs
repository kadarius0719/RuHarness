//! A unit's staticlib read for three facts (docs/PERF-DESIGN.md §3.2, build
//! note 15): its panic runtime, whether it carries std, and fat LTO. A small
//! `ar` reader in safe Rust beside `objsyms` — BSD long names (`#1/<len>`)
//! and GNU's `//` name table — no tool in between.

use crate::objsyms;

/// One member: its name and where its bytes lie in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    pub name: String,
    pub data: std::ops::Range<usize>,
}

const MAGIC: &[u8] = b"!<arch>\n";
const HEADER: usize = 60;

/// The members of an `ar` archive, in order (symbol tables and GNU's name
/// table left out). `Err` names what could not be read.
pub(crate) fn members(bytes: &[u8]) -> Result<Vec<Member>, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("not an ar archive".into());
    }
    let mut at = MAGIC.len();
    let mut out = Vec::new();
    let mut gnu_names: Option<std::ops::Range<usize>> = None;
    while at < bytes.len() {
        if bytes.len() - at < HEADER {
            return Err("a member header is cut short".into());
        }
        let h = &bytes[at..at + HEADER];
        if &h[58..60] != b"`\n" {
            return Err("a member header has no terminator".into());
        }
        let field = |r: std::ops::Range<usize>| -> &str {
            std::str::from_utf8(&h[r]).unwrap_or("").trim_end()
        };
        let size: usize = field(48..58)
            .parse()
            .map_err(|_| "a member size is not a number".to_string())?;
        let start = at + HEADER;
        let end = start
            .checked_add(size)
            .filter(|e| *e <= bytes.len())
            .ok_or("a member runs past the end")?;
        let raw = field(0..16);
        let (name, data_start) = if let Some(len) = raw.strip_prefix("#1/") {
            // BSD: the name is the data's first `len` bytes (NUL-padded).
            let len: usize = len
                .parse()
                .map_err(|_| "a BSD name length is not a number")?;
            if len > size {
                return Err("a BSD name runs past its member".into());
            }
            let name = String::from_utf8_lossy(&bytes[start..start + len])
                .trim_end_matches('\0')
                .to_string();
            (name, start + len)
        } else if raw == "//" {
            gnu_names = Some(start..end);
            at = end + (size & 1);
            continue;
        } else if raw == "/" || raw == "/SYM64/" || raw.starts_with("__.SYMDEF") {
            at = end + (size & 1);
            continue;
        } else if let Some(offset) = raw.strip_prefix('/') {
            // GNU: an offset into the `//` table, the name ending at "/\n".
            let offset: usize = offset
                .parse()
                .map_err(|_| "a GNU name offset is not a number")?;
            let table = gnu_names
                .clone()
                .ok_or("a GNU long name before its table")?;
            let names = &bytes[table];
            let rest = names
                .get(offset..)
                .ok_or("a GNU name offset past its table")?;
            let len = rest
                .windows(2)
                .position(|w| w == b"/\n")
                .ok_or("a GNU long name has no end")?;
            (String::from_utf8_lossy(&rest[..len]).to_string(), start)
        } else {
            (raw.trim_end_matches('/').to_string(), start)
        };
        out.push(Member {
            name,
            data: data_start..end,
        });
        at = end + (size & 1);
    }
    Ok(out)
}

/// A unit's panic runtime as its archive shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanicRuntime {
    /// A `panic_abort` member.
    Abort,
    /// A `panic_unwind` member.
    Unwind,
    /// Neither (fat LTO merges it; no-std has none).
    None,
}

impl PanicRuntime {
    /// The closed value stored on a row.
    pub(crate) fn token(self) -> &'static str {
        match self {
            PanicRuntime::Abort => "abort",
            PanicRuntime::Unwind => "unwind",
            PanicRuntime::None => "none",
        }
    }
}

/// What a unit's archive says (§3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchiveFacts {
    pub runtime: PanicRuntime,
    /// A std member.
    pub std: bool,
    /// std's code merged into the crate's own member: no std member, but
    /// `rust_eh_personality` defined.
    pub fat_lto: bool,
}

impl ArchiveFacts {
    /// A no-std unit: no std member and not fat LTO.
    pub(crate) fn no_std(&self) -> bool {
        !self.std && !self.fat_lto
    }
}

/// Whether member `name` is crate `krate`'s own object — `<krate>-<hash>.
/// <krate>…`, possibly after a thin-LTO `<crate>-<hash>.` prefix — matched
/// as name segments, never a crate whose own name merely contains it.
pub(crate) fn is_crate_member(name: &str, krate: &str) -> bool {
    let segments: Vec<&str> = name.split('.').collect();
    let at = |i: usize| -> bool {
        let (Some(first), Some(second)) = (segments.get(i), segments.get(i + 1)) else {
            return false;
        };
        let Some(hash) = first.strip_prefix(krate).and_then(|r| r.strip_prefix('-')) else {
            return false;
        };
        !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()) && *second == krate
    };
    at(0) || (at(1) && is_prefix_segment(segments[0]))
}

/// A thin-LTO prefix segment: `<crate>-<hex>`.
fn is_prefix_segment(s: &str) -> bool {
    s.rsplit_once('-').is_some_and(|(name, hash)| {
        !name.is_empty() && !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// Read a staticlib's facts.
pub(crate) fn archive_facts(bytes: &[u8]) -> Result<ArchiveFacts, String> {
    let members = members(bytes)?;
    let has = |krate: &str| members.iter().any(|m| is_crate_member(&m.name, krate));
    let runtime = if has("panic_abort") {
        PanicRuntime::Abort
    } else if has("panic_unwind") {
        PanicRuntime::Unwind
    } else {
        PanicRuntime::None
    };
    let std = has("std");
    let fat_lto = !std
        && members.iter().any(|m| {
            objsyms::defined(&bytes[m.data.clone()])
                .is_ok_and(|defined| defined.iter().any(|d| d.name == "rust_eh_personality"))
        });
    Ok(ArchiveFacts {
        runtime,
        std,
        fat_lto,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, size: usize) -> Vec<u8> {
        let mut h = format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{size:<10}",
            "0", "0", "0", "644"
        )
        .into_bytes();
        h.extend_from_slice(b"`\n");
        assert_eq!(h.len(), HEADER);
        h
    }

    fn bsd(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut a = MAGIC.to_vec();
        a.extend(header("__.SYMDEF", 4));
        a.extend_from_slice(&[0, 0, 0, 0]);
        for (name, data) in members {
            let mut padded = name.as_bytes().to_vec();
            while padded.len() % 8 != 0 {
                padded.push(0);
            }
            let size = padded.len() + data.len();
            a.extend(header(&format!("#1/{}", padded.len()), size));
            a.extend(&padded);
            a.extend_from_slice(data);
            if size % 2 == 1 {
                a.push(b'\n');
            }
        }
        a
    }

    fn gnu(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut a = MAGIC.to_vec();
        let mut table = Vec::new();
        let mut offsets = Vec::new();
        for (name, _) in members {
            offsets.push(table.len());
            table.extend_from_slice(name.as_bytes());
            table.extend_from_slice(b"/\n");
        }
        a.extend(header("/", 4));
        a.extend_from_slice(&[0, 0, 0, 0]);
        a.extend(header("//", table.len()));
        a.extend(&table);
        if table.len() % 2 == 1 {
            a.push(b'\n');
        }
        for ((_, data), offset) in members.iter().zip(offsets) {
            a.extend(header(&format!("/{offset}"), data.len()));
            a.extend_from_slice(data);
            if data.len() % 2 == 1 {
                a.push(b'\n');
            }
        }
        a
    }

    const ABORT: &str = "panic_abort-7df4dcaac3b27020.panic_abort.553ac0090239a4b3-cgu.0.rcgu.o";
    const UNWIND: &str = "panic_unwind-1475b884aeb90966.panic_unwind.ab9d04ec962c77d9-cgu.0.rcgu.o";
    const STD: &str = "std-8676e64d1195d4db.std.aa3e602c3b1d377d-cgu.0.rcgu.o";
    const THIN_STD: &str =
        "thinlto-5a05faefe14e9fad.std-8676e64d1195d4db.std.aa3e602c3b1d377d-cgu.0.rcgu.o.rcgu.o";

    #[test]
    fn bsd_and_gnu_long_names_read() {
        for archive in [
            bsd(&[(ABORT, b"x"), (STD, b"yy"), ("short.o", b"zzz")]),
            gnu(&[(ABORT, b"x"), (STD, b"yy"), ("short.o", b"zzz")]),
        ] {
            let m = members(&archive).expect("reads");
            let names: Vec<&str> = m.iter().map(|m| m.name.as_str()).collect();
            assert_eq!(names, [ABORT, STD, "short.o"]);
            assert_eq!(&archive[m[1].data.clone()], b"yy");
            assert_eq!(&archive[m[2].data.clone()], b"zzz");
        }
        assert!(members(b"not an archive").is_err());
        let mut cut = bsd(&[(STD, b"yy")]);
        cut.truncate(cut.len() - 1);
        assert!(members(&cut).is_err());
    }

    #[test]
    fn members_are_matched_as_name_segments() {
        assert!(is_crate_member(ABORT, "panic_abort"));
        assert!(is_crate_member(UNWIND, "panic_unwind"));
        assert!(is_crate_member(STD, "std"));
        assert!(is_crate_member(THIN_STD, "std"), "after a thin-LTO prefix");
        assert!(!is_crate_member(STD, "panic_abort"));
        // A crate whose own name merely contains them.
        assert!(!is_crate_member(
            "my_std-0123abcd.my_std.0-cgu.0.rcgu.o",
            "std"
        ));
        assert!(!is_crate_member("stdx-0123abcd.stdx.0-cgu.0.rcgu.o", "std"));
        assert!(!is_crate_member(
            "not_panic_abort-0123.not_panic_abort.o",
            "panic_abort"
        ));
        assert!(!is_crate_member("std-xyz.std.o", "std"), "the hash is hex");
    }

    /// Real staticlibs, built here (§4): an aborting, an unwinding, a
    /// thin-LTO, a fat-LTO and a no-std one, each read right.
    #[test]
    fn real_staticlibs_read_right() {
        if std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("no cargo here: skipped");
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("perf-ar-{}", harness_core::hash::random_hex(6)));
        let build = |name: &str, profile: &str, no_std: bool| -> Vec<u8> {
            let c = dir.join(name);
            std::fs::create_dir_all(c.join("src")).expect("dir");
            std::fs::write(
                c.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
                     [lib]\ncrate-type = [\"staticlib\"]\n[profile.release]\n{profile}\n[workspace]\n"
                ),
            )
            .expect("manifest");
            let lib = if no_std {
                "#![no_std]\n#[no_mangle] pub extern \"C\" fn add1(x: i32) -> i32 { x.wrapping_add(1) }\n\
                 #[panic_handler] fn p(_: &core::panic::PanicInfo) -> ! { loop {} }\n"
            } else {
                "#[no_mangle] pub extern \"C\" fn add1(x: i32) -> i32 { let v: Vec<i32> = vec![x]; v[0] + 1 }\n"
            };
            std::fs::write(c.join("src/lib.rs"), lib).expect("lib");
            // Its own target folder, named: a CARGO_TARGET_DIR set for the
            // outer test run would send the build elsewhere.
            let status = std::process::Command::new("cargo")
                .args(["build", "--release", "--offline", "-q", "--target-dir"])
                .arg(c.join("target"))
                .current_dir(&c)
                .status()
                .expect("cargo");
            assert!(status.success(), "{name} builds");
            std::fs::read(c.join(format!("target/release/lib{name}.a"))).expect("archive")
        };
        let aborting =
            archive_facts(&build("aborting", "panic = \"abort\"", false)).expect("facts");
        assert_eq!(
            aborting,
            ArchiveFacts {
                runtime: PanicRuntime::Abort,
                std: true,
                fat_lto: false
            }
        );
        let unwinding = archive_facts(&build("unwinding", "", false)).expect("facts");
        assert_eq!(
            unwinding,
            ArchiveFacts {
                runtime: PanicRuntime::Unwind,
                std: true,
                fat_lto: false
            }
        );
        let thin = archive_facts(&build("thinlto", "lto = \"thin\"", false)).expect("facts");
        assert_eq!(
            thin,
            ArchiveFacts {
                runtime: PanicRuntime::Unwind,
                std: true,
                fat_lto: false
            }
        );
        let fat = archive_facts(&build("fatlto", "lto = true", false)).expect("facts");
        assert_eq!(
            fat,
            ArchiveFacts {
                runtime: PanicRuntime::None,
                std: false,
                fat_lto: true
            }
        );
        let no_std = archive_facts(&build("nostd", "panic = \"abort\"", true)).expect("facts");
        assert_eq!(
            no_std,
            ArchiveFacts {
                runtime: PanicRuntime::None,
                std: false,
                fat_lto: false
            }
        );
        assert!(no_std.no_std() && !fat.no_std());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_facts_of_each_kind() {
        let aborting = archive_facts(&bsd(&[(ABORT, b""), (STD, b"")])).expect("facts");
        assert_eq!(
            aborting,
            ArchiveFacts {
                runtime: PanicRuntime::Abort,
                std: true,
                fat_lto: false
            }
        );
        let unwinding = archive_facts(&bsd(&[(STD, b""), (UNWIND, b"")])).expect("facts");
        assert_eq!(unwinding.runtime, PanicRuntime::Unwind);
        let no_std = archive_facts(&bsd(&[("core-d9dfac99c0c5cf1c.core.8a-cgu.0.rcgu.o", b"")]))
            .expect("facts");
        assert_eq!(
            no_std,
            ArchiveFacts {
                runtime: PanicRuntime::None,
                std: false,
                fat_lto: false
            }
        );
        assert!(no_std.no_std());
        let thin = archive_facts(&gnu(&[(THIN_STD, b"")])).expect("facts");
        assert!(thin.std);
    }
}
