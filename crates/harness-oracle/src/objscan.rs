//! Native scan of a staticlib for **loader-executed material**: anything that
//! makes the dynamic loader or the C runtime run candidate code without a
//! call from the C side — the constructor forgery (a pre-main static
//! initializer that prints forged output and exits before `main`).
//!
//! `nm` alone cannot see this: constructor entries are usually *local*
//! symbols (or no symbol at all — clang emits only an assembler temporary),
//! and on Mach-O what makes a section a constructor table is its **type**,
//! not its name — `#[link_section = "__DATA,__anything,mod_init_funcs"]` runs
//! before `main` just like `__mod_init_func` does. So the archive is parsed
//! here, in safe Rust, with no tool in between:
//!
//! - `ar` archives in the BSD (macOS) and GNU (Linux) flavours;
//! - Mach-O 64-bit little-endian members: every section whose type is
//!   `mod_init_funcs`, `mod_term_funcs`, `init_func_offsets`, `interposing` or
//!   `thread_local_init_function_pointers`, or whose name is
//!   `__mod_init_func` / `__mod_term_func` / `__init_offsets` / `__interpose`
//!   / `__objc_*` (libobjc runs `+load` methods at image load) in any
//!   segment, plus `LC_LINKER_OPTION` load commands;
//! - ELF 64-bit little-endian members: every section of type `INIT_ARRAY`,
//!   `FINI_ARRAY` or `PREINIT_ARRAY`, or named `.init_array` / `.fini_array`
//!   / `.preinit_array` / `.ctors` / `.dtors` / `.init` / `.fini` (exactly,
//!   or with a `.suffix`), plus every `STT_GNU_IFUNC` symbol (its resolver
//!   runs while the loader applies relocations).
//!
//! Each hit is a *finding* keyed by kind, section and archive member. The
//! symbol-set check subtracts the empty-crate baseline's findings as a
//! multiset (glibc targets legitimately carry std's own `.init_array.00099`
//! entry) and fails on any excess. A member in a format the scanner does not
//! understand is recorded as *unscanned* and treated the same way, so a
//! candidate cannot hide material in, say, an LLVM bitcode member.

use std::collections::{BTreeMap, BTreeSet};

/// What a staticlib carries that the loader would execute, plus what the
/// scanner could not look into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LoaderScan {
    /// Findings with their multiplicity.
    pub findings: BTreeMap<String, usize>,
    /// Archive members in a format the scanner does not understand.
    pub unscanned: BTreeMap<String, usize>,
    /// Number of object members scanned successfully. Zero for the BASELINE
    /// means the scanner does not understand this platform's objects at all.
    pub scanned: usize,
}

impl LoaderScan {
    /// Record one occurrence of `finding`.
    pub(crate) fn add_finding(&mut self, finding: String) {
        *self.findings.entry(finding).or_insert(0) += 1;
    }

    /// Record one member the scanner could not parse.
    pub(crate) fn add_unscanned(&mut self, member: String) {
        *self.unscanned.entry(member).or_insert(0) += 1;
    }
}

/// Result of [`scan_archive`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ArchiveScan {
    /// The findings.
    pub scan: LoaderScan,
    /// `(segment, section)` of every flagged Mach-O section, so the caller
    /// can ask `nm -m` which symbols live there.
    pub flagged_macho: BTreeSet<(String, String)>,
}

/// Scan the bytes of a staticlib. Never fails: an archive that cannot be
/// parsed is one big unscanned member.
pub(crate) fn scan_archive(bytes: &[u8]) -> ArchiveScan {
    let mut out = ArchiveScan::default();
    let members = match archive_members(bytes) {
        Ok(members) => members,
        Err(why) => {
            out.scan.add_unscanned(format!("<archive: {why}>"));
            return out;
        }
    };
    for member in &members {
        scan_member(member, &mut out);
    }
    out
}

/// One archive member.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Member<'a> {
    name: String,
    data: &'a [u8],
}

const AR_MAGIC: &[u8] = b"!<arch>\n";
const AR_HEADER_LEN: usize = 60;

/// Names of the archive's own bookkeeping members (symbol and string
/// tables). A member with one of these names is still scanned when its
/// content is an object file — the name alone never exempts anything.
const AR_TABLE_NAMES: [&str; 7] = [
    "__.SYMDEF",
    "__.SYMDEF SORTED",
    "__.SYMDEF_64",
    "__.SYMDEF_64 SORTED",
    "/",
    "//",
    "/SYM64/",
];

/// Split an `ar` archive into members (BSD `#1/<len>` and GNU `/<offset>`
/// long names both resolved).
fn archive_members(bytes: &[u8]) -> Result<Vec<Member<'_>>, String> {
    if !bytes.starts_with(AR_MAGIC) {
        return Err("not an ar archive".to_string());
    }
    let mut members = Vec::new();
    let mut gnu_names: &[u8] = &[];
    let mut pos = AR_MAGIC.len();
    while pos < bytes.len() {
        let header = slice(bytes, pos, AR_HEADER_LEN)
            .ok_or_else(|| format!("truncated member header at byte {pos}"))?;
        if &header[58..60] != b"`\n" {
            return Err(format!("bad member header at byte {pos}"));
        }
        let size: usize = ascii_field(&header[48..58])
            .parse()
            .map_err(|_| format!("bad member size at byte {pos}"))?;
        let data_start = pos + AR_HEADER_LEN;
        let mut data = slice(bytes, data_start, size)
            .ok_or_else(|| format!("member at byte {pos} overruns the archive"))?;
        let raw_name = ascii_field(&header[0..16]);
        let name = if let Some(len) = raw_name.strip_prefix("#1/") {
            // BSD: the name is the first <len> bytes of the data.
            let len: usize = len
                .parse()
                .map_err(|_| format!("bad BSD name length at byte {pos}"))?;
            let name_bytes =
                slice(data, 0, len).ok_or_else(|| format!("bad BSD name at byte {pos}"))?;
            data = &data[len..];
            c_string(name_bytes)
        } else if raw_name == "//" {
            gnu_names = data;
            raw_name
        } else if let Some(offset) = raw_name
            .strip_prefix('/')
            .and_then(|o| o.parse::<usize>().ok())
        {
            // GNU: an offset into the `//` table; entries end with "/\n".
            let tail = gnu_names.get(offset..).unwrap_or(&[]);
            let end = tail
                .iter()
                .position(|b| *b == b'\n')
                .unwrap_or(tail.len());
            let entry = printable(&String::from_utf8_lossy(&tail[..end]));
            entry.strip_suffix('/').unwrap_or(&entry).to_string()
        } else if AR_TABLE_NAMES.contains(&raw_name.as_str()) {
            raw_name
        } else {
            // GNU short names end with '/'; BSD short names do not.
            raw_name.strip_suffix('/').unwrap_or(&raw_name).to_string()
        };
        members.push(Member { name, data });
        // Members are padded to an even offset.
        pos = data_start + size + (size & 1);
    }
    Ok(members)
}

fn scan_member(member: &Member<'_>, out: &mut ArchiveScan) {
    let data = member.data;
    let parsed = if data.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
        scan_macho64(&member.name, data, out)
    } else if data.starts_with(b"\x7fELF") {
        scan_elf64(&member.name, data, &mut out.scan)
    } else if AR_TABLE_NAMES.contains(&member.name.as_str()) {
        // The archive's own symbol / string table.
        return;
    } else {
        None
    };
    match parsed {
        Some(()) => out.scan.scanned += 1,
        None => out.scan.add_unscanned(member.name.clone()),
    }
}

// ---------------------------------------------------------------- Mach-O --

const LC_SEGMENT_64: u32 = 0x19;
const LC_LINKER_OPTION: u32 = 0x2d;
const MACHO_HEADER_LEN: