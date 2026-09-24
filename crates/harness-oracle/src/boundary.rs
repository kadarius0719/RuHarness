//! Design B (docs/ORACLE-HARDENING.md §B, as decided in §B.R): the
//! measured-footprint boundary check — the pure half. The runtime half is
//! harness-owned C under `src/boundary/` ([`GUARD_C`] & co.), compiled into
//! every boundary build; the orchestration (builds, runs, the verdict) is in
//! [`run`](crate::boundary_run).
//!
//! This module never spawns a process. It holds the runtime sources and
//! their digest, the strict parsers of what a confined run leaves in its temp
//! dir (§B.5), window derivation and learning (§B.4 phases M and L), the
//! window table the tight runs read, and the generated call wrapper and
//! classifier probes (§B.6).
//!
//! Trust: every file parsed here is written by harness code inside a confined
//! run whose other code is target-owned (the C unit) or model-written (the
//! driver, a Rust candidate). The formats carry numbers only; a malformed file
//! is a red C-side check, never candidate evidence; every name a detail shows
//! is resolved from the harness's own parse of the interface lines.

use harness_scan::InterfaceSig;
use std::fmt::Write as _;

/// The runtime's internal header (the wrapper's interface).
pub(crate) const GUARD_INTERNAL_H: &str = include_str!("boundary/ruharness_guard_internal.h");
/// The runtime.
pub(crate) const GUARD_C: &str = include_str!("boundary/ruharness_guard.c");
/// The tracing canary (instrumented in the measure build only).
pub(crate) const PROBE_C: &str = include_str!("boundary/ruharness_probe.c");

/// File names the harness writes into `build/<unit>/bd/`.
pub(crate) const GUARD_INTERNAL_H_NAME: &str = "ruharness_guard_internal.h";
/// See [`GUARD_INTERNAL_H_NAME`].
pub(crate) const GUARD_C_NAME: &str = "ruharness_guard.c";
/// See [`GUARD_INTERNAL_H_NAME`].
pub(crate) const PROBE_C_NAME: &str = "ruharness_probe.c";
/// See [`GUARD_INTERNAL_H_NAME`].
pub(crate) const WRAPPER_C_NAME: &str = "ruharness_calls.c";

/// The flags that make clang call `__sanitizer_cov_{load,store}N` before
/// every traced access (`edge` is required: without it the flags are
/// accepted and instrument nothing — §B.1).
pub(crate) const COVERAGE_FLAG: &str = "-fsanitize-coverage=edge,trace-loads,trace-stores";
/// Selects the runtime's measure variant (it then references ASan).
pub(crate) const MEASURE_DEFINE: &str = "-DRUHARNESS_MEASURE";
/// The out file a run leaves in its temp dir.
pub(crate) const OUT_FILE: &str = "ruharness-guard.out";
/// Largest out file read back.
pub(crate) const MAX_OUT_BYTES: u64 = 16 << 20;

/// Runtime limits (mirrored in `ruharness_guard.c`).
pub(crate) const MAX_CALLS: usize = 65536;
/// Most distinct objects one call may receive.
pub(crate) const MAX_OBJS: usize = 16;
/// Most data-pointer arguments one call may record.
pub(crate) const MAX_ARGS: usize = 64;
/// Largest object shadowed, in bytes.
pub(crate) const MAX_OBJ_BYTES: u64 = 16 << 20;

/// The version of the wrapper's shape, part of the runtime digest: bump when
/// [`render_wrapper`]'s output changes meaning.
const WRAPPER_TEMPLATE: &str =
    "ruharness-wrapper 1: enter(sym, frame); arg(param, p, elem) per data pointer; ret; exit";

/// Prefix of the `inputs.toolchain` entry recording that the check ran.
pub(crate) const TOOLCHAIN_ENTRY_PREFIX: &str = "boundary: sancov+guard-pages rt=";

/// 8 hex of blake3 over the harness-owned runtime sources and the wrapper
/// template: a change to any of them is a judge change.
pub(crate) fn runtime_digest() -> String {
    let mut bytes = Vec::new();
    for part in [GUARD_C, GUARD_INTERNAL_H, PROBE_C, WRAPPER_TEMPLATE] {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    let full = harness_core::hash::bytes_hash(&bytes);
    let hex = full.strip_prefix("blake3:").unwrap_or(&full);
    hex.chars().take(8).collect()
}

/// The window table's layouts (§B.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Each window ends on a page boundary (over-reads past it fault).
    Tail,
    /// Each window starts on a page boundary (reads before it fault).
    Head,
}

impl Layout {
    /// Both layouts, in the order every phase runs them.
    pub(crate) const BOTH: [Layout; 2] = [Layout::Tail, Layout::Head];

    /// The name the runtime and the details use.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Layout::Tail => "tail",
            Layout::Head => "head",
        }
    }
}

/// Where one data-pointer argument of a call pointed (measure mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgKind {
    /// NULL.
    Null,
    /// Memory ASan could not place (unit-owned, a literal, uninstrumented):
    /// passed through unshadowed.
    Pass,
    /// Byte `off` of object `obj` of the call (`off == size` = one past the end).
    Obj {
        /// Object index within the call.
        obj: u32,
        /// Byte offset into it.
        off: u64,
    },
}

/// One recorded argument: the parameter index and where it pointed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArgRec {
    /// Parameter index within the symbol's signature.
    pub param: u32,
    /// Where it pointed.
    pub kind: ArgKind,
}

/// One object of a call as the measure run saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MObj {
    /// Object size in bytes (ASan's).
    pub size: u64,
    /// Bytes per element (`sizeof *p` of the parameter it arrived through).
    pub elem: u64,
    /// Byte hull `[lo, hi)` of the C's traced accesses during the call, or
    /// `None` when untouched.
    pub hull: Option<(u64, u64)>,
}

/// One unit call as the measure run saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MCall {
    /// Index into the interface signatures.
    pub sym: u32,
    /// Data-pointer arguments in parameter order.
    pub args: Vec<ArgRec>,
    /// Objects by index.
    pub objs: Vec<MObj>,
}

/// The measure run's record (phase M).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Measurement {
    /// Whether the tracing canary fired.
    pub probe: bool,
    /// Calls, by `call number - 1`.
    pub calls: Vec<MCall>,
    /// The first call in which the C read the driver's stack outside every
    /// object (through a pointer field: §B.R-11), if any.
    pub foreign_stack: Option<u32>,
}

/// Parse the measure run's out file. `params` lists, per symbol index, the
/// parameter indices the wrapper records (its data pointers, in order).
pub(crate) fn parse_measurement(bytes: &[u8], params: &[Vec<u32>]) -> Result<Measurement, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some("ruharness-guard 1 measure") {
        return Err("bad header".into());
    }
    let probe = match lines.next() {
        Some("probe 1") => true,
        Some("probe 0") => false,
        _ => return Err("bad probe line".into()),
    };
    let mut m = Measurement {
        probe,
        calls: Vec::new(),
        foreign_stack: None,
    };
    let mut ended = false;
    let close_call = |call: &MCall| -> Result<(), String> {
        for a in &call.args {
            if let ArgKind::Obj { obj, off } = a.kind {
                let o = call
                    .objs
                    .get(obj as usize)
                    .ok_or("an argument names an object the call never recorded")?;
                if off > o.size {
                    return Err("an argument offset lies past its object".into());
                }
            }
        }
        Ok(())
    };
    for line in lines {
        if ended {
            return Err("text after `end`".into());
        }
        let f: Vec<&str> = line.split(' ').collect();
        match f.as_slice() {
            ["call", n, sym] => {
                if let Some(prev) = m.calls.last() {
                    close_call(prev)?;
                }
                let (n, sym) = (num(n)?, num(sym)?);
                if n as usize != m.calls.len() + 1 || m.calls.len() >= MAX_CALLS {
                    return Err("call numbers out of order or too many".into());
                }
                if sym as usize >= params.len() {
                    return Err("unknown symbol index".into());
                }
                m.calls.push(MCall {
                    sym: sym as u32,
                    args: Vec::new(),
                    objs: Vec::new(),
                });
            }
            ["arg", n, param, at] => {
                let (n, param) = (num(n)?, num(param)?);
                let n_calls = m.calls.len();
                let Some(call) = m.calls.last_mut() else {
                    return Err("`arg` before any call".into());
                };
                if n as usize != n_calls {
                    return Err("`arg` of a call that is not the current one".into());
                }
                if !call.objs.is_empty() {
                    return Err("`arg` after the call's objects".into());
                }
                let expected = &params[call.sym as usize];
                let pos = call.args.len();
                if pos >= expected.len() || expected[pos] as u64 != param || pos >= MAX_ARGS {
                    return Err("`arg` out of parameter order".into());
                }
                let kind = match *at {
                    "null" => ArgKind::Null,
                    "pass" => ArgKind::Pass,
                    other => {
                        let rest = other.strip_prefix("obj:").ok_or("bad `arg` target")?;
                        let (j, off) = rest.split_once(':').ok_or("bad `arg` target")?;
                        let (j, off) = (num(j)?, num(off)?);
                        if j as usize >= MAX_OBJS {
                            return Err("object index too large".into());
                        }
                        ArgKind::Obj { obj: j as u32, off }
                    }
                };
                call.args.push(ArgRec {
                    param: param as u32,
                    kind,
                });
            }
            ["obj", n, j, size, elem, lo, hi] => {
                let (n, j) = (num(n)?, num(j)?);
                let n_calls = m.calls.len();
                let Some(call) = m.calls.last_mut() else {
                    return Err("`obj` before any call".into());
                };
                if n as usize != n_calls
                    || j as usize != call.objs.len()
                    || call.objs.len() >= MAX_OBJS
                {
                    return Err("`obj` out of order".into());
                }
                let (size, elem, lo, hi) = (num(size)?, num(elem)?, num(lo)?, num(hi)?);
                if size == 0 || size > MAX_OBJ_BYTES || elem == 0 {
                    return Err("object out of bounds".into());
                }
                let hull = match (lo, hi) {
                    (0, 0) => None,
                    (lo, hi) if lo < hi && hi <= size => Some((lo, hi)),
                    _ => return Err("inverted or out-of-object hull".into()),
                };
                call.objs.push(MObj { size, elem, hull });
            }
            ["foreign", n] => {
                let n = num(n)?;
                if n == 0 || n as usize != m.calls.len() {
                    return Err("`foreign` outside the current call".into());
                }
                if m.foreign_stack.is_none() {
                    m.foreign_stack = Some(n as u32);
                }
            }
            ["end"] => ended = true,
            _ => return Err("unknown line".into()),
        }
    }
    if !ended {
        return Err("no `end` line (the run did not finish normally)".into());
    }
    if let Some(last) = m.calls.last() {
        close_call(last)?;
    }
    Ok(m)
}

/// One object's enforced window, in elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    /// Object size in bytes.
    pub size: u64,
    /// Bytes per element.
    pub elem: u64,
    /// First element the C may touch.
    pub lo: u64,
    /// One past the last element the C may touch (`lo == hi` = untouched).
    pub hi: u64,
    /// Phase L widened the window to the whole object (an untraced access).
    pub widened: bool,
}

impl Window {
    /// Elements in the object (the last one may be partial).
    pub(crate) fn count(&self) -> u64 {
        self.size.div_ceil(self.elem)
    }
}

/// One call's windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WCall {
    /// Symbol index.
    pub sym: u32,
    /// Data-pointer arguments in parameter order.
    pub args: Vec<ArgRec>,
    /// Windows by object index.
    pub objs: Vec<Window>,
}

/// What the tight runs enforce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Windows {
    /// Calls, by `call number - 1`.
    pub calls: Vec<WCall>,
}

/// Phase M's windows: each hull rounded out to whole elements.
pub(crate) fn derive_windows(m: &Measurement) -> Windows {
    Windows {
        calls: m
            .calls
            .iter()
            .map(|c| WCall {
                sym: c.sym,
                args: c.args.clone(),
                objs: c
                    .objs
                    .iter()
                    .map(|o| {
                        let (lo, hi) = match o.hull {
                            None => (0, 0),
                            Some((lo, hi)) => (lo / o.elem, hi.div_ceil(o.elem)),
                        };
                        Window {
                            size: o.size,
                            elem: o.elem,
                            lo,
                            hi,
                            widened: false,
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The window table a learn or tight run reads (`RUHARNESS_GUARD_WINDOWS`):
/// strictly sequential, per-call counts, no lookahead.
pub(crate) fn render_table(w: &Windows, layout: Layout) -> String {
    let mut out = format!("ruharness-windows 1 {} {}\n", w.calls.len(), layout.name());
    for (i, c) in w.calls.iter().enumerate() {
        let _ = writeln!(
            out,
            "call {} {} {} {}",
            i + 1,
            c.sym,
            c.objs.len(),
            c.args.len()
        );
        for (j, o) in c.objs.iter().enumerate() {
            let _ = writeln!(out, "obj {j} {} {} {} {}", o.size, o.elem, o.lo, o.hi);
        }
        for a in &c.args {
            let (kind, obj, off) = match a.kind {
                ArgKind::Null => (0, 0, 0),
                ArgKind::Pass => (1, 0, 0),
                ArgKind::Obj { obj, off } => (2, obj, off),
            };
            let _ = writeln!(out, "arg {} {kind} {obj} {off}", a.param);
        }
    }
    out.push_str("end\n");
    out
}

/// One in-call fault a learn run recorded: `(call, object, byte)`.
pub(crate) type LearnEvent = (u32, u32, u64);

/// Parse a learn run's out file.
pub(crate) fn parse_learn(
    bytes: &[u8],
    layout: Layout,
    w: &Windows,
) -> Result<Vec<LearnEvent>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some(&format!("ruharness-guard 1 learn-{}", layout.name())[..]) {
        return Err("bad header".into());
    }
    let mut out = Vec::new();
    let mut ended = false;
    for line in lines {
        if ended {
            return Err("text after `end`".into());
        }
        let f: Vec<&str> = line.split(' ').collect();
        match f.as_slice() {
            ["learn", n, j, byte] => {
                let (n, j, byte) = (num(n)?, num(j)?, num(byte)?);
                let obj = w
                    .calls
                    .get((n as usize).wrapping_sub(1))
                    .and_then(|c| c.objs.get(j as usize))
                    .ok_or("`learn` of an unknown call or object")?;
                if byte >= obj.size {
                    return Err("`learn` outside its object".into());
                }
                if out.len() >= MAX_CALLS * MAX_OBJS {
                    return Err("too many learn events".into());
                }
                out.push((n as u32, j as u32, byte));
            }
            ["end"] => ended = true,
            _ => return Err("unknown line".into()),
        }
    }
    if !ended {
        return Err("no `end` line (the run did not finish normally)".into());
    }
    Ok(out)
}

/// Apply learn events (phase L): an untraced C access outside an object's
/// window widens the window to the whole object. Returns how many objects
/// were widened.
pub(crate) fn apply_learn(w: &mut Windows, events: &[LearnEvent]) -> usize {
    let mut widened = 0;
    for &(call, obj, byte) in events {
        let o = &mut w.calls[call as usize - 1].objs[obj as usize];
        let k = byte / o.elem;
        if !(o.widened || o.lo <= k && k < o.hi) {
            o.lo = 0;
            o.hi = o.count();
            o.widened = true;
            widened += 1;
        }
    }
    widened
}

/// How a tight run's out file ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TightEnd {
    /// The run reached its normal end.
    Ended,
    /// An in-call access outside the window: `(call, object, byte)`; `byte`
    /// may be negative (before the object) or past its size.
    Fault(u32, u32, i64),
    /// An in-call access to a shadow of an EARLIER call (a retained pointer):
    /// `(call, earlier call, object, byte)`.
    Stale(u32, u32, u32, i64),
    /// The guard was tampered with (`signal | handler | exception-port | canary`).
    Tamper(String),
    /// The candidate changed the driver's control flow (`call n` / `arg n:p`).
    Diverged(String),
    /// A runtime limit or setup failure (`RH-ERROR`): the check is not applicable.
    Error(String),
}

/// Parse a tight run's out file: the header and exactly one terminal record.
/// An empty body (the process died without writing one) is `Err`.
pub(crate) fn parse_tight(bytes: &[u8], layout: Layout) -> Result<TightEnd, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some(&format!("ruharness-guard 1 {}", layout.name())[..]) {
        return Err("bad header".into());
    }
    let mut end: Option<TightEnd> = None;
    for line in lines {
        if end.is_some() {
            return Err("text after the run's last record".into());
        }
        let f: Vec<&str> = line.split(' ').collect();
        end = Some(match f.as_slice() {
            ["end"] => TightEnd::Ended,
            ["fault", n, j, b] => TightEnd::Fault(num(n)? as u32, num(j)? as u32, signed(b)?),
            ["stale", n, m, j, b] => {
                TightEnd::Stale(num(n)? as u32, num(m)? as u32, num(j)? as u32, signed(b)?)
            }
            ["tamper", what @ ("signal" | "handler" | "exception-port" | "canary")] => {
                TightEnd::Tamper((*what).to_string())
            }
            ["diverged", what @ ("call" | "arg"), n] => {
                if !n.bytes().all(|b| b.is_ascii_digit() || b == b':') || n.len() > 24 {
                    return Err("bad divergence".into());
                }
                TightEnd::Diverged(format!("{what} {n}"))
            }
            ["error", rest @ ..] => {
                let reason = rest.join(" ");
                if reason.is_empty()
                    || reason.len() > 120
                    || !reason.bytes().all(|b| (0x20..0x7f).contains(&b))
                {
                    return Err("bad error reason".into());
                }
                TightEnd::Error(reason)
            }
            _ => return Err("unknown line".into()),
        });
    }
    end.ok_or_else(|| "no record (the run died before writing one)".to_string())
}

/// The element size a data-pointer parameter's objects are measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ElemSize {
    /// `sizeof *p` (the pointee is complete).
    SizeofPointee,
    /// One byte (`void *`, an incomplete pointee): "granularity unchecked".
    One,
}

/// One data-pointer parameter the wrapper records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WrapperParam {
    /// Parameter index.
    pub index: u32,
    /// Its element size.
    pub elem: ElemSize,
}

/// One symbol of the wrapper.
#[derive(Debug, Clone)]
pub(crate) struct WrapperSym<'a> {
    /// The parsed interface line.
    pub sig: &'a InterfaceSig,
    /// The interface line's exact text.
    pub line: &'a str,
    /// The data-pointer parameters, in parameter order.
    pub params: Vec<WrapperParam>,
}

/// The interface line with its declarator renamed to `ruharness_call_<sym>`.
fn renamed_line(sym: &WrapperSym<'_>) -> String {
    let (s, e) = sym.sig.name_span;
    format!(
        "{}ruharness_call_{}{}",
        &sym.line[..s],
        sym.sig.symbol,
        &sym.line[e..]
    )
}

/// The generated call wrapper (§B.6). `includes` are the unit's own
/// `#include` spellings; symbol index = position in `syms`.
pub(crate) fn render_wrapper(includes: &[String], syms: &[WrapperSym<'_>]) -> String {
    let mut out = String::from(
        "/* Generated by RuHarness (design B call wrapper) -- harness-owned, never edited. */\n",
    );
    for inc in includes {
        let _ = writeln!(out, "#include {inc}");
    }
    let _ = writeln!(out, "#include \"{GUARD_INTERNAL_H_NAME}\"");
    for (i, sym) in syms.iter().enumerate() {
        let sig = sym.sig;
        let _ = writeln!(out, "\n{}\n{{", renamed_line(sym));
        let _ = writeln!(out, "    ruharness_enter({i}, __builtin_frame_address(0));");
        let mut args: Vec<String> = sig.params.iter().map(|p| p.name.clone()).collect();
        for wp in &sym.params {
            let name = &sig.params[wp.index as usize].name;
            let elem = match wp.elem {
                ElemSize::SizeofPointee => format!("sizeof *{name}"),
                ElemSize::One => "1".to_string(),
            };
            let _ = writeln!(
                out,
                "    __typeof__({name}) rh_{name} = (__typeof__({name}))ruharness_arg({}, (const void *){name}, {elem});",
                wp.index
            );
            args[wp.index as usize] = format!("rh_{name}");
        }
        let call = format!("{}({})", sig.symbol, args.join(", "));
        if sig.returns_void {
            let _ = writeln!(out, "    {call};\n    ruharness_exit();\n}}");
        } else if sig.returns_pointer {
            let _ = writeln!(
                out,
                "    __typeof__({call}) rh_ret = {call};\n    rh_ret = (__typeof__(rh_ret))ruharness_ret((void *)rh_ret);\n    ruharness_exit();\n    return rh_ret;\n}}"
            );
        } else {
            let _ = writeln!(
                out,
                "    __typeof__({call}) rh_ret = {call};\n    ruharness_exit();\n    return rh_ret;\n}}"
            );
        }
    }
    out
}

/// The `-D<sym>=ruharness_call_<sym>` flags the driver TU is compiled with.
/// Symbols are plain C identifiers (the interface parse refuses others), so
/// nothing else reaches the argument.
pub(crate) fn rename_flags(sigs: &[&InterfaceSig]) -> Vec<String> {
    sigs.iter()
        .map(|s| format!("-D{0}=ruharness_call_{0}", s.symbol))
        .collect()
}

/// A compile-time classification probe for one parameter (§B.R-9), each a
/// tiny translation unit with the symbol's own prototype: [`Probe::Baseline`]
/// (an empty body) must compile, or the unit's headers do not declare the
/// line's types and the check is not applicable; then [`Probe::IsNotPointer`]
/// fails to compile iff the parameter's type is a pointer; then
/// [`Probe::PointeeComplete`] compiles iff `sizeof *p` is well-formed (under
/// `-Werror=pointer-arith`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// An empty body.
    Baseline,
    /// `_Static_assert(__builtin_classify_type(p) != 5, "")`.
    IsNotPointer,
    /// `(void)sizeof(char[sizeof *p])`.
    PointeeComplete,
}

/// The probe translation unit for parameter `param` of `sym`.
pub(crate) fn render_probe(
    includes: &[String],
    sym: &WrapperSym<'_>,
    param: usize,
    probe: Probe,
) -> String {
    let mut out = String::new();
    for inc in includes {
        let _ = writeln!(out, "#include {inc}");
    }
    let name = sym.sig.params.get(param).map_or("", |p| p.name.as_str());
    let body = match probe {
        Probe::Baseline => String::new(),
        Probe::IsNotPointer => {
            format!("_Static_assert(__builtin_classify_type({name}) != 5, \"pointer\");")
        }
        Probe::PointeeComplete => format!("(void)sizeof(char[sizeof *{name}]);"),
    };
    let (s, e) = sym.sig.name_span;
    let _ = writeln!(
        out,
        "{}ruharness_probe_{}{}\n{{\n    {body}\n}}",
        &sym.line[..s],
        sym.sig.symbol,
        &sym.line[e..]
    );
    out
}

/// Where a faulting byte lies relative to a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultCategory {
    /// The C does not touch the object in that call.
    Untouched,
    /// Before the C's window.
    Below,
    /// After the C's window.
    Above,
}

/// Classify a fault at `byte` of a window (`byte` may be outside the object).
pub(crate) fn categorize(w: &Window, byte: i64) -> FaultCategory {
    if w.lo == w.hi {
        FaultCategory::Untouched
    } else if byte < 0 || (byte as u64) / w.elem < w.lo {
        FaultCategory::Below
    } else {
        FaultCategory::Above
    }
}

fn num(s: &str) -> Result<u64, String> {
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("bad number".into());
    }
    s.parse().map_err(|_| "bad number".to_string())
}

fn signed(s: &str) -> Result<i64, String> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s),
    };
    let v = i64::try_from(num(digits)?).map_err(|_| "bad number".to_string())?;
    Ok(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_scan::parse_interface;

    const LINE: &str =
        "void read_scalefactors(bs_t *bs, uint8_t *pba, uint8_t *scfcod, int bands, float *scf)";

    fn params() -> Vec<Vec<u32>> {
        vec![vec![0, 1, 2, 4]]
    }

    const MEASURED: &str = "ruharness-guard 1 measure\nprobe 1\n\
        call 1 0\narg 1 0 obj:0:0\narg 1 1 obj:1:0\narg 1 2 pass\narg 1 4 null\n\
        obj 1 0 16 16 0 0\nobj 1 1 20 1 1 4\n\
        call 2 0\narg 2 0 obj:0:0\narg 2 1 obj:1:3\narg 2 2 obj:1:0\narg 2 4 obj:2:0\n\
        obj 2 0 16 16 8 12\nobj 2 1 20 1 0 20\nobj 2 2 240 4 0 14\nforeign 2\nend\n";

    #[test]
    fn a_measurement_parses_and_derives_element_windows() {
        let m = parse_measurement(MEASURED.as_bytes(), &params()).expect("parses");
        assert!(m.probe);
        assert_eq!(m.calls.len(), 2);
        assert_eq!(m.foreign_stack, Some(2));
        assert_eq!(m.calls[0].args[2].kind, ArgKind::Pass);
        assert_eq!(m.calls[1].args[1].kind, ArgKind::Obj { obj: 1, off: 3 });
        let w = derive_windows(&m);
        let c1 = &w.calls[0].objs;
        assert_eq!((c1[0].lo, c1[0].hi), (0, 0)); // untouched struct
        assert_eq!((c1[1].lo, c1[1].hi), (1, 4)); // bytes [1,4)
        let c2 = &w.calls[1].objs;
        assert_eq!((c2[0].lo, c2[0].hi), (0, 1)); // bytes [8,12) of a 16-byte element
        assert_eq!((c2[2].lo, c2[2].hi), (0, 4)); // bytes [0,14) of 4-byte floats
        assert_eq!(c2[2].count(), 60);
        let table = render_table(&w, Layout::Head);
        assert!(table.starts_with("ruharness-windows 1 2 head\ncall 1 0 2 4\nobj 0 16 16 0 0\nobj 1 20 1 1 4\narg 0 2 0 0\narg 1 2 1 0\narg 2 1 0 0\narg 4 0 0 0\ncall 2 0 3 4\n"), "{table}");
        assert!(table.ends_with("arg 4 2 2 0\nend\n"), "{table}");
    }

    #[test]
    fn malformed_measurements_are_refused() {
        let p = params();
        for (what, bad) in [
            ("header", MEASURED.replace("1 measure\n", "1 tail\n")),
            ("no end", MEASURED.replace("end\n", "")),
            ("after end", format!("{MEASURED}call 3 0\n")),
            ("call order", MEASURED.replace("call 2 0", "call 3 0")),
            ("unknown sym", MEASURED.replace("call 2 0", "call 2 1")),
            (
                "arg order",
                MEASURED.replace("arg 1 1 obj:1:0", "arg 1 2 obj:1:0"),
            ),
            (
                "arg after obj",
                MEASURED.replace("obj 1 1 20 1 1 4\n", "obj 1 1 20 1 1 4\narg 1 4 null\n"),
            ),
            ("obj order", MEASURED.replace("obj 1 1 20", "obj 1 2 20")),
            (
                "zero elem",
                MEASURED.replace("obj 1 0 16 16 0 0", "obj 1 0 16 0 0 0"),
            ),
            (
                "hull past size",
                MEASURED.replace("obj 1 1 20 1 1 4", "obj 1 1 20 1 1 40"),
            ),
            (
                "inverted hull",
                MEASURED.replace("obj 1 1 20 1 1 4", "obj 1 1 20 1 4 1"),
            ),
            (
                "arg names missing obj",
                MEASURED.replace("arg 2 4 obj:2:0", "arg 2 4 obj:5:0"),
            ),
            (
                "arg off past object",
                MEASURED.replace("arg 2 1 obj:1:3", "arg 2 1 obj:1:21"),
            ),
            (
                "foreign wrong call",
                MEASURED.replace("foreign 2", "foreign 1"),
            ),
            ("junk", MEASURED.replace("probe 1", "probe 1\nhello")),
            (
                "sign",
                MEASURED.replace("obj 1 1 20 1 1 4", "obj 1 1 20 1 +1 4"),
            ),
        ] {
            assert!(
                parse_measurement(bad.as_bytes(), &p).is_err(),
                "accepted: {what}"
            );
        }
    }

    #[test]
    fn learning_widens_only_outside_the_window() {
        let m = parse_measurement(MEASURED.as_bytes(), &params()).expect("parses");
        let mut w = derive_windows(&m);
        let ev = parse_learn(
            b"ruharness-guard 1 learn-tail\nlearn 1 1 2\nlearn 1 1 7\nlearn 2 0 3\nend\n",
            Layout::Tail,
            &w,
        )
        .expect("parses");
        // byte 2 of object (1,1) is inside [1,4): nothing; byte 7 is outside:
        // widened; call 2's struct: byte 3 is element 0, inside [0,1): nothing.
        assert_eq!(apply_learn(&mut w, &ev), 1);
        let o = &w.calls[0].objs[1];
        assert_eq!((o.lo, o.hi, o.widened), (0, 20, true));
        assert_eq!(apply_learn(&mut w, &ev), 0);
        assert!(parse_learn(b"ruharness-guard 1 learn-head\nend\n", Layout::Tail, &w).is_err());
        assert!(parse_learn(
            b"ruharness-guard 1 learn-tail\nlearn 9 0 0\nend\n",
            Layout::Tail,
            &w
        )
        .is_err());
        assert!(parse_learn(
            b"ruharness-guard 1 learn-tail\nlearn 1 1 20\nend\n",
            Layout::Tail,
            &w
        )
        .is_err());
        assert!(parse_learn(
            b"ruharness-guard 1 learn-tail\nlearn 1 1 2\n",
            Layout::Tail,
            &w
        )
        .is_err());
    }

    #[test]
    fn tight_records_parse_strictly() {
        let ok = |s: &str, l| parse_tight(s.as_bytes(), l).expect("parses");
        assert_eq!(
            ok("ruharness-guard 1 tail\nend\n", Layout::Tail),
            TightEnd::Ended
        );
        assert_eq!(
            ok("ruharness-guard 1 tail\nfault 3 1 -2\n", Layout::Tail),
            TightEnd::Fault(3, 1, -2)
        );
        assert_eq!(
            ok("ruharness-guard 1 head\nstale 4 2 0 9\n", Layout::Head),
            TightEnd::Stale(4, 2, 0, 9)
        );
        assert_eq!(
            ok("ruharness-guard 1 head\ntamper signal\n", Layout::Head),
            TightEnd::Tamper("signal".into())
        );
        assert_eq!(
            ok("ruharness-guard 1 tail\ndiverged arg 3:2\n", Layout::Tail),
            TightEnd::Diverged("arg 3:2".into())
        );
        assert_eq!(
            ok("ruharness-guard 1 tail\nerror mmap failed\n", Layout::Tail),
            TightEnd::Error("mmap failed".into())
        );
        for bad in [
            "ruharness-guard 1 head\nend\n",
            "ruharness-guard 1 tail\n",
            "ruharness-guard 1 tail\nfault 1 1 1\nend\n",
            "ruharness-guard 1 tail\ntamper root\n",
            "ruharness-guard 1 tail\nfault x 1 1\n",
            "ruharness-guard 1 tail\ndiverged call 1; rm -rf\n",
        ] {
            assert!(
                parse_tight(bad.as_bytes(), Layout::Tail).is_err(),
                "accepted: {bad:?}"
            );
        }
    }

    #[test]
    fn the_wrapper_shadows_data_pointers_and_relocates_pointer_returns() {
        let sig = parse_interface(LINE).expect("parses");
        let ret = "int* static_alias(int *outer)";
        let rsig = parse_interface(ret).expect("parses");
        let syms = vec![
            WrapperSym {
                sig: &sig,
                line: LINE,
                params: [0u32, 1, 2, 4]
                    .iter()
                    .map(|&i| WrapperParam {
                        index: i,
                        elem: ElemSize::SizeofPointee,
                    })
                    .collect(),
            },
            WrapperSym {
                sig: &rsig,
                line: ret,
                params: vec![WrapperParam {
                    index: 0,
                    elem: ElemSize::One,
                }],
            },
        ];
        let w = render_wrapper(&["\"lib.h\"".into()], &syms);
        assert!(
            w.contains("#include \"lib.h\"\n#include \"ruharness_guard_internal.h\"\n"),
            "{w}"
        );
        assert!(w.contains("\nvoid ruharness_call_read_scalefactors(bs_t *bs, uint8_t *pba, uint8_t *scfcod, int bands, float *scf)\n{\n    ruharness_enter(0, __builtin_frame_address(0));\n"), "{w}");
        assert!(w.contains("    __typeof__(scf) rh_scf = (__typeof__(scf))ruharness_arg(4, (const void *)scf, sizeof *scf);\n    read_scalefactors(rh_bs, rh_pba, rh_scfcod, bands, rh_scf);\n    ruharness_exit();\n}\n"), "{w}");
        assert!(w.contains("\nint* ruharness_call_static_alias(int *outer)\n{\n    ruharness_enter(1, __builtin_frame_address(0));\n    __typeof__(outer) rh_outer = (__typeof__(outer))ruharness_arg(0, (const void *)outer, 1);\n    __typeof__(static_alias(rh_outer)) rh_ret = static_alias(rh_outer);\n    rh_ret = (__typeof__(rh_ret))ruharness_ret((void *)rh_ret);\n    ruharness_exit();\n    return rh_ret;\n}\n"), "{w}");
        assert_eq!(
            rename_flags(&[&sig, &rsig]),
            vec![
                "-Dread_scalefactors=ruharness_call_read_scalefactors".to_string(),
                "-Dstatic_alias=ruharness_call_static_alias".to_string()
            ]
        );
        let probe = render_probe(&["\"lib.h\"".into()], &syms[0], 3, Probe::IsNotPointer);
        assert!(probe.contains("void ruharness_probe_read_scalefactors(bs_t *bs, uint8_t *pba, uint8_t *scfcod, int bands, float *scf)\n{\n    _Static_assert(__builtin_classify_type(bands) != 5, \"pointer\");\n}"), "{probe}");
        let probe = render_probe(&[], &syms[0], 0, Probe::PointeeComplete);
        assert!(
            probe.contains("    (void)sizeof(char[sizeof *bs]);\n"),
            "{probe}"
        );
        let probe = render_probe(&[], &syms[1], 0, Probe::Baseline);
        assert!(
            probe.contains("int* ruharness_probe_static_alias(int *outer)\n{\n    \n}"),
            "{probe}"
        );
    }

    #[test]
    fn fault_categories_and_the_digest() {
        let w = Window {
            size: 20,
            elem: 4,
            lo: 1,
            hi: 3,
            widened: false,
        };
        assert_eq!(categorize(&w, 0), FaultCategory::Below);
        assert_eq!(categorize(&w, -8), FaultCategory::Below);
        assert_eq!(categorize(&w, 12), FaultCategory::Above);
        assert_eq!(categorize(&w, 64), FaultCategory::Above);
        let u = Window { lo: 0, hi: 0, ..w };
        assert_eq!(categorize(&u, 4), FaultCategory::Untouched);
        let d = runtime_digest();
        assert_eq!(d.len(), 8);
        assert!(d.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
