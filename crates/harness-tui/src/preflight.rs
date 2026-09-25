//! The read preflight (docs/MCP-DESIGN.md §4 "Size, and what is read at
//! all"; docs/COCKPIT-WRAPPER-DESIGN.md §6.3): checked BEFORE a snapshot is
//! read, so a hostile target cannot hang, exhaust or steer a reader. One
//! implementation for both clients of the read model: harness-mcp runs it
//! before every tool call; the cockpit before every load (on its loader
//! thread) and again when a Re-check, Accept, Retry or Resume is confirmed
//! (those confirms read the ledger and hash a crate). Std and harness-core
//! only.

use std::path::{Path, PathBuf};

/// Largest `facts.jsonl` / `plan.toml` read: they grow with the project.
pub const MAX_PROJECT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest other ledger file read (a record, a verdict, the config).
pub const MAX_LEDGER_FILE_BYTES: u64 = 1024 * 1024;
/// Largest crate tree (`src/**` plus the manifests) indexed or hashed.
pub const MAX_CRATE_BYTES: u64 = 32 * 1024 * 1024;
/// Largest total of the ledger files a snapshot holds in memory at once.
pub const MAX_RETAINED_BYTES: u64 = 256 * 1024 * 1024;
/// Largest total a read hashes (every unit's include closure and driver,
/// twice — its status and its binding —, every facts file, the unit
/// crates): harness-mcp's loop answers nothing else while a read runs (the
/// cockpit reads on a loader thread), so its work is bounded (a few
/// seconds). Revisit when a real target needs more.
pub const MAX_HASHED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Most files `facts.jsonl` may record.
pub const MAX_FACTS_FILES: usize = 50_000;
/// Most (plan unit × facts file) pairs a read walks: every unit's include
/// closure indexes the facts once.
pub const MAX_UNIT_FACT_PAIRS: u64 = 50_000_000;
/// Most (plan bytes × plan units) a read may parse: the status re-reads the
/// plan for every unit that looks inconsistent.
pub const MAX_PLAN_PARSE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Checked BEFORE anything is read, so a hostile target cannot hang,
/// exhaust or steer the server (docs/MCP-DESIGN.md §4 "Size", §R2 TRUST-2,
/// TRUST-5): every file the read model reads or hashes in-process is a
/// regular file within its cap — never a FIFO or a device, which would block
/// or never end.
/// - Ledger files (`harness.toml`, `migration/**` records and verdicts) are
///   never symlinks: the harness writes regular files, and a link could
///   name a file outside the target whose content a parse error would quote.
/// - Every path `facts.jsonl` and `plan.toml` name (sources, includes,
///   drivers) is relative with normal components only — an absolute one
///   would replace the root on `join`, a `..` leave it — and, followed, a
///   regular file. (Any file name the scanner records is fine: `_priv.h`.)
/// - Crate trees (the unit crate, every candidate and hand edit) hold no
///   symlink under `src/` (the hash walk follows links: a loop would never
///   end) — dotfiles aside, which the hash and the index skip — and are
///   within [`MAX_CRATE_BYTES`].
/// - What a snapshot keeps in memory is within [`MAX_RETAINED_BYTES`], and
///   what a read hashes within [`MAX_HASHED_BYTES`].
///
/// An entry that vanishes during the walk is skipped (the CLI replaces
/// candidates while an act runs: a read meanwhile must not be refused). A
/// file that changes between this check and the read is not covered
/// (bounded reads in harness-core are §7 "Later"); a concurrent hostile
/// writer is outside the threat model. `Err` names the first file that
/// fails.
pub fn preflight(target: &Path) -> Result<(), String> {
    let mut retained = 0u64;
    let mut hashed = 0u64;
    let migration = target.join("migration");
    let units = migration.join("units");
    real_dir(&migration)?;
    real_dir(&units)?;
    retained += ledger(&target.join("harness.toml"), MAX_LEDGER_FILE_BYTES)?;
    let facts_path = migration.join("facts.jsonl");
    let plan_path = migration.join("plan.toml");
    retained += ledger(&facts_path, MAX_PROJECT_FILE_BYTES)?;
    retained += ledger(&plan_path, MAX_PROJECT_FILE_BYTES)?;
    for dir in entries(&units)? {
        if !real_dir(&dir)? {
            continue;
        }
        for name in ["oracle-latest.json", "superseded.jsonl"] {
            retained += ledger(&dir.join(name), MAX_LEDGER_FILE_BYTES)?;
        }
        for records in ["attempts", "driver-attempts"] {
            let records = dir.join(records);
            if !real_dir(&records)? {
                continue;
            }
            for a in entries(&records)? {
                if !real_dir(&a)? {
                    continue;
                }
                for name in ["attempt.json", "attempt-verdict.json"] {
                    retained += ledger(&a.join(name), MAX_LEDGER_FILE_BYTES)?;
                }
                for sub in ["candidate", harness_core::attempts::HUMAN_EDIT_DIR] {
                    crate_tree(&a.join(sub))?;
                }
            }
        }
    }
    if retained > MAX_RETAINED_BYTES {
        return Err(format!(
            "the ledger holds {retained} bytes of records and verdicts (> \
             {MAX_RETAINED_BYTES}); not read"
        ));
    }
    // What is hashed: every path the facts and the plan name.
    let mut sizes: std::collections::BTreeMap<String, u64> = Default::default();
    let mut sized = |rel: &str| -> Result<u64, String> {
        if let Some(n) = sizes.get(rel) {
            return Ok(*n);
        }
        let n = source(target, rel)?;
        sizes.insert(rel.to_string(), n);
        Ok(n)
    };
    let too_much = |what: &str| format!("a read of this target would {what}; not read");
    let facts = if facts_path.exists() {
        let facts = harness_core::facts::Facts::load(&facts_path).map_err(|e| e.to_string())?;
        if facts.files.len() > MAX_FACTS_FILES {
            return Err(too_much(&format!(
                "index more than {MAX_FACTS_FILES} files"
            )));
        }
        for f in &facts.files {
            hashed += sized(&f.path)?;
            for inc in &f.includes {
                sized(inc)?;
            }
        }
        // Every facts file is hashed once per read, with or without a plan.
        if hashed > MAX_HASHED_BYTES {
            return Err(too_much(&format!(
                "hash more than {MAX_HASHED_BYTES} bytes"
            )));
        }
        Some(facts)
    } else {
        None
    };
    if plan_path.exists() {
        let plan = harness_core::plan::Plan::load(&plan_path).map_err(|e| e.to_string())?;
        let units_n = plan.units.len() as u64;
        let plan_bytes = std::fs::metadata(&plan_path).map_or(0, |m| m.len());
        if plan_bytes.saturating_mul(units_n) > MAX_PLAN_PARSE_BYTES {
            return Err(too_much(&format!(
                "parse more than {MAX_PLAN_PARSE_BYTES} bytes of plan"
            )));
        }
        let facts_n = facts.as_ref().map_or(0, |f| f.files.len() as u64);
        if units_n.saturating_mul(facts_n) > MAX_UNIT_FACT_PAIRS {
            return Err(too_much(&format!(
                "walk more than {MAX_UNIT_FACT_PAIRS} unit-and-file pairs"
            )));
        }
        for unit in &plan.units {
            let closure = match &facts {
                Some(facts) => facts.include_closure(&unit.files),
                None => unit.files.clone(),
            };
            let mut unit_bytes = 0;
            for f in &closure {
                unit_bytes += sized(f)?;
            }
            match unit.oracle_param_str("driver") {
                Some(rel) => unit_bytes += sized(rel)?,
                None => {
                    if harness_core::plan::is_clean_segment(&unit.id) {
                        let driver = units.join(&unit.id).join("driver.c");
                        unit_bytes += ledger(&driver, MAX_PROJECT_FILE_BYTES)?;
                    }
                }
            }
            hashed = hashed.saturating_add(unit_bytes.saturating_mul(2));
            if let Some(name) = unit.oracle_param_str("rust_crate") {
                if harness_core::plan::is_clean_segment(&unit.id)
                    && harness_core::plan::is_clean_segment(name)
                {
                    // Hashed for the provenance, the status and its re-check.
                    hashed += 3 * crate_tree(&units.join(&unit.id).join(name))?;
                }
            }
            if hashed > MAX_HASHED_BYTES {
                return Err(too_much(&format!(
                    "hash more than {MAX_HASHED_BYTES} bytes"
                )));
            }
        }
    }
    Ok(())
}

/// The entries of `dir` (none when it is absent or vanished).
fn entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => Ok(rd.flatten().map(|e| e.path()).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// `true` when `path` is a real directory, `false` when absent or a plain
/// file; a symlink is refused.
fn real_dir(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink; not read (the ledger is never linked)",
            path.display()
        )),
        Ok(meta) => Ok(meta.is_dir()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// A ledger file: absent, or a regular file (never a symlink) within `cap`.
/// Its size.
fn ledger(path: &Path, cap: u64) -> Result<u64, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !meta.file_type().is_file() {
        return Err(format!(
            "{} is not a regular file (a symlink, FIFO or device); not read",
            path.display()
        ));
    }
    if meta.len() > cap {
        return Err(format!(
            "{} is too large ({} bytes > {cap}); not read",
            path.display(),
            meta.len()
        ));
    }
    Ok(meta.len())
}

/// Whether `rel` stays inside the root on `join`: relative, and only
/// normal components (no `/`, `..`, `.` or prefix).
fn stays_inside(rel: &str) -> bool {
    use std::path::Component;
    let path = Path::new(rel);
    !rel.is_empty()
        && !rel.contains('\0')
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

/// A path the facts or the plan name: it stays inside the root, and —
/// followed, as the hash reads it — it is absent or a regular file within
/// the project cap. Its size.
fn source(target: &Path, rel: &str) -> Result<u64, String> {
    if !stays_inside(rel) {
        return Err(format!(
            "the ledger names a path that leaves the target (absolute, or `..`): {:?}; not read",
            rel.chars().take(120).collect::<String>()
        ));
    }
    let path = target.join(rel);
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !meta.is_file() {
        return Err(format!(
            "{} is not a regular file (a FIFO or device?); not read",
            path.display()
        ));
    }
    if meta.len() > MAX_PROJECT_FILE_BYTES {
        return Err(format!(
            "{} is too large ({} bytes > {MAX_PROJECT_FILE_BYTES}); not read",
            path.display(),
            meta.len()
        ));
    }
    Ok(meta.len())
}

/// [`crate_tree`] alone: the checks a crate passes before it is hashed or
/// copied on the UI thread (the cockpit's hand edit), without the whole
/// preflight. Its size, or why not.
pub fn check_crate(dir: &Path) -> Result<u64, String> {
    crate_tree(dir)
}

/// A crate tree: absent, or `Cargo.toml`/`Cargo.lock` and every entry
/// under `src/` (dotfiles aside) regular and never a symlink, within
/// [`MAX_CRATE_BYTES`]. Its size.
fn crate_tree(dir: &Path) -> Result<u64, String> {
    if !real_dir(dir)? {
        return Ok(0);
    }
    let mut total = 0u64;
    for name in ["Cargo.toml", "Cargo.lock"] {
        total += ledger(&dir.join(name), MAX_CRATE_BYTES)?;
    }
    let mut stack = vec![dir.join("src")];
    while let Some(d) = stack.pop() {
        if !real_dir(&d)? {
            continue;
        }
        for path in entries(&d)? {
            // The hash and the index skip dotfiles (an editor's lock link).
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.file_type().is_file() {
                total += meta.len();
            } else {
                return Err(format!(
                    "{} is not a regular file (a symlink, FIFO or device); not read",
                    path.display()
                ));
            }
        }
        if total > MAX_CRATE_BYTES {
            return Err(format!(
                "the crate at {} is too large (> {MAX_CRATE_BYTES} bytes); not read",
                dir.display()
            ));
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory removed on drop — also when a test fails (a
    /// mutation run fails many on purpose).
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let dir =
                std::env::temp_dir().join(format!("harness-tui-pf-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TmpDir(dir)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A copy of the zopfli target (its ledger, config and sources).
    fn zopfli_copy(dst: &Path) -> PathBuf {
        fn copy(src: &Path, dst: &Path) {
            std::fs::create_dir_all(dst).unwrap();
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                if name == "build" || name == "target" || name == ".lock" {
                    continue;
                }
                let (from, to) = (entry.path(), dst.join(&name));
                if from.is_dir() {
                    copy(&from, &to);
                } else {
                    std::fs::copy(&from, &to).unwrap();
                }
            }
        }
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        copy(&repo.join("targets/zopfli"), dst);
        dst.canonicalize().unwrap()
    }

    fn fifo(path: &Path) {
        let _ = std::fs::remove_file(path);
        assert!(std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success());
    }

    fn sparse(path: &Path, len: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(len).unwrap();
    }

    /// §R2 VC-1, VC-7: what the CLI accepts, the preflight accepts — any
    /// file name the scanner records, an editor's lock link beside a crate
    /// source (a dotfile the hash and the index skip), the committed ledgers.
    #[test]
    fn the_preflight_passes_what_the_cli_reads() {
        let _guard = TmpDir::new("preflight-ok");
        let base = _guard.0.canonicalize().unwrap();
        let t = zopfli_copy(&base.join("zopfli"));
        let f = t.join("migration/facts.jsonl");
        let text = std::fs::read_to_string(&f).unwrap();
        std::fs::write(
            &f,
            text.replacen(
                "\"includes\":[",
                "\"includes\":[\"src/zopfli/_priv h+.h\",",
                1,
            ),
        )
        .unwrap();
        std::fs::write(t.join("src/zopfli/_priv h+.h"), "/* ok */\n").unwrap();
        let src = t.join("migration/units/u001-katajainen/katajainen_rs/src");
        std::os::unix::fs::symlink("beaumorton@host.1234", src.join(".#lib.rs")).unwrap();
        preflight(&t).unwrap();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        preflight(&repo.join("targets/zopfli")).unwrap();
        for case in std::fs::read_dir(repo.join("targets/tractor/cases/Public-Tests/B01_organic"))
            .unwrap()
            .flatten()
            .take(10)
        {
            preflight(&case.path()).unwrap();
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// §R2 TESTS-4: every file the preflight guards, one at a time — each
    /// case fails the check on a fresh copy, and the copy passes without it.
    #[test]
    fn the_preflight_guards_every_file_the_read_model_reads() {
        let _guard = TmpDir::new("preflight");
        let base = _guard.0.canonicalize().unwrap();
        let unit = "u001-katajainen";
        type Case = (&'static str, &'static str, fn(&Path));
        let cases: Vec<Case> = vec![
            ("harness.toml too large", "too large", |t| {
                sparse(&t.join("harness.toml"), MAX_LEDGER_FILE_BYTES + 1)
            }),
            ("facts too large", "too large", |t| {
                sparse(&t.join("migration/facts.jsonl"), MAX_PROJECT_FILE_BYTES + 1)
            }),
            ("plan too large", "too large", |t| {
                sparse(&t.join("migration/plan.toml"), MAX_PROJECT_FILE_BYTES + 1)
            }),
            ("verdict too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/oracle-latest.json"),
                    MAX_LEDGER_FILE_BYTES + 1,
                )
            }),
            ("supersessions too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/superseded.jsonl"),
                    MAX_LEDGER_FILE_BYTES + 1,
                )
            }),
            ("record too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/attempts/a-1/attempt.json"),
                    MAX_LEDGER_FILE_BYTES + 1,
                )
            }),
            ("attempt verdict too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/attempts/a-1/attempt-verdict.json"),
                    MAX_LEDGER_FILE_BYTES + 1,
                )
            }),
            ("driver a FIFO", "not a regular file", |t| {
                fifo(&t.join("migration/units/u001-katajainen/driver.c"))
            }),
            ("a C source a FIFO", "not a regular file", |t| {
                fifo(&t.join("src/zopfli/katajainen.c"))
            }),
            ("a header a device", "not a regular file", |t| {
                let h = t.join("src/zopfli/katajainen.h");
                std::fs::remove_file(&h).unwrap();
                std::os::unix::fs::symlink("/dev/zero", &h).unwrap();
            }),
            ("a facts path absolute", "leaves the target", |t| {
                let f = t.join("migration/facts.jsonl");
                let text = std::fs::read_to_string(&f).unwrap();
                std::fs::write(
                    &f,
                    text.replacen(
                        "\"path\":\"src/zopfli/blocksplitter.c\"",
                        "\"path\":\"/dev/zero\"",
                        1,
                    ),
                )
                .unwrap();
            }),
            ("the config a symlink", "not a regular file", |t| {
                let c = t.join("harness.toml");
                let outside = t.parent().unwrap().join("outside.toml");
                std::fs::rename(&c, &outside).unwrap();
                std::os::unix::fs::symlink(&outside, &c).unwrap();
            }),
            ("the plan a symlink", "not a regular file", |t| {
                let p = t.join("migration/plan.toml");
                let outside = t.parent().unwrap().join("outside-plan.toml");
                std::fs::rename(&p, &outside).unwrap();
                std::os::unix::fs::symlink(&outside, &p).unwrap();
            }),
            ("a unit dir a symlink", "symlink", |t| {
                let d = t.join("migration/units/u-linked");
                std::os::unix::fs::symlink(t.join("migration/units/u001-katajainen"), &d).unwrap();
            }),
            (
                "the unit crate's src holds a link",
                "not a regular file",
                |t| {
                    let src = t.join("migration/units/u001-katajainen/katajainen_rs/src");
                    std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
                },
            ),
            (
                "a candidate's src holds a link",
                "not a regular file",
                |t| {
                    let src = t.join("migration/units/u001-katajainen/attempts/a-1/candidate/src");
                    std::fs::create_dir_all(&src).unwrap();
                    std::os::unix::fs::symlink("/etc/hosts", src.join("lib.rs")).unwrap();
                },
            ),
            ("a crate too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/katajainen_rs/src/big.rs"),
                    MAX_CRATE_BYTES + 1,
                )
            }),
            ("a driver record too large", "too large", |t| {
                sparse(
                    &t.join("migration/units/u001-katajainen/driver-attempts/d-1/attempt.json"),
                    MAX_LEDGER_FILE_BYTES + 1,
                )
            }),
            (
                "an include (not itself a facts file) a FIFO",
                "not a regular file",
                |t| {
                    let f = t.join("migration/facts.jsonl");
                    let text = std::fs::read_to_string(&f).unwrap();
                    std::fs::write(
                        &f,
                        text.replacen("\"includes\":[", "\"includes\":[\"src/zopfli/extra.h\",", 1),
                    )
                    .unwrap();
                    fifo(&t.join("src/zopfli/extra.h"));
                },
            ),
            (
                "a plan unit file (not in the facts) a FIFO",
                "not a regular file",
                |t| {
                    let p = t.join("migration/plan.toml");
                    let text = std::fs::read_to_string(&p).unwrap();
                    std::fs::write(
                        &p,
                        text.replacen(
                            "files = [\"src/zopfli/katajainen.c\"]",
                            "files = [\"src/zopfli/katajainen.c\", \"src/zopfli/planned.c\"]",
                            1,
                        ),
                    )
                    .unwrap();
                    fifo(&t.join("src/zopfli/planned.c"));
                },
            ),
            ("a default driver a FIFO", "not a regular file", |t| {
                // u-cache names no driver: its default is units/u-cache/driver.c.
                let d = t.join("migration/units/u-cache");
                std::fs::create_dir_all(&d).unwrap();
                fifo(&d.join("driver.c"));
            }),
            ("a crate manifest a FIFO", "not a regular file", |t| {
                fifo(&t.join("migration/units/u001-katajainen/katajainen_rs/Cargo.toml"))
            }),
            (
                "a hand edit's src holds a link",
                "not a regular file",
                |t| {
                    let src = t.join("migration/units/u001-katajainen/attempts/a-1/edit/src");
                    std::fs::create_dir_all(&src).unwrap();
                    std::os::unix::fs::symlink("/etc/hosts", src.join("logic.rs")).unwrap();
                },
            ),
            ("a facts path with `..`", "leaves the target", |t| {
                let f = t.join("migration/facts.jsonl");
                let text = std::fs::read_to_string(&f).unwrap();
                std::fs::write(
                    &f,
                    text.replacen(
                        "\"path\":\"src/zopfli/blocksplitter.c\"",
                        "\"path\":\"src/../../x.c\"",
                        1,
                    ),
                )
                .unwrap();
            }),
            ("too much to hash", "would hash more than", |t| {
                // Units that all name one big (sparse) file: hashed twice each.
                sparse(&t.join("src/zopfli/big.c"), MAX_PROJECT_FILE_BYTES - 1);
                let p = t.join("migration/plan.toml");
                let mut text = std::fs::read_to_string(&p).unwrap();
                let n = MAX_HASHED_BYTES / (2 * (MAX_PROJECT_FILE_BYTES - 1)) + 1;
                for i in 0..n {
                    text.push_str(&format!(
                        "\n[[unit]]\nid = \"u-big-{i}\"\nstatus = \"pending\"\n\
                         files = [\"src/zopfli/big.c\"]\nsource_hash = \"blake3:00\"\n\
                         symbols = []\ndepends_on = []\n"
                    ));
                }
                std::fs::write(&p, text).unwrap();
            }),
            (
                "too much to hash in the facts alone (no plan units)",
                "hash more than",
                |t| {
                    sparse(&t.join("src/zopfli/big.c"), MAX_PROJECT_FILE_BYTES - 1);
                    let f = t.join("migration/facts.jsonl");
                    let mut text = std::fs::read_to_string(&f).unwrap();
                    let n = MAX_HASHED_BYTES / (MAX_PROJECT_FILE_BYTES - 1) + 1;
                    for _ in 0..n {
                        text.push_str(
                            "{\"k\":\"file\",\"path\":\"src/zopfli/big.c\",\"hash\":\"blake3:00\",\
                         \"includes\":[]}\n",
                        );
                    }
                    std::fs::write(&f, text).unwrap();
                    let p = t.join("migration/plan.toml");
                    let plan = std::fs::read_to_string(&p).unwrap();
                    let head = plan.split("[[unit]]").next().unwrap().to_string();
                    std::fs::write(&p, head).unwrap();
                },
            ),
            ("too many facts files", "index more than", |t| {
                let f = t.join("migration/facts.jsonl");
                let mut text = std::fs::read_to_string(&f).unwrap();
                for i in 0..=MAX_FACTS_FILES {
                    text.push_str(&format!(
                        "{{\"k\":\"file\",\"path\":\"src/gen/f{i}.c\",\"hash\":\"blake3:00\",\
                         \"includes\":[]}}\n"
                    ));
                }
                std::fs::write(&f, text).unwrap();
            }),
            ("too many unit-and-file pairs", "unit-and-file pairs", |t| {
                let f = t.join("migration/facts.jsonl");
                let mut text = std::fs::read_to_string(&f).unwrap();
                for i in 0..25_000 {
                    text.push_str(&format!(
                        "{{\"k\":\"file\",\"path\":\"src/gen/f{i}.c\",\"hash\":\"blake3:00\",\
                         \"includes\":[]}}\n"
                    ));
                }
                std::fs::write(&f, text).unwrap();
                let p = t.join("migration/plan.toml");
                let mut plan = std::fs::read_to_string(&p).unwrap();
                for i in 0..2_000 {
                    plan.push_str(&format!(
                        "\n[[unit]]\nid = \"u-gen-{i}\"\nstatus = \"pending\"\n\
                         files = [\"src/zopfli/katajainen.c\"]\nsource_hash = \"blake3:00\"\n\
                         symbols = []\ndepends_on = []\n"
                    ));
                }
                std::fs::write(&p, plan).unwrap();
            }),
            (
                "a plan too large to re-parse per unit",
                "parse more than",
                |t| {
                    let p = t.join("migration/plan.toml");
                    let mut text = std::fs::read_to_string(&p).unwrap();
                    for i in 0..20_000 {
                        text.push_str(&format!(
                            "\n[[unit]]\nid = \"u-gen-{i}\"\nstatus = \"pending\"\n\
                         files = [\"src/zopfli/katajainen.c\"]\nsource_hash = \"blake3:00\"\n\
                         symbols = []\ndepends_on = []\n"
                        ));
                    }
                    std::fs::write(&p, text).unwrap();
                },
            ),
            ("too many records in all", "holds", |t| {
                let n = MAX_RETAINED_BYTES / MAX_LEDGER_FILE_BYTES + 1;
                for i in 0..n {
                    sparse(
                        &t.join(format!(
                            "migration/units/u001-katajainen/attempts/a-{i}/attempt.json"
                        )),
                        MAX_LEDGER_FILE_BYTES,
                    );
                }
            }),
        ];
        for (i, (what, why, spoil)) in cases.into_iter().enumerate() {
            let t = zopfli_copy(&base.join(format!("case-{i}/zopfli")));
            std::fs::create_dir_all(t.join("migration/units").join(unit).join("attempts/a-1"))
                .unwrap();
            assert!(preflight(&t).is_ok(), "{what}: the fresh copy passes");
            spoil(&t);
            let err = preflight(&t).expect_err(what);
            assert!(err.contains(why), "{what}: {err}");
            let _ = std::fs::remove_dir_all(base.join(format!("case-{i}")));
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
