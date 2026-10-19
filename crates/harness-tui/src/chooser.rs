//! The cockpit's target (docs/PROJECT-MAP-DESIGN.md §3.7): opened on a
//! project, the cockpit finds its target as the command line does — the
//! tool `--tool` names, else the root's `harness.toml`, else the project's
//! only mapped tool. A project of several tools (and one with no tool yet,
//! or a map) opens on the Project view instead, whose rows list the tools to
//! open ([`crate::project::applies`]); the command line's refusal, which
//! names them, is what reaches a caller that is not the cockpit.

use harness_core::config::{self, Found};
use std::path::Path;

/// The target to open at `root`: `Ok(None)` for the root's own
/// `harness.toml`, `Ok(Some(id))` for a mapped tool. `Err` is one sentence,
/// the command line's.
pub fn choose(root: &Path, tool: Option<&str>) -> Result<Option<String>, String> {
    match config::find_target(root, tool) {
        Ok(Found::Tool(id)) => Ok(Some(id)),
        Ok(Found::Root) if root.join(config::CONFIG_FILE).is_file() => Ok(None),
        // The command line's own sentences (harness-core's): no target here,
        // `--tool` on a project without tools.
        Ok(Found::Root) => Err(harness_core::Error::no_target_here(root).to_string()),
        Err(e) => Err(e.opening(root, tool).to_string()),
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

    #[test]
    fn one_tool_opens_without_asking() {
        let p = project("one", &["t-lz4"]);
        assert_eq!(choose(&p.0, None), Ok(Some("t-lz4".into())));
        assert!(!crate::project::applies(&p.0, None));
    }

    /// Several tools: the Project view lists them (no text prompt before
    /// the screen); `--tool` picks one, and the refusal names them.
    #[test]
    fn several_tools_open_on_the_project_view() {
        let p = project("several", &["t-b", "t-a"]);
        assert!(crate::project::applies(&p.0, None));
        let err = choose(&p.0, None).unwrap_err();
        assert!(err.contains("t-a, t-b") && err.contains("--tool"), "{err}");
        assert_eq!(choose(&p.0, Some("t-b")), Ok(Some("t-b".into())));
        assert!(choose(&p.0, Some("t-c"))
            .unwrap_err()
            .contains("no mapped tool t-c"));
    }

    #[test]
    fn a_root_harness_toml_wins_without_tool() {
        let p = project("root", &["t-a", "t-b"]);
        std::fs::write(p.0.join(config::CONFIG_FILE), "schema_version = 1\n").unwrap();
        assert_eq!(choose(&p.0, None), Ok(None));
        assert_eq!(choose(&p.0, Some("t-a")), Ok(Some("t-a".into())));
        let bare = project("bare", &[]);
        assert_eq!(
            choose(&bare.0, None).unwrap_err(),
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
            choose(&p.0, Some("t-a")).unwrap_err(),
            harness_core::Error::no_tools_to_pick(&p.0).to_string()
        );
    }
}
