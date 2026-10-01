//! perf's builds (docs/PERF-DESIGN.md §3.2): as verify builds them — the C
//! with `-O2 -ffp-contract=off` through the same `cc` invocation, its
//! objects compiled once into `.perf/obj/`, each side one link into its
//! slot; every object, staticlib and binary hashed after its build and
//! checked before the first run; the scratch folders made by helpers that
//! refuse links.

use crate::exec::Runner;
use crate::{inside, Base, CcInvocation};
use harness_core::error::Error;
use harness_core::ledger::Ledger;
use std::path::{Path, PathBuf};

/// perf's scratch folder in `migration/build/`, made fresh each run.
pub(crate) const PERF_BUILD_DIR: &str = ".perf";
/// The kept outputs of behaves-differently rows, in `migration/build/`.
pub(crate) const PERF_OUT_DIR: &str = ".perf-out";
/// The runs' logs, in `migration/build/` (build note 29): never recreated
/// by the `.perf` helper, the last [`KEPT_LOGS`] kept.
pub(crate) const PERF_LOGS_DIR: &str = "perf-logs";
/// How many run logs are kept.
pub(crate) const KEPT_LOGS: usize = 20;
/// Most units a plan may hold for perf's slots.
pub(crate) const MAX_SLOTS: usize = 999;

/// `migration/build/` resolved inside `root` (made when missing).
fn build_root(root: &Path) -> Result<PathBuf, Error> {
    let raw = Ledger::new(root.to_path_buf()).build_dir();
    std::fs::create_dir_all(&raw).map_err(|e| Error::io(&raw, e))?;
    inside("perf", "ledger build dir", &raw, root)
}

/// A folder in `migration/build/`: a link or a non-folder in its place is
/// removed; `fresh` empties it too. Canonical and contained.
fn build_folder(root: &Path, name: &str, fresh: bool) -> Result<PathBuf, Error> {
    let build_root = build_root(root)?;
    let raw = build_root.join(name);
    match std::fs::symlink_metadata(&raw) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            std::fs::remove_file(&raw).map_err(|e| Error::io(&raw, e))?
        }
        Ok(_) if fresh => std::fs::remove_dir_all(&raw).map_err(|e| Error::io(&raw, e))?,
        Ok(_) => {}
        Err(_) => {}
    }
    if !raw.is_dir() {
        std::fs::create_dir(&raw).map_err(|e| Error::io(&raw, e))?;
    }
    inside("perf", name, &raw, &build_root)
}

/// `migration/build/.perf/`, made fresh.
pub(crate) fn perf_scratch(root: &Path) -> Result<PathBuf, Error> {
    build_folder(root, PERF_BUILD_DIR, true)
}

/// `migration/build/.perf-out/`, kept between runs.
pub(crate) fn perf_out(root: &Path) -> Result<PathBuf, Error> {
    build_folder(root, PERF_OUT_DIR, false)
}

/// `migration/build/perf-logs/`, kept between runs; the oldest logs past
/// [`KEPT_LOGS`] are removed.
pub(crate) fn perf_logs(root: &Path) -> Result<PathBuf, Error> {
    let dir = build_folder(root, PERF_LOGS_DIR, false)?;
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map_err(|e| Error::io(&dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    logs.sort();
    let extra = logs.len().saturating_sub(KEPT_LOGS.saturating_sub(1));
    for (_, path) in logs.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
    Ok(dir)
}

/// A folder inside a resolved folder (a link or file in its place
/// removed), canonical and contained.
pub(crate) fn sub_folder(parent: &Path, name: &str) -> Result<PathBuf, Error> {
    let raw = parent.join(name);
    match std::fs::symlink_metadata(&raw) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            std::fs::remove_file(&raw).map_err(|e| Error::io(&raw, e))?
        }
        _ => {}
    }
    if !raw.is_dir() {
        std::fs::create_dir_all(&raw).map_err(|e| Error::io(&raw, e))?;
    }
    inside("perf", name, &raw, parent)
}

/// A side's slot (§3.2): `.perf/bin/<slot>/<name>`, four characters each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// `p000`: the C.
    C,
    /// `p001`…`p999`: the unit at that 1-based plan position.
    Unit(usize),
    /// `pall`: the program as it stands.
    AsItStands,
}

impl Slot {
    /// The slot's folder name; a plan position past [`MAX_SLOTS`] is refused
    /// by name.
    pub(crate) fn name(self) -> Result<String, Error> {
        match self {
            Slot::C => Ok("p000".into()),
            Slot::AsItStands => Ok("pall".into()),
            Slot::Unit(n) if (1..=MAX_SLOTS).contains(&n) => Ok(format!("p{n:03}")),
            Slot::Unit(_) => Err(Error::InvalidPlan(format!(
                "perf measures a plan of at most {MAX_SLOTS} units"
            ))),
        }
    }
}

/// A file perf built, with its bytes' blake3 taken right after the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hashed {
    pub path: PathBuf,
    pub digest: String,
}

impl Hashed {
    /// Hash `path` now.
    pub(crate) fn new(path: &Path) -> Result<Hashed, Error> {
        Ok(Hashed {
            path: path.to_path_buf(),
            digest: harness_core::hash::file_hash(path)?,
        })
    }

    /// Refuse when the file changed since it was built.
    pub(crate) fn check(&self) -> Result<(), Error> {
        let now = harness_core::hash::file_hash(&self.path)?;
        if now == self.digest {
            Ok(())
        } else {
            Err(Error::Invariant(format!(
                "{} changed after perf built it — measure again",
                self.path.display()
            )))
        }
    }
}

/// verify's whole-program compile: `cc` with the C's flags, the target's
/// include dirs, `inputs` and the target's `extra_link_args` (shared by
/// verify's whole-program and feature steps and by perf).
pub(crate) fn whole_cc_into(
    base: &Base,
    link_args: &[String],
    runner: &Runner,
    out: &Path,
    inputs: &[PathBuf],
) -> Result<(), Error> {
    let includes = base.includes();
    crate::cc_compile(
        runner,
        &CcInvocation {
            includes: &includes,
            cflags: &[],
            quiet: true,
            out,
            inputs,
            libs: link_args,
        },
    )
}

/// Compile each C file once into `obj_dir` (`<stem>.o`), with the flags
/// [`whole_cc_into`] gives them; `Ok(Err(first lines))` when the compiler
/// ran and failed — a set-up failure of the C, in its own words.
pub(crate) fn compile_objects(
    base: &Base,
    runner: &Runner,
    c_files: &[PathBuf],
    obj_dir: &Path,
) -> Result<Result<Vec<Hashed>, String>, Error> {
    let includes = base.includes();
    let mut out = Vec::with_capacity(c_files.len());
    for c in c_files {
        let stem = c
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Invariant(format!("{}: not a UTF-8 name", c.display())))?;
        let obj = obj_dir.join(format!("{stem}.o"));
        let cflags = ["-c".to_string()];
        let inv = CcInvocation {
            includes: &includes,
            cflags: &cflags,
            quiet: true,
            out: &obj,
            inputs: std::slice::from_ref(c),
            libs: &[],
        };
        if let Err(words) = crate::cc_outcome(runner, &inv)? {
            return Ok(Err(words));
        }
        out.push(Hashed::new(&obj)?);
    }
    Ok(Ok(out))
}

/// Link one side into `slot_dir/<name>`: `objects` and `libs` (staticlibs,
/// in order) with the target's `extra_link_args`; `group` wraps the
/// staticlibs in `--start-group … --end-group` (GNU ld). `Ok(Err(first
/// lines))` when the link ran and failed.
pub(crate) fn link_side(
    base: &Base,
    link_args: &[String],
    runner: &Runner,
    out: &Path,
    objects: &[PathBuf],
    libs: &[PathBuf],
    group: bool,
) -> Result<Result<Hashed, String>, Error> {
    let includes = base.includes();
    let mut inputs: Vec<PathBuf> = objects.to_vec();
    let mut tail: Vec<String> = Vec::new();
    if group && libs.len() > 1 {
        tail.push("-Wl,--start-group".into());
        for lib in libs {
            tail.push(crate::path_str(lib)?.to_string());
        }
        tail.push("-Wl,--end-group".into());
    } else {
        inputs.extend(libs.iter().cloned());
    }
    tail.extend(link_args.iter().cloned());
    let inv = CcInvocation {
        includes: &includes,
        cflags: &[],
        quiet: true,
        out,
        inputs: &inputs,
        libs: &tail,
    };
    match crate::cc_outcome(runner, &inv)? {
        Ok(()) => Ok(Ok(Hashed::new(out)?)),
        Err(words) => Ok(Err(words)),
    }
}

/// The C files a unit's program keeps: every one of `c_files` but the
/// unit's `replaces` — or the (0-based) index of the first entry that
/// names no file among them, which would make the program the C alone
/// (verify's whole-program and feature steps refuse it the same way).
pub(crate) fn kept_c_files(
    c_files: &[PathBuf],
    replaces: &[PathBuf],
) -> Result<Vec<PathBuf>, usize> {
    if let Some(i) = replaces.iter().position(|r| !c_files.contains(r)) {
        return Err(i);
    }
    Ok(c_files
        .iter()
        .filter(|c| !replaces.contains(c))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C built from objects compiled once and linked into a slot runs
    /// as verify's one-step build does; a unit's program links its
    /// staticlib in place of its `replaces`.
    #[test]
    fn objects_once_then_a_link_per_side() {
        let bench = crate::testutil::ToolBench::new("perf-build");
        let root = bench.root().canonicalize().expect("root");
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(&p, text).expect("write");
        };
        put(
            "harness.toml",
            "schema_version = 1\n[target]\nname = \"tool\"\nsource_dir = \"src\"\n\
             [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n",
        );
        put(
            "src/main.c",
            "#include <stdio.h>\nint unit(int);\nint main(void) { printf(\"%d\\n\", unit(41)); return 0; }\n",
        );
        put("src/unit.c", "int unit(int x) { return x + 1; }\n");
        let target = harness_core::TargetContext::load(&root).expect("target");
        let base = Base::resolve(&target, "perf", &["cc"]).expect("base");
        let scratch = perf_scratch(&root).expect("scratch");
        let obj = sub_folder(&scratch, "obj").expect("obj");
        let c_files = crate::program_c_files_in(&base, "perf").expect("c files");
        let objects = compile_objects(&base, bench.runner(), &c_files, &obj)
            .expect("runs")
            .expect("compiles");
        assert_eq!(objects.len(), 2);
        let slot =
            sub_folder(&scratch, &format!("bin/{}", Slot::C.name().expect("slot"))).expect("slot");
        let paths: Vec<PathBuf> = objects.iter().map(|h| h.path.clone()).collect();
        let c = link_side(
            &base,
            &[],
            bench.runner(),
            &slot.join("tool"),
            &paths,
            &[],
            false,
        )
        .expect("runs")
        .expect("links");
        c.check().expect("unchanged");
        let whole = scratch.join("whole");
        whole_cc_into(&base, &[], bench.runner(), &whole, &c_files).expect("verify's build");
        let run = |p: &Path| std::process::Command::new(p).output().expect("runs").stdout;
        assert_eq!(run(&c.path), b"42\n");
        assert_eq!(run(&c.path), run(&whole));
        // A unit's program: unit.c out, a staticlib in.
        let crate_dir = crate::testutil::fixture_crate(
            &root,
            "unit_rs",
            true,
            "#[no_mangle] pub extern \"C\" fn unit(x: i32) -> i32 { x + 2 }\n",
        );
        let lib = bench.build(&crate_dir);
        let unit_c = root.join("src/unit.c").canonicalize().expect("unit.c");
        let kept = kept_c_files(&c_files, std::slice::from_ref(&unit_c)).expect("kept");
        let kept_objects: Vec<PathBuf> = objects
            .iter()
            .filter(|h| kept.iter().any(|k| k.file_stem() == h.path.file_stem()))
            .map(|h| h.path.clone())
            .collect();
        let slot = sub_folder(
            &scratch,
            &format!("bin/{}", Slot::Unit(1).name().expect("slot")),
        )
        .expect("slot");
        let unit = link_side(
            &base,
            &[],
            bench.runner(),
            &slot.join("tool"),
            &kept_objects,
            &[lib],
            false,
        )
        .expect("runs")
        .expect("links");
        assert_eq!(run(&unit.path), b"43\n");
        // A link that fails is evidence, with the linker's words.
        let broken = link_side(
            &base,
            &[],
            bench.runner(),
            &slot.join("broken"),
            &kept_objects,
            &[],
            false,
        )
        .expect("runs");
        assert!(broken.is_err(), "unit() is missing");
    }

    #[test]
    fn slots_are_four_characters_and_bounded() {
        assert_eq!(Slot::C.name().expect("slot"), "p000");
        assert_eq!(Slot::Unit(1).name().expect("slot"), "p001");
        assert_eq!(Slot::Unit(12).name().expect("slot"), "p012");
        assert_eq!(Slot::Unit(999).name().expect("slot"), "p999");
        assert_eq!(Slot::AsItStands.name().expect("slot"), "pall");
        assert!(Slot::Unit(1000).name().is_err());
        assert!(Slot::Unit(0).name().is_err());
    }

    #[test]
    fn kept_files_and_a_mismatched_entry() {
        let c = |n: &str| PathBuf::from(format!("/t/src/{n}"));
        let files = vec![c("a.c"), c("b.c"), c("main.c")];
        assert_eq!(
            kept_c_files(&files, &[c("b.c")]),
            Ok(vec![c("a.c"), c("main.c")])
        );
        assert_eq!(kept_c_files(&files, &[c("b.c"), c("gone.c")]), Err(1));
        assert_eq!(kept_c_files(&files, &[]), Ok(files.clone()));
    }

    #[test]
    fn a_changed_build_is_refused() {
        let dir =
            std::env::temp_dir().join(format!("perf-h-{}", harness_core::hash::random_hex(6)));
        std::fs::create_dir_all(&dir).expect("dir");
        let f = dir.join("bin");
        std::fs::write(&f, b"one").expect("write");
        let h = Hashed::new(&f).expect("hash");
        h.check().expect("unchanged");
        std::fs::write(&f, b"two").expect("write");
        assert!(h.check().is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_folders_refuse_links_and_keep_twenty_logs() {
        let base =
            std::env::temp_dir().join(format!("perf-f-{}", harness_core::hash::random_hex(6)));
        let root = base.join("t");
        std::fs::create_dir_all(root.join("migration/build")).expect("dir");
        std::fs::create_dir_all(base.join("elsewhere")).expect("dir");
        let root = root.canonicalize().expect("canonical");
        // A link in .perf's place is removed, never followed.
        std::os::unix::fs::symlink(base.join("elsewhere"), root.join("migration/build/.perf"))
            .expect("link");
        let scratch = perf_scratch(&root).expect("scratch");
        assert!(scratch.starts_with(&root));
        assert!(!std::fs::symlink_metadata(&scratch)
            .expect("meta")
            .file_type()
            .is_symlink());
        std::fs::write(scratch.join("old"), b"x").expect("write");
        let again = perf_scratch(&root).expect("scratch");
        assert!(!again.join("old").exists(), "made fresh");
        // .perf-out is kept.
        let out = perf_out(&root).expect("out");
        std::fs::write(out.join("keep"), b"x").expect("write");
        assert!(perf_out(&root).expect("out").join("keep").exists());
        // Logs: the oldest past 20 go.
        let logs = perf_logs(&root).expect("logs");
        for i in 0..25 {
            std::fs::write(logs.join(format!("{i:02}.log")), b"x").expect("write");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let logs = perf_logs(&root).expect("logs");
        let left = std::fs::read_dir(&logs).expect("read").count();
        assert_eq!(left, KEPT_LOGS - 1, "room for this run's log");
        assert!(!logs.join("00.log").exists() && logs.join("24.log").exists());
        std::fs::remove_dir_all(&base).ok();
    }
}
