//! The symbols an object file defines, read from its own symbol table in safe
//! Rust — no tool in between, so the features map needs nothing beyond `cc`
//! on the allowlist (docs/FEATURES-PROBE-REDESIGN.md §3.5, §3.2). Mach-O and
//! ELF, 64-bit little-endian, the objects the map's own compiles write.

/// One defined symbol: its C-level name (Mach-O's leading `_` dropped),
/// whether other objects can bind to it, and whether it is code (an ELF
/// function, a Mach-O symbol in a section of instructions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Defined {
    pub name: String,
    pub external: bool,
    pub function: bool,
}

/// Every symbol `object` defines (in a section: not undefined, not common,
/// no debugging entry). `Err` names what could not be read.
pub(crate) fn defined(object: &[u8]) -> Result<Vec<Defined>, String> {
    if object.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
        macho(object)
    } else if object.starts_with(b"\x7fELF") {
        elf(object)
    } else {
        Err("not a 64-bit little-endian Mach-O or ELF object".to_string())
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// The NUL-terminated name at `at` in a string table.
fn name_at(strings: &[u8], at: usize) -> Option<String> {
    let tail = strings.get(at..)?;
    let end = tail.iter().position(|b| *b == 0)?;
    Some(String::from_utf8_lossy(&tail[..end]).into_owned())
}

const LC_SYMTAB: u32 = 0x2;
const LC_SEGMENT_64: u32 = 0x19;
const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_SECT: u8 = 0x0e;
const N_EXT: u8 = 0x01;
const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x400;

/// Mach-O: `LC_SYMTAB`'s `nlist_64` entries of type `N_SECT`; a section's
/// flags (`LC_SEGMENT_64`, numbered from 1 in order) say whether it holds
/// instructions.
fn macho(b: &[u8]) -> Result<Vec<Defined>, String> {
    let bad = |what: &str| format!("the Mach-O object's {what} cannot be read");
    let ncmds = u32_at(b, 16).ok_or_else(|| bad("header"))? as usize;
    let mut at: usize = 32;
    let mut code: Vec<bool> = Vec::new();
    let mut symtab: Option<usize> = None;
    for _ in 0..ncmds {
        let cmd = u32_at(b, at).ok_or_else(|| bad("load commands"))?;
        let size = u32_at(b, at.saturating_add(4)).ok_or_else(|| bad("load commands"))? as usize;
        if cmd == LC_SYMTAB {
            symtab = Some(at);
        }
        if cmd == LC_SEGMENT_64 {
            let nsects = u32_at(b, at.saturating_add(64)).ok_or_else(|| bad("segments"))? as usize;
            for k in 0..nsects {
                let flags = k
                    .checked_mul(80)
                    .and_then(|o| o.checked_add(at.saturating_add(72 + 64)))
                    .and_then(|o| u32_at(b, o))
                    .ok_or_else(|| bad("sections"))?;
                code.push(flags & (S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS) != 0);
            }
        }
        if size == 0 {
            return Err(bad("load commands"));
        }
        at = at.checked_add(size).ok_or_else(|| bad("load commands"))?;
    }
    let Some(at) = symtab else {
        return Ok(Vec::new());
    };
    let field = |k: usize| u32_at(b, at.saturating_add(8 + 4 * k)).map(|v| v as usize);
    let (Some(symoff), Some(nsyms), Some(stroff), Some(strsize)) =
        (field(0), field(1), field(2), field(3))
    else {
        return Err(bad("symbol table"));
    };
    let strings = b
        .get(stroff..stroff.saturating_add(strsize))
        .ok_or_else(|| bad("string table"))?;
    let mut out = Vec::new();
    for k in 0..nsyms {
        let e = k
            .checked_mul(16)
            .and_then(|o| o.checked_add(symoff))
            .ok_or_else(|| bad("symbol table"))?;
        let (Some(strx), Some(&ty), Some(&sect)) = (
            u32_at(b, e),
            b.get(e.saturating_add(4)),
            b.get(e.saturating_add(5)),
        ) else {
            return Err(bad("symbol table"));
        };
        if ty & N_STAB != 0 || ty & N_TYPE != N_SECT {
            continue;
        }
        let name = name_at(strings, strx as usize).ok_or_else(|| bad("names"))?;
        let name = name.strip_prefix('_').unwrap_or(&name).to_string();
        out.push(Defined {
            name,
            external: ty & N_EXT != 0,
            function: (sect as usize)
                .checked_sub(1)
                .and_then(|i| code.get(i))
                .copied()
                .unwrap_or(false),
        });
    }
    Ok(out)
}

const SHT_SYMTAB: u32 = 2;
const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
const STB_LOCAL: u8 = 0;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;
const STT_FILE: u8 = 4;
const STT_GNU_IFUNC: u8 = 10;

/// ELF: `.symtab`'s entries defined in a section (not sections or files).
/// Every offset is checked: a malformed object is "cannot be read".
fn elf(b: &[u8]) -> Result<Vec<Defined>, String> {
    let bad = |what: &str| format!("the ELF object's {what} cannot be read");
    if b.get(4) != Some(&2) || b.get(5) != Some(&1) {
        return Err("not a 64-bit little-endian ELF object".to_string());
    }
    let size_of = |v: u64, what: &str| usize::try_from(v).map_err(|_| bad(what));
    let shoff = size_of(u64_at(b, 0x28).ok_or_else(|| bad("header"))?, "header")?;
    let shentsize = u16_at(b, 0x3a).ok_or_else(|| bad("header"))? as usize;
    let section = |i: usize| -> Result<usize, String> {
        i.checked_mul(shentsize)
            .and_then(|o| o.checked_add(shoff))
            .ok_or_else(|| bad("sections"))
    };
    let at = |base: usize, off: usize| base.checked_add(off).ok_or_else(|| bad("sections"));
    // 0xff00 sections or more: e_shnum is 0 and section 0's sh_size holds
    // the count.
    let shnum = match u16_at(b, 0x3c).ok_or_else(|| bad("header"))? {
        0 if shoff != 0 => size_of(
            u64_at(b, at(section(0)?, 0x20)?).ok_or_else(|| bad("sections"))?,
            "sections",
        )?,
        n => n as usize,
    };
    for i in 0..shnum {
        let s = section(i)?;
        if u32_at(b, at(s, 4)?).ok_or_else(|| bad("sections"))? != SHT_SYMTAB {
            continue;
        }
        let (Some(offset), Some(size), Some(link)) = (
            u64_at(b, at(s, 0x18)?),
            u64_at(b, at(s, 0x20)?),
            u32_at(b, at(s, 0x28)?),
        ) else {
            return Err(bad("symbol table"));
        };
        let (offset, size) = (
            size_of(offset, "symbol table")?,
            size_of(size, "symbol table")?,
        );
        let strs = section(link as usize)?;
        let (Some(str_off), Some(str_size)) =
            (u64_at(b, at(strs, 0x18)?), u64_at(b, at(strs, 0x20)?))
        else {
            return Err(bad("string table"));
        };
        let (str_off, str_size) = (
            size_of(str_off, "string table")?,
            size_of(str_size, "string table")?,
        );
        let strings = str_off
            .checked_add(str_size)
            .and_then(|end| b.get(str_off..end))
            .ok_or_else(|| bad("string table"))?;
        let mut out = Vec::new();
        for k in 1..size / 24 {
            let e = k
                .checked_mul(24)
                .and_then(|o| o.checked_add(offset))
                .ok_or_else(|| bad("symbol table"))?;
            let (Some(name), Some(&info), Some(shndx)) = (
                u32_at(b, e),
                b.get(e.saturating_add(4)),
                u16_at(b, e.saturating_add(6)),
            ) else {
                return Err(bad("symbol table"));
            };
            let kind = info & 0xf;
            if matches!(shndx, SHN_UNDEF | SHN_ABS | SHN_COMMON)
                || matches!(kind, STT_SECTION | STT_FILE)
            {
                continue;
            }
            out.push(Defined {
                name: name_at(strings, name as usize).ok_or_else(|| bad("names"))?,
                external: info >> 4 != STB_LOCAL,
                function: matches!(kind, STT_FUNC | STT_GNU_IFUNC),
            });
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real object from this machine's `cc`: an external and a static
    /// function, an external variable, and an undefined call — each read as
    /// the compiler wrote it.
    #[test]
    fn an_objects_own_symbols_read_back() {
        let dir = std::env::temp_dir().join(format!("rh-objsyms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("o.c");
        std::fs::write(
            &src,
            "extern int g(int);\nstatic int s(int x) { return g(x) + 1; }\n\
             int e(int x) { return s(x) * 2; }\nint v = 3;\n\
             int (*keep)(int) = s;\n",
        )
        .unwrap();
        let obj = dir.join("o.o");
        let built = std::process::Command::new("cc")
            .args(["-O0", "-c", "-o"])
            .arg(&obj)
            .arg(&src)
            .status()
            .expect("cc runs");
        assert!(built.success());
        let bytes = std::fs::read(&obj).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let found = defined(&bytes).unwrap();
        let has = |name: &str, external: bool| {
            found
                .iter()
                .any(|d| d.name == name && d.external == external)
        };
        assert!(has("e", true), "{found:?}");
        assert!(has("v", true), "{found:?}");
        assert!(has("s", false), "{found:?}");
        let function = |name: &str| found.iter().any(|d| d.name == name && d.function);
        assert!(function("e") && function("s"), "code: {found:?}");
        assert!(!function("v") && !function("keep"), "data: {found:?}");
        assert!(!found.iter().any(|d| d.name == "g"), "undefined: {found:?}");
        assert!(defined(b"not an object").is_err());
        assert!(defined(&bytes[..40]).is_err(), "cut short");
    }

    /// A 64-bit little-endian ELF object: sections 0 (null), 1 `.symtab`
    /// (at `symtab_at`), 2 its strings — `f` a global function, `d` a local
    /// object; `shnum` in the header (0: the count is section 0's size).
    fn elf_object(shnum: u16, symtab_at: u64) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        let strings = b"\0f\0d\0";
        let symbols_at = 64usize;
        let mut symbols = vec![0u8; 24 * 3];
        // f: name 1, STB_GLOBAL | STT_FUNC, section 1.
        symbols[24..28].copy_from_slice(&1u32.to_le_bytes());
        symbols[28] = (1 << 4) | 2;
        symbols[30..32].copy_from_slice(&1u16.to_le_bytes());
        // d: name 3, STB_LOCAL | STT_OBJECT, section 1.
        symbols[48..52].copy_from_slice(&3u32.to_le_bytes());
        symbols[52] = 1;
        symbols[54..56].copy_from_slice(&1u16.to_le_bytes());
        b.extend(&symbols);
        let strings_at = b.len();
        b.extend(strings);
        let shoff = b.len();
        let mut header = |kind: u32, offset: u64, size: u64, link: u32| {
            let mut h = vec![0u8; 64];
            h[4..8].copy_from_slice(&kind.to_le_bytes());
            h[0x18..0x20].copy_from_slice(&offset.to_le_bytes());
            h[0x20..0x28].copy_from_slice(&size.to_le_bytes());
            h[0x28..0x2c].copy_from_slice(&link.to_le_bytes());
            b.extend(h);
        };
        header(0, 0, if shnum == 0 { 3 } else { 0 }, 0);
        let symtab_at = if symtab_at == 0 {
            symbols_at as u64
        } else {
            symtab_at
        };
        header(SHT_SYMTAB, symtab_at, symbols.len() as u64, 2);
        header(3, strings_at as u64, strings.len() as u64, 0);
        b[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        b[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        b[0x3c..0x3e].copy_from_slice(&shnum.to_le_bytes());
        b
    }

    /// Review: 0xff00 sections or more keep their count in section 0 (the
    /// header's is 0) — never "defines nothing"; a malformed offset is
    /// "cannot be read", never a panic.
    #[test]
    fn an_elf_objects_section_count_and_offsets_are_read_with_care() {
        let want = vec![
            Defined {
                name: "f".to_string(),
                external: true,
                function: true,
            },
            Defined {
                name: "d".to_string(),
                external: false,
                function: false,
            },
        ];
        assert_eq!(defined(&elf_object(3, 0)), Ok(want.clone()));
        assert_eq!(defined(&elf_object(0, 0)), Ok(want), "extended numbering");
        assert!(defined(&elf_object(3, u64::MAX - 16)).is_err());
        assert!(defined(&elf_object(3, 1 << 40)).is_err());
    }
}
