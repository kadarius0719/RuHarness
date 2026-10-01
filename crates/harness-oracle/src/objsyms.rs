//! The symbols an object file defines, read from its own symbol table in safe
//! Rust — no tool in between, so the features map needs nothing beyond `cc`
//! on the allowlist (docs/FEATURES-PROBE-REDESIGN.md §3.5, §3.2). Mach-O and
//! ELF, 64-bit little-endian, the objects the map's own compiles write.

/// One defined symbol: its C-level name (Mach-O's leading `_` dropped) and
/// whether other objects can bind to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Defined {
    pub name: String,
    pub external: bool,
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
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The NUL-terminated name at `at` in a string table.
fn name_at(strings: &[u8], at: usize) -> Option<String> {
    let tail = strings.get(at..)?;
    let end = tail.iter().position(|b| *b == 0)?;
    Some(String::from_utf8_lossy(&tail[..end]).into_owned())
}

const LC_SYMTAB: u32 = 0x2;
const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_SECT: u8 = 0x0e;
const N_EXT: u8 = 0x01;

/// Mach-O: `LC_SYMTAB`'s `nlist_64` entries of type `N_SECT`.
fn macho(b: &[u8]) -> Result<Vec<Defined>, String> {
    let bad = |what: &str| format!("the Mach-O object's {what} cannot be read");
    let ncmds = u32_at(b, 16).ok_or_else(|| bad("header"))? as usize;
    let mut at = 32;
    for _ in 0..ncmds {
        let cmd = u32_at(b, at).ok_or_else(|| bad("load commands"))?;
        let size = u32_at(b, at + 4).ok_or_else(|| bad("load commands"))? as usize;
        if cmd == LC_SYMTAB {
            let field = |k: usize| u32_at(b, at + 8 + 4 * k).map(|v| v as usize);
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
                let e = symoff + 16 * k;
                let (Some(strx), Some(&ty)) = (u32_at(b, e), b.get(e + 4)) else {
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
                });
            }
            return Ok(out);
        }
        if size == 0 {
            return Err(bad("load commands"));
        }
        at += size;
    }
    Ok(Vec::new())
}

const SHT_SYMTAB: u32 = 2;
const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
const STB_LOCAL: u8 = 0;
const STT_SECTION: u8 = 3;
const STT_FILE: u8 = 4;

/// ELF: `.symtab`'s entries defined in a section (not sections or files).
fn elf(b: &[u8]) -> Result<Vec<Defined>, String> {
    let bad = |what: &str| format!("the ELF object's {what} cannot be read");
    if b.get(4) != Some(&2) || b.get(5) != Some(&1) {
        return Err("not a 64-bit little-endian ELF object".to_string());
    }
    let shoff = u64_at(b, 0x28).ok_or_else(|| bad("header"))? as usize;
    let shentsize = u16_at(b, 0x3a).ok_or_else(|| bad("header"))? as usize;
    let shnum = u16_at(b, 0x3c).ok_or_else(|| bad("header"))? as usize;
    let section = |i: usize| shoff + i * shentsize;
    for i in 0..shnum {
        let s = section(i);
        if u32_at(b, s + 4).ok_or_else(|| bad("sections"))? != SHT_SYMTAB {
            continue;
        }
        let (Some(offset), Some(size), Some(link)) = (
            u64_at(b, s + 0x18),
            u64_at(b, s + 0x20),
            u32_at(b, s + 0x28),
        ) else {
            return Err(bad("symbol table"));
        };
        let strs = section(link as usize);
        let (Some(str_off), Some(str_size)) = (u64_at(b, strs + 0x18), u64_at(b, strs + 0x20))
        else {
            return Err(bad("string table"));
        };
        let strings = b
            .get(str_off as usize..(str_off as usize).saturating_add(str_size as usize))
            .ok_or_else(|| bad("string table"))?;
        let mut out = Vec::new();
        for k in 1..(size as usize) / 24 {
            let e = offset as usize + 24 * k;
            let (Some(name), Some(&info), Some(shndx)) =
                (u32_at(b, e), b.get(e + 4), u16_at(b, e + 6))
            else {
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
        assert!(!found.iter().any(|d| d.name == "g"), "undefined: {found:?}");
        assert!(defined(b"not an object").is_err());
        assert!(defined(&bytes[..40]).is_err(), "cut short");
    }
}
