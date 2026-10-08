//! harness-mcp's reads on a file-list target (docs/PROJECT-MAP-DESIGN.md
//! §3.7): a hand-written liblzg-shaped mapped tool, scanned and planned by
//! the CLI, read with `--tool` — the status, a unit with its C, and a
//! request page — none refused for its form.

mod common;

use common::*;
use serde_json::json;
use std::path::Path;
use std::process::Command;

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn every_read_works_on_a_file_list_tool() {
    let tmp = TempDir::new("file-list");
    let root = &tmp.0;
    put(
        root,
        "src/lib/encode.c",
        "#include \"internal.h\"\nunsigned lzg_encode(unsigned n) { return lzg_min(n, 9); }\n",
    );
    put(
        root,
        "src/lib/internal.h",
        "#include \"../include/lzg.h\"\n\
         static inline unsigned lzg_min(unsigned a, unsigned b) { return a < b ? a : b; }\n",
    );
    put(
        root,
        "src/include/lzg.h",
        "unsigned lzg_encode(unsigned n);\n",
    );
    put(
        root,
        "src/tools/lzg.c",
        "#include <lzg.h>\nint main(void) { return (int)lzg_encode(3); }\n",
    );
    put(
        root,
        "src/other/decode.c",
        "int lzg_decode(void) { return 1; }\n",
    );
    put(
        root,
        "migration/tools/t-lzg/harness.toml",
        "schema_version = 2\n[target]\nname = \"lzg\"\nfiles = [\n\
         { path = \"src/lib/encode.c\", include_dirs = [\"src/include\"] },\n\
         { path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] },\n]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [] }\n",
    );
    let harness = harness_bin();
    let target = root.to_str().unwrap();
    // A hand-written tool holds no results: no adoption is asked.
    for cmd in ["scan", "plan"] {
        let args = [cmd, "--target", target, "--tool", "t-lzg"];
        let out = Command::new(&harness).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut c = Client::start(&[
        "--target",
        target,
        "--tool",
        "t-lzg",
        "--harness",
        harness.to_str().unwrap(),
    ]);
    c.initialize();
    c.call(1, "harness_status", json!({}), None);
    let (r, _) = c.response(&json!(1), 30);
    assert!(!is_error(&r), "{r}");
    // Which tool's ledger the answers come from: in the status, and in the
    // start line.
    assert_eq!(structured(&r)["tool"], "t-lzg", "{r}");
    let started = c.stderr.lock().unwrap().clone();
    assert!(
        started.contains(&format!("serving {target} · tool t-lzg (")),
        "{started}"
    );
    let status = structured(&r).to_string();
    for id in ["u-encode", "u-lzg"] {
        assert!(status.contains(id), "{id}: {status}");
    }
    assert!(!status.contains("decode"), "{status}");
    c.call(2, "harness_unit", json!({"unit": "u-encode"}), None);
    let (r, _) = c.response(&json!(2), 30);
    assert!(!is_error(&r), "{r}");
    let unit = structured(&r).to_string();
    assert!(
        unit.contains("src/lib/encode.c") && unit.contains("lzg_encode"),
        "{unit}"
    );
    c.call(3, "harness_request", json!({"unit": "u-encode"}), None);
    let (r, _) = c.response(&json!(3), 30);
    let request = r.to_string();
    for words in ["lists its files", "source_dir"] {
        assert!(!request.contains(words), "{request}");
    }
    c.close_stdin();
    assert!(c.wait_exit(30).success());
}
