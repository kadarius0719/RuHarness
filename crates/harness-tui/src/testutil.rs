//! Fixture targets for the front end's tests: committed ledgers copied to a
//! scratch dir (build products and the lock file left out), so a test can
//! write to its ledger and nothing it reads races another command.

use std::path::{Path, PathBuf};

/// The workspace root.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The tractor case with a verified unit, a superseded attempt and a
/// promoted one (`u-lib`, provenance `a-13c941dfff95`).
pub const READ_SCALEFACTORS: &str =
    "targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib";

fn copy(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == "build" || name == ".lock" {
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

/// A scratch copy of the committed target at `rel`, unique per `tag`.
pub fn scratch_target(rel: &str, tag: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("harness-tui-{tag}-{:010}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy(&repo().join(rel), &dst);
    // Adopted for this test process (docs/PROJECT-MAP-DESIGN.md §3.7).
    harness_core::adopt::testing::adopt(&dst);
    dst.canonicalize().unwrap()
}

/// Like [`scratch_target`], without the person's features
/// (`migration/features`): the target before `features init`.
pub fn scratch_target_without_features(rel: &str, tag: &str) -> PathBuf {
    let dst = scratch_target(rel, tag);
    match std::fs::remove_dir_all(dst.join("migration/features")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("{e}"),
        _ => dst,
    }
}

/// The mapped tool of [`file_list_tool`].
pub const LZG_TOOL: &str = "t-lzg";

/// A liblzg-shaped project (docs/PROJECT-MAP-DESIGN.md §4) with the
/// hand-written file-list tool [`LZG_TOOL`]: `src/lib/encode.c` (through
/// `internal.h`, which includes `"../include/lzg.h"`) and `src/tools/lzg.c`
/// (`<lzg.h>` through `src/include`) listed; `src/include/unused.h` and
/// `src/other/decode.c` not part of the tool; a C file in the tool's ledger.
/// The facts are written as the scanner records them, the plan by the
/// planner, `u-encode` given its `replaces`; adopted.
pub fn file_list_tool(tag: &str) -> TmpDir {
    use harness_core::facts::{FileRecord, SymbolRecord};
    let dir = TmpDir::new(tag);
    let root = &dir.0;
    let put = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put(
        "src/lib/encode.c",
        "#include \"internal.h\"\nunsigned lzg_encode(unsigned n) { return lzg_min(n, 9); }\n",
    );
    put(
        "src/lib/internal.h",
        "#include \"../include/lzg.h\"\n\
         static inline unsigned lzg_min(unsigned a, unsigned b) { return a < b ? a : b; }\n",
    );
    put("src/include/lzg.h", "unsigned lzg_encode(unsigned n);\n");
    put(
        "src/tools/lzg.c",
        "#include <lzg.h>\nint main(void) { return (int)lzg_encode(3); }\n",
    );
    put("src/include/unused.h", "#define NEVER_READ 1\n");
    put("src/other/decode.c", "int lzg_decode(void) { return 1; }\n");
    let ledger = format!("migration/tools/{LZG_TOOL}");
    put(
        &format!("{ledger}/harness.toml"),
        "schema_version = 2\n[target]\nname = \"lzg\"\nfiles = [\n\
         { path = \"src/lib/encode.c\", include_dirs = [\"src/include\"] },\n\
         { path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] },\n]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [] }\n",
    );
    put(
        &format!("{ledger}/units/u-encode/driver.c"),
        "int model_written(void) { return 1; }\n",
    );
    let file = |path: &str, includes: &[&str]| FileRecord {
        path: path.into(),
        hash: harness_core::hash::file_hash(&root.join(path)).unwrap(),
        includes: includes.iter().map(|s| (*s).to_string()).collect(),
    };
    let sym = |file: &str, name: &str| SymbolRecord {
        name: name.into(),
        kind: "function".into(),
        file: file.into(),
        visibility: "public".into(),
        signature: format!("int {name}(void)"),
        span: (1, 1),
    };
    let facts = harness_core::Facts {
        frontend: "test-inline".into(),
        files: vec![
            file("src/include/lzg.h", &[]),
            file("src/lib/encode.c", &["src/lib/internal.h"]),
            file("src/lib/internal.h", &["src/include/lzg.h"]),
            file("src/tools/lzg.c", &["src/include/lzg.h"]),
        ],
        symbols: vec![
            sym("src/lib/encode.c", "lzg_encode"),
            sym("src/tools/lzg.c", "main"),
        ],
        ..harness_core::Facts::default()
    };
    facts
        .store(&root.join(&ledger).join("facts.jsonl"))
        .unwrap();
    let computed = harness_core::planner::compute_units(&facts).unwrap();
    let plan_path = root.join(&ledger).join("plan.toml");
    let (text, _) =
        harness_core::plan::reconcile_to_string(&plan_path, None, "lzg", &computed).unwrap();
    // The oracle table closes u-encode's block (keys after it would be its).
    let mut blocks: Vec<String> = text.split("[[unit]]").map(str::to_string).collect();
    let at = blocks
        .iter()
        .position(|b| b.contains("id = \"u-encode\""))
        .unwrap();
    blocks[at] = format!(
        "{}\n\n[unit.oracle]\nkind = \"c-abi-differential\"\n\
         replaces = [\"src/lib/encode.c\"]\n\n",
        blocks[at].trim_end()
    );
    std::fs::write(&plan_path, blocks.join("[[unit]]")).unwrap();
    harness_core::adopt::testing::adopt(root);
    dir
}

/// A scratch directory removed on drop — also when a test fails.
pub struct TmpDir(pub PathBuf);

impl TmpDir {
    /// A fresh one, unique per `tag`.
    pub fn new(tag: &str) -> TmpDir {
        let dir = std::env::temp_dir().join(format!("harness-tui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir.canonicalize().unwrap())
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A test's child that does not end by itself while the test runs (a loop,
/// a stopped shell): SIGKILLed when the guard drops — when the test ends,
/// passing or failing — unless it is gone, or its pid now names another
/// process (its command name and its parent, this test process, are
/// checked first, so a reused pid is never killed).
pub struct KillOnDrop {
    pid: u32,
    comm: String,
}

impl KillOnDrop {
    /// Guard `pid`, a child of this process, read now as the program it is.
    pub fn new(pid: u32) -> KillOnDrop {
        KillOnDrop {
            pid,
            comm: comm_of(pid),
        }
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.comm.is_empty()
            || comm_of(self.pid) != self.comm
            || parent_of(self.pid) != Some(std::process::id())
        {
            return;
        }
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", &self.pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// `pid`'s parent (`ps -o ppid=`), `None` when it is gone.
fn parent_of(pid: u32) -> Option<u32> {
    std::process::Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
}

/// `pid`'s command name (`ps -o comm=`), empty when it is gone.
fn comm_of(pid: u32) -> String {
    std::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::process::ExitStatusExt;

    /// The guard kills its own child, and never a process this test did
    /// not start — here a grandchild, whose parent is another process.
    #[test]
    fn the_guard_kills_only_a_child_of_this_process() {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30 & echo $!; wait"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        let stdout = child.stdout.take().unwrap();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .unwrap();
        let grandchild: u32 = line.trim().parse().unwrap();
        drop(KillOnDrop::new(grandchild));
        // A killed one would be reaped by its shell's `wait` by now.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let spared = !comm_of(grandchild).is_empty();
        drop(KillOnDrop::new(child.id()));
        let status = child.wait().unwrap();
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", &grandchild.to_string()])
            .stderr(std::process::Stdio::null())
            .status();
        assert!(spared, "a grandchild was killed");
        assert_eq!(status.signal(), Some(9), "its own child is killed");
    }
}
