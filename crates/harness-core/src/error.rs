//! Typed errors for the core crate (§10.2: thiserror at crate boundaries).

use std::path::{Path, PathBuf};

/// Errors produced by harness-core operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem I/O failure, with the path involved.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path being read or written.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A ledger file failed to parse.
    #[error("parse error in {path}: {message}")]
    Parse {
        /// File that failed to parse.
        path: PathBuf,
        /// Human-readable description.
        message: String,
    },
    /// A ledger file carries a schema version newer than this harness supports.
    #[error("{path} has schema_version {found}, but this harness supports up to {supported}; upgrade the harness")]
    SchemaTooNew {
        /// File with the newer schema.
        path: PathBuf,
        /// Version found in the file.
        found: u64,
        /// Maximum version this build understands.
        supported: u64,
    },
    /// The plan violates a structural invariant (missing unit, cycle, …).
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
    /// A perf results file (`migration/perf/program.json` or a unit's file
    /// under `migration/perf/units/`) could not be read or broke a rule:
    /// the message starts with the file's path.
    #[error("results file: {0}")]
    ResultsFile(String),
    /// A referenced unit does not exist in the plan.
    #[error("unknown unit `{0}`")]
    UnknownUnit(String),
    /// Any other invariant violation, described in prose.
    #[error("{0}")]
    Invariant(String),
    /// A recorded attempt, re-judged by HEAD on its RECORDED replies, did not
    /// reproduce its record (docs/REPLAY-DESIGN.md): a finding about the
    /// harness or the judge — its evidence is intact (an integrity failure
    /// is an [`Error::Invariant`] instead).
    #[error(
        "attempt {attempt} does not reproduce from its traces — the recorded evidence was left \
         untouched; {} difference(s): {}",
        .differences.len(),
        .differences.join("; ")
    )]
    Diverged {
        /// The attempt id.
        attempt: String,
        /// The strict-tier differences, in order.
        differences: Vec<String>,
        /// The outcome the re-judged trajectory reached (a supersession of a
        /// green record needs `loosening` unless this is not green).
        replayed_outcome: String,
    },
    /// Another harness command holds the ledger's writer lock
    /// (docs/CLI-HARDENING.md §1). `holder` is the record it wrote — `None`
    /// when it had not written one yet.
    #[error(
        "ledger is locked by another harness command ({}); {}",
        crate::ledger::describe_holder(.holder),
        if .holder.is_some() { "wait for it or stop it" } else { "retry" }
    )]
    Locked {
        /// The holder's record, when readable.
        holder: Option<crate::ledger::Holder>,
    },
    /// A ledger input is stale against the tree: the command refuses rather
    /// than act on evidence that no longer describes the sources.
    #[error("{subject} is stale: {hint}")]
    Stale {
        /// What is stale (`facts.jsonl`, `unit \`u\``, `finding f-…`).
        subject: String,
        /// What changed and what to run.
        hint: String,
    },
    /// The `external` hand-off wrote a request and waits for its response
    /// file (docs/SCHEMAS.md "Provider profiles"). `attempt` is the attempt
    /// being resumed, when one exists (the trajectory fills it in).
    #[error("awaiting response: {}", .path.display())]
    Awaiting {
        /// The response file the next run will pick up.
        path: PathBuf,
        /// The attempt id, when the hand-off happened inside a trajectory.
        attempt: Option<String>,
    },
    /// The harness was cancelled (SIGINT/SIGTERM/SIGHUP) while a child was
    /// running: the child was killed and its end is NOT evidence
    /// (docs/CLI-HARDENING.md §3).
    #[error("interrupted: the harness was cancelled while a child process was running")]
    Interrupted,
    /// `harness migrate --answer` was refused before anything was written:
    /// the attempt it would resume does not wait on that request
    /// (docs/CHAT-PANE-DESIGN.md §4.3).
    #[error("--answer refused: {why}")]
    AnswerRefused {
        /// Why.
        why: String,
    },
    /// `harness migrate --answer` ran, but the attempt never asked for the
    /// answered hand-off: it asked for another request first, or finished
    /// (docs/CHAT-PANE-DESIGN.md §4.3). Nothing was written with the answer.
    #[error("answer unused: the run never asked for hand-off {key} ({why})")]
    AnswerUnused {
        /// The `--answer-key`.
        key: String,
        /// What the run did instead.
        why: String,
    },
    /// The ledger at `root` was made elsewhere: its root is not listed in
    /// this computer's adoption file, or its token is missing or different
    /// (docs/PROJECT-MAP-DESIGN.md §3.7). Nothing was read beyond the plan's
    /// unit count. Its words are the command line's
    /// ([`crate::adopt::Way::Command`]); the cockpit and harness-mcp say
    /// their own ([`Error::words_for`]).
    #[error(
        "{}",
        crate::adopt::refusal(.root, *.units, *.verified, crate::adopt::Way::Command)
    )]
    NotAdopted {
        /// The canonical root refused.
        root: PathBuf,
        /// Units its plan claims.
        units: usize,
        /// Of which verified or merged.
        verified: usize,
    },
    /// `--adopt` was refused: the root's `migration/` is not the harness's
    /// (it holds none of its files, or something outside the ledger's fixed
    /// names), so it is the project's own.
    #[error(
        "{}/migration is the project's own folder, and the harness needs that name for its \
         ledger: rename the folder, or run the harness on a copy of the project with it renamed",
        root.display()
    )]
    ForeignMigration {
        /// The project root.
        root: PathBuf,
    },
    /// `--target`/`--tool` name no target: a bad tool id, a tool the project
    /// does not have, or several tools and no `--tool`
    /// (docs/PROJECT-MAP-DESIGN.md §3.7). `why` is the one sentence.
    #[error("{why}")]
    NoTarget {
        /// What is wrong and what to do.
        why: String,
    },
}

impl Error {
    /// Convenience constructor for [`Error::Io`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// True when this is an I/O error for a file that does not exist —
    /// callers use it to distinguish "not created yet" from real failures
    /// that must propagate (e.g. [`Error::SchemaTooNew`]).
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound
        )
    }

    /// Convenience constructor for [`Error::Parse`].
    pub fn parse(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Error::Parse {
            path: path.into(),
            message: message.into(),
        }
    }

    /// This error in the words `way`'s reader needs: a not-adopted refusal
    /// tells each reader only its own way to adopt; any other error reads
    /// as it is.
    pub fn words_for(&self, way: crate::adopt::Way) -> String {
        match self {
            Error::NotAdopted {
                root,
                units,
                verified,
            } => crate::adopt::refusal(root, *units, *verified, way),
            other => other.to_string(),
        }
    }

    /// `--target <root>` names no target: the one sentence the command
    /// line, the cockpit and harness-mcp all say.
    pub fn no_target_here(root: &Path) -> Error {
        Error::NoTarget {
            why: format!(
                "{} is not a harness target (no harness.toml, and no mapped tool under \
                 migration/tools/); point --target at a folder that holds a harness.toml",
                root.display()
            ),
        }
    }

    /// `--tool` on a project that has no mapped tools.
    pub fn no_tools_to_pick(root: &Path) -> Error {
        Error::NoTarget {
            why: format!(
                "{}: this project has no mapped tools; drop --tool (its harness.toml is the \
                 target)",
                root.display()
            ),
        }
    }

    /// An error from finding or opening `--target <root> [--tool <tool>]`
    /// put in the one sentence every reader says for it: no target here,
    /// or `--tool` on a project without tools. Any other error is kept.
    pub fn opening(self, root: &Path, tool: Option<&str>) -> Error {
        let no_tools = crate::config::mapped_tools(root).is_empty();
        let has_config = std::fs::symlink_metadata(root.join(crate::config::CONFIG_FILE)).is_ok();
        match (&self, tool) {
            (Error::NoTarget { .. }, Some(_)) if no_tools && has_config => {
                Error::no_tools_to_pick(root)
            }
            (Error::NoTarget { .. }, Some(_)) if no_tools => Error::no_target_here(root),
            (_, None) if no_tools && !has_config && self.is_not_found() => {
                Error::no_target_here(root)
            }
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_target_reads_one_way() {
        let root = std::env::temp_dir().join(format!(
            "ruharness-error-no-target-{}-{}",
            std::process::id(),
            crate::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let missing = Error::io(
            root.join("harness.toml"),
            std::io::Error::from(std::io::ErrorKind::NotFound),
        );
        assert_eq!(
            missing.opening(&root, None).to_string(),
            Error::no_target_here(&root).to_string()
        );
        std::fs::write(root.join("harness.toml"), "").unwrap();
        let no_tool = Error::NoTarget {
            why: "has no mapped tool t-x (it has none)".into(),
        };
        assert_eq!(
            no_tool.opening(&root, Some("t-x")).to_string(),
            format!(
                "{}: this project has no mapped tools; drop --tool (its harness.toml is the \
                 target)",
                root.display()
            )
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
