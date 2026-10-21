//! The two-piece fixture of docs/BEYOND-LEAVES-DESIGN.md §7: a callee
//! (`u-callee`, `callee_mix`) and a caller (`u-caller`, `caller_sum`) whose
//! C calls it, a whole program that calls both, a test program (driver)
//! for each, a plan that knows the caller depends on the callee, and the
//! facts a scan would record.

use super::write;
use harness_core::facts::{FileRecord, RefRecord, SymbolRecord};
use harness_core::{Facts, Plan, TargetContext, Unit};
use std::path::{Path, PathBuf};

const CALLEE_H: &str = "int callee_mix(int a);\n";
const CALLEE_C: &str = "#include \"callee.h\"\n\
int callee_mix(int a) { return (int)((unsigned)a * 31u + 7u); }\n";
const CALLER_H: &str = "int caller_sum(int n);\n";
const CALLER_C: &str = "#include \"caller.h\"\n#include \"callee.h\"\n\
int caller_sum(int n) {\n\
  unsigned s = 0;\n\
  for (int i = 0; i < n; i++) s += (unsigned)callee_mix(i) ^ (unsigned)i;\n\
  return (int)(s % 100000u);\n\
}\n";
const MAIN_C: &str = "#include <stdio.h>\n#include \"caller.h\"\n#include \"callee.h\"\n\
int main(int argc, char** argv) {\n\
  int count = 0, c;\n\
  FILE* f = argc > 2 ? fopen(argv[2], \"rb\") : NULL;\n\
  if (!f) return 2;\n\
  while ((c = fgetc(f)) != EOF) count++;\n\
  fclose(f);\n\
  printf(\"%d %d\\n\", caller_sum(count % 50), callee_mix(count));\n\
  return 0;\n\
}\n";
const CALLEE_DRIVER: &str = "#include <stdio.h>\n#include \"callee.h\"\n\
int main(void) {\n\
  for (int a = -300; a <= 300; a++) printf(\"%d %d\\n\", a, callee_mix(a));\n\
  printf(\"%d\\n\", callee_mix(2147483647));\n\
  return 0;\n\
}\n";
const CALLER_DRIVER: &str = "#include <stdio.h>\n#include \"caller.h\"\n\
int main(void) {\n\
  for (int n = -3; n <= 80; n++) printf(\"%d %d\\n\", n, caller_sum(n));\n\
  return 0;\n\
}\n";

/// The callee's faithful Rust.
pub const CALLEE_RS: &str = "#[no_mangle]\n\
pub extern \"C\" fn callee_mix(a: i32) -> i32 {\n\
    (a as u32).wrapping_mul(31).wrapping_add(7) as i32\n}\n";

/// The caller's faithful Rust: it calls the callee through an `extern`
/// block, as the program stands.
pub const CALLER_RS: &str = "extern \"C\" {\n    fn callee_mix(a: i32) -> i32;\n}\n\
#[no_mangle]\n\
pub extern \"C\" fn caller_sum(n: i32) -> i32 {\n\
    let mut s: u32 = 0;\n\
    let mut i = 0;\n\
    while i < n {\n\
        s = s.wrapping_add((unsafe { callee_mix(i) } as u32) ^ (i as u32));\n\
        i += 1;\n\
    }\n\
    (s % 100000) as i32\n}\n";

/// A caller Rust that names the callee wrongly: nothing defines it.
pub const CALLER_RS_FORGETS: &str = "extern \"C\" {\n    fn callee_mixx(a: i32) -> i32;\n}\n\
#[no_mangle]\n\
pub extern \"C\" fn caller_sum(n: i32) -> i32 {\n\
    let mut s: u32 = 0;\n\
    let mut i = 0;\n\
    while i < n {\n\
        s = s.wrapping_add((unsafe { callee_mixx(i) } as u32) ^ (i as u32));\n\
        i += 1;\n\
    }\n\
    (s % 100000) as i32\n}\n";

/// The plan; `caller_status` and `callee_status` are plan status words,
/// `caller_deps` the caller's `depends_on` line.
pub fn plan_toml(callee_status: &str, caller_status: &str, caller_deps: &str) -> String {
    format!(
        "schema_version = 1\ntarget = \"two\"\n\n\
         [[unit]]\nid = \"u-caller\"\nstatus = \"{caller_status}\"\nfiles = [\"src/two/caller.c\"]\n\
         symbols = [\"caller_sum\"]\ninterface = [\"int caller_sum(int n)\"]\n\
         depends_on = [{caller_deps}]\n\n\
         [unit.oracle]\nkind = \"c-abi-differential\"\n\
         driver = \"migration/units/u-caller/driver.c\"\nrust_crate = \"caller_rs\"\n\
         replaces = [\"src/two/caller.c\"]\n\n\
         [[unit]]\nid = \"u-callee\"\nstatus = \"{callee_status}\"\nfiles = [\"src/two/callee.c\"]\n\
         symbols = [\"callee_mix\"]\ninterface = [\"int callee_mix(int a)\"]\ndepends_on = []\n\n\
         [unit.oracle]\nkind = \"c-abi-differential\"\n\
         driver = \"migration/units/u-callee/driver.c\"\nrust_crate = \"callee_rs\"\n\
         replaces = [\"src/two/callee.c\"]\n"
    )
}

/// Write (or rewrite) the plan.
pub fn set_plan(root: &Path, callee_status: &str, caller_status: &str, caller_deps: &str) {
    write(
        &root.join("migration/plan.toml"),
        &plan_toml(callee_status, caller_status, caller_deps),
    );
}

/// A unit crate `migration/units/<unit>/<name>` whose `src/lib.rs` is
/// `lib_rs`; returns its folder.
pub fn write_crate(root: &Path, unit: &str, name: &str, lib_rs: &str) -> PathBuf {
    let dir = root.join("migration/units").join(unit).join(name);
    write(
        &dir.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [lib]\ncrate-type = [\"staticlib\"]\n\n[workspace]\n\n\
             [profile.release]\npanic = \"abort\"\n"
        ),
    );
    write(&dir.join("src/lib.rs"), lib_rs);
    dir
}

fn file(path: &str, includes: &[&str]) -> FileRecord {
    FileRecord {
        path: path.into(),
        hash: String::new(),
        includes: includes.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn symbol(name: &str, path: &str, signature: &str) -> SymbolRecord {
    SymbolRecord {
        name: name.into(),
        kind: "function".into(),
        file: path.into(),
        visibility: "public".into(),
        signature: signature.into(),
        span: (1, 3),
    }
}

fn call(from: &str, path: &str, to: &str, resolved: bool) -> RefRecord {
    RefRecord {
        from: from.into(),
        file: path.into(),
        to: to.into(),
        refkind: "call".into(),
        resolved,
    }
}

/// Lay out the whole fixture under `root` (the whole-program check on,
/// `-c <sample>`), the callee and the caller both pending, the caller's Rust
/// [`CALLER_RS`]; returns the target.
pub fn lay_out(root: &Path) -> TargetContext {
    write(
        &root.join("harness.toml"),
        "schema_version = 1\n\n[target]\nname = \"two\"\nsource_dir = \"src/two\"\n\n\
         [oracle]\nallowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]\n\n\
         [oracle.whole_program]\nargs = [\"-c\"]\n",
    );
    write(&root.join("src/two/callee.h"), CALLEE_H);
    write(&root.join("src/two/callee.c"), CALLEE_C);
    write(&root.join("src/two/caller.h"), CALLER_H);
    write(&root.join("src/two/caller.c"), CALLER_C);
    write(&root.join("src/two/main.c"), MAIN_C);
    write(
        &root.join("migration/units/u-callee/driver.c"),
        CALLEE_DRIVER,
    );
    write(
        &root.join("migration/units/u-caller/driver.c"),
        CALLER_DRIVER,
    );
    set_plan(root, "pending", "pending", "\"u-callee\"");
    write_crate(root, "u-caller", "caller_rs", CALLER_RS);
    let facts = Facts {
        frontend: "test-inline".to_string(),
        files: vec![
            file("src/two/callee.c", &["src/two/callee.h"]),
            file("src/two/callee.h", &[]),
            file(
                "src/two/caller.c",
                &["src/two/callee.h", "src/two/caller.h"],
            ),
            file("src/two/caller.h", &[]),
            file("src/two/main.c", &["src/two/callee.h", "src/two/caller.h"]),
        ],
        symbols: vec![
            symbol("callee_mix", "src/two/callee.c", "int callee_mix(int a)"),
            symbol("caller_sum", "src/two/caller.c", "int caller_sum(int n)"),
            symbol("main", "src/two/main.c", "int main(int argc, char** argv)"),
        ],
        refs: vec![
            call("caller_sum", "src/two/caller.c", "callee_mix", true),
            call("main", "src/two/main.c", "caller_sum", true),
            call("main", "src/two/main.c", "callee_mix", true),
            call("main", "src/two/main.c", "fopen", false),
            call("main", "src/two/main.c", "printf", false),
        ],
    };
    facts
        .store(&root.join("migration/facts.jsonl"))
        .expect("facts stored");
    harness_core::adopt::testing::adopt(root);
    TargetContext::load(root).expect("target loads")
}

/// The unit `id` as the plan on disk holds it.
pub fn unit(root: &Path, id: &str) -> Unit {
    let plan = Plan::load(&root.join("migration/plan.toml")).expect("plan loads");
    plan.unit(id).expect("unit").clone()
}
