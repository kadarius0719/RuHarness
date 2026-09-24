//! Design B (docs/ORACLE-HARDENING.md §B.4): the boundary check's
//! orchestration — the builds, the five phases and the verdict check. The pure
//! parts (formats, windows, the generated wrapper) are in [`crate::boundary`];
//! the runtime is the harness-owned C it embeds.
//!
//! Everything runs through the machinery every other check uses: allowlisted
//! `cc` under the tool profile, every built binary confined (a fresh
//! `TMPDIR`, the run profile), every detail scrubbed by the caller. Artifacts
//! live in `migration/build/<unit>/bd/`, recreated on each run.
//!
//! Outcomes, by whose fault they are (§B.R-5, §B.R-6):
//! - the C side — the driver, the unit's C, the interface lines, the
//!   toolchain — cannot be measured: a FAILED check whose detail starts with
//!   [`C_SIDE_LEAD_IN`] (never candidate evidence; `verify` demotes, `bench`
//!   reports a PROBLEM, the migrate judge refuses to feed it to a model);
//! - the Rust touched what the C did not: a failed check with the harness's
//!   own wording (class `oracle`);
//! - the Rust crashed elsewhere: `candidate run failed: …` (class
//!   `crash-timeout`), as for the differential driver.

use crate::boundary::{
    self, apply_learn, categorize, derive_windows, parse_learn, parse_measurement, parse_tight,
    render_probe, render_table, render_wrapper, ArgKind, ElemSize, FaultCategory, Layout,
    Measurement, Probe, TightEnd, Windows, WrapperParam, WrapperSym,
};
use crate::confine::{Collected, Confinement, Extras};
use crate::exec::{RunFailure, RunOutput, Runner};
use crate::{cc_outcome, CcInvocation};
use harness_core::error::Error;
use harness_core::verdict::Check;
use harness_core::Unit;
use harness_scan::{parse_interface, InterfaceSig};
use std::path::{Path, PathBuf};

/// Name of the check in verdicts.
pub(crate) const CHECK_NAME: &str = "boundary";

/// Lead-in of every C-side detail (the migrate judge keys on it).
pub(crate) const C_SIDE_LEAD_IN: &str = harness_core::verdict::BOUNDARY_C_SIDE_LEAD_IN;

/// Most bytes of a compiler's stderr quoted in a detail.
const STDERR_CAP: usize = 600;

/// What the check needs from the verification in progress (canonical paths;
/// the caller validated containment).
pub(crate) struct BoundaryCtx<'a> {
    /// Spawns tools.
    pub runner: &'a Runner,
    /// Runs built binaries.
    pub confined: &'a Confinement<'a>,
    /// `-I` dirs in search order (the source dir, then the include dirs).
    pub includes: &'a [PathBuf],
    /// The unit's headers (its include closure), for the wrapper's includes.
    pub headers: &'a [PathBuf],
    /// The unit's `.c` files.
    pub unit_c: &'a [PathBuf],
    /// The validated differential driver.
    pub driver: &'a Path,
    /// The candidate staticlib.
    pub rust_lib: &'a Path,
    /// The unit build dir (`bd/` is created inside it).
    pub build: &'a Path,
    /// The unit (interface lines, symbols).
    pub unit: &'a Unit,
}

/// A failed check on the C side.
fn c_side(reason: String) -> Check {
    Check {
        name: CHECK_NAME.into(),
        passed: false,
        detail: format!("{C_SIDE_LEAD_IN}{reason}"),
    }
}

/// A failed check indicting the candidate.
fn red(detail: String) -> Check {
    Check {
        name: CHECK_NAME.into(),
        passed: false,
        detail,
    }
}

/// Compiler stderr, capped, for a detail.
fn cap(stderr: &str) -> String {
    let mut s: String = stderr
        .chars()
        .filter(|c| *c != '\r')
        .take(STDERR_CAP)
        .collect();
    if stderr.len() > STDERR_CAP {
        s.push_str(" [truncated]");
    }
    s
}

/// Run the check. `Err` is reserved for harness faults (a path that cannot
/// be written, a tool that cannot run); every outcome about the unit, the
/// driver or the candidate is a [`Check`].
pub(crate) fn run(ctx: &BoundaryCtx<'_>) -> Result<Check, Error> {
    // 1. Interfaces: every plan symbol exactly once, every line parseable.
    let mut sigs: Vec<(InterfaceSig, &str)> = Vec::new();
    for line in &ctx.unit.interface {
        match parse_interface(line) {
            Ok(sig) => {
                if !ctx.unit.symbols.contains(&sig.symbol) {
                    return Ok(c_side(format!(
                        "an interface line declares `{}`, which is not one of the unit's symbols",
                        sig.symbol
                    )));
                }
                if sigs.iter().any(|(s, _)| s.symbol == sig.symbol) {
                    return Ok(c_side(format!(
                        "two interface lines declare `{}`",
                        sig.symbol
                    )));
                }
                sigs.push((sig, line.as_str()));
            }
            Err(why) => {
                return Ok(c_side(format!(
                    "an interface line cannot be used for the call wrapper: {why}"
                )))
            }
        }
    }
    for sym in &ctx.unit.symbols {
        if !sigs.iter().any(|(s, _)| &s.symbol == sym) {
            return Ok(c_side(format!("no interface line declares `{sym}`")));
        }
    }

    // 2. A fresh bd/ with the runtime, the probe and (soon) the wrapper.
    let bd = ctx.build.join("bd");
    if bd.exists() {
        std::fs::remove_dir_all(&bd).map_err(|e| Error::io(&bd, e))?;
    }
    std::fs::create_dir_all(&bd).map_err(|e| Error::io(&bd, e))?;
    let bd = bd.canonicalize().map_err(|e| Error::io(&bd, e))?;
    if !bd.starts_with(ctx.build) {
        return Err(Error::Invariant(
            "the boundary build dir escaped the unit build dir".into(),
        ));
    }
    let write = |name: &str, text: &str| -> Result<PathBuf, Error> {
        let path = bd.join(name);
        std::fs::write(&path, text).map_err(|e| Error::io(&path, e))?;
        Ok(path)
    };
    write(boundary::GUARD_INTERNAL_H_NAME, boundary::GUARD_INTERNAL_H)?;
    let guard_c = write(boundary::GUARD_C_NAME, boundary::GUARD_C)?;
    let probe_c = write(boundary::PROBE_C_NAME, boundary::PROBE_C)?;
    let mut includes: Vec<PathBuf> = vec![bd.clone()];
    includes.extend(ctx.includes.iter().cloned());
    let header_includes: Vec<String> = ctx
        .headers
        .iter()
        .map(|h| format!("{:?}", h.display().to_string()))
        .collect();

    let cc =
        |out: &Path, inputs: &[PathBuf], cflags: &[&str]| -> Result<Result<(), String>, Error> {
            let cflags: Vec<String> = cflags.iter().map(|s| (*s).to_string()).collect();
            cc_outcome(
                ctx.runner,
                &CcInvocation {
                    includes: &includes,
                    cflags: &cflags,
                    quiet: true,
                    out,
                    inputs,
                    libs: &[],
                },
            )
        };

    // 3. Classification: the symbol's prototype must compile on its own
    //    (else the unit's headers do not declare its types: not applicable);
    //    syntactic data pointers as they are; every other non-function-pointer
    //    parameter probed with the compiler (§B.R-9).
    let mut syms: Vec<WrapperSym<'_>> = Vec::new();
    for (i, (sig, line)) in sigs.iter().enumerate() {
        let probe_sym = WrapperSym {
            sig,
            line,
            params: Vec::new(),
        };
        let probe = |k: usize, which: Probe, flags: &[&str]| -> Result<bool, Error> {
            let tag = match which {
                Probe::Baseline => "base",
                Probe::IsNotPointer => "ptr",
                Probe::PointeeComplete => "elem",
            };
            let src = write(
                &format!("probe_{i}_{k}_{tag}.c"),
                &render_probe(&header_includes, &probe_sym, k, which),
            )?;
            Ok(cc(
                &bd.join(format!("probe_{i}_{k}_{tag}.o")),
                std::slice::from_ref(&src),
                flags,
            )?
            .is_ok())
        };
        if !probe(0, Probe::Baseline, &["-O0", "-c"])? {
            return Ok(c_side(format!(
                "the prototype of `{}` does not compile against the unit's headers (a type the \
                 headers do not declare): the call wrapper cannot be generated",
                sig.symbol
            )));
        }
        let mut params = Vec::new();
        for (k, p) in sig.params.iter().enumerate() {
            if p.function_pointer {
                continue;
            }
            // A pointer type makes the static assertion fail.
            let is_pointer = p.data_pointer || !probe(k, Probe::IsNotPointer, &["-O0", "-c"])?;
            if !is_pointer {
                continue;
            }
            let complete = probe(
                k,
                Probe::PointeeComplete,
                &["-O0", "-c", "-Werror=pointer-arith"],
            )?;
            params.push(WrapperParam {
                index: k as u32,
                elem: if complete {
                    ElemSize::SizeofPointee
                } else {
                    ElemSize::One
                },
            });
        }
        syms.push(WrapperSym { sig, line, params });
    }
    if syms.iter().all(|s| s.params.is_empty()) {
        return Ok(c_side(
            "no data-pointer parameter: nothing for the boundary check to guard".into(),
        ));
    }
    let params_per_sym: Vec<Vec<u32>> = syms
        .iter()
        .map(|s| s.params.iter().map(|p| p.index).collect())
        .collect();
    let wrapper_c = write(
        boundary::WRAPPER_C_NAME,
        &render_wrapper(&header_includes, &syms),
    )?;
    let sig_refs: Vec<&InterfaceSig> = syms.iter().map(|s| s.sig).collect();
    let renames = boundary::rename_flags(&sig_refs);
    let rename_refs: Vec<&str> = renames.iter().map(String::as_str).collect();

    // 4. Builds (every failure is the C side's).
    macro_rules! build {
        ($what:expr, $out:expr, $inputs:expr, $flags:expr) => {{
            let out: PathBuf = $out;
            match cc(&out, $inputs, $flags)? {
                Ok(()) => out,
                Err(stderr) => {
                    return Ok(c_side(format!(
                        "{} does not compile: {}",
                        $what,
                        cap(&stderr)
                    )))
                }
            }
        }};
    }
    let rt = build!(
        "the guard runtime",
        bd.join("rt.o"),
        std::slice::from_ref(&guard_c),
        &["-O0", "-c"]
    );
    let rt_measure = build!(
        "the guard runtime (measure)",
        bd.join("rt_measure.o"),
        std::slice::from_ref(&guard_c),
        &["-O0", "-c", boundary::MEASURE_DEFINE]
    );
    let probe_cov = build!(
        "the tracing probe",
        bd.join("probe_cov.o"),
        std::slice::from_ref(&probe_c),
        &["-O0", "-c", boundary::COVERAGE_FLAG]
    );
    let wrap = build!(
        "the generated call wrapper",
        bd.join("wrap.o"),
        std::slice::from_ref(&wrapper_c),
        &["-O0", "-c"]
    );
    let mut drv_flags: Vec<&str> = vec!["-O0", "-c", "-fsanitize=address"];
    drv_flags.extend(rename_refs.iter().copied());
    let driver_in = [ctx.driver.to_path_buf()];
    let drv_asan = build!(
        "the driver (AddressSanitizer, renamed calls)",
        bd.join("drv_asan.o"),
        &driver_in,
        &drv_flags
    );
    let mut drv_plain_flags: Vec<&str> = vec!["-O0", "-c"];
    drv_plain_flags.extend(rename_refs.iter().copied());
    let drv = build!(
        "the driver (renamed calls)",
        bd.join("drv.o"),
        &driver_in,
        &drv_plain_flags
    );
    let plain_o = build!("the driver", bd.join("plain.o"), &driver_in, &["-O0", "-c"]);
    let mut unit_o1 = Vec::new();
    let mut unit_o0 = Vec::new();
    let mut unit_plain = Vec::new();
    for (i, c) in ctx.unit_c.iter().enumerate() {
        let one = std::slice::from_ref(c);
        unit_o1.push(build!(
            "the unit's C (instrumented, -O1)",
            bd.join(format!("unit_cov_o1_{i}.o")),
            one,
            &["-O1", "-c", boundary::COVERAGE_FLAG]
        ));
        unit_o0.push(build!(
            "the unit's C (instrumented, -O0)",
            bd.join(format!("unit_cov_o0_{i}.o")),
            one,
            &["-O0", "-c", boundary::COVERAGE_FLAG]
        ));
        unit_plain.push(build!(
            "the unit's C",
            bd.join(format!("unit_{i}.o")),
            one,
            &["-O0", "-c"]
        ));
    }
    // Every instrumented unit object must actually reference the callbacks
    // (M12: a build slip or a `no_sanitize` attribute would trace nothing).
    for obj in unit_o1.iter().chain(unit_o0.iter()) {
        let nm = ctx
            .runner
            .tool(&["nm".into(), "-u".into(), path_str(obj)?.to_string()])?;
        if !String::from_utf8_lossy(&nm).contains("sanitizer_cov_") {
            return Ok(c_side(
                "an instrumented unit object references no coverage callback: load/store tracing \
                 is inactive for this unit (a `no_sanitize` attribute, or an unsupported toolchain)"
                    .into(),
            ));
        }
    }
    let link = |what: &str,
                out: &str,
                inputs: Vec<PathBuf>,
                flags: &[&str]|
     -> Result<Result<PathBuf, Check>, Error> {
        let out_path = bd.join(out);
        Ok(match cc(&out_path, &inputs, flags)? {
            Ok(()) => Ok(out_path),
            Err(stderr) => Err(c_side(format!("{what} does not link: {}", cap(&stderr)))),
        })
    };
    let mut plain_in = vec![plain_o];
    plain_in.extend(unit_plain.iter().cloned());
    let plain = match link("the plain driver", "plain", plain_in, &["-O0"])? {
        Ok(p) => p,
        Err(c) => return Ok(c),
    };
    let mut measure_in = vec![drv_asan.clone(), wrap.clone()];
    measure_in.extend(unit_o1.iter().cloned());
    measure_in.extend([rt_measure.clone(), probe_cov.clone()]);
    let bd_measure = match link(
        "the measure build",
        "bd_measure",
        measure_in,
        &["-O0", "-fsanitize=address"],
    )? {
        Ok(p) => p,
        Err(c) => return Ok(c),
    };
    let mut measure0_in = vec![drv_asan, wrap.clone()];
    measure0_in.extend(unit_o0.iter().cloned());
    measure0_in.extend([rt_measure, probe_cov]);
    let bd_measure_o0 = match link(
        "the measure build (-O0)",
        "bd_measure_o0",
        measure0_in,
        &["-O0", "-fsanitize=address"],
    )? {
        Ok(p) => p,
        Err(c) => return Ok(c),
    };
    let mut c_in = vec![drv.clone(), wrap.clone()];
    c_in.extend(unit_plain.iter().cloned());
    c_in.push(rt.clone());
    let bd_c = match link("the guarded C build", "bd_c", c_in, &["-O0"])? {
        Ok(p) => p,
        Err(c) => return Ok(c),
    };
    let rs_in = vec![drv, wrap, ctx.rust_lib.to_path_buf(), rt];
    let bd_rs = match link("the guarded Rust build", "bd_rs", rs_in, &["-O0"])? {
        Ok(p) => p,
        Err(c) => return Ok(c),
    };

    // 5. Phase 0: the plain reference run.
    let plain_out = match ctx.confined.run(&plain, &[], &[])? {
        Ok(out) => out,
        Err(e) => return Ok(c_side(format!("the driver's plain run failed: {e}"))),
    };

    // A guarded run: mode, optional table, both streams, the out file.
    let guarded = |bin: &Path,
                   mode: &str,
                   table: Option<&Path>|
     -> Result<(Result<RunOutput, RunFailure>, Option<Collected>), Error> {
        let mode_os = std::ffi::OsString::from(mode);
        let mut env: Vec<(&str, &std::ffi::OsStr)> = vec![("RUHARNESS_GUARD", mode_os.as_os_str())];
        let table_os = table.map(|t| t.as_os_str().to_os_string());
        if let Some(t) = &table_os {
            env.push(("RUHARNESS_GUARD_WINDOWS", t.as_os_str()));
        }
        let inputs: Vec<PathBuf> = table.map(Path::to_path_buf).into_iter().collect();
        ctx.confined.run_with(
            bin,
            &[],
            &inputs,
            &Extras {
                env: &env,
                collect: Some((boundary::OUT_FILE, boundary::MAX_OUT_BYTES)),
            },
        )
    };
    let bytes_of = |c: Option<Collected>| -> Result<Vec<u8>, String> {
        match c {
            Some(Collected::Bytes(b)) => Ok(b),
            Some(Collected::Missing) | None => Err("the run left no record".into()),
            Some(Collected::Invalid) => {
                Err("the run's record is not a regular file within the cap".into())
            }
        }
    };

    // 6. Phase M: measure at -O1, falling back to -O0 when the instrumented
    //    unit's behavior differs from the plain run's.
    let mut measurement: Option<Measurement> = None;
    let mut last_problem = String::new();
    for (bin, level) in [(&bd_measure, "-O1"), (&bd_measure_o0, "-O0")] {
        let (run, rec) = guarded(bin, "measure", None)?;
        match run {
            Ok(out) if out == plain_out => {
                match bytes_of(rec).and_then(|b| parse_measurement(&b, &params_per_sym)) {
                    Ok(m) => {
                        measurement = Some(m);
                        break;
                    }
                    Err(why) => {
                        last_problem =
                            format!("the measure run ({level}) left an unusable record: {why}")
                    }
                }
            }
            Ok(_) => {
                last_problem =
                    format!("the measure run ({level}) prints something else than the plain run")
            }
            Err(e) => last_problem = format!("the measure run ({level}) failed: {e}"),
        }
    }
    let Some(m) = measurement else {
        return Ok(c_side(last_problem));
    };
    if !m.probe {
        return Ok(c_side(
            "load/store tracing is inactive in this toolchain (the probe did not fire)".into(),
        ));
    }
    let mut windows = derive_windows(&m);

    // 7. Phase L: one learn run per layout.
    let table_path = |layout: Layout| bd.join(format!("windows-{}.txt", layout.name()));
    let mut widened = 0usize;
    for layout in Layout::BOTH {
        let table = table_path(layout);
        std::fs::write(&table, render_table(&windows, layout)).map_err(|e| Error::io(&table, e))?;
        let (run, rec) = guarded(&bd_c, &format!("learn-{}", layout.name()), Some(&table))?;
        match run {
            Ok(out) if out == plain_out => {}
            Ok(_) => {
                return Ok(c_side(format!(
                    "the C prints something else under {} guarding",
                    layout.name()
                )))
            }
            Err(e) => {
                return Ok(c_side(format!(
                    "the C failed under {} guarding (learn): {e}",
                    layout.name()
                )))
            }
        }
        let events = match bytes_of(rec).and_then(|b| parse_learn(&b, layout, &windows)) {
            Ok(ev) => ev,
            Err(why) => {
                return Ok(c_side(format!(
                    "the {} learn run left an unusable record: {why}",
                    layout.name()
                )))
            }
        };
        widened += apply_learn(&mut windows, &events);
    }

    // 8. Phase C: the C, strictly, in both layouts.
    for layout in Layout::BOTH {
        let table = table_path(layout);
        std::fs::write(&table, render_table(&windows, layout)).map_err(|e| Error::io(&table, e))?;
        let (run, rec) = guarded(&bd_c, layout.name(), Some(&table))?;
        let ok = matches!(&run, Ok(out) if *out == plain_out);
        let ended = matches!(
            bytes_of(rec).and_then(|b| parse_tight(&b, layout)),
            Ok(TightEnd::Ended)
        );
        if !ok || !ended {
            let why = match run {
                Err(e) => format!("failed: {e}"),
                Ok(_) => "printed something else, or left no clean record".into(),
            };
            return Ok(c_side(format!(
                "the C does not run clean under its own measured windows ({} layout): {why}",
                layout.name()
            )));
        }
    }

    // 9. Phase R: the Rust, strictly, tail then head; the first failing
    //    layout is reported.
    for layout in Layout::BOTH {
        let table = table_path(layout);
        let (run, rec) = guarded(&bd_rs, layout.name(), Some(&table))?;
        let end = bytes_of(rec).and_then(|b| parse_tight(&b, layout));
        match (&run, &end) {
            (Ok(out), Ok(TightEnd::Ended)) if *out == plain_out => continue,
            (Ok(out), Ok(TightEnd::Ended)) => {
                return Ok(crate::run_diff_check(CHECK_NAME, &plain_out, out));
            }
            (_, Ok(TightEnd::Fault(call, obj, byte))) => {
                return Ok(red(describe_fault(
                    &windows, &sigs, *call, *obj, *byte, layout,
                )));
            }
            (_, Ok(TightEnd::Stale(call, from, obj, byte))) => {
                return Ok(red(format!(
                    "in call {call}, the Rust touched byte {byte} of an object that was passed to \
                     call {from} (object {obj}) and no longer exists: a pointer retained across \
                     calls ({} layout)",
                    layout.name()
                )));
            }
            (_, Ok(TightEnd::Tamper(what))) => {
                return Ok(red(format!(
                    "the guard was tampered with ({what}): the candidate altered how memory faults \
                     are delivered instead of staying inside the C's footprint ({} layout)",
                    layout.name()
                )));
            }
            (_, Ok(TightEnd::Diverged(what))) => {
                return Ok(red(format!(
                    "the Rust changed the driver's control flow ({what}): a different call or \
                     argument sequence than the C produced ({} layout)",
                    layout.name()
                )));
            }
            (_, Ok(TightEnd::Error(reason))) => {
                return Err(Error::Invariant(format!(
                    "boundary runtime error in the Rust run that did not occur in the C run: {reason}"
                )));
            }
            (Err(e), _) => {
                return Ok(red(format!("candidate run failed: {e}")));
            }
            (Ok(_), Err(why)) => {
                return Ok(red(format!(
                    "the candidate run ended without a clean guard record ({why}; {} layout)",
                    layout.name()
                )));
            }
        }
    }

    // 10. Green: what was proven, in numbers.
    let calls = windows.calls.len();
    let objects: usize = windows.calls.iter().map(|c| c.objs.len()).sum();
    let untouched = windows
        .calls
        .iter()
        .flat_map(|c| &c.objs)
        .filter(|o| o.lo == o.hi)
        .count();
    let partial = windows
        .calls
        .iter()
        .flat_map(|c| &c.objs)
        .filter(|o| o.lo < o.hi && (o.hi - o.lo) < o.count())
        .count();
    let unshadowed = windows
        .calls
        .iter()
        .flat_map(|c| &c.args)
        .filter(|a| a.kind == ArgKind::Pass)
        .count();
    let mut detail = format!(
        "Rust stays inside the C's footprint: {calls} call(s), {objects} guarded object(s) \
         ({untouched} untouched by the C, {partial} partially touched, {widened} widened), \
         {unshadowed} argument(s) unshadowed; tail and head layouts clean"
    );
    if let Some(n) = m.foreign_stack {
        detail.push_str(&format!(
            "; note: in call {n} the C reads the driver's stack through a pointer field (unchecked)"
        ));
    }
    Ok(Check {
        name: CHECK_NAME.into(),
        passed: true,
        detail,
    })
}

/// The harness's own description of a fault (§B.R-6): category, window,
/// call, symbol and parameter — never an address, never the raw element.
fn describe_fault(
    w: &Windows,
    sigs: &[(InterfaceSig, &str)],
    call: u32,
    obj: u32,
    byte: i64,
    layout: Layout,
) -> String {
    let Some(c) = w.calls.get(call as usize - 1) else {
        return format!("the Rust touched memory outside the C's footprint (call {call})");
    };
    let sym = sigs
        .get(c.sym as usize)
        .map_or("?", |(s, _)| s.symbol.as_str());
    let param = c
        .args
        .iter()
        .find(|a| matches!(a.kind, ArgKind::Obj { obj: j, .. } if j == obj))
        .and_then(|a| {
            sigs.get(c.sym as usize)
                .and_then(|(s, _)| s.params.get(a.param as usize))
        })
        .map_or_else(|| format!("object {obj}"), |p| format!("`{}`", p.name));
    let Some(win) = c.objs.get(obj as usize) else {
        return format!("the Rust touched memory outside the C's footprint (call {call} of {sym})");
    };
    let category = match categorize(win, byte) {
        FaultCategory::Untouched => "the C does not touch it in that call".to_string(),
        FaultCategory::Below => format!("below the C's window (elements [{}, {}))", win.lo, win.hi),
        FaultCategory::Above => format!("above the C's window (elements [{}, {}))", win.lo, win.hi),
    };
    let widened = if win.widened {
        " [window widened to the whole object by an untraced C access]"
    } else {
        ""
    };
    format!(
        "in call {call} of {sym}, the Rust touched the object passed as {param} ({} x {} bytes) \
         {category}{widened}; {} layout. Touch only what the C touches on the same call: convert a \
         pointer to a reference or slice only on the path where the C dereferences it, or read \
         through an accessor at exactly the indices the C reads",
        win.count(),
        win.elem,
        layout.name()
    )
}

fn path_str(p: &Path) -> Result<&str, Error> {
    p.to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", p.display())))
}
