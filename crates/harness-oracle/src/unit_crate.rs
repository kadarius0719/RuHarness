//! The unit-crate check (docs/PROJECT-MAP-DESIGN.md §3.7): before any cargo
//! run on a unit crate — verify, the boundary check, perf, the benchmark
//! build — the crate folder may hold only what cargo needs to build the
//! unit's own Rust, and its manifest nothing that makes cargo run or fetch
//! code. A ledger can come from a download, so a `build.rs`, a `.cargo/`
//! folder or a dependency beside an exact manifest is refused by name
//! before cargo ever sees it.

use harness_core::error::Error;
use std::path::Path;

/// Finder's folder notes: ignored wherever they appear.
const DS_STORE: &str = ".DS_Store";

/// Top-level manifest keys a unit crate may not hold: each makes cargo
/// fetch, build or run code beyond the unit's own sources (cargo reads the
/// underscore spellings as the dashed ones), or, for `cargo-features`,
/// switches on unstable manifest behaviour on a nightly toolchain.
const REFUSED_TABLES: [&str; 8] = [
    "dependencies",
    "dev-dependencies",
    "dev_dependencies",
    "build-dependencies",
    "build_dependencies",
    "patch",
    "target",
    "cargo-features",
];

/// The cache file the symbol-set baseline crate keeps beside its manifest
/// ([`crate::symbols`]).
pub(crate) const BASELINE_CACHE: &str = "symbols.txt";

/// Refuse the unit crate at `crate_dir` (canonical) unless the folder holds
/// only `Cargo.toml`, an optional `Cargo.lock`, `src/*.rs` and the
/// harness-made `target/` (a real folder; `.DS_Store` ignored), and its
/// manifest passes [`manifest_problem`]. `unit` names the unit in the
/// refusal. Reads the folder; never runs anything.
pub(crate) fn check_unit_crate(unit: &str, crate_dir: &Path) -> Result<(), Error> {
    check_crate(unit, crate_dir, &[])
}

/// [`check_unit_crate`] for the harness's own symbol-set baseline crate
/// (`<build>/symbol-baseline/<strategy>/`), which also keeps its cache file
/// ([`BASELINE_CACHE`]): it lies in the target's ledger, so a download can
/// plant a `build.rs` or a manifest there as in any unit crate.
pub(crate) fn check_baseline_crate(crate_dir: &Path) -> Result<(), Error> {
    check_crate(crate::symbols::BASELINE_DIR, crate_dir, &[BASELINE_CACHE])
}

fn check_crate(unit: &str, crate_dir: &Path, also: &[&str]) -> Result<(), Error> {
    let refuse = |what: String| {
        Error::InvalidPlan(format!(
            "unit `{unit}`: its crate folder {} {what}; a unit crate holds only Cargo.toml, \
             Cargo.lock, src/*.rs and the harness's target/, so cargo is not run on it",
            crate_dir.display()
        ))
    };
    for (name, kind) in entries(crate_dir)? {
        match (name.as_str(), kind) {
            (DS_STORE, _) => {}
            ("Cargo.toml" | "Cargo.lock", Kind::File) => {}
            ("target" | "src", Kind::Dir) => {}
            (other, Kind::File) if also.contains(&other) => {}
            (_, Kind::Link) => return Err(refuse(format!("holds `{name}`, a link"))),
            _ => return Err(refuse(format!("holds `{name}`"))),
        }
    }
    let src = crate_dir.join("src");
    if src.is_dir() {
        for (name, kind) in entries(&src)? {
            let rust = name.ends_with(".rs") && name.len() > ".rs".len();
            match kind {
                _ if name == DS_STORE => {}
                Kind::File if rust => {}
                Kind::Link => return Err(refuse(format!("holds `src/{name}`, a link"))),
                _ => return Err(refuse(format!("holds `src/{name}`"))),
            }
        }
    }
    let manifest = crate_dir.join("Cargo.toml");
    if manifest.is_file() {
        let text = std::fs::read_to_string(&manifest).map_err(|e| Error::io(&manifest, e))?;
        if let Some(why) = manifest_problem(&text) {
            return Err(Error::InvalidPlan(format!(
                "unit `{unit}`: its crate's Cargo.toml {why}, which could make cargo fetch, \
                 build or run code beyond the unit's own sources, so cargo is not run on it"
            )));
        }
    }
    Ok(())
}

/// What kind of entry a folder holds, never following a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    File,
    Dir,
    Link,
    Other,
}

/// The entries of `dir`, by name (lossy: a non-UTF-8 name is shown with
/// replacement characters and matches no allowed name), sorted.
fn entries(dir: &Path) -> Result<Vec<(String, Kind)>, Error> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let file_type = entry.file_type().map_err(|e| Error::io(entry.path(), e))?;
        let kind = if file_type.is_symlink() {
            Kind::Link
        } else if file_type.is_dir() {
            Kind::Dir
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        let name: String = entry
            .file_name()
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .collect();
        out.push((name, kind));
    }
    out.sort();
    Ok(out)
}

/// Why a unit crate's manifest `text` is refused, or `None`: it must parse,
/// and hold no `build` key but `build = false` (the harness's own manifest
/// says so), no `links` or `workspace` key in `[package]`, no
/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` (either
/// spelling), `[patch]` or `[target.*]` table, no `cargo-features`, no
/// `path` in `[lib]` or any `[[bin]]` (a source outside the crate's
/// digest), and an empty `[workspace]` — without one cargo searches the
/// folders above for a workspace root, whose profile would then apply.
/// zopfli's hand-written crate and the benchmark's crates (an earlier
/// harness manifest) pass.
pub(crate) fn manifest_problem(text: &str) -> Option<String> {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => return Some("does not parse".into()),
    };
    if let Some(package) = table.get("package").and_then(toml::Value::as_table) {
        match package.get("build") {
            None | Some(toml::Value::Boolean(false)) => {}
            Some(_) => return Some("names a build script (`build`)".into()),
        }
        for key in ["links", "workspace"] {
            if package.contains_key(key) {
                return Some(format!("holds `package.{key}`"));
            }
        }
    }
    for key in REFUSED_TABLES {
        if table.contains_key(key) {
            return Some(if key == "cargo-features" {
                "holds `cargo-features`".into()
            } else {
                format!("holds `[{key}]`")
            });
        }
    }
    if table
        .get("lib")
        .and_then(toml::Value::as_table)
        .is_some_and(|lib| lib.contains_key("path"))
    {
        return Some("holds `lib.path`".into());
    }
    if let Some(bins) = table.get("bin") {
        let has_path = match bins.as_array() {
            Some(bins) => bins
                .iter()
                .any(|b| b.as_table().is_none_or(|t| t.contains_key("path"))),
            None => true,
        };
        if has_path {
            return Some("holds a `[[bin]]` with a `path`".into());
        }
    }
    match table.get("workspace") {
        None => Some("has no `[workspace]` (an empty one keeps cargo inside the crate)".into()),
        Some(toml::Value::Table(t)) if t.is_empty() => None,
        Some(_) => Some("holds a `[workspace]` that is not empty".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// The harness's own manifest as `candidate_manifest` writes it (the
    /// benchmark's 30 crates carry this one).
    const HARNESS_MANIFEST: &str =
        "# Generated by RuHarness: harness-owned, never model-written.\n\
         [package]\nname = \"u_rs\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\", \"rlib\"]\n\n\
         [profile.release]\npanic = \"abort\"\n\n[workspace]\n";

    fn put(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A crate with the harness's manifest, a lock, three sources.
    fn harness_crate(tmp: &TempDir) -> std::path::PathBuf {
        let dir = tmp.path().join("u_rs");
        put(&dir, "Cargo.toml", HARNESS_MANIFEST);
        put(&dir, "Cargo.lock", "version = 4\n");
        for f in ["lib", "logic", "ffi"] {
            put(&dir, &format!("src/{f}.rs"), "\n");
        }
        dir.canonicalize().unwrap()
    }

    #[test]
    fn the_harness_crate_with_its_target_and_finder_notes_is_accepted() {
        let tmp = TempDir::new("unit-crate-ok");
        let dir = harness_crate(&tmp);
        crate::prepare_target_dir(&dir).unwrap();
        put(&dir, ".DS_Store", "x");
        put(&dir, "src/.DS_Store", "x");
        check_unit_crate("u-ok", &dir).expect("accepted");
        // `build = false`, as the harness may write it, is no build script.
        let with_false = HARNESS_MANIFEST.replace(
            "edition = \"2021\"\n",
            "edition = \"2021\"\nbuild = false\n",
        );
        put(&dir, "Cargo.toml", &with_false);
        check_unit_crate("u-ok", &dir).expect("build = false accepted");
    }

    /// zopfli's committed, hand-written crate passes as it stands.
    #[test]
    fn zopflis_committed_crate_is_accepted() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../targets/zopfli/migration/units/u001-katajainen/katajainen_rs")
            .canonicalize()
            .unwrap();
        check_unit_crate("u001-katajainen", &dir).expect("zopfli's crate");
    }

    #[test]
    fn a_build_script_a_cargo_folder_an_extra_file_or_a_link_is_refused_by_name() {
        for (case, make, named) in [
            ("build", "build.rs", "`build.rs`"),
            ("cargo", ".cargo/config.toml", "`.cargo`"),
            ("extra", "README.md", "`README.md`"),
            ("nested", "src/sub/mod.rs", "`src/sub`"),
            ("not-rust", "src/data.bin", "`src/data.bin`"),
        ] {
            let tmp = TempDir::new(&format!("unit-crate-{case}"));
            let dir = harness_crate(&tmp);
            put(&dir, make, "fn main() {}\n");
            let err = check_unit_crate("u-bad", &dir).expect_err(case).to_string();
            assert!(err.contains(named), "{case}: {err}");
            assert!(err.contains("unit `u-bad`"), "{case}: {err}");
            assert!(err.contains("cargo is not run on it"), "{case}: {err}");
        }
        // A target/ that is a link is not the harness's folder.
        let tmp = TempDir::new("unit-crate-target-link");
        let dir = harness_crate(&tmp);
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("target")).unwrap();
        let err = check_unit_crate("u-bad", &dir)
            .expect_err("link")
            .to_string();
        assert!(err.contains("`target`, a link"), "{err}");
    }

    #[test]
    fn a_manifest_that_could_run_or_fetch_code_is_refused_by_key() {
        for (extra, named) in [
            ("\n[dependencies]\nlibc = \"0.2\"\n", "`[dependencies]`"),
            ("\n[dev-dependencies]\nx = \"1\"\n", "`[dev-dependencies]`"),
            (
                "\n[build-dependencies]\nx = \"1\"\n",
                "`[build-dependencies]`",
            ),
            (
                "\n[patch.crates-io]\nx = { path = \"../x\" }\n",
                "`[patch]`",
            ),
            (
                "\n[target.'cfg(unix)'.dependencies]\nx = \"1\"\n",
                "`[target]`",
            ),
            ("\n[dev_dependencies]\nx = \"1\"\n", "`[dev_dependencies]`"),
            (
                "\n[build_dependencies]\nx = \"1\"\n",
                "`[build_dependencies]`",
            ),
            (
                "\n[[bin]]\nname = \"b\"\npath = \"../../x.rs\"\n",
                "`[[bin]]`",
            ),
        ] {
            let tmp = TempDir::new("unit-crate-manifest");
            let dir = harness_crate(&tmp);
            put(&dir, "Cargo.toml", &format!("{HARNESS_MANIFEST}{extra}"));
            let err = check_unit_crate("u-deps", &dir)
                .expect_err(named)
                .to_string();
            assert!(err.contains(named), "{err}");
            assert!(err.contains("unit `u-deps`"), "{err}");
        }
        for (package_key, named) in [
            ("build = \"gen.rs\"", "build script"),
            ("build = true", "build script"),
            ("links = \"z\"", "`package.links`"),
            ("workspace = \"..\"", "`package.workspace`"),
        ] {
            let text = HARNESS_MANIFEST.replace(
                "edition = \"2021\"\n",
                &format!("edition = \"2021\"\n{package_key}\n"),
            );
            assert!(
                manifest_problem(&text).is_some_and(|w| w.contains(named)),
                "{package_key}: {:?}",
                manifest_problem(&text)
            );
        }
        // A source outside the crate, unstable manifest features, and no
        // `[workspace]` at all (cargo would look above for one).
        let lib_path = HARNESS_MANIFEST.replace("[lib]\n", "[lib]\npath = \"../../x.rs\"\n");
        assert_eq!(
            manifest_problem(&lib_path).as_deref(),
            Some("holds `lib.path`")
        );
        let features = format!("cargo-features = [\"edition2024\"]\n{HARNESS_MANIFEST}");
        assert_eq!(
            manifest_problem(&features).as_deref(),
            Some("holds `cargo-features`")
        );
        let no_workspace = HARNESS_MANIFEST.replace("\n[workspace]\n", "\n");
        assert!(
            manifest_problem(&no_workspace).is_some_and(|w| w.contains("no `[workspace]`")),
            "{:?}",
            manifest_problem(&no_workspace)
        );
        // A `[[bin]]` without a path is not refused by this rule.
        let bin = format!("{HARNESS_MANIFEST}\n[[bin]]\nname = \"b\"\n");
        assert_eq!(manifest_problem(&bin), None);
        let members =
            HARNESS_MANIFEST.replace("[workspace]\n", "[workspace]\nmembers = [\"..\"]\n");
        assert_eq!(
            manifest_problem(&members).as_deref(),
            Some("holds a `[workspace]` that is not empty")
        );
        assert_eq!(
            manifest_problem("[package\n").as_deref(),
            Some("does not parse")
        );
        assert_eq!(manifest_problem(HARNESS_MANIFEST), None);
    }

    /// Every unit crate committed in this repository — zopfli's and the
    /// benchmark cases', promoted and attempted — passes the manifest rule.
    #[test]
    fn every_committed_unit_manifest_passes() {
        let targets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets");
        let mut seen = 0;
        let mut stack = vec![targets];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    if name != "target" && name != ".bench" && name != ".scorer-vendor" {
                        stack.push(path);
                    }
                } else if name == "Cargo.toml"
                    && path.components().any(|c| c.as_os_str() == "units")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert_eq!(manifest_problem(&text), None, "{}", path.display());
                    seen += 1;
                }
            }
        }
        assert!(seen >= 100, "found only {seen} unit manifests");
    }
}
