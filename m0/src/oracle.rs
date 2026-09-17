//! M0 oracle for unit u001-katajainen (§3.5): differential driver, mixed
//! whole-program build, sanitizer run. Deterministic, no LLM involvement.
//!
//! Subprocess discipline (§12.2, M0 subset): explicit argv arrays, an
//! executable allowlist, working directory pinned to the repo root, writes
//! confined to `migration/build/`, `migration/units/`, and `target/`.
//! Container/network isolation is deferred to M1 (recorded in DECISIONS.md).

use std::path::{Path, PathBuf};
use std::process::Command;

const ALLOWED_EXES: &[&str] = &["cc", "cargo"];

struct CheckResult {
    name: String,
    passed: bool,
    detail: String,
}

pub fn run(root: &Path) -> Result<(), String> {
    let src = root.join("targets/zopfli/src/zopfli");
    let unit = root.join("targets/zopfli/migration/units/u001-katajainen");
    let build = root.join("targets/zopfli/migration/build");
    std::fs::create_dir_all(&build).map_err(|e| format!("mkdir build: {e}"))?;

    let mut checks: Vec<CheckResult> = Vec::new();

    // 1. Rust staticlib for the unit.
    exec(
        root,
        &["cargo", "build", "--release", "-p", "katajainen_rs"],
        true,
    )?;
    let rust_lib = root.join("target/release/libkatajainen_rs.a");
    if !rust_lib.exists() {
        return Err(format!(
            "expected staticlib missing: {}",
            rust_lib.display()
        ));
    }

    // 2. Differential driver: C-linked vs Rust-linked, byte-identical stdout.
    let driver = unit.join("driver.c");
    let kata_c = src.join("katajainen.c");
    cc(root, &src, &build.join("drv_c"), &[&driver, &kata_c], &[])?;
    cc(
        root,
        &src,
        &build.join("drv_rs"),
        &[&driver, &rust_lib],
        &[],
    )?;
    let out_c = exec_path(root, &build.join("drv_c"), &[])?;
    let out_rs = exec_path(root, &build.join("drv_rs"), &[])?;
    std::fs::write(build.join("drv_c.out"), &out_c).map_err(|e| e.to_string())?;
    std::fs::write(build.join("drv_rs.out"), &out_rs).map_err(|e| e.to_string())?;
    checks.push(diff_check("differential-driver", &out_c, &out_rs));

    // 3. Whole-program: zopfli all-C vs mixed C/Rust, byte-identical output.
    let mut c_files: Vec<PathBuf> = std::fs::read_dir(&src)
        .map_err(|e| format!("reading {}: {e}", src.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("c"))
        .collect();
    c_files.sort();
    let all_c: Vec<&Path> = c_files.iter().map(|p| p.as_path()).collect();
    let mixed: Vec<&Path> = c_files
        .iter()
        .filter(|p| p.file_name().and_then(|f| f.to_str()) != Some("katajainen.c"))
        .map(|p| p.as_path())
        .chain(std::iter::once(rust_lib.as_path()))
        .collect();
    cc(root, &src, &build.join("zopfli_c"), &all_c, &["-lm"])?;
    cc(root, &src, &build.join("zopfli_mixed"), &mixed, &["-lm"])?;

    let samples = write_samples(&build)?;
    for sample in &samples {
        let name = sample
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("sample");
        let sample_str = path_str(sample)?;
        let gz_c = exec_path(root, &build.join("zopfli_c"), &["-c", sample_str])?;
        let gz_mixed = exec_path(root, &build.join("zopfli_mixed"), &["-c", sample_str])?;
        checks.push(diff_check(
            &format!("whole-program:{name}"),
            &gz_c,
            &gz_mixed,
        ));
    }

    // 4. Sanitizers on the C-side driver (validates driver + baseline).
    let san_flags = [
        "-fsanitize=address,undefined",
        "-fno-sanitize-recover=all",
        "-g",
        "-O1",
    ];
    let san_bin = build.join("drv_c_san");
    let san_build = cc(root, &src, &san_bin, &[&driver, &kata_c], &san_flags);
    match san_build {
        Ok(()) => {
            let passed = exec_path(root, &san_bin, &[]).is_ok();
            checks.push(CheckResult {
                name: "sanitizers".into(),
                passed,
                detail: if passed {
                    "asan+ubsan clean".into()
                } else {
                    "sanitizer reported errors".into()
                },
            });
        }
        Err(e) => checks.push(CheckResult {
            name: "sanitizers".into(),
            passed: false,
            detail: format!("sanitizer build failed: {e}"),
        }),
    }

    report(&unit, &checks)?;
    let failed: Vec<_> = checks.iter().filter(|c| !c.passed).collect();
    for c in &checks {
        println!(
            "oracle: [{}] {} — {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    if failed.is_empty() {
        println!("oracle: VERDICT GREEN ({} checks)", checks.len());
        Ok(())
    } else {
        Err(format!(
            "oracle: VERDICT RED — {} of {} checks failed",
            failed.len(),
            checks.len()
        ))
    }
}

fn diff_check(name: &str, a: &[u8], b: &[u8]) -> CheckResult {
    if a == b {
        CheckResult {
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
        CheckResult {
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

/// Compile with cc: `cc <flags> -O2 -I<src> -o <out> <inputs...>`.
fn cc(
    root: &Path,
    include_dir: &Path,
    out: &Path,
    inputs: &[&Path],
    extra: &[&str],
) -> Result<(), String> {
    let include = format!("-I{}", path_str(include_dir)?);
    let out_s = path_str(out)?.to_string();
    let mut argv: Vec<&str> = vec!["cc"];
    argv.extend(extra);
    if !extra.iter().any(|f| f.starts_with("-O")) {
        argv.push("-O2");
    }
    argv.extend(["-w", &include, "-o", &out_s]);
    let input_strs: Vec<&str> = inputs
        .iter()
        .map(|p| path_str(p))
        .collect::<Result<_, _>>()?;
    argv.extend(input_strs);
    exec(root, &argv, true).map(|_| ())
}

/// Run an allowlisted executable; error (with stderr) on non-zero exit when
/// `require_success`. Returns stdout bytes.
fn exec(root: &Path, argv: &[&str], require_success: bool) -> Result<Vec<u8>, String> {
    let exe = argv[0];
    if !ALLOWED_EXES.contains(&exe) {
        return Err(format!("executable `{exe}` not on the oracle allowlist"));
    }
    let output = Command::new(exe)
        .args(&argv[1..])
        .current_dir(root)
        .output()
        .map_err(|e| format!("spawning {exe}: {e}"))?;
    if require_success && !output.status.success() {
        return Err(format!(
            "`{}` failed ({}):\n{}",
            argv.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

/// Run a binary we just built inside `migration/build/` (not on the general
/// allowlist — it is confined to that directory by construction).
fn exec_path(root: &Path, bin: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(bin)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("spawning {}: {e}", bin.display()))?;
    if !output.status.success() {
        return Err(format!(
            "`{} {}` failed ({}):\n{}",
            bin.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn write_samples(build: &Path) -> Result<Vec<PathBuf>, String> {
    let text_path = build.join("sample_text.txt");
    let rand_path = build.join("sample_rand.bin");
    let empty_path = build.join("sample_empty");

    let phrase =
        b"the quick brown fox jumps over the lazy dog; pack my box with five dozen liquor jugs.\n";
    let mut text = Vec::with_capacity(32 * 1024);
    while text.len() < 30_000 {
        text.extend_from_slice(phrase);
    }
    std::fs::write(&text_path, &text).map_err(|e| e.to_string())?;

    let mut state: u64 = 0x2545F4914F6CDD1D;
    let mut rand = Vec::with_capacity(16 * 1024);
    while rand.len() < 16 * 1024 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        rand.extend_from_slice(&state.to_le_bytes());
    }
    std::fs::write(&rand_path, &rand).map_err(|e| e.to_string())?;
    std::fs::write(&empty_path, b"").map_err(|e| e.to_string())?;

    Ok(vec![text_path, rand_path, empty_path])
}

fn report(unit: &Path, checks: &[CheckResult]) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut md = String::from("# Oracle result — u001-katajainen\n\n");
    md.push_str(&format!(
        "Run: `cargo run -p harness-m0 -- oracle` (unix time {now})\n\n"
    ));
    for c in checks {
        md.push_str(&format!(
            "- **{}**: {} — {}\n",
            c.name,
            if c.passed { "PASS" } else { "FAIL" },
            c.detail
        ));
    }
    let verdict = if checks.iter().all(|c| c.passed) {
        "GREEN"
    } else {
        "RED"
    };
    md.push_str(&format!("\nVerdict: **{verdict}**\n"));
    std::fs::write(unit.join("oracle-latest.md"), md).map_err(|e| e.to_string())
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", p.display()))
}
