//! C-vs-C self-validation of a differential driver (docs/M4-DESIGN.md §5.3,
//! §5.4, R1, R5): before any Rust exists, a generated driver is held to the
//! ORIGINAL C only — it must build cleanly, have the driver shape, call
//! every unit symbol, be deterministic and UB-free, and kill enough mutants
//! of the unit to be worth trusting as the oracle's test.
//!
//! Everything runs through the same machinery as `verify`: allowlisted
//! tools under the tool sandbox profile, every built binary confined
//! (module `confine`), every check detail scrubbed. Artifacts live in
//! `migration/build/<unit>/dv/` (recreated on each call); a mutant's source
//! is written to `dv/mut-<index>/<basename>`, a directory that can never
//! collide with the driver or a unit file.

use crate::confine::Confinement;
use crate::exec::{RunFailure, RunOutput, Runner};
use crate::sandbox::{self, HostDirs, ProfileSpec};
use crate::scrub::Scrubber;
use crate::{shape, Base, CcInvocation};
use harness_core::config::TargetContext;
use harness_core::driver::{
    evaluate_mutation, sample_mutants, DriverValidation, DriverValidationInputs, MutantOutcome,
    MutationStats,
};
use harness_core::error::Error;
use harness_core::hash;
use harness_core::verdict::Check;
use harness_core::Unit;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Tools driver validation runs by name.
const VALIDATE_TOOLS: [&str; 2] = ["cc", "nm"];

/// The strict warning set of the `driver-build` check (driver translation
/// unit only: the unit's own C is compiled as in `verify`).
const STRICT_FLAGS: [&str; 7] = [
    "-Wall",
    "-Werror=implicit-function-declaration",
    "-Werror=int-conversion",
    "-Werror=incompatible-pointer-types",
    "-Werror=format",
    "-Werror=return-type",
    "-Werror=uninitialized",
];

/// Determinism runs.
const DETERMINISM_RUNS: usize = 3;

/// Largest acceptable driver output.
const MAX_DRIVER_OUTPUT: usize = 256 * 1024;

/// Wall-clock limit for one mutant's run.
const MUTANT_TIMEOUT: Duration = Duration::from_secs(10);
/// Slowest allowed run of the original driver: a third of [`MUTANT_TIMEOUT`].
const MAX_DRIVER_RUN: Duration = Duration::from_secs(3);

/// Validate `driver` (a path inside the target root — the promoted
/// `units/<id>/driver.c` or an attempt's `candidate/driver.c`) against the
/// ORIGINAL C of `unit`. Checks, in order, stopping at the first failure:
/// `driver-build`, `driver-shape`, `symbols-called`, `determinism`,
/// `opt-levels`, `sanitizers`, `mutation`.
///
/// A driver that does not compile, misbehaves, or is too weak yields a
/// NON-green record (evidence for a repair turn); `Err` is reserved for
/// harness failures — a hostile config, missing facts, a tool that cannot
/// run, or mutation sites none of whose sampled mutants compile.
pub fn validate_driver(
    target: &TargetContext,
    unit: &Unit,
    driver: &Path,
) -> Result<DriverValidation, Error> {
    let scrubber = Scrubber::from_env(&target.root);
    run(target, unit, driver, &scrubber).map_err(|e| scrubber.scrub_error(e))
}

fn run(
    target: &TargetContext,
    unit: &Unit,
    driver: &Path,
    scrubber: &Scrubber,
) -> Result<DriverValidation, Error> {
    let policy = target.config.driver_policy().map_err(Error::InvalidPlan)?;
    let base = Base::new(target, unit, &VALIDATE_TOOLS)?;
    let root = base.root.clone();
    let driver_raw = if driver.is_absolute() {
        driver.to_path_buf()
    } else {
        root.join(driver)
    };
    let driver = crate::inside(&unit.id, "driver", &driver_raw, &root)?;
    let mut unit_c: Vec<(String, PathBuf)> = Vec::new();
    for rel in unit.files.iter().filter(|f| f.ends_with(".c")) {
        unit_c.push((
            rel.clone(),
            crate::inside(&unit.id, "unit file", &root.join(rel), &root)?,
        ));
    }
    if unit_c.is_empty() {
        return Err(Error::InvalidPlan(format!(
            "unit `{}` has no .c file to validate a driver against",
            unit.id
        )));
    }
    let facts = crate::load_facts(target)?;
    let unit_source = hash::file_set_hash_on_disk(&root, &facts.include_closure(&unit.files))?;
    let driver_digest = hash::file_hash(&driver)?;

    // A fresh artifact dir per validation (stale mutants must never count).
    let (_, build) = base.build_dirs(&unit.id)?;
    let dv_raw = build.join("dv");
    if dv_raw.exists() {
        let canon = crate::inside(&unit.id, "validation dir", &dv_raw, &build)?;
        std::fs::remove_dir_all(&canon).map_err(|e| Error::io(&canon, e))?;
    }
    std::fs::create_dir_all(&dv_raw).map_err(|e| Error::io(&dv_raw, e))?;
    let dv = crate::inside(&unit.id, "validation dir", &dv_raw, &build)?;

    let host = match sandbox::sandbox_mode() {
        "sandbox-exec" => Some(HostDirs::from_env()?),
        _ => None,
    };
    let tool_profile = match &host {
        Some(host) => Some(sandbox::render_profile(&ProfileSpec {
            host,
            target_root: &root,
            toolchain: true,
            write_dirs: std::slice::from_ref(&dv),
            write_files: &[],
        })?),
        None => None,
    };
    let runner = Runner {
        cwd: root.clone(),
        allowlist: base.allowlist.clone(),
        timeout: base.timeout,
        max_output: crate::exec::DEFAULT_MAX_OUTPUT,
        tool_profile,
    };
    let cc_version = crate::tool_first_line(&runner, &["cc", "--version"])?;
    let inputs = DriverValidationInputs {
        unit_source,
        driver: driver_digest,
        toolchain: vec![
            cc_version,
            format!("sandbox: {}", sandbox::sandbox_mode()),
            crate::CFLAGS_TOOLCHAIN_ENTRY.to_string(),
        ],
    };
    let ctx = Ctx {
        target,
        unit,
        base: &base,
        driver: &driver,
        unit_c: &unit_c,
        dv: &dv,
        runner: &runner,
        confined: Confinement {
            runner: &runner,
            host: host.as_ref(),
            target_root: &root,
        },
        facts: &facts,
    };
    let (mut checks, mutation) = ctx.checks(&policy)?;
    for check in &mut checks {
        scrubber.scrub_check(check);
    }
    Ok(DriverValidation::new(
        unit.id.clone(),
        inputs,
        policy,
        checks,
        mutation,
    ))
}

/// Everything the checks share.
struct Ctx<'a> {
    target: &'a TargetContext,
    unit: &'a Unit,
    base: &'a Base,
    driver: &'a Path,
    /// `(repo-relative, canonical)` of the unit's `.c` files.
    unit_c: &'a [(String, PathBuf)],
    /// Canonical artifact dir `migration/build/<unit>/dv`.
    dv: &'a Path,
    runner: &'a Runner,
    confined: Confinement<'a>,
    facts: &'a harness_core::Facts,
}

fn pass(name: &str, detail: String) -> Check {
    Check {
        name: name.into(),
        passed: true,
        detail,
    }
}

fn fail(name: &str, detail: String) -> Check {
    Check {
        name: name.into(),
        passed: false,
        detail,
    }
}

impl Ctx<'_> {
    fn unit_paths(&self) -> Vec<PathBuf> {
        self.unit_c.iter().map(|(_, p)| p.clone()).collect()
    }

    /// Compile with the usual include path; a compiler failure is `Ok(Err)`.
    fn cc(
        &self,
        out: &Path,
        inputs: &[PathBuf],
        cflags: &[String],
        quiet: bool,
    ) -> Result<Result<(), String>, Error> {
        crate::cc_outcome(
            self.runner,
            &CcInvocation {
                includes: &self.base.includes(),
                cflags,
                quiet,
                out,
                inputs,
                libs: &[],
            },
        )
    }

    /// Run the checks in order, stopping at the first failure.
    fn checks(
        &self,
        policy: &harness_core::config::DriverPolicy,
    ) -> Result<(Vec<Check>, Option<MutationStats>), Error> {
        let mut checks: Vec<Check> = Vec::new();
        macro_rules! gate {
            ($check:expr) => {{
                let check: Check = $check;
                let ok = check.passed;
                checks.push(check);
                if !ok {
                    return Ok((checks, None));
                }
            }};
        }

        // 1. driver-build: the driver alone under the strict warning set,
        // then linked with the unit's C at -O2.
        let obj = self.dv.join("driver.o");
        let strict: Vec<String> = std::iter::once("-c")
            .chain(STRICT_FLAGS)
            .map(str::to_string)
            .collect();
        let driver_in = [self.driver.to_path_buf()];
        let o2 = self.dv.join("drv_o2");
        let mut o2_inputs = vec![obj.clone()];
        o2_inputs.extend(self.unit_paths());
        gate!(match self.cc(&obj, &driver_in, &strict, false)? {
            Err(stderr) => fail(
                "driver-build",
                format!("the driver does not compile: {stderr}")
            ),
            Ok(()) => match self.cc(&o2, &o2_inputs, &[], true)? {
                Err(stderr) => fail(
                    "driver-build",
                    format!("the driver does not link against the unit: {stderr}"),
                ),
                Ok(()) => pass(
                    "driver-build",
                    format!(
                        "compiles with -Wall {}; links against {} unit file(s)",
                        STRICT_FLAGS[1..].join(" "),
                        self.unit_c.len()
                    ),
                ),
            },
        });

        // 2. driver-shape (R1), on the object just built.
        let syms = shape::object_symbols(self.runner, &obj)?;
        let source = std::fs::read(self.driver).map_err(|e| Error::io(self.driver, e))?;
        let lint = harness_scan::lint_driver(
            &source,
            &self.unit.symbols,
            &crate::unit_header_names(self.target, self.facts, self.unit),
        );
        gate!(shape::shape_check(
            shape::object_violations(&syms, &self.unit.symbols),
            lint
        ));

        // 3. symbols-called: the object references every unit symbol.
        let missing: Vec<&str> = self
            .unit
            .symbols
            .iter()
            .filter(|s| !syms.undefined.contains(*s))
            .map(String::as_str)
            .collect();
        gate!(if missing.is_empty() {
            pass(
                "symbols-called",
                format!("calls all {} unit symbol(s)", self.unit.symbols.len()),
            )
        } else {
            fail(
                "symbols-called",
                format!("the driver never calls: {}", missing.join(", ")),
            )
        });

        // 4. determinism: three separate runs, identical, bounded.
        let mut outputs: Vec<RunOutput> = Vec::new();
        let mut run_failed: Option<Check> = None;
        let mut slowest = Duration::ZERO;
        for i in 0..DETERMINISM_RUNS {
            let started = std::time::Instant::now();
            match self.confined.run(&o2, &[], &[]) {
                Ok(out) => {
                    slowest = slowest.max(started.elapsed());
                    outputs.push(out);
                }
                Err(e) => {
                    run_failed = Some(fail("determinism", format!("run {} failed: {e}", i + 1)));
                    break;
                }
            }
        }
        if let Some(check) = run_failed {
            gate!(check);
        }
        // A mutant timeout counts as a KILL only because the original driver
        // finishes far inside the mutant limit; a slow driver would make
        // every mutant "killed" by the clock (M4 correctness review: a weak
        // driver with a 14 s busy loop went 7/22 -> 22/22). The bound is far
        // from real drivers (all 91 TRACTOR drivers run in < 0.4 s).
        if slowest > MAX_DRIVER_RUN {
            gate!(fail(
                "determinism",
                // No measured time in the text: evidence must be deterministic.
                format!(
                    "a run of the driver took longer than the {}s limit (one third of the {}s \
                     mutant timeout, so that a mutant timeout is evidence of changed behavior); \
                     do less work per run",
                    MAX_DRIVER_RUN.as_secs(),
                    MUTANT_TIMEOUT.as_secs()
                )
            ));
        }
        let pinned = outputs.first().cloned().unwrap_or_default();
        gate!(determinism_check(&outputs));

        // 5. opt-levels: -O0 must print exactly what -O2 printed (both streams).
        let o0 = self.dv.join("drv_o0");
        let mut o0_inputs = driver_in.to_vec();
        o0_inputs.extend(self.unit_paths());
        gate!(
            match self.cc(&o0, &o0_inputs, &["-O0".to_string()], true)? {
                Err(stderr) => fail("opt-levels", format!("the -O0 build failed: {stderr}")),
                Ok(()) => match self.confined.run(&o0, &[], &[]) {
                    Err(e) => fail("opt-levels", format!("the -O0 build's run failed: {e}")),
                    Ok(out) if out == pinned => pass(
                        "opt-levels",
                        format!("-O0 output identical to -O2 ({} bytes)", out.stdout.len()),
                    ),
                    Ok(out) => {
                        let (stream, a, b) = differing_stream(&out, &pinned);
                        fail(
                            "opt-levels",
                            format!(
                                "the -O0 build prints something else than -O2{stream} (lens {} vs \
                                 {}, first diff at byte {}) — undefined or unspecified behavior?",
                                a.len(),
                                b.len(),
                                first_diff(a, b)
                            ),
                        )
                    }
                },
            }
        );

        // 6. sanitizers: the same flags as verify's.
        let san = self.dv.join("drv_san");
        let san_flags: Vec<String> = crate::SANITIZER_FLAGS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        gate!(match self.cc(&san, &o0_inputs, &san_flags, true)? {
            Err(stderr) => fail("sanitizers", format!("sanitizer build failed: {stderr}")),
            Ok(()) => crate::sanitizer_check(self.confined.run(&san, &[], &[])),
        });

        // 7. mutation adequacy.
        let (stats, check) = self.mutation(&obj, &pinned, policy)?;
        checks.push(check);
        Ok((checks, Some(stats)))
    }

    /// Sample, build and run the unit's mutants against the driver object,
    /// then apply the gate.
    fn mutation(
        &self,
        driver_obj: &Path,
        pinned: &RunOutput,
        policy: &harness_core::config::DriverPolicy,
    ) -> Result<(MutationStats, Check), Error> {
        let mut all = Vec::new();
        for (rel, path) in self.unit_c {
            let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
            all.extend(harness_scan::mutants(rel, &bytes)?);
        }
        let sites = u32::try_from(all.len()).unwrap_or(u32::MAX);
        let max = usize::try_from(policy.max_mutants).unwrap_or(usize::MAX);
        let sampled = sample_mutants(&all, &self.unit.symbols, max);

        let mutant_runner = Runner {
            timeout: MUTANT_TIMEOUT,
            ..self.runner.clone()
        };
        let confined = Confinement {
            runner: &mutant_runner,
            ..self.confined
        };
        // Trivial Compiler Equivalence: every original unit file compiled
        // once to an object with exactly the flags a mutant gets; a mutant
        // whose object is byte-identical is provably equivalent (identical
        // object code => identical linked program) and is never counted.
        let mut originals: Vec<(String, Vec<u8>)> = Vec::new();
        for (rel, path) in self.unit_c {
            let dir = self.dv.join(format!("orig-{}", originals.len()));
            std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
            let obj = self.compile_object(path, &dir)?.ok_or_else(|| {
                Error::Invariant(format!(
                    "mutation: the ORIGINAL {rel} does not compile to an object on its own"
                ))
            })?;
            originals.push((
                rel.clone(),
                std::fs::read(&obj).map_err(|e| Error::io(&obj, e))?,
            ));
        }
        let mut results = Vec::with_capacity(sampled.len());
        for (index, mutant) in sampled.into_iter().enumerate() {
            let outcome =
                self.run_mutant(index, &mutant, driver_obj, pinned, &confined, &originals)?;
            results.push((mutant, outcome));
        }
        evaluate_mutation(sites, &results, &self.unit.symbols, policy)
    }

    fn run_mutant(
        &self,
        index: usize,
        mutant: &harness_core::driver::Mutant,
        driver_obj: &Path,
        pinned: &RunOutput,
        confined: &Confinement<'_>,
        originals: &[(String, Vec<u8>)],
    ) -> Result<MutantOutcome, Error> {
        let (_, original) = self
            .unit_c
            .iter()
            .find(|(rel, _)| *rel == mutant.file)
            .ok_or_else(|| Error::Invariant(format!("mutant of unknown file {}", mutant.file)))?;
        let bytes = std::fs::read(original).map_err(|e| Error::io(original, e))?;
        let mutated = mutant.apply(&bytes).ok_or_else(|| {
            Error::Invariant(format!(
                "mutant span {}..{} does not fit {}",
                mutant.start, mutant.end, mutant.file
            ))
        })?;
        let dir = self.dv.join(format!("mut-{index}"));
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let name = original
            .file_name()
            .ok_or_else(|| Error::Invariant(format!("{} has no file name", mutant.file)))?;
        let mutated_path = dir.join(name);
        std::fs::write(&mutated_path, &mutated).map_err(|e| Error::io(&mutated_path, e))?;

        // The mutated copy's own quoted includes must still find the
        // original's neighbours: its original directory is searched first.
        let includes = self.mutant_includes(original)?;
        let Some(mutant_obj) = self.compile_object_with(&mutated_path, &dir, &includes)? else {
            return Ok(MutantOutcome::NotCompiled);
        };
        let obj_bytes = std::fs::read(&mutant_obj).map_err(|e| Error::io(&mutant_obj, e))?;
        if originals
            .iter()
            .any(|(rel, bytes)| *rel == mutant.file && *bytes == obj_bytes)
        {
            return Ok(MutantOutcome::Equivalent);
        }
        let mut inputs = vec![driver_obj.to_path_buf(), mutant_obj];
        inputs.extend(
            self.unit_c
                .iter()
                .filter(|(_, p)| p != original)
                .map(|(_, p)| p.clone()),
        );
        let bin = dir.join("bin");
        let built = crate::cc_outcome(
            self.runner,
            &CcInvocation {
                includes: &includes,
                cflags: &[],
                quiet: true,
                out: &bin,
                inputs: &inputs,
                libs: &[],
            },
        )?;
        if built.is_err() {
            return Ok(MutantOutcome::NotCompiled);
        }
        Ok(match confined.run(&bin, &[], &[]) {
            Ok(out) if out == *pinned => MutantOutcome::Survived,
            Ok(_) | Err(RunFailure::Failed(_)) | Err(RunFailure::TimedOut { .. }) => {
                MutantOutcome::Killed
            }
        })
    }
}

impl Ctx<'_> {
    /// `-I` order for a (possibly mutated copy of a) unit file: the
    /// ORIGINAL file's directory first, then the usual include dirs.
    fn mutant_includes(&self, original: &Path) -> Result<Vec<PathBuf>, Error> {
        let orig_dir = original
            .parent()
            .ok_or_else(|| Error::Invariant(format!("{} has no parent", original.display())))?
            .to_path_buf();
        Ok(std::iter::once(orig_dir)
            .chain(self.base.includes())
            .collect())
    }

    /// Compile an original unit file to `<dir>/<stem>.o`.
    fn compile_object(&self, source: &Path, dir: &Path) -> Result<Option<PathBuf>, Error> {
        let includes = self.mutant_includes(source)?;
        self.compile_object_with(source, dir, &includes)
    }

    /// `cc -c` of `source` into `<dir>/<stem>.o` with the mutant flags;
    /// `None` when it does not compile.
    fn compile_object_with(
        &self,
        source: &Path,
        dir: &Path,
        includes: &[PathBuf],
    ) -> Result<Option<PathBuf>, Error> {
        let stem = source
            .file_stem()
            .ok_or_else(|| Error::Invariant(format!("{} has no file stem", source.display())))?;
        let mut name = stem.to_os_string();
        name.push(".o");
        let obj = dir.join(name);
        let built = crate::cc_outcome(
            self.runner,
            &CcInvocation {
                includes,
                cflags: &["-c".to_string()],
                quiet: true,
                out: &obj,
                inputs: &[source.to_path_buf()],
                libs: &[],
            },
        )?;
        Ok(built.is_ok().then_some(obj))
    }
}

/// The `determinism` check over the collected runs.
fn determinism_check(outputs: &[RunOutput]) -> Check {
    let Some(pinned) = outputs.first() else {
        return fail("determinism", "no run completed".into());
    };
    if let Some((i, other)) = outputs
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, o)| *o != pinned)
    {
        let (stream, a, b) = differing_stream(other, pinned);
        return fail(
            "determinism",
            format!(
                "run {} printed something else than run 1{stream} (lens {} vs {}, first diff at \
                 byte {}) — the output depends on addresses, uninitialized memory or other run \
                 state",
                i + 1,
                a.len(),
                b.len(),
                first_diff(a, b)
            ),
        );
    }
    // Observations belong on stdout; stderr is compared, never required.
    let first = &pinned.stdout;
    if first.is_empty() {
        return fail("determinism", "the driver printed nothing".into());
    }
    if first.len() > MAX_DRIVER_OUTPUT {
        return fail(
            "determinism",
            format!(
                "the driver printed {} bytes; at most {MAX_DRIVER_OUTPUT} are allowed",
                first.len()
            ),
        );
    }
    pass(
        "determinism",
        format!(
            "{} runs, {} bytes, byte-identical",
            outputs.len(),
            first.len()
        ),
    )
}

/// Which streams two runs differ in, as a label for the detail, and the
/// byte strings its lens/first-diff describe: stdout alone → no label (the
/// wording predating stderr comparison); stderr alone → ` on stderr`; both →
/// ` on stdout and stderr` (numbers for stdout).
fn differing_stream<'a>(a: &'a RunOutput, b: &'a RunOutput) -> (&'static str, &'a [u8], &'a [u8]) {
    match (a.stdout != b.stdout, a.stderr != b.stderr) {
        (true, false) => ("", &a.stdout, &b.stdout),
        (true, true) => (" on stdout and stderr", &a.stdout, &b.stdout),
        _ => (" on stderr", &a.stderr, &b.stderr),
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(stdout: &[u8], stderr: &[u8]) -> RunOutput {
        RunOutput {
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }
    }

    fn outs(stdout: &[&[u8]]) -> Vec<RunOutput> {
        stdout.iter().map(|o| out(o, b"")).collect()
    }

    #[test]
    fn determinism_rules() {
        let ok = determinism_check(&outs(&[b"a\n", b"a\n", b"a\n"]));
        assert!(ok.passed, "{}", ok.detail);
        assert_eq!(ok.detail, "3 runs, 2 bytes, byte-identical");
        let differs = determinism_check(&outs(&[b"a1", b"a1", b"a2"]));
        assert!(!differs.passed);
        assert!(differs.detail.contains("run 3"), "{}", differs.detail);
        assert!(
            differs.detail.contains("first diff at byte 1"),
            "{}",
            differs.detail
        );
        assert!(!determinism_check(&outs(&[b"", b"", b""])).passed);
        let big = vec![b'x'; MAX_DRIVER_OUTPUT + 1];
        let too_big = determinism_check(&outs(&[&big, &big, &big]));
        assert!(!too_big.passed);
        assert!(too_big.detail.contains("262144"), "{}", too_big.detail);
    }

    /// stderr is part of what a run printed: a run that differs only there
    /// is not deterministic (M4: the stdout-only compare let a unit that
    /// reports on stderr verify while wrong).
    #[test]
    fn determinism_compares_stderr_too() {
        let differs = determinism_check(&[out(b"a\n", b"e1"), out(b"a\n", b"e2")]);
        assert!(!differs.passed);
        assert!(
            differs
                .detail
                .contains("run 2 printed something else than run 1 on stderr"),
            "{}",
            differs.detail
        );
        assert!(
            differs.detail.contains("first diff at byte 1"),
            "{}",
            differs.detail
        );
        let both = determinism_check(&[out(b"a\n", b"e1"), out(b"b\n", b"e2")]);
        assert!(
            both.detail
                .contains("than run 1 on stdout and stderr (lens 2 vs 2, first diff at byte 0)"),
            "{}",
            both.detail
        );
        // stderr alone is not an observation: stdout must still say something.
        assert!(!determinism_check(&[out(b"", b"e"), out(b"", b"e")]).passed);
        assert!(determinism_check(&[out(b"a", b"e"), out(b"a", b"e")]).passed);
    }
}
