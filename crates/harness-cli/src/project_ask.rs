//! `harness project ask` (docs/PROJECT-MAP-DESIGN.md §3.4, §3.8): put the
//! project map's open questions — or, with `--build`, its build files — to
//! a model through the hand-off, and show the answers as the model's words.
//! Builds nothing and needs no sandbox; reads the map as `project map`
//! wrote it; writes only `migration/map/project-map.reply.json` (the
//! questions) or `migration/map/config.proposed.toml` (`--build`), under
//! the project lock. The prompts, the reply contracts and the files live in
//! [`harness_llm::projectask`].
//!
//! Every string from the project or the model is printed through
//! [`safe_line`] (newlines and tabs too); `--json` events carry them raw,
//! escaped by [`report::event`].

use crate::{out, report};
use anyhow::{bail, Result};
use harness_core::adopt;
use harness_core::config::LlmSection;
use harness_core::error::Error;
use harness_core::ledger::{WriterLock, MAP_DIR};
use harness_core::text::safe_line;
use harness_llm::projectask::{self, Answer, BuildFiles, Earlier, Item, MapView, SetItem};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The flags of `project ask`, as given.
pub(crate) struct AskArgs {
    pub(crate) target: PathBuf,
    pub(crate) build: bool,
    pub(crate) programs: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) allow_guessed: bool,
}

/// `--programs`' value: ids joined by commas, each a program id.
pub(crate) fn parse_programs(value: &str) -> std::result::Result<String, String> {
    let ids = split_programs(value);
    if ids.is_empty() {
        return Err("name at least one program id, like t-main".into());
    }
    for id in &ids {
        harness_core::config::check_tool_id(id)?;
        if !id.starts_with("t-") {
            return Err(format!(
                "`{}` is a library id: --programs takes program ids (t-…)",
                safe_line(id)
            ));
        }
    }
    Ok(value.to_string())
}

fn split_programs(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl AskArgs {
    /// The command that resumes this run: every flag given (never
    /// `--adopt` or `--json`), always the provider and model used, each value
    /// shell-quoted and attached.
    fn resume_command(&self, provider: &str, model: &str) -> String {
        let q = report::shell_quote;
        let mut cmd = format!(
            "harness project ask --target={}",
            q(&self.target.to_string_lossy())
        );
        if self.build {
            cmd.push_str(" --build");
        }
        if let Some(p) = &self.programs {
            cmd.push_str(&format!(" --programs={}", q(p)));
        }
        if self.allow_guessed {
            cmd.push_str(" --allow-guessed");
        }
        cmd.push_str(&format!(" --provider={} --model={}", q(provider), q(model)));
        cmd
    }
}

/// `harness project ask`: exit 0 with the answers shown and written; 1
/// refused, or awaiting a hand-off (the `awaiting` event and the resume
/// command).
pub(crate) fn cmd_ask(args: AskArgs) -> Result<u8> {
    if !args.target.is_dir() {
        bail!(
            "{} is not a folder: point --target at a mapped C project's folder",
            safe_line(&args.target.display().to_string())
        );
    }
    let root = args.target.canonicalize()?;
    adopt::check(&root)?;
    // Before the lock (which would make `migration/map/`): no map, nothing
    // to ask.
    if std::fs::symlink_metadata(root.join(projectask::MAP_FILE)).is_err() {
        bail!("no map written yet: run `harness project map`");
    }
    let defaults = LlmSection::default();
    let provider_name = args.provider.clone().unwrap_or(defaults.provider);
    let model = args.model.clone().unwrap_or(defaults.model);
    let max_tokens = defaults.max_tokens;
    let resume = args.resume_command(&provider_name, &model);

    let _lock = WriterLock::acquire_project(&root, "project ask")?;
    let map = projectask::read_map(&root)?;
    // The refusals (a guess, nothing open, an unknown program) come before
    // anything is made.
    let items = if args.build {
        None
    } else {
        let programs = args
            .programs
            .as_deref()
            .map(split_programs)
            .unwrap_or_default();
        Some(projectask::open_items(
            &root,
            &map,
            &programs,
            args.allow_guessed,
        )?)
    };
    let traces = traces_dir(&root)?;
    let provider = harness_llm::providers::resolve(&provider_name, &traces)?;
    let awaiting = match items {
        None => ask_build(&root, &map, &provider, &model, max_tokens, &traces)?,
        Some(items) => ask_questions(&items, &root, &map, &provider, &model, max_tokens, &traces)?,
    };
    match awaiting {
        None => Ok(0),
        Some(path) => {
            let e = Error::Awaiting {
                path: path.clone(),
                attempt: None,
            };
            eprintln!("{e:#}");
            eprintln!(
                "project ask: external provider mode — write each response beside its request \
                 under {} as {{\"text\": <the reply>, \"input_tokens\": 0, \"output_tokens\": \
                 0, \"stop_reason\": \"end_turn\"}}, and re-run with --model naming whoever \
                 answers (the model you ask, or yourself): {resume}",
                projectask::TRACES_DIR
            );
            report::event(&report::Awaiting {
                k: "awaiting",
                attempt: None,
                path: path.display().to_string(),
                resume,
                args: report::args_without_answer(),
                request_key: report::request_key_of(&path),
            });
            Err(e.into())
        }
    }
}

/// `migration/map/traces/`, made when absent; a link in place of it (or of
/// `migration/map/`) is refused.
fn traces_dir(root: &Path) -> Result<PathBuf> {
    for rel in [MAP_DIR, projectask::TRACES_DIR] {
        let dir = root.join(rel);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => bail!(
                "{rel} is not a folder (links are refused): remove it and run `harness project \
                 ask` again"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
            }
            Err(e) => return Err(Error::io(&dir, e).into()),
        }
    }
    Ok(root.join(projectask::TRACES_DIR))
}

// ---------- --build ----------

fn ask_build(
    root: &Path,
    map: &MapView,
    provider: &harness_llm::ResolvedProvider,
    model: &str,
    max_tokens: u32,
    traces: &Path,
) -> Result<Option<PathBuf>> {
    let files = projectask::read_build_files(
        root,
        &map.build_evidence.build_files,
        &harness_oracle::projectmap::evidence::is_build_file,
    );
    show_files(&files);
    if map.configuration.source == "guessed" {
        out(
            "project ask --build: the map's configuration is a guess; a proposal is allowed at \
             any time, since it only proposes"
                .into(),
        );
    }
    // Path flags must resolve inside the project, links followed, as every
    // other source of flags does.
    let check_path =
        |flag: &str| harness_oracle::projectmap::config::check_flags(root, &[flag.to_string()]);
    let outcome = projectask::run_build(
        provider,
        model,
        max_tokens,
        root,
        &files,
        traces,
        &check_path,
    )?;
    let Some(p) = outcome.proposal else {
        return Ok(outcome.awaiting);
    };
    out(format!(
        "project ask --build: the model's proposal ({}): configuration `{}` from {}, {} flag(s)",
        safe_line(model),
        safe_line(&p.name),
        p.from,
        p.flags.len()
    ));
    for f in &p.flags {
        out(format!(
            "  {} (cites {})",
            safe_line(&f.flag),
            if f.cites.is_empty() {
                "nothing".to_string()
            } else {
                safe_line(&f.cites.join(", "))
            }
        ));
    }
    for a in &p.assumptions {
        out(format!("  the model's assumption: {}", safe_line(a)));
    }
    report::event(&ProposalEvent {
        k: "project-proposal",
        path: projectask::PROPOSED_FILE,
        name: &p.name,
        from: &p.from,
        flags: p.flags.iter().map(|f| f.flag.as_str()).collect(),
        model,
        provider: &provider.profile,
    });
    out(format!(
        "project ask --build: wrote {} and changed nothing else: copy what you accept into \
         migration/map/config.toml and run `harness project map` again",
        projectask::PROPOSED_FILE
    ));
    Ok(None)
}

/// The files sent, and each left out with why.
fn show_files(files: &BuildFiles) {
    let sent: Vec<String> = files.sent.iter().map(|f| safe_line(&f.path)).collect();
    out(format!(
        "project ask --build: sending {} build file(s): {}",
        sent.len(),
        if sent.is_empty() {
            "none".to_string()
        } else {
            sent.join(", ")
        }
    ));
    for (path, why) in &files.left_out {
        out(format!(
            "project ask --build: not sent: {} ({why})",
            safe_line(path)
        ));
    }
}

#[derive(Serialize)]
struct ProposalEvent<'a> {
    k: &'static str,
    path: &'a str,
    name: &'a str,
    from: &'a str,
    flags: Vec<&'a str>,
    model: &'a str,
    provider: &'a str,
}

// ---------- the questions ----------

fn ask_questions(
    items: &[Item],
    root: &Path,
    map: &MapView,
    provider: &harness_llm::ResolvedProvider,
    model: &str,
    max_tokens: u32,
    traces: &Path,
) -> Result<Option<PathBuf>> {
    if map.configuration.source == "guessed" {
        out(
            "project ask: the configuration is a guess and --allow-guessed was given: a wrong \
             flag changes the closures, so these questions may be wrong"
                .into(),
        );
    }
    let indexes: Vec<&str> = items.iter().map(Item::index).collect();
    let batches = projectask::batches(items, model, max_tokens)?;
    out(format!(
        "project ask: asking {} ({}) about {} item(s) in {} call(s): {}",
        safe_line(&provider.profile),
        safe_line(model),
        items.len(),
        batches.len(),
        indexes.join(", ")
    ));
    for b in &batches {
        for item in b {
            if let Item::Set(s) = item {
                for d in &s.definers {
                    if let Some(cut) = &d.slice_cut {
                        out(format!(
                            "project ask: {} {}: {cut}",
                            d.index,
                            safe_line(&d.path)
                        ));
                    } else if let Some(why) = d.no_slice.filter(|w| w.starts_with("left out")) {
                        out(format!(
                            "project ask: {} {}: no slice sent ({why})",
                            d.index,
                            safe_line(&d.path)
                        ));
                    }
                }
            }
        }
    }
    let outcome =
        projectask::run_questions(provider, model, max_tokens, root, map, &batches, traces)?;
    match outcome.earlier {
        Earlier::OtherMap if outcome.wrote => out(
            "project ask: the reply file there was for another map, so it was replaced, unread"
                .to_string(),
        ),
        Earlier::Unreadable if outcome.wrote => out(
            "project ask: the reply file there could not be read, so it was replaced".to_string(),
        ),
        _ => {}
    }
    for (index, answer) in &outcome.answers {
        let Some(item) = items.iter().find(|i| i.index() == index) else {
            continue;
        };
        show_answer(item, answer, model, &provider.profile);
    }
    if !outcome.answers.is_empty() {
        out(
            "project ask: the model's words above are labels and advice only: nothing was built \
             or linked, and the choice of each held set stays yours (`harness project accept` \
             never reads the reply)"
                .into(),
        );
    }
    if outcome.wrote {
        out(format!(
            "project ask: wrote {} ({} answer(s) this run, under this map's digests)",
            projectask::REPLY_FILE,
            outcome.answers.len()
        ));
    }
    Ok(outcome.awaiting)
}

fn show_answer(item: &Item, answer: &Answer, model: &str, provider: &str) {
    let model_shown = safe_line(model);
    match (item, answer) {
        (
            Item::Program(p),
            Answer::Program {
                kind,
                name,
                purpose,
            },
        ) => {
            out(format!(
                "{} {} ({}): the model's label ({model_shown}): kind {kind}, name \"{}\"; \
                 purpose, in its words: {}",
                p.index,
                p.id,
                safe_line(&p.path),
                safe_line(name),
                safe_line(purpose)
            ));
            report::event(&AnswerEvent {
                k: "project-answer",
                item: &p.index,
                model,
                provider,
                program: Some(&p.id),
                path: Some(&p.path),
                kind: Some(kind),
                name: Some(name),
                purpose: Some(purpose),
                keep: None,
                keep_path: None,
                reason: None,
                linked_in: Vec::new(),
                not_linked_in: Vec::new(),
            });
        }
        (Item::Set(s), Answer::Set { keep, reason }) => {
            let symbols = s
                .symbols
                .iter()
                .map(|x| safe_line(x))
                .collect::<Vec<_>>()
                .join(", ");
            let holders = s.programs.join(", ");
            let (linked_in, not_linked_in, not_tried) = linked_split(s, keep);
            let keep_path = s
                .definers
                .iter()
                .find(|d| &d.index == keep)
                .map(|d| d.path.as_str());
            match keep_path {
                Some(path) => {
                    let mut line = format!(
                        "{} ({symbols}; held by {holders}): the model's advice ({model_shown}): \
                         keep {keep} {}, reason {reason}",
                        s.index,
                        safe_line(path)
                    );
                    if !linked_in.is_empty() {
                        line.push_str(&format!("; in {} that choice linked", linked_in.join(", ")));
                    }
                    if !not_linked_in.is_empty() {
                        line.push_str(&format!(
                            "; in {} it did not link",
                            not_linked_in.join(", ")
                        ));
                    }
                    if !not_tried.is_empty() {
                        line.push_str(&format!(
                            "; in {} the map did not link these choices (too many to try)",
                            not_tried.join(", ")
                        ));
                    }
                    out(line);
                }
                None => out(format!(
                    "{} ({symbols}; held by {holders}): the model could not decide (undecided, \
                     reason {reason}) ({model_shown}): the choice goes to you — keep one of {}",
                    s.index,
                    s.definers
                        .iter()
                        .map(|d| format!("{} {}", d.index, safe_line(&d.path)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
            report::event(&AnswerEvent {
                k: "project-answer",
                item: &s.index,
                model,
                provider,
                program: None,
                path: None,
                kind: None,
                name: None,
                purpose: None,
                keep: Some(keep),
                keep_path,
                reason: Some(reason),
                linked_in,
                not_linked_in,
            });
        }
        _ => {}
    }
}

/// The programs holding `s` where the advised definer's choice linked,
/// those where it did not, and those where the map did not try (the map's
/// own link results).
fn linked_split(s: &SetItem, keep: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    if keep == "undecided" {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut yes = Vec::new();
    let mut no = Vec::new();
    let mut untried = Vec::new();
    for l in &s.links {
        if !l.tried {
            untried.push(l.program.clone());
        } else if l.linked.iter().any(|d| d == keep) {
            yes.push(l.program.clone());
        } else {
            no.push(l.program.clone());
        }
    }
    yes.dedup();
    no.dedup();
    untried.dedup();
    (yes, no, untried)
}

#[derive(Serialize)]
struct AnswerEvent<'a> {
    k: &'static str,
    item: &'a str,
    model: &'a str,
    provider: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    program: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    purpose: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    linked_in: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    not_linked_in: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resume_command_carries_every_flag_and_the_model() {
        let args = AskArgs {
            target: PathBuf::from("-odd dir"),
            build: true,
            programs: None,
            provider: None,
            model: Some("m'1".into()),
            allow_guessed: true,
        };
        assert_eq!(
            args.resume_command("external", "m'1"),
            "harness project ask --target='-odd dir' --build --allow-guessed \
             --provider=external --model='m'\\''1'"
        );
    }

    #[test]
    fn programs_take_program_ids_only() {
        assert!(parse_programs("t-a,t-b").is_ok());
        assert!(parse_programs("l-lib").is_err());
        assert!(parse_programs("T-A").is_err());
        assert!(parse_programs(",").is_err());
    }
}
