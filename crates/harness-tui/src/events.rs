//! The `ruharness-events` stream a `harness --json` command writes on stdout
//! (docs/SCHEMAS.md "The events stream"), one typed [`Event`] per line.
//!
//! `k` is an open enum and fields are additive: an unknown kind — or a known
//! kind missing a field this reader needs — is kept as [`Event::Other`] with
//! its line, never dropped; unknown fields are ignored; a line that is not a
//! JSON object is [`Event::NotJson`]. Every value is the ledger's own,
//! verbatim: this module never interprets one (the cockpit's display filter
//! runs on whatever it renders).

use serde_json::Value;
use std::io::BufRead;

/// The schema this reader knows (the header's `schema`).
pub const EVENTS_SCHEMA: &str = "ruharness-events";
/// The newest `schema_version` this reader knows.
pub const EVENTS_SCHEMA_VERSION: u64 = 1;
/// Longest line kept, in bytes; the rest of a longer line is discarded.
pub const MAX_EVENT_LINE_BYTES: usize = 1024 * 1024;

/// The holder of the writer lock, as a `locked` error names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    /// Its pid, when recorded.
    pub pid: Option<u64>,
    /// Its command (`migrate u-lib`).
    pub command: String,
    /// When it took the lock (RFC 3339), when recorded.
    pub started: String,
}

/// One event of the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The first line.
    Header {
        /// `schema` (normally [`EVENTS_SCHEMA`]).
        schema: String,
        /// `schema_version`.
        schema_version: u64,
        /// The subcommand.
        command: String,
        /// The rest of the command line, without `--json`.
        args: Vec<String>,
        /// The harness's pid.
        pid: Option<u64>,
        /// The harness version.
        harness: String,
    },
    /// A human line the command printed.
    Message {
        /// The line.
        text: String,
    },
    /// A turn is about to be sent.
    TurnStart {
        /// Unit id.
        unit: String,
        /// Attempt id.
        attempt: String,
        /// 1-based turn index.
        index: u64,
        /// Turn kind (`translate`, `repair`, `steer`, `human`, …).
        kind: String,
    },
    /// A turn was journaled.
    TurnEnd {
        /// Unit id.
        unit: String,
        /// Attempt id.
        attempt: String,
        /// 1-based turn index.
        index: u64,
        /// Turn kind.
        kind: String,
        /// The Turn's `result` (closed set in the ledger, open here).
        result: String,
    },
    /// An attempt ended.
    Attempt {
        /// Unit id.
        unit: String,
        /// Attempt id.
        id: String,
        /// Its outcome.
        outcome: String,
        /// Provider profile.
        provider: String,
        /// Model.
        model: String,
        /// Whether it was promoted.
        promoted: bool,
        /// Why it was or was not (courtesy).
        promotion: String,
    },
    /// One oracle check.
    Check {
        /// Unit id.
        unit: String,
        /// Check name.
        name: String,
        /// Whether it passed.
        passed: bool,
        /// Its detail.
        detail: String,
    },
    /// A verdict was stored.
    Verdict {
        /// Unit id.
        unit: String,
        /// Green or red.
        green: bool,
        /// Where it was stored.
        path: String,
    },
    /// A promotion ended.
    Promote {
        /// Unit id.
        unit: String,
        /// Attempt id.
        attempt: String,
        /// `verified` / `rolled-back`.
        result: String,
    },
    /// The `external` hand-off is waiting for a response file.
    Awaiting {
        /// The attempt (none for triage).
        attempt: Option<String>,
        /// The response file the command waits for.
        path: String,
        /// The human re-run hint (a hint, never executed by a client).
        resume: String,
        /// The command line after the program name, without `--json`.
        args: Option<Vec<String>>,
    },
    /// The command failed.
    Error {
        /// `locked` / `stale` / `awaiting` / `interrupted` / `harness`, open.
        kind: String,
        /// The message.
        message: String,
        /// The lock holder, for `locked`.
        holder: Option<Holder>,
    },
    /// The last line.
    Result {
        /// The shell-visible exit code.
        exit: i64,
        /// The signal the harness died by, when it did (best effort).
        signal: Option<String>,
    },
    /// A kind this reader does not know (or a known kind it cannot read).
    Other {
        /// The `k` value.
        k: String,
        /// The line, verbatim.
        line: String,
    },
    /// A line that is not a JSON object with a string `k`.
    NotJson {
        /// The line, verbatim.
        line: String,
    },
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn num(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn flag(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn strings(v: &Value, key: &str) -> Option<Vec<String>> {
    v.get(key)?
        .as_array()?
        .iter()
        .map(|a| a.as_str().map(str::to_string))
        .collect()
}

/// Parse one line of the stream.
pub fn parse_line(line: &str) -> Event {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Event::NotJson { line: line.into() };
    };
    let Some(k) = v.get("k").and_then(Value::as_str) else {
        return Event::NotJson { line: line.into() };
    };
    known(k, &v).unwrap_or_else(|| Event::Other {
        k: k.to_string(),
        line: line.into(),
    })
}

fn known(k: &str, v: &Value) -> Option<Event> {
    Some(match k {
        "header" => Event::Header {
            schema: text(v, "schema")?,
            schema_version: num(v, "schema_version")?,
            command: text(v, "command")?,
            args: strings(v, "args").unwrap_or_default(),
            pid: num(v, "pid"),
            harness: text(v, "harness").unwrap_or_default(),
        },
        "message" => Event::Message {
            text: text(v, "text")?,
        },
        "turn-start" => Event::TurnStart {
            unit: text(v, "unit")?,
            attempt: text(v, "attempt")?,
            index: num(v, "index")?,
            kind: text(v, "kind")?,
        },
        "turn-end" => Event::TurnEnd {
            unit: text(v, "unit")?,
            attempt: text(v, "attempt")?,
            index: num(v, "index")?,
            kind: text(v, "kind")?,
            result: text(v, "result")?,
        },
        "attempt" => Event::Attempt {
            unit: text(v, "unit")?,
            id: text(v, "id")?,
            outcome: text(v, "outcome")?,
            provider: text(v, "provider").unwrap_or_default(),
            model: text(v, "model").unwrap_or_default(),
            promoted: flag(v, "promoted").unwrap_or(false),
            promotion: text(v, "promotion").unwrap_or_default(),
        },
        "check" => Event::Check {
            unit: text(v, "unit")?,
            name: text(v, "name")?,
            passed: flag(v, "passed")?,
            detail: text(v, "detail").unwrap_or_default(),
        },
        "verdict" => Event::Verdict {
            unit: text(v, "unit")?,
            green: flag(v, "green")?,
            path: text(v, "path")?,
        },
        "promote" => Event::Promote {
            unit: text(v, "unit")?,
            attempt: text(v, "attempt")?,
            result: text(v, "result")?,
        },
        "awaiting" => Event::Awaiting {
            attempt: text(v, "attempt"),
            path: text(v, "path")?,
            resume: text(v, "resume").unwrap_or_default(),
            args: strings(v, "args"),
        },
        "error" => Event::Error {
            kind: text(v, "kind")?,
            message: text(v, "message")?,
            holder: v.get("holder").filter(|h| h.is_object()).map(|h| Holder {
                pid: num(h, "pid"),
                command: text(h, "command").unwrap_or_default(),
                started: text(h, "started").unwrap_or_default(),
            }),
        },
        "result" => Event::Result {
            exit: v.get("exit").and_then(Value::as_i64)?,
            signal: text(v, "signal"),
        },
        _ => return None,
    })
}

/// Read one line of at most `cap` bytes from `reader`, without its `\n`
/// (and a trailing `\r`): the bytes of a longer line past `cap` are read
/// and discarded, so a runaway line can never exhaust memory. `None` at
/// EOF. Invalid UTF-8 is replaced, never an error.
pub fn read_line_bounded(reader: &mut impl BufRead, cap: usize) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    let mut seen_any = false;
    loop {
        let (consumed, done) = {
            let buf = match reader.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if buf.is_empty() {
                if !seen_any {
                    return Ok(None);
                }
                break;
            }
            seen_any = true;
            match buf.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    let room = cap.saturating_sub(line.len());
                    line.extend_from_slice(&buf[..i.min(room)]);
                    (i + 1, true)
                }
                None => {
                    let room = cap.saturating_sub(line.len());
                    line.extend_from_slice(&buf[..buf.len().min(room)]);
                    (buf.len(), false)
                }
            }
        };
        reader.consume(consumed);
        if done {
            break;
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<Event> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/events")
            .join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        text.lines().map(parse_line).collect()
    }

    #[test]
    fn a_recorded_migrate_stream_reads_as_typed_events() {
        let evs = fixture("migrate-green.ndjson");
        assert!(
            matches!(&evs[0], Event::Header { command, schema_version: 1, .. } if command == "migrate")
        );
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::TurnStart { index: 1, .. })));
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::TurnEnd { result, .. } if result == "green")));
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Check { passed: true, .. })));
        assert!(evs.iter().any(
            |e| matches!(e, Event::Verdict { path, .. } if path.ends_with("attempt-verdict.json"))
        ));
        assert!(evs.iter().any(|e| matches!(
            e,
            Event::Attempt {
                promoted: false,
                ..
            }
        )));
        assert_eq!(
            evs.last(),
            Some(&Event::Result {
                exit: 0,
                signal: None
            })
        );
        assert!(!evs
            .iter()
            .any(|e| matches!(e, Event::Other { .. } | Event::NotJson { .. })));
    }

    #[test]
    fn awaiting_locked_and_rolled_back_streams() {
        let evs = fixture("migrate-awaiting.ndjson");
        let awaiting = evs
            .iter()
            .find_map(|e| match e {
                Event::Awaiting {
                    attempt,
                    path,
                    resume,
                    args,
                } => Some((attempt.clone(), path.clone(), resume.clone(), args.clone())),
                _ => None,
            })
            .expect("an awaiting event");
        assert!(awaiting.0.is_some_and(|a| a.starts_with("a-")));
        assert!(awaiting.1.ends_with(".response.json"));
        assert!(awaiting.2.starts_with("harness migrate "));
        assert!(awaiting
            .3
            .is_some_and(|a| !a.contains(&"--json".to_string())));
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Error { kind, .. } if kind == "awaiting")));
        assert!(matches!(evs.last(), Some(Event::Result { exit: 1, .. })));

        let evs = fixture("scan-locked.ndjson");
        let holder = evs
            .iter()
            .find_map(|e| match e {
                Event::Error {
                    kind,
                    holder: Some(h),
                    ..
                } if kind == "locked" => Some(h.clone()),
                _ => None,
            })
            .expect("a locked error with its holder");
        assert!(holder.pid.is_some());
        assert!(holder.command.starts_with("verify "));

        let evs = fixture("promote-rolled-back.ndjson");
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Promote { result, .. } if result == "rolled-back")));
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Check { passed: false, .. })));
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Verdict { .. })),
            "a rolled-back promotion stores no verdict"
        );
        assert!(matches!(evs.last(), Some(Event::Result { exit: 10, .. })));
    }

    #[test]
    fn unknown_kinds_fields_and_garbage_are_kept_not_dropped() {
        assert_eq!(
            parse_line(r#"{"k":"future-thing","x":1}"#),
            Event::Other {
                k: "future-thing".into(),
                line: r#"{"k":"future-thing","x":1}"#.into()
            }
        );
        // A known kind missing a field this reader needs: kept as Other.
        assert!(matches!(
            parse_line(r#"{"k":"verdict","unit":"u"}"#),
            Event::Other { .. }
        ));
        // Unknown fields are ignored.
        assert_eq!(
            parse_line(r#"{"k":"message","text":"hi","new":true}"#),
            Event::Message { text: "hi".into() }
        );
        for garbage in ["not json", "[1,2]", r#"{"k":3}"#, ""] {
            assert!(
                matches!(parse_line(garbage), Event::NotJson { .. }),
                "{garbage:?}"
            );
        }
        assert_eq!(
            parse_line(r#"{"k":"result","exit":130,"signal":"SIGINT"}"#),
            Event::Result {
                exit: 130,
                signal: Some("SIGINT".into())
            }
        );
    }

    #[test]
    fn lines_are_bounded_and_split_exactly() {
        let data = format!("short\r\n{}\nlast", "x".repeat(100));
        let mut reader = std::io::BufReader::with_capacity(7, data.as_bytes());
        let mut lines = Vec::new();
        while let Some(line) = read_line_bounded(&mut reader, 10).unwrap() {
            lines.push(line);
        }
        assert_eq!(lines, ["short", &"x".repeat(10), "last"]);
        let mut empty = std::io::BufReader::new(&b""[..]);
        assert_eq!(read_line_bounded(&mut empty, 10).unwrap(), None);
        let mut blank = std::io::BufReader::new(&b"\n\n"[..]);
        assert_eq!(
            read_line_bounded(&mut blank, 10).unwrap(),
            Some(String::new())
        );
        assert_eq!(
            read_line_bounded(&mut blank, 10).unwrap(),
            Some(String::new())
        );
        assert_eq!(read_line_bounded(&mut blank, 10).unwrap(), None);
        let mut bad = std::io::BufReader::new(&b"a\xffb\n"[..]);
        assert_eq!(
            read_line_bounded(&mut bad, 10).unwrap(),
            Some("a\u{fffd}b".into())
        );
    }
}
