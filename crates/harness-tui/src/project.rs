//! The cockpit's project mode (docs/PROJECT-MAP-DESIGN.md §3.7 "The cockpit
//! and harness-mcp"): opened on a project root with no `harness.toml` and
//! no mapped tool, the cockpit offers the project's three acts before the
//! terminal is taken — **Map the project**, and after a map **Ask** and
//! **Accept a program** — each a dialog that says what it runs, how long it
//! takes and what it writes, and runs the `harness project …` command with
//! the same words as on the command line. Once a tool exists the cockpit
//! opens it as on any target (the tool chooser lists several). Only the
//! person's typed answers run anything; harness-mcp never runs these acts.

use harness_core::config;
use harness_core::runtime_view::shell_quote;
use harness_core::text::safe_line;
use std::io::{BufRead, Write};
use std::path::Path;

/// The map file the mode reads, relative to the root.
const MAP_FILE: &str = "migration/map/project-map.json";
/// The largest map file the mode reads.
const MAX_MAP_BYTES: u64 = 64 << 20;

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

impl Act {
    /// The act's name on the menu.
    pub fn label(self, mapped: bool) -> &'static str {
        match self {
            Act::Map if mapped => "Map the project again",
            Act::Map => "Map the project",
            Act::Ask => "Ask a model for advice",
            Act::Accept => "Accept a program",
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
    /// Its held sets (a `main` program's).
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
}

impl MapView {
    /// Any program holds a set.
    pub fn held(&self) -> bool {
        self.offered.iter().any(|o| !o.held.is_empty())
    }

    /// The `main` programs and libraries: what `accept` takes.
    pub fn acceptable(&self) -> Vec<&Offered> {
        self.offered
            .iter()
            .filter(|o| o.kind == "main" || o.kind == "library")
            .collect()
    }
}

/// The map under `root`, when there is one this mode can read.
pub fn read_map(root: &Path) -> Option<MapView> {
    let path = root.join(MAP_FILE);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_MAP_BYTES {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let empty = Vec::new();
    let list = |v: &serde_json::Value| v.as_array().unwrap_or(&empty).clone();
    let mut offered = Vec::new();
    for p in list(&v["programs"]) {
        let id = s(&p["id"]);
        let closure = list(&v["closures"])
            .into_iter()
            .find(|c| c["program"].as_str() == Some(id.as_str()));
        let held = closure
            .map(|c| {
                let questions: Vec<String> = list(&c["questions"]).iter().map(s).collect();
                list(&c["duplicates"])
                    .iter()
                    .filter(|d| questions.contains(&s(&d["set"])))
                    .map(|d| HeldSet {
                        set: s(&d["set"]),
                        definers: list(&d["definers"])
                            .iter()
                            .map(|x| (s(&x["index"]), s(&x["path"])))
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        offered.push(Offered {
            id,
            path: s(&p["path"]),
            kind: s(&p["kind"]),
            held,
        });
    }
    for l in list(&v["libraries"]) {
        offered.push(Offered {
            id: s(&l["id"]),
            path: list(&l["files"]).first().map(s).unwrap_or_default(),
            kind: "library".into(),
            held: Vec::new(),
        });
    }
    let c = &v["configuration"];
    Some(MapView {
        offered,
        configuration: s(&c["name"]),
        proposed: c["proposed"].as_bool().unwrap_or(false),
        guessed: c["source"].as_str() == Some("guessed"),
    })
}

/// The project mode applies: no `--tool`, no root `harness.toml`, no
/// mapped tool.
pub fn applies(root: &Path, tool: Option<&str>) -> bool {
    tool.is_none()
        && std::fs::symlink_metadata(root.join(config::CONFIG_FILE)).is_err()
        && config::mapped_tools(root).is_empty()
}

/// The acts offered: Map always; Ask and Accept once a map exists.
pub fn offered(map: Option<&MapView>) -> Vec<Act> {
    match map {
        None => vec![Act::Map],
        Some(_) => vec![Act::Map, Act::Ask, Act::Accept],
    }
}

/// The command an act runs (the words after `harness`), as argv.
pub fn argv(
    act: Act,
    root: &Path,
    map: Option<&MapView>,
    id: &str,
    keeps: &[String],
) -> Vec<String> {
    let target = root.to_string_lossy().into_owned();
    let mut v: Vec<String> = vec!["project".into()];
    match act {
        Act::Map => v.extend(["map".into(), "--target".into(), target]),
        Act::Ask => {
            v.extend(["ask".into(), "--target".into(), target]);
            // With no held choice, the question worth asking is the build.
            if !map.is_some_and(MapView::held) {
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
pub fn dialog(act: Act, map: Option<&MapView>, argv: &[String]) -> String {
    let runs = format!("It runs: {}", safe_line(&command_line(argv)));
    let (what, long, writes) = match act {
        Act::Map => (
            "Map the project: find its programs and libraries, the files each one needs, what \
             they share, and whether each program links (its code is compiled and linked in the \
             sandbox, never run)."
                .to_string(),
            "a few seconds for a small project, minutes for a large one (at most 30 minutes)",
            "migration/map/project-map.json (and migration/.gitignore the first time); the \
             project's own files are not changed",
        ),
        Act::Ask => {
            let what = if map.is_some_and(MapView::held) {
                "Ask a model which file to keep in each held choice: its answer is advice shown \
                 beside the choice; you still decide with Accept a program."
            } else {
                "Ask a model to read the project's build files and propose a configuration (its \
                 name, from and flags); you state it yourself in migration/map/config.toml."
            };
            (
                what.to_string(),
                "one model call through the external hand-off: the command stops and waits for \
                 the answer file, and prints the command that resumes",
                "migration/map/project-map.reply.json (or migration/map/config.proposed.toml) and \
                 its traces under migration/map/traces/; builds nothing",
            )
        }
        Act::Accept => (
            "Accept a program: check the map still matches the project, apply your picks, link \
             the program once more and write it as a tool you can scan, plan and migrate."
                .to_string(),
            "about as long as the map (it maps the project again to check nothing changed), \
             then one link",
            "migration/tools/<id>/harness.toml only (an id accepted before keeps its ledger)",
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
pub fn heading(root: &Path, map: Option<&MapView>) -> String {
    let mut t = format!(
        "harness-tui: {} holds no harness.toml and no tool yet: it is a C project to map.\n",
        safe_line(&root.display().to_string())
    );
    match map {
        None => t.push_str("  No map yet.\n"),
        Some(m) => {
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

/// The person's pick of a program and its keeps, for Accept: `None` when
/// they typed nothing that names one.
fn pick_program(
    map: &MapView,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<Option<(String, Vec<String>)>, String> {
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
    let Some(answer) = read(input)? else {
        return Ok(None);
    };
    let Some(o) = answer
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| offered.get(i))
    else {
        return Ok(None);
    };
    let mut keeps = Vec::new();
    for h in &o.held {
        // The choice is the person's: the definers are listed, none
        // suggested.
        let mut t = format!(
            "{} holds the choice {}: linking cannot tell its files apart, so keep which one?\n",
            safe_line(&o.id),
            h.set
        );
        for (i, (index, path)) in h.definers.iter().enumerate() {
            t.push_str(&format!("  {}. {index} {}\n", i + 1, safe_line(path)));
        }
        t.push_str(&format!(
            "Type its number (1-{}) and Enter; anything else goes back: ",
            h.definers.len()
        ));
        say(output, &t)?;
        let Some(answer) = read(input)? else {
            return Ok(None);
        };
        let Some((index, _)) = answer
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| h.definers.get(i))
        else {
            return Ok(None);
        };
        keeps.push(format!("{}={index}", h.set));
    }
    Ok(Some((o.id.clone(), keeps)))
}

/// The project mode at `root`: the menu, each act's dialog, the command run
/// by `exec` (its argv after `harness`; its exit code). `Ok(true)` when a
/// tool exists now (the cockpit opens it), `Ok(false)` when the person left.
pub fn run(
    root: &Path,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    exec: &mut dyn FnMut(&[String]) -> Result<i32, String>,
) -> Result<bool, String> {
    loop {
        if !config::mapped_tools(root).is_empty() {
            return Ok(true);
        }
        let map = read_map(root);
        let acts = offered(map.as_ref());
        let mut t = heading(root, map.as_ref());
        for (i, act) in acts.iter().enumerate() {
            t.push_str(&format!("  {}. {}\n", i + 1, act.label(map.is_some())));
        }
        t.push_str(&format!(
            "Type a number (1-{}) and Enter; anything else leaves: ",
            acts.len()
        ));
        say(output, &t)?;
        let Some(answer) = read(input)? else {
            return Ok(false);
        };
        let Some(&act) = answer
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| acts.get(i))
        else {
            say(
                output,
                "nothing run; start the cockpit again to map the project\n",
            )?;
            return Ok(false);
        };
        let (id, keeps) = match (act, &map) {
            (Act::Accept, Some(m)) => match pick_program(m, input, output)? {
                Some(pick) => pick,
                None => continue,
            },
            _ => (String::new(), Vec::new()),
        };
        let argv = argv(act, root, map.as_ref(), &id, &keeps);
        say(
            output,
            &format!(
                "\n{}Run it? Type y and Enter; anything else goes back: ",
                dialog(act, map.as_ref(), &argv)
            ),
        )?;
        let Some(answer) = read(input)? else {
            return Ok(false);
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

    const MAP: &str = r#"{
      "root_hash": "blake3:aa", "inputs_hash": "blake3:bb",
      "configuration": {"name": "make", "from": "make", "source": "guessed", "proposed": true},
      "programs": [
        {"id": "t-unlzg", "index": "p1", "path": "tools/unlzg.c", "kind": "main", "kind_guess": "tool"},
        {"id": "t-fuzz", "path": "fuzz/f.c", "kind": "fuzz", "kind_guess": "test"}
      ],
      "closures": [
        {"program": "t-unlzg", "files": ["tools/unlzg.c"], "questions": ["d1"],
         "duplicates": [{"set": "d1", "symbols": ["lzg_decode"],
           "definers": [{"index": "d1.1", "path": "src/decode.c"}, {"index": "d1.2", "path": "src/mini.c"}],
           "links": ["d1.1", "d1.2"]}]}
      ],
      "libraries": [{"id": "l-checksum", "files": ["src/checksum.c"], "needs_from_outside": []}]
    }"#;

    fn write_map(root: &Path) {
        let dir = root.join("migration/map");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("project-map.json"), MAP).unwrap();
    }

    fn drive(
        root: &Path,
        typed: &str,
        exec: &mut dyn FnMut(&[String]) -> Result<i32, String>,
    ) -> (Result<bool, String>, String) {
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
        assert_eq!(got, Ok(false));
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
        assert_eq!(got, Ok(false));
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

    /// After a map: Ask and Accept are offered; the shipped configuration
    /// is shown as proposed; Accept lists the programs and libraries and
    /// asks for each held choice without suggesting a file; once a tool
    /// exists the mode hands over to the cockpit.
    #[test]
    fn after_a_map_ask_and_accept_are_offered() {
        let p = project("mapped");
        write_map(&p.0);
        let map = read_map(&p.0).unwrap();
        assert!(map.proposed && map.held());
        let (got, said) = drive(&p.0, "2\nn\nx\n", &mut |_| Ok(0));
        assert_eq!(got, Ok(false));
        for words in [
            "The map shows 2 program(s) and 1 library.",
            "Its configuration (make) came with the project: proposed, not yours yet — state it \
             with `harness project map --adopt`, or write your own migration/map/config.toml.",
            "  1. Map the project again\n  2. Ask a model for advice\n  3. Accept a program\n",
            "Ask a model which file to keep in each held choice: its answer is advice shown \
             beside the choice; you still decide with Accept a program.",
            "It takes: one model call through the external hand-off",
            "It writes: migration/map/project-map.reply.json",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
        let target = shell_quote(&p.0.to_string_lossy());
        assert!(
            said.contains(&format!("It runs: harness project ask --target {target}\n")),
            "{said}"
        );
        // Accept: the program, its held choice, then the dialog; `y` runs
        // accept with the person's keep, and the tool it writes ends the
        // mode.
        let root = p.0.clone();
        let mut ran = Vec::new();
        let (got, said) = drive(&p.0, "3\n1\n2\ny\n", &mut |argv| {
            ran.push(argv.to_vec());
            let dir = config::tool_dir(&root, "t-unlzg");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("harness.toml"), "schema_version = 2\n").unwrap();
            Ok(0)
        });
        assert_eq!(got, Ok(true));
        for words in [
            "Which one?\n  1. t-unlzg — tools/unlzg.c (program)\n  2. l-checksum — \
             src/checksum.c (library)\n",
            "t-unlzg holds the choice d1: linking cannot tell its files apart, so keep which \
             one?\n  1. d1.1 src/decode.c\n  2. d1.2 src/mini.c\n",
            "Accept a program: check the map still matches the project",
            "It writes: migration/tools/<id>/harness.toml only (an id accepted before keeps its \
             ledger).",
        ] {
            assert!(said.contains(words), "{words}\n{said}");
        }
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
        assert!(!applies(&p.0, None), "a tool exists: the chooser opens it");
        assert_eq!(
            crate::chooser::choose(&p.0, None, None),
            Ok(Some("t-unlzg".to_string()))
        );
    }

    #[test]
    fn the_mode_applies_only_to_a_project_without_target() {
        let p = project("applies");
        assert!(applies(&p.0, None));
        assert!(!applies(&p.0, Some("t-a")));
        std::fs::write(p.0.join("harness.toml"), "schema_version = 1\n").unwrap();
        assert!(!applies(&p.0, None));
    }

    /// With no held choice Ask asks for the build.
    #[test]
    fn ask_without_a_held_choice_asks_for_the_build() {
        let p = project("ask-build");
        let map = MapView {
            guessed: true,
            configuration: "guessed".into(),
            ..MapView::default()
        };
        let argv = argv(Act::Ask, &p.0, Some(&map), "", &[]);
        assert_eq!(argv.last().map(String::as_str), Some("--build"));
        let words = dialog(Act::Ask, Some(&map), &argv);
        assert!(words.contains("propose a configuration"), "{words}");
        assert!(heading(&p.0, Some(&map)).contains("Its configuration is a guess"));
    }
}
