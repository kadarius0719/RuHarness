//! The cockpit's project mode (docs/PROJECT-MAP-DESIGN.md §3.7 "The cockpit
//! and harness-mcp"): opened on a project root with no `harness.toml`, the
//! cockpit offers the project's acts before the terminal is taken — **Map
//! the project**, and after a map **Ask** and **Accept a program** — each a
//! dialog that says what it runs, how long it takes and what it writes, and
//! runs the `harness project …` command with the same words as on the
//! command line. Once a map and a tool exist the same menu leads with each
//! tool to open (its program's path beside its id), and keeps Map, Ask and
//! Accept, so a second program can be accepted from the cockpit. Only the
//! person's typed answers run anything; harness-mcp never runs these acts.
//!
//! The acts follow the open question: while the configuration is a guess,
//! Ask asks for the build (`--build`) and Accept says, before any picker,
//! that a stated configuration is needed. Accept's picker shows a reply's
//! advice beside the definers, labelled as the model's, none preselected,
//! and skips a set the choice just made does not reach.
//!
//! The map file and the reply file live in the project: every id, set and
//! index is shape-checked before it is shown or put in a command, and every
//! string shown goes through [`safe_line`].

use harness_core::config;
use harness_core::runtime_view::shell_quote;
use harness_core::text::safe_line;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

/// The map file the mode reads, relative to the root.
const MAP_FILE: &str = "migration/map/project-map.json";
/// The reply file `harness project ask` writes, relative to the root.
const REPLY_FILE: &str = "migration/map/project-map.reply.json";
/// The largest map file the mode reads.
const MAX_MAP_BYTES: u64 = 64 << 20;
/// The largest reply file the mode reads.
const MAX_REPLY_BYTES: u64 = 16 << 20;
/// The held choices one `ask` call carries (harness-llm's `MAX_BATCH`).
const ASK_BATCH: usize = 10;
/// The reasons a set's advice may give (harness-llm's `SET_REASONS`).
const SET_REASONS: &[&str] = &["platform", "alternative-implementation", "cannot-tell"];

/// One of the project's acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Open an accepted tool, by id.
    Open(String),
    /// `harness project map`.
    Map,
    /// `harness project ask`.
    Ask,
    /// `harness project accept <id>`.
    Accept,
}

impl Act {
    /// The act's name on the menu.
    pub fn label(&self, mapped: bool) -> String {
        match self {
            Act::Open(id) => format!("Open {}", safe_line(id)),
            Act::Map if mapped => "Map the project again".into(),
            Act::Map => "Map the project".into(),
            Act::Ask => "Ask a model for advice".into(),
            Act::Accept => "Accept a program".into(),
        }
    }
}

/// What the mode ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// The person left.
    Leave,
    /// Open a target: `Some(id)` the tool they picked, `None` the only
    /// tool a run of Accept just wrote (the chooser finds it).
    Open(Option<String>),
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
    /// Its held sets (a `main` program's), in index order.
    pub held: Vec<HeldSet>,
}

/// What the mode reads of the map file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapView {
    /// Its programs and libraries.
    pub offered: Vec<Offered>,
    /// The configuration's name.
    pub configuration: String,
    /// It came with the project and is only proposed.
    pub proposed: bool,
    /// It is a guess.
    pub guessed: bool,
    /// The map's `root_hash` and `inputs_hash` (a reply is bound to them).
    pub digests: (String, String),
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

    /// The `main` programs and libraries: what `accept` takes.
    pub fn acceptable(&self) -> Vec<&Offered> {
        self.offered
            .iter()
            .filter(|o| o.kind == "main" || o.kind == "library")
            .collect()
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

/// The map under `root`: `Ok(None)` when there is none, `Err` (one
/// sentence) when it is not one this mode reads — too large, not JSON, or
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
    let mut offered = Vec::new();
    for p in list(&v["programs"]) {
        let id = s(&p["id"]);
        if !is_id(&id, "t-") {
            return Err(shapes());
        }
        let closure = list(&v["closures"])
            .into_iter()
            .find(|c| c["program"].as_str() == Some(id.as_str()));
        let mut held = Vec::new();
        if let Some(c) = closure {
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
        }
        offered.push(Offered {
            id,
            path: s(&p["path"]),
            kind: s(&p["kind"]),
            held,
        });
    }
    for l in list(&v["libraries"]) {
        let id = s(&l["id"]);
        if !is_id(&id, "l-") {
            return Err(shapes());
        }
        offered.push(Offered {
            id,
            path: list(&l["files"]).first().map(s).unwrap_or_default(),
            kind: "library".into(),
            held: Vec::new(),
        });
    }
    let c = &v["configuration"];
    Ok(Some(MapView {
        offered,
        configuration: s(&c["name"]),
        proposed: c["proposed"].as_bool().unwrap_or(false),
        guessed: c["source"].as_str() == Some("guessed"),
        digests: (s(&v["root_hash"]), s(&v["inputs_hash"])),
    }))
}

/// A model's advice on one held set, from the reply file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// The advised definer's index, or `undecided`.
    pub keep: String,
    /// One of the reasons.
    pub reason: String,
    /// The model that gave it.
    pub model: String,
}

/// The advice of the reply file under `root`, by set, when it is bound to
/// `map`'s digests; each answer checked as `ask` checks a fresh one (its
/// keep one of the set's definers or `undecided`, its reason one of the
/// list, its model one showable line), and dropped when it fails.
pub fn read_advice(root: &Path, map: &MapView) -> BTreeMap<String, Advice> {
    let mut out = BTreeMap::new();
    let path = root.join(REPLY_FILE);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return out;
    };
    if !meta.file_type().is_file() || meta.len() > MAX_REPLY_BYTES {
        return out;
    }
    let Some(v) = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    else {
        return out;
    };
    if v["root_hash"].as_str() != Some(map.digests.0.as_str())
        || v["inputs_hash"].as_str() != Some(map.digests.1.as_str())
    {
        return out;
    }
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
            let showable = !model.is_empty()
                && model.chars().count() <= 200
                && !model.chars().any(harness_core::text::unsafe_to_show);
            if known && SET_REASONS.contains(&reason) && showable {
                out.insert(
                    h.set.clone(),
                    Advice {
                        keep: keep.to_string(),
                        reason: reason.to_string(),
                        model: model.to_string(),
                    },
                );
            }
        }
    }
    out
}

/// The project mode applies: no `--tool`, no root `harness.toml`, and
/// either no mapped tool yet or a map to act on.
pub fn applies(root: &Path, tool: Option<&str>) -> bool {
    tool.is_none()
        && std::fs::symlink_metadata(root.join(config::CONFIG_FILE)).is_err()
        && (config::mapped_tools(root).is_empty()
            || std::fs::symlink_metadata(root.join(MAP_FILE)).is_ok())
}

/// A tool as the chooser names it: its id and, from the map, its program's
/// file (or a library's first file); the id alone when the map does not
/// name it.
pub fn tool_label(root: &Path, map: Option<&MapView>, id: &str) -> String {
    let from_map = map.and_then(|m| m.offered.iter().find(|o| o.id == id));
    match from_map {
        Some(o) if o.kind == "library" => {
            format!("{} — library, {}", safe_line(id), safe_line(&o.path))
        }
        Some(o) => format!("{} — {}", safe_line(id), safe_line(&o.path)),
        None => {
            let _ = root;
            safe_line(id)
        }
    }
}

/// The acts offered: each tool to open, then Map always; Ask and Accept
/// once a map exists.
pub fn offered(map: Option<&MapView>, tools: &[String]) -> Vec<Act> {
    let mut acts: Vec<Act> = tools.iter().cloned().map(Act::Open).collect();
    acts.push(Act::Map);
    if map.is_some() {
        acts.extend([Act::Ask, Act::Accept]);
    }
    acts
}

/// The command an act runs (the words after `harness`), as argv.
pub fn argv(
    act: &Act,
    root: &Path,
    map: Option<&MapView>,
    id: &str,
    keeps: &[String],
) -> Vec<String> {
    let target = root.to_string_lossy().into_owned();
    let mut v: Vec<String> = vec!["project".into()];
    match act {
        Act::Open(_) => return Vec::new(),
        Act::Map => v.extend(["map".into(), "--target".into(), target]),
        Act::Ask => {
            v.extend(["ask".into(), "--target".into(), target]);
            // The open question: the build while the configuration is a
            // guess (or nothing is held), else the held choices.
            if map.is_none_or(|m| m.unstated() || !m.held()) {
                v.push("--build".into());
            }
        }
        Act::Accept => {
            v.extend(["accept".into(), id.to_string(), "--target".into(), target]);
            for k in keeps {
                v.extend(["--keep".into(), k.clone()]);
            }
        }
    }
    v
}

/// `harness <argv>` as the person would type it.
pub fn command_line(argv: &[String]) -> String {
    let mut s = String::from("harness");
    for a in argv {
        s.push(' ');
        s.push_str(&shell_quote(a));
    }
    s
}

/// The dialog's words for an act: what it runs, how long it takes and what
/// it writes.
pub fn dialog(act: &Act, map: Option<&MapView>, argv: &[String]) -> String {
    let runs = format!("It runs: {}", safe_line(&command_line(argv)));
    let build = argv.iter().any(|a| a == "--build");
    let (what, long, writes): (String, String, &str) = match act {
        Act::Open(_) => return String::new(),
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
    format!("{what}\n  {runs}\n  It takes: {long}.\n  It writes: {writes}.\n")
}

/// Say `text` on `output`.
fn say(output: &mut dyn Write, text: &str) -> Result<(), String> {
    output
        .write_all(text.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|e| format!("the terminal: {e}"))
}

/// Read one line; `None` at the end of input.
fn read(input: &mut dyn BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    let n = input
        .read_line(&mut line)
        .map_err(|e| format!("the terminal: {e}"))?;
    Ok((n > 0).then(|| line.trim().to_string()))
}

/// The lines that open the mode: what the folder is and what the map says.
pub fn heading(root: &Path, map: Result<Option<&MapView>, &str>, tools: &[String]) -> String {
    let mut t = if tools.is_empty() {
        format!(
            "harness-tui: {} holds no harness.toml and no tool yet: it is a C project to map.\n",
            safe_line(&root.display().to_string())
        )
    } else {
        format!(
            "harness-tui: {} is a mapped C project with {} accepted tool(s).\n",
            safe_line(&root.display().to_string()),
            tools.len()
        )
    };
    match map {
        Err(why) => t.push_str(&format!("  {}\n", safe_line(why))),
        Ok(None) => t.push_str("  No map yet.\n"),
        Ok(Some(m)) => {
            let programs = m.offered.iter().filter(|o| o.kind != "library").count();
            let libraries = m.offered.len() - programs;
            t.push_str(&format!(
                "  The map shows {programs} program(s) and {libraries} librar{}.\n",
                if libraries == 1 { "y" } else { "ies" }
            ));
            if m.proposed {
                t.push_str(&format!(
                    "  Its configuration ({}) came with the project: proposed, not yours yet — \
                     state it with `harness project map --adopt`, or write your own \
                     migration/map/config.toml.\n",
                    safe_line(&m.configuration)
                ));
            } else if m.guessed {
                t.push_str(
                    "  Its configuration is a guess: a program is accepted under a stated one \
                     (write it in migration/map/config.toml, or Ask for a proposal).\n",
                );
            } else {
                t.push_str(&format!(
                    "  Its configuration: {}.\n",
                    safe_line(&m.configuration)
                ));
            }
        }
    }
    t
}

/// Read a number from 1 to `n`; `None` for anything else.
fn pick_number(input: &mut dyn BufRead, n: usize) -> Result<Option<usize>, String> {
    Ok(read(input)?
        .and_then(|a| a.parse::<usize>().ok())
        .and_then(|k| k.checked_sub(1))
        .filter(|&i| i < n))
}

/// The person's pick of a program and its keeps, for Accept: `None` when
/// they typed nothing that names one. Each held set is asked in index
/// order, a set the choices made so far do not reach skipped; a model's
/// advice from the reply file is shown beside the definers, labelled as
/// the model's, and none is preselected.
fn pick_program(
    root: &Path,
    map: &MapView,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<Option<(String, Vec<String>)>, String> {
    if map.unstated() {
        say(
            output,
            "Accept needs a stated configuration, and this map's is a guess (or came with the \
             project): write it in migration/map/config.toml, or Ask for a proposal, then map \
             again.\n\n",
        )?;
        return Ok(None);
    }
    let offered = map.acceptable();
    if offered.is_empty() {
        say(
            output,
            "The map offers no program or library to accept; map the project again after it \
             changes.\n",
        )?;
        return Ok(None);
    }
    let mut t = String::from("Which one?\n");
    for (i, o) in offered.iter().enumerate() {
        t.push_str(&format!(
            "  {}. {} — {} ({})\n",
            i + 1,
            safe_line(&o.id),
            safe_line(&o.path),
            if o.kind == "library" {
                "library"
            } else {
                "program"
            }
        ));
    }
    t.push_str(&format!(
        "Type its number (1-{}) and Enter; anything else goes back: ",
        offered.len()
    ));
    say(output, &t)?;
    let Some(i) = pick_number(input, offered.len())? else {
        return Ok(None);
    };
    let o = offered[i];
    let advice = read_advice(root, map);
    let mut kept: Vec<String> = Vec::new();
    let mut keeps = Vec::new();
    for h in &o.held {
        // Reached only under a choice the person did not make: skipped.
        if !h.under.is_empty() && !h.under.iter().any(|u| kept.contains(u)) {
            continue;
        }
        // The choice is the person's: the definers are listed, none
        // suggested; the model's advice, when there is some, beside them.
        let mut t = format!(
            "{} holds the choice {}: linking cannot tell its files apart, so keep which one?\n",
            safe_line(&o.id),
            safe_line(&h.set)
        );
        for (i, (index, path)) in h.definers.iter().enumerate() {
            t.push_str(&format!(
                "  {}. {} {}\n",
                i + 1,
                safe_line(index),
                safe_line(path)
            ));
        }
        if let Some(a) = advice.get(&h.set) {
            if a.keep == "undecided" {
                t.push_str(&format!(
                    "  The model's advice ({}): it could not decide (reason {}).\n",
                    safe_line(&a.model),
                    a.reason
                ));
            } else {
                t.push_str(&format!(
                    "  The model's advice ({}): keep {}, reason {} — advice only, the choice is \
                     yours.\n",
                    safe_line(&a.model),
                    safe_line(&a.keep),
                    a.reason
                ));
            }
        }
        t.push_str(&format!(
            "Type its number (1-{}) and Enter; anything else goes back: ",
            h.definers.len()
        ));
        say(output, &t)?;
        let Some(i) = pick_number(input, h.definers.len())? else {
            return Ok(None);
        };
        let index = &h.definers[i].0;
        kept.push(index.clone());
        keeps.push(format!("{}={index}", h.set));
    }
    Ok(Some((o.id.clone(), keeps)))
}

/// The project mode at `root`: the menu, each act's dialog, the command run
/// by `exec` (its argv after `harness`; its exit code). [`Next::Open`] when
/// the person picked a tool to open or Accept wrote the first one,
/// [`Next::Leave`] when they left.
pub fn run(
    root: &Path,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    exec: &mut dyn FnMut(&[String]) -> Result<i32, String>,
) -> Result<Next, String> {
    loop {
        let tools = config::mapped_tools(root);
        let mapped = read_map(root);
        let map = mapped.as_ref().ok().and_then(Option::as_ref);
        let acts = offered(map, &tools);
        let mut t = heading(
            root,
            mapped.as_ref().map(Option::as_ref).map_err(String::as_str),
            &tools,
        );
        for (i, act) in acts.iter().enumerate() {
            let label = match act {
                Act::Open(id) => format!("Open {}", tool_label(root, map, id)),
                other => other.label(map.is_some()),
            };
            t.push_str(&format!("  {}. {label}\n", i + 1));
        }
        t.push_str(&format!(
            "Type a number (1-{}) and Enter; anything else leaves: ",
            acts.len()
        ));
        say(output, &t)?;
        let Some(i) = pick_number(input, acts.len())? else {
            say(
                output,
                "nothing run; start the cockpit again to map the project\n",
            )?;
            return Ok(Next::Leave);
        };
        let act = acts[i].clone();
        if let Act::Open(id) = &act {
            return Ok(Next::Open(Some(id.clone())));
        }
        let (id, keeps) = match (&act, map) {
            (Act::Accept, Some(m)) => match pick_program(root, m, input, output)? {
                Some(pick) => pick,
                None => continue,
            },
            _ => (String::new(), Vec::new()),
        };
        let argv = argv(&act, root, map, &id, &keeps);
        say(
            output,
            &format!(
                "\n{}Run it? Type y and Enter; anything else goes back: ",
                dialog(&act, map, &argv)
            ),
        )?;
        let Some(answer) = read(input)? else {
            return Ok(Next::Leave);
        };
        if !matches!(answer.as_str(), "y" | "Y") {
            continue;
        }
        let code = exec(&argv)?;
        say(
            output,
            &format!(
                "{} ended with exit {code}{}\n\n",
                safe_line(&command_line(&argv)),
                if code == 0 {
                    ""
                } else {
                    " (its words are above)"
                }
            ),
        )?;
        // The first tool accepted: the cockpit opens it.
        if act == Act::Accept && code == 0 && tools.is_empty() {
            return Ok(Next::Open(None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str) -> crate::testutil::TmpDir {
        let tmp = crate::testutil::TmpDir::new(&format!("project-mode-{tag}"));
        std::fs::write(tmp.0.join("a.c"), "int main(void) { return 0; }\n").unwrap();
        tmp
    }

    /// A map whose configuration is `source` (`proposed` when shipped):
    /// t-unlzg holds d1 (lzg_decode), and d2 only when d1.1 is kept.
    fn map_text(source: &str, proposed: bool) -> String {
        format!(
            r#"{{
      "root_hash": "blake3:aa", "inputs_hash": "blake3:bb",
      "configuration": {{"name": "make", "from": "make", "source": "{source}", "proposed": {proposed}}},
      "programs": [
        {{"id": "t-unlzg", "index": "p1", "path": "tools/unlzg.c", "kind": "main", "kind_guess": "tool"}},
        {{"id": "t-fuzz", "path": "fuzz/f.c", "kind": "fuzz", "kind_guess": "test"}}
      ],
      "closures": [
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
      "libraries": [{{"id": "l-checksum", "files": ["src/checksum.c"], "needs_from_outside": []}}]
    }}"#
        )
    }

    fn write_map(root: &Path, text: &str) {
        let dir = root.join("migration/map");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project-map.json"), text).unwrap();
    }

    fn drive(
        root: &Path,
        typed: &str,
        exec: &mut dyn FnMut(&[String]) -> Result<i32, String>,
    ) -> (Result<Next, String>, String) {
        let mut input = std::io::Cursor::new(typed.as_bytes().to_vec());
        let mut output = Vec::new();
        let got = run(root, &mut input, &mut output, exec);
        (got, String::from_utf8(output).unwrap())
    }

    /// No map: only Map the project is offered, its dialog says what it
    /// runs, how long and what it writes, and only a typed `y` runs it.
    #[test]
    fn with_no_map_only_map_the_project_is_offered() {
        let p = project("no-map");
        let mut ran: Vec<Vec<String>> = Vec::new();
        let (got, said) = drive(&p.0, "1\nn\n2\n", &mut |argv| {
            ran.push(argv.to_vec());
            Ok(0)
        });
        assert_eq!(got, Ok(Next::Leave));
        assert!(ran.is_empty(), "{ran:?}");
        assert!(
            said.contains("holds no harness.toml and no tool yet"),
            "{said}"
        );
        assert!(said.contains("No map yet."), "{said}");
        assert!(said.contains("  1. Map the project\n"), "{said}");
        assert!(!said.contains("Accept a program"), "{said}");
        assert!(!said.contains("Ask a model"), "{said}");
        let target = shell_quote(&p.0.to_string_lossy());
        for words in [
            format!("It runs: harness project map --target {target}"),
            "It takes: a few seconds for a small project, minutes for a large one (at most 30 \
             minutes)."
                .to_string(),
            "It writes: migration/map/project-map.json (and migration/.gitignore the first \
             time); the project's own files are not changed."
                .to_string(),
            "Run it? Type y and Enter; anything else goes back:".to_string(),
        ] {
            assert!(said.contains(&words), "{words}\n{said}");
        }
        // `y` runs the same words.
        let mut ran = Vec::new();
        let (got, said) = drive(&p.0, "1\ny\nq\n", &mut |argv| {
            ran.push(argv.to_vec());
            Ok(0)
        });
        assert_eq!(got, Ok(Next::Leave));
        assert_eq!(
            ran,
            [vec![
                "project".to_string(),
                "map".into(),
                "--target".into(),
                p.0.to_string_lossy().into_owned()
            ]]
        );
        assert!(
            said.contains(&format!(
                "harness project map --target {target} ended with exit 0"
            )),
            "{said}"
        );
    }

    /// After a map with a stated configuration: Ask and Accept are
    /// offered; Accept lists the programs and libraries and asks for each
    /// held choice without suggesting a file; the first tool it writes
    /// ends the mode.
    #[test]
    fn after_a_map_ask_and_accept_are_offered() {
        let p = project("mapped");
        write_map(&p.0, &map_text("stated", false));
        let map = read_map(&p.0).unwrap().unwrap();
        assert!(!map.unstated() && map.held());
        let (got, said) = drive(&p.0, "2\nn\nx\n", &mut |_| Ok(0));
        assert_eq!(got, Ok(Next::Leave));
        for words in [
            "The map shows 2 program(s) and 1 library.",
            "Its configuration: make.",
            "  1. Map the project again\n  2. Ask a model for advice\n  3. Accept a program\n",
            "Ask a model which file to keep in each held choice: its answer is advice, shown \
             beside the choice when you Accept a program; you still decide.",
            "It takes: one model call through the external hand-off (one for each 10 held \
             choices)",
            "It writes: migration/map/project-map.reply.json",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
        let target = shell_quote(&p.0.to_string_lossy());
        assert!(
            said.contains(&format!("It runs: harness project ask --target {target}\n")),
            "{said}"
        );
        // Accept: the program, its held choice (d1.2 kept, so d2 is not
        // reached and not asked), then the dialog; `y` runs accept with the
        // person's keep, and the tool it writes ends the mode.
        let root = p.0.clone();
        let mut ran = Vec::new();
        let (got, said) = drive(&p.0, "3\n1\n2\ny\n", &mut |argv| {
            ran.push(argv.to_vec());
            let dir = config::tool_dir(&root, "t-unlzg");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("harness.toml"), "schema_version = 2\n").unwrap();
            Ok(0)
        });
        assert_eq!(got, Ok(Next::Open(None)));
        for words in [
            "Which one?\n  1. t-unlzg — tools/unlzg.c (program)\n  2. l-checksum — \
             src/checksum.c (library)\n",
            "t-unlzg holds the choice d1: linking cannot tell its files apart, so keep which \
             one?\n  1. d1.1 src/decode.c\n  2. d1.2 src/mini.c\n",
            "Accept a program: check the map still matches the project",
            "It writes: migration/tools/<id>/harness.toml only (an id accepted before keeps its \
             ledger and what you added to that file).",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
        assert!(!said.contains("choice d2"), "d2 is not reached: {said}");
        assert!(!said.contains("model's advice"), "no reply: {said}");
        assert!(
            !said.contains("t-fuzz —"),
            "a fuzzer is not offered: {said}"
        );
        assert!(
            said.contains(&format!(
                "It runs: harness project accept t-unlzg --target {target} --keep d1=d1.2\n"
            )),
            "{said}"
        );
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0][..3], ["project", "accept", "t-unlzg"]);
        // A tool exists and so does the map: the mode still applies, the
        // tool first on the menu with its program's path, the acts kept.
        assert!(applies(&p.0, None));
        let (got, said) = drive(&p.0, "1\n", &mut |_| Ok(0));
        assert_eq!(got, Ok(Next::Open(Some("t-unlzg".into()))));
        for words in [
            "is a mapped C project with 1 accepted tool(s).",
            "  1. Open t-unlzg — tools/unlzg.c\n  2. Map the project again\n  3. Ask a model \
             for advice\n  4. Accept a program\n",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
        // Without a map, a tool opens through the chooser as before.
        std::fs::remove_file(p.0.join(MAP_FILE)).unwrap();
        assert!(!applies(&p.0, None));
        assert_eq!(
            crate::chooser::choose(&p.0, None, None),
            Ok(Some("t-unlzg".to_string()))
        );
    }

    /// A set reached only under a choice is asked when that choice is
    /// made; the reply's advice is shown beside the definers, labelled as
    /// the model's, none preselected; a reply for another map, or one
    /// naming no definer of the set, is not shown.
    #[test]
    fn accept_asks_the_reached_sets_and_shows_the_models_advice() {
        let p = project("advice");
        write_map(&p.0, &map_text("stated", false));
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
        let mut ran = Vec::new();
        let (got, said) = drive(&p.0, "3\n1\n1\n2\nn\n", &mut |argv| {
            ran.push(argv.to_vec());
            Ok(0)
        });
        assert_eq!(got, Ok(Next::Leave));
        assert!(ran.is_empty());
        for words in [
            "  1. d1.1 src/decode.c\n  2. d1.2 src/mini.c\n  The model's advice (claude-x): \
             keep d1.2, reason alternative-implementation — advice only, the choice is yours.\n",
            "t-unlzg holds the choice d2: linking cannot tell its files apart, so keep which \
             one?\n  1. d2.1 src/check_a.c\n  2. d2.2 src/check_b.c\n",
            "--keep d1=d1.1 --keep d2=d2.2\n",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
        for (root_hash, keep) in [("blake3:other", "d1.2"), ("blake3:aa", "d9.1")] {
            reply(root_hash, keep);
            let (_, said) = drive(&p.0, "3\n1\n2\nn\n", &mut |_| Ok(0));
            assert!(
                !said.contains("model's advice"),
                "{root_hash} {keep}: {said}"
            );
        }
    }

    /// While the configuration is a guess: Ask asks for the build, and
    /// Accept says before any picker that a stated configuration is
    /// needed, running nothing.
    #[test]
    fn a_guessed_map_asks_for_the_build_and_accept_says_why_not() {
        let p = project("guess");
        write_map(&p.0, &map_text("guessed", false));
        let map = read_map(&p.0).unwrap().unwrap();
        assert!(map.unstated() && map.held());
        let ask = argv(&Act::Ask, &p.0, Some(&map), "", &[]);
        assert_eq!(ask.last().map(String::as_str), Some("--build"));
        let words = dialog(&Act::Ask, Some(&map), &ask);
        assert!(words.contains("propose a configuration"), "{words}");
        assert!(
            words.contains("It writes: migration/map/config.proposed.toml"),
            "{words}"
        );
        let mut ran = Vec::new();
        let (got, said) = drive(&p.0, "3\nx\n", &mut |argv| {
            ran.push(argv.to_vec());
            Ok(0)
        });
        assert_eq!(got, Ok(Next::Leave));
        assert!(ran.is_empty());
        assert!(
            said.contains(
                "Accept needs a stated configuration, and this map's is a guess (or came with \
                 the project): write it in migration/map/config.toml, or Ask for a proposal, \
                 then map again."
            ),
            "{said}"
        );
        assert!(!said.contains("Which one?"), "{said}");
        // A shipped configuration: the same, and the heading says how to
        // state it.
        write_map(&p.0, &map_text("guessed", true));
        let map = read_map(&p.0).unwrap().unwrap();
        assert!(heading(&p.0, Ok(Some(&map)), &[])
            .contains("Its configuration (make) came with the project: proposed, not yours yet"));
        assert_eq!(
            argv(&Act::Ask, &p.0, Some(&map), "", &[])
                .last()
                .map(String::as_str),
            Some("--build")
        );
        // Over ten held sets: the dialog counts the calls.
        let many = MapView {
            offered: vec![Offered {
                id: "t-a".into(),
                path: "a.c".into(),
                kind: "main".into(),
                held: (1..=11)
                    .map(|n| HeldSet {
                        set: format!("d{n}"),
                        definers: vec![],
                        under: vec![],
                    })
                    .collect(),
            }],
            ..MapView::default()
        };
        let words = dialog(&Act::Ask, Some(&many), &["ask".into()]);
        assert!(words.contains("It takes: 2 model calls"), "{words}");
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
            let (got, said) = drive(&p.0, "2\n", &mut |_| Ok(0));
            assert_eq!(got, Ok(Next::Leave));
            assert!(!said.contains('\u{1b}'), "{said:?}");
            assert!(!said.contains("FAKE"), "{said}");
            assert!(said.contains("  1. Map the project\n"), "{said}");
            assert!(!said.contains("Accept a program"), "{said}");
        }
    }

    #[test]
    fn the_mode_applies_only_to_a_project_without_target() {
        let p = project("applies");
        assert!(applies(&p.0, None));
        assert!(!applies(&p.0, Some("t-a")));
        std::fs::write(p.0.join("harness.toml"), "schema_version = 1\n").unwrap();
        assert!(!applies(&p.0, None));
    }
}
