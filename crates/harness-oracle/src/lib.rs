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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use harness_core::config::TargetContext;
use harness_core::error::Error;
use harness_core::hash;
use harness_core::ledger::Ledger;
use harness_core::traits::OracleStrategy;
use harness_core::verdict::{Check, VerdictInputs};
use harness_core::{Facts, Unit, Verdict};
use std::path::{Path, PathBuf};
use std::process::Command;

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
/// Kind-owned config (in `harness.toml` `[oracle]`): `extra_link_args`, a
/// list of extra `cc` arguments for the whole-program link (e.g. `["-lm"]`).
/// The core-owned `allowlist` key governs which named executables may run.
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

impl OracleStrategy for CAbiDifferential {
    fn kind(&self) -> &'static str {
        C_ABI_DIFFERENTIAL_KIND
    }

    /// Run the three M0 checks for `unit` and return a content-bound verdict.
    ///
    /// The caller (the CLI) persists the verdict and updates plan status;
    /// this method writes nothing outside the ledger build dir and the unit
    /// crate's own `target/`.
    fn verify(&self, target: &TargetContext, unit: &Unit) -> Result<Verdict, Error> {
        let ledger = Ledger::new(target.root.clone());
        let facts_path = ledger.facts_path();
        if !facts_path.exists() {
            return Err(Error::Invariant(format!(
                "facts file missing at {}; run `harness scan` first",
                facts_path.display()
            )));
        }
        let facts = Facts::load(&facts_path)?;

        let root = &target.root;
        let allowlist = target.config.oracle_allowlist();
        let driver_rel = required_param(unit, "driver")?.to_string();
        let rust_crate = required_param(unit, "rust_crate")?.to_string();
        let replaces = unit.oracle_param_list("replaces");
        let source_dir = root.join(&target.config.target.source_dir);
        let build = ledger.build_dir().join(&unit.id);
        std::fs::create_dir_all(&build).map_err(|e| Error::io(&build, e))?;

        // 1. The unit's Rust staticlib, built inside the crate's own target/
        // (--target-dir pinned explicitly so an inherited CARGO_TARGET_DIR
        // can never make find_staticlib pick up a stale artifact). A build
        // failure is a CANDIDATE failure — the likeliest failure mode of
        // LLM-written Rust — so it becomes a red check, not a harness error.
        let crate_dir = ledger.unit_dir(&unit.id).join(&rust_crate);
        let manifest = crate_dir.join("Cargo.toml");
        let crate_target_dir = crate_dir.join("target");
        let build_result = exec_tool(
            root,
            &allowlist,
            &[
                "cargo".to_string(),
                "build".to_string(),
                "--release".to_string(),
                "--manifest-path".to_string(),
                path_str(&manifest)?.to_string(),
                "--target-dir".to_string(),
                path_str(&crate_target_dir)?.to_string(),
            ],
        )
        .and_then(|_| find_staticlib(&crate_target_dir.join("release")));

        // Digests of the tree actually tested (after the build attempt, so a
        // freshly generated Cargo.lock is part of the rust_crate digest),
        // plus the toolchain identities.
        let mut inputs = compute_inputs(target, unit, &facts)?;
        inputs.toolchain = vec![
            tool_first_line(root, &allowlist, &["rustc", "-V"])?,
            tool_first_line(root, &allowlist, &["cc", "--version"])?,
        ];

        let rust_lib = match build_result {
            Ok(lib) => lib,
            Err(e) => {
                let checks = vec![Check {
                    name: "rust-build".into(),
                    passed: false,
                    detail: format!("unit crate failed to build: {e}"),
                }];
                return Ok(Verdict::new(unit.id.clone(), inputs, checks));
            }
        };

        let mut checks: Vec<Check> = Vec::new();
        let driver = root.join(&driver_rel);
        let replace_paths: Vec<PathBuf> = replaces.iter().map(|r| root.join(r)).collect();

        // 2. Differential driver: C-linked vs Rust-linked, byte-identical
        // stdout. A crash of either binary is a failed check (evidence),
        // never a harness error.
        let mut drv_c_inputs = vec![driver.clone()];
        drv_c_inputs.extend(replace_paths.iter().cloned());
        let drv_rs_inputs = vec![driver, rust_lib.clone()];
        cc_compile(
            root,
            &allowlist,
            &source_dir,
            &build.join("drv_c"),
            &drv_c_inputs,
            &[],
            &[],
        )?;
        cc_compile(
            root,
            &allowlist,
            &source_dir,
            &build.join("drv_rs"),
            &drv_rs_inputs,
            &[],
            &[],
        )?;
        match (
            run_built(root, &build.join("drv_c"), &[]),
            run_built(root, &build.join("drv_rs"), &[]),
        ) {
            (Ok(out_c), Ok(out_rs)) => {
                write_file(&build.join("drv_c.out"), &out_c)?;
                write_file(&build.join("drv_rs.out"), &out_rs)?;
                checks.push(diff_check("differential-driver", &out_c, &out_rs));
            }
            (c, r) => checks.push(run_failure_check("differential-driver", c, r)),
        }

        // 3. Whole-program: all C vs (all minus replaces) + staticlib, run
        // over the deterministic samples.
        let mut c_files: Vec<PathBuf> = std::fs::read_dir(&source_dir)
            .map_err(|e| Error::io(&source_dir, e))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("c"))
            .collect();
        c_files.sort();
        // Every `replaces` entry must actually match a collected C file —
        // otherwise the mixed link silently degenerates to C-vs-C and the
        // check proves nothing.
        for (rel, p) in replaces.iter().zip(&replace_paths) {
            let canon = p.canonicalize().map_err(|e| Error::io(p, e))?;
            let matched = c_files
                .iter()
                .any(|c| c.canonicalize().map(|cc| cc == canon).unwrap_or(false));
            if !matched {
                return Err(Error::InvalidPlan(format!(
                    "unit `{}`: replaces entry `{rel}` does not match any .c file in {}",
                    unit.id,
                    source_dir.display()
                )));
            }
        }
        let canon_replaces: Vec<PathBuf> = replace_paths
            .iter()
            .map(|p| p.canonicalize().map_err(|e| Error::io(p, e)))
            .collect::<Result<_, _>>()?;
        let mixed: Vec<PathBuf> = c_files
            .iter()
            .filter(|p| {
                p.canonicalize()
                    .map(|cc| !canon_replaces.contains(&cc))
                    .unwrap_or(true)
            })
            .cloned()
            .chain(std::iter::once(rust_lib))
            .collect();
        let link_args = extra_link_args(target);
        cc_compile(
            root,
            &allowlist,
            &source_dir,
            &build.join("whole_c"),
            &c_files,
            &[],
            &link_args,
        )?;
        cc_compile(
            root,
            &allowlist,
            &source_dir,
            &build.join("whole_mixed"),
            &mixed,
            &[],
            &link_args,
        )?;
        for sample in write_samples(&build)? {
            let name = sample
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("sample")
                .to_string();
            let sample_str = path_str(&sample)?.to_string();
            let check_name = format!("whole-program:{name}");
            match (
                run_built(root, &build.join("whole_c"), &["-c", &sample_str]),
                run_built(root, &build.join("whole_mixed"), &["-c", &sample_str]),
            ) {
                (Ok(gz_c), Ok(gz_mixed)) => checks.push(diff_check(&check_name, &gz_c, &gz_mixed)),
                (c, r) => checks.push(run_failure_check(&check_name, c, r)),
            }
        }

        // 4. Sanitizers on the C-side driver (validates driver + baseline).
        let san_flags: Vec<String> = [
            "-fsanitize=address,undefined",
            "-fno-sanitize-recover=all",
            "-g",
            "-O1",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let san_bin = build.join("drv_c_san");
        match cc_compile(
            root,
            &allowlist,
            &source_dir,
            &san_bin,
            &drv_c_inputs,
            &san_flags,
            &[],
        ) {
            Ok(()) => {
                let passed = run_built(root, &san_bin, &[]).is_ok();
                checks.push(Check {
                    name: "sanitizers".into(),
                    passed,
                    detail: if passed {
                        "asan+ubsan clean".into()
                    } else {
                        "sanitizer reported errors".into()
                    },
                });
            }
            Err(e) => checks.push(Check {
                name: "sanitizers".into(),
                passed: false,
                detail: format!("sanitizer build failed: {e}"),
            }),
        }

        Ok(Verdict::new(unit.id.clone(), inputs, checks))
    }
}

/// A failed check for a run where at least one side did not exit cleanly.
/// Baseline (C-side) and candidate failures are both evidence — M0's oracle
/// found a real C-baseline SIGBUS exactly this way.
fn run_failure_check(
    name: &str,
    c_side: Result<Vec<u8>, Error>,
    candidate: Result<Vec<u8>, Error>,
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

/// The kind-owned `extra_link_args` config key (defaults to empty).
fn extra_link_args(target: &TargetContext) -> Vec<String> {
    target
        .config
        .oracle
        .get("extra_link_args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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

/// Compile with cc:
/// `cc <cflags> [-O2 unless cflags sets -O*] -w -I<dir> -o <out> <inputs…> <libs…>`.
///
/// `libs` (e.g. `-lm` from `extra_link_args`) go AFTER the inputs: linkers
/// with `--as-needed` defaults (Ubuntu gcc) drop libraries listed before the
/// objects that reference them.
fn cc_compile(
    root: &Path,
    allowlist: &[String],
    include_dir: &Path,
    out: &Path,
    inputs: &[PathBuf],
    cflags: &[String],
    libs: &[String],
) -> Result<(), Error> {
    let mut argv: Vec<String> = vec!["cc".to_string()];
    argv.extend(cflags.iter().cloned());
    if !cflags.iter().any(|f| f.starts_with("-O")) {
        argv.push("-O2".to_string());
    }
    argv.push("-w".to_string());
    argv.push(format!("-I{}", path_str(include_dir)?));
    argv.push("-o".to_string());
    argv.push(path_str(out)?.to_string());
    for input in inputs {
        argv.push(path_str(input)?.to_string());
    }
    argv.extend(libs.iter().cloned());
    exec_tool(root, allowlist, &argv).map(|_| ())
}

/// Run a named tool with explicit argv, checked against the `[oracle]`
/// allowlist. Errors (with stderr) on non-zero exit. Returns stdout bytes.
fn exec_tool(root: &Path, allowlist: &[String], argv: &[String]) -> Result<Vec<u8>, Error> {
    let exe = argv
        .first()
        .ok_or_else(|| Error::Invariant("oracle: empty argv".into()))?;
    if !allowlist.iter().any(|a| a == exe) {
        return Err(Error::Invariant(format!(
            "executable `{exe}` is not on the [oracle] allowlist in harness.toml"
        )));
    }
    let output = Command::new(exe)
        .args(&argv[1..])
        .current_dir(root)
        .output()
        .map_err(|e| Error::Invariant(format!("spawning {exe}: {e}")))?;
    if !output.status.success() {
        return Err(Error::Invariant(format!(
            "`{}` failed ({}):\n{}",
            argv.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

/// Run a binary the oracle just built (inside the build dir or the unit
/// crate's `target/`) — run by path, exempt from the name allowlist. Errors
/// (with stderr) on non-zero exit. Returns stdout bytes.
fn run_built(root: &Path, bin: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = Command::new(bin)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| Error::Invariant(format!("spawning {}: {e}", bin.display())))?;
    if !output.status.success() {
        return Err(Error::Invariant(format!(
            "`{} {}` failed ({}):\n{}",
            bin.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

/// First stdout line of an allowlisted tool (toolchain identity strings).
fn tool_first_line(root: &Path, allowlist: &[String], argv: &[&str]) -> Result<String, Error> {
    let argv: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
    let out = exec_tool(root, allowlist, &argv)?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
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
