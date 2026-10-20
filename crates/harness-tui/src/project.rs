//! The cockpit's Project view (docs/PROJECT-MAP-DESIGN.md §3.7 "The cockpit
//! and harness-mcp"; pass 3 of docs/reviews/2026-10-18-cockpit-usability-
//! critique.md): opened on a project root with no `harness.toml`, the
//! cockpit's left pane lists what the map found — before any map one row,
//! **Map the project**; after a map the programs, the libraries and the
//! accepted tools — and its right pane the project in words, with its Next
//! step. Enter on a row opens the menu of the project's acts — **Map the
//! project**, **Ask**, **Accept this program** — each the cockpit's own
//! dialog, which says what it runs, how long it takes and what it writes,
//! and runs the `harness project …` command with the same words as on the
//! command line. Only the person's armed yes runs anything; harness-mcp
//! never runs these acts.
//!
//! This module reads the map and the model's reply and builds the words and
//! the commands; the screen is the cockpit's (`app::project_view` and the
//! view's project pane).
//!
//! The acts follow the open question: while the configuration is a guess,
//! Ask asks for the build (`--build`) and Accept says why it waits. Accept's
//! picker shows a reply's advice beside the definers, labelled as the
//! model's, none preselected, and skips a set the choices made so far do not
//! reach.
//!
//! The map file and the reply file live in the project: every id, set and
//! index is shape-checked before it is shown or put in a command, and every
//! string shown goes through [`safe_line`].

use harness_core::config;
use harness_core::text::safe_line;
use std::collections::BTreeMap;
use std::path::Path;

/// The map file the view reads, relative to the root.
pub const MAP_FILE: &str = "migration/map/project-map.json";
/// The reply file `harness project ask` writes, relative to the root.
pub const REPLY_FILE: &str = "migration/map/project-map.reply.json";
/// The largest map file the view reads.
const MAX_MAP_BYTES: u64 = 64 << 20;
/// The largest reply file the view reads.
const MAX_REPLY_BYTES: u64 = 16 << 20;
/// The held choices one `ask` call carries (harness-llm's `MAX_BATCH`).
const ASK_BATCH: usize = 10;
/// The reasons a set's advice may give (harness-llm's `SET_REASONS`).
const SET_REASONS: &[&str] = &["platform", "alternative-implementation", "cannot-tell"];
/// The kinds a model may give a program (harness-llm's `PROGRAM_KINDS`).
const PROGRAM_KINDS: &[&str] = &["tool", "test", "example", "benchmark", "other"];
/// The map's guesses of a program's kind, from its folder.
const KIND_GUESSES: &[&str] = &["tool", "test", "example", "benchmark"];
/// A model's label name and purpose, at most (harness-llm's limits).
const LABEL_NAME_MAX: usize = 40;
const LABEL_LINE_MAX: usize = 200;
/// A set's why, at most (harness-llm's `WHY_MAX_CHARS`).
const WHY_MAX: usize = 300;
/// The longest run name the cockpit passes to `--run-name`.
pub const MAX_RUN_NAME: usize = 64;

/// One of the project's acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// `harness project map`.
    Map,
    /// `harness project ask`.
    Ask,
    /// `harness project accept <id>`.
    Accept,
}

/// What linking (or compiling) found for a program or a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Its files link into one program.
    Linked,
    /// A duplicate set waits for the person's choice.
    Held,
    /// A file of it did not compile under the configuration.
    DidNotCompile,
    /// It was linked and a symbol is missing or defined twice.
    DidNotLink,
    /// Its file list is incomplete for another reason.
    Incomplete,
    /// A fuzzer (linked only with the project's driver).
    Fuzzer,
    /// A fuzzer driver (never a tool).
    Driver,
    /// A library whose files compile.
    Compiles,
}

impl State {
    /// The state in words, as the rows and the View say it.
    pub fn words(self) -> &'static str {
        match self {
            State::Linked => "links",
            State::Held => "needs a decision",
            State::DidNotCompile => "did not compile",
            State::DidNotLink => "did not link",
            State::Incomplete => "incomplete",
            State::Fuzzer => "fuzzer, not a tool",
            State::Driver => "fuzzer driver, not a tool",
            State::Compiles => "compiles",
        }
    }

    /// Its mark in the tree.
    pub fn glyph(self) -> &'static str {
        match self {
            State::Linked | State::Compiles => "✓",
            State::Held => "⚠",
            State::DidNotCompile | State::DidNotLink => "✗",
            State::Incomplete | State::Fuzzer | State::Driver => "·",
        }
    }
}

/// A held duplicate set of one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSet {
    /// `d1…`.
    pub set: String,
    /// `(index, path)` of each definer.
    pub definers: Vec<(String, String)>,
    /// The definer indexes it is reached under, when not every choice
    /// reaches it (empty: reached whatever is kept).
    pub under: Vec<String>,
}

/// A program or library the map offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    /// Its id.
    pub id: String,
    /// Its file (a library: its first file).
    pub path: String,
    /// `main`, `fuzz`, `driver` or `library`.
    pub kind: String,
    /// The map's guess of what it is (`tool`, `test`, `example`,
    /// `benchmark`); empty for a library.
    pub kind_guess: String,
    /// What linking or compiling found.
    pub state: State,
    /// Its held sets (a `main` program's), in index order.
    pub held: Vec<HeldSet>,
    /// Its index in the map (`p1…`, a `main` program's; empty otherwise):
    /// a model's label is keyed by it.
    pub index: String,
    /// What failed, in one line, when it did not compile or link: the
    /// map's first error ("src/a.c:3: a syntax error"); empty otherwise.
    pub problem: String,
}

impl Offered {
    /// What the map knows it is, in words: "a tool, by its folder" (the
    /// map's guess from where its file sits), "a library", "a fuzzer".
    pub fn what(&self) -> String {
        match self.kind.as_str() {
            "library" => "a library".into(),
            "fuzz" => "a fuzzer".into(),
            "driver" => "a fuzzer driver".into(),
            _ if self.kind_guess.is_empty() => "a program".into(),
            _ => format!(
                "{} {}, by its folder",
                article(&self.kind_guess),
                self.kind_guess
            ),
        }
    }

    /// A `main` program or a library: what `accept` takes.
    pub fn acceptable(&self) -> bool {
        self.kind == "main" || self.kind == "library"
    }

    /// "program" or "library", for the words.
    pub fn noun(&self) -> &'static str {
        if self.kind == "library" {
            "library"
        } else {
            "program"
        }
    }

    /// The next held set to ask about, given the definers kept so far
    /// (`kept`, indexes) and the sets already answered (`asked`): a set
    /// reached only under a choice the person did not make is skipped.
    pub fn next_held(&self, kept: &[String], asked: &[String]) -> Option<&HeldSet> {
        self.held.iter().find(|h| {
            !asked.contains(&h.set)
                && (h.under.is_empty() || h.under.iter().any(|u| kept.contains(u)))
        })
    }
}

/// "a" or "an" before `word`.
fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// A model's label of one program, from the reply file (`harness project
/// ask` asks what each program is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// One of `tool`, `test`, `example`, `benchmark`, `other`.
    pub kind: String,
    /// Its name, one short line.
    pub name: String,
    /// What it does, one line.
    pub purpose: String,
    /// The model that gave it.
    pub model: String,
}

impl Label {
    /// The label in words, as the model's: "The model calls it “LZG
    /// compressor” (a tool)".
    pub fn words(&self) -> String {
        format!(
            "The model calls it “{}” ({} {})",
            safe_line(&self.name),
            article(&self.kind),
            self.kind
        )
    }
}

/// What the view reads of the map file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapView {
    /// Its programs and libraries.
    pub offered: Vec<Offered>,
    /// Files holding a `main` that did not compile (no id: listed only).
    pub not_compiled: Vec<String>,
    /// The configuration's name.
    pub configuration: String,
    /// It came with the project and is only proposed.
    pub proposed: bool,
    /// It is a guess.
    pub guessed: bool,
    /// The map's `root_hash` and `inputs_hash` (a reply is bound to them).
    pub digests: (String, String),
    /// The example `config.toml` the map suggests for its build files (the
    /// one `project map` prints): what "Write the build configuration"
    /// starts from.
    pub example: String,
}

impl MapView {
    /// Any program holds a set.
    pub fn held(&self) -> bool {
        self.offered.iter().any(|o| !o.held.is_empty())
    }

    /// The held sets, each once.
    pub fn held_sets(&self) -> usize {
        let mut sets: Vec<&str> = self
            .offered
            .iter()
            .flat_map(|o| o.held.iter().map(|h| h.set.as_str()))
            .collect();
        sets.sort_unstable();
        sets.dedup();
        sets.len()
    }

    /// The configuration is not the person's yet (a guess, or proposed):
    /// nothing can be accepted, and the open question is the build.
    pub fn unstated(&self) -> bool {
        self.guessed || self.proposed
    }

    /// The configuration's state in plain words.
    pub fn configuration_words(&self) -> String {
        if self.proposed {
            format!(
                "proposed — it came with the project ({}), not yours yet",
                safe_line(&self.configuration)
            )
        } else if self.guessed {
            "a guess — no migration/map/config.toml states the build yet".into()
        } else {
            format!("stated ({})", safe_line(&self.configuration))
        }
    }

    /// A program or library of the map, by id.
    pub fn get(&self, id: &str) -> Option<&Offered> {
        self.offered.iter().find(|o| o.id == id)
    }
}

/// A decimal number without a leading zero, at most 9 digits.
fn is_number(s: &str) -> bool {
    (1..=9).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
}

/// `d<n>`.
fn is_set(s: &str) -> bool {
    s.strip_prefix('d').is_some_and(is_number)
}

/// `d<n>.<m>`, of any set.
fn is_definer(s: &str) -> bool {
    s.split_once('.')
        .is_some_and(|(set, n)| is_set(set) && is_number(n))
}

/// A `t-` or `l-` tool id.
fn is_id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix) && config::is_tool_id(s)
}

/// A run name the cockpit passes to `--run-name`: letters, digits, `.`,
/// `_` and `-`, not starting with `-` or `.`, at most [`MAX_RUN_NAME`].
pub fn is_run_name(s: &str) -> bool {
    (1..=MAX_RUN_NAME).contains(&s.len())
        && !s.starts_with(['-', '.'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The map under `root`: `Ok(None)` when there is none, `Err` (one
/// sentence) when it is not one this view reads — too large, not JSON, or
/// an id, set or index outside its shape.
pub fn read_map(root: &Path) -> Result<Option<MapView>, String> {
    let path = root.join(MAP_FILE);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    let bad = |why: &str| format!("the map ({MAP_FILE}) {why}: map the project again");
    if !meta.file_type().is_file() || meta.len() > MAX_MAP_BYTES {
        return Err(bad("is a link or too large"));
    }
    let bytes = std::fs::read(&path).map_err(|_| bad("could not be read"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| bad("could not be read as JSON"))?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let empty = Vec::new();
    let list = |v: &serde_json::Value| v.as_array().unwrap_or(&empty).clone();
    let shapes = || bad("holds an id or an index that is not one the map writes");
    // The files that did not compile under the configuration, each with
    // what the map says of it.
    let failures: Vec<(String, serde_json::Value)> = list(&v["files"])
        .iter()
        .filter(|f| !f["compiled"].is_null() && f["compiled"].as_str() != Some("ok"))
        .map(|f| (s(&f["path"]), f["compiled"].clone()))
        .collect();
    let failed: Vec<String> = failures.iter().map(|(p, _)| p.clone()).collect();
    // The first of `files` that did not compile, in words.
    let compile_problem = |files: &[String]| {
        files
            .iter()
            .find_map(|f| failures.iter().find(|(p, _)| p == f))
            .map(|(p, c)| compile_words(p, c))
            .unwrap_or_default()
    };
    let mut offered = Vec::new();
    for p in list(&v["programs"]) {
        let id = s(&p["id"]);
        if !is_id(&id, "t-") {
            return Err(shapes());
        }
        let kind = s(&p["kind"]);
        let closure = list(&v["closures"])
            .into_iter()
            .find(|c| c["program"].as_str() == Some(id.as_str()));
        let mut held = Vec::new();
        let mut problem = String::new();
        let mut state = match kind.as_str() {
            "fuzz" => State::Fuzzer,
            "driver" => State::Driver,
            _ => State::Incomplete,
        };
        if let Some(c) = &closure {
            let questions: Vec<String> = list(&c["questions"]).iter().map(s).collect();
            for d in list(&c["duplicates"]) {
                let set = s(&d["set"]);
                if !is_set(&set) {
                    return Err(shapes());
                }
                if !questions.contains(&set) {
                    continue;
                }
                let mut definers = Vec::new();
                for x in list(&d["definers"]) {
                    let index = s(&x["index"]);
                    if index
                        .strip_prefix(&set)
                        .and_then(|r| r.strip_prefix('.'))
                        .is_none()
                        || !is_definer(&index)
                    {
                        return Err(shapes());
                    }
                    definers.push((index, s(&x["path"])));
                }
                let under: Vec<String> = list(&d["under"]).iter().map(s).collect();
                if !under.iter().all(|u| is_definer(u)) {
                    return Err(shapes());
                }
                held.push(HeldSet {
                    set,
                    definers,
                    under,
                });
            }
            if kind == "main" {
                let files: Vec<String> = list(&c["files"]).iter().map(s).collect();
                problem = compile_problem(&files);
                if problem.is_empty() && c["linked"].is_object() {
                    problem = link_words(&c["linked"]);
                }
                state = if files.iter().any(|f| failed.contains(f)) {
                    State::DidNotCompile
                } else if !held.is_empty() {
                    State::Held
                } else if c["linked"].as_str() == Some("ok") {
                    State::Linked
                } else if c["linked"].is_object() {
                    State::DidNotLink
                } else {
                    State::Incomplete
                };
            }
        }
        // A `main` program's index (`p1…`); any other shape is dropped:
        // it only keys a model's label.
        let index = s(&p["index"]);
        let index = if index.strip_prefix('p').is_some_and(is_number) {
            index
        } else {
            String::new()
        };
        // The map's guess is one of its four words; anything else is not
        // shown.
        let kind_guess = s(&p["kind_guess"]);
        let kind_guess = if KIND_GUESSES.contains(&kind_guess.as_str()) {
            kind_guess
        } else {
            String::new()
        };
        offered.push(Offered {
            id,
            path: s(&p["path"]),
            kind,
            kind_guess,
            state,
            held,
            index,
            problem,
        });
    }
    for l in list(&v["libraries"]) {
        let id = s(&l["id"]);
        if !is_id(&id, "l-") {
            return Err(shapes());
        }
        let files: Vec<String> = list(&l["files"]).iter().map(s).collect();
        offered.push(Offered {
            id,
            path: files.first().cloned().unwrap_or_default(),
            kind: "library".into(),
            kind_guess: String::new(),
            state: if files.iter().any(|f| failed.contains(f)) {
                State::DidNotCompile
            } else {
                State::Compiles
            },
            held: Vec::new(),
            index: String::new(),
            problem: compile_problem(&files),
        });
    }
    let c = &v["configuration"];
    let build_files: Vec<String> = list(&v["build_evidence"]["build_files"])
        .iter()
        .map(s)
        .collect();
    Ok(Some(MapView {
        offered,
        not_compiled: list(&v["programs_not_compiled"]).iter().map(s).collect(),
        configuration: s(&c["name"]),
        proposed: c["proposed"].as_bool().unwrap_or(false),
        guessed: c["source"].as_str() == Some("guessed"),
        digests: (s(&v["root_hash"]), s(&v["inputs_hash"])),
        example: harness_oracle::projectmap::config::example(
            harness_oracle::projectmap::config::example_from(&build_files),
        ),
    }))
}

/// A file's compile failure in words, from the map's record: where, then
/// what ("src/a.c:3: a syntax error").
fn compile_words(path: &str, compiled: &serde_json::Value) -> String {
    let at = compiled["at"].as_str().unwrap_or(path);
    let what = match compiled["reason"].as_str() {
        Some("missing-header") => match compiled["header"].as_str() {
            Some(h) => format!("the header {h} is not found"),
            None => "a header is not found".into(),
        },
        Some("syntax") => "a syntax error".into(),
        _ => match compiled["detail"].as_str() {
            Some(d) => format!("it does not compile ({d})"),
            None => "it does not compile".into(),
        },
    };
    format!("{at}: {what}")
}

/// A link failure in words, from the map's record: what nothing provides,
/// else what is defined twice, else what did not compile for the link.
fn link_words(linked: &serde_json::Value) -> String {
    let names = |key: &str| -> Vec<String> {
        linked[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let some = |v: &[String]| {
        let shown = v.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        match v.len() {
            0..=3 => shown,
            n => format!("{shown} and {} more", n - 3),
        }
    };
    let (missing, doubled, not_compiled) =
        (names("missing"), names("doubled"), names("not_compiled"));
    if !missing.is_empty() {
        format!("nothing it links defines {}", some(&missing))
    } else if !doubled.is_empty() {
        format!("defined twice: {}", some(&doubled))
    } else if !not_compiled.is_empty() {
        format!("did not compile for the link: {}", some(&not_compiled))
    } else {
        "some symbols could not be checked".into()
    }
}

/// A model's advice on one held set, from the reply file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// The advised definer's index, or `undecided`.
    pub keep: String,
    /// One of the reasons.
    pub reason: String,
    /// Why, in the model's plain words (none in an older reply).
    pub why: Option<String>,
    /// The model that gave it.
    pub model: String,
}

/// A reason tag in words: shown only where the model gave no why (an
/// older reply).
pub fn reason_words(reason: &str) -> &'static str {
    match reason {
        "platform" => "they are the same code for different platforms",
        "alternative-implementation" => "they are two implementations of the same thing",
        _ => "the model could not tell them apart",
    }
}

impl Advice {
    /// Why, in the model's words, labelled as the model's: "Why
    /// (claude-x): …"; the reason in words when the reply gave no why.
    pub fn why_line(&self) -> String {
        let why = match &self.why {
            Some(w) => safe_line(w).trim_end_matches('.').to_string(),
            None => reason_words(&self.reason).to_string(),
        };
        let model = safe_line(&self.model);
        if self.keep == "undecided" {
            format!("The model ({model}) could not decide: {why}.")
        } else {
            format!("Why ({model}): {why}.")
        }
    }

    /// The advice in words, labelled as the model's.
    pub fn words(&self) -> String {
        if self.keep == "undecided" {
            format!(
                "The model's advice ({}): it could not decide (reason {}).",
                safe_line(&self.model),
                self.reason
            )
        } else {
            format!(
                "The model's advice ({}): keep {}, reason {} — advice only, the choice is yours.",
                safe_line(&self.model),
                safe_line(&self.keep),
                self.reason
            )
        }
    }
}

/// The advice of the reply file under `root`, by set, when it is bound to
/// `map`'s digests; each answer checked as `ask` checks a fresh one (its
/// keep one of the set's definers or `undecided`, its reason one of the
/// list, its model one showable line), and dropped when it fails.
pub fn read_advice(root: &Path, map: &MapView) -> BTreeMap<String, Advice> {
    let mut out = BTreeMap::new();
    let Some(v) = read_reply(root, map) else {
        return out;
    };
    let Some(items) = v["items"].as_object() else {
        return out;
    };
    for o in &map.offered {
        for h in &o.held {
            let Some(item) = items.get(&h.set) else {
                continue;
            };
            let (Some(keep), Some(reason), Some(model)) = (
                item["keep"].as_str(),
                item["reason"].as_str(),
                item["model"].as_str(),
            ) else {
                continue;
            };
            let known = keep == "undecided" || h.definers.iter().any(|(i, _)| i == keep);
            // A why that is not one showable line within its limit drops
            // the answer, as `ask` would refuse it.
            let why = item["why"].as_str();
            let why_ok = item["why"].is_null()
                || why.is_some_and(|w| {
                    !w.trim().is_empty()
                        && w.chars().count() <= WHY_MAX
                        && !w.chars().any(harness_core::text::unsafe_to_show)
                });
            let showable = !model.is_empty()
                && model.chars().count() <= 200
                && !model.chars().any(harness_core::text::unsafe_to_show);
            if known && SET_REASONS.contains(&reason) && showable && why_ok {
                out.insert(
                    h.set.clone(),
                    Advice {
                        keep: keep.to_string(),
                        reason: reason.to_string(),
                        why: why.map(String::from),
                        model: model.to_string(),
                    },
                );
            }
        }
    }
    out
}

/// The reply file under `root` as JSON, when it is a plain file within its
/// size and bound to `map`'s digests.
fn read_reply(root: &Path, map: &MapView) -> Option<serde_json::Value> {
    let path = root.join(REPLY_FILE);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_REPLY_BYTES {
        return None;
    }
    let v = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())?;
    (v["root_hash"].as_str() == Some(map.digests.0.as_str())
        && v["inputs_hash"].as_str() == Some(map.digests.1.as_str()))
    .then_some(v)
}

/// The model's labels of the map's programs, by program id, from the reply
/// file under `root` when it is bound to `map`'s digests; each checked as
/// `ask` checks a fresh one (its kind one of the list, its name and purpose
/// one showable line within their limits), and dropped when it fails.
pub fn read_labels(root: &Path, map: &MapView) -> BTreeMap<String, Label> {
    let mut out = BTreeMap::new();
    let Some(v) = read_reply(root, map) else {
        return out;
    };
    let Some(items) = v["items"].as_object() else {
        return out;
    };
    let showable = |s: &str, max: usize| {
        !s.trim().is_empty()
            && s.chars().count() <= max
            && !s.chars().any(harness_core::text::unsafe_to_show)
    };
    for o in map.offered.iter().filter(|o| !o.index.is_empty()) {
        let Some(item) = items.get(&o.index) else {
            continue;
        };
        let (Some(kind), Some(name), Some(purpose), Some(model)) = (
            item["kind"].as_str(),
            item["name"].as_str(),
            item["purpose"].as_str(),
            item["model"].as_str(),
        ) else {
            continue;
        };
        if PROGRAM_KINDS.contains(&kind)
            && showable(name, LABEL_NAME_MAX)
            && showable(purpose, LABEL_LINE_MAX)
            && showable(model, 200)
        {
            out.insert(
                o.id.clone(),
                Label {
                    kind: kind.to_string(),
                    name: name.to_string(),
                    purpose: purpose.to_string(),
                    model: model.to_string(),
                },
            );
        }
    }
    out
}

/// The folder is a project rather than a target: no root `harness.toml`
/// (its tools, if any, live under `migration/tools/`).
pub fn is_project(root: &Path) -> bool {
    std::fs::symlink_metadata(root.join(config::CONFIG_FILE)).is_err()
}

/// The cockpit opens on the Project view: no `--tool`, no root
/// `harness.toml`, and not a project whose only tool has no map beside it
/// (that tool opens, as the command line finds it) — no tool yet, a map to
/// act on, or several tools to choose from.
pub fn applies(root: &Path, tool: Option<&str>) -> bool {
    tool.is_none()
        && is_project(root)
        && (config::mapped_tools(root).len() != 1
            || std::fs::symlink_metadata(root.join(MAP_FILE)).is_ok())
}

/// A tool as the Project view names it: its id and, from the map, its
/// program's file (or a library's first file); the id alone when the map
/// does not name it.
pub fn tool_label(map: Option<&MapView>, id: &str) -> String {
    match map.and_then(|m| m.get(id)) {
        Some(o) if o.kind == "library" => {
            format!("{} — library, {}", safe_line(id), safe_line(&o.path))
        }
        Some(o) => format!("{} — {}", safe_line(id), safe_line(&o.path)),
        None => safe_line(id),
    }
}

/// Ask asks for the build: while the configuration is a guess (or
/// proposed), or when nothing is held.
pub fn asks_for_build(map: Option<&MapView>) -> bool {
    map.is_none_or(|m| m.unstated() || !m.held())
}

/// The command an act runs (the words after `harness`), as argv: `id`,
/// `keeps` and `run_name` are Accept's.
pub fn argv(
    act: Act,
    root: &Path,
    map: Option<&MapView>,
    id: &str,
    keeps: &[String],
    run_name: Option<&str>,
) -> Vec<String> {
    let target = root.to_string_lossy().into_owned();
    let mut v: Vec<String> = vec!["project".into()];
    match act {
        Act::Map => v.extend(["map".into(), "--target".into(), target]),
        Act::Ask => {
            v.extend(["ask".into(), "--target".into(), target]);
            // The open question: the build while the configuration is a
            // guess (or nothing is held), else the held choices.
            if asks_for_build(map) {
                v.push("--build".into());
            }
        }
        Act::Accept => {
            v.extend(["accept".into(), id.to_string(), "--target".into(), target]);
            for k in keeps {
                v.extend(["--keep".into(), k.clone()]);
            }
            if let Some(name) = run_name {
                v.extend(["--run-name".into(), name.to_string()]);
            }
        }
    }
    v
}

/// The dialog's words for an act: what it does, how long it takes and what
/// it writes — one entry per paragraph. The command itself is said once,
/// whole, at the dialog's end (its "Command:" line).
pub fn dialog(act: Act, map: Option<&MapView>, argv: &[String]) -> Vec<String> {
    let build = argv.iter().any(|a| a == "--build");
    let (what, long, writes): (String, String, &str) = match act {
        Act::Map => (
            "Map the project: find its programs and libraries, the files each one needs, what \
             they share, and whether each program links (its code is compiled and linked in the \
             sandbox, never run)."
                .to_string(),
            "a few seconds for a small project, minutes for a large one (at most 30 minutes)"
                .into(),
            "migration/map/project-map.json (and migration/.gitignore the first time); the \
             project's own files are not changed",
        ),
        Act::Ask if build => (
            "Ask a model to read the project's build files and propose a configuration (its \
             name, from and flags); you state it yourself in migration/map/config.toml."
                .to_string(),
            "one model call through the external hand-off: the command stops and waits for the \
             answer file, and prints the command that resumes"
                .into(),
            "migration/map/config.proposed.toml and its trace under migration/map/traces/; \
             builds nothing",
        ),
        Act::Ask => {
            let calls = map.map_or(1, |m| m.held_sets().div_ceil(ASK_BATCH).max(1));
            (
                "Ask a model which file to keep in each held choice: its answer is advice, shown \
                 beside the choice when you Accept a program; you still decide."
                    .to_string(),
                format!(
                    "{} through the external hand-off (one for each {ASK_BATCH} held choices): \
                     the command stops and waits for the answer files, and prints the command \
                     that resumes",
                    if calls == 1 {
                        "one model call".to_string()
                    } else {
                        format!("{calls} model calls")
                    }
                ),
                "migration/map/project-map.reply.json and its traces under \
                 migration/map/traces/; builds nothing",
            )
        }
        Act::Accept => (
            "Accept a program: check the map still matches the project, apply your picks, link \
             the program once more and write it as a tool you can scan, plan and migrate."
                .to_string(),
            "about as long as the map (it maps and link-checks the project again to check the map \
             still says what it finds), then one more link"
                .into(),
            "migration/tools/<id>/harness.toml only (an id accepted before keeps its ledger and \
             what you added to that file)",
        ),
    };
    vec![
        what,
        format!("It takes: {long}."),
        format!("It writes: {writes}."),
    ]
}

/// Why Accept waits, before any picker, while the configuration is not
/// stated.
pub const ACCEPT_NEEDS_STATED: &str = "Accept needs a stated configuration, and this map's is a \
     guess (or came with the project): choose Write the build configuration (or Ask for a \
     proposal — ask in chat), then Map the project again";

/// The picker's question for one held set.
pub fn keep_question(id: &str, set: &str) -> String {
    format!(
        "{} holds the choice {}: two files define the same function, so keep which one?",
        safe_line(id),
        safe_line(set)
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn project(tag: &str) -> crate::testutil::TmpDir {
        let tmp = crate::testutil::TmpDir::new(&format!("project-map-{tag}"));
        std::fs::write(tmp.0.join("a.c"), "int main(void) { return 0; }\n").unwrap();
        tmp
    }

    /// A map whose configuration is `source` (`proposed` when shipped):
    /// t-unlzg holds d1 (lzg_decode), and d2 only when d1.1 is kept; t-lzg
    /// links; src/bad.c did not compile.
    pub(crate) fn map_text(source: &str, proposed: bool) -> String {
        format!(
            r#"{{
      "root_hash": "blake3:aa", "inputs_hash": "blake3:bb",
      "configuration": {{"name": "make", "from": "make", "source": "{source}", "proposed": {proposed}}},
      "files": [{{"path": "src/bad.c", "compiled": {{"reason": "syntax"}}}},
                {{"path": "src/decode.c", "compiled": "ok"}}],
      "programs": [
        {{"id": "t-lzg", "index": "p1", "path": "tools/lzg.c", "kind": "main", "kind_guess": "tool"}},
        {{"id": "t-unlzg", "index": "p2", "path": "tools/unlzg.c", "kind": "main", "kind_guess": "tool"}},
        {{"id": "t-fuzz", "path": "fuzz/f.c", "kind": "fuzz", "kind_guess": "test"}}
      ],
      "programs_not_compiled": ["tools/broken.c"],
      "closures": [
        {{"program": "t-lzg", "files": ["tools/lzg.c"], "questions": [], "duplicates": [], "linked": "ok"}},
        {{"program": "t-unlzg", "files": ["tools/unlzg.c"], "questions": ["d1", "d2"],
         "duplicates": [
           {{"set": "d1", "symbols": ["lzg_decode"],
             "definers": [{{"index": "d1.1", "path": "src/decode.c"}}, {{"index": "d1.2", "path": "src/mini.c"}}],
             "links": ["d1.1", "d1.2"]}},
           {{"set": "d2", "symbols": ["lzg_check"], "under": ["d1.1"],
             "definers": [{{"index": "d2.1", "path": "src/check_a.c"}}, {{"index": "d2.2", "path": "src/check_b.c"}}],
             "links": ["d2.1", "d2.2"]}}
         ]}}
      ],
      "libraries": [{{"id": "l-checksum", "files": ["src/checksum.c"], "needs_from_outside": []}},
                    {{"id": "l-bad", "files": ["src/bad.c"], "needs_from_outside": []}}]
    }}"#
        )
    }

    pub(crate) fn write_map(root: &Path, text: &str) {
        let dir = root.join("migration/map");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project-map.json"), text).unwrap();
    }

    /// The map's programs and libraries, each with its state in words.
    #[test]
    fn the_map_is_read_with_each_programs_state() {
        let p = project("states");
        assert_eq!(read_map(&p.0), Ok(None));
        write_map(&p.0, &map_text("stated", false));
        let map = read_map(&p.0).unwrap().unwrap();
        let state = |id: &str| map.get(id).unwrap().state;
        assert_eq!(state("t-lzg"), State::Linked);
        assert_eq!(state("t-unlzg"), State::Held);
        assert_eq!(state("t-fuzz"), State::Fuzzer);
        assert_eq!(state("l-checksum"), State::Compiles);
        assert_eq!(state("l-bad"), State::DidNotCompile);
        assert_eq!(map.not_compiled, ["tools/broken.c"]);
        assert_eq!(map.get("t-lzg").unwrap().kind_guess, "tool");
        assert_eq!(State::Held.words(), "needs a decision");
        assert_eq!(State::Linked.words(), "links");
        assert_eq!(map.configuration_words(), "stated (make)");
        assert!(!map.unstated() && map.held());
        // d2 is reached only when d1.1 is kept.
        let o = map.get("t-unlzg").unwrap();
        assert_eq!(o.next_held(&[], &[]).unwrap().set, "d1");
        assert!(o.next_held(&["d1.2".into()], &["d1".into()]).is_none());
        assert_eq!(
            o.next_held(&["d1.1".into()], &["d1".into()]).unwrap().set,
            "d2"
        );
        write_map(&p.0, &map_text("guessed", false));
        let map = read_map(&p.0).unwrap().unwrap();
        assert_eq!(
            map.configuration_words(),
            "a guess — no migration/map/config.toml states the build yet"
        );
        write_map(&p.0, &map_text("guessed", true));
        let map = read_map(&p.0).unwrap().unwrap();
        assert!(map
            .configuration_words()
            .starts_with("proposed — it came with the project (make)"));
    }

    /// The dialogs' words: what each act does, how long it takes and what
    /// it writes; the command is said once, by the dialog's "Command:" line
    /// (finding 10 of the newcomer's ten minutes: never twice).
    #[test]
    fn each_act_says_what_it_runs_how_long_and_what_it_writes() {
        let p = project("words");
        let target = p.0.to_string_lossy().into_owned();
        let map_argv = argv(Act::Map, &p.0, None, "", &[], None);
        assert_eq!(
            map_argv,
            [
                "project".to_string(),
                "map".into(),
                "--target".into(),
                p.0.to_string_lossy().into_owned()
            ]
        );
        assert_eq!(
            dialog(Act::Map, None, &map_argv),
            [
                "Map the project: find its programs and libraries, the files each one needs, \
                 what they share, and whether each program links (its code is compiled and \
                 linked in the sandbox, never run)."
                    .to_string(),
                "It takes: a few seconds for a small project, minutes for a large one (at most \
                 30 minutes)."
                    .into(),
                "It writes: migration/map/project-map.json (and migration/.gitignore the first \
                 time); the project's own files are not changed."
                    .into(),
            ]
        );
        write_map(&p.0, &map_text("stated", false));
        let map = read_map(&p.0).unwrap().unwrap();
        let ask = argv(Act::Ask, &p.0, Some(&map), "", &[], None);
        assert_eq!(ask, ["project", "ask", "--target", target.as_str()]);
        let words = dialog(Act::Ask, Some(&map), &ask).join("\n");
        assert!(!words.contains("It runs"), "{words}");
        for w in [
            "Ask a model which file to keep in each held choice: its answer is advice, shown \
             beside the choice when you Accept a program; you still decide.",
            "It takes: one model call through the external hand-off (one for each 10 held \
             choices)",
            "It writes: migration/map/project-map.reply.json",
        ] {
            assert!(words.contains(w), "{w}\n{words}");
        }
        let accept = argv(
            Act::Accept,
            &p.0,
            Some(&map),
            "t-unlzg",
            &["d1=d1.2".into()],
            Some("unlzg2"),
        );
        assert_eq!(
            accept,
            [
                "project",
                "accept",
                "t-unlzg",
                "--target",
                target.as_str(),
                "--keep",
                "d1=d1.2",
                "--run-name",
                "unlzg2"
            ]
        );
        let words = dialog(Act::Accept, Some(&map), &accept).join("\n");
        for w in [
            "Accept a program: check the map still matches the project",
            "It writes: migration/tools/<id>/harness.toml only (an id accepted before keeps its \
             ledger and what you added to that file).",
        ] {
            assert!(words.contains(w), "{w}\n{words}");
        }
        // A guess: Ask asks for the build.
        write_map(&p.0, &map_text("guessed", false));
        let map = read_map(&p.0).unwrap().unwrap();
        let ask = argv(Act::Ask, &p.0, Some(&map), "", &[], None);
        assert_eq!(ask.last().map(String::as_str), Some("--build"));
        let words = dialog(Act::Ask, Some(&map), &ask).join("\n");
        assert!(words.contains("propose a configuration"), "{words}");
        assert!(
            words.contains("It writes: migration/map/config.proposed.toml"),
            "{words}"
        );
        // Over ten held sets: the dialog counts the calls.
        let many = MapView {
            offered: vec![Offered {
                id: "t-a".into(),
                path: "a.c".into(),
                kind: "main".into(),
                kind_guess: "tool".into(),
                state: State::Held,
                held: (1..=11)
                    .map(|n| HeldSet {
                        set: format!("d{n}"),
                        definers: vec![],
                        under: vec![],
                    })
                    .collect(),
                index: "p1".into(),
                problem: String::new(),
            }],
            ..MapView::default()
        };
        let words = dialog(Act::Ask, Some(&many), &["ask".into()]).join("\n");
        assert!(words.contains("It takes: 2 model calls"), "{words}");
    }

    /// A reply bound to another map, or naming no definer of the set, is
    /// not shown; a good one is labelled as the model's.
    #[test]
    fn the_models_advice_is_read_only_when_it_fits_the_map() {
        let p = project("advice");
        write_map(&p.0, &map_text("stated", false));
        let map = read_map(&p.0).unwrap().unwrap();
        let reply = |root_hash: &str, keep: &str| {
            std::fs::write(
                p.0.join(REPLY_FILE),
                format!(
                    r#"{{"schema": "ruharness-project-map-reply", "root_hash": "{root_hash}",
                    "inputs_hash": "blake3:bb", "items": {{
                    "d1": {{"keep": "{keep}", "reason": "alternative-implementation",
                            "model": "claude-x", "provider": "external"}}}}}}"#
                ),
            )
            .unwrap();
        };
        reply("blake3:aa", "d1.2");
        let advice = read_advice(&p.0, &map);
        assert_eq!(
            advice["d1"].words(),
            "The model's advice (claude-x): keep d1.2, reason alternative-implementation — \
             advice only, the choice is yours."
        );
        for (root_hash, keep) in [("blake3:other", "d1.2"), ("blake3:aa", "d9.1")] {
            reply(root_hash, keep);
            assert!(read_advice(&p.0, &map).is_empty(), "{root_hash} {keep}");
        }
    }

    /// What the map knows of a program, in words; a model's label, read
    /// only when it fits the map; the example configuration the map
    /// suggests, the one `project map` prints (finding 7 of the newcomer's
    /// ten minutes: the rows say what each program is).
    #[test]
    fn a_program_says_what_the_map_and_the_model_know_of_it() {
        let p = project("labels");
        let text = map_text("guessed", false).replacen(
            "\"root_hash\"",
            "\"build_evidence\": {\"build_files\": [\"src/Makefile\"]}, \"root_hash\"",
            1,
        );
        write_map(&p.0, &text);
        let map = read_map(&p.0).unwrap().unwrap();
        let lzg = map.get("t-lzg").unwrap();
        assert_eq!(lzg.index, "p1");
        assert_eq!(lzg.what(), "a tool, by its folder");
        assert_eq!(map.get("t-fuzz").unwrap().what(), "a fuzzer");
        assert_eq!(map.get("l-checksum").unwrap().what(), "a library");
        assert_eq!(
            map.example,
            "[[configuration]]\nname = \"make\"\nfrom = \"make\"\nflags = []  # the -I and -D \
             flags the build passes, each joined, like \"-Isrc/include\"\n"
        );
        // The example is a configuration the map reads.
        assert_eq!(
            harness_oracle::projectmap::config::check_text(&p.0, &map.example)
                .unwrap()
                .len(),
            1
        );
        assert!(read_labels(&p.0, &map).is_empty());
        let reply = |root_hash: &str, name: &str| {
            std::fs::write(
                p.0.join(REPLY_FILE),
                format!(
                    r#"{{"root_hash": "{root_hash}", "inputs_hash": "blake3:bb", "items": {{
                    "p1": {{"kind": "tool", "name": "{name}", "purpose": "compresses a file",
                            "model": "claude-x", "provider": "external"}}}}}}"#
                ),
            )
            .unwrap();
        };
        reply("blake3:aa", "LZG compressor");
        let labels = read_labels(&p.0, &map);
        assert_eq!(
            labels["t-lzg"].words(),
            "The model calls it “LZG compressor” (a tool)"
        );
        assert_eq!(labels.len(), 1);
        // Bound to another map, or a name with a terminal code: not shown.
        for (root_hash, name) in [("blake3:other", "x"), ("blake3:aa", "a\\u001b[31mb")] {
            reply(root_hash, name);
            assert!(read_labels(&p.0, &map).is_empty(), "{root_hash} {name}");
        }
    }

    /// A map file whose set or index holds terminal codes, or whose id
    /// starts with `-`, is refused before anything of it is shown or run.
    #[test]
    fn a_map_with_ids_outside_their_shapes_is_refused() {
        let p = project("shapes");
        let good = map_text("stated", false);
        for bad in [
            good.replace(
                "\"set\": \"d1\"",
                "\"set\": \"d1\\u001b]0;TITLE\\u0007\\nFAKE\"",
            ),
            good.replace("\"index\": \"d1.1\"", "\"index\": \"d1.1\\u001b[31mRED\""),
            good.replace("\"id\": \"t-unlzg\"", "\"id\": \"--help\""),
        ] {
            write_map(&p.0, &bad);
            let err = read_map(&p.0).unwrap_err();
            assert!(
                err.contains("holds an id or an index that is not one the map writes"),
                "{err}"
            );
            assert!(!err.contains('\u{1b}'));
        }
    }

    #[test]
    fn the_view_applies_only_to_a_project_without_target() {
        let p = project("applies");
        assert!(applies(&p.0, None));
        assert!(!applies(&p.0, Some("t-a")));
        // One tool and no map: that tool opens, as the command line finds
        // it; several tools, or a map: the Project view.
        let tool = |id: &str| {
            let dir = config::tool_dir(&p.0, id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("harness.toml"), "schema_version = 2\n").unwrap();
        };
        tool("t-a");
        assert!(!applies(&p.0, None));
        tool("t-b");
        assert!(applies(&p.0, None));
        std::fs::write(p.0.join("harness.toml"), "schema_version = 1\n").unwrap();
        assert!(!applies(&p.0, None));
        assert!(!is_project(&p.0));
    }

    #[test]
    fn run_names_are_plain() {
        assert!(is_run_name("lzg"));
        assert!(is_run_name("lzg_2.x-1"));
        for bad in ["", "-x", ".x", "a b", "a/b", "a\u{1b}b", &"x".repeat(65)] {
            assert!(!is_run_name(bad), "{bad:?}");
        }
    }
}
