//! The activity panel's plain-language narration
//! (docs/COCKPIT-WRAPPER-DESIGN.md §6.1): a stateful [`Narrator`] fed the
//! run's argv and every event of its `ruharness-events` stream says what the
//! running command is doing now, and — once it is reaped — how it ended.
//! Every value it quotes is untrusted: the view filters it for display.

use crate::events::Event;
use std::ffi::OsString;
use std::time::Duration;

/// A check's name in words (the raw name stays in the details).
pub fn check_words(name: &str) -> String {
    match name {
        "symbol-set" => "same exports".into(),
        "capabilities" => "allowed calls only".into(),
        "driver-shape" => "driver shape".into(),
        "rust-build" => "the Rust builds".into(),
        "differential-driver" => "same outputs as C".into(),
        "sanitizers" => "sanitizers".into(),
        "boundary" => "boundary calls".into(),
        n if n.starts_with("whole-program") => "whole program".into(),
        n => n.into(),
    }
}

/// A turn's result in words.
pub fn result_words(result: &str) -> String {
    match result {
        "green" => "passed".into(),
        "format" => "the reply was not in the expected form".into(),
        "check" => "a check failed".into(),
        "build" => "the Rust did not build".into(),
        "oracle" => "the outputs differ from C".into(),
        "crash-timeout" => "it crashed or timed out".into(),
        "truncated" => "the reply was cut off".into(),
        "blocked" => "the safety scan refused it".into(),
        other => other.into(),
    }
}

/// `41 s`, `2 min 5 s`.
pub fn elapsed_words(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {} s", s / 60, s % 60)
    }
}

/// How the run ended, for the idle line and the `[Try again]` offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// Exit 0.
    Done,
    /// Exit 10: the oracle said red.
    Red,
    /// Paused on a hand-off.
    Paused,
    /// Refused because another command held the writer lock.
    Locked,
    /// Refused (exit 1) for another reason.
    Refused,
    /// Stopped by a signal, or interrupted.
    Stopped,
    /// Anything else.
    Other,
}

/// See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narrator {
    /// The act in words ("Re-check u-lib").
    pub label: String,
    subcommand: String,
    model: Option<String>,
    steer: bool,
    step: String,
    checks: Vec<(String, bool)>,
    verdict: Option<String>,
    awaited: bool,
    error: Option<(String, String)>,
    last_message: Option<String>,
    locked_by: Option<String>,
}

fn arg_value(argv: &[OsString], flag: &str) -> Option<String> {
    argv.iter().find_map(|a| {
        a.to_string_lossy()
            .strip_prefix(flag)
            .and_then(|rest| rest.strip_prefix('='))
            .map(str::to_string)
    })
}

impl Narrator {
    /// A narrator for the run of `argv` (`harness --json <subcommand> …`).
    pub fn new(label: &str, argv: &[OsString]) -> Narrator {
        let subcommand = argv
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .find(|a| !a.starts_with('-'))
            .unwrap_or_default();
        let step = match subcommand.as_str() {
            "verify" | "promote" => {
                "Running the oracle… (the checks are reported when it finishes)".into()
            }
            _ => "Starting…".into(),
        };
        Narrator {
            label: label.to_string(),
            subcommand,
            model: arg_value(argv, "--model"),
            steer: argv
                .iter()
                .any(|a| a.to_string_lossy().starts_with("--steer=")),
            step,
            checks: Vec::new(),
            verdict: None,
            awaited: false,
            error: None,
            last_message: None,
            locked_by: None,
        }
    }

    /// What the command is doing now.
    pub fn step(&self) -> &str {
        &self.step
    }

    /// The checks this run reported, in order.
    pub fn checks(&self) -> &[(String, bool)] {
        &self.checks
    }

    /// One event of the run.
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::Header { .. } | Event::Result { .. } | Event::Other { .. } => {}
            Event::NotJson { line } => self.step = line.clone(),
            Event::Message { text } => {
                self.step = text.clone();
                self.last_message = Some(text.clone());
            }
            Event::TurnStart { index, kind, .. } => {
                self.step = match kind.as_str() {
                    "human" => format!("Turn {index}: checking your hand edit"),
                    _ => {
                        let model = self
                            .model
                            .as_deref()
                            .map(|m| format!(" (`{m}`)"))
                            .unwrap_or_default();
                        let why = match kind.as_str() {
                            "translate" => " for a translation".to_string(),
                            "repair" => " to repair it".to_string(),
                            "steer" => " to apply your note".to_string(),
                            other => format!(" ({other})"),
                        };
                        format!("Turn {index}: asking the model{model}{why}")
                    }
                };
            }
            Event::TurnEnd { index, result, .. } => {
                self.step = format!("Turn {index}: {}", result_words(result));
            }
            Event::Check { name, passed, .. } => {
                self.checks.push((name.clone(), *passed));
                self.step = format!(
                    "Checked: {} — {}",
                    check_words(name),
                    if *passed { "passed" } else { "FAILED" }
                );
            }
            Event::Verdict { green, .. } => {
                let n = self.checks.len();
                let text = if *green {
                    format!("GREEN — all {n} checks passed")
                } else {
                    let failed: Vec<String> = self
                        .checks
                        .iter()
                        .filter(|(_, ok)| !ok)
                        .map(|(name, _)| check_words(name))
                        .collect();
                    format!(
                        "RED — {} of {n} checks failed: {}",
                        failed.len(),
                        failed.join(", ")
                    )
                };
                self.step = format!("Verdict: {text}");
                self.verdict = Some(text);
            }
            Event::Attempt { id, outcome, .. } => {
                self.step = format!("Recorded attempt {id}: {outcome}");
            }
            Event::Promote {
                unit,
                attempt,
                result,
            } => {
                self.step = if result == "verified" {
                    format!("Promoted {attempt} into {unit}: verified")
                } else {
                    format!("Promoting {attempt} rolled back — the crate is unchanged")
                };
            }
            Event::Awaiting { path, .. } => {
                self.awaited = true;
                self.step = if self.steer {
                    format!(
                        "Paused: waiting for the answer to the hand-off (request next to \
                         {path}). Write the answer, then choose Resume."
                    )
                } else {
                    // The cockpit's own acts never pose one (Retry refuses
                    // an unseeded `external` attempt): a guard.
                    "Paused: a BLIND hand-off — only the audited protocol \
                     (targets/tractor/handoff-tools) may answer it; an answer written by hand \
                     is recorded as pipeline output."
                        .into()
                };
            }
            Event::Error {
                kind,
                message,
                holder,
            } => {
                self.error = Some((kind.clone(), message.clone()));
                match kind.as_str() {
                    // Folded into the pause.
                    "awaiting" => {}
                    "locked" => {
                        let who = holder
                            .as_ref()
                            .map_or_else(|| "another command".into(), |h| h.command.clone());
                        self.locked_by = Some(who.clone());
                        self.step = format!(
                            "Another command is changing this project ({who}). Nothing was done."
                        );
                    }
                    "stale" => self.step = format!("Out of date: {message}"),
                    "interrupted" => self.step = "Stopped.".into(),
                    _ => self.step = format!("The harness reported: {message}"),
                }
            }
        }
    }

    /// How a reaped run ended: `code` is its exit code, `signal` the
    /// signal's name when it died by one.
    pub fn ending(&self, code: Option<i32>, signal: Option<&str>) -> Ending {
        if signal.is_some() {
            return Ending::Stopped;
        }
        match (code, self.error.as_ref().map(|(k, _)| k.as_str())) {
            (Some(0), _) => Ending::Done,
            (Some(10), _) => Ending::Red,
            (Some(1), _) if self.awaited => Ending::Paused,
            (Some(1), Some("locked")) => Ending::Locked,
            (_, Some("interrupted")) => Ending::Stopped,
            (Some(1), _) => Ending::Refused,
            _ => Ending::Other,
        }
    }

    /// The idle line's sentence for a reaped run.
    pub fn last(&self, code: Option<i32>, signal: Option<&str>, took: Duration) -> String {
        let outcome = match self.ending(code, signal) {
            Ending::Done => match (&self.verdict, self.subcommand.as_str()) {
                (Some(v), _) => v.clone(),
                _ => "Done".into(),
            },
            Ending::Red => match &self.verdict {
                Some(v) => v.clone(),
                None => "Finished — red".into(),
            },
            Ending::Paused => "Paused".into(),
            Ending::Locked => format!(
                "Refused: another command is changing this project ({}) — nothing was done",
                self.locked_by.as_deref().unwrap_or("another command")
            ),
            Ending::Refused => {
                let why = self
                    .error
                    .as_ref()
                    .filter(|(k, _)| k != "awaiting")
                    .map(|(_, m)| m.clone())
                    .or_else(|| self.last_message.clone())
                    .unwrap_or_else(|| "exit 1".into());
                format!("Refused: {why}")
            }
            Ending::Stopped => match signal {
                Some(sig) => format!("Stopped ({sig})"),
                None => "Stopped".into(),
            },
            Ending::Other => match code {
                Some(c) => format!("exit {c}"),
                None => "ended".into(),
            },
        };
        format!("{} — {outcome} ({})", self.label, elapsed_words(took))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::parse_line;
    use std::path::PathBuf;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn fixture(name: &str) -> Vec<Event> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/events")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .map(parse_line)
            .collect()
    }

    fn run(n: &mut Narrator, events: &[Event]) -> Vec<String> {
        events
            .iter()
            .map(|e| {
                n.on_event(e);
                n.step().to_string()
            })
            .collect()
    }

    #[test]
    fn a_green_migrate_is_narrated_turn_by_turn() {
        let mut n = Narrator::new(
            "Modify a-13c9",
            &argv(&[
                "harness",
                "--json",
                "migrate",
                "u-lib",
                "--model=m-1",
                "--steer=x",
            ]),
        );
        let steps = run(&mut n, &fixture("migrate-green.ndjson"));
        assert!(
            steps
                .iter()
                .any(|s| s.starts_with("Turn 1: asking the model (`m-1`)")),
            "{steps:#?}"
        );
        assert!(steps.iter().any(|s| s == "Turn 1: passed"), "{steps:#?}");
        assert!(
            steps.iter().any(|s| s.starts_with("Recorded attempt a-")),
            "{steps:#?}"
        );
        assert_eq!(n.ending(Some(0), None), Ending::Done);
    }

    #[test]
    fn an_awaiting_run_pauses_and_a_blind_one_is_flagged() {
        let events = fixture("migrate-awaiting.ndjson");
        let mut blind = Narrator::new("Retry", &argv(&["harness", "--json", "migrate", "u"]));
        run(&mut blind, &events);
        assert!(blind.step().contains("BLIND"), "{}", blind.step());
        assert_eq!(blind.ending(Some(1), None), Ending::Paused);
        let mut steer = Narrator::new(
            "Modify",
            &argv(&["harness", "--json", "migrate", "u", "--steer=- x"]),
        );
        run(&mut steer, &events);
        assert!(steer.step().starts_with("Paused: waiting for the answer"));
        assert_eq!(
            steer.last(Some(1), None, Duration::from_secs(3)),
            "Modify — Paused (3 s)"
        );
    }

    #[test]
    fn a_locked_refusal_names_the_holder_and_offers_try_again() {
        let mut n = Narrator::new("Scan the project", &argv(&["harness", "--json", "scan"]));
        run(&mut n, &fixture("scan-locked.ndjson"));
        assert!(
            n.step()
                .starts_with("Another command is changing this project ("),
            "{}",
            n.step()
        );
        assert!(n.step().ends_with("Nothing was done."));
        assert_eq!(n.ending(Some(1), None), Ending::Locked);
        assert_eq!(
            n.last(Some(1), None, Duration::from_secs(1)),
            "Scan the project — Refused: another command is changing this project (verify \
             u001-katajainen) — nothing was done (1 s)"
        );
    }

    #[test]
    fn a_rolled_back_promotion_says_the_crate_is_unchanged() {
        let mut n = Narrator::new(
            "Accept a-1",
            &argv(&["harness", "--json", "promote", "u", "a-1"]),
        );
        assert!(n.step().starts_with("Running the oracle"));
        let steps = run(&mut n, &fixture("promote-rolled-back.ndjson"));
        assert!(
            steps
                .iter()
                .any(|s| s.contains("rolled back — the crate is unchanged")),
            "{steps:#?}"
        );
    }

    #[test]
    fn a_verdict_counts_this_runs_checks() {
        let mut n = Narrator::new("Re-check u-lib", &argv(&["harness", "--json", "verify"]));
        let check = |name: &str, passed| Event::Check {
            unit: "u".into(),
            name: name.into(),
            passed,
            detail: String::new(),
        };
        n.on_event(&check("symbol-set", true));
        assert_eq!(n.step(), "Checked: same exports — passed");
        n.on_event(&check("differential-driver", false));
        assert_eq!(n.step(), "Checked: same outputs as C — FAILED");
        n.on_event(&check("whole-program:zopfli", true));
        n.on_event(&Event::Verdict {
            unit: "u".into(),
            green: false,
            path: "p".into(),
        });
        assert_eq!(
            n.step(),
            "Verdict: RED — 1 of 3 checks failed: same outputs as C"
        );
        assert_eq!(
            n.last(Some(10), None, Duration::from_secs(125)),
            "Re-check u-lib — RED — 1 of 3 checks failed: same outputs as C (2 min 5 s)"
        );
        assert_eq!(n.ending(None, Some("SIGINT")), Ending::Stopped);
    }

    /// Fixtures recorded from the CLI's own runs (docs/COCKPIT-WRAPPER-
    /// DESIGN.md §13): verify green and red, plan, detect, a stale refusal.
    #[test]
    fn recorded_cli_runs_are_narrated() {
        let argv_of = |sub: &str| argv(&["harness", "--json", sub, "u-lib"]);
        let mut green = Narrator::new("Re-check u-lib", &argv_of("verify"));
        run(&mut green, &fixture("verify-green.ndjson"));
        assert_eq!(
            green.last(Some(0), None, Duration::from_secs(41)),
            "Re-check u-lib — GREEN — all 7 checks passed (41 s)"
        );
        let mut red = Narrator::new("Re-check u-lib", &argv_of("verify"));
        let steps = run(&mut red, &fixture("verify-red.ndjson"));
        assert!(
            steps.contains(&"Checked: same outputs as C — FAILED".to_string()),
            "{steps:#?}"
        );
        assert_eq!(red.ending(Some(10), None), Ending::Red);
        assert_eq!(
            red.last(Some(10), None, Duration::from_secs(3)),
            "Re-check u-lib — RED — 1 of 6 checks failed: same outputs as C (3 s)"
        );
        let mut plan = Narrator::new("Refresh the plan", &argv_of("plan"));
        let steps = run(&mut plan, &fixture("plan.ndjson"));
        assert!(
            steps.contains(&"plan: no changes (1 units)".to_string()),
            "{steps:#?}"
        );
        assert_eq!(
            plan.last(Some(0), None, Duration::ZERO),
            "Refresh the plan — Done (0 s)"
        );
        let mut detect = Narrator::new("Find hazards", &argv_of("detect"));
        let steps = run(&mut detect, &fixture("detect.ndjson"));
        assert!(
            steps.iter().any(|s| s.starts_with("detect: 0 finding(s)")),
            "{steps:#?}"
        );
        let mut stale = Narrator::new("Find hazards", &argv_of("detect"));
        run(&mut stale, &fixture("detect-stale.ndjson"));
        assert!(
            stale
                .step()
                .starts_with("Out of date: facts.jsonl is stale"),
            "{}",
            stale.step()
        );
        assert_eq!(stale.ending(Some(1), None), Ending::Refused);
        assert!(stale
            .last(Some(1), None, Duration::ZERO)
            .starts_with("Find hazards — Refused: facts.jsonl is stale"));
    }

    #[test]
    fn words_for_checks_and_results() {
        assert_eq!(check_words("whole-program:zopfli"), "whole program");
        assert_eq!(check_words("mystery"), "mystery");
        assert_eq!(result_words("crash-timeout"), "it crashed or timed out");
        assert_eq!(result_words("odd"), "odd");
    }
}
