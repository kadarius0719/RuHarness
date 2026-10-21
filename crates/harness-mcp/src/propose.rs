//! The chat proposes the person's features and workloads
//! (docs/COCKPIT-WRAPPER-DESIGN.md §9.6), in cockpit mode only:
//!
//! - `harness_features_propose {model, text}` checks `text` with the
//!   features file's own loader (`harness_core::features::parse`, what
//!   `harness features save` runs) and writes it, under a header naming it
//!   as the model's proposal, to `features/features.proposed.toml` beside
//!   the person's file;
//! - `harness_workloads_propose {model, text}` the same for the workloads
//!   file (`harness_core::perf::workloads::parse`), to
//!   `perf/workloads.proposed.toml`.
//!
//! Each writes that one file and nothing else, atomically. Never the
//! person's own file: the cockpit opens the proposal in their editor as
//! the draft, and their save writes it. A proposal that does not validate
//! is refused with the loader's message and nothing is written; the chat
//! fixes it and proposes again. The cockpit's permission check refuses the
//! call while a draft of the person's is kept (the draft slot is theirs),
//! and passes it only under the chat's own model.

use crate::tools::{Kind, Param, Tool};
use harness_core::features;
use harness_core::ledger::Ledger;
use harness_core::perf::workloads;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Which of the person's files a proposal is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// `features/features.toml`.
    Features,
    /// `perf/workloads.toml`.
    Workloads,
}

impl Which {
    /// The tool that proposes it.
    pub fn of_tool(name: &str) -> Option<Which> {
        match name {
            "harness_features_propose" => Some(Which::Features),
            "harness_workloads_propose" => Some(Which::Workloads),
            _ => None,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Which::Features => "features",
            Which::Workloads => "workloads",
        }
    }

    /// The folder under the ledger that holds both files.
    fn folder(self) -> &'static str {
        match self {
            Which::Features => features::FEATURES_DIR,
            Which::Workloads => harness_core::perf::PERF_DIR,
        }
    }

    fn file(self, ledger: &Ledger) -> PathBuf {
        match self {
            Which::Features => features::features_path(ledger),
            Which::Workloads => workloads::workloads_path(ledger),
        }
    }

    fn proposed(self, ledger: &Ledger) -> PathBuf {
        match self {
            Which::Features => features::proposed_path(ledger),
            Which::Workloads => workloads::proposed_path(ledger),
        }
    }

    /// The file cap: the person's file's own, which the cockpit reads the
    /// proposal with.
    fn cap(self) -> u64 {
        match self {
            Which::Features => features::MAX_FEATURES_BYTES,
            Which::Workloads => workloads::MAX_WORKLOADS_BYTES,
        }
    }

    /// The cockpit's item that opens the proposal.
    fn opens_with(self) -> &'static str {
        match self {
            Which::Features => "Write your features file",
            Which::Workloads => "Write your workloads file",
        }
    }
}

/// The rule both tools state (and the brief repeats).
pub const PROPOSE_RULE: &str = "You PROPOSE, the person writes: the tool writes your text to a \
proposal file beside the person's own and nothing else; the cockpit opens it in the person's \
editor as their draft, and their save writes the file. Send the whole file in its syntax; a \
text that does not validate is refused with the reason and nothing is written — fix it and \
propose again. Then tell the person, one sentence per scenario (or workload), what each checks \
and why you chose it.";

fn param(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        kind: Kind::Str,
        required: true,
        description,
        one_of: None,
    }
}

/// `harness_features_propose` and `harness_workloads_propose` (cockpit
/// mode).
pub fn tools() -> Vec<Tool> {
    let untrusted = crate::tools::UNTRUSTED_RULE;
    let output: Vec<(&'static str, &'static [&'static str])> = vec![
        ("kind", &["string"]),
        ("file", &["string"]),
        ("features", &["integer"]),
        ("scenarios", &["integer"]),
        ("workloads", &["integer"]),
        ("next", &["string"]),
        ("error", &["object", "null"]),
    ];
    vec![
        Tool {
            name: "harness_features_propose",
            title: "Propose the person's features file",
            description: format!(
                "Writes your proposal of the person's features file (features.toml's syntax: \
                 schema_version = 1, [[feature]] tables with id and name, [[scenario]] tables \
                 with feature, id, args and an optional input) to \
                 features/features.proposed.toml beside it, after checking it with the \
                 features file's own loader — the check `harness features save` runs. Never \
                 the features file itself. {PROPOSE_RULE} {untrusted}"
            ),
            read_only: false,
            destructive: false,
            open_world: false,
            params: vec![
                param(
                    "model",
                    "Your own model id: the proposal's header names it (the cockpit checks it \
                     is the chat's).",
                ),
                param(
                    "text",
                    "The whole features file you propose, in features.toml's syntax; a \
                     one-line comment above each [[scenario]] may say why you chose it. At \
                     most 64 KiB.",
                ),
            ],
            output: output.clone(),
        },
        Tool {
            name: "harness_workloads_propose",
            title: "Propose the person's workloads file",
            description: format!(
                "Writes your proposal of the person's workloads file (workloads.toml's syntax: \
                 schema_version = 1, [[workload]] tables with id, args, an optional input file \
                 in the project and optional runs) to perf/workloads.proposed.toml beside it, \
                 after checking it with the workloads file's own loader — the check `harness \
                 perf save` runs. Never the workloads file itself. {PROPOSE_RULE} {untrusted}"
            ),
            read_only: false,
            destructive: false,
            open_world: false,
            params: vec![
                param(
                    "model",
                    "Your own model id: the proposal's header names it (the cockpit checks it \
                     is the chat's).",
                ),
                param(
                    "text",
                    "The whole workloads file you propose, in workloads.toml's syntax; a \
                     one-line comment above each [[workload]] may say why you chose it. At \
                     most 64 KiB.",
                ),
            ],
            output,
        },
    ]
}

/// The comment the proposal starts with: whose it is, when, and what
/// the person does with it.
pub fn header(which: Which, model: &str, date: &str) -> String {
    let noun = which.noun();
    format!(
        "# The chat's proposal for your {noun} file, written by {model} on {date}.\n\
         # Nothing reads this file. In the cockpit, {} (or, once the file exists,\n\
         # Edit from the chat's proposal…) opens it as your draft: read it, change what you\n\
         # like, and save. Saving writes {}.toml and removes this proposal.\n\n",
        which.opens_with(),
        noun
    )
}

/// The proposal `text`, checked with the person's file's own loader and
/// written under its header to the proposal file in `ledger` (whose folder
/// is `ledger_rel` under `root`) — that file only, atomically, never
/// through a link. `Err` says why nothing was written.
pub fn propose(
    root: &Path,
    ledger: &Ledger,
    ledger_rel: &str,
    which: Which,
    model: &str,
    text: &str,
) -> Result<Value, String> {
    let noun = which.noun();
    if !crate::fence::valid_model(model) {
        return Err(
            "`model` must be your own model id (letters, digits, `.`, `-`, `_`, `:`, `/`, `@`, \
             `[`, `]`; at most 128)"
                .into(),
        );
    }
    if text.trim().is_empty() {
        return Err(format!(
            "`text` must be the whole {noun} file you propose: it is empty"
        ));
    }
    let date = harness_core::ledger::now_utc();
    let mut whole = header(which, model, date.get(..10).unwrap_or(&date));
    whole.push_str(text);
    if !whole.ends_with('\n') {
        whole.push('\n');
    }
    if whole.len() as u64 > which.cap() {
        return Err(format!(
            "`text` is over the {} KiB a {noun} file may hold (with the proposal's header): \
             propose a shorter file; nothing was written",
            which.cap() >> 10
        ));
    }
    // The loader the person's save runs, on the text as sent (its line
    // numbers are the chat's own).
    let counts = match which {
        Which::Features => {
            let f = features::parse(text, &which.file(ledger), ledger_rel).map_err(|e| {
                let words = match e {
                    harness_core::Error::InvalidPlan(m) => m,
                    other => other.to_string(),
                };
                not_valid(noun, &words)
            })?;
            json!({"features": f.features.len(), "scenarios": f.scenarios.len()})
        }
        Which::Workloads => {
            let w = workloads::parse(text, &which.file(ledger)).map_err(|e| {
                let words = match e {
                    workloads::ParseError::Rule(r) => r.to_string(),
                    workloads::ParseError::TooNew(e) => e.to_string(),
                };
                not_valid(noun, &words)
            })?;
            json!({"workloads": w.workloads.len()})
        }
    };
    // No link on the way: the ledger's folders, then the proposal's own.
    let dir = real_dir(root, ledger, which.folder())?;
    let path = which.proposed(ledger);
    let shown = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .display()
        .to_string();
    match std::fs::symlink_metadata(&path) {
        Ok(m) if !m.file_type().is_file() => {
            return Err(format!(
                "{shown} is not a regular file (a link, or a folder): nothing was written — the \
                 person removes it first"
            ))
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "{shown} cannot be inspected ({e}): nothing was written"
            ))
        }
    }
    debug_assert!(path.starts_with(&dir));
    harness_core::ledger::write_atomic(&path, whole.as_bytes())
        .map_err(|e| format!("{shown} could not be written: {e}"))?;
    let mut out = json!({
        "kind": noun,
        "file": shown,
        "next": format!(
            "the proposal is written beside the person's {noun} file and nothing else changed: \
             tell the person, one sentence per {}, what each checks and why you chose it; they \
             open it with {} in the cockpit, change what they like, and save",
            if which == Which::Features { "scenario" } else { "workload" },
            which.opens_with()
        ),
    });
    if let (Some(o), Some(c)) = (out.as_object_mut(), counts.as_object()) {
        for (k, v) in c {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok(out)
}

fn not_valid(noun: &str, words: &str) -> String {
    format!(
        "the proposal is not a valid {noun} file: {words}; nothing was written — fix it and \
         propose again"
    )
}

/// The ledger's folder `folder` under `root`, every step a real folder
/// (made when missing, as `harness features save` makes it): a link or a
/// file on the way is refused in words.
fn real_dir(root: &Path, ledger: &Ledger, folder: &str) -> Result<PathBuf, String> {
    let under = ledger.dir();
    let rel = under.strip_prefix(root).map_err(|_| {
        format!(
            "the ledger {} is not under the project: nothing was written",
            under.display()
        )
    })?;
    let mut cur = root.to_path_buf();
    for part in rel.components().chain(Path::new(folder).components()) {
        cur.push(part);
        let shown = cur.strip_prefix(root).unwrap_or(&cur).display().to_string();
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!(
                    "{shown} is a link: nothing was written — the person replaces it with a \
                     real folder outside the cockpit"
                ))
            }
            Ok(m) if !m.is_dir() => {
                return Err(format!("{shown} is not a folder: nothing was written"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur)
                    .map_err(|e| format!("{shown} could not be made ({e}): nothing was written"))?;
            }
            Err(e) => {
                return Err(format!(
                    "{shown} cannot be inspected ({e}): nothing was written"
                ))
            }
        }
    }
    Ok(cur)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A folder-form target with a ledger folder, adopted here.
    pub(crate) fn target(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mcp-propose-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("migration")).unwrap();
        std::fs::write(
            root.join("harness.toml"),
            "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        harness_core::adopt::testing::adopt(&root);
        root.canonicalize().unwrap()
    }

    pub(crate) const FEATURES: &str = "schema_version = 1\n\n[[feature]]\nid = \"gzip\"\n\
name = \"Compress to gzip\"\n\n[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\n\
# Compresses English text: what the program is for.\n[[scenario]]\nfeature = \"gzip\"\n\
id = \"text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"sample:text\"\n\n\
# The help: the options it says it has.\n[[scenario]]\nfeature = \"help\"\nid = \"flag\"\n\
args = [\"-h\"]\n";

    pub(crate) const WORKLOADS: &str = "schema_version = 1\n\n\
# A large C file: long enough to time.\n[[workload]]\nid = \"big-c\"\n\
args = [\"-c\", \"{input}\"]\ninput = \"src/big.c\"\n";

    /// A valid proposal is written beside the file, under its header,
    /// and nothing else: the person's file is not made.
    #[test]
    fn a_valid_proposal_is_written_with_its_header_and_nothing_else() {
        let root = target("ok");
        let ledger = Ledger::new(&root);
        let v = propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "claude-opus-5-5",
            FEATURES,
        )
        .unwrap();
        assert_eq!(v["file"], "migration/features/features.proposed.toml");
        assert_eq!(v["features"], 2);
        assert_eq!(v["scenarios"], 2);
        let text = std::fs::read_to_string(features::proposed_path(&ledger)).unwrap();
        assert!(
            text.starts_with(
                "# The chat's proposal for your features file, written by claude-opus-5-5 on 20"
            ),
            "{text}"
        );
        assert!(text.ends_with(FEATURES), "{text}");
        // The whole file, header and all, is still a valid features file.
        features::parse(&text, Path::new("features.toml"), "migration").unwrap();
        assert!(!features::features_path(&ledger).exists());
        let names: Vec<_> = std::fs::read_dir(features::features_dir(&ledger))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [features::PROPOSED_FILE], "only the proposal");

        let v = propose(
            &root,
            &ledger,
            "migration",
            Which::Workloads,
            "claude-opus-5-5",
            WORKLOADS,
        )
        .unwrap();
        assert_eq!(v["file"], "migration/perf/workloads.proposed.toml");
        assert_eq!(v["workloads"], 1);
        let text = std::fs::read_to_string(workloads::proposed_path(&ledger)).unwrap();
        assert!(text.contains("your workloads file"), "{text}");
        workloads::parse(&text, Path::new("workloads.toml")).unwrap();
        assert!(!workloads::workloads_path(&ledger).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Invalid text is refused with the loader's own words; nothing is
    /// written.
    #[test]
    fn invalid_text_is_refused_with_the_loaders_message() {
        let root = target("bad");
        let ledger = Ledger::new(&root);
        let bad = FEATURES.replace("\"-h\"", "\"a/b\"");
        let why = propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "claude-opus-5-5",
            &bad,
        )
        .unwrap_err();
        assert!(why.contains("an argument cannot contain \"/\""), "{why}");
        assert!(why.contains("nothing was written"), "{why}");
        assert!(!features::proposed_path(&ledger).exists());
        let why = propose(
            &root,
            &ledger,
            "migration",
            Which::Workloads,
            "claude-opus-5-5",
            "schema_version = 1\n[[workload]]\nid = \"x\"\nargs = [\"{input}\"]\n",
        )
        .unwrap_err();
        assert!(why.contains("line"), "{why}");
        assert!(why.contains("{input}"), "{why}");
        assert!(!workloads::proposed_path(&ledger).exists());
        // Empty, over the cap, a bad model: refused, nothing written.
        assert!(propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "claude-opus-5-5",
            " "
        )
        .unwrap_err()
        .contains("empty"));
        let big = format!("{FEATURES}{}", "# x\n".repeat(20_000));
        assert!(propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "claude-opus-5-5",
            &big
        )
        .unwrap_err()
        .contains("over the 64 KiB"));
        assert!(propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "no model!",
            FEATURES
        )
        .unwrap_err()
        .contains("`model`"));
        assert!(!features::proposed_path(&ledger).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A link on the way — the features folder, or the proposal itself —
    /// is refused, and nothing is written through it.
    #[test]
    fn a_link_on_the_way_is_refused() {
        let root = target("link");
        let ledger = Ledger::new(&root);
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, features::features_dir(&ledger)).unwrap();
        let why = propose(
            &root,
            &ledger,
            "migration",
            Which::Features,
            "claude-opus-5-5",
            FEATURES,
        )
        .unwrap_err();
        assert!(why.contains("migration/features is a link"), "{why}");
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
        std::fs::create_dir_all(harness_core::perf::perf_dir(&ledger)).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("w.toml"), workloads::proposed_path(&ledger))
            .unwrap();
        let why = propose(
            &root,
            &ledger,
            "migration",
            Which::Workloads,
            "claude-opus-5-5",
            WORKLOADS,
        )
        .unwrap_err();
        assert!(why.contains("is not a regular file"), "{why}");
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
