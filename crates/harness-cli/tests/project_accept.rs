//! `harness project accept` through the binary (docs/PROJECT-MAP-DESIGN.md
//! §3.6, §3.8, §4): its refusals, the person's picks with no reply, the
//! re-link before anything is written, the liblzg shape, a library, an id
//! accepted again, the accepted id kept across maps, the "what changed"
//! report of a later map, and zopfli accepted from its map with the same
//! facts, source hashes and a green verify. Every child runs with the test
//! process's own adoption file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, canonical temporary folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-accept-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir.canonicalize().unwrap())
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.0.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn tool(&self, id: &str) -> PathBuf {
        harness_core::config::tool_dir(&self.0, id).join("harness.toml")
    }

    /// Recorded as made on this computer, then the person's own
    /// configuration written (so it is stated, not proposed).
    fn stated(&self, flags: &str) {
        harness_core::adopt::testing::adopt(&self.0);
        self.write(
            "migration/map/config.toml",
            &format!("[[configuration]]\nname = \"plain\"\nfrom = \"stated\"\nflags = [{flags}]\n"),
        );
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn harness(args: &[&str]) -> Run {
    let file = harness_core::adopt::testing::adoption_file();
    let out = Command::new(env!("CARGO_BIN_EXE_harness"))
        .args(args)
        .env(harness_core::adopt::ADOPTED_ENV, file)
        .output()
        .expect("spawn harness");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn map(tmp: &Tmp) -> Run {
    let run = harness(&["project", "map", "--target", tmp.arg()]);
    assert_eq!(run.code, 0, "{}", run.all());
    run
}

fn accept(tmp: &Tmp, id: &str, extra: &[&str]) -> Run {
    let mut args = vec!["project", "accept", id, "--target", tmp.arg()];
    args.extend(extra);
    harness(&args)
}

fn refused(run: &Run, says: &str) {
    assert_eq!(run.code, 1, "{}", run.all());
    assert!(run.stderr.contains(says), "{says}\n{}", run.all());
    // One sentence: one line.
    assert_eq!(run.stderr.trim_end().lines().count(), 1, "{}", run.stderr);
}

fn map_json(root: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(root.join("migration/map/project-map.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// A tool's harness.toml as every command loads it (`path` is
/// `<root>/migration/tools/<id>/harness.toml`).
fn config(path: &Path) -> harness_core::config::TargetConfig {
    let root = path.ancestors().nth(4).unwrap();
    harness_core::config::TargetConfig::load_file(path, root)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn listed(path: &Path) -> Vec<String> {
    config(path)
        .target
        .files()
        .unwrap()
        .iter()
        .map(|f| f.path.clone())
        .collect()
}

/// liblzg's shape (§4): `<lzg.h>` from a sibling `include/`; `decode.c` and
/// `mini.c` both define the decoder, and `decode.c` needs `checksum.c` too;
/// two programs, both choices linking for each, so both are held on d1.
fn lzg(tmp: &Tmp) {
    tmp.write(
        "include/lzg.h",
        "int lzg_decode(int x);\nint lzg_size(int x);\nunsigned lzg_checksum(int x);\n",
    );
    tmp.write(
        "src/decode.c",
        "#include <lzg.h>\nint lzg_decode(int x) { return x + (int)lzg_checksum(x); }\n\
         int lzg_size(int x) { return x; }\n",
    );
    tmp.write(
        "src/checksum.c",
        "#include <lzg.h>\nunsigned lzg_checksum(int x) { return (unsigned)x * 3u; }\n",
    );
    tmp.write(
        "src/mini.c",
        "#include <lzg.h>\nint lzg_decode(int x) { return x; }\nint lzg_size(int x) { return x; }\n",
    );
    tmp.write(
        "tools/unlzg.c",
        "#include <lzg.h>\nint main(void) { return lzg_decode(1) + lzg_size(2); }\n",
    );
    tmp.write(
        "tools/lzgcheck.c",
        "#include <lzg.h>\nint main(void) { return lzg_decode(3) - lzg_size(3); }\n",
    );
}

/// The liblzg shape: both programs held, each settled by `--keep` with no
/// reply — by index and by path — each pick said in words before the file
/// is written, the closure recomputed (the third file only with the
/// definer that needs it), the unchosen file "alternative not kept".
#[test]
fn the_liblzg_shape_is_settled_by_the_persons_keeps() {
    let tmp = Tmp::new("lzg");
    lzg(&tmp);
    tmp.stated("");
    map(&tmp);
    let m = map_json(&tmp.0);
    for id in ["t-unlzg", "t-lzgcheck"] {
        let c = m["closures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["program"] == id)
            .unwrap();
        assert_eq!(c["questions"], serde_json::json!(["d1"]), "{c}");
    }
    // Held: refused, the definers named, none suggested.
    let run = accept(&tmp, "t-unlzg", &[]);
    refused(
        &run,
        "duplicate set d1 of t-unlzg (lzg_decode, lzg_size) is not settled: pick its definer \
         yourself with --keep d1=<index or path> (its definers: d1.1 src/decode.c, d1.2 \
         src/mini.c)",
    );
    assert!(!tmp.tool("t-unlzg").exists());

    // By index.
    let run = accept(&tmp, "t-unlzg", &["--keep", "d1=d1.1"]);
    assert_eq!(run.code, 0, "{}", run.all());
    let pick = "project accept t-unlzg: keeping `src/decode.c` over `src/mini.c` for \
                `lzg_decode, lzg_size`";
    let not_kept = "  src/mini.c: alternative not kept";
    let wrote = "project accept: wrote migration/tools/t-unlzg/harness.toml";
    for says in [pick, not_kept, wrote] {
        assert!(run.stdout.contains(says), "{says}\n{}", run.stdout);
    }
    // In words before it is written.
    assert!(run.stdout.find(pick) < run.stdout.find(wrote));
    assert_eq!(
        listed(&tmp.tool("t-unlzg")),
        ["src/checksum.c", "src/decode.c", "tools/unlzg.c"]
    );
    let c = config(&tmp.tool("t-unlzg"));
    let list = c.target.file_list().unwrap();
    assert_eq!(c.target.name, "unlzg");
    assert_eq!(list.files[0].include_dirs, ["include"]);
    assert_eq!(list.configuration.name, "plain");
    assert_eq!(list.picks.len(), 1);
    assert_eq!(list.picks[0].definers, ["src/decode.c", "src/mini.c"]);
    assert_eq!(list.picks[0].keep, "src/decode.c");
    assert_eq!(list.picks[0].by, "person");
    let stamp = list.map.as_ref().unwrap();
    assert_eq!(stamp.root_hash, m["root_hash"].as_str().unwrap());
    assert_eq!(stamp.inputs_hash, m["inputs_hash"].as_str().unwrap());

    // By path: the third file stays only with the definer that needs it.
    let run = accept(&tmp, "t-lzgcheck", &["--keep", "d1=src/mini.c"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout.contains(
            "project accept t-lzgcheck: keeping `src/mini.c` over `src/decode.c` for \
             `lzg_decode, lzg_size`"
        ),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("  src/decode.c: alternative not kept"));
    assert_eq!(
        listed(&tmp.tool("t-lzgcheck")),
        ["src/mini.c", "tools/lzgcheck.c"]
    );

    // A pick that names no set, or no definer of it: refused.
    refused(
        &accept(&tmp, "t-unlzg", &["--keep", "d7=d7.1"]),
        "--keep d7=d7.1 names no duplicate set of t-unlzg (t-unlzg's are d1)",
    );
    refused(
        &accept(&tmp, "t-unlzg", &["--keep", "d1=src/checksum.c"]),
        "--keep d1=src/checksum.c names no definer of d1: its definers are d1.1 src/decode.c, \
         d1.2 src/mini.c",
    );
    // Both tools work as targets.
    for id in ["t-unlzg", "t-lzgcheck"] {
        let run = harness(&["scan", "--target", tmp.arg(), "--tool", id]);
        assert_eq!(run.code, 0, "{id}: {}", run.all());
    }
}

/// Accepting an id again rewrites only its harness.toml: its ledger stays.
#[test]
fn accepting_an_id_again_keeps_its_ledger() {
    let tmp = Tmp::new("again");
    lzg(&tmp);
    tmp.stated("");
    map(&tmp);
    assert_eq!(accept(&tmp, "t-unlzg", &["--keep", "d1=d1.1"]).code, 0);
    let run = harness(&["scan", "--target", tmp.arg(), "--tool", "t-unlzg"]);
    assert_eq!(run.code, 0, "{}", run.all());
    let run = harness(&["plan", "--target", tmp.arg(), "--tool", "t-unlzg"]);
    assert_eq!(run.code, 0, "{}", run.all());
    let ledger = harness_core::config::tool_dir(&tmp.0, "t-unlzg");
    let plan = std::fs::read(ledger.join("plan.toml")).unwrap();
    let facts = std::fs::read(ledger.join("facts.jsonl")).unwrap();
    tmp.write(
        "migration/tools/t-unlzg/units/u-x/notes.md",
        "the person's notes\n",
    );
    let run = accept(
        &tmp,
        "t-unlzg",
        &["--keep", "d1=d1.2", "--run-name", "unlzg2"],
    );
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout
            .contains("its ledger (plan, units, verdicts) is kept"),
        "{}",
        run.stdout
    );
    assert_eq!(
        listed(&tmp.tool("t-unlzg")),
        ["src/mini.c", "tools/unlzg.c"]
    );
    assert_eq!(config(&tmp.tool("t-unlzg")).target.name, "unlzg2");
    assert_eq!(std::fs::read(ledger.join("plan.toml")).unwrap(), plan);
    assert_eq!(std::fs::read(ledger.join("facts.jsonl")).unwrap(), facts);
    assert_eq!(
        tmp.read("migration/tools/t-unlzg/units/u-x/notes.md"),
        "the person's notes\n"
    );
}

/// A folder holding one of the configuration's `system_headers` reaches
/// the tool as `-idirafter<dir>`, as the map compiled it, and leaves the
/// file's own include folders.
#[test]
fn a_system_header_folder_becomes_an_idirafter_flag() {
    let tmp = Tmp::new("idirafter");
    tmp.write("compat/rh_config.h", "#define RH_CFG 1\n");
    tmp.write(
        "src/main.c",
        "#include <rh_config.h>\nint main(void) { return RH_CFG - 1; }\n",
    );
    harness_core::adopt::testing::adopt(&tmp.0);
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"plain\"\nfrom = \"stated\"\nflags = []\n\
         system_headers = [\"rh_config.h\"]\n",
    );
    map(&tmp);
    let m = map_json(&tmp.0);
    let main = m["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/main.c")
        .unwrap();
    assert_eq!(
        main["include_dirs"],
        serde_json::json!(["compat"]),
        "{main}"
    );
    let run = accept(&tmp, "t-main", &[]);
    assert_eq!(run.code, 0, "{}", run.all());
    let c = config(&tmp.tool("t-main"));
    let list = c.target.file_list().unwrap();
    assert!(list.files[0].include_dirs.is_empty(), "{:?}", list.files);
    assert_eq!(list.configuration.flags, ["-idiraftercompat"]);
    let run = harness(&["scan", "--target", tmp.arg(), "--tool", "t-main"]);
    assert_eq!(run.code, 0, "{}", run.all());
}

/// A library: compiled, never linked; no program, its id as `name`.
#[test]
fn a_library_is_accepted_with_its_id_as_name() {
    let tmp = Tmp::new("library");
    tmp.write(
        "lib/crc.h",
        "unsigned crc(unsigned x);\nunsigned crc_step(unsigned x);\n",
    );
    tmp.write(
        "lib/crc.c",
        "#include \"crc.h\"\nunsigned crc(unsigned x) { return crc_step(x) ^ 1u; }\n",
    );
    tmp.write(
        "lib/step.c",
        "#include \"crc.h\"\nunsigned crc_step(unsigned x) { return x * 31u; }\n",
    );
    tmp.stated("");
    map(&tmp);
    let m = map_json(&tmp.0);
    assert_eq!(m["libraries"][0]["id"], "l-crc", "{m}");
    let run = accept(&tmp, "l-crc", &[]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout
            .contains("(library, 2 file(s), compiled, not linked"),
        "{}",
        run.stdout
    );
    let c = config(&tmp.tool("l-crc"));
    assert_eq!(c.target.name, "l-crc");
    assert_eq!(listed(&tmp.tool("l-crc")), ["lib/crc.c", "lib/step.c"]);
    assert!(!tmp
        .read("migration/tools/l-crc/harness.toml")
        .contains("extra_link_args"));
    // A pick has no place on a library.
    refused(
        &accept(&tmp, "l-crc", &["--keep", "d1=d1.1"]),
        "--keep d1=d1.1 names no duplicate set of l-crc (l-crc has none)",
    );
    let run = harness(&["scan", "--target", tmp.arg(), "--tool", "l-crc"]);
    assert_eq!(run.code, 0, "{}", run.all());
}

/// The refusals, one sentence each, nothing written: a guessed and a
/// proposed configuration, an unknown id, the map changed, a root
/// harness.toml, an incomplete closure.
#[test]
fn accept_refuses_in_one_sentence_and_writes_nothing() {
    // A guess: no config.toml.
    let tmp = Tmp::new("guess");
    tmp.write("main.c", "int main(void) { return 0; }\n");
    harness_core::adopt::testing::adopt(&tmp.0);
    map(&tmp);
    refused(
        &accept(&tmp, "t-main", &[]),
        "the configuration is a guess, and a tool is built under a stated one",
    );
    assert!(!tmp.0.join("migration/tools").exists());
    // No map yet.
    let fresh = Tmp::new("no-map");
    fresh.write("main.c", "int main(void) { return 0; }\n");
    refused(
        &accept(&fresh, "t-main", &[]),
        "there is no project map yet: run `harness project map` first",
    );

    // Proposed: a config.toml that came with the project (decision 8).
    let shipped = Tmp::new("shipped");
    shipped.write("main.c", "int main(void) { return 0; }\n");
    shipped.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []\n",
    );
    map(&shipped);
    refused(
        &accept(&shipped, "t-main", &[]),
        "the configuration came with the project: state it with `harness project map --adopt`, \
         or write your own migration/map/config.toml",
    );
    // Stated once with --adopt: accepted.
    let run = harness(&["project", "map", "--target", shipped.arg(), "--adopt"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert_eq!(accept(&shipped, "t-main", &[]).code, 0);

    // Stated: an unknown id; the map changed; a root harness.toml.
    let tmp = Tmp::new("stated");
    tmp.write("main.c", "int main(void) { return 0; }\n");
    tmp.stated("");
    map(&tmp);
    refused(
        &accept(&tmp, "t-other", &[]),
        "the map has no program or library t-other (its ids are t-main)",
    );
    tmp.write("main.c", "int main(void) { return 1; }\n");
    refused(
        &accept(&tmp, "t-main", &[]),
        "the project changed since the map was made (its files, its configuration or the \
         compiler): run `harness project map` again, read the screen, then accept",
    );
    map(&tmp);
    tmp.write("harness.toml", "schema_version = 1\n");
    refused(
        &accept(&tmp, "t-main", &[]),
        "this project is already a folder-form target (it has a harness.toml at its root)",
    );
    std::fs::remove_file(tmp.0.join("harness.toml")).unwrap();
    assert!(!tmp.0.join("migration/tools").exists());
    // A bad id is a usage error.
    let run = accept(&tmp, "main", &[]);
    assert_eq!(run.code, 2, "{}", run.all());

    // Incomplete: a file that does not compile may define what main needs.
    let broken = Tmp::new("incomplete");
    broken.write(
        "main.c",
        "int helper(int x);\nint main(void) { return helper(1); }\n",
    );
    broken.write(
        "helper.c",
        "int helper(int x) { return x + 1; }\nint broken(void) { return }\n",
    );
    broken.stated("");
    map(&broken);
    refused(
        &accept(&broken, "t-main", &[]),
        "the closure of t-main is incomplete (helper.c did not compile and may define helper)",
    );
    assert!(!broken.0.join("migration/tools").exists());
}

/// accept links again before it writes: the map's own link result is
/// never taken as proof (a map file claiming "ok" for a program that does
/// not link is refused), and a person's keep that does not link is refused.
#[test]
fn accept_links_again_before_writing() {
    let tmp = Tmp::new("relink");
    tmp.write(
        "main.c",
        "int nowhere(int);\nint main(void) { return nowhere(1); }\n",
    );
    tmp.stated("");
    map(&tmp);
    // The map file claims the link proved it.
    let path = tmp.0.join("migration/map/project-map.json");
    let mut m = map_json(&tmp.0);
    assert_ne!(m["closures"][0]["linked"], "ok");
    m["closures"][0]["linked"] = serde_json::json!("ok");
    std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    refused(
        &accept(&tmp, "t-main", &[]),
        "t-main does not link with these files (missing nowhere): pick another definer with \
         --keep, or fix the program, map again, then accept",
    );
    assert!(!tmp.tool("t-main").exists());

    // A set linking settled; the person keeps the other definer, which
    // does not link.
    let tmp = Tmp::new("keep-nolink");
    tmp.write(
        "main.c",
        "int pick(void);\nint main(void) { return pick(); }\n",
    );
    tmp.write(
        "a/pick.c",
        "int absent_one(void);\nint pick(void) { return absent_one(); }\n",
    );
    tmp.write("b/pick.c", "int pick(void) { return 0; }\n");
    tmp.stated("");
    map(&tmp);
    let m = map_json(&tmp.0);
    assert_eq!(m["closures"][0]["duplicates"][0]["choice"]["keep"], "d1.2");
    refused(
        &accept(&tmp, "t-main", &["--keep", "d1=d1.1"]),
        "t-main does not link with these files (missing absent_one)",
    );
    // The linking's own choice is recorded as such.
    let run = accept(&tmp, "t-main", &[]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout
            .contains("keeping `b/pick.c` over `a/pick.c` for `pick` (settled by linking)"),
        "{}",
        run.stdout
    );
    let c = config(&tmp.tool("t-main"));
    assert_eq!(c.target.file_list().unwrap().picks[0].by, "links");
}

/// A program at the path of an accepted tool keeps its id across maps: a
/// second `main.c` would otherwise move both to their folder forms.
#[test]
fn an_accepted_tool_keeps_its_id_across_maps() {
    let tmp = Tmp::new("keep-id");
    tmp.write("app/main.c", "int main(void) { return 0; }\n");
    tmp.stated("");
    map(&tmp);
    assert_eq!(map_json(&tmp.0)["programs"][0]["id"], "t-main");
    assert_eq!(accept(&tmp, "t-main", &[]).code, 0);
    tmp.write("demo/main.c", "int main(void) { return 2; }\n");
    map(&tmp);
    let m = map_json(&tmp.0);
    let ids: Vec<(String, String)> = m["programs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["path"].as_str().unwrap().to_string(),
                p["id"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        ids,
        [
            ("demo/main.c".to_string(), "t-demo-main".to_string()),
            ("app/main.c".to_string(), "t-main".to_string()),
        ]
    );
}

/// A later map reports, per accepted tool, what changed, and records it in
/// the map file for `state status`: a header edit alone (its own files for
/// one tool, "nothing to do" for the other), a configuration change, a file
/// it now needs, a tool that no longer links, and the programs not accepted
/// as tools. A tool whose digests match says nothing.
#[test]
fn a_later_map_reports_what_changed_for_each_accepted_tool() {
    let tmp = Tmp::new("changed");
    lzg(&tmp);
    tmp.write("solo/util.h", "int util(int x);\n");
    tmp.write(
        "solo/solo.c",
        "#include \"util.h\"\nint main(void) { return util(1); }\n",
    );
    tmp.write(
        "solo/util.c",
        "#include \"util.h\"\nint util(int x) { return x; }\n",
    );
    tmp.stated("");
    map(&tmp);
    assert_eq!(accept(&tmp, "t-unlzg", &["--keep", "d1=d1.1"]).code, 0);
    assert_eq!(accept(&tmp, "t-solo", &[]).code, 0);
    // Nothing changed: nothing said.
    let run = map(&tmp);
    assert!(!run.stdout.contains("accepted tool"), "{}", run.stdout);

    // A header edit alone.
    tmp.write(
        "include/lzg.h",
        "int lzg_decode(int x);\nint lzg_size(int x);\nunsigned lzg_checksum(int x);\n/* v2 */\n",
    );
    let run = map(&tmp);
    let unlzg_files = "its own files changed since it was accepted (include/lzg.h); its closure, \
                       configuration and link are the same: scan it to read them (`harness scan \
                       --tool t-unlzg`); accepting it again only clears this note";
    for says in [
        format!("accepted tool t-unlzg changed since it was accepted: {unlzg_files}"),
        // A file elsewhere: nothing to do, and the notice stays silent.
        "accepted tool t-solo: a file elsewhere in the project changed; nothing to do for this \
         tool"
            .to_string(),
        "programs not accepted as tools: t-lzgcheck (tools/lzgcheck.c); accept one with \
         `harness project accept <id>`"
            .to_string(),
    ] {
        assert!(run.stdout.contains(&says), "{says}\n{}", run.stdout);
    }
    let changed: Vec<(String, String)> = map_json(&tmp.0)["accepted_tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["id"].as_str().unwrap().to_string(),
                r["changed"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        changed,
        [
            ("t-solo".to_string(), "none".to_string()),
            ("t-unlzg".to_string(), "files".to_string())
        ]
    );
    // state status says the map's sentence, and nothing for t-solo.
    let status = |id: &str| {
        let run = harness(&["state", "status", "--target", tmp.arg(), "--tool", id]);
        assert_eq!(run.code, 0, "{}", run.all());
        run.stdout
    };
    assert!(
        status("t-unlzg").contains(&format!("status: {unlzg_files}")),
        "{}",
        status("t-unlzg")
    );
    assert!(
        !status("t-solo").contains("elsewhere"),
        "{}",
        status("t-solo")
    );

    // A file it now needs, a configuration change, a new program, a tool
    // that no longer links.
    tmp.write(
        "tools/unlzg.c",
        "#include <lzg.h>\nint extra(void);\n\
         int main(void) { return lzg_decode(1) + lzg_size(2) + extra(); }\n",
    );
    tmp.write("src/extra.c", "int extra(void) { return 7; }\n");
    tmp.write("tools/newtool.c", "int main(void) { return 0; }\n");
    tmp.write("solo/util.c", "#include \"util.h\"\n");
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"plain\"\nfrom = \"stated\"\nflags = [\"-DLZG_FAST\"]\n",
    );
    let run = map(&tmp);
    let unlzg = "the project's files and the configuration or the compiler changed: closure \
                 changed: it now needs src/extra.c; configuration changed: it was plain, from \
                 stated, flags none, the map's is plain, from stated, flags -DLZG_FAST; accept it \
                 again with `harness project accept t-unlzg`";
    for says in [
        format!("accepted tool t-unlzg changed since it was accepted: {unlzg}"),
        "accepted tool t-solo changed since it was accepted: the project's files and the \
         configuration or the compiler changed: closure changed: it no longer needs \
         solo/util.c; configuration changed:"
            .to_string(),
        "it no longer links (missing util); accept it again with `harness project accept \
         t-solo`"
            .to_string(),
        "programs not accepted as tools: t-lzgcheck (tools/lzgcheck.c), t-newtool \
         (tools/newtool.c)"
            .to_string(),
    ] {
        assert!(run.stdout.contains(&says), "{says}\n{}", run.stdout);
    }
    assert!(status("t-unlzg").contains(&format!("status: {unlzg}")));
    // The tools were not touched.
    assert_eq!(
        listed(&tmp.tool("t-unlzg")),
        ["src/checksum.c", "src/decode.c", "tools/unlzg.c"]
    );
}

/// Edit the map file in place.
fn forge(tmp: &Tmp, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = tmp.0.join("migration/map/project-map.json");
    let mut m = map_json(&tmp.0);
    edit(&mut m);
    std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
}

/// Every other refusal through the binary, each alone, one sentence,
/// nothing written: a fuzzer and a fuzz driver, two definers for one set,
/// a library whose needed symbol may be defined in a file that did not
/// compile, an ambiguous include, a file in no compile_commands.json entry,
/// flags that differ, a bad run name, a run name for a library, the map's
/// limit, a map file whose choices, paths or ids are not the ones mapping
/// again finds, and a `migration/tools` that is a link (refused before any
/// lock is taken there).
#[test]
fn every_refusal_fires_alone_and_writes_nothing() {
    let nothing_written = |tmp: &Tmp| {
        assert!(
            !tmp.0.join("migration/tools").exists(),
            "a tool was written"
        );
    };

    // A fuzzer, and the driver that serves two of them.
    let tmp = Tmp::new("fuzz");
    let fuzzer = |n: u8| {
        format!(
            "#include <stddef.h>\n#include <stdint.h>\n\
             int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n) {{ return (int)n + {n}; }}\n"
        )
    };
    tmp.write("fuzz/fz.c", &fuzzer(0));
    tmp.write("fuzz/fz2.c", &fuzzer(1));
    tmp.write(
        "fuzz/driver.c",
        "#include <stddef.h>\n#include <stdint.h>\n\
         int LLVMFuzzerTestOneInput(const uint8_t *d, size_t n);\n\
         int main(void) { return LLVMFuzzerTestOneInput(0, 0); }\n",
    );
    tmp.write("app/app.c", "int main(void) { return 0; }\n");
    tmp.stated("");
    map(&tmp);
    refused(
        &accept(&tmp, "t-fz", &[]),
        "t-fz is a fuzzer, linked only with the project's fuzz driver: accept a main program or \
         a library",
    );
    refused(
        &accept(&tmp, "t-driver", &[]),
        "t-driver is a fuzz driver, never a tool alone: accept a main program or a library",
    );
    nothing_written(&tmp);

    // Two definers named for one set.
    let tmp = Tmp::new("two-keeps");
    tmp.write("main.c", "int f(void);\nint main(void) { return f(); }\n");
    tmp.write("a/f.c", "int f(void) { return 1; }\n");
    tmp.write("b/f.c", "int f(void) { return 2; }\n");
    tmp.stated("");
    map(&tmp);
    refused(
        &accept(&tmp, "t-main", &["--keep", "d1=d1.1", "--keep", "d1=d1.2"]),
        "--keep names two definers for d1 (a/f.c and b/f.c): keep one",
    );
    nothing_written(&tmp);
    // The map file's choice is not taken: forged as settled by linking,
    // the set is still held (both definers link).
    forge(&tmp, |m| {
        m["closures"][0]["duplicates"][0]["choice"] =
            serde_json::json!({"keep": "d1.2", "by": "links"});
        m["closures"][0]["questions"] = serde_json::json!([]);
    });
    refused(
        &accept(&tmp, "t-main", &[]),
        "duplicate set d1 of t-main (f) is not settled: pick its definer yourself with --keep \
         d1=<index or path> (its definers: d1.1 a/f.c, d1.2 b/f.c)",
    );
    // Its definers' paths swapped: the map file no longer says what
    // mapping again finds.
    map(&tmp);
    forge(&tmp, |m| {
        let d = &mut m["closures"][0]["duplicates"][0]["definers"];
        let first = d[0]["path"].clone();
        d[0]["path"] = d[1]["path"].clone();
        d[1]["path"] = first;
    });
    refused(
        &accept(&tmp, "t-main", &["--keep", "d1=d1.1"]),
        "migration/map/project-map.json does not say what mapping the project again finds (its \
         programs, libraries or the files and choices of t-main differ): run `harness project \
         map` again, read the screen, then accept",
    );
    // A program pointed at another file.
    map(&tmp);
    forge(&tmp, |m| {
        m["programs"][0]["path"] = serde_json::json!("a/f.c")
    });
    refused(
        &accept(&tmp, "t-main", &["--keep", "d1=d1.1"]),
        "does not say what mapping the project again finds",
    );
    // An id that is no id: refused before it is printed.
    map(&tmp);
    forge(&tmp, |m| {
        m["programs"][0]["id"] = serde_json::json!("t-x\nproject accept: wrote it")
    });
    refused(
        &accept(&tmp, "t-nope", &[]),
        "migration/map/project-map.json holds an id or an index that is not one the map writes: \
         run `harness project map` again",
    );
    // The limit hit.
    map(&tmp);
    forge(&tmp, |m| {
        m["limits_hit"] = serde_json::json!([{"limit": "files", "at": "x"}])
    });
    refused(
        &accept(&tmp, "t-main", &[]),
        "the map stopped at a limit and holds no programs",
    );
    nothing_written(&tmp);

    // A library whose code calls into a file that did not compile.
    let tmp = Tmp::new("lib-incomplete");
    tmp.write(
        "lib/crc.h",
        "unsigned crc(unsigned x);\nunsigned crc_step(unsigned x);\n",
    );
    tmp.write(
        "lib/crc.c",
        "#include \"crc.h\"\nunsigned crc(unsigned x) { return crc_step(x) ^ 1u; }\n",
    );
    tmp.write(
        "lib/step.c",
        "#include \"crc.h\"\nunsigned crc_step(unsigned x) { return x * 31u }\n",
    );
    tmp.stated("");
    map(&tmp);
    assert_eq!(map_json(&tmp.0)["libraries"][0]["id"], "l-crc");
    refused(
        &accept(&tmp, "l-crc", &[]),
        "the library l-crc is incomplete (lib/step.c did not compile and may define crc_step): \
         make those files compile or readable, map again, then accept",
    );
    nothing_written(&tmp);

    // An ambiguous include the configuration does not settle.
    let tmp = Tmp::new("ambiguous");
    tmp.write(
        "main.c",
        "#include \"x.h\"\n#include \"config.h\"\nint main(void) { return X + CONFIG - 2; }\n",
    );
    tmp.write("a/x.h", "#define X 1\n");
    tmp.write("a/config.h", "#define CONFIG 1\n");
    tmp.write("b/config.h", "#define CONFIG 2\n");
    tmp.stated("");
    map(&tmp);
    refused(
        &accept(&tmp, "t-main", &[]),
        "a file of t-main includes config.h, which a/config.h and b/config.h each hold, and the \
         configuration does not say which: settle it in migration/map/config.toml with -I or \
         system_headers, map again, then accept",
    );
    nothing_written(&tmp);

    // compile_commands.json: a file no entry lists, then flags that differ.
    let tmp = Tmp::new("cc");
    tmp.write("main.c", "int g(void);\nint main(void) { return g(); }\n");
    tmp.write("g.c", "int g(void) { return 0; }\n");
    let entry = |file: &str, flags: &[&str]| {
        let mut arguments = vec!["cc"];
        arguments.extend(flags);
        arguments.extend(["-c", file]);
        serde_json::json!({"directory": tmp.arg(), "file": file, "arguments": arguments})
    };
    tmp.write(
        "compile_commands.json",
        &serde_json::json!([entry("main.c", &[])]).to_string(),
    );
    harness_core::adopt::testing::adopt(&tmp.0);
    tmp.write(
        "migration/map/config.toml",
        "[[configuration]]\nname = \"cc\"\nfrom = \"compile_commands\"\nflags = []\n",
    );
    map(&tmp);
    refused(
        &accept(&tmp, "t-main", &[]),
        "g.c is in no compile_commands.json entry, so its flags are a guess: regenerate \
         compile_commands.json or state the build in migration/map/config.toml, map again, then \
         accept",
    );
    tmp.write(
        "compile_commands.json",
        &serde_json::json!([entry("main.c", &["-DA"]), entry("g.c", &["-DB"])]).to_string(),
    );
    map(&tmp);
    refused(
        &accept(&tmp, "t-main", &[]),
        "the files compile with different flags (main.c differs from g.c), so the configuration \
         is still a guess: state one build in migration/map/config.toml and map again",
    );
    nothing_written(&tmp);

    // A file name that is no run name; a run name for a library.
    let tmp = Tmp::new("run-name");
    tmp.write("main+x.c", "int main(void) { return 0; }\n");
    tmp.write("lib/crc.c", "unsigned crc(unsigned x) { return x; }\n");
    tmp.stated("");
    map(&tmp);
    refused(
        &accept(&tmp, "t-main-x", &[]),
        "`main+x` cannot be the run name (the file name features run the program as): give one \
         with --run-name, 1 to 64 letters, digits, ., _ or -",
    );
    for name in ["x", "../../escape", ".hidden"] {
        refused(
            &accept(&tmp, "l-crc", &["--run-name", name]),
            "--run-name names the program a tool runs, and l-crc is a library, which runs \
             nothing: leave --run-name out (a library's name is its id)",
        );
    }
    nothing_written(&tmp);

    // `migration/tools` a link to a folder outside holding a tool of that
    // id: refused before any lock, so no lock file is made out there.
    let outside = Tmp::new("outside");
    outside.write("t-main/harness.toml", "schema_version = 2\n");
    std::os::unix::fs::symlink(&outside.0, tmp.0.join("migration/tools")).unwrap();
    refused(
        &accept(&tmp, "t-main-x", &["--run-name", "x"]),
        "migration/tools is not a folder (a link or a file), so no tool is written there and no \
         lock is taken in it: move it away",
    );
    let names: Vec<String> = std::fs::read_dir(outside.0.join("t-main"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["harness.toml"], "no lock file outside the project");
}

/// A set reached only under one choice (d2, under d1.1): kept d1.2 and a
/// pick for d2, accept says d2 is not reached and records only d1; kept
/// d1.1, d2 must be picked and both are recorded.
#[test]
fn a_nested_set_is_recorded_only_when_reached() {
    let tmp = Tmp::new("nested");
    tmp.write("main.c", "int f(void);\nint main(void) { return f(); }\n");
    tmp.write("a/f.c", "int g(void);\nint f(void) { return g(); }\n");
    tmp.write("b/f.c", "int f(void) { return 2; }\n");
    tmp.write("c/g.c", "int g(void) { return 3; }\n");
    tmp.write("d/g.c", "int g(void) { return 4; }\n");
    tmp.stated("");
    map(&tmp);
    let m = map_json(&tmp.0);
    assert_eq!(
        m["closures"][0]["questions"],
        serde_json::json!(["d1", "d2"])
    );
    assert_eq!(
        m["closures"][0]["duplicates"][1]["under"],
        serde_json::json!(["d1.1"])
    );
    let run = accept(&tmp, "t-main", &["--keep", "d1=d1.2", "--keep", "d2=d2.1"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout
            .contains("project accept t-main: d2 is not reached with these picks; not recorded"),
        "{}",
        run.stdout
    );
    let list = config(&tmp.tool("t-main"))
        .target
        .file_list()
        .unwrap()
        .clone();
    assert_eq!(list.picks.len(), 1, "{:?}", list.picks);
    assert_eq!(list.picks[0].keep, "b/f.c");
    assert_eq!(listed(&tmp.tool("t-main")), ["b/f.c", "main.c"]);
    // Kept d1.1: d2 is reached and must be picked.
    refused(
        &accept(&tmp, "t-main", &["--keep", "d1=d1.1"]),
        "duplicate set d2 of t-main (g) is not settled",
    );
    let run = accept(&tmp, "t-main", &["--keep", "d1=d1.1", "--keep", "d2=d/g.c"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(!run.stdout.contains("not reached"), "{}", run.stdout);
    let list = config(&tmp.tool("t-main"))
        .target
        .file_list()
        .unwrap()
        .clone();
    let kept: Vec<&str> = list.picks.iter().map(|p| p.keep.as_str()).collect();
    assert_eq!(kept, ["a/f.c", "d/g.c"]);
    assert_eq!(listed(&tmp.tool("t-main")), ["a/f.c", "d/g.c", "main.c"]);
}

/// Accepting again keeps what the person added to the tool's harness.toml
/// (a whole-program check, a model, a stage override, an allowlist entry)
/// and says so; the first accept writes the whole-program check and the
/// model as commented examples and says the check is off.
#[test]
fn accepting_again_keeps_what_the_person_added() {
    let tmp = Tmp::new("keep-added");
    tmp.write("app/main.c", "int main(void) { return 0; }\n");
    tmp.stated("");
    map(&tmp);
    let run = accept(&tmp, "t-main", &[]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout.contains(
            "project accept t-main: the whole-program check is off until you fill in \
             [oracle.whole_program] in migration/tools/t-main/harness.toml (a commented example \
             is there)"
        ),
        "{}",
        run.stdout
    );
    let first = tmp.read("migration/tools/t-main/harness.toml");
    for line in [
        "# [oracle.whole_program]\n# args = [\"-c\"]\n",
        "# model = \"my-claude-code\"\n",
    ] {
        assert!(first.contains(line), "{line}\n{first}");
    }
    assert!(!run.stdout.contains("kept from"), "{}", run.stdout);
    // The person fills in the check and names who answers.
    let added = first.replace(
        "# [oracle.whole_program]\n# args = [\"-c\"]\n",
        "[oracle.whole_program]\nargs = [\"-9\"]\n",
    );
    let added = added.replace(
        "# model = \"my-claude-code\"\n",
        "model = \"my-claude-code\"\n\n[llm.driver]\nmax_repairs = 2\n",
    );
    let added = added.replace(
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\"]",
        "allowlist = [\"cc\", \"cargo\", \"rustc\", \"nm\", \"rustfmt\"]",
    );
    std::fs::write(tmp.tool("t-main"), &added).unwrap();
    let c = config(&tmp.tool("t-main"));
    assert!(c.oracle.contains_key("whole_program"));
    // Accepted again, with another run name: the run name changes, the rest
    // is kept, and the closing line says what.
    let run = accept(&tmp, "t-main", &["--run-name", "app"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert!(
        run.stdout.contains(
            "; kept from the harness.toml there: oracle.allowlist (rustfmt), \
             oracle.whole_program, llm.driver, llm.model (its own comments are not carried over)"
        ),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("whole-program check is off"),
        "{}",
        run.stdout
    );
    let c = config(&tmp.tool("t-main"));
    assert_eq!(c.target.name, "app");
    assert_eq!(c.oracle["whole_program"]["args"].to_string(), "[\"-9\"]");
    assert_eq!(c.llm.model, "my-claude-code");
    assert_eq!(c.llm.driver.as_ref().unwrap().max_repairs, Some(2));
    assert!(c.oracle_allowlist().contains(&"rustfmt".to_string()));
    let again = tmp.read("migration/tools/t-main/harness.toml");
    assert!(!again.contains("# [oracle.whole_program]"), "{again}");
    assert!(!again.contains("# model ="), "{again}");
    // And again: the same file (the merge is stable).
    let run = accept(&tmp, "t-main", &["--run-name", "app"]);
    assert_eq!(run.code, 0, "{}", run.all());
    assert_eq!(tmp.read("migration/tools/t-main/harness.toml"), again);
}

/// An accepted library keeps its id when a new file that sorts first joins
/// it; when its files move to another library, the "what changed" line
/// names that library's id.
#[test]
fn a_library_keeps_its_id_when_a_file_joins_it() {
    let tmp = Tmp::new("lib-id");
    tmp.write(
        "lib/crc.h",
        "unsigned crc(unsigned);\nunsigned crc_step(unsigned);\nunsigned aa(unsigned);\n",
    );
    tmp.write(
        "lib/crc.c",
        "#include \"crc.h\"\nunsigned crc(unsigned x) { return crc_step(x) ^ 1u; }\n",
    );
    tmp.write(
        "lib/step.c",
        "#include \"crc.h\"\nunsigned crc_step(unsigned x) { return x * 31u; }\n",
    );
    tmp.stated("");
    map(&tmp);
    assert_eq!(accept(&tmp, "l-crc", &[]).code, 0);
    // lib/aa.c, which crc.c now calls, sorts before crc.c.
    tmp.write(
        "lib/aa.c",
        "#include \"crc.h\"\nunsigned aa(unsigned x) { return x; }\n",
    );
    tmp.write(
        "lib/crc.c",
        "#include \"crc.h\"\nunsigned crc(unsigned x) { return crc_step(aa(x)) ^ 1u; }\n",
    );
    let run = map(&tmp);
    let m = map_json(&tmp.0);
    assert_eq!(m["libraries"][0]["id"], "l-crc", "{}", run.stdout);
    assert_eq!(
        m["libraries"][0]["files"],
        serde_json::json!(["lib/aa.c", "lib/crc.c", "lib/step.c"])
    );
    assert!(
        run.stdout.contains(
            "accepted tool l-crc changed since it was accepted: the project's files changed: \
             closure changed: it now needs lib/aa.c; accept it again with `harness project \
             accept l-crc`"
        ),
        "{}",
        run.stdout
    );
    assert_eq!(accept(&tmp, "l-crc", &[]).code, 0);
}

/// zopfli (§4): on a copy without its root harness.toml, with a
/// config.toml stating `flags = []`, `map` then `accept` gives a tool
/// whose scan writes the committed facts byte for byte, whose plan keeps
/// every committed `source_hash`, and whose `u001-katajainen` (the
/// committed crate copied under the tool's ledger) verifies green.
#[test]
fn zopfli_accepted_from_its_map_scans_plans_and_verifies_as_committed() {
    let tmp = Tmp::new("zopfli");
    copy_dir(
        &repo().join("targets/zopfli"),
        &tmp.0,
        &["harness.toml", "migration/build", "migration/.lock"],
    );
    tmp.stated("");
    map(&tmp);
    let m = map_json(&tmp.0);
    // The program's id comes from its file, src/zopfli/zopfli_bin.c.
    assert_eq!(m["programs"][0]["id"], "t-zopfli_bin");
    let run = accept(&tmp, "t-zopfli_bin", &["--run-name", "zopfli"]);
    assert_eq!(run.code, 0, "{}", run.all());
    let tool = tmp.tool("t-zopfli_bin");
    let c = config(&tool);
    let list = c.target.file_list().unwrap();
    assert_eq!(c.target.name, "zopfli");
    assert_eq!(list.files.len(), 13);
    assert!(list.files.iter().all(|f| f.include_dirs.is_empty()));
    assert!(list.configuration.flags.is_empty());
    assert!(tmp
        .read("migration/tools/t-zopfli_bin/harness.toml")
        .contains("extra_link_args = [\"-lm\"]"));

    let target = tmp.arg().to_string();
    let run = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend(["--target", &target, "--tool", "t-zopfli_bin"]);
        let r = harness(&argv);
        assert_eq!(r.code, 0, "{args:?}: {}", r.all());
        r
    };
    run(&["scan"]);
    let ledger = harness_core::config::tool_dir(&tmp.0, "t-zopfli_bin");
    assert_eq!(
        std::fs::read(ledger.join("facts.jsonl")).unwrap(),
        std::fs::read(repo().join("targets/zopfli/migration/facts.jsonl")).unwrap(),
        "facts.jsonl differs from the committed one"
    );
    // A fresh plan: every unit's source_hash, by its files, as committed.
    run(&["plan"]);
    // Each unit's `files` line and `source_hash` line.
    let hashes = |text: &str| -> std::collections::BTreeMap<String, String> {
        text.split("[[unit]]")
            .skip(1)
            .map(|block| {
                let line = |key: &str| {
                    block
                        .lines()
                        .find(|l| l.starts_with(key))
                        .unwrap_or_else(|| panic!("{key} in {block}"))
                        .to_string()
                };
                (line("files = "), line("source_hash = "))
            })
            .collect()
    };
    let committed =
        std::fs::read_to_string(repo().join("targets/zopfli/migration/plan.toml")).unwrap();
    let fresh = std::fs::read_to_string(ledger.join("plan.toml")).unwrap();
    assert_eq!(hashes(&fresh), hashes(&committed));
    // The committed plan and unit under the tool's ledger: verify green.
    std::fs::write(
        ledger.join("plan.toml"),
        committed.replace(
            "driver = \"migration/units/",
            "driver = \"migration/tools/t-zopfli_bin/units/",
        ),
    )
    .unwrap();
    copy_dir(
        &repo().join("targets/zopfli/migration/units/u001-katajainen"),
        &ledger.join("units/u001-katajainen"),
        &["katajainen_rs/target"],
    );
    let r = run(&["plan"]);
    assert!(r.stdout.contains("plan: no changes"), "{}", r.stdout);
    let r = run(&["verify", "u001-katajainen", "--allow-unsandboxed"]);
    assert!(r.stdout.contains("u001-katajainen GREEN"), "{}", r.all());
}

/// Copy `src` into `dst`, leaving out the names in `skip` (relative to
/// `src`).
fn copy_dir(src: &Path, dst: &Path, skip: &[&str]) {
    fn walk(src: &Path, dst: &Path, rel: &str, skip: &[&str]) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let here = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if skip.contains(&here.as_str()) {
                continue;
            }
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&entry.path(), &dst.join(&name), &here, skip);
            } else if kind.is_file() {
                std::fs::copy(entry.path(), dst.join(&name)).unwrap();
            }
        }
    }
    walk(src, dst, "", skip);
}
