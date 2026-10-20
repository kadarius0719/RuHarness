//! The project map's open question, for the cockpit's chat
//! (docs/MCP-DESIGN.md §R7, docs/CHAT-PANE-DESIGN.md §3.6):
//!
//! - `harness_map_questions` returns what `harness project ask` would send
//!   a model — the open items' facts, or the build files while the
//!   configuration is not stated — as its system and user text, fenced and
//!   paged like a hand-off's request;
//! - `harness_map_answer` takes the chat's reply to it and records it
//!   through `ask`'s own validators and writers (harness-llm's
//!   `projectask`), under the project lock: refused in full, with the
//!   validator's sentence, when any part fails; else the answers merged
//!   into `migration/map/project-map.reply.json` (or the proposal written to
//!   `migration/map/config.proposed.toml`), recorded as the chat model's,
//!   through the provider `chat`.
//!
//! Nothing here builds, links or runs anything; the person's Accept never
//! reads the reply (the advice is shown beside the choice).

use crate::fence::{short, untrusted};
use crate::reads::{pages_of, REQUEST_PAGE_BYTES};
use harness_llm::projectask::{self, BuildFiles, Item, MapView};
use serde_json::{json, Value};
use std::path::Path;

/// The provider profile a chat's answer is recorded under.
pub const PROVIDER: &str = "chat";
/// The largest answer taken (as a hand-off's answer).
pub const ANSWER_MAX_BYTES: usize = 512 * 1024;
/// The two texts of the question, as their pages fence them.
const PARTS: [&str; 2] = ["map-question-system", "map-question-user"];

/// What the map asks now.
enum Open {
    /// The held choices (and named programs), in `ask`'s batches.
    Questions(Vec<Vec<Item>>),
    /// The build: the files `ask --build` sends.
    Build(BuildFiles),
}

/// The map under `root` and its open question, as `project ask` finds it
/// (the build while the configuration is not stated or nothing is held,
/// else the held choices); `Err` in `ask`'s words.
fn open(root: &Path) -> Result<(MapView, Open), String> {
    if std::fs::symlink_metadata(root.join(projectask::MAP_FILE)).is_err() {
        return Err(
            "no map written yet: the person maps the project first (the Project view's Map the \
             project)"
                .into(),
        );
    }
    let map = projectask::read_map(root).map_err(|e| e.to_string())?;
    if projectask::asks_for_build(&map) {
        let files = projectask::read_build_files(
            root,
            &map.build_evidence.build_files,
            &harness_oracle::projectmap::evidence::is_build_file,
        );
        if files.sent.is_empty() {
            return Err(
                "the map asks for the build, and no build file could be sent (none was found, \
                 or each is over the size caps or reached through a link): the person writes \
                 migration/map/config.toml (the Project view's Write the build configuration)"
                    .into(),
            );
        }
        return Ok((map, Open::Build(files)));
    }
    let items = projectask::open_items(root, &map, &[], false).map_err(|e| e.to_string())?;
    let batches = projectask::batches(&items, PROVIDER, max_tokens()).map_err(|e| e.to_string())?;
    Ok((map, Open::Questions(batches)))
}

fn max_tokens() -> u32 {
    harness_core::config::LlmSection::default().max_tokens
}

/// The batch `batch` (1-based) of `batches`.
fn pick(batches: &[Vec<Item>], batch: u64) -> Result<&[Item], String> {
    usize::try_from(batch)
        .ok()
        .and_then(|b| b.checked_sub(1))
        .and_then(|ix| batches.get(ix))
        .map(Vec::as_slice)
        .ok_or_else(|| {
            format!(
                "batch {batch} of {}: there is no such batch (they count from 1)",
                batches.len()
            )
        })
}

/// `harness_map_questions {batch?, page?}`: the request `project ask` would
/// send for batch `batch` (or for the build), page `page` of its system
/// prompt then its user message, each fenced.
pub fn questions(root: &Path, batch: u64, page: u64) -> Result<Value, String> {
    if page == 0 {
        return Err("`page` counts from 1".into());
    }
    let (_, open) = open(root)?;
    let (kind, request, batches, about) = match &open {
        Open::Build(files) => {
            let request = projectask::build_request(files, PROVIDER, max_tokens())
                .map_err(|e| e.to_string())?;
            let sent: Vec<Value> = files
                .sent
                .iter()
                .map(|f| crate::fence::path("build-file", &f.path))
                .collect();
            (
                "build",
                request,
                1u64,
                json!({"files": sent, "left_out": files.left_out.len()}),
            )
        }
        Open::Questions(batches) => {
            let shown = pick(batches, batch)?;
            let request = projectask::questions_request(shown, PROVIDER, max_tokens())
                .map_err(|e| e.to_string())?;
            let items: Vec<Value> = shown.iter().map(|i| short("item", i.index())).collect();
            (
                "questions",
                request,
                batches.len() as u64,
                json!({"items": items}),
            )
        }
    };
    if kind == "build" && batch != 1 {
        return Err("the build is asked in one batch: batch 1".into());
    }
    let parts = [request.system.as_str(), request.user.as_str()];
    let pages = pages_of(parts, PARTS, REQUEST_PAGE_BYTES);
    let total = pages.len() as u64;
    let Some(shown) = usize::try_from(page - 1).ok().and_then(|ix| pages.get(ix)) else {
        return Err(format!("page {page} of {total}: there is no such page"));
    };
    let fenced = |ix: usize| match shown[ix] {
        Some((a, b)) => untrusted(PARTS[ix], &parts[ix][a..b], usize::MAX),
        None => Value::Null,
    };
    let mut out = json!({
        "kind": kind,
        "batch": batch,
        "batches": batches,
        "page": page,
        "pages": total,
        "system": fenced(0),
        "user": fenced(1),
        "omitted": if page < total { json!({"next_page": page + 1}) } else { Value::Null },
        "answer_with": {"tool": "harness_map_answer", "arguments": {"batch": batch}},
    });
    if let (Some(o), Some(a)) = (out.as_object_mut(), about.as_object()) {
        for (k, v) in a {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok(out)
}

/// `harness_map_answer {model, text, batch?}`: the chat's reply to batch
/// `batch`'s request, validated in full by `ask`'s own contract and
/// recorded as `model`'s, under the project lock. `Err` says why nothing
/// was recorded — the validator's own sentence for a reply that fails it.
pub fn answer(root: &Path, model: &str, batch: u64, text: &str) -> Result<Value, String> {
    if !crate::fence::valid_model(model) {
        return Err(
            "`model` must be your own model id (letters, digits, `.`, `-`, `_`, `:`, `/`, `@`, \
             `[`, `]`; at most 128)"
                .into(),
        );
    }
    if text.trim().is_empty() || text.len() > ANSWER_MAX_BYTES {
        return Err(format!(
            "`text` must be the whole reply: not empty, at most {} KiB",
            ANSWER_MAX_BYTES >> 10
        ));
    }
    // As `project ask`: under the project lock, on the map as it is now.
    let _lock = harness_core::ledger::WriterLock::acquire_project(root, "the chat's map answer")
        .map_err(|e| e.to_string())?;
    let (map, open) = open(root)?;
    match open {
        Open::Build(files) => {
            if batch != 1 {
                return Err("the build is asked in one batch: batch 1".into());
            }
            let check_path = |flag: &str| {
                harness_oracle::projectmap::config::check_flags(root, &[flag.to_string()])
            };
            let p = projectask::validate_proposal(text, &files, &check_path)
                .map_err(|why| refused(&why))?;
            projectask::write_proposal(root, &p, model, PROVIDER).map_err(|e| e.to_string())?;
            Ok(json!({
                "kind": "build",
                "recorded": [short("configuration", &p.name)],
                "file": projectask::PROPOSED_FILE,
                "next": "the proposal is written beside the person's configuration and nothing \
                         else changed: the person copies what they accept into \
                         migration/map/config.toml (Write the build configuration) and maps the \
                         project again",
            }))
        }
        Open::Questions(batches) => {
            let shown = pick(&batches, batch)?;
            let answers = projectask::validate_answers(text, shown).map_err(|why| refused(&why))?;
            let earlier = projectask::record_answers(root, &map, &answers, model, PROVIDER)
                .map_err(|e| e.to_string())?;
            let recorded: Vec<Value> = answers.iter().map(|(i, _)| short("item", i)).collect();
            Ok(json!({
                "kind": "questions",
                "recorded": recorded,
                "file": projectask::REPLY_FILE,
                "replaced": matches!(
                    earlier,
                    projectask::Earlier::OtherMap | projectask::Earlier::Unreadable
                ),
                "next": "recorded as advice: the cockpit shows it beside each choice when the \
                         person Accepts a program; the choice stays theirs, and nothing was built",
            }))
        }
    }
}

/// A reply that fails the contract, in words: nothing was recorded.
fn refused(why: &str) -> String {
    format!(
        "the answer does not follow the contract: {why}; nothing was recorded — read the \
         request's system part again and answer it whole"
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A mapped project with one held set (d1: two definers of `decode`)
    /// under a stated configuration; `guessed` makes it a guess.
    pub(crate) fn project(tag: &str, guessed: bool) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("mcp-map-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("migration/map")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.c"), "int decode(void) { return 1; }\n").unwrap();
        std::fs::write(root.join("src/b.c"), "int decode(void) { return 2; }\n").unwrap();
        std::fs::write(
            root.join("src/main.c"),
            "int decode(void);\nint main(void) { return decode(); }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("Makefile"),
            "CFLAGS = -Isrc -DFAST\nall:\n\tcc src/*.c\n",
        )
        .unwrap();
        let file = |path: &str| {
            json!({"path": path, "bytes": 30, "functions": 1, "includes": [],
                   "defines": ["decode"], "main": path == "src/main.c"})
        };
        let map = json!({
            "schema": "ruharness-project-map", "schema_version": 1,
            "root_hash": "blake3:aa", "inputs_hash": "blake3:bb",
            "configuration": {"name": "make", "from": "make",
                              "source": if guessed { "guessed" } else { "stated" }},
            "files": [file("src/a.c"), file("src/b.c"), file("src/main.c")],
            "programs": [{"id": "t-main", "index": "p1", "path": "src/main.c", "kind": "main"}],
            "closures": [{"program": "t-main",
                "duplicates": [{"set": "d1", "symbols": ["decode"],
                    "definers": [{"index": "d1.1", "path": "src/a.c"},
                                 {"index": "d1.2", "path": "src/b.c"}],
                    "links": ["d1.1", "d1.2"]}],
                "questions": ["d1"], "linked": "ok"}],
            "build_evidence": {"build_files": ["Makefile"]},
        });
        std::fs::write(
            root.join(projectask::MAP_FILE),
            serde_json::to_string_pretty(&map).unwrap(),
        )
        .unwrap();
        // Made here, so adopted here (as the person's `--adopt` would).
        harness_core::adopt::testing::adopt(&root);
        root.canonicalize().unwrap()
    }

    fn text(v: &Value) -> String {
        v["text"].as_str().unwrap_or_default().to_string()
    }

    /// The open question is what `project ask` sends, fenced: the held set
    /// with its definers' facts and slices, the reply contract in the system
    /// part.
    #[test]
    fn the_open_questions_are_what_ask_would_send() {
        let root = project("questions", false);
        let v = questions(&root, 1, 1).unwrap();
        assert_eq!(v["kind"], "questions");
        assert_eq!(v["batches"], 1);
        assert_eq!(v["items"][0]["text"], "d1");
        assert_eq!(v["system"]["untrusted"], "map-question-system");
        // The very request `ask` builds for this batch.
        let map = projectask::read_map(&root).unwrap();
        let items = projectask::open_items(&root, &map, &[], false).unwrap();
        let want = projectask::questions_request(&items, PROVIDER, max_tokens()).unwrap();
        assert_eq!(text(&v["system"]), want.system);
        assert_eq!(text(&v["user"]), want.user);
        assert!(text(&v["user"]).contains("src/b.c"), "{v}");
        assert!(v["omitted"].is_null());
        assert_eq!(v["answer_with"]["tool"], "harness_map_answer");
        // No such batch or page.
        assert!(questions(&root, 2, 1)
            .unwrap_err()
            .contains("no such batch"));
        assert!(questions(&root, 1, 9).unwrap_err().contains("no such page"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A good answer is recorded through `ask`'s writer, as the chat
    /// model's; a forged one — an index not asked, a keep that is no
    /// definer — is refused in full with the validator's sentence, and the
    /// file is left as it was.
    #[test]
    fn a_good_answer_is_recorded_and_a_forged_one_refused_whole() {
        let root = project("answer", false);
        let reply = root.join(projectask::REPLY_FILE);
        let forged = [
            (
                r#"[{"item": "d1", "keep": "d1.2", "reason": "platform"},
                    {"item": "d7", "keep": "d7.1", "reason": "platform"}]"#,
                "answer #2 names the item `d7`, which was not asked",
            ),
            (
                r#"[{"item": "d1", "keep": "../../etc", "reason": "platform"}]"#,
                "item `d1`: keep",
            ),
            (r#"{"item": "d1"}"#, "the reply is not a JSON array"),
        ];
        for (text, sentence) in forged {
            let why = answer(&root, "claude-opus-5-5", 1, text).unwrap_err();
            assert!(why.contains(sentence), "{why}");
            assert!(why.contains("nothing was recorded"), "{why}");
            assert!(
                !reply.exists(),
                "a refused answer wrote {}",
                reply.display()
            );
        }
        // A why over its limit: refused with the limit named.
        let long = format!(
            r#"[{{"item": "d1", "keep": "d1.2", "reason": "platform", "why": "{}"}}]"#,
            "x".repeat(301)
        );
        let why = answer(&root, "claude-opus-5-5", 1, &long).unwrap_err();
        assert!(why.contains("`why` is over 300 characters"), "{why}");
        assert!(!reply.exists());
        assert!(answer(&root, "not a model!", 1, "[]")
            .unwrap_err()
            .contains("`model`"));
        let v = answer(
            &root,
            "claude-opus-5-5",
            1,
            r#"[{"item": "d1", "keep": "d1.2", "reason": "alternative-implementation",
                 "why": "decode.c is the library's own decoder;\nmini.c is a copy for other projects."}]"#,
        )
        .unwrap();
        assert_eq!(v["kind"], "questions");
        assert_eq!(v["recorded"][0]["text"], "d1");
        let file: Value = serde_json::from_slice(&std::fs::read(&reply).unwrap()).unwrap();
        assert_eq!(file["root_hash"], "blake3:aa");
        assert_eq!(file["items"]["d1"]["keep"], "d1.2");
        assert_eq!(file["items"]["d1"]["model"], "claude-opus-5-5");
        assert_eq!(file["items"]["d1"]["provider"], "chat");
        // The why passes through to the reply file, folded to one line.
        assert_eq!(
            file["items"]["d1"]["why"],
            "decode.c is the library's own decoder; mini.c is a copy for other projects."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// While the configuration is a guess the open question is the build:
    /// the build files `ask --build` sends; a proposal is checked by its
    /// validator (a path leaving the project refused) and written beside the
    /// configuration.
    #[test]
    fn a_guess_asks_for_the_build_and_records_a_proposal() {
        let root = project("build", true);
        let v = questions(&root, 1, 1).unwrap();
        assert_eq!(v["kind"], "build");
        assert_eq!(v["files"][0]["text"], "Makefile");
        assert!(text(&v["user"]).contains("CFLAGS"), "{v}");
        let bad = r#"{"name": "make", "from": "make", "assumptions": [],
                      "flags": [{"flag": "-I/etc", "cites": ["Makefile:1"]}]}"#;
        let why = answer(&root, "claude-opus-5-5", 1, bad).unwrap_err();
        assert!(
            why.contains("the answer does not follow the contract"),
            "{why}"
        );
        assert!(!root.join(projectask::PROPOSED_FILE).exists());
        let good = r#"{"name": "make", "from": "make", "assumptions": ["the default target"],
                       "flags": [{"flag": "-Isrc", "cites": ["Makefile:1"]},
                                 {"flag": "-DFAST", "cites": ["Makefile:1"]}]}"#;
        let v = answer(&root, "claude-opus-5-5", 1, good).unwrap();
        assert_eq!(v["kind"], "build");
        let toml = std::fs::read_to_string(root.join(projectask::PROPOSED_FILE)).unwrap();
        assert!(toml.contains("\"-Isrc\""), "{toml}");
        assert!(toml.contains("claude-opus-5-5"), "{toml}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// No map: said in the person's words.
    #[test]
    fn no_map_is_said() {
        let root = std::env::temp_dir().join(format!("mcp-map-none-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(questions(&root, 1, 1)
            .unwrap_err()
            .contains("no map written yet"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
