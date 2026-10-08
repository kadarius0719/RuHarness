//! The cockpit's tool chooser (docs/PROJECT-MAP-DESIGN.md §3.7): opened on
//! a project, the cockpit finds its target as the command line does — the
//! tool `--tool` names, else the root's `harness.toml`, else the project's
//! only mapped tool — and, where the command line refuses because the
//! project has several tools, it lists them for the person to pick, once,
//! before the terminal is taken.

use harness_core::config::{self, Found};
use std::io::{BufRead, Write};
use std::path::Path;

/// The target to open at `root`: `Ok(None)` for the root's own
/// `harness.toml`, `Ok(Some(id))` for a mapped tool. With several tools and
/// no `tool`, `ask` (a terminal) lists them for the person to pick; without
/// one the refusal names them. `Err` is one sentence.
pub fn choose(
    root: &Path,
    tool: Option<&str>,
    ask: Option<(&mut dyn BufRead, &mut dyn Write)>,
) -> Result<Option<String>, String> {
    match config::find_target(root, tool) {
        Ok(Found::Tool(id)) => Ok(Some(id)),
        Ok(Found::Root) if root.join(config::CONFIG_FILE).is_file() => Ok(None),
        // The command line's own sentences (harness-core's): no target here,
        // `--tool` on a project without tools.
        Ok(Found::Root) => Err(harness_core::Error::no_target_here(root).to_string()),
        Err(e) => {
            let tools = config::mapped_tools(root);
            match ask {
                Some((input, output)) if tool.is_none() && tools.len() > 1 => {
                    pick(root, &tools, input, output)
                }
                _ => Err(e.opening(root, tool).to_string()),
            }
        }
    }
}

/// List `tools` and read the person's pick: a number from the list, or a
/// tool's id. Anything else opens nothing.
fn pick(
    root: &Path,
    tools: &[String],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<Option<String>, String> {
    let mut text = format!(
        "harness-tui: {} has {} mapped tools:\n",
        root.display(),
        tools.len()
    );
    for (i, id) in tools.iter().enumerate() {
        text.push_str(&format!("  {}. {id}\n", i + 1));
    }
    text.push_str(&format!(
        "Open which one? Type its number (1-{}) and Enter; anything else opens nothing: ",
        tools.len()
    ));
    output
        .write_all(text.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|e| format!("the terminal: {e}"))?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| format!("the terminal: {e}"))?;
    let answer = line.trim();
    let chosen = answer
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| tools.get(i))
        .or_else(|| tools.iter().find(|id| id.as_str() == answer));
    match chosen {
        Some(id) => Ok(Some(id.clone())),
        None => Err(format!(
            "no tool picked; start again with --tool <id> (one of {})",
            tools.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str, tools: &[&str]) -> crate::testutil::TmpDir {
        let tmp = crate::testutil::TmpDir::new(&format!("chooser-{tag}"));
        for id in tools {
            let dir = config::tool_dir(&tmp.0, id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(config::CONFIG_FILE), "schema_version = 2\n").unwrap();
        }
        tmp
    }

    fn ask_with(
        root: &Path,
        tool: Option<&str>,
        typed: &str,
    ) -> (Result<Option<String>, String>, String) {
        let mut input = std::io::Cursor::new(typed.as_bytes().to_vec());
        let mut output = Vec::new();
        let got = choose(root, tool, Some((&mut input, &mut output)));
        (got, String::from_utf8(output).unwrap())
    }

    #[test]
    fn one_tool_opens_without_asking() {
        let p = project("one", &["t-lz4"]);
        assert_eq!(choose(&p.0, None, None), Ok(Some("t-lz4".into())));
        let (got, said) = ask_with(&p.0, None, "");
        assert_eq!(got, Ok(Some("t-lz4".into())));
        assert!(said.is_empty(), "{said}");
    }

    #[test]
    fn several_tools_are_listed_for_the_person_to_pick() {
        let p = project("several", &["t-b", "t-a"]);
        let (got, said) = ask_with(&p.0, None, "2\n");
        assert_eq!(got, Ok(Some("t-b".into())));
        assert!(
            said.contains("2 mapped tools") && said.contains("  1. t-a\n  2. t-b\n"),
            "{said}"
        );
        let (got, _) = ask_with(&p.0, None, "t-a\n");
        assert_eq!(got, Ok(Some("t-a".into())));
        let (got, _) = ask_with(&p.0, None, "\n");
        assert!(got.unwrap_err().contains("no tool picked"));
        // Away from a terminal: refused, the tools named.
        let err = choose(&p.0, None, None).unwrap_err();
        assert!(err.contains("t-a, t-b") && err.contains("--tool"), "{err}");
        // --tool picks without asking.
        let (got, said) = ask_with(&p.0, Some("t-b"), "");
        assert_eq!(got, Ok(Some("t-b".into())));
        assert!(said.is_empty());
        let (got, _) = ask_with(&p.0, Some("t-c"), "1\n");
        assert!(got.unwrap_err().contains("no mapped tool t-c"));
    }

    #[test]
    fn a_root_harness_toml_wins_without_tool() {
        let p = project("root", &["t-a", "t-b"]);
        std::fs::write(p.0.join(config::CONFIG_FILE), "schema_version = 1\n").unwrap();
        assert_eq!(choose(&p.0, None, None), Ok(None));
        assert_eq!(choose(&p.0, Some("t-a"), None), Ok(Some("t-a".into())));
        let bare = project("bare", &[]);
        assert_eq!(
            choose(&bare.0, None, None).unwrap_err(),
            harness_core::Error::no_target_here(&bare.0).to_string()
        );
    }

    /// `--tool` on a project with a harness.toml and no tools: the command
    /// line's sentence, which says to drop it.
    #[test]
    fn a_tool_on_a_project_without_tools_is_told_to_drop_it() {
        let p = project("no-tools", &[]);
        std::fs::write(p.0.join(config::CONFIG_FILE), "schema_version = 1\n").unwrap();
        assert_eq!(
            choose(&p.0, Some("t-a"), None).unwrap_err(),
            harness_core::Error::no_tools_to_pick(&p.0).to_string()
        );
    }
}
