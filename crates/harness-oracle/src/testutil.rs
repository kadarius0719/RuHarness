//! Shared helpers for this crate's unit tests: self-cleaning temp dirs and a
//! "tool bench" — a throwaway target root with a real [`Runner`] (sandboxed
//! exactly like production when the sandbox is available) for tests that
//! drive the real `cargo`/`nm`.

use crate::exec::{Runner, DEFAULT_MAX_OUTPUT};
use crate::sandbox::{self, HostDirs, ProfileSpec};
use crate::symbols::SymbolCtx;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A unique directory under the system temp dir, removed on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "ruharness-oracle-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir.canonicalize().expect("canonical temp dir"))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A throwaway target root plus a production-shaped runner.
pub(crate) struct ToolBench {
    tmp: TempDir,
    build_root: PathBuf,
    host: Option<HostDirs>,
    runner: Runner,
    rustc_version: String,
}

impl ToolBench {
    pub(crate) fn new(tag: &str) -> ToolBench {
        let tmp = TempDir::new(tag);
        let build_root = tmp.path().join("migration/build");
        std::fs::create_dir_all(&build_root).expect("build root");
        let host = match sandbox::sandbox_mode() {
            "sandbox-exec" => Some(HostDirs::from_env().expect("HOME is set")),
            _ => None,
        };
        // The bench root lives under the temp dir, which every profile may
        // write, so fixture crates need no extra write rules.
        let tool_profile = host.as_ref().map(|host| {
            sandbox::render_profile(&ProfileSpec {
                host,
                target_root: tmp.path(),
                toolchain: true,
                write_dirs: &[],
                write_files: &[],
            })
            .expect("profile renders")
        });
        let runner = Runner {
            cwd: tmp.path().to_path_buf(),
            allowlist: ["cc", "cargo", "rustc", "nm"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            timeout: Duration::from_secs(300),
            max_output: DEFAULT_MAX_OUTPUT,
            tool_profile,
        };
        let version = runner
            .tool(&["rustc".to_string(), "-V".to_string()])
            .expect("rustc -V");
        let rustc_version = String::from_utf8_lossy(&version).trim().to_string();
        ToolBench {
            tmp,
            build_root,
            host,
            runner,
            rustc_version,
        }
    }

    pub(crate) fn root(&self) -> &Path {
        self.tmp.path()
    }

    pub(crate) fn build_root(&self) -> &Path {
        &self.build_root
    }

    pub(crate) fn runner(&self) -> &Runner {
        &self.runner
    }

    pub(crate) fn symbol_ctx(&self) -> SymbolCtx<'_> {
        SymbolCtx {
            runner: &self.runner,
            host: self.host.as_ref(),
            root: self.tmp.path(),
            build_root: &self.build_root,
            rustc_version: &self.rustc_version,
        }
    }

    /// Build a fixture crate exactly the way `verify` builds a unit crate.
    pub(crate) fn build(&self, crate_dir: &Path) -> PathBuf {
        let target_dir = crate::prepare_target_dir(crate_dir).expect("target dir");
        crate::build_staticlib(
            &self.runner,
            self.runner.tool_profile.as_deref(),
            crate_dir,
            &target_dir,
        )
        .expect("fixture crate builds")
    }
}

/// Write a dependency-free staticlib crate `<root>/fixtures/<name>` whose
/// `src/lib.rs` is `lib_rs`; returns the (canonical) crate dir.
pub(crate) fn fixture_crate(root: &Path, name: &str, panic_abort: bool, lib_rs: &str) -> PathBuf {
    let dir = root.join("fixtures").join(name);
    std::fs::create_dir_all(dir.join("src")).expect("fixture dirs");
    let mut manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n"
    );
    if panic_abort {
        manifest.push_str("\n[profile.release]\npanic = \"abort\"\n");
    }
    std::fs::write(dir.join("Cargo.toml"), manifest).expect("fixture manifest");
    std::fs::write(dir.join("src/lib.rs"), lib_rs).expect("fixture lib.rs");
    dir.canonicalize().expect("canonical fixture dir")
}
