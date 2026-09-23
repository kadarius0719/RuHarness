//! The **symbol-set check** (docs/SCHEMAS.md "Trust boundaries"): the
//! candidate staticlib's defined external symbols that are not Rust-mangled,
//! minus a baseline captured from an empty harness-owned crate built with the
//! same toolchain, must equal the unit's `symbols` exactly. A candidate that
//! exports `printf`, `malloc`, or any other extra global — and could thereby
//! shadow what the C side links against and forge a green differential —
//! cannot pass.
//!
//! Subtraction is stricter than a plain set difference. STRONG definitions
//! are subtracted as a **multiset**: a name the baseline defines once and the
//! candidate defines twice is still reported, so a candidate cannot hide an
//! export behind a compiler-builtins name the baseline happens to contain.
//! WEAK definitions are idempotent under linking (ELF emits one
//! `DW.ref.rust_eh_personality` per object that needs it, so their number
//! legitimately varies with the candidate) and are subtracted as a set.
//!
//! The same check also bounds **pre-main constructors**. A candidate can carry
//! a static initializer whose entry lives in a constructor section
//! (`__mod_init_func` / `__mod_term_func` / `__init_offsets` on macOS,
//! `.init_array` / `.fini_array` / `.ctors` / `.dtors` on ELF) — code that runs
//! before `main` and could print forged output and `exit`, invisible to the
//! defined-external symbol check because the initializer is a mangled local.
//! So the constructor sections are counted too, and any excess over the
//! baseline fails the check. On macOS the count comes from `nm -m`; on Linux
//! from `objdump -h` when `objdump` is allowlisted and runnable — otherwise the
//! scan is recorded as unavailable and never fails the check on its own.

use crate::exec::Runner;
use crate::sandbox::{self, HostDirs, ProfileSpec};
use harness_core::error::Error;
use harness_core::verdict::Check;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Name of the check in verdicts.
pub(crate) const CHECK_NAME: &str = "symbol-set";

/// Directory (inside the ledger build dir) holding the baseline crates. A
/// unit may not use this as its id: unit build dirs are siblings of it.
pub(crate) const BASELINE_DIR: &str = "symbol-baseline";

/// First line of a baseline cache file. Bumped to v2 for the constructor
/// section counts; a v1 cache is a miss and is rebuilt.
const CACHE_HEADER: &str = "ruharness-symbol-baseline v2";

/// How many unexpected/missing names a failure detail lists.
const DETAIL_LIMIT: usize = 10;

/// Mach-O sections whose entries are called before `main` / after it returns.
/// `__init_offsets` is the newer linker's replacement for `__mod_init_func`.
const CTOR_SECTIONS_MACHO: [&str; 3] = ["__mod_init_func", "__mod_term_func", "__init_offsets"];

/// ELF sections whose entries are called before `main` / after it returns.
/// Matched by prefix, so priority-suffixed forms (`.init_array.00100`,
/// `.ctors.65534`) are caught, while `.rela.init_array` (relocations) is not.
const CTOR_SECTIONS_ELF: [&str; 4] = [".init_array", ".fini_array", ".ctors", ".dtors"];

/// The message recorded in the check detail when no constructor-section
/// scanner is available on this platform (Linux without a runnable objdump).
const SCAN_UNAVAILABLE: &str = "constructor-section scan unavailable on this platform";

/// The normalized, non-Rust-mangled defined external symbols of a staticlib.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SymbolTable {
    /// Strong definitions with their multiplicity across archive members.
    pub strong: BTreeMap<String, usize>,
    /// Weak definitions (nm types `W`/`V`/`w`/`v`/`u`).
    pub weak: BTreeSet<String>,
}

impl SymbolTable {
    fn add(&mut self, weak: bool, name: &str) {
        if weak {
            self.weak.insert(name.to_string());
        } else {
            *self.strong.entry(name.to_string()).or_insert(0) += 1;
        }
    }

    /// True when `name` is defined at all, strongly or weakly.
    #[cfg(test)]
    pub(crate) fn defines(&self, name: &str) -> bool {
        self.strong.contains_key(name) || self.weak.contains(name)
    }
}

/// The result of scanning a staticlib for constructor sections. `counts` maps
/// each constructor section name to how many entries fell in it; on macOS
/// `names` additionally records the symbols seen there (for the failure
/// detail). `available` is false only when no scanner could run on this
/// platform, in which case the check must not fail for that reason alone —
/// the [`Default`] value is exactly this "no scanner ran" state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CtorScan {
    /// Constructor section name -> entry count.
    pub counts: BTreeMap<String, usize>,
    /// Constructor section name -> symbol names seen there (macOS only).
    pub names: BTreeMap<String, BTreeSet<String>>,
    /// False when no scanner was available (Linux without a runnable objdump).
    pub available: bool,
}

impl CtorScan {
    /// An available scan with no constructor sections at all.
    fn empty_available() -> CtorScan {
        CtorScan {
            available: true,
            ..CtorScan::default()
        }
    }
}

/// The empty-crate baseline the symbol-set check subtracts: the toolchain's
/// own defined externals plus its constructor-section counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Baseline {
    /// Non-Rust-mangled defined external symbols of the empty crate.
    pub symbols: SymbolTable,
    /// Constructor-section counts of the empty crate.
    pub ctors: CtorScan,
}

/// One `nm` line: a defined external symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NmSymbol {
    /// Weak definition (`W`/`V`/`w`/`v`/`u`) rather than a strong one.
    pub weak: bool,
    /// The raw name exactly as nm printed it.
    pub name: String,
}

/// Everything the check needs from the verify run in progress.
pub(crate) struct SymbolCtx<'a> {
    /// Child-process runner (allowlist, timeout, cwd).
    pub runner: &'a Runner,
    /// Host dirs for the baseline build's sandbox profile (`None` = the
    /// sandbox is unavailable and the build runs unsandboxed).
    pub host: Option<&'a HostDirs>,
    /// Canonical target root.
    pub root: &'a Path,
    /// Canonical ledger build dir (`<root>/migration/build`).
    pub build_root: &'a Path,
    /// First line of `rustc -V` — the baseline cache key.
    pub rustc_version: &'a str,
}

/// Run the symbol-set check for `staticlib` against the unit's `expected`
/// symbols. `Err` means the harness could not perform the check (nm or the
/// baseline build failed); a mismatch — an unexpected/missing symbol OR a
/// pre-main constructor section beyond the baseline — is a failed [`Check`].
pub(crate) fn symbol_set_check(
    ctx: &SymbolCtx<'_>,
    staticlib: &Path,
    panic_abort: bool,
    expected: &[String],
) -> Result<Check, Error> {
    let baseline = baseline_table(ctx, panic_abort)?;
    let candidate = defined_unmangled(ctx.runner, staticlib)?;
    let candidate_ctors = scan_ctor_sections(ctx.runner, staticlib)?;
    Ok(combine(
        compare(&candidate, &baseline.symbols, expected),
        ctor_finding(&candidate_ctors, &baseline.ctors),
    ))
}

/// `nm` argv listing DEFINED EXTERNAL symbols of `lib`.
pub(crate) fn nm_argv(lib: &str) -> Vec<String> {
    let flags: &[&str] = if cfg!(target_os = "macos") {
        &["-gU"]
    } else {
        &["-g", "--defined-only"]
    };
    let mut argv = vec!["nm".to_string()];
    argv.extend(flags.iter().map(|f| (*f).to_string()));
    argv.push(lib.to_string());
    argv
}

/// Run nm on `lib` and tabulate its normalized, non-Rust-mangled symbols.
fn defined_unmangled(runner: &Runner, lib: &Path) -> Result<SymbolTable, Error> {
    let lib_str = lib
        .to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", lib.display())))?;
    let stdout = runner.tool(&nm_argv(lib_str))?;
    Ok(tabulate_unmangled(
        &String::from_utf8_lossy(&stdout),
        cfg!(target_os = "macos"),
    ))
}

/// `nm -m` argv (Mach-O only): the verbose listing whose per-symbol
/// `(segment,section)` column locates constructor entries.
pub(crate) fn nm_m_argv(lib: &str) -> Vec<String> {
    vec!["nm".to_string(), "-m".to_string(), lib.to_string()]
}

/// `objdump -h` argv (ELF): the section-header table used to count
/// constructor sections when `objdump` is allowlisted and runnable.
pub(crate) fn objdump_h_argv(lib: &str) -> Vec<String> {
    vec!["objdump".to_string(), "-h".to_string(), lib.to_string()]
}

/// Scan `lib` for constructor sections. On macOS this always runs (`nm` is a
/// required tool); on Linux it uses `objdump -h` when `objdump` is allowlisted
/// and runnable, otherwise it returns an unavailable scan (which never fails
/// the check on its own). `Err` only when a scanner that should have run
/// (macOS `nm -m`) failed outright.
pub(crate) fn scan_ctor_sections(runner: &Runner, lib: &Path) -> Result<CtorScan, Error> {
    let lib_str = lib
        .to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", lib.display())))?;
    if cfg!(target_os = "macos") {
        let stdout = runner.tool(&nm_m_argv(lib_str))?;
        Ok(ctor_scan_from_macho(&String::from_utf8_lossy(&stdout)))
    } else {
        // ELF: nm lists no per-entry constructor symbols, so counting needs
        // objdump. Only run it when the target opted it onto the allowlist,
        // and treat a spawn/run failure as "unavailable" rather than an error.
        if !runner.allowlist.iter().any(|a| a == "objdump") {
            return Ok(CtorScan::default());
        }
        match runner.tool(&objdump_h_argv(lib_str)) {
            Ok(stdout) => Ok(ctor_scan_from_objdump(&String::from_utf8_lossy(&stdout))),
            Err(_) => Ok(CtorScan::default()),
        }
    }
}

/// Build a [`CtorScan`] from `nm -m` output.
pub(crate) fn ctor_scan_from_macho(nm_m_output: &str) -> CtorScan {
    let mut scan = CtorScan::empty_available();
    for (section, symbol) in parse_ctor_sections_macho(nm_m_output) {
        *scan.counts.entry(section.clone()).or_insert(0) += 1;
        scan.names.entry(section).or_default().insert(symbol);
    }
    scan
}

/// Build a [`CtorScan`] from `objdump -h` output.
pub(crate) fn ctor_scan_from_objdump(objdump_h_output: &str) -> CtorScan {
    CtorScan {
        counts: parse_ctor_sections_elf(objdump_h_output),
        names: BTreeMap::new(),
        available: true,
    }
}

/// `(section, symbol)` for every symbol `nm -m` places in a constructor
/// section, in ANY segment. A line reads
/// `<addr> (<segment>,<section>) <attrs…> <name>`; the section is the field
/// after the comma and the symbol is the last whitespace token.
pub(crate) fn parse_ctor_sections_macho(nm_m_output: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in nm_m_output.lines() {
        let Some(open) = line.find('(') else { continue };
        let Some(rel_close) = line[open..].find(')') else {
            continue;
        };
        let inside = &line[open + 1..open + rel_close];
        let Some((_segment, section)) = inside.split_once(',') else {
            continue;
        };
        if CTOR_SECTIONS_MACHO.contains(&section) {
            if let Some(name) = line.split_whitespace().last() {
                out.push((section.to_string(), name.to_string()));
            }
        }
    }
    out
}

/// Section-name -> occurrence count for every constructor section in
/// `objdump -h` output. A header row reads `<idx> <name> <size> …` with a
/// numeric index first; a name is counted when it starts with one of the ELF
/// constructor section prefixes (so `.rela.init_array` is excluded).
pub(crate) fn parse_ctor_sections_elf(objdump_h_output: &str) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in objdump_h_output.lines() {
        let mut fields = line.split_whitespace();
        let Some(idx) = fields.next() else { continue };
        if idx.is_empty() || !idx.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Some(name) = fields.next() else { continue };
        if let Some(prefix) = CTOR_SECTIONS_ELF
            .iter()
            .find(|p| name == **p || name.starts_with(&format!("{p}.")))
        {
            *counts.entry((*prefix).to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// The constructor-section portion of the symbol-set check: how the
/// candidate's counts compare with the baseline's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CtorFinding {
    /// True when the candidate has more constructor entries than the baseline.
    failed: bool,
    /// Text to fold into the check detail (an excess, or the unavailable note).
    detail: Option<String>,
}

/// Compare candidate vs baseline constructor sections. Any section the
/// candidate populates beyond the baseline's count is a failure whose detail
/// names the section (and, on macOS, the offending symbols). When either scan
/// was unavailable the finding is a note, never a failure.
pub(crate) fn ctor_finding(candidate: &CtorScan, baseline: &CtorScan) -> CtorFinding {
    if !candidate.available || !baseline.available {
        return CtorFinding {
            failed: false,
            detail: Some(SCAN_UNAVAILABLE.to_string()),
        };
    }
    let mut offending: Vec<String> = Vec::new();
    for (section, count) in &candidate.counts {
        let base = baseline.counts.get(section).copied().unwrap_or(0);
        if *count > base {
            let listed = match candidate.names.get(section) {
                Some(names) if !names.is_empty() => {
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    format!("{section} ({})", list_limited(&refs))
                }
                _ => format!("{section} ({} beyond baseline)", count - base),
            };
            offending.push(listed);
        }
    }
    if offending.is_empty() {
        CtorFinding {
            failed: false,
            detail: None,
        }
    } else {
        CtorFinding {
            failed: true,
            detail: Some(format!(
                "pre-main constructor sections beyond the baseline — {}",
                offending.join("; ")
            )),
        }
    }
}

/// Fold the constructor finding into the symbol comparison's [`Check`]. The
/// check passes only when both pass; an unavailable-scan note is appended to a
/// still-green detail so the verdict records that the scan did not run.
pub(crate) fn combine(symbols: Check, ctors: CtorFinding) -> Check {
    let passed = symbols.passed && !ctors.failed;
    let detail = match ctors.detail {
        Some(extra) => format!("{}; {extra}", symbols.detail),
        None => symbols.detail,
    };
    Check {
        name: CHECK_NAME.into(),
        passed,
        detail,
    }
}

/// Parse nm output into the table of normalized unmangled names.
pub(crate) fn tabulate_unmangled(nm_output: &str, macos: bool) -> SymbolTable {
    let mut table = SymbolTable::default();
    for symbol in parse_nm(nm_output) {
        let name = normalize(&symbol.name, macos);
        if !is_rust_mangled(name) {
            table.add(symbol.weak, name);
        }
    }
    table
}

/// Defined symbols from `nm` output: lines shaped `<address> <type> <name>`.
/// Archive member headers (`foo.o:`), blank lines and anything else without
/// an address and a type are skipped. The address column is hex, or dashes
/// (llvm-nm prints `----------------` for some weak definitions). The name is
/// EVERYTHING after the type column, so an export name containing spaces
/// stays one (unexpected) symbol instead of being truncated into an innocent
/// looking one.
pub(crate) fn parse_nm(nm_output: &str) -> Vec<NmSymbol> {
    let mut symbols = Vec::new();
    for line in nm_output.lines() {
        let line = line.trim();
        let Some((addr, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let addr_ok = addr.len() >= 8
            && (addr.chars().all(|c| c.is_ascii_hexdigit()) || addr.chars().all(|c| c == '-'));
        if !addr_ok {
            continue;
        }
        let rest = rest.trim_start();
        let Some((ty, name)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let mut ty_chars = ty.chars();
        let (Some(t), None) = (ty_chars.next(), ty_chars.next()) else {
            continue;
        };
        if !(t.is_ascii_alphabetic() || t == '-' || t == '?') || t == 'U' {
            continue;
        }
        let name = name.trim();
        if !name.is_empty() {
            symbols.push(NmSymbol {
                weak: matches!(t, 'W' | 'w' | 'V' | 'v' | 'u'),
                name: name.to_string(),
            });
        }
    }
    symbols
}

/// Strip the one leading underscore Mach-O prepends to every C-level name.
/// A raw name without it (possible via `#[export_name = "\x01…"]`) is kept
/// verbatim — it can only ever show up as an unexpected symbol.
pub(crate) fn normalize(raw: &str, macos: bool) -> &str {
    if macos {
        raw.strip_prefix('_').unwrap_or(raw)
    } else {
        raw
    }
}

/// True for Rust-mangled names, tested on the NORMALIZED form (so `_ZN…` /
/// `_R…` on every platform; the raw Mach-O spellings are `__ZN…` / `__R…`).
///
/// Deliberately shape-checked rather than prefix-checked, so that a candidate
/// cannot smuggle an arbitrary export past the check by starting its
/// `#[export_name]` with `_R` or `_ZN`:
/// - legacy: `_ZN…17h<16 hex>E`, optionally followed by an LLVM `.suffix`;
/// - v0: `_R`, optional decimal version, a path tag (`C M X Y N I B`), then
///   only `[A-Za-z0-9_]`, optionally followed by a `.`/`$` vendor suffix.
pub(crate) fn is_rust_mangled(name: &str) -> bool {
    let ident = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if let Some(body) = name.strip_prefix("_ZN") {
        // Legacy symbols escape punctuation as `$LT$`, `..` etc.
        let core = body.split_once(".llvm.").map_or(body, |(core, _)| core);
        let Some(inner) = core.strip_suffix('E') else {
            return false;
        };
        // The trailing path element is always the hash: `17h` + 16 hex.
        const HASH_LEN: usize = 3 + 16;
        if inner.len() <= HASH_LEN || !inner.is_char_boundary(inner.len() - HASH_LEN) {
            return false;
        }
        let (path, hash) = inner.split_at(inner.len() - HASH_LEN);
        let hash_ok = hash.strip_prefix("17h").is_some_and(|h| {
            h.len() == 16
                && h.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        });
        return hash_ok
            && !path.is_empty()
            && path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'));
    }
    if let Some(body) = name.strip_prefix("_R") {
        let core = body.split(['.', '$']).next().unwrap_or(body);
        let path = core.trim_start_matches(|c: char| c.is_ascii_digit());
        return path
            .chars()
            .next()
            .is_some_and(|tag| "CMXYNIB".contains(tag))
            && path.len() > 1
            && ident(path);
    }
    false
}

/// Compare `(candidate − baseline)` with the unit's symbols. A candidate
/// name survives the subtraction when it is strongly defined more often than
/// in the baseline, or weakly defined and unknown to the baseline.
pub(crate) fn compare(
    candidate: &SymbolTable,
    baseline: &SymbolTable,
    expected: &[String],
) -> Check {
    let strong = candidate
        .strong
        .iter()
        .filter(|(name, count)| **count > baseline.strong.get(*name).copied().unwrap_or(0))
        .map(|(name, _)| name.as_str());
    let weak = candidate
        .weak
        .iter()
        .filter(|name| !baseline.weak.contains(*name) && !baseline.strong.contains_key(*name))
        .map(String::as_str);
    let exported: BTreeSet<&str> = strong.chain(weak).collect();
    let expected: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let unexpected: Vec<&str> = exported.difference(&expected).copied().collect();
    let missing: Vec<&str> = expected.difference(&exported).copied().collect();
    if unexpected.is_empty() && missing.is_empty() {
        return Check {
            name: CHECK_NAME.into(),
            passed: true,
            detail: format!(
                "{} exported symbol(s) match the unit's symbols exactly",
                expected.len()
            ),
        };
    }
    let mut parts: Vec<String> = Vec::new();
    if !unexpected.is_empty() {
        parts.push(format!("unexpected: {}", list_limited(&unexpected)));
    }
    if !missing.is_empty() {
        parts.push(format!("missing: {}", list_limited(&missing)));
    }
    Check {
        name: CHECK_NAME.into(),
        passed: false,
        detail: format!(
            "staticlib exports differ from the unit's symbols — {}",
            parts.join("; ")
        ),
    }
}

fn list_limited(names: &[&str]) -> String {
    let mut text = names
        .iter()
        .take(DETAIL_LIMIT)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > DETAIL_LIMIT {
        text.push_str(&format!(" (+{} more)", names.len() - DETAIL_LIMIT));
    }
    text
}

/// True when the crate's `Cargo.toml` contains the literal line
/// `panic = "abort"` (whitespace and trailing comments tolerated). The
/// baseline is built with the same panic strategy, because the strategy
/// decides which runtime pieces (and their unmangled symbols) are linked in.
pub(crate) fn manifest_sets_panic_abort(cargo_toml: &str) -> bool {
    cargo_toml.lines().any(|line| {
        let code = line.split('#').next().unwrap_or("");
        let squeezed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        squeezed == "panic=\"abort\""
    })
}

/// `Cargo.toml` of the empty, harness-owned baseline crate.
fn baseline_manifest(panic_abort: bool) -> String {
    let mut text = String::from(
        "# Generated by RuHarness: the empty crate the symbol-set check subtracts.\n\
         [package]\n\
         name = \"ruharness_symbol_baseline\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         publish = false\n\
         \n\
         [lib]\n\
         crate-type = [\"staticlib\"]\n\
         \n\
         [workspace]\n",
    );
    if panic_abort {
        text.push_str("\n[profile.release]\npanic = \"abort\"\n");
    }
    text
}

/// The baseline for the current toolchain and panic strategy: read from the
/// cache file next to the baseline crate when its recorded `rustc -V` matches,
/// otherwise rebuilt (and re-cached).
///
/// The cache lives in `<build>/symbol-baseline/<strategy>/symbols.txt`. No
/// sandboxed child that runs candidate code can write there: built binaries
/// may only write to temp, and unit build dirs are siblings (the id
/// `symbol-baseline` itself is refused).
pub(crate) fn baseline_table(ctx: &SymbolCtx<'_>, panic_abort: bool) -> Result<Baseline, Error> {
    let strategy = if panic_abort { "abort" } else { "unwind" };
    let dir = ctx.build_root.join(BASELINE_DIR).join(strategy);
    let cache = dir.join("symbols.txt");
    if let Some(baseline) = read_cache(&cache, ctx.rustc_version) {
        return Ok(baseline);
    }

    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| Error::io(&src, e))?;
    let dir = dir.canonicalize().map_err(|e| Error::io(&dir, e))?;
    if !dir.starts_with(ctx.build_root) {
        return Err(Error::InvalidPlan(format!(
            "symbol baseline dir {} escapes the ledger build dir {}",
            dir.display(),
            ctx.build_root.display()
        )));
    }
    write(&dir.join("Cargo.toml"), &baseline_manifest(panic_abort))?;
    write(
        &dir.join("src/lib.rs"),
        "//! Empty on purpose: the symbol-set baseline (generated by RuHarness).\n",
    )?;
    let target_dir = crate::prepare_target_dir(&dir)?;
    let profile = match ctx.host {
        Some(host) => Some(sandbox::render_profile(&ProfileSpec {
            host,
            target_root: ctx.root,
            toolchain: true,
            write_dirs: std::slice::from_ref(&target_dir),
            write_files: &[dir.join("Cargo.lock")],
        })?),
        None => None,
    };
    let lib = crate::build_staticlib(ctx.runner, profile.as_deref(), &dir, &target_dir)
        .map_err(|e| Error::Invariant(format!("building the symbol-set baseline crate: {e}")))?;
    let baseline = Baseline {
        symbols: defined_unmangled(ctx.runner, &lib)?,
        ctors: scan_ctor_sections(ctx.runner, &lib)?,
    };

    // One line per definition: `S <name>` (strong, repeated per occurrence)
    // or `W <name>` (weak). Then the constructor-section block: `A 1|0` for
    // whether the scan ran, and `I <section> <count>` per populated section.
    let mut text = format!("{CACHE_HEADER}\n{}\n", ctx.rustc_version);
    for (name, count) in &baseline.symbols.strong {
        for _ in 0..*count {
            text.push_str(&format!("S {name}\n"));
        }
    }
    for name in &baseline.symbols.weak {
        text.push_str(&format!("W {name}\n"));
    }
    text.push_str(&format!("A {}\n", u8::from(baseline.ctors.available)));
    for (section, count) in &baseline.ctors.counts {
        text.push_str(&format!("I {section} {count}\n"));
    }
    harness_core::ledger::write_atomic(&dir.join("symbols.txt"), text.as_bytes())?;
    Ok(baseline)
}

/// A cache hit requires the exact header, the exact `rustc -V` line, the
/// constructor-scan availability marker, and nothing but well-formed entries;
/// anything else is a miss (and a rebuild).
fn read_cache(path: &Path, rustc_version: &str) -> Option<Baseline> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != CACHE_HEADER || lines.next()? != rustc_version {
        return None;
    }
    let mut baseline = Baseline::default();
    let mut saw_availability = false;
    for line in lines.filter(|l| !l.is_empty()) {
        match line.split_once(' ')? {
            ("S", name) => baseline.symbols.add(false, name),
            ("W", name) => baseline.symbols.add(true, name),
            ("A", "1") => {
                baseline.ctors.available = true;
                saw_availability = true;
            }
            ("A", "0") => {
                baseline.ctors.available = false;
                saw_availability = true;
            }
            ("I", rest) => {
                let (section, count) = rest.split_once(' ')?;
                let count: usize = count.parse().ok()?;
                baseline.ctors.counts.insert(section.to_string(), count);
            }
            _ => return None,
        }
    }
    // A v2 cache always records availability; its absence means a truncated or
    // hand-mangled file — treat it as a miss rather than a silent default.
    saw_availability.then_some(baseline)
}

fn write(path: &Path, text: &str) -> Result<(), Error> {
    std::fs::write(path, text).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{fixture_crate, TempDir, ToolBench};
    use std::path::PathBuf;

    fn baseline_cache_path(build_root: &Path, panic_abort: bool) -> PathBuf {
        build_root
            .join(BASELINE_DIR)
            .join(if panic_abort { "abort" } else { "unwind" })
            .join("symbols.txt")
    }

    /// A table of strong definitions (repeat a name for multiplicity).
    fn counts(names: &[&str]) -> SymbolTable {
        let mut t = SymbolTable::default();
        for n in names {
            t.add(false, n);
        }
        t
    }

    fn with_weak(mut table: SymbolTable, names: &[&str]) -> SymbolTable {
        for n in names {
            table.add(true, n);
        }
        table
    }

    const MACOS_NM: &str = "\n\
katajainen_rs-d4e5.katajainen_rs.cgu.0.rcgu.o:\n\
\n\
katajainen_rs-d4e5.katajainen_rs.cgu.1.rcgu.o:\n\
0000000000000000 T _ZopfliLengthLimitedCodeLengths\n\
0000000000000040 T __ZN4core3fmt5write17h0123456789abcdefE\n\
0000000000000080 T __RNvCs1234_7mycrate3foo\n\
\n\
std-8676.std.cgu.0.rcgu.o:\n\
---------------- W ___isOSVersionAtLeast\n\
---------------- T _rust_eh_personality\n\
                 U _malloc\n\
0000000000000100 S _spaced name\n";

    const LINUX_NM: &str = "\n\
katajainen_rs-d4e5.katajainen_rs.cgu.1.rcgu.o:\n\
0000000000000000 T ZopfliLengthLimitedCodeLengths\n\
0000000000000000 T _ZN4core3fmt5write17h0123456789abcdefE\n\
0000000000000000 W _RNvCs1234_7mycrate3foo\n\
0000000000000000 T rust_eh_personality\n\
0000000000000000 V DW.ref.rust_eh_personality\n\
                 U malloc\n";

    #[test]
    fn nm_lines_need_an_address_and_a_type() {
        let names: Vec<(bool, String)> = parse_nm(MACOS_NM)
            .into_iter()
            .map(|s| (s.weak, s.name))
            .collect();
        let expect = |weak: bool, name: &str| (weak, name.to_string());
        assert_eq!(
            names,
            vec![
                expect(false, "_ZopfliLengthLimitedCodeLengths"),
                expect(false, "__ZN4core3fmt5write17h0123456789abcdefE"),
                expect(false, "__RNvCs1234_7mycrate3foo"),
                expect(true, "___isOSVersionAtLeast"),
                expect(false, "_rust_eh_personality"),
                expect(false, "_spaced name"),
            ]
        );
        assert!(parse_nm("foo.o:\n\nlib.a(bar.o):\nnm: no symbols\n").is_empty());
        // Undefined entries never count, whatever the flags let through.
        assert!(parse_nm("0000000000000000 U _malloc\n").is_empty());
    }

    #[test]
    fn both_platform_spellings_normalize_to_the_same_counts() {
        let mac = tabulate_unmangled(MACOS_NM, true);
        assert_eq!(
            mac,
            with_weak(
                counts(&[
                    "ZopfliLengthLimitedCodeLengths",
                    "rust_eh_personality",
                    "spaced name",
                ]),
                &["__isOSVersionAtLeast"]
            )
        );
        let linux = tabulate_unmangled(LINUX_NM, false);
        assert_eq!(
            linux,
            with_weak(
                counts(&["ZopfliLengthLimitedCodeLengths", "rust_eh_personality"]),
                &["DW.ref.rust_eh_personality"]
            )
        );
    }

    #[test]
    fn nm_flags_follow_the_platform() {
        let argv = nm_argv("/x/lib.a");
        assert_eq!(argv.first().map(String::as_str), Some("nm"));
        assert_eq!(argv.last().map(String::as_str), Some("/x/lib.a"));
        if cfg!(target_os = "macos") {
            assert_eq!(argv[1..argv.len() - 1], ["-gU".to_string()]);
        } else {
            assert_eq!(
                argv[1..argv.len() - 1],
                ["-g".to_string(), "--defined-only".to_string()]
            );
        }
    }

    #[test]
    fn mangled_names_are_recognized_by_shape_not_prefix() {
        for mangled in [
            "_ZN4core3fmt5write17h0123456789abcdefE",
            "_ZN4core3fmt5write17h0123456789abcdefE.llvm.1234567890",
            "_ZN55_$LT$X$u20$as$u20$core..fmt..Debug$GT$3fmt17hdeadbeefdeadbeefE",
            "_RNvCs1234_7mycrate3foo",
            "_RNvNtCs1234_7mycrate3bar3baz.llvm.99",
            "_R0NvCs1234_7mycrate3foo",
            "_RINvCs1_1a1bpE",
        ] {
            assert!(is_rust_mangled(mangled), "{mangled} must count as mangled");
        }
        for plain in [
            "printf",
            "malloc",
            "ZopfliLengthLimitedCodeLengths",
            "rust_eh_personality",
            "__rust_alloc",
            // Prefix lookalikes a hostile #[export_name] might try:
            "_Revil",
            "_Read",
            "_R",
            "_RN",
            "_Rprintf",
            "_ZNevil",
            "_ZN6printfE",
            "_ZN6printf17hXYZ3456789abcdefE",
            "_ZN17h0123456789abcdefE",
            // Un-normalized spellings are not accepted either.
            "ZN4core3fmt5write17h0123456789abcdefE",
            "RNvCs1234_7mycrate3foo",
            "__ZN4core3fmt5write17h0123456789abcdefE",
            "_RNv with space",
        ] {
            assert!(!is_rust_mangled(plain), "{plain} must NOT count as mangled");
        }
    }

    #[test]
    fn compare_passes_only_on_exact_equality() {
        let base = counts(&["rust_eh_personality", "__udivti3"]);
        let cand = counts(&["rust_eh_personality", "__udivti3", "Foo", "Bar"]);
        let ok = compare(&cand, &base, &["Foo".into(), "Bar".into()]);
        assert!(ok.passed, "{}", ok.detail);
        assert_eq!(ok.name, "symbol-set");
        assert_eq!(
            ok.detail,
            "2 exported symbol(s) match the unit's symbols exactly"
        );

        let extra = compare(&cand, &base, &["Foo".into()]);
        assert!(!extra.passed);
        assert!(extra.detail.contains("unexpected: Bar"), "{}", extra.detail);
        assert!(!extra.detail.contains("missing"), "{}", extra.detail);

        let missing = compare(&cand, &base, &["Foo".into(), "Bar".into(), "Baz".into()]);
        assert!(!missing.passed);
        assert!(
            missing.detail.contains("missing: Baz"),
            "{}",
            missing.detail
        );

        let nothing_expected = compare(&base, &base, &[]);
        assert!(nothing_expected.passed, "{}", nothing_expected.detail);
    }

    #[test]
    fn a_second_definition_of_a_baseline_name_is_still_unexpected() {
        let base = counts(&["memcpy", "rust_eh_personality"]);
        let cand = counts(&["memcpy", "memcpy", "rust_eh_personality", "Foo"]);
        let check = compare(&cand, &base, &["Foo".into()]);
        assert!(!check.passed);
        assert!(
            check.detail.contains("unexpected: memcpy"),
            "{}",
            check.detail
        );
    }

    #[test]
    fn weak_definitions_subtract_as_a_set() {
        // ELF: one weak DW.ref.rust_eh_personality per object that unwinds —
        // the candidate's own object legitimately adds another.
        let base = with_weak(
            counts(&["rust_eh_personality"]),
            &["DW.ref.rust_eh_personality"],
        );
        let cand = with_weak(
            counts(&["rust_eh_personality", "Foo"]),
            &["DW.ref.rust_eh_personality"],
        );
        let check = compare(&cand, &base, &["Foo".into()]);
        assert!(check.passed, "{}", check.detail);

        // A weak name the baseline has never heard of is still an export …
        let cand = with_weak(counts(&["rust_eh_personality", "Foo"]), &["sneaky_weak"]);
        let check = compare(&cand, &base, &["Foo".into()]);
        assert!(
            check.detail.contains("unexpected: sneaky_weak"),
            "{}",
            check.detail
        );

        // … and a STRONG definition of a name the baseline only defines
        // weakly is an override, not a duplicate.
        let cand = counts(&["rust_eh_personality", "Foo", "DW.ref.rust_eh_personality"]);
        let check = compare(&cand, &base, &["Foo".into()]);
        assert!(
            check
                .detail
                .contains("unexpected: DW.ref.rust_eh_personality"),
            "{}",
            check.detail
        );

        // The unit's own symbol may be weak or strong.
        let cand = with_weak(counts(&["rust_eh_personality"]), &["Foo"]);
        assert!(compare(&cand, &base, &["Foo".into()]).passed);
    }

    #[test]
    fn failure_detail_lists_at_most_ten_names_per_side() {
        let names: Vec<String> = (0..14).map(|i| format!("sym{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let check = compare(&counts(&refs), &SymbolTable::default(), &[]);
        assert!(!check.passed);
        assert!(check.detail.contains("sym09"), "{}", check.detail);
        assert!(!check.detail.contains("sym10"), "{}", check.detail);
        assert!(check.detail.contains("(+4 more)"), "{}", check.detail);

        let check = compare(&SymbolTable::default(), &SymbolTable::default(), &names);
        assert!(check.detail.contains("missing: sym00"), "{}", check.detail);
        assert!(check.detail.contains("(+4 more)"), "{}", check.detail);
    }

    #[test]
    fn panic_abort_is_detected_from_the_literal_manifest_line() {
        assert!(manifest_sets_panic_abort(
            "[profile.release]\npanic = \"abort\"\n"
        ));
        assert!(manifest_sets_panic_abort(
            "[profile.release]\n  panic=\"abort\"   # harness-owned\n"
        ));
        assert!(!manifest_sets_panic_abort(
            "[profile.release]\npanic = \"unwind\"\n"
        ));
        assert!(!manifest_sets_panic_abort("# panic = \"abort\"\n"));
        assert!(!manifest_sets_panic_abort("[package]\nname = \"x\"\n"));
        assert!(baseline_manifest(true).contains("panic = \"abort\""));
        assert!(!baseline_manifest(false).contains("panic"));
        assert!(manifest_sets_panic_abort(&baseline_manifest(true)));
    }

    #[test]
    fn cache_requires_matching_header_and_rustc_version() {
        let tmp = TempDir::new("symcache");
        let path = tmp.path().join("symbols.txt");
        std::fs::write(
            &path,
            format!(
                "{CACHE_HEADER}\nrustc 1.0.0\nS foo\nS foo\nS bar\nW baz\nA 1\nI __mod_init_func 2\n"
            ),
        )
        .expect("write cache");
        let hit = read_cache(&path, "rustc 1.0.0").expect("hit");
        assert_eq!(
            hit.symbols,
            with_weak(counts(&["foo", "foo", "bar"]), &["baz"])
        );
        assert!(hit.ctors.available);
        assert_eq!(hit.ctors.counts.get("__mod_init_func"), Some(&2));
        assert!(read_cache(&path, "rustc 2.0.0").is_none());
        std::fs::write(&path, "something else\nrustc 1.0.0\nS foo\nA 1\n").expect("write cache");
        assert!(read_cache(&path, "rustc 1.0.0").is_none());
        // Malformed entries are a miss, never a partial baseline.
        std::fs::write(
            &path,
            format!("{CACHE_HEADER}\nrustc 1.0.0\nS foo\nbogus\nA 1\n"),
        )
        .expect("write cache");
        assert!(read_cache(&path, "rustc 1.0.0").is_none());
        // A v1-shaped cache with no availability marker is a miss (rebuild).
        std::fs::write(&path, format!("{CACHE_HEADER}\nrustc 1.0.0\nS foo\n"))
            .expect("write cache");
        assert!(read_cache(&path, "rustc 1.0.0").is_none());
        // The previous header version is unconditionally a miss.
        std::fs::write(&path, "ruharness-symbol-baseline v1\nrustc 1.0.0\nS foo\n")
            .expect("write cache");
        assert!(read_cache(&path, "rustc 1.0.0").is_none());
        assert!(read_cache(&tmp.path().join("absent.txt"), "rustc 1.0.0").is_none());
    }

    /// Real toolchain, tiny fixture crates: the matching crate passes; an
    /// extra `#[no_mangle]` export and a `printf` shadow both fail.
    #[test]
    fn fixture_crates_pass_and_fail_the_real_check() {
        let bench = ToolBench::new("symfix");
        let ctx = bench.symbol_ctx();

        let good = fixture_crate(
            bench.root(),
            "good_rs",
            false,
            "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n",
        );
        let lib = bench.build(&good);
        let check =
            symbol_set_check(&ctx, &lib, false, &["unit_add".to_string()]).expect("check runs");
        assert!(check.passed, "{}", check.detail);

        // The same lib against a plan that owns a symbol it does not define.
        let check = symbol_set_check(
            &ctx,
            &lib,
            false,
            &["unit_add".to_string(), "unit_sub".to_string()],
        )
        .expect("check runs");
        assert!(!check.passed);
        assert!(
            check.detail.contains("missing: unit_sub"),
            "{}",
            check.detail
        );

        let extra = fixture_crate(
            bench.root(),
            "extra_rs",
            false,
            "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n\
             #[no_mangle]\npub extern \"C\" fn sneaky_helper() -> i32 { 7 }\n",
        );
        let lib = bench.build(&extra);
        let check =
            symbol_set_check(&ctx, &lib, false, &["unit_add".to_string()]).expect("check runs");
        assert!(!check.passed);
        assert!(
            check.detail.contains("unexpected: sneaky_helper"),
            "{}",
            check.detail
        );

        let shadow = fixture_crate(
            bench.root(),
            "shadow_rs",
            false,
            "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n\
             #[export_name = \"printf\"]\npub extern \"C\" fn forged(_fmt: *const u8) -> i32 { 0 }\n\
             #[export_name = \"malloc\"]\npub extern \"C\" fn forged_alloc(_n: usize) -> *mut u8 { core::ptr::null_mut() }\n",
        );
        let lib = bench.build(&shadow);
        let check =
            symbol_set_check(&ctx, &lib, false, &["unit_add".to_string()]).expect("check runs");
        assert!(!check.passed);
        assert!(
            check.detail.contains("unexpected: malloc, printf"),
            "{}",
            check.detail
        );
    }

    /// `panic = "abort"` candidates are compared with an abort baseline,
    /// cached separately from the unwind one.
    #[test]
    fn panic_abort_candidates_use_their_own_baseline() {
        let bench = ToolBench::new("symabort");
        let ctx = bench.symbol_ctx();
        let krate = fixture_crate(
            bench.root(),
            "abort_rs",
            true,
            "#[no_mangle]\npub extern \"C\" fn unit_neg(a: i32) -> i32 { a.wrapping_neg() }\n",
        );
        let manifest = std::fs::read_to_string(krate.join("Cargo.toml")).expect("manifest");
        assert!(manifest_sets_panic_abort(&manifest));
        let lib = bench.build(&krate);
        let check =
            symbol_set_check(&ctx, &lib, true, &["unit_neg".to_string()]).expect("check runs");
        assert!(check.passed, "{}", check.detail);
        assert!(baseline_cache_path(bench.build_root(), true).exists());
        assert!(!baseline_cache_path(bench.build_root(), false).exists());
    }

    /// The baseline is built once per toolchain: a second call reads the
    /// cache file, and a cache recorded for another `rustc -V` is rebuilt.
    #[test]
    fn baseline_is_cached_by_rustc_version() {
        let bench = ToolBench::new("symbase");
        let ctx = bench.symbol_ctx();
        let first = baseline_table(&ctx, false).expect("baseline builds");
        assert!(first.symbols.defines("rust_eh_personality"), "{first:?}");
        assert!(!first
            .symbols
            .strong
            .keys()
            .chain(first.symbols.weak.iter())
            .any(|k| is_rust_mangled(k)));
        // The empty baseline crate carries no pre-main constructors, and on a
        // platform with a scanner (macOS `nm -m`) the scan is available.
        if cfg!(target_os = "macos") {
            assert!(first.ctors.available);
            assert!(first.ctors.counts.is_empty(), "{:?}", first.ctors);
        }

        let cache = baseline_cache_path(bench.build_root(), false);
        let text = std::fs::read_to_string(&cache).expect("cache written");
        assert!(text.starts_with(&format!("{CACHE_HEADER}\n{}\n", ctx.rustc_version)));
        // The availability marker is persisted for the next reader.
        assert!(text.lines().any(|l| l == "A 1" || l == "A 0"), "{text}");

        // Proof of a cache read: a marker appended to the file shows up.
        std::fs::write(&cache, format!("{text}S cache_marker_symbol\n")).expect("edit cache");
        let second = baseline_table(&ctx, false).expect("cache hit");
        assert!(second.symbols.defines("cache_marker_symbol"));

        // Proof of invalidation: another toolchain's cache is rebuilt.
        let stale = text.replacen(ctx.rustc_version, "rustc 0.0.0 (stale)", 1);
        std::fs::write(&cache, format!("{stale}S cache_marker_symbol\n")).expect("edit cache");
        let third = baseline_table(&ctx, false).expect("rebuild");
        assert_eq!(third, first);
        assert!(!third.symbols.defines("cache_marker_symbol"));
    }

    #[test]
    fn parse_macho_constructor_sections_by_name_in_any_segment() {
        let nm_m = "\
0000000000000040 (__DATA,__mod_init_func) non-external [no dead strip] __ZN7ctor_rs4CTOR17hf714a3cdfa196e91E
0000000000000040 (__DATA,__mod_init_func) non-external ltmp2
0000000000000010 (__DATA,__mod_term_func) non-external __ZN7ctor_rs4DTOR17h74a7ab87403178c4E
0000000000000000 (__TEXT,__init_offsets) external _also_ctor
0000000000000000 (__TEXT,__text) external _unit_add
0000000000000000 (__DATA,__const) non-external _data
                 (undefined) external _malloc
";
        let scan = ctor_scan_from_macho(nm_m);
        assert!(scan.available);
        assert_eq!(scan.counts.get("__mod_init_func"), Some(&2));
        assert_eq!(scan.counts.get("__mod_term_func"), Some(&1));
        assert_eq!(scan.counts.get("__init_offsets"), Some(&1));
        // A plain code/data symbol never counts.
        assert!(!scan.counts.contains_key("__text"));
        assert!(scan.names["__init_offsets"].contains("_also_ctor"));
    }

    #[test]
    fn parse_elf_constructor_sections_excluding_relocations() {
        let objdump = "\
In archive libx.a:

x.o:     file format elf64-x86-64

Sections:
Idx Name             Size     VMA              Type
  0                  00000000 0000000000000000
  2 .text            00000042 0000000000000000 TEXT
  3 .init_array      00000008 0000000000000000 DATA
  4 .rela.init_array 00000018 0000000000000000
  5 .fini_array      00000008 0000000000000000 DATA
  6 .ctors.65534     00000008 0000000000000000 DATA
";
        let counts = parse_ctor_sections_elf(objdump);
        assert_eq!(counts.get(".init_array"), Some(&1));
        assert_eq!(counts.get(".fini_array"), Some(&1));
        assert_eq!(counts.get(".ctors"), Some(&1));
        // `.rela.init_array` is a relocation section, not a constructor list.
        assert_eq!(counts.values().sum::<usize>(), 3);
    }

    #[test]
    fn ctor_finding_flags_excess_and_notes_unavailability() {
        let base = {
            let mut s = CtorScan::empty_available();
            s.counts.insert("__mod_init_func".into(), 1);
            s
        };
        // Same count as the baseline: no finding.
        let same = {
            let mut s = CtorScan::empty_available();
            s.counts.insert("__mod_init_func".into(), 1);
            s
        };
        let finding = ctor_finding(&same, &base);
        assert!(!finding.failed);
        assert!(finding.detail.is_none());

        // One entry beyond the baseline, with the symbol named.
        let excess = {
            let mut s = CtorScan::empty_available();
            s.counts.insert("__mod_init_func".into(), 2);
            s.names
                .entry("__mod_init_func".into())
                .or_default()
                .insert("_ZN7ctor_rs4CTORE".into());
            s
        };
        let finding = ctor_finding(&excess, &base);
        assert!(finding.failed);
        let detail = finding.detail.expect("detail");
        assert!(detail.contains("__mod_init_func"), "{detail}");
        assert!(detail.contains("_ZN7ctor_rs4CTORE"), "{detail}");

        // An unavailable scan is a note, never a failure.
        let finding = ctor_finding(&CtorScan::default(), &base);
        assert!(!finding.failed);
        assert_eq!(finding.detail.as_deref(), Some(SCAN_UNAVAILABLE));
        let finding = ctor_finding(&excess, &CtorScan::default());
        assert!(!finding.failed);
        assert_eq!(finding.detail.as_deref(), Some(SCAN_UNAVAILABLE));
    }

    #[test]
    fn combine_appends_notes_and_folds_failures() {
        let pass = Check {
            name: CHECK_NAME.into(),
            passed: true,
            detail: "1 exported symbol(s) match the unit's symbols exactly".into(),
        };
        // A clean constructor scan leaves the detail untouched.
        let clean = combine(
            pass.clone(),
            CtorFinding {
                failed: false,
                detail: None,
            },
        );
        assert!(clean.passed);
        assert_eq!(
            clean.detail,
            "1 exported symbol(s) match the unit's symbols exactly"
        );
        // An unavailable note is appended but does not fail the check.
        let noted = combine(
            pass.clone(),
            CtorFinding {
                failed: false,
                detail: Some(SCAN_UNAVAILABLE.into()),
            },
        );
        assert!(noted.passed);
        assert!(noted.detail.ends_with(SCAN_UNAVAILABLE), "{}", noted.detail);
        // A constructor excess fails an otherwise-green symbol comparison.
        let failed = combine(
            pass,
            CtorFinding {
                failed: true,
                detail: Some("pre-main constructor sections beyond the baseline — x".into()),
            },
        );
        assert!(!failed.passed);
        assert!(
            failed.detail.contains("pre-main constructor"),
            "{}",
            failed.detail
        );
    }

    /// The review's exact attack: a candidate whose symbol set is clean but
    /// which carries a pre-main static initializer in `__mod_init_func`. The
    /// defined-external symbol check passes it (the initializer is a mangled
    /// local); the constructor scan must fail it. macOS only — the ELF path
    /// needs `objdump`, which this bench does not allowlist.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_pre_main_constructor_fails_the_symbol_set_check() {
        let bench = ToolBench::new("symctor");
        let ctx = bench.symbol_ctx();
        let krate = fixture_crate(
            bench.root(),
            "ctor_rs",
            false,
            "#[no_mangle]\npub extern \"C\" fn unit_add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n\
             extern \"C\" fn forge() { std::process::exit(0); }\n\
             #[used]\n#[link_section = \"__DATA,__mod_init_func\"]\n\
             static CTOR: extern \"C\" fn() = forge;\n",
        );
        let lib = bench.build(&krate);
        // Sanity: the symbol comparison alone would have passed — the exported
        // set is exactly `unit_add` (the initializer is a mangled local).
        let symbols = defined_unmangled(ctx.runner, &lib).expect("nm");
        let base = baseline_table(&ctx, false).expect("baseline");
        assert!(compare(&symbols, &base.symbols, &["unit_add".to_string()]).passed);

        // The full check catches the constructor.
        let check =
            symbol_set_check(&ctx, &lib, false, &["unit_add".to_string()]).expect("check runs");
        assert!(!check.passed, "{}", check.detail);
        assert!(
            check
                .detail
                .contains("pre-main constructor sections beyond the baseline"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("__mod_init_func"), "{}", check.detail);
    }
}
