//! Oracle strategies: concrete [`OracleStrategy`] implementations.
//!
//! At M1 there is one strategy, [`CAbiDifferential`] — the M0 zopfli oracle
//! generalized to any unit and target. It proves a unit's Rust staticlib is
//! observably identical to the C it replaces via a differential driver, a
//! mixed whole-program build over deterministic samples, and a sanitizer run.
//! Verdicts are content-bound (docs/SCHEMAS.md "Verdicts"): digests and
//! toolchain strings, never timestamps.
//!
//! Subprocess discipline (§12.2): explicit argv arrays, the core-owned
//! `[oracle] allowlist` in `harness.toml` for named tools, working directory
//! pinned to the target root, writes confined to the ledger build dir and the
//! unit crate's own `target/`. Binaries the oracle itself just built are run
//! by path and are exempt from the name allowlist, as in M0.
//!
//! M3 trust boundaries (docs/SCHEMAS.md "Trust boundaries") — target-owned
//! files are hostile input and model output is untrusted code, so:
//! - `[oracle] extra_link_args` accepts only `-l<name>`;
//! - every path derived from the plan is canonicalized and must stay inside
//!   the target root (the unit crate inside `migration/units/<id>/`);
//! - every child gets a scrubbed environment and a wall-clock timeout
//!   (`[oracle] timeout_secs`, default 120) — module `exec`; each child leads
//!   its own process group and a timeout kills the whole group, so a
//!   grandchild cannot outlive the run that spawned it;
//! - on macOS every build and every run is wrapped in `sandbox-exec` — module
//!   `sandbox`; the mode applied is recorded as `sandbox: <mode>` in
//!   `inputs.toolchain` (see [`sandbox_mode`]). A built-binary run is confined
//!   further: it may `exec` nothing but itself;
//! - the `symbol-set` check (module `symbols`) refuses candidates whose
//!   staticlib exports anything but the unit's symbols, or which smuggle a
//!   pre-main constructor into an init/term section beyond the baseline;
//! - every [`Check`] detail and every error carrying a command line or tool
//!   stderr is machine-path-scrubbed (module `scrub`) before it leaves the
//!   crate, so committed verdicts are byte-identical across machines.
//!
//! M4 additions (docs/M4-DESIGN.md §R):
//! - every C compile passes `-ffp-contract=off` (R4: Apple clang on arm64
//!   fuses multiply-add even at `-O0`; the reference Linux build and Rust do
//!   not) and gets `-I` for `[target] include_dirs` after the source dir;
//!   the flag is recorded as `cflags: -ffp-contract=off` in `toolchain`;
//! - every run of a built binary is confined (module `confine`, R1): a fresh
//!   per-run `TMPDIR`, no reads under the home dir or the target root beyond
//!   the binary and its listed inputs;
//! - two new gating checks right after `symbol-set`, each ending the run on
//!   failure: `capabilities` (R2, module `capabilities`) and `driver-shape`
//!   (R1, module `shape`);
//! - the whole-program check is opt-in via `[oracle.whole_program] args`
//!   (R8);
//! - [`validate_driver`]: C-vs-C self-validation of a generated driver with
//!   mutation adequacy (module `validate`, R5).
//!
//! What the oracle does NOT prove (docs/SCHEMAS.md "Trust boundaries"): the
//! `sanitizers` check instruments the C baseline and driver only. Stable Rust
//! has no AddressSanitizer, so the candidate's `ffi.rs` shim is NOT
//! sanitizer-verified — its safety rests on the compiler-enforced shim
//! structure (`#[deny(unsafe_code)]` in `logic`, `unsafe` confined to `ffi`)
//! and on the differential and symbol-set checks, not on ASan/UBSan coverage.
//!
//! Dependency note: this crate depends on `harness-scan` (a workspace path
//! dependency, no crates.io addition) for exactly two pure functions —
//! `mutants` and `lint_driver` — so the tree-sitter C grammar has a single
//! owner and the oracle never re-implements C parsing.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod capabilities;
mod confine;
mod exec;
mod sandbox;
mod scrub;
mod shape;
mod symbols;
#[cfg(test)]
mod testutil;
mod validate;

pub use sandbox::sandbox_mode;
pub use validate::validate_driver;

use confine::Confinement;
use exec::{RunFailure, Runner};
use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::hash;
use harness_core::ledger::Ledger;
use harness_core::traits::OracleStrategy;
use harness_core::verdict::{Check, VerdictInputs};
use harness_core::{Facts, Unit, Verdict};
use sandbox::{HostDirs, ProfileSpec};
use scrub::Scrubber;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Tools the c-abi-differential kind runs by name; all four must be on the
/// core-owned `[oracle] allowlist`.
const REQUIRED_TOOLS: [&str; 4] = ["cc", "cargo", "rustc", "nm"];

/// Passed to EVERY C compile the oracle runs (R4). Without it Apple clang on
/// arm64 contracts `a*b+c` into an FMA even at `-O0`, while the reference
/// Linux x86-64 build and Rust never do, so float-heavy units would differ
/// for reasons that are not the translation's fault.
pub(crate) const FP_CONTRACT_OFF: &str = "-ffp-contract=off";

/// The `inputs.toolchain` entry recording [`FP_CONTRACT_OFF`].
pub(crate) const CFLAGS_TOOLCHAIN_ENTRY: &str = "cflags: -ffp-contract=off";

/// The sanitizer build flags (verify's `sanitizers` check and
/// `validate_driver`'s use the same).
pub(crate) const SANITIZER_FLAGS: [&str; 4] = [
    "-fsanitize=address,undefined",
    "-fno-sanitize-recover=all",
    "-g",
    "-O1",
];

/// Most flags `[oracle.whole_program] args` may hold (R8).
const MAX_WHOLE_PROGRAM_ARGS: usize = 4;

/// The `[unit.oracle] kind` string handled by [`CAbiDifferential`].
pub const C_ABI_DIFFERENTIAL_KIND: &str = "c-abi-differential";

/// The C-ABI differential oracle.
///
/// Kind-owned unit parameters (in `[unit.oracle]`):
/// - `driver` — repo-relative path of the differential driver `.c` file;
/// - `rust_crate` — directory name of the unit's Rust crate, relative to the
///   unit dir `migration/units/<id>/`;
/// - `replaces` — repo-relative `.c` files the staticlib replaces (excluded
///   from the mixed whole-program link).
///
/// Kind-owned config (in `harness.toml` `[oracle]`):
/// - `extra_link_args` — extra libraries for the whole-program link; ONLY
///   `-l<name>` entries (`^-l[A-Za-z0-9_+.-]+$`, e.g. `["-lm"]`) are accepted,
///   anything else is an [`Error::InvalidPlan`] before any subprocess runs;
/// - `timeout_secs` — wall-clock limit for every child process (default 120).
///   A built binary that exceeds it is a failed check (`timed out after Ns`);
///   a tool that exceeds it is an error (a red `rust-build` for the unit
///   crate's `cargo build`).
///
/// The core-owned `allowlist` key governs which named executables may run;
/// this kind needs `cc`, `cargo`, `rustc` and `nm`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CAbiDifferential;

/// Compute a unit's verdict input digests **from the working tree**, with the
/// `toolchain` field left empty.
///
/// Pure hashing — this never runs a subprocess — so the CLI reuses it for
/// staleness reporting (`harness state status`). Digests:
/// - `unit_source`: file-set hash of the unit's files plus their transitive
///   project includes (per `facts`);
/// - `driver`: per-file hash of the `driver` param (empty string when the
///   unit has no `driver` param);
/// - `rust_crate`: the closed-list crate digest (`Cargo.toml`, `Cargo.lock`
///   when present, and `src/**`) per docs/SCHEMAS.md, paths repo-relative to
///   the target root;
/// - `replaces`: the `replaces` param, verbatim.
pub fn compute_inputs(
    target: &TargetContext,
    unit: &Unit,
    facts: &Facts,
) -> Result<VerdictInputs, Error> {
    let ledger = Ledger::new(target.root.clone());
    let closure = facts.include_closure(&unit.files);
    let unit_source = hash::file_set_hash_on_disk(&target.root, &closure)?;
    let driver = match unit.oracle_param_str("driver") {
        Some(rel) => hash::file_hash(&target.root.join(rel))?,
        None => String::new(),
    };
    let crate_dir = ledger
        .unit_dir(&unit.id)
        .join(required_param(unit, "rust_crate")?);
    let rust_crate = hash::unit_crate_file_set_hash(&target.root, &crate_dir)?;
    Ok(VerdictInputs {
        unit_source,
        driver,
        rust_crate,
        replaces: unit.oracle_param_list("replaces"),
        toolchain: Vec::new(),
    })
}

/// The target-level resolution `verify` and `validate_driver` share:
/// validated config and canonical, containment-checked directories. Pure
/// inspection — building it never spawns a process or creates anything.
#[derive(Debug)]
pub(crate) struct Base {
    /// Canonical target root (cwd of every child).
    pub root: PathBuf,
    /// Canonical source dir (`[target] source_dir`), inside `root`.
    pub source_dir: PathBuf,
    /// Canonical `[target] include_dirs`, in order, each inside `source_dir`.
    pub include_dirs: Vec<PathBuf>,
    /// `[oracle] timeout_secs`.
    pub timeout: Duration,
    /// The core-owned tool allowlist.
    pub allowlist: Vec<String>,
}

impl Base {
    /// Validate the tool allowlist (`tools` must all be on it), the timeout,
    /// the unit id, and resolve the source/include dirs.
    pub(crate) fn new(target: &TargetContext, unit: &Unit, tools: &[&str]) -> Result<Base, Error> {
        let timeout = timeout_secs(target)?;
        let allowlist = target.config.oracle_allowlist();
        for tool in tools {
            if !allowlist.iter().any(|a| a == tool) {
                return Err(Error::Invariant(format!(
                    "oracle kind `{C_ABI_DIFFERENTIAL_KIND}` needs `{tool}` on the [oracle] \
                     allowlist in harness.toml (required: {})",
                    tools.join(", ")
                )));
            }
        }
        // The id becomes a path segment under both units/ and build/.
        if !harness_core::plan::is_clean_segment(&unit.id) || unit.id == symbols::BASELINE_DIR {
            return Err(Error::InvalidPlan(format!(
                "unit id {:?} cannot be used as a directory name by the oracle",
                unit.id
            )));
        }
        let root = target
            .root
            .canonicalize()
            .map_err(|e| Error::io(&target.root, e))?;
        let source_dir = inside(
            &unit.id,
            "[target] source_dir",
            &root.join(&target.config.target.source_dir),
            &root,
        )?;
        // Include dirs must stay inside source_dir after symlink resolution
        // too (R2: nothing outside source_dir reaches a compile or a prompt).
        let mut include_dirs = Vec::new();
        for dir in &target.config.target.include_dirs {
            include_dirs.push(inside(
                &unit.id,
                "[target] include_dirs entry",
                &root.join(dir),
                &source_dir,
            )?);
        }
        Ok(Base {
            root,
            source_dir,
            include_dirs,
            timeout,
            allowlist,
        })
    }

    /// `-I` dirs in search order: the source dir, then the include dirs.
    pub(crate) fn includes(&self) -> Vec<PathBuf> {
        std::iter::once(self.source_dir.clone())
            .chain(self.include_dirs.iter().cloned())
            .collect()
    }

    /// Create (if needed) and resolve `(ledger build dir, unit build dir)`.
    /// Build dirs are gitignored scratch, but their location is still
    /// target-controlled (a committed symlink): resolve, then check.
    pub(crate) fn build_dirs(&self, unit_id: &str) -> Result<(PathBuf, PathBuf), Error> {
        let build_root_raw = Ledger::new(self.root.clone()).build_dir();
        std::fs::create_dir_all(&build_root_raw).map_err(|e| Error::io(&build_root_raw, e))?;
        let build_root = inside(unit_id, "ledger build dir", &build_root_raw, &self.root)?;
        let build_raw = build_root.join(unit_id);
        std::fs::create_dir_all(&build_raw).map_err(|e| Error::io(&build_raw, e))?;
        let build = inside(unit_id, "unit build dir", &build_raw, &build_root)?;
        Ok((build_root, build))
    }
}

/// Everything `verify` derives from target-owned input, validated BEFORE any
/// subprocess runs: canonical, containment-checked paths plus the validated
/// kind-owned config.
#[derive(Debug)]
struct Prepared {
    /// Target-level resolution (root, source/include dirs, timeout, tools).
    base: Base,
    /// Canonical ledger build dir, inside `root`.
    build_root: PathBuf,
    /// Canonical unit build dir (`<build_root>/<unit.id>`).
    build: PathBuf,
    /// Canonical unit crate dir, inside `<root>/migration/units/<id>/` —
    /// `None` when the crate does not exist yet (a red `rust-build`).
    crate_dir: Option<PathBuf>,
    /// The crate dir as configured (for messages when it is missing).
    crate_dir_raw: PathBuf,
    /// Canonical driver path, inside `root`.
    driver: PathBuf,
    /// `(plan entry, canonical path)` per `replaces` entry, inside `root`.
    replaces: Vec<(String, PathBuf)>,
    /// Validated `[oracle] extra_link_args`.
    link_args: Vec<String>,
    /// Validated `[oracle.whole_program] args`; `None` = not configured.
    whole_program: Option<Vec<String>>,
}

impl Prepared {
    /// Validate and resolve. Pure filesystem inspection (plus creating the
    /// gitignored build dirs) — never spawns a process.
    fn new(target: &TargetContext, unit: &Unit) -> Result<Prepared, Error> {
        // Config first: a hostile harness.toml is refused before anything
        // else is even looked at.
        let link_args = extra_link_args(target)?;
        let whole_program = whole_program_args(target)?;
        let base = Base::new(target, unit, &REQUIRED_TOOLS)?;
        let driver_rel = required_param(unit, "driver")?;
        let rust_crate = required_param(unit, "rust_crate")?;

        let root = base.root.clone();
        let ledger = Ledger::new(root.clone());
        let inside_root = |what: &str, path: &Path| inside(&unit.id, what, path, &root);

        let driver = inside_root("driver", &root.join(driver_rel))?;
        let mut replaces = Vec::new();
        for rel in unit.oracle_param_list("replaces") {
            let canon = inside_root("replaces entry", &root.join(&rel))?;
            replaces.push((rel, canon));
        }

        let unit_dir = ledger.unit_dir(&unit.id);
        let crate_dir_raw = unit_dir.join(rust_crate);
        let crate_dir = if crate_dir_raw.exists() {
            let canon = inside(&unit.id, "rust_crate", &crate_dir_raw, &unit_dir)?;
            if canon == unit_dir {
                return Err(Error::InvalidPlan(format!(
                    "unit `{}`: rust_crate {rust_crate:?} must name a directory inside {}",
                    unit.id,
                    unit_dir.display()
                )));
            }
            Some(canon)
        } else {
            None
        };

        let (build_root, build) = base.build_dirs(&unit.id)?;
        Ok(Prepared {
            base,
            build_root,
            build,
            crate_dir,
            crate_dir_raw,
            driver,
            replaces,
            link_args,
            whole_program,
        })
    }
}

/// Canonicalize `path` and require the result to be inside `base` (itself
/// canonical): defense in depth behind the plan-load path validation — a
/// symlink committed to the target cannot redirect the oracle elsewhere.
fn inside(unit_id: &str, what: &str, path: &Path, base: &Path) -> Result<PathBuf, Error> {
    let canon = path.canonicalize().map_err(|e| Error::io(path, e))?;
    if !canon.starts_with(base) {
        return Err(Error::InvalidPlan(format!(
            "unit `{unit_id}`: {what} {} resolves to {}, outside {}",
            path.display(),
            canon.display(),
            base.display()
        )));
    }
    Ok(canon)
}

impl OracleStrategy for CAbiDifferential {
    fn kind(&self) -> &'static str {
        C_ABI_DIFFERENTIAL_KIND
    }

    /// Run the checks for `unit` and return a content-bound verdict:
    /// `symbol-set`, `capabilities`, `driver-shape`, `differential-driver`,
    /// `whole-program:<sample>` ×3 (or one `whole-program` "not configured"
    /// check) and `sanitizers` — or a verdict that stops at a lone red
    /// `rust-build`, or at the first red of the three gating checks
    /// (`symbol-set`, `capabilities`, `driver-shape`): nothing of a candidate
    /// or driver that fails one of them is ever linked or run.
    ///
    /// The caller (the CLI) persists the verdict and updates plan status;
    /// this method writes nothing outside the ledger build dir and the unit
    /// crate's own `target/` (and `Cargo.lock`).
    fn verify(&self, target: &TargetContext, unit: &Unit) -> Result<Verdict, Error> {
        // One scrubber for the whole run: every check detail and every error
        // that leaves this method is rewritten through it, so committed
        // evidence carries `<target>`/`<home>`/`<cargo>`/… placeholders, never
        // this machine's absolute paths.
        let scrubber = Scrubber::from_env(&target.root);
        self.run_verify(target, unit, &scrubber)
            .map_err(|e| scrubber.scrub_error(e))
    }
}

impl CAbiDifferential {
    /// The body of [`OracleStrategy::verify`], run under `scrubber`: it scrubs
    /// every [`Check`] detail before the checks enter the verdict, and
    /// [`OracleStrategy::verify`] scrubs any error this returns.
    fn run_verify(
        &self,
        target: &TargetContext,
        unit: &Unit,
        scrubber: &Scrubber,
    ) -> Result<Verdict, Error> {
        let prep = Prepared::new(target, unit)?;
        let facts = load_facts(target)?;
        let root = &prep.base.root;

        // The crate's target dir is created by the harness, so the sandboxed
        // cargo needs no write access to the crate dir itself.
        let crate_target_dir = match &prep.crate_dir {
            Some(dir) => Some(prepare_target_dir(dir)?),
            None => None,
        };

        // Sandbox profiles. Tools (cc, cargo, rustc, nm) may write the unit
        // build dir, the crate's target/ and its Cargo.lock. Built binaries —
        // the only place candidate code executes — run confined (module
        // `confine`): a fresh TMPDIR is their only writable place, and they
        // may read nothing under the home dir or the target root but
        // themselves and their listed inputs, nor exec anything else.
        let host = match sandbox_mode() {
            "sandbox-exec" => Some(HostDirs::from_env()?),
            _ => None,
        };
        let tool_profile = match &host {
            Some(host) => {
                let mut write_dirs = vec![prep.build.clone()];
                write_dirs.extend(crate_target_dir.iter().cloned());
                let write_files: Vec<PathBuf> = prep
                    .crate_dir
                    .iter()
                    .map(|d| d.join("Cargo.lock"))
                    .collect();
                Some(sandbox::render_profile(&ProfileSpec {
                    host,
                    target_root: root,
                    toolchain: true,
                    write_dirs: &write_dirs,
                    write_files: &write_files,
                })?)
            }
            None => None,
        };
        let runner = Runner {
            cwd: root.clone(),
            allowlist: prep.base.allowlist.clone(),
            timeout: prep.base.timeout,
            max_output: exec::DEFAULT_MAX_OUTPUT,
            tool_profile,
        };
        let confined = Confinement {
            runner: &runner,
            host: host.as_ref(),
            target_root: root,
        };

        // The single place check details are scrubbed: every verdict this
        // method returns is built here, so no machine path reaches the ledger.
        let finish = |inputs: VerdictInputs, mut checks: Vec<Check>| -> Verdict {
            for check in &mut checks {
                scrubber.scrub_check(check);
            }
            Verdict::new(unit.id.clone(), inputs, checks)
        };

        let rustc_version = tool_first_line(&runner, &["rustc", "-V"])?;
        let cc_version = tool_first_line(&runner, &["cc", "--version"])?;

        // 1. The unit's Rust staticlib, built inside the crate's own target/
        // (--target-dir pinned explicitly so find_staticlib can never pick up
        // an artifact from elsewhere; --offline because the sandbox has no
        // network and unit crates have no dependencies). A build failure —
        // timeout included — is a CANDIDATE failure, the likeliest failure
        // mode of LLM-written Rust, so it becomes a red check, not a harness
        // error.
        let build_result = match (&prep.crate_dir, &crate_target_dir) {
            (Some(crate_dir), Some(target_dir)) => build_staticlib(
                &runner,
                runner.tool_profile.as_deref(),
                crate_dir,
                target_dir,
            ),
            _ => Err(Error::Invariant(format!(
                "unit crate directory {} does not exist",
                prep.crate_dir_raw.display()
            ))),
        };

        // Digests of the tree actually tested (after the build attempt, so a
        // freshly generated Cargo.lock is part of the rust_crate digest),
        // plus the toolchain identities, the sandbox mode and the cflags.
        let mut inputs = compute_inputs(target, unit, &facts)?;
        inputs.toolchain = vec![
            rustc_version.clone(),
            cc_version,
            format!("sandbox: {}", sandbox_mode()),
            CFLAGS_TOOLCHAIN_ENTRY.to_string(),
        ];

        let rust_lib = match build_result {
            Ok(lib) => lib,
            Err(e) => {
                let checks = vec![Check {
                    name: "rust-build".into(),
                    passed: false,
                    detail: format!("unit crate failed to build: {e}"),
                }];
                return Ok(finish(inputs, checks));
            }
        };

        let mut checks: Vec<Check> = Vec::new();

        // 2. Symbol set: the staticlib must export exactly the unit's
        // symbols. A candidate that also defines `printf` could forge every
        // later check, so a failure here ends the run — it is never linked.
        let (panic_abort, crate_dir) = match &prep.crate_dir {
            Some(dir) => {
                let manifest = dir.join("Cargo.toml");
                let text =
                    std::fs::read_to_string(&manifest).map_err(|e| Error::io(&manifest, e))?;
                (symbols::manifest_sets_panic_abort(&text), dir.clone())
            }
            None => {
                return Err(Error::Invariant(
                    "unit crate built but its directory is gone".into(),
                ))
            }
        };
        let symbol_check = symbols::symbol_set_check(
            &symbols::SymbolCtx {
                runner: &runner,
                host: host.as_ref(),
                root,
                build_root: &prep.build_root,
                rustc_version: &rustc_version,
            },
            &rust_lib,
            panic_abort,
            &unit.symbols,
        )?;
        let symbols_ok = symbol_check.passed;
        checks.push(symbol_check);
        if !symbols_ok {
            return Ok(finish(inputs, checks));
        }

        // 3. Capabilities (R2): the candidate's own code may not reach fs /
        // env / process / net / os / thread / time beyond what the C unit
        // itself uses, nor carry asm. Gating: never linked when red.
        let caps = capabilities::capabilities_check(
            &runner,
            &rust_lib,
            &crate_dir,
            &capabilities::unit_classes(&facts, unit),
        )?;
        let caps_ok = caps.passed;
        checks.push(caps);
        if !caps_ok {
            return Ok(finish(inputs, checks));
        }

        // 4. Driver shape (R1): the driver, compiled alone with the flags of
        // its real builds, may define only `main`, call only the unit and the
        // libc allowlist, carry no weak symbol, and pass the source lint.
        let includes = prep.base.includes();
        let shape_obj = prep.build.join("driver_shape.o");
        let shape_check = match cc_outcome(
            &runner,
            &CcInvocation {
                includes: &includes,
                cflags: &["-c".to_string()],
                quiet: true,
                out: &shape_obj,
                inputs: std::slice::from_ref(&prep.driver),
                libs: &[],
            },
        )? {
            Err(stderr) => shape::not_compiled(&stderr),
            Ok(_) => {
                let syms = shape::object_symbols(&runner, &shape_obj)?;
                let source = std::fs::read(&prep.driver).map_err(|e| Error::io(&prep.driver, e))?;
                let lint = harness_scan::lint_driver(
                    &source,
                    &unit.symbols,
                    &unit_header_names(target, &facts, unit),
                );
                shape::shape_check(shape::object_violations(&syms, &unit.symbols), lint)
            }
        };
        let shape_ok = shape_check.passed;
        checks.push(shape_check);
        if !shape_ok {
            return Ok(finish(inputs, checks));
        }

        let build = &prep.build;
        let replace_paths: Vec<PathBuf> = prep.replaces.iter().map(|(_, p)| p.clone()).collect();

        // 5. Differential driver: C-linked vs Rust-linked, byte-identical
        // stdout. A crash or timeout of either binary is a failed check
        // (evidence), never a harness error.
        let mut drv_c_inputs = vec![prep.driver.clone()];
        drv_c_inputs.extend(replace_paths.iter().cloned());
        let drv_rs_inputs = vec![prep.driver.clone(), rust_lib.clone()];
        let cc = |out: &Path, inputs: &[PathBuf], cflags: &[String], libs: &[String]| {
            cc_compile(
                &runner,
                &CcInvocation {
                    includes: &includes,
                    cflags,
                    quiet: true,
                    out,
                    inputs,
                    libs,
                },
            )
        };
        cc(&build.join("drv_c"), &drv_c_inputs, &[], &[])?;
        cc(&build.join("drv_rs"), &drv_rs_inputs, &[], &[])?;
        match (
            confined.run(&build.join("drv_c"), &[], &[]),
            confined.run(&build.join("drv_rs"), &[], &[]),
        ) {
            (Ok(out_c), Ok(out_rs)) => {
                write_file(&build.join("drv_c.out"), &out_c)?;
                write_file(&build.join("drv_rs.out"), &out_rs)?;
                checks.push(diff_check("differential-driver", &out_c, &out_rs));
            }
            (c, r) => checks.push(run_failure_check("differential-driver", c, r)),
        }

        // 6. Whole-program (opt-in, R8): all C vs (all minus replaces) +
        // staticlib, run with the configured flags over the deterministic
        // samples.
        match &prep.whole_program {
            None => checks.push(Check {
                name: "whole-program".into(),
                passed: true,
                detail: "not configured for this target".into(),
            }),
            Some(args) => {
                checks.extend(self.whole_program(&prep, unit, &confined, args, &rust_lib)?);
            }
        }

        // 7. Sanitizers on the C-side driver (validates driver + baseline).
        let san_flags: Vec<String> = SANITIZER_FLAGS.iter().map(|s| (*s).to_string()).collect();
        let san_bin = build.join("drv_c_san");
        match cc(&san_bin, &drv_c_inputs, &san_flags, &[]) {
            Ok(()) => checks.push(sanitizer_check(confined.run(&san_bin, &[], &[]))),
            Err(e) => checks.push(Check {
                name: "sanitizers".into(),
                passed: false,
                detail: format!("sanitizer build failed: {e}"),
            }),
        }

        Ok(finish(inputs, checks))
    }

    /// The configured whole-program checks: build all-C and mixed, then one
    /// `whole-program:<sample>` check per deterministic sample. Each run may
    /// read exactly its sample; the harness appends the sample path to the
    /// configured flags.
    fn whole_program(
        &self,
        prep: &Prepared,
        unit: &Unit,
        confined: &Confinement<'_>,
        args: &[String],
        rust_lib: &Path,
    ) -> Result<Vec<Check>, Error> {
        let source_dir = &prep.base.source_dir;
        let build = &prep.build;
        let includes = prep.base.includes();
        let cc = |out: &Path, inputs: &[PathBuf]| {
            cc_compile(
                confined.runner,
                &CcInvocation {
                    includes: &includes,
                    cflags: &[],
                    quiet: true,
                    out,
                    inputs,
                    libs: &prep.link_args,
                },
            )
        };
        let mut c_files: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(source_dir).map_err(|e| Error::io(source_dir, e))? {
            let path = entry.map_err(|e| Error::io(source_dir, e))?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("c") {
                // Canonical + contained, like every other compiler input: a
                // symlinked .c must not pull in a file outside the target.
                c_files.push(inside(&unit.id, "source file", &path, &prep.base.root)?);
            }
        }
        c_files.sort();
        c_files.dedup();
        // Every `replaces` entry must actually match a collected C file —
        // otherwise the mixed link silently degenerates to C-vs-C and the
        // check proves nothing.
        let replace_paths: Vec<PathBuf> = prep.replaces.iter().map(|(_, p)| p.clone()).collect();
        for (rel, canon) in &prep.replaces {
            if !c_files.contains(canon) {
                return Err(Error::InvalidPlan(format!(
                    "unit `{}`: replaces entry `{rel}` does not match any .c file in {}",
                    unit.id,
                    source_dir.display()
                )));
            }
        }
        let mixed: Vec<PathBuf> = c_files
            .iter()
            .filter(|p| !replace_paths.contains(p))
            .cloned()
            .chain(std::iter::once(rust_lib.to_path_buf()))
            .collect();
        cc(&build.join("whole_c"), &c_files)?;
        cc(&build.join("whole_mixed"), &mixed)?;
        let mut checks = Vec::new();
        for sample in write_samples(build)? {
            let name = sample
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("sample")
                .to_string();
            let sample_str = path_str(&sample)?.to_string();
            let mut argv: Vec<&str> = args.iter().map(String::as_str).collect();
            argv.push(&sample_str);
            let check_name = format!("whole-program:{name}");
            let inputs = std::slice::from_ref(&sample);
            match (
                confined.run(&build.join("whole_c"), &argv, inputs),
                confined.run(&build.join("whole_mixed"), &argv, inputs),
            ) {
                (Ok(gz_c), Ok(gz_mixed)) => checks.push(diff_check(&check_name, &gz_c, &gz_mixed)),
                (c, r) => checks.push(run_failure_check(&check_name, c, r)),
            }
        }
        Ok(checks)
    }
}

/// The `sanitizers` check from the instrumented driver's run.
///
/// This instruments the C baseline and driver ONLY. Stable Rust has no
/// AddressSanitizer, so the candidate crate's `ffi.rs` shim is not
/// sanitizer-verified here; its safety rests on the compiler-enforced shim
/// structure and the differential/symbol-set checks (see the crate docs).
fn sanitizer_check(run: Result<Vec<u8>, RunFailure>) -> Check {
    let (passed, detail) = match run {
        Ok(_) => (true, "asan+ubsan clean".to_string()),
        Err(timeout @ RunFailure::TimedOut { .. }) => (false, timeout.to_string()),
        Err(RunFailure::Failed(_)) => (false, "sanitizer reported errors".to_string()),
    };
    Check {
        name: "sanitizers".into(),
        passed,
        detail,
    }
}

/// A failed check for a run where at least one side did not exit cleanly
/// (crash, non-zero exit, or timeout — detail `timed out after Ns`).
/// Baseline (C-side) and candidate failures are both evidence — M0's oracle
/// found a real C-baseline SIGBUS exactly this way.
fn run_failure_check(
    name: &str,
    c_side: Result<Vec<u8>, RunFailure>,
    candidate: Result<Vec<u8>, RunFailure>,
) -> Check {
    let mut parts: Vec<String> = Vec::new();
    if let Err(e) = &c_side {
        parts.push(format!("C-side run failed: {e}"));
    }
    if let Err(e) = &candidate {
        parts.push(format!("candidate run failed: {e}"));
    }
    Check {
        name: name.into(),
        passed: false,
        detail: parts.join(" | "),
    }
}

/// A kind-owned `[unit.oracle]` parameter this kind cannot run without.
fn required_param<'a>(unit: &'a Unit, key: &str) -> Result<&'a str, Error> {
    unit.oracle_param_str(key).ok_or_else(|| {
        Error::InvalidPlan(format!(
            "unit `{}`: [unit.oracle] kind `{C_ABI_DIFFERENTIAL_KIND}` requires the `{key}` key",
            unit.id
        ))
    })
}

/// True iff `arg` matches `^-l[A-Za-z0-9_+.-]+$` — the only shape
/// `[oracle] extra_link_args` accepts (docs/SCHEMAS.md "Trust boundaries").
fn is_allowed_link_arg(arg: &str) -> bool {
    arg.strip_prefix("-l").is_some_and(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'))
    })
}

/// The kind-owned `extra_link_args` config key (defaults to empty).
/// `harness.toml` is target-owned, hostile input: anything but `-l<name>`
/// (`-Wl,…`, `-fplugin=…`, `@file`, a path, a non-string) is refused with an
/// error naming the offending entry.
fn extra_link_args(target: &TargetContext) -> Result<Vec<String>, Error> {
    let Some(value) = target.config.oracle.get("extra_link_args") else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or_else(|| {
        Error::InvalidPlan(format!(
            "[oracle] extra_link_args must be an array of `-l<name>` strings, got {value}"
        ))
    })?;
    let mut args = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.as_str() {
            Some(arg) if is_allowed_link_arg(arg) => args.push(arg.to_string()),
            _ => {
                return Err(Error::InvalidPlan(format!(
                    "[oracle] extra_link_args entry {entry} is not allowed: only `-l<name>` \
                     (^-l[A-Za-z0-9_+.-]+$) is accepted"
                )))
            }
        }
    }
    Ok(args)
}

/// The kind-owned `[oracle] timeout_secs` key: wall-clock limit for every
/// child process, in whole seconds (default 120, must be at least 1).
fn timeout_secs(target: &TargetContext) -> Result<Duration, Error> {
    let secs = match target.config.oracle.get("timeout_secs") {
        None => exec::DEFAULT_TIMEOUT_SECS,
        Some(value) => value
            .as_integer()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n >= 1)
            .ok_or_else(|| {
                Error::parse(
                    target.root.join("harness.toml"),
                    format!("[oracle] timeout_secs must be a positive integer, got {value}"),
                )
            })?,
    };
    Ok(Duration::from_secs(secs))
}

/// Byte-compare two outputs into a named check.
fn diff_check(name: &str, a: &[u8], b: &[u8]) -> Check {
    if a == b {
        Check {
            name: name.into(),
            passed: true,
            detail: format!("{} bytes identical", a.len()),
        }
    } else {
        let idx = a
            .iter()
            .zip(b.iter())
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        Check {
            name: name.into(),
            passed: false,
            detail: format!(
                "outputs differ (lens {} vs {}, first diff at byte {idx})",
                a.len(),
                b.len()
            ),
        }
    }
}

/// The kind-owned `[oracle.whole_program]` table (R8): `None` when absent —
/// the whole-program check is opt-in. `args` holds FLAGS only (each matching
/// `^-{1,2}[A-Za-z0-9][A-Za-z0-9-]*$`, at most 4); the harness appends the
/// sample path. Anything else — a path, a value with `=`, a fifth flag, an
/// unknown key, a non-table — is an [`Error::InvalidPlan`] before any
/// subprocess runs.
fn whole_program_args(target: &TargetContext) -> Result<Option<Vec<String>>, Error> {
    let Some(value) = target.config.oracle.get("whole_program") else {
        return Ok(None);
    };
    let table = value.as_table().ok_or_else(|| {
        Error::InvalidPlan(format!(
            "[oracle.whole_program] must be a table with an `args` array, got {value}"
        ))
    })?;
    if let Some(key) = table.keys().find(|k| k.as_str() != "args") {
        return Err(Error::InvalidPlan(format!(
            "[oracle.whole_program] key `{key}` is not supported (only `args`)"
        )));
    }
    let entries = match table.get("args") {
        None => return Ok(Some(Vec::new())),
        Some(v) => v.as_array().ok_or_else(|| {
            Error::InvalidPlan(format!(
                "[oracle.whole_program] args must be an array of flags, got {v}"
            ))
        })?,
    };
    if entries.len() > MAX_WHOLE_PROGRAM_ARGS {
        return Err(Error::InvalidPlan(format!(
            "[oracle.whole_program] args has {} entries; at most {MAX_WHOLE_PROGRAM_ARGS} flags \
             are accepted",
            entries.len()
        )));
    }
    let mut args = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.as_str() {
            Some(flag) if is_allowed_whole_program_flag(flag) => args.push(flag.to_string()),
            _ => {
                return Err(Error::InvalidPlan(format!(
                    "[oracle.whole_program] args entry {entry} is not allowed: only flags \
                     (^-{{1,2}}[A-Za-z0-9][A-Za-z0-9-]*$) are accepted; the harness appends the \
                     sample path itself"
                )))
            }
        }
    }
    Ok(Some(args))
}

/// True iff `flag` matches `^-{1,2}[A-Za-z0-9][A-Za-z0-9-]*$`.
fn is_allowed_whole_program_flag(flag: &str) -> bool {
    let body = flag
        .strip_prefix("--")
        .or_else(|| flag.strip_prefix('-'))
        .unwrap_or("");
    body.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Load the ledger's facts (written by `harness scan`).
pub(crate) fn load_facts(target: &TargetContext) -> Result<Facts, Error> {
    let facts_path = Ledger::new(target.root.clone()).facts_path();
    if !facts_path.exists() {
        return Err(Error::Invariant(format!(
            "facts file missing at {}; run `harness scan` first",
            facts_path.display()
        )));
    }
    Facts::load(&facts_path)
}

/// The quoted-include names a driver may use for the unit's own headers:
/// every header of the unit's include closure as its repo-relative path, its
/// basename, and its path relative to `source_dir` and to each include dir.
pub(crate) fn unit_header_names(target: &TargetContext, facts: &Facts, unit: &Unit) -> Vec<String> {
    let clean = |p: &str| {
        p.split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect::<Vec<_>>()
            .join("/")
    };
    let search: Vec<String> = std::iter::once(&target.config.target.source_dir)
        .chain(target.config.target.include_dirs.iter())
        .map(|d| clean(d))
        .collect();
    let mut names = std::collections::BTreeSet::new();
    for header in facts
        .include_closure(&unit.files)
        .into_iter()
        .filter(|p| p.ends_with(".h"))
    {
        if let Some(base) = header.rsplit('/').next() {
            names.insert(base.to_string());
        }
        for dir in &search {
            if let Some(rel) = header.strip_prefix(&format!("{dir}/")) {
                names.insert(rel.to_string());
            }
        }
        names.insert(header);
    }
    names.into_iter().collect()
}

/// One C compiler invocation. EVERY compile the oracle runs goes through
/// [`cc_argv`], so `-ffp-contract=off` (R4) and the include search order
/// (source dir, then `[target] include_dirs`) are uniform:
/// `cc -ffp-contract=off <cflags> [-O2 unless cflags sets -O*] [-w]
/// -I<dir>… -o <out> <inputs…> <libs…>`.
///
/// `libs` (e.g. `-lm` from `extra_link_args`) go AFTER the inputs: linkers
/// with `--as-needed` defaults (Ubuntu gcc) drop libraries listed before the
/// objects that reference them.
pub(crate) struct CcInvocation<'a> {
    /// `-I` dirs, in search order.
    pub includes: &'a [PathBuf],
    /// Extra flags (`-c`, `-O0`, sanitizers, warnings).
    pub cflags: &'a [String],
    /// Suppress warnings (`-w`); false for the strict driver build.
    pub quiet: bool,
    /// Output path.
    pub out: &'a Path,
    /// Source/object/archive inputs.
    pub inputs: &'a [PathBuf],
    /// Trailing libraries.
    pub libs: &'a [String],
}

/// The argv for `inv` (see [`CcInvocation`]).
pub(crate) fn cc_argv(inv: &CcInvocation<'_>) -> Result<Vec<String>, Error> {
    let mut argv: Vec<String> = vec!["cc".to_string(), FP_CONTRACT_OFF.to_string()];
    argv.extend(inv.cflags.iter().cloned());
    if !inv.cflags.iter().any(|f| f.starts_with("-O")) {
        argv.push("-O2".to_string());
    }
    if inv.quiet {
        argv.push("-w".to_string());
    }
    for dir in inv.includes {
        argv.push(format!("-I{}", path_str(dir)?));
    }
    argv.push("-o".to_string());
    argv.push(path_str(inv.out)?.to_string());
    for input in inv.inputs {
        argv.push(path_str(input)?.to_string());
    }
    argv.extend(inv.libs.iter().cloned());
    Ok(argv)
}

/// Compile; any failure is an `Err` (a harness-side build).
pub(crate) fn cc_compile(runner: &Runner, inv: &CcInvocation<'_>) -> Result<(), Error> {
    runner.tool(&cc_argv(inv)?).map(|_| ())
}

/// Compile where a failure is EVIDENCE: `Ok(Err(stderr excerpt))` when the
/// compiler ran and failed.
pub(crate) fn cc_outcome(
    runner: &Runner,
    inv: &CcInvocation<'_>,
) -> Result<Result<(), String>, Error> {
    Ok(runner.tool_outcome(&cc_argv(inv)?)?.map(|_| ()))
}

/// First stdout line of an allowlisted tool (toolchain identity strings).
fn tool_first_line(runner: &Runner, argv: &[&str]) -> Result<String, Error> {
    let argv: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
    let out = runner.tool(&argv)?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
}

/// Ensure `<crate_dir>/target` exists and resolves inside the (canonical)
/// crate dir; returns its canonical path. The HARNESS creates it — cargo
/// otherwise creates it via a temp sibling inside the crate dir, which the
/// sandbox (writes limited to `target/` and `Cargo.lock`) rightly denies.
pub(crate) fn prepare_target_dir(crate_dir: &Path) -> Result<PathBuf, Error> {
    let target_dir = crate_dir.join("target");
    if !target_dir.exists() {
        std::fs::create_dir_all(&target_dir).map_err(|e| Error::io(&target_dir, e))?;
        // What cargo would have written had it created the directory.
        write_file(
            &target_dir.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55\n\
              # This file is a cache directory tag created by RuHarness on cargo's behalf.\n\
              # For information about cache directory tags see https://bford.info/cachedir/\n",
        )?;
    }
    let canon = target_dir
        .canonicalize()
        .map_err(|e| Error::io(&target_dir, e))?;
    if !canon.starts_with(crate_dir) {
        return Err(Error::InvalidPlan(format!(
            "crate target dir {} resolves to {}, outside {}",
            target_dir.display(),
            canon.display(),
            crate_dir.display()
        )));
    }
    Ok(canon)
}

/// `cargo build --release --offline` of the staticlib crate at `crate_dir`
/// into `target_dir` (from [`prepare_target_dir`]), under `profile`; returns
/// the single `lib*.a` produced.
pub(crate) fn build_staticlib(
    runner: &Runner,
    profile: Option<&str>,
    crate_dir: &Path,
    target_dir: &Path,
) -> Result<PathBuf, Error> {
    let manifest = crate_dir.join("Cargo.toml");
    runner.tool_with_profile(
        &[
            "cargo".to_string(),
            "build".to_string(),
            "--release".to_string(),
            "--offline".to_string(),
            "--manifest-path".to_string(),
            path_str(&manifest)?.to_string(),
            "--target-dir".to_string(),
            path_str(target_dir)?.to_string(),
        ],
        profile,
    )?;
    find_staticlib(&target_dir.join("release"))
}

/// The single `lib*.a` in a crate's `target/release/` directory.
fn find_staticlib(dir: &Path) -> Result<PathBuf, Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    let mut libs: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("lib") && name.ends_with(".a") {
            libs.push(entry.path());
        }
    }
    libs.sort();
    match libs.as_slice() {
        [lib] => Ok(lib.clone()),
        [] => Err(Error::Invariant(format!(
            "no staticlib (lib*.a) in {} — does the unit crate set crate-type = [\"staticlib\"]?",
            dir.display()
        ))),
        many => Err(Error::Invariant(format!(
            "expected exactly one staticlib in {}, found {}",
            dir.display(),
            many.len()
        ))),
    }
}

/// The three deterministic M0 samples, (re)written into the build dir:
/// repeated pangram text (~30KB), a 16KB xorshift64 binary blob, and an
/// empty file. Byte-identical on every run.
fn write_samples(build: &Path) -> Result<Vec<PathBuf>, Error> {
    let text_path = build.join("sample_text.txt");
    let rand_path = build.join("sample_rand.bin");
    let empty_path = build.join("sample_empty");

    let phrase =
        b"the quick brown fox jumps over the lazy dog; pack my box with five dozen liquor jugs.\n";
    let mut text = Vec::with_capacity(32 * 1024);
    while text.len() < 30_000 {
        text.extend_from_slice(phrase);
    }
    write_file(&text_path, &text)?;

    let mut state: u64 = 0x2545F4914F6CDD1D;
    let mut rand = Vec::with_capacity(16 * 1024);
    while rand.len() < 16 * 1024 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        rand.extend_from_slice(&state.to_le_bytes());
    }
    write_file(&rand_path, &rand)?;
    write_file(&empty_path, b"")?;

    Ok(vec![text_path, rand_path, empty_path])
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    std::fs::write(path, bytes).map_err(|e| Error::io(path, e))
}

fn path_str(p: &Path) -> Result<&str, Error> {
    p.to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", p.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::TargetContext;

    fn zopfli_unit() -> Unit {
        toml::from_str(
            r#"
            id = "u001-katajainen"
            status = "pending"
            files = ["src/zopfli/katajainen.c"]

            [oracle]
            kind = "c-abi-differential"
            driver = "migration/units/u001-katajainen/driver.c"
            rust_crate = "katajainen_rs"
            replaces = ["src/zopfli/katajainen.c"]
            "#,
        )
        .expect("unit snippet parses")
    }

    #[test]
    fn kind_string() {
        assert_eq!(CAbiDifferential.kind(), C_ABI_DIFFERENTIAL_KIND);
    }

    #[test]
    fn diff_check_reports_first_difference() {
        let ok = diff_check("x", b"abc", b"abc");
        assert!(ok.passed);
        assert_eq!(ok.detail, "3 bytes identical");

        let bad = diff_check("x", b"abc", b"abd");
        assert!(!bad.passed);
        assert!(
            bad.detail.contains("first diff at byte 2"),
            "{}",
            bad.detail
        );

        let truncated = diff_check("x", b"abc", b"ab");
        assert!(!truncated.passed);
        assert!(truncated.detail.contains("first diff at byte 2"));
    }

    #[test]
    fn missing_required_param_is_an_invalid_plan() {
        let unit: Unit = toml::from_str(
            r#"
            id = "u-x"
            status = "pending"

            [oracle]
            kind = "c-abi-differential"
            "#,
        )
        .expect("unit snippet parses");
        let err = required_param(&unit, "rust_crate").expect_err("must be missing");
        assert!(err.to_string().contains("rust_crate"), "{err}");
    }

    fn context_with_oracle(oracle_toml: &str) -> TargetContext {
        let config: harness_core::TargetConfig = toml::from_str(&format!(
            "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n[oracle]\n{oracle_toml}\n"
        ))
        .expect("config snippet parses");
        TargetContext {
            root: PathBuf::from("/nonexistent/ruharness-target"),
            config,
        }
    }

    #[test]
    fn whole_program_flags_are_flags_only() {
        for ok in ["-c", "--quiet", "-O2", "--i-1", "-9"] {
            assert!(is_allowed_whole_program_flag(ok), "{ok}");
        }
        for bad in [
            "",
            "-",
            "--",
            "---x",
            "c",
            "/etc/passwd",
            "--out=x",
            "-c x",
            "-c\n",
            "-\u{e9}",
            "--_x",
        ] {
            assert!(!is_allowed_whole_program_flag(bad), "{bad:?}");
        }
        assert_eq!(
            whole_program_args(&context_with_oracle("")).expect("absent"),
            None
        );
        assert_eq!(
            whole_program_args(&context_with_oracle(
                "[oracle.whole_program]\nargs = [\"-c\"]"
            ))
            .expect("valid"),
            Some(vec!["-c".to_string()])
        );
        let err = whole_program_args(&context_with_oracle(
            "[oracle.whole_program]\nargs = [\"-c\"]\ncmd = \"sh\"",
        ))
        .expect_err("unknown key");
        assert!(err.to_string().contains("`cmd`"), "{err}");
    }

    /// Every compile carries `-ffp-contract=off` first (R4) and the include
    /// dirs in search order; `quiet` controls `-w`; an explicit `-O*` wins.
    #[test]
    fn cc_argv_is_uniform() {
        let includes = vec![PathBuf::from("/t/src"), PathBuf::from("/t/src/include")];
        let inputs = vec![PathBuf::from("/t/d.c")];
        let argv = cc_argv(&CcInvocation {
            includes: &includes,
            cflags: &["-c".to_string()],
            quiet: true,
            out: Path::new("/t/b/d.o"),
            inputs: &inputs,
            libs: &["-lm".to_string()],
        })
        .expect("argv");
        assert_eq!(
            argv,
            [
                "cc",
                "-ffp-contract=off",
                "-c",
                "-O2",
                "-w",
                "-I/t/src",
                "-I/t/src/include",
                "-o",
                "/t/b/d.o",
                "/t/d.c",
                "-lm"
            ]
        );
        let strict = cc_argv(&CcInvocation {
            includes: &includes,
            cflags: &["-O0".to_string()],
            quiet: false,
            out: Path::new("/t/b/x"),
            inputs: &inputs,
            libs: &[],
        })
        .expect("argv");
        assert!(!strict.contains(&"-w".to_string()), "{strict:?}");
        assert!(!strict.contains(&"-O2".to_string()), "{strict:?}");
        assert_eq!(strict[1], FP_CONTRACT_OFF);
    }

    #[test]
    fn unit_header_names_cover_paths_basenames_and_search_relative_forms() {
        let mut target = context_with_oracle("");
        target.config.target.source_dir = "./src/".into();
        target.config.target.include_dirs = vec!["src/include".into()];
        let facts = Facts {
            frontend: "t".into(),
            files: vec![
                harness_core::facts::FileRecord {
                    path: "src/u.c".into(),
                    hash: String::new(),
                    includes: vec!["src/include/sub/api.h".into()],
                },
                harness_core::facts::FileRecord {
                    path: "src/include/sub/api.h".into(),
                    hash: String::new(),
                    includes: Vec::new(),
                },
            ],
            ..Facts::default()
        };
        let unit: Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"src/u.c\"]\nsymbols = [\"f\"]\n",
        )
        .expect("unit");
        assert_eq!(
            unit_header_names(&target, &facts, &unit),
            vec![
                "api.h".to_string(),
                "include/sub/api.h".to_string(),
                "src/include/sub/api.h".to_string(),
                "sub/api.h".to_string(),
            ]
        );
    }

    #[test]
    fn link_args_accept_only_dash_l_names() {
        for ok in ["-lm", "-lstdc++", "-lfoo_bar", "-lz.1", "-lpthread-2"] {
            assert!(is_allowed_link_arg(ok), "{ok}");
        }
        for bad in [
            "",
            "-l",
            "-L/tmp",
            "-lm -Wl,-rpath,/tmp",
            "-Wl,-e,_evil",
            "-fplugin=/tmp/evil.so",
            "@/tmp/args",
            "/tmp/evil.o",
            "-l/tmp/evil",
            "-lm\n-Wl,x",
            "-l\u{e9}",
            " -lm",
            "-o/tmp/x",
        ] {
            assert!(!is_allowed_link_arg(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn hostile_link_args_are_an_invalid_plan_naming_the_entry() {
        let ok = context_with_oracle("extra_link_args = [\"-lm\", \"-lz\"]");
        assert_eq!(
            extra_link_args(&ok).expect("valid"),
            vec!["-lm".to_string(), "-lz".to_string()]
        );
        assert!(extra_link_args(&context_with_oracle(""))
            .expect("absent key")
            .is_empty());

        for (snippet, offender) in [
            (
                "extra_link_args = [\"-lm\", \"-Wl,-rpath,/tmp\"]",
                "-Wl,-rpath,/tmp",
            ),
            ("extra_link_args = [\"-fplugin=/x.so\"]", "-fplugin=/x.so"),
            ("extra_link_args = [\"-lm\", 7]", "7"),
            ("extra_link_args = \"-lm\"", "-lm"),
        ] {
            let err = extra_link_args(&context_with_oracle(snippet)).expect_err(snippet);
            assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");
            assert!(err.to_string().contains(offender), "{err}");
        }
    }

    #[test]
    fn timeout_defaults_to_120_and_must_be_a_positive_integer() {
        assert_eq!(
            timeout_secs(&context_with_oracle("")).expect("default"),
            Duration::from_secs(120)
        );
        assert_eq!(
            timeout_secs(&context_with_oracle("timeout_secs = 7")).expect("set"),
            Duration::from_secs(7)
        );
        for bad in [
            "timeout_secs = 0",
            "timeout_secs = -5",
            "timeout_secs = \"60\"",
            "timeout_secs = 1.5",
        ] {
            let err = timeout_secs(&context_with_oracle(bad)).expect_err(bad);
            assert!(err.to_string().contains("timeout_secs"), "{err}");
        }
    }

    #[test]
    fn timeouts_of_built_binaries_become_failed_checks() {
        let check = run_failure_check(
            "differential-driver",
            Ok(b"fine".to_vec()),
            Err(RunFailure::TimedOut { secs: 120 }),
        );
        assert!(!check.passed);
        assert_eq!(check.name, "differential-driver");
        assert_eq!(check.detail, "candidate run failed: timed out after 120s");

        let both = run_failure_check(
            "x",
            Err(RunFailure::Failed("boom".into())),
            Err(RunFailure::TimedOut { secs: 3 }),
        );
        assert_eq!(
            both.detail,
            "C-side run failed: boom | candidate run failed: timed out after 3s"
        );

        let san = sanitizer_check(Err(RunFailure::TimedOut { secs: 9 }));
        assert!(!san.passed);
        assert_eq!(san.detail, "timed out after 9s");
        let san = sanitizer_check(Err(RunFailure::Failed("asan: heap-use-after-free".into())));
        assert_eq!(san.detail, "sanitizer reported errors");
        let san = sanitizer_check(Ok(Vec::new()));
        assert!(san.passed);
        assert_eq!(san.detail, "asan+ubsan clean");
    }

    /// A minimal on-disk target for `Prepared::new` (which never spawns).
    fn scaffold(tmp: &testutil::TempDir, oracle_toml: &str) -> TargetContext {
        let root = tmp.path();
        std::fs::write(
            root.join("harness.toml"),
            format!(
                "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n\
                 [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n{oracle_toml}\n"
            ),
        )
        .expect("harness.toml");
        std::fs::create_dir_all(root.join("src")).expect("src");
        std::fs::write(root.join("src/u.c"), "int u(void) { return 1; }\n").expect("u.c");
        let unit_dir = root.join("migration/units/u1");
        std::fs::create_dir_all(unit_dir.join("u_rs/src")).expect("crate");
        std::fs::write(unit_dir.join("driver.c"), "int main(void) { return 0; }\n")
            .expect("driver");
        TargetContext::load(root).expect("target loads")
    }

    fn unit_with(id: &str, driver: &str, rust_crate: &str, replaces: &str) -> Unit {
        toml::from_str(&format!(
            "id = \"{id}\"\nstatus = \"pending\"\nfiles = [\"src/u.c\"]\nsymbols = [\"u\"]\n\
             [oracle]\nkind = \"c-abi-differential\"\ndriver = \"{driver}\"\n\
             rust_crate = \"{rust_crate}\"\nreplaces = [\"{replaces}\"]\n"
        ))
        .expect("unit snippet parses")
    }

    #[test]
    fn prepared_paths_are_canonical_and_inside_the_target() {
        let tmp = testutil::TempDir::new("prep-ok");
        let target = scaffold(&tmp, "extra_link_args = [\"-lm\"]\ntimeout_secs = 30");
        let unit = unit_with("u1", "migration/units/u1/driver.c", "u_rs", "src/u.c");
        let prep = Prepared::new(&target, &unit).expect("valid unit prepares");
        let root = tmp.path();
        assert_eq!(prep.base.root, root);
        assert_eq!(prep.base.source_dir, root.join("src"));
        assert_eq!(prep.driver, root.join("migration/units/u1/driver.c"));
        assert_eq!(
            prep.crate_dir.as_deref(),
            Some(root.join("migration/units/u1/u_rs").as_path())
        );
        assert_eq!(prep.build_root, root.join("migration/build"));
        assert_eq!(prep.build, root.join("migration/build/u1"));
        assert!(prep.build.is_dir());
        assert_eq!(
            prep.replaces,
            vec![("src/u.c".to_string(), root.join("src/u.c"))]
        );
        assert_eq!(prep.link_args, vec!["-lm".to_string()]);
        assert_eq!(prep.base.timeout, Duration::from_secs(30));
    }

    #[test]
    fn a_missing_unit_crate_is_not_a_harness_error() {
        let tmp = testutil::TempDir::new("prep-nocrate");
        let target = scaffold(&tmp, "");
        let unit = unit_with("u1", "migration/units/u1/driver.c", "not_there", "src/u.c");
        let prep = Prepared::new(&target, &unit).expect("prepares");
        assert!(prep.crate_dir.is_none());
        assert!(prep.crate_dir_raw.ends_with("migration/units/u1/not_there"));
    }

    #[test]
    fn traversal_in_plan_fields_is_an_invalid_plan() {
        let tmp = testutil::TempDir::new("prep-dotdot");
        let target = scaffold(&tmp, "");
        let outside = tmp.path().parent().expect("temp parent");
        let stray = format!("ruharness-outside-{}.c", std::process::id());
        std::fs::write(outside.join(&stray), "int x;\n").expect("outside file");
        let cases = [
            unit_with("u1", &format!("../{stray}"), "u_rs", "src/u.c"),
            unit_with(
                "u1",
                "migration/units/u1/driver.c",
                "u_rs",
                &format!("../{stray}"),
            ),
            // Inside the root, but not inside migration/units/u1/.
            unit_with(
                "u1",
                "migration/units/u1/driver.c",
                "../../../src",
                "src/u.c",
            ),
            unit_with("u1", "migration/units/u1/driver.c", ".", "src/u.c"),
        ];
        for unit in &cases {
            let err = Prepared::new(&target, unit).expect_err("must be refused");
            assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");
        }
        let _ = std::fs::remove_file(outside.join(&stray));

        for bad_id in ["../u1", "a/b", "", ".hidden", "symbol-baseline"] {
            let unit = unit_with(bad_id, "migration/units/u1/driver.c", "u_rs", "src/u.c");
            let err = Prepared::new(&target, &unit).expect_err(bad_id);
            assert!(matches!(err, Error::InvalidPlan(_)), "{bad_id:?}: {err:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_redirect_the_oracle_outside_the_target() {
        use std::os::unix::fs::symlink;
        let outside = testutil::TempDir::new("prep-outside");
        std::fs::create_dir_all(outside.path().join("evil_rs/src")).expect("outside crate");
        std::fs::write(outside.path().join("evil.c"), "int x;\n").expect("outside file");

        // Driver, replaces entry and crate dir each symlinked out of the tree.
        let tmp = testutil::TempDir::new("prep-symlink");
        let target = scaffold(&tmp, "");
        let unit_dir = tmp.path().join("migration/units/u1");
        symlink(outside.path().join("evil.c"), unit_dir.join("link.c")).expect("symlink");
        symlink(outside.path().join("evil.c"), tmp.path().join("src/link.c")).expect("symlink");
        symlink(outside.path().join("evil_rs"), unit_dir.join("link_rs")).expect("symlink");
        for unit in [
            unit_with("u1", "migration/units/u1/link.c", "u_rs", "src/u.c"),
            unit_with("u1", "migration/units/u1/driver.c", "u_rs", "src/link.c"),
            unit_with("u1", "migration/units/u1/driver.c", "link_rs", "src/u.c"),
        ] {
            let err = Prepared::new(&target, &unit).expect_err("symlink escape");
            assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");
            assert!(err.to_string().contains("outside"), "{err}");
        }

        // A crate whose target/ points elsewhere is refused before cargo runs.
        symlink(outside.path(), unit_dir.join("u_rs/target")).expect("symlink");
        let err = prepare_target_dir(&unit_dir.join("u_rs")).expect_err("target escape");
        assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");

        // A committed `migration/build` symlink cannot move the build dir.
        let tmp2 = testutil::TempDir::new("prep-buildlink");
        let target2 = scaffold(&tmp2, "");
        symlink(outside.path(), tmp2.path().join("migration/build")).expect("symlink");
        let unit = unit_with("u1", "migration/units/u1/driver.c", "u_rs", "src/u.c");
        let err = Prepared::new(&target2, &unit).expect_err("build dir escape");
        assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");
        assert!(
            !outside.path().join("u1").exists(),
            "nothing created outside"
        );
    }

    #[test]
    fn the_allowlist_must_cover_every_tool_of_the_kind() {
        let tmp = testutil::TempDir::new("prep-allow");
        let target = scaffold(&tmp, "");
        let mut no_nm = target.clone();
        no_nm.config.oracle.insert(
            "allowlist".into(),
            toml::Value::Array(
                ["cc", "cargo", "rustc"]
                    .iter()
                    .map(|s| toml::Value::String((*s).into()))
                    .collect(),
            ),
        );
        let unit = unit_with("u1", "migration/units/u1/driver.c", "u_rs", "src/u.c");
        let err = Prepared::new(&no_nm, &unit).expect_err("nm missing");
        assert!(err.to_string().contains("`nm`"), "{err}");
    }

    /// Link-arg validation comes before everything else in `verify` — with a
    /// hostile arg the oracle fails as an invalid plan even though the target
    /// has no facts, no crate and no sources, i.e. before any subprocess.
    #[test]
    fn verify_refuses_hostile_link_args_before_running_anything() {
        let tmp = testutil::TempDir::new("prep-linkarg");
        let target = scaffold(&tmp, "extra_link_args = [\"-lm\", \"-Wl,-e,_evil\"]");
        let unit = unit_with("u1", "migration/units/u1/driver.c", "u_rs", "src/u.c");
        let err = CAbiDifferential
            .verify(&target, &unit)
            .expect_err("hostile link arg");
        assert!(matches!(err, Error::InvalidPlan(_)), "{err:?}");
        assert!(err.to_string().contains("-Wl,-e,_evil"), "{err}");
        assert!(
            !tmp.path().join("migration/build").exists(),
            "nothing may be created or run before config validation"
        );
    }

    #[test]
    fn prepared_target_dir_is_created_by_the_harness_with_a_cachedir_tag() {
        let tmp = testutil::TempDir::new("prep-target");
        let dir = prepare_target_dir(tmp.path()).expect("created");
        assert_eq!(dir, tmp.path().join("target"));
        let tag = std::fs::read_to_string(dir.join("CACHEDIR.TAG")).expect("tag");
        assert!(tag.starts_with("Signature: 8a477f597d28d172789f06886806bc55"));
        // Idempotent on an existing dir.
        assert_eq!(prepare_target_dir(tmp.path()).expect("again"), dir);
    }

    /// Smoke test of `compute_inputs` against the real zopfli target.
    /// Guarded on the facts file's existence (a sibling M1 task generates
    /// `facts.jsonl`, so the test must pass either way): real facts when
    /// present, otherwise a minimal in-memory equivalent covering the unit.
    #[test]
    fn smoke_compute_inputs_zopfli() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli");
        let target = TargetContext::load(&root).expect("harness.toml loads");
        let facts_path = Ledger::new(target.root.clone()).facts_path();
        let facts = if facts_path.exists() {
            Facts::load(&facts_path).expect("facts.jsonl loads")
        } else {
            Facts {
                frontend: "test-inline".to_string(),
                files: vec![harness_core::facts::FileRecord {
                    path: "src/zopfli/katajainen.c".to_string(),
                    hash: String::new(),
                    includes: vec!["src/zopfli/katajainen.h".to_string()],
                }],
                ..Facts::default()
            }
        };
        let unit = zopfli_unit();

        let inputs = compute_inputs(&target, &unit, &facts).expect("compute_inputs");
        assert!(
            inputs.unit_source.starts_with("blake3:"),
            "{}",
            inputs.unit_source
        );
        assert!(inputs.driver.starts_with("blake3:"), "{}", inputs.driver);
        assert!(
            inputs.rust_crate.starts_with("blake3:"),
            "{}",
            inputs.rust_crate
        );
        assert_eq!(inputs.replaces, vec!["src/zopfli/katajainen.c".to_string()]);
        assert!(inputs.toolchain.is_empty());

        // Deterministic: identical tree, identical digests.
        let again = compute_inputs(&target, &unit, &facts).expect("compute_inputs again");
        assert_eq!(inputs.unit_source, again.unit_source);
        assert_eq!(inputs.driver, again.driver);
        assert_eq!(inputs.rust_crate, again.rust_crate);
    }
}
