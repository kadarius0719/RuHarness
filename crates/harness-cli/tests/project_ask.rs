//! `harness project ask` through the binary (docs/PROJECT-MAP-DESIGN.md
//! §3.4, §3.8, §4 "The model step"), over a **fixed map fixture**
//! (`tests/fixtures/project_ask/`: a small project and its hand-written
//! `project-map.json`), so the request bytes and the trace keys are the same
//! on every platform. The recorded responses under `fixtures/project_ask/
//! traces/` answer the fixture's three questions (the held sets, the held
//! sets with `--programs t-main`, and `--build`); every other reply is
//! written by the test into the response file the hand-off names. No model
//! is ever called. Every child runs with the test process's own adoption
//! file, and each copy of the fixture is adopted before it is asked.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_ask")
}

const MAP: &str = "migration/map/project-map.json";
const REPLY: &str = "migration/map/project-map.reply.json";
const PROPOSED: &str = "migration/map/config.proposed.toml";
const TRACES: &str = "migration/map/traces";

/// A fresh copy of the fixture project with its map in place, adopted.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "harness-cli-ask-{tag}-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        copy_dir(&fixture().join("project"), &dir);
        std::fs::create_dir_all(dir.join("migration/map")).unwrap();
        std::fs::copy(fixture().join("project-map.json"), dir.join(MAP)).unwrap();
        harness_core::adopt::testing::adoption_file();
        harness_core::adopt::adopt(&dir).unwrap();
        Tmp(dir)
    }

    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn map(&self) -> serde_json::Value {
        serde_json::from_str(&self.read(MAP)).unwrap()
    }

    fn set_map(&self, map: &serde_json::Value) {
        self.write(MAP, &serde_json::to_string_pretty(map).unwrap());
    }

    fn reply(&self) -> serde_json::Value {
        serde_json::from_str(&self.read(REPLY)).unwrap()
    }

    /// The recorded responses, copied into the traces folder.
    fn with_recorded(&self) -> &Tmp {
        copy_dir(&fixture().join("traces"), &self.path(TRACES));
        self
    }

    /// Request files with no response beside them, sorted.
    fn pending(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = std::fs::read_dir(self.path(TRACES))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                name.ends_with(".request.json")
                    && !p
                        .with_file_name(name.replace(".request.json", ".response.json"))
                        .exists()
            })
            .collect();
        out.sort();
        out
    }

    /// Answer the pending `request` with `text`; the response file's path.
    fn answer(&self, request: &Path, text: &str) -> PathBuf {
        let name = request.file_name().unwrap().to_string_lossy();
        let response = request.with_file_name(name.replace(".request.json", ".response.json"));
        let body = serde_json::json!({
            "text": text,
            "input_tokens": 0,
            "output_tokens": 0,
            "stop_reason": "end_turn",
        });
        std::fs::write(&response, serde_json::to_string_pretty(&body).unwrap()).unwrap();
        response
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
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

/// `harness project ask --target <tmp> <extra…>`.
fn ask(t: &Tmp, extra: &[&str]) -> Run {
    let mut args = vec!["project", "ask", "--target", t.arg()];
    args.extend_from_slice(extra);
    harness(&args)
}

fn the_request(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A valid reply to the held sets d1 and d2 (and p2 when asked).
const SETS_REPLY: &str = r#"[{"item":"d1","keep":"d1.2","reason":"platform"},
{"item":"d2","keep":"undecided","reason":"cannot-tell"}]"#;

// ---------- the recorded answers ----------

#[test]
fn a_recorded_answer_is_read_as_advice_and_shown_with_its_link_results() {
    for provider in ["replay", "external"] {
        let t = Tmp::new("recorded");
        t.with_recorded();
        let run = ask(&t, &["--allow-guessed", "--provider", provider]);
        assert_eq!(run.code, 0, "{provider}: {}{}", run.stdout, run.stderr);
        let out = &run.stdout;
        // The advised definer, and from the map's own link results whether
        // that choice linked in each program holding the set.
        assert!(
            out.contains(
                "d1 (decode; held by t-main, t-other): the model's advice (claude-sonnet-5): \
                 keep d1.2 src/mini.c, reason platform; in t-main that choice linked; in \
                 t-other it did not link"
            ),
            "{out}"
        );
        // `undecided` goes to the person.
        assert!(
            out.contains(
                "d2 (fast_crc; held by t-main): the model could not decide (undecided, reason \
                 cannot-tell) (claude-sonnet-5): the choice goes to you — keep one of d2.1 \
                 src/crc_a.c, d2.2 src/crc_b.c"
            ),
            "{out}"
        );
        assert!(out.contains("labels and advice only"), "{out}");
        let reply = t.reply();
        assert_eq!(reply["schema"], "ruharness-project-map-reply");
        assert_eq!(reply["root_hash"], t.map()["root_hash"]);
        assert_eq!(reply["inputs_hash"], t.map()["inputs_hash"]);
        assert_eq!(reply["items"]["d1"]["keep"], "d1.2");
        assert_eq!(reply["items"]["d1"]["model"], "claude-sonnet-5");
        assert_eq!(reply["items"]["d1"]["provider"], provider);
        assert_eq!(reply["items"]["d2"]["keep"], "undecided");
        // No source text in the reply file, and nothing else written.
        let text = t.read(REPLY);
        assert!(!text.contains("SOURCE-SENTINEL"), "{text}");
        assert!(!text.contains("while (*s)"), "{text}");
        assert!(!t.path(PROPOSED).exists());
    }
}

#[test]
fn the_request_fences_the_project_text() {
    let t = Tmp::new("fence");
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let pending = t.pending();
    assert_eq!(pending.len(), 1);
    let req = the_request(&pending[0]);
    let user = req["user"].as_str().unwrap();
    let system = req["system"].as_str().unwrap();
    // The slice of each duplicated definition is sent, fenced and escaped.
    assert!(user.contains("SOURCE-SENTINEL-DECODE"), "{user}");
    assert!(user.contains("\\u003c/project_facts_0>"), "{user}");
    assert!(!user.contains("</project_facts_0>"), "{user}");
    assert!(user.contains("item=d1 kind=duplicate-set definers=d1.1,d1.2"));
    assert!(
        !system.contains("decode"),
        "no project text in the trusted part"
    );
    // Programs are asked about only when named.
    assert!(!user.contains("item=p"), "{user}");
}

// ---------- refusals ----------

#[test]
fn ask_is_refused_while_the_configuration_is_a_guess_unless_allowed() {
    let t = Tmp::new("guessed");
    let run = ask(&t, &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(
            "the configuration is a guess, so the questions may be wrong: state it in \
             migration/map/config.toml (or ask a model to propose one with `harness project ask \
             --build`) and map again, or pass --allow-guessed"
        ),
        "{}",
        run.stderr
    );
    assert!(!t.path(TRACES).exists(), "nothing made before the refusal");
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("awaiting response: "), "{}", run.stderr);
    // The awaiting line names the envelope and who answers.
    assert!(
        run.stderr.contains(
            "as {\"text\": <the reply>, \"input_tokens\": 0, \"output_tokens\": 0, \
             \"stop_reason\": \"end_turn\"}, then re-run (the answer is recorded as \
             `claude-sonnet-5`'s; if another model or a person answers, first run it with \
             --model naming who answers: that writes the request to answer)"
        ),
        "{}",
        run.stderr
    );
    // A stated configuration needs no flag.
    let mut map = t.map();
    map["configuration"]["source"] = "stated".into();
    t.set_map(&map);
    let run = ask(&t, &[]);
    assert!(run.stderr.contains("awaiting response: "), "{}", run.stderr);
}

#[test]
fn ask_is_refused_with_no_open_questions_but_build_is_not() {
    let t = Tmp::new("closed");
    let mut map = t.map();
    for c in map["closures"].as_array_mut().unwrap() {
        c["questions"] = serde_json::json!([]);
    }
    t.set_map(&map);
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("nothing is open: every program linked and every duplicate set is settled"),
        "{}",
        run.stderr
    );
    let run = ask(&t, &["--build"]);
    assert!(run.stderr.contains("awaiting response: "), "{}", run.stderr);
    // Under a guess with nothing held, the open question is the build.
    let run = ask(&t, &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(
            "the configuration is a guess, and that is the question still open: ask a model to \
             propose one with `harness project ask --build`, or state it yourself in \
             migration/map/config.toml, then run `harness project map` again"
        ),
        "{}",
        run.stderr
    );
    // "Every program linked" only when it is true.
    let mut map = t.map();
    map["configuration"]["source"] = "stated".into();
    map["closures"][2]["linked"] = serde_json::json!({"missing": ["gone"], "doubled": []});
    t.set_map(&map);
    let run = ask(&t, &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(
            "nothing is open: no duplicate set is held, but t-other did not link (see `harness \
             project map`); a model is not asked about that"
        ),
        "{}",
        run.stderr
    );
}

/// A set the map held without linking its choices (more than it tries)
/// is said to be so, never "it did not link".
#[test]
fn a_set_the_map_did_not_link_is_said_so() {
    let t = Tmp::new("not-tried");
    t.with_recorded();
    let mut map = t.map();
    for c in map["closures"].as_array_mut().unwrap() {
        if c["program"] == "t-main" {
            for d in c["duplicates"].as_array_mut().unwrap() {
                d["links"] = serde_json::json!([]);
            }
        }
    }
    t.set_map(&map);
    let run = ask(&t, &["--allow-guessed", "--provider", "replay"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains(
            "d1 (decode; held by t-main, t-other): the model's advice (claude-sonnet-5): keep \
             d1.2 src/mini.c, reason platform; in t-other it did not link; in t-main the map did \
             not link these choices (too many to try)"
        ),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("in t-main it did not link"),
        "{}",
        run.stdout
    );
}

/// A response cut short names its file and says to answer again (`ask`
/// has no budget flag to raise).
#[test]
fn a_truncated_response_names_its_file() {
    let t = Tmp::new("truncated");
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let request = t.pending().remove(0);
    let response = t.answer(&request, SETS_REPLY);
    let mut body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
    body["stop_reason"] = "max_tokens".into();
    std::fs::write(&response, body.to_string()).unwrap();
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains(&format!(
            "external: the response file {} says the reply was cut short (stop_reason \
             `max_tokens`): delete it and answer again with the whole reply and stop_reason \
             \"end_turn\"",
            response.display()
        )),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("larger budget"), "{}", run.stderr);
    assert!(!t.path(REPLY).exists());
}

#[test]
fn ask_without_a_map_says_to_map_first() {
    let t = Tmp::new("nomap");
    std::fs::remove_file(t.path(MAP)).unwrap();
    let run = ask(&t, &["--build"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("no map written yet: run `harness project map`"),
        "{}",
        run.stderr
    );
}

// ---------- the resume command ----------

#[test]
fn the_resume_command_carries_every_flag_but_adopt_and_json() {
    let t = Tmp::new("resume");
    let run = harness(&[
        "--json",
        "--adopt",
        "project",
        "ask",
        "--target",
        t.arg(),
        "--build",
        "--model",
        "m-test",
        "--allow-guessed",
    ]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let want = format!(
        "harness project ask --target={} --build --allow-guessed --provider=external \
         --model=m-test",
        t.arg()
    );
    assert!(run.stderr.contains(&want), "{}", run.stderr);
    let awaiting: serde_json::Value = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|e| e["k"] == "awaiting")
        .expect("an awaiting event");
    assert_eq!(awaiting["resume"], want.as_str());
    let key = awaiting["request_key"].as_str().unwrap();
    assert!(t.path(&format!("{TRACES}/{key}.request.json")).is_file());
    // The model id is in the request, so in its key.
    let req = the_request(&t.path(&format!("{TRACES}/{key}.request.json")));
    assert_eq!(req["model"], "m-test");
}

// ---------- forged and failed replies ----------

#[test]
fn forged_replies_are_refused_in_full_naming_the_index_and_the_rule() {
    let t = Tmp::new("forged");
    let run = ask(&t, &["--allow-guessed", "--programs", "t-main"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let request = t.pending().remove(0);
    let user = the_request(&request)["user"].as_str().unwrap().to_string();
    assert!(user.contains("item=p2 kind=program"), "{user}");
    let p2 = r#"{"item":"p2","kind":"tool","name":"Main","purpose":"Decodes and checks."}"#;
    let d1 = r#"{"item":"d1","keep":"d1.2","reason":"platform"}"#;
    let d2 = r#"{"item":"d2","keep":"undecided","reason":"cannot-tell"}"#;
    let cases: Vec<(String, &str)> = vec![
        (
            format!(
                "[{p2},{d1},{d2},{}]",
                r#"{"item":"d9","keep":"undecided","reason":"cannot-tell"}"#
            ),
            "names the item `d9`, which was not asked",
        ),
        (format!("[{p2},{d1}]"), "item `d2` is not answered"),
        (
            format!(
                "[{},{d1},{d2}]",
                r#"{"item":"p2","kind":"tool","name":"Main","purpose":"x","why":"y"}"#
            ),
            "item `p2`: unknown field `why`",
        ),
        (
            format!(
                "[{},{d1},{d2}]",
                r#"{"item":"p2","kind":"daemon","name":"Main","purpose":"x"}"#
            ),
            "item `p2`: the kind `daemon` is not one of",
        ),
        (
            format!(
                "[{},{d1},{d2}]",
                r#"{"item":"p2","kind":"tool","name":"A name that runs on for over forty characters","purpose":"x"}"#
            ),
            "item `p2`: `name` is over 40 characters",
        ),
        (
            format!(
                "[{},{d1},{d2}]",
                "{\"item\":\"p2\",\"kind\":\"tool\",\"name\":\"Main\\u001b]0;x\",\"purpose\":\"x\"}"
            ),
            "holds a character that cannot be shown (U+001B)",
        ),
        (
            format!(
                "[{p2},{},{d2}]",
                r#"{"item":"d1","keep":"d2.1","reason":"platform"}"#
            ),
            "item `d1`: keep `d2.1` is not one of its definers",
        ),
        // A key twice: a plain reader would keep the last copy, a person
        // reading the file the first.
        (
            format!(
                "[{p2},{},{d2}]",
                r#"{"item":"d1","keep":"d1.1","keep":"d1.2","reason":"platform"}"#
            ),
            "an object names the key `keep` twice",
        ),
        (
            format!(
                "[{},{d1},{d2}]",
                r#"{"item":"d9","item":"p2","kind":"tool","name":"Main","purpose":"x"}"#
            ),
            "an object names the key `item` twice",
        ),
        // A name with no character of its own.
        (
            format!(
                "[{},{d1},{d2}]",
                "{\"item\":\"p2\",\"kind\":\"tool\",\"name\":\"\\u0301\\u0301\",\"purpose\":\"x\"}"
            ),
            "item `p2`: `name` holds only combining marks, no character of its own",
        ),
    ];
    for (reply, says) in &cases {
        let response = t.answer(&request, reply);
        let run = ask(&t, &["--allow-guessed", "--programs", "t-main"]);
        assert_eq!(run.code, 1, "{reply}");
        assert!(run.stderr.contains(says), "{says}\n{}", run.stderr);
        assert!(
            run.stderr.contains("delete it and answer again"),
            "{}",
            run.stderr
        );
        assert!(
            run.stderr.contains(&response.display().to_string()),
            "names the response file: {}",
            run.stderr
        );
        assert!(!t.path(REPLY).exists(), "refused in full: {reply}");
        std::fs::remove_file(&response).unwrap();
    }
    // Answered again, correctly.
    t.answer(&request, &format!("[{p2},{d1},{d2}]"));
    let run = ask(&t, &["--allow-guessed", "--programs", "t-main"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains(
            "p2 t-main (src/main.c): the model's label (claude-sonnet-5): kind tool, name \
             \"Main\"; purpose, in its words: Decodes and checks."
        ),
        "{}",
        run.stdout
    );
}

#[test]
fn a_recorded_reply_that_fails_the_contract_says_to_record_a_live_run() {
    let t = Tmp::new("replaybad");
    ask(&t, &["--allow-guessed"]);
    let request = t.pending().remove(0);
    t.answer(&request, "[]");
    let run = ask(&t, &["--allow-guessed", "--provider", "replay"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("record a live run"), "{}", run.stderr);
}

#[test]
fn a_build_reply_outside_the_grammar_or_citing_a_file_not_sent_is_refused() {
    let t = Tmp::new("buildbad");
    let run = ask(&t, &["--build"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let request = t.pending().remove(0);
    for (reply, says) in [
        (
            r#"{"name":"make","from":"make","flags":[{"flag":"-DUSE_FAST","cites":["Makefile:2"]},{"flag":"-fplugin=evil.so","cites":["Makefile:2"]}],"assumptions":[]}"#,
            "flag `-fplugin=evil.so`: the flag `-fplugin=evil.so` is not one the harness passes",
        ),
        (
            r#"{"name":"make","from":"make","flags":[{"flag":"-DUSE_FAST","cites":["config.mk:1"]}],"assumptions":[]}"#,
            "the cite `config.mk:1` names a file that was not sent",
        ),
        (
            r#"{"name":"make","from":"make","flags":[{"flag":"-Iinclude/../../etc","cites":["Makefile:2"]}],"assumptions":[]}"#,
            "names a path outside the project",
        ),
        (
            r#"{"name":"make","from":"make","flags":[],"assumptions":["one\ntwo"]}"#,
            "cannot be shown (U+000A)",
        ),
    ] {
        let response = t.answer(&request, reply);
        let run = ask(&t, &["--build"]);
        assert_eq!(run.code, 1, "{reply}");
        assert!(run.stderr.contains(says), "{says}\n{}", run.stderr);
        assert!(run.stderr.contains("delete it and answer again"));
        assert!(!t.path(PROPOSED).exists(), "refused in full");
        std::fs::remove_file(response).unwrap();
    }
}

// ---------- --build ----------

#[test]
fn build_is_allowed_under_a_guess_and_writes_only_the_proposal() {
    let t = Tmp::new("build");
    t.with_recorded();
    let map_before = t.read(MAP);
    let run = ask(&t, &["--build", "--provider", "replay"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("project ask --build: sending 2 build file(s): CMakeLists.txt, Makefile"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("the map's configuration is a guess"));
    assert!(run
        .stdout
        .contains("  -DUSE_FAST (cites Makefile:2, CMakeLists.txt:3)"));
    let text = t.read(PROPOSED);
    assert!(text.starts_with("# The model's proposal (claude-sonnet-5 through `replay`)"));
    assert!(text.contains("copy the [[configuration]] entry"));
    // config.toml's own form (the library's test reads it back with toml).
    assert!(
        text.contains(
            "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = [\n    \
             \"-DUSE_FAST\", # cites Makefile:2, CMakeLists.txt:3\n    \"-Iinclude\", # cites \
             Makefile:2\n    \"-std=c99\", # cites Makefile:2\n]\n"
        ),
        "{text}"
    );
    assert!(text.contains("# - The Makefile's default target is the build that counts."));
    // Nothing else changed.
    assert_eq!(t.read(MAP), map_before);
    assert!(!t.path(REPLY).exists());
    assert!(!t.path("migration/map/config.toml").exists());
}

#[test]
fn the_build_files_sent_are_capped_and_the_left_out_named() {
    let t = Tmp::new("caps");
    let line = "# filler line for the size caps of project ask --build\n";
    let fill = |kib: usize| line.repeat(kib * 1024 / line.len() + 1);
    t.write("sub/big.mk", &fill(70));
    t.write("a.mk", &fill(60));
    t.write("b.mk", &fill(50));
    t.write("c.mk", &fill(30));
    std::os::unix::fs::symlink(t.path("Makefile"), t.path("link.mk")).unwrap();
    t.write("README.txt", "not a build file\n");
    let mut map = t.map();
    map["build_evidence"]["build_files"] = serde_json::json!([
        "CMakeLists.txt",
        "Makefile",
        "README.txt",
        "a.mk",
        "b.mk",
        "c.mk",
        "link.mk",
        "sub/big.mk"
    ]);
    t.set_map(&map);
    let run = ask(&t, &["--build"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let out = &run.stdout;
    assert!(
        out.contains("sending 4 build file(s): CMakeLists.txt, Makefile, b.mk, c.mk"),
        "{out}"
    );
    assert!(
        out.contains("not sent: sub/big.mk (it is over 64 KiB)"),
        "{out}"
    );
    assert!(
        out.contains(
            "not sent: a.mk (the build files together are over 128 KiB, and it is the largest)"
        ),
        "{out}"
    );
    assert!(out.contains("not sent: link.mk (it is a link"), "{out}");
    assert!(
        out.contains("not sent: README.txt (it is not a build file by its name)"),
        "{out}"
    );
    let req = the_request(&t.pending()[0]);
    let user = req["user"].as_str().unwrap();
    assert!(user.contains("\"path\":\"b.mk\""));
    assert!(!user.contains("\"path\":\"a.mk\""));
    assert!(user.contains("4 more build file(s) were found and not sent"));
    assert!(user.len() < 140 * 1024);
}

// ---------- the reply file ----------

#[test]
fn a_reply_for_another_map_is_replaced_unread() {
    let t = Tmp::new("othermap");
    t.with_recorded();
    t.write(
        REPLY,
        r#"{"schema":"ruharness-project-map-reply","root_hash":"blake3:9999","inputs_hash":"blake3:2222222222222222222222222222222222222222222222222222222222222222","items":{"p9":{"kind":"tool","name":"OLD","purpose":"old","model":"m","provider":"p"},"d1":{"keep":"d1.1","reason":"platform","model":"forged","provider":"p"}}}"#,
    );
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .contains("the reply file there was for another map, so it was replaced, unread"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains("forged"));
    let reply = t.reply();
    assert_eq!(reply["root_hash"], t.map()["root_hash"]);
    let items: Vec<&String> = reply["items"].as_object().unwrap().keys().collect();
    assert_eq!(items, ["d1", "d2"]);
    assert_eq!(reply["items"]["d1"]["keep"], "d1.2");
}

#[test]
fn the_reply_is_merged_when_the_map_did_not_change_the_latest_answer_winning() {
    let t = Tmp::new("merge");
    t.with_recorded();
    let map = t.map();
    t.write(
        REPLY,
        &serde_json::json!({
            "schema": "ruharness-project-map-reply",
            "root_hash": map["root_hash"],
            "inputs_hash": map["inputs_hash"],
            "items": {
                "p1": {"kind": "example", "name": "Demo", "purpose": "Shows it.", "model": "earlier", "provider": "external"},
                "d1": {"keep": "d1.1", "reason": "platform", "model": "earlier", "provider": "external"},
                // Kept answers are checked again: an index the map does not
                // hold, a label holding a terminal code, a keep that is a
                // path are dropped.
                "p77": {"kind": "tool", "name": "x", "purpose": "y", "model": "m", "provider": "external"},
                "p3": {"kind": "tool", "name": "\u{1b}[2Jx", "purpose": "y", "model": "m", "provider": "external"},
                "d3": {"keep": "../../etc/passwd", "reason": "platform", "model": "m", "provider": "external"}
            }
        })
        .to_string(),
    );
    let run = ask(&t, &["--allow-guessed"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let reply = t.reply();
    let items: Vec<&String> = reply["items"].as_object().unwrap().keys().collect();
    assert_eq!(items, ["d1", "d2", "p1"]);
    assert_eq!(reply["items"]["p1"]["model"], "earlier", "kept");
    assert_eq!(reply["items"]["d1"]["keep"], "d1.2", "the latest wins");
    assert_eq!(reply["items"]["d1"]["model"], "claude-sonnet-5");
}

#[test]
fn programs_are_asked_by_id() {
    let t = Tmp::new("programs");
    t.with_recorded();
    let run = ask(
        &t,
        &[
            "--allow-guessed",
            "--programs",
            "t-main",
            "--provider",
            "replay",
        ],
    );
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("asking replay (claude-sonnet-5) about 3 item(s) in 1 call(s): p2, d1, d2"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("p2 t-main (src/main.c): the model's label (claude-sonnet-5): kind tool"),
        "{}",
        run.stdout
    );
    assert_eq!(t.reply()["items"]["p2"]["kind"], "tool");
    // An id the map does not hold, and a library id (a usage error).
    let run = ask(&t, &["--allow-guessed", "--programs", "t-nope"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("the map has no program `t-nope`: name one of t-demo, t-main, t-other"),
        "{}",
        run.stderr
    );
    let run = ask(&t, &["--allow-guessed", "--programs", "l-lib"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    // A fuzz driver has its own main(), but is no main program.
    let mut map = t.map();
    map["programs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "t-driver", "path": "src/main.c", "kind": "driver", "kind_guess": "test"
        }));
    t.set_map(&map);
    let run = ask(&t, &["--allow-guessed", "--programs", "t-driver"]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("`t-driver` is a fuzz driver: only a main program is asked about"),
        "{}",
        run.stderr
    );
    // --build and --programs do not go together.
    let run = ask(&t, &["--build", "--programs", "t-main"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
}

#[test]
fn every_batch_is_written_in_one_run_and_merged_as_it_validates() {
    let t = Tmp::new("batches");
    let mut map = t.map();
    let mut ids = Vec::new();
    for n in 4..=12 {
        let id = format!("t-extra{n}");
        let path = format!("extra/extra{n}.c");
        map["programs"].as_array_mut().unwrap().push(serde_json::json!({
            "id": id, "index": format!("p{n}"), "path": path, "kind": "main", "kind_guess": "tool"
        }));
        map["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "path": path, "bytes": 10, "functions": 1, "includes": []
            }));
        ids.push(id);
    }
    t.set_map(&map);
    let mut all = vec!["t-demo".to_string(), "t-main".into(), "t-other".into()];
    all.extend(ids);
    let programs = all.join(",");
    // 12 programs and 2 sets: two calls.
    let run = ask(&t, &["--allow-guessed", "--programs", &programs]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(
        run.stdout.contains("about 14 item(s) in 2 call(s)"),
        "{}",
        run.stdout
    );
    let pending = t.pending();
    assert_eq!(pending.len(), 2, "every batch's request written");
    let first = pending
        .iter()
        .find(|p| {
            the_request(p)["user"]
                .as_str()
                .unwrap()
                .contains("item=p1 ")
        })
        .unwrap()
        .clone();
    let key = first
        .file_name()
        .unwrap()
        .to_string_lossy()
        .replace(".request.json", "");
    assert!(
        run.stderr.contains(&format!("{key}.response.json")),
        "the first awaiting path: {}",
        run.stderr
    );
    // Answer the first call only: it is merged; the run still awaits.
    let answers: Vec<String> = (1..=10)
        .map(|n| {
            format!(r#"{{"item":"p{n}","kind":"tool","name":"P{n}","purpose":"Program {n}."}}"#)
        })
        .collect();
    t.answer(&first, &format!("[{}]", answers.join(",")));
    let run = ask(&t, &["--allow-guessed", "--programs", &programs]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stderr.contains("awaiting response: "));
    assert_eq!(t.reply()["items"].as_object().unwrap().len(), 10);
}

// ---------- the display filter ----------

#[test]
fn project_strings_are_filtered_on_screen_and_escaped_in_json() {
    let t = Tmp::new("filter");
    let evil = "examples/ev\u{1b}[31mil\u{202e}.c";
    t.write(evil, "int main(void) { return 0; }\n");
    let mut map = t.map();
    map["programs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "t-evil", "index": "p4", "path": evil, "kind": "main", "kind_guess": "example"
        }));
    map["files"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "path": evil, "bytes": 29, "functions": 1, "includes": []
        }));
    t.set_map(&map);
    let run = ask(&t, &["--allow-guessed", "--programs", "t-evil"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let request = t.pending().remove(0);
    assert!(
        the_request(&request)["user"]
            .as_str()
            .unwrap()
            .contains("ev\\u001b[31mil"),
        "sent inside the fence, JSON-escaped"
    );
    t.answer(
        &request,
        &format!(
            "[{},{}]",
            r#"{"item":"p4","kind":"example","name":"Evil","purpose":"Shows a name."}"#,
            SETS_REPLY.trim_start_matches('[').trim_end_matches(']')
        ),
    );
    let run = ask(&t, &["--allow-guessed", "--programs", "t-evil"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains("p4 t-evil (examples/ev?[31mil?.c)"),
        "{}",
        run.stdout
    );
    assert!(!run.stdout.contains('\u{1b}') && !run.stdout.contains('\u{202e}'));
    let run = harness(&[
        "--json",
        "project",
        "ask",
        "--target",
        t.arg(),
        "--allow-guessed",
        "--programs",
        "t-evil",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!run.stdout.contains('\u{1b}') && !run.stdout.contains('\u{202e}'));
    let answer = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|e| e["k"] == "project-answer" && e["item"] == "p4")
        .expect("the answer event");
    assert_eq!(answer["path"], evil, "carried raw, escaped on the wire");
    assert!(run.stdout.contains("\\u001b[31m") && run.stdout.contains("\\u202e"));
}

// ---------- the fixture's recorded responses ----------

/// Rewrites `fixtures/project_ask/traces/` from the fixture: asks the three
/// questions in `external` mode and files the fixed answers below. Run by
/// hand (`cargo test -p harness-cli --test project_ask -- --ignored`) when a
/// prompt changes; the request files are committed beside the responses so
/// a review sees what was asked.
#[test]
#[ignore]
fn regenerate_the_recorded_responses() {
    let out = fixture().join("traces");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let t = Tmp::new("regen");
    let runs: [(&[&str], String); 3] = [
        (&["--allow-guessed"], SETS_REPLY.to_string()),
        (
            &["--allow-guessed", "--programs", "t-main"],
            format!(
                "[{},{}]",
                r#"{"item":"p2","kind":"tool","name":"Main","purpose":"Decodes its input and checks it."}"#,
                SETS_REPLY.trim_start_matches('[').trim_end_matches(']')
            ),
        ),
        (
            &["--build"],
            r#"{"name":"make","from":"make","flags":[{"flag":"-DUSE_FAST","cites":["Makefile:2","CMakeLists.txt:3"]},{"flag":"-Iinclude","cites":["Makefile:2"]},{"flag":"-std=c99","cites":["Makefile:2"]}],"assumptions":["The Makefile's default target is the build that counts."]}"#
                .to_string(),
        ),
    ];
    for (args, reply) in runs {
        let run = ask(&t, args);
        assert_eq!(run.code, 1, "{}", run.stderr);
        let request = t.pending().remove(0);
        let response = t.answer(&request, &reply);
        for p in [request, response] {
            std::fs::copy(&p, out.join(p.file_name().unwrap())).unwrap();
        }
        assert_eq!(ask(&t, args).code, 0);
    }
}
