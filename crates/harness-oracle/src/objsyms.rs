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
/// no debugging entry). `Err` names what could not be read. Common symbols
/// are left out here; [`external`] is the read that reports them (as
/// definitions of kind [`Kind::Common`]).
pub(crate) fn defined(object: &[u8]) -> Result<Vec<Defined>, String> {
    symbols(object).map(|read| read.defined)
}

/// The external symbols `object` leaves undefined (Mach-O's leading `_`
/// dropped; a common symbol is not one) — which objects still reference a
/// symbol the link misses (fix pass 3's check).
pub(crate) fn undefined(object: &[u8]) -> Result<Vec<String>, String> {
    symbols(object).map(|read| read.undefined)
}

// The project map's read below (kinds, weakness, commons) is wired in by the
// map's step b (docs/PROJECT-MAP-DESIGN.md §5); until then only the tests
// call it, hence the `dead_code` allowances outside tests.

/// What an external symbol an object defines holds (the project map's read,
/// docs/PROJECT-MAP-DESIGN.md §3.1 step 5).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Code: an ELF function, a symbol in a Mach-O section of instructions.
    Function,
    /// Writable, initialized data (Mach-O `__DATA,__data`, ELF `SHF_WRITE`).
    Data,
    /// Constant data (Mach-O `__TEXT,__const`, `__DATA_CONST`, any `__const`
    /// section; an ELF section without `SHF_WRITE`, as `.rodata`).
    ReadOnly,
    /// Zero-filled storage (Mach-O `__DATA,__bss`, or `__DATA,__common` — a
    /// tentative definition under `-fno-common`, a strong one; ELF
    /// `SHT_NOBITS`).
    Bss,
    /// A common symbol: on Mach-O an undefined external symbol with a size in
    /// `n_value`, on ELF `SHN_COMMON`. The linker merges several of one name.
    Common,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Kind {
    /// The kind's name as the map stores it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Function => "function",
            Kind::Data => "data",
            Kind::ReadOnly => "read-only",
            Kind::Bss => "bss",
            Kind::Common => "common",
        }
    }
}

/// One external symbol an object defines: its name (Mach-O's one leading
/// `_` dropped, otherwise raw — check it with [`identifier_shaped`]), its
/// kind, and whether the definition is weak (Mach-O `N_WEAK_DEF`, ELF
/// `STB_WEAK`).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Definition {
    pub name: String,
    pub kind: Kind,
    pub weak: bool,
}

/// One external symbol an object needs: its name (as [`Definition`]'s) and
/// whether the reference is weak (Mach-O `N_WEAK_REF`, ELF `STB_WEAK`).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Need {
    pub name: String,
    pub weak: bool,
}

/// The external symbols of one object: those it defines (commons included)
/// and those it needs (a common symbol is never a need). Local symbols are
/// never read.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct External {
    pub defines: Vec<Definition>,
    pub needs: Vec<Need>,
}

/// The external symbols `object` defines and needs, with kinds and weakness
/// — the project map's read (docs/PROJECT-MAP-DESIGN.md §3.1 step 5). Unlike
/// [`defined`], it reports common symbols, as definitions. An ELF absolute
/// symbol (`SHN_ABS`) is not reported, as [`defined`] skips it. `Err` names
/// what could not be read.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn external(object: &[u8]) -> Result<External, String> {
    symbols(object).map(|read| read.external)
}

/// A name with exactly one `$` suffix, both sides made of letters, digits
/// and `_` (Apple's libc variants: `realpath$DARWIN_EXTSN`,
/// `opendir$INODE64`), gives its base name (`realpath`); any other name gives
/// `None`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn dollar_base(name: &str) -> Option<&str> {
    let (base, suffix) = name.split_once('$')?;
    let plain =
        |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    (plain(base) && plain(suffix)).then_some(base)
}

/// Whether `name` is shaped like a C identifier (`[A-Za-z_][A-Za-z0-9_]*`),
/// after the `$`-suffix rule of [`dollar_base`]: an `asm` label can name a
/// symbol with any text, and such a name is counted, never stored.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn identifier_shaped(name: &str) -> bool {
    let name = dollar_base(name).unwrap_or(name);
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// One object's symbols as every read above sees them, read in one pass.
#[derive(Default)]
struct Symbols {
    defined: Vec<Defined>,
    undefined: Vec<String>,
    external: External,
}

fn symbols(object: &[u8]) -> Result<Symbols, String> {
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
const N_UNDF: u8 = 0x00;
const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x400;
const SECTION_TYPE: u32 = 0xff;
const S_ZEROFILL: u32 = 0x1;
const S_GB_ZEROFILL: u32 = 0xc;
const S_THREAD_LOCAL_ZEROFILL: u32 = 0x12;
const N_WEAK_REF: u16 = 0x0040;
const N_WEAK_DEF: u16 = 0x0080;

/// A 16-byte, NUL-padded Mach-O segment or section name at `at`.
fn fixed_name(b: &[u8], at: usize) -> Option<&[u8]> {
    let field = b.get(at..at.checked_add(16)?)?;
    let end = field.iter().position(|c| *c == 0).unwrap_or(field.len());
    field.get(..end)
}

/// The kind of what a Mach-O section holds, from its flags and its
/// `segment,section` names: instructions are code; a zero-fill section
/// (`__bss`, `__common`) is bss; `__TEXT`'s other sections, `__DATA_CONST`
/// and any `__const` section are read-only; the rest is data.
fn macho_kind(flags: u32, segment: &[u8], section: &[u8]) -> Kind {
    if flags & (S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS) != 0 {
        Kind::Function
    } else if matches!(
        flags & SECTION_TYPE,
        S_ZEROFILL | S_GB_ZEROFILL | S_THREAD_LOCAL_ZEROFILL
    ) {
        Kind::Bss
    } else if segment == b"__TEXT" || segment == b"__DATA_CONST" || section == b"__const" {
        Kind::ReadOnly
    } else {
        Kind::Data
    }
}

/// Mach-O: `LC_SYMTAB`'s `nlist_64` entries of type `N_SECT`; a section's
/// flags (`LC_SEGMENT_64`, numbered from 1 in order) say whether it holds
/// instructions, and with its names what kind of symbol it holds.
fn macho(b: &[u8]) -> Result<Symbols, String> {
    let bad = |what: &str| format!("the Mach-O object's {what} cannot be read");
    let ncmds = u32_at(b, 16).ok_or_else(|| bad("header"))? as usize;
    let mut at: usize = 32;
    let mut kinds: Vec<Kind> = Vec::new();
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
                // A section_64: sectname at 0, segname at 16, flags at 64.
                let s = k
                    .checked_mul(80)
                    .and_then(|o| o.checked_add(at.saturating_add(72)))
                    .ok_or_else(|| bad("sections"))?;
                let flags = s
                    .checked_add(64)
                    .and_then(|o| u32_at(b, o))
                    .ok_or_else(|| bad("sections"))?;
                // The names lie before the flags, so they are in the slice
                // whenever the flags are.
                let section = fixed_name(b, s).unwrap_or_default();
                let segment = s
                    .checked_add(16)
                    .and_then(|o| fixed_name(b, o))
                    .unwrap_or_default();
                kinds.push(macho_kind(flags, segment, section));
            }
        }
        if size == 0 {
            return Err(bad("load commands"));
        }
        at = at.checked_add(size).ok_or_else(|| bad("load commands"))?;
    }
    let Some(at) = symtab else {
        return Ok(Symbols::default());
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
    let mut read = Symbols::default();
    let name_of = |strx: u32| -> Result<String, String> {
        let name = name_at(strings, strx as usize).ok_or_else(|| bad("names"))?;
        Ok(name.strip_prefix('_').unwrap_or(&name).to_string())
    };
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
        // n_desc: the weak flags (an unreadable one reads as no flag).
        let desc = u16_at(b, e.saturating_add(6)).unwrap_or(0);
        if ty & N_STAB == 0 && ty & N_TYPE == N_UNDF && ty & N_EXT != 0 {
            match u64_at(b, e.saturating_add(8)) {
                // An external undefined symbol: a need.
                Some(0) => {
                    let name = name_of(strx)?;
                    read.external.needs.push(Need {
                        name: name.clone(),
                        weak: desc & N_WEAK_REF != 0,
                    });
                    read.undefined.push(name);
                }
                // With a size in n_value it is a common symbol: a definition
                // (never weak; N_WEAK_DEF is read on section symbols only).
                Some(_) => read.external.defines.push(Definition {
                    name: name_of(strx)?,
                    kind: Kind::Common,
                    weak: false,
                }),
                None => {}
            }
            continue;
        }
        if ty & N_STAB != 0 || ty & N_TYPE != N_SECT {
            continue;
        }
        let name = name_of(strx)?;
        let kind = (sect as usize)
            .checked_sub(1)
            .and_then(|i| kinds.get(i))
            .copied();
        if ty & N_EXT != 0 {
            read.external.defines.push(Definition {
                name: name.clone(),
                kind: kind.unwrap_or(Kind::Data),
                weak: desc & N_WEAK_DEF != 0,
            });
        }
        read.defined.push(Defined {
            name,
            external: ty & N_EXT != 0,
            function: kind == Some(Kind::Function),
        });
    }
    Ok(read)
}

const SHT_SYMTAB: u32 = 2;
const SHT_SYMTAB_SHNDX: u32 = 18;
const SHN_LORESERVE: u16 = 0xff00;
const SHN_XINDEX: u16 = 0xffff;
const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
const STB_LOCAL: u8 = 0;
const STB_WEAK: u8 = 2;
const SHT_NOBITS: u32 = 8;
const SHF_WRITE: u64 = 0x1;
const STT_NOTYPE: u8 = 0;
const STT_FUNC: u8 = 2;
const SHF_EXECINSTR: u64 = 0x4;
const STT_SECTION: u8 = 3;
const STT_FILE: u8 = 4;
const STT_GNU_IFUNC: u8 = 10;

/// ELF: `.symtab`'s entries defined in a section (not sections or files).
/// Every offset is checked: a malformed object is "cannot be read".
fn elf(b: &[u8]) -> Result<Symbols, String> {
    let bad = |what: &str| format!("the ELF object's {what} cannot be read");
    if b.get(4) != Some(&2) || b.get(5) != Some(&1) {
        return Err("not a 64-bit little-endian ELF object".to_string());
    }
    let size_of = |v: u64, what: &str| usize::try_from(v).map_err(|_| bad(what));
    let shoff = size_of(u64_at(b, 0x28).ok_or_else(|| bad("header"))?, "header")?;
    let shentsize = u16_at(b, 0x3a).ok_or_else(|| bad("header"))? as usize;
    // An Elf64_Shdr is 64 bytes (the fields below assume it); any other size
    // with a section table is a malformed object, never a long loop over one
    // header (fix pass 2's check).
    if shoff != 0 && shentsize != 64 {
        return Err(bad("section headers"));
    }
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
        // Extended section indexes (SHN_XINDEX): the SHT_SYMTAB_SHNDX table
        // linked to this symbol table, entry k (fix pass 3's check).
        let mut shndx_table: Option<usize> = None;
        for j in 0..shnum {
            let h = section(j)?;
            if u32_at(b, at(h, 4)?) == Some(SHT_SYMTAB_SHNDX)
                && u32_at(b, at(h, 0x28)?) == Some(i as u32)
            {
                shndx_table = Some(size_of(
                    u64_at(b, at(h, 0x18)?).ok_or_else(|| bad("section indexes"))?,
                    "section indexes",
                )?);
            }
        }
        let mut read = Symbols::default();
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
            let binding = info >> 4;
            if shndx == SHN_UNDEF && binding != STB_LOCAL && name != 0 {
                let name = name_at(strings, name as usize).ok_or_else(|| bad("names"))?;
                read.external.needs.push(Need {
                    name: name.clone(),
                    weak: binding == STB_WEAK,
                });
                read.undefined.push(name);
                continue;
            }
            // A common symbol: a definition for the map's read only.
            if shndx == SHN_COMMON && binding != STB_LOCAL && name != 0 {
                read.external.defines.push(Definition {
                    name: name_at(strings, name as usize).ok_or_else(|| bad("names"))?,
                    kind: Kind::Common,
                    weak: binding == STB_WEAK,
                });
                continue;
            }
            if matches!(shndx, SHN_UNDEF | SHN_ABS | SHN_COMMON)
                || matches!(kind, STT_SECTION | STT_FILE)
            {
                continue;
            }
            // The symbol's section's (sh_type, sh_flags), if it has a plain
            // or extended index to a section that exists; a reserved index
            // has none (fix pass 3's check).
            let holder = || -> Option<(u32, u64)> {
                let index = match shndx {
                    SHN_XINDEX => k
                        .checked_mul(4)
                        .and_then(|o| o.checked_add(shndx_table?))
                        .and_then(|o| u32_at(b, o))? as usize,
                    reserved if reserved >= SHN_LORESERVE => return None,
                    plain => plain as usize,
                };
                if index >= shnum {
                    return None;
                }
                let h = section(index).ok()?;
                Some((
                    u32_at(b, h.checked_add(4)?)?,
                    u64_at(b, h.checked_add(0x08)?)?,
                ))
            };
            let holder = holder();
            // Code: a function, or an untyped symbol in a section of
            // instructions (a function written in assembly without `.type`,
            // as Mach-O's section rule reads it; fix pass 2's check).
            let function = matches!(kind, STT_FUNC | STT_GNU_IFUNC)
                || (kind == STT_NOTYPE
                    && holder.is_some_and(|(_, flags)| flags & SHF_EXECINSTR != 0));
            let name = name_at(strings, name as usize).ok_or_else(|| bad("names"))?;
            if binding != STB_LOCAL {
                read.external.defines.push(Definition {
                    name: name.clone(),
                    kind: match holder {
                        _ if function => Kind::Function,
                        Some((SHT_NOBITS, _)) => Kind::Bss,
                        Some((_, flags)) if flags & SHF_WRITE == 0 => Kind::ReadOnly,
                        _ => Kind::Data,
                    },
                    weak: binding == STB_WEAK,
                });
            }
            read.defined.push(Defined {
                name,
                external: binding != STB_LOCAL,
                function,
            });
        }
        return Ok(read);
    }
    Ok(Symbols::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map's test file: every kind, weak definitions and references, a
    /// static function (never reported) and an `asm` label that is no C
    /// identifier.
    const KINDS_C: &str = "\
int strong_fn(int x) { return x + 1; }
__attribute__((weak)) int weak_fn(int x) { return x * 2; }
extern int opt __attribute__((weak));
int use_opt(void) { return &opt ? opt : 0; }
extern int needed(int);
int calls(void) { return needed(3); }
int tentative;
int zeroed = 0;
int initialized = 7;
const int constant = 9;
static int hidden(int x) { return x - 1; }
int (*keep)(int) = hidden;
int odd(void) __asm__(\"odd.label\");
int odd(void) { return 4; }
";

    /// `KINDS_C` compiled by this machine's `cc -c` with `args` in a fresh
    /// temp folder; `None` when `cc` refuses (an option this host's compiler
    /// lacks).
    fn compile(tag: &str, args: &[&str]) -> Option<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("rh-objsyms-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("k.c");
        std::fs::write(&src, KINDS_C).unwrap();
        let obj = dir.join("k.o");
        let built = std::process::Command::new("cc")
            .args(args)
            .args(["-O0", "-c", "-o"])
            .arg(&obj)
            .arg(&src)
            .stderr(std::process::Stdio::null())
            .status()
            .expect("cc runs");
        let bytes = built.success().then(|| std::fs::read(&obj).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    fn def(name: &str, kind: Kind, weak: bool) -> Definition {
        Definition {
            name: name.to_string(),
            kind,
            weak,
        }
    }

    /// The definitions and needs `KINDS_C` gives, sorted by name, with
    /// `tentative` of kind `tentative`.
    fn expected(tentative: Kind) -> (Vec<Definition>, Vec<Need>) {
        let mut defines = vec![
            def("strong_fn", Kind::Function, false),
            def("weak_fn", Kind::Function, true),
            def("use_opt", Kind::Function, false),
            def("calls", Kind::Function, false),
            def("odd.label", Kind::Function, false),
            def("tentative", tentative, false),
            def("zeroed", Kind::Bss, false),
            def("initialized", Kind::Data, false),
            def("keep", Kind::Data, false),
            def("constant", Kind::ReadOnly, false),
        ];
        defines.sort_by(|a, b| a.name.cmp(&b.name));
        let needs = vec![
            Need {
                name: "needed".to_string(),
                weak: false,
            },
            Need {
                name: "opt".to_string(),
                weak: true,
            },
        ];
        (defines, needs)
    }

    /// Checks one object of `KINDS_C`: the new read's kinds, weak flags and
    /// names, and the old reads' answers unchanged (`defined()` leaves a
    /// common symbol out, `undefined()` names the needs).
    fn check(bytes: &[u8], tentative: Kind) {
        let mut got = external(bytes).unwrap();
        got.defines.sort_by(|a, b| a.name.cmp(&b.name));
        got.needs.sort_by(|a, b| a.name.cmp(&b.name));
        let (defines, needs) = expected(tentative);
        assert_eq!(got.defines, defines, "{tentative:?}");
        assert_eq!(got.needs, needs, "{tentative:?}");
        // Local symbols (the static function, Mach-O's ltmp labels) are never
        // reported; the underscore is gone; the asm label stays raw.
        assert!(got.defines.iter().all(|d| !d.name.starts_with('_')));
        assert!(!identifier_shaped("odd.label"));

        let old = defined(bytes).unwrap();
        let has = |name: &str| old.iter().any(|d| d.name == name);
        assert!(has("hidden") && !old.iter().any(|d| d.name == "hidden" && d.external));
        assert!(old.iter().any(|d| d.name == "strong_fn" && d.function));
        assert!(old.iter().any(|d| d.name == "constant" && !d.function));
        assert_eq!(has("tentative"), tentative != Kind::Common, "{old:?}");
        assert!(!has("needed") && !has("opt"));
        let mut old_undefined = undefined(bytes).unwrap();
        old_undefined.sort();
        assert_eq!(old_undefined, vec!["needed".to_string(), "opt".to_string()]);
    }

    /// Mach-O, from this Mac's `cc`: a tentative definition is common by
    /// default (Apple clang) and a strong bss under `-fno-common`.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_macho_objects_external_symbols_have_kinds_and_weakness() {
        let common = compile("macho", &[]).expect("cc compiles");
        assert!(common.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]));
        check(&common, Kind::Common);
        let strict = compile("macho-nocommon", &["-fno-common"]).expect("cc compiles");
        check(&strict, Kind::Bss);
    }

    /// ELF, made with `cc -target x86_64-unknown-linux-gnu -c` (clang): the
    /// test is gated to hosts where that works — gcc has no `-target`, so on
    /// such a host it says so and checks nothing. Clang's ELF default is
    /// `-fno-common` (a strong bss); `-fcommon` gives an `SHN_COMMON` symbol.
    #[test]
    fn an_elf_objects_external_symbols_have_kinds_and_weakness() {
        let target = ["-target", "x86_64-unknown-linux-gnu"];
        let Some(strict) = compile("elf", &target) else {
            eprintln!("skipped: this host's cc cannot make an x86_64 ELF object with -target");
            return;
        };
        assert!(strict.starts_with(b"\x7fELF"));
        check(&strict, Kind::Bss);
        let common = compile("elf-common", &[target[0], target[1], "-fcommon"])
            .expect("cc compiles with -fcommon");
        check(&common, Kind::Common);
    }

    #[test]
    fn a_macho_sections_names_and_flags_give_the_kind() {
        let k =
            |flags: u32, seg: &str, sect: &str| macho_kind(flags, seg.as_bytes(), sect.as_bytes());
        assert_eq!(
            k(S_ATTR_PURE_INSTRUCTIONS, "__TEXT", "__text"),
            Kind::Function
        );
        assert_eq!(
            k(S_ATTR_SOME_INSTRUCTIONS, "__TEXT", "__stubs"),
            Kind::Function
        );
        assert_eq!(k(0, "__TEXT", "__const"), Kind::ReadOnly);
        assert_eq!(k(0, "__TEXT", "__cstring"), Kind::ReadOnly);
        assert_eq!(k(0, "__DATA_CONST", "__got"), Kind::ReadOnly);
        assert_eq!(k(0, "__DATA", "__const"), Kind::ReadOnly);
        assert_eq!(k(0, "__DATA", "__data"), Kind::Data);
        assert_eq!(k(S_ZEROFILL, "__DATA", "__bss"), Kind::Bss);
        assert_eq!(k(S_ZEROFILL, "__DATA", "__common"), Kind::Bss);
        assert_eq!(
            k(S_THREAD_LOCAL_ZEROFILL, "__DATA", "__thread_bss"),
            Kind::Bss
        );
        let names: Vec<&str> = [
            Kind::Function,
            Kind::Data,
            Kind::ReadOnly,
            Kind::Bss,
            Kind::Common,
        ]
        .iter()
        .map(|k| k.as_str())
        .collect();
        assert_eq!(names, ["function", "data", "read-only", "bss", "common"]);
    }

    #[test]
    fn a_libc_dollar_suffix_and_the_identifier_shape() {
        assert_eq!(dollar_base("realpath$DARWIN_EXTSN"), Some("realpath"));
        assert_eq!(dollar_base("opendir$INODE64"), Some("opendir"));
        assert_eq!(dollar_base("realpath"), None);
        assert_eq!(dollar_base("a$b$c"), None, "one suffix only");
        assert_eq!(dollar_base("$x"), None);
        assert_eq!(dollar_base("x$"), None);
        assert_eq!(dollar_base("x$a.b"), None);
        assert!(identifier_shaped("realpath$DARWIN_EXTSN"));
        assert!(identifier_shaped("_private9"));
        assert!(identifier_shaped("x"));
        assert!(!identifier_shaped(""));
        assert!(!identifier_shaped("9lives"));
        assert!(!identifier_shaped("odd.label"));
        assert!(!identifier_shaped("a b"));
        assert!(!identifier_shaped("a$b$c"));
        assert!(!identifier_shaped("$x"));
        assert!(!identifier_shaped("é"));
    }

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
        assert_eq!(
            undefined(&bytes),
            Ok(vec!["g".to_string()]),
            "g is referenced"
        );
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
        // Fix pass 2's check: a section entry size of 0 is malformed, never a
        // long loop over one header.
        let mut zero = elf_object(0, 0);
        zero[0x3a..0x3c].copy_from_slice(&0u16.to_le_bytes());
        assert!(defined(&zero).is_err());
        // An untyped symbol in a section of instructions is code; in a data
        // section it is not.
        let mut asm = elf_object(3, 0);
        let shoff = u64::from_le_bytes(asm[0x28..0x30].try_into().unwrap()) as usize;
        let symtab = shoff + 64;
        let symbols =
            u64::from_le_bytes(asm[symtab + 0x18..symtab + 0x20].try_into().unwrap()) as usize;
        // f becomes STT_NOTYPE in section 1 (the symbol table's own header).
        asm[symbols + 28] = 1 << 4;
        let found = defined(&asm).unwrap();
        assert!(!found[0].function, "section 1 is not code: {found:?}");
        asm[symtab + 0x08..symtab + 0x10].copy_from_slice(&SHF_EXECINSTR.to_le_bytes());
        let found = defined(&asm).unwrap();
        assert!(found[0].function, "{found:?}");
        assert!(!found[1].function, "an object stays data: {found:?}");
        // Fix pass 3's check: a reserved section index never borrows another
        // section's flags; SHN_XINDEX with no index table is not code.
        for reserved in [0xff05u16, SHN_XINDEX] {
            let mut odd = asm.clone();
            odd[symbols + 30..symbols + 32].copy_from_slice(&reserved.to_le_bytes());
            let found = defined(&odd).unwrap();
            assert!(!found[0].function, "{reserved:#x}: {found:?}");
        }
    }

    /// Fix pass 4's check — extended section indexes: an untyped symbol
    /// whose st_shndx is SHN_XINDEX takes its section from entry k of the
    /// SHT_SYMTAB_SHNDX table linked to the symbol table — code when that
    /// section holds instructions, data otherwise; a table linked to another
    /// section is not this symbol table's.
    #[test]
    fn an_extended_index_is_read_from_its_own_table() {
        // Sections: 0 null, 1 .symtab (link 2), 2 .strtab, 3 the index table
        // (link 1), 4 code (SHF_EXECINSTR), 5 data, 6 a decoy index table
        // linked to section 2.
        let build = |entries: [u32; 3], decoy: [u32; 3]| -> Vec<u8> {
            let mut b = vec![0u8; 64];
            b[..4].copy_from_slice(b"\x7fELF");
            b[4] = 2;
            b[5] = 1;
            let strings = b"\0c\0d\0";
            let mut symbols = vec![0u8; 24 * 3];
            for (k, name) in [(1usize, 1u32), (2, 3)] {
                symbols[24 * k..24 * k + 4].copy_from_slice(&name.to_le_bytes());
                symbols[24 * k + 4] = 1 << 4; // STB_GLOBAL | STT_NOTYPE
                symbols[24 * k + 6..24 * k + 8].copy_from_slice(&SHN_XINDEX.to_le_bytes());
            }
            let symbols_at = b.len();
            b.extend(&symbols);
            let strings_at = b.len();
            b.extend(strings);
            let table_at = b.len();
            for e in entries {
                b.extend(e.to_le_bytes());
            }
            let decoy_at = b.len();
            for e in decoy {
                b.extend(e.to_le_bytes());
            }
            while !b.len().is_multiple_of(8) {
                b.push(0);
            }
            let shoff = b.len();
            let mut header = |kind: u32, flags: u64, offset: usize, size: usize, link: u32| {
                let mut h = vec![0u8; 64];
                h[4..8].copy_from_slice(&kind.to_le_bytes());
                h[0x08..0x10].copy_from_slice(&flags.to_le_bytes());
                h[0x18..0x20].copy_from_slice(&(offset as u64).to_le_bytes());
                h[0x20..0x28].copy_from_slice(&(size as u64).to_le_bytes());
                h[0x28..0x2c].copy_from_slice(&link.to_le_bytes());
                b.extend(h);
            };
            header(0, 0, 0, 0, 0);
            header(SHT_SYMTAB, 0, symbols_at, symbols.len(), 2);
            header(3, 0, strings_at, strings.len(), 0);
            header(SHT_SYMTAB_SHNDX, 0, table_at, 12, 1);
            header(1, 0x2 | SHF_EXECINSTR, 0, 0, 0);
            header(1, 0x2 | 0x1, 0, 0, 0);
            header(SHT_SYMTAB_SHNDX, 0, decoy_at, 12, 2);
            b[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
            b[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
            b[0x3c..0x3e].copy_from_slice(&7u16.to_le_bytes());
            b
        };
        let code = |b: &[u8]| -> Vec<(String, bool)> {
            defined(b)
                .unwrap()
                .into_iter()
                .map(|d| (d.name, d.function))
                .collect()
        };
        let pair = |c: bool, d: bool| vec![("c".to_string(), c), ("d".to_string(), d)];
        // c in the code section, d in the data section.
        assert_eq!(code(&build([0, 4, 5], [0, 5, 4])), pair(true, false));
        // Swapped: c in the data section, d in the code section.
        assert_eq!(code(&build([0, 5, 4], [0, 4, 5])), pair(false, true));
        // An entry past the last section is not code.
        assert_eq!(code(&build([0, 4, 99], [0, 4, 4])), pair(true, false));
    }
}
