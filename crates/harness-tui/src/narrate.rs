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

/// Why a check did not run, for a check the oracle records as not run (an
/// unconfigured whole-program check, recorded `passed: true`): `verify`'s
/// own screen line ([`harness_oracle::check_screen_line`]) decides, so the
/// cockpit and the CLI never disagree, and the sentence is the CLI's
/// ("not configured for this target (add [oracle.whole_program] …)").
/// `None` for a check that ran.
pub fn not_run_why(name: &str, passed: bool, detail: &str) -> Option<String> {
    let line = harness_oracle::check_screen_line(&harness_core::verdict::Check {
        name: name.into(),
        passed,
        detail: detail.into(),
    });
    let rest = line.strip_prefix("[SKIP] ")?;
    Some(
        rest.split_once("not run: ")
            .map_or(rest, |(_, why)| why)
            .to_string(),
    )
}

/// "1 not run (whole-program: not configured …)" for the checks that did
/// not run, each `(name, why)`; empty when every check ran.
pub fn not_run_words(not_run: &[(String, String)]) -> String {
    if not_run.is_empty() {
        return String::new();
    }
    let each: Vec<String> = not_run
        .iter()
        .map(|(name, why)| format!("{name}: {why}"))
        .collect();
    format!("{} not run ({})", not_run.len(), each.join("; "))
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
    /// A run for the chat (`--requester=chat`): its hand-offs are the
    /// chat's to answer (docs/CHAT-PANE-DESIGN.md §3.4).
    chat: bool,
    turn: Option<u64>,
    step: String,
    checks: Vec<(String, bool)>,
    /// The checks that did not run: name, why (the CLI's words).
    not_run: Vec<(String, String)>,
    verdict: Option<String>,
    awaited: bool,
    error: Option<(String, String)>,
    last_message: Option<String>,
    locked_by: Option<String>,
    /// The perf rows this run wrote.
    perf_rows: usize,
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
            chat: argv.iter().any(|a| a == "--requester=chat"),
            turn: None,
            step,
            checks: Vec::new(),
            not_run: Vec::new(),
            verdict: None,
            awaited: false,
            error: None,
            last_message: None,
            locked_by: None,
            perf_rows: 0,
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
                self.turn = Some(*index);
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
            Event::Check {
                name,
                passed,
                detail,
                ..
            } => {
                self.checks.push((name.clone(), *passed));
                let state = match not_run_why(name, *passed, detail) {
                    Some(why) => {
                        let state = format!("not run: {why}");
                        self.not_run.push((name.clone(), why));
                        state
                    }
                    None if *passed => "passed".into(),
                    None => "FAILED".into(),
                };
                self.step = format!("Checked: {} — {state}", check_words(name));
            }
            Event::Verdict { green, .. } => {
                let n = self.checks.len();
                let skipped = not_run_words(&self.not_run);
                let text = if *green && skipped.is_empty() {
                    format!("GREEN — all {n} checks passed")
                } else if *green {
                    let passed = n - self.not_run.len();
                    let s = if passed == 1 { "" } else { "s" };
                    format!("GREEN — {passed} check{s} passed, {skipped}")
                } else {
                    let failed: Vec<String> = self
                        .checks
                        .iter()
                        .filter(|(_, ok)| !ok)
                        .map(|(name, _)| check_words(name))
                        .collect();
                    let skipped = if skipped.is_empty() {
                        skipped
                    } else {
                        format!("; {skipped}")
                    };
                    format!(
                        "RED — {} of {n} checks failed: {}{skipped}",
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
            Event::PerfRow {
                side,
                unit,
                workload,
                words,
                ..
            } => {
                self.perf_rows += 1;
                let who = match (side.as_str(), unit) {
                    ("unit", Some(u)) => u.clone(),
                    ("program", _) => "the program as it stands".into(),
                    _ => "the C".into(),
                };
                self.step = format!("{who} on {workload} — {words}");
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
                self.step = if self.chat {
                    self.paused_words()
                } else if self.steer {
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

    fn paused_words(&self) -> String {
        match self.turn {
            Some(n) => format!("Paused: the chat answers turn {n}"),
            None => "Paused: the chat answers its turn".into(),
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
                // docs/PERF-DESIGN.md §3.11: "Measured 4 rows — see Speed".
                (None, "perf") if self.perf_rows > 0 => format!(
                    "Measured {} row{} — see Speed",
                    self.perf_rows,
                    if self.perf_rows == 1 { "" } else { "s" }
                ),
                _ => "Done".into(),
            },
            Ending::Red => match &self.verdict {
                Some(v) => v.clone(),
                None => "Finished — red".into(),
            },
            Ending::Paused if self.chat => self.paused_words(),
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
        // A chat run: the chat answers it (docs/CHAT-PANE-DESIGN.md §3.4).
        let mut chat = Narrator::new(
            "Migrate u (asked in chat)",
            &argv(&["harness", "--json", "migrate", "u", "--requester=chat"]),
        );
        run(&mut chat, &events);
        assert_eq!(chat.step(), "Paused: the chat answers turn 1");
        assert_eq!(
            chat.last(Some(1), None, Duration::from_secs(3)),
            "Migrate u (asked in chat) — Paused: the chat answers turn 1 (3 s)"
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
            "Re-check u-lib — GREEN — 6 checks passed, 1 not run (whole-program: not \
             configured for this target (add [oracle.whole_program] args = [...] to \
             harness.toml)) (41 s)"
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
            "Re-check u-lib — RED — 1 of 6 checks failed: same outputs as C; 1 not run \
             (whole-program: not configured for this target (add [oracle.whole_program] \
             args = [...] to harness.toml)) (3 s)"
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

    /// An unconfigured whole-program check did not run: the narration says
    /// so, as `verify` does, and never counts it among the passes.
    #[test]
    fn an_unconfigured_whole_program_check_is_not_run_not_passed() {
        let mut n = Narrator::new("Re-check u", &argv(&["harness", "--json", "verify"]));
        let check = |name: &str, detail: &str| Event::Check {
            unit: "u".into(),
            name: name.into(),
            passed: true,
            detail: detail.into(),
        };
        n.on_event(&check("symbol-set", "ok"));
        n.on_event(&check(
            "whole-program",
            harness_oracle::WHOLE_PROGRAM_NOT_CONFIGURED,
        ));
        assert_eq!(
            n.step(),
            "Checked: whole program — not run: not configured for this target (add \
             [oracle.whole_program] args = [...] to harness.toml)"
        );
        n.on_event(&Event::Verdict {
            unit: "u".into(),
            green: true,
            path: "p".into(),
        });
        assert_eq!(
            n.step(),
            "Verdict: GREEN — 1 check passed, 1 not run (whole-program: not configured \
             for this target (add [oracle.whole_program] args = [...] to harness.toml))"
        );
        // A whole-program check that ran is a pass like any other.
        assert_eq!(
            not_run_why("whole-program", true, "3 sample(s) identical"),
            None
        );
        assert_eq!(
            not_run_why("whole-program", false, "not configured for this target"),
            None
        );
    }

    /// The verify fixtures say what today's binary prints: every check's
    /// screen line is `verify: ` + `harness_oracle::check_screen_line`.
    #[test]
    fn the_verify_fixtures_print_what_verify_prints() {
        for name in ["verify-green.ndjson", "verify-red.ndjson"] {
            let events = fixture(name);
            let lines: Vec<String> = events
                .iter()
                .filter_map(|e| match e {
                    Event::Check {
                        name,
                        passed,
                        detail,
                        ..
                    } => Some(format!(
                        "verify: {}",
                        harness_oracle::check_screen_line(&harness_core::verdict::Check {
                            name: name.clone(),
                            passed: *passed,
                            detail: detail.clone(),
                        })
                    )),
                    _ => None,
                })
                .collect();
            let printed: Vec<&String> = events
                .iter()
                .filter_map(|e| match e {
                    Event::Message { text } if text.starts_with("verify: [") => Some(text),
                    _ => None,
                })
                .collect();
            assert_eq!(printed.len(), lines.len(), "{name}");
            for (p, l) in printed.iter().zip(&lines) {
                assert_eq!(*p, l, "{name}");
            }
        }
    }

    #[test]
    fn words_for_checks_and_results() {
        assert_eq!(check_words("whole-program:zopfli"), "whole program");
        assert_eq!(check_words("mystery"), "mystery");
        assert_eq!(result_words("crash-timeout"), "it crashed or timed out");
        assert_eq!(result_words("odd"), "odd");
    }

    #[test]
    fn a_perf_run_counts_its_rows() {
        let mut n = Narrator::new(
            "Measure speed",
            &argv(&["harness", "--json", "perf", "run", "--target=/t"]),
        );
        for line in [
            r#"{"k":"perf-row","side":"c","workload":"big","outcome":"baseline","words":"CPU about 1.2 s"}"#,
            r#"{"k":"perf-row","side":"unit","unit":"u001","workload":"big","outcome":"measured","words":"about as fast"}"#,
        ] {
            n.on_event(&parse_line(line));
        }
        assert_eq!(n.step(), "u001 on big — about as fast");
        assert!(
            n.last(Some(0), None, Duration::from_secs(3))
                .starts_with("Measure speed — Measured 2 rows — see Speed"),
            "{}",
            n.last(Some(0), None, Duration::from_secs(3))
        );
        let mut none = Narrator::new(
            "Measure speed",
            &argv(&["harness", "--json", "perf", "run"]),
        );
        none.on_event(&parse_line(r#"{"k":"perf-row","side":"program"}"#));
        assert!(none.last(Some(0), None, Duration::ZERO).contains("— Done"));
    }
}
