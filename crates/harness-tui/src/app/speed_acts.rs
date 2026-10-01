//! Measuring speed from the cockpit (docs/PERF-DESIGN.md §3.11): the
//! Measure acts' argv and gates, and their confirm dialogs' words — what
//! runs, how many times, about how long (§6), what it writes, no verdict,
//! the ledger's lock, that Cancel keeps finished rows, and to keep the
//! computer quiet.

use super::{os, Act, App, Mode, Pending};
use crate::speed::{Group, SideKey};
use harness_core::perf::estimate::{self, Estimate, Job};
use std::ffi::OsString;

impl App {
    /// Why perf cannot measure now, from the workloads file's state (and
    /// the platform) in the words `perf run` refuses with (§3.1 *States*:
    /// "Measure is greyed with the same words"); `None` when it can.
    pub fn speed_gate(&self) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return Some(
                "perf runs on macOS only for now — the Linux launcher is not built yet".into(),
            );
        }
        match &self.speed.group {
            Group::NoFile | Group::NoWorkload | Group::FileError(_) => self
                .speed
                .blocker
                .clone()
                .or_else(|| Some("the workloads file cannot be used".into())),
            _ => None,
        }
    }

    /// Why `id` is not one perf would measure now, in perf's own words for
    /// the reason it would leave it out (§3.2).
    fn not_measurable(&self, id: &str) -> String {
        let Some(u) = self.snapshot.unit(id) else {
            return format!("unit {id} is gone");
        };
        if !matches!(
            u.unit.status,
            harness_core::UnitStatus::Verified | harness_core::UnitStatus::Merged
        ) {
            return "perf measures a verified unit's Rust — accept an attempt first".into();
        }
        match crate::speed::left_out_today(u) {
            Some("accept-interrupted") => format!(
                "{}; Measure does not",
                harness_core::perf::accept_interrupted_words(
                    u.report
                        .promotion_interrupted
                        .as_deref()
                        .unwrap_or("legacy"),
                    id
                )
            ),
            Some("replaces-changed") => {
                format!("{id}'s replaced files changed since verify — Re-check it")
            }
            _ => "its verdict is not green and fresh — Re-check it first".into(),
        }
    }

    /// A Measure act's argv and its label (the one argv builder's part for
    /// them): `perf run`, `--unit=ID` for a unit, `--as-it-stands-only` for
    /// the program as it stands, `--workload=W`… `--runs=31` to measure
    /// again.
    pub(super) fn measure_argv(
        &self,
        act: Act,
        unit: Option<&str>,
    ) -> Result<(Vec<OsString>, String), String> {
        if let Some(why) = self.speed_gate() {
            return Err(why);
        }
        let measurable = |id: &str| -> Result<(), String> {
            if self.speed.measurable.iter().any(|m| m == id) {
                Ok(())
            } else {
                Err(self.not_measurable(id))
            }
        };
        // `--as-it-stands-only` with fewer than two measurable units: perf
        // would build everything, then refuse (§3.10) — said here instead.
        let two_units = || -> Result<(), String> {
            if self.speed.measurable.len() < 2 {
                return Err(
                    "the program as it stands needs two verified units — with one, that \
                     unit's own row measures the same program"
                        .into(),
                );
            }
            Ok(())
        };
        let mut rest = vec![os("perf"), os("run"), self.target_arg()];
        let label = match act {
            Act::Measure => match unit {
                Some(id) => {
                    measurable(id)?;
                    rest.push(os(format!("--unit={id}")));
                    format!("Measure {id}'s speed")
                }
                None => act.label().to_string(),
            },
            Act::MeasureProgram => {
                two_units()?;
                rest.push(os("--as-it-stands-only"));
                act.label().to_string()
            }
            Act::MeasureMore => {
                let side = unit.map_or(SideKey::AsItStands, |u| SideKey::Unit(u.to_string()));
                let workloads = self.speed.more_runs(&side);
                if workloads.is_empty() {
                    return Err("no row asks to be measured again".into());
                }
                match unit {
                    Some(id) => {
                        measurable(id)?;
                        rest.push(os(format!("--unit={id}")));
                    }
                    None => {
                        two_units()?;
                        rest.push(os("--as-it-stands-only"))
                    }
                }
                for w in &workloads {
                    rest.push(os(format!("--workload={w}")));
                }
                rest.push(os("--runs=31"));
                format!(
                    "Measure {} again with 31 runs",
                    unit.unwrap_or("the program as it stands")
                )
            }
            _ => return Err("not a Measure act".into()),
        };
        Ok((self.with_sandbox_flag(self.harness_argv(&rest)?), label))
    }

    /// The Measure dialogs' words (§3.11).
    pub(super) fn measure_words(&self, p: &Pending) -> (String, Vec<String>) {
        let m = &self.speed;
        let units = &m.measurable;
        let program = units.len() >= 2;
        let unit = p.unit.as_deref();
        let all: Vec<String> = m.workloads.iter().map(|(id, _)| id.clone()).collect();
        let more = match p.act {
            Act::MeasureMore => {
                m.more_runs(&unit.map_or(SideKey::AsItStands, |u| SideKey::Unit(u.into())))
            }
            _ => Vec::new(),
        };
        let measured = if p.act == Act::MeasureMore {
            &more
        } else {
            &all
        };
        let n_runs = |only: &[String]| -> String {
            if p.act == Act::MeasureMore {
                return "31 runs a side".into();
            }
            let runs: Vec<u32> = m
                .workloads
                .iter()
                .filter(|(id, _)| only.contains(id))
                .map(|(_, n)| *n)
                .collect();
            match runs.first() {
                Some(n) if runs.iter().all(|r| r == n) => format!("{n} runs a side"),
                _ => "each workload's own runs a side".into(),
            }
        };
        let on = format!(
            "{} workload{} ({})",
            measured.len(),
            if measured.len() == 1 { "" } else { "s" },
            measured.join(", ")
        );
        let (title, what, job, built): (String, String, Job, Vec<String>) = match (p.act, unit) {
            (Act::Measure, None) => {
                let mut sides = vec!["the C alone".to_string()];
                if !units.is_empty() {
                    sides.push(format!(
                        "the program with each verified unit's Rust alone ({})",
                        units.join(", ")
                    ));
                }
                if program {
                    sides.push("the program as it stands (all of them together)".into());
                }
                let rows = units.len() as u32 + program as u32;
                (
                    "Measure speed?".into(),
                    format!(
                        "Runs your program on {on}: {} — {}.",
                        sides.join(", then "),
                        n_runs(measured)
                    ),
                    m.job(None, None, true, rows, units.len() as u32, rows),
                    units.clone(),
                )
            }
            (Act::Measure, Some(id)) => (
                format!("Measure {id}'s speed?"),
                format!(
                    "Runs your program on {on}: the C and the program with {id}'s Rust alone, \
                     taking turns — {}.",
                    n_runs(measured)
                ),
                m.job(None, None, false, 1, 1, 1),
                vec![id.to_string()],
            ),
            (Act::MeasureProgram, _) => (
                "Measure the program as it stands?".into(),
                format!(
                    "Runs your program on {on}: the C and the program with every verified unit's \
                     Rust ({}), taking turns — {}.",
                    units.join(", "),
                    n_runs(measured)
                ),
                m.job(None, None, false, 1, units.len() as u32, 1),
                units.clone(),
            ),
            (_, unit) => {
                let who = unit.map_or("the program as it stands".to_string(), |u| {
                    format!("{u}'s Rust")
                });
                let crates = if unit.is_some() {
                    1
                } else {
                    units.len() as u32
                };
                (
                    format!(
                        "Measure {} again with 31 runs?",
                        unit.unwrap_or("the program as it stands")
                    ),
                    format!(
                        "Runs the C and {who} again on {on} — the rows that could not tell, were \
                         \"probably\" or a close call — 31 runs a side. On a busy computer it \
                         may still not tell.",
                    ),
                    m.job(Some(&more), Some(31), false, 1, crates, 1),
                    unit.map_or_else(|| units.clone(), |u| vec![u.to_string()]),
                )
            }
        };
        let mut body = vec![what];
        let cold: Vec<String> = built
            .iter()
            .filter(|id| {
                self.snapshot
                    .unit(id)
                    .and_then(|u| u.crate_dir.as_deref())
                    .is_some_and(estimate::crate_cold)
            })
            .cloned()
            .collect();
        let cold_words = if cold.is_empty() {
            String::new()
        } else {
            format!(
                ", plus building {}'s Rust, which may take minutes",
                cold.join("'s and ")
            )
        };
        body.push(match job.estimate() {
            e @ Estimate::Seconds(_) => {
                format!("Takes {}, builds included{cold_words}.", e.words())
            }
            e => {
                let w = e.words();
                let mut c = w.chars();
                let first = c.next().map(|f| f.to_uppercase().collect::<String>());
                format!("{}{}{cold_words}.", first.unwrap_or_default(), c.as_str())
            }
        });
        body.push(
            "Writes migration/perf/ (its rows replace the ones they measure again) and scratch \
             folders under migration/build/ (.perf, .perf-out with the kept outputs, perf-logs); \
             the first time, or after an update, it builds perf's launcher into \
             ~/Library/Caches/ruharness/perf."
                .into(),
        );
        body.push(
            "No verdict changes: perf only measures. It holds the ledger's lock while it runs; \
             Cancel keeps the rows already finished."
                .into(),
        );
        body.push(
            "Keep the computer quiet while it measures — other work makes the numbers noisier."
                .into(),
        );
        (title, body)
    }

    /// Compare the outputs (§3.11): the kept files of `side`'s row on
    /// `workload`, around their first difference — a unified diff of text,
    /// hex rows of anything else, control characters escaped — when they
    /// are regular files whose size and blake3 match what the row recorded;
    /// else "the two outputs are not on this computer — measure again".
    pub fn compare_outputs(&self, side: &SideKey, workload: &str) -> Result<Mode, String> {
        let row = self
            .speed
            .row(side, workload)
            .ok_or_else(|| format!("no row on {workload}"))?;
        let d = row
            .difference
            .as_ref()
            .ok_or("this row found no difference")?;
        let stream = if d.stream == "stderr" {
            "stderr"
        } else {
            "stdout"
        };
        let unit = match side {
            SideKey::Unit(id) => Some(id.as_str()),
            _ => None,
        };
        let dir = harness_core::perf::kept_outputs_dir(&self.config.target, unit);
        let gone = || "the two outputs are not on this computer — measure again".to_string();
        let read = |which: &str| -> Result<Vec<u8>, String> {
            let name = format!("{workload}.{which}.{stream}");
            let kept = d.kept.iter().find(|k| k.name == name).ok_or_else(gone)?;
            let path = dir.join(&name);
            let meta = std::fs::symlink_metadata(&path).map_err(|_| gone())?;
            if !meta.is_file() || meta.len() != kept.size {
                return Err(gone());
            }
            let bytes = std::fs::read(&path).map_err(|_| gone())?;
            if harness_core::hash::bytes_hash(&bytes) != kept.blake3 {
                return Err(gone());
            }
            Ok(bytes)
        };
        let c = read("c")?;
        let other = read("other")?;
        let who = match side {
            SideKey::Unit(id) => format!("{id}'s Rust"),
            _ => "the program as it stands".into(),
        };
        let offset = if d.stream == stream {
            d.offset as usize
        } else {
            first_difference(&c, &other)
        };
        let mut lines = vec![
            format!(
                "--- the C's {stream} ({} bytes, ended {})",
                c.len(),
                d.c_end
            ),
            format!(
                "+++ {who}: {stream} ({} bytes, ended {})",
                other.len(),
                d.other_end
            ),
            format!(
                "@@ first difference at byte {} · kept in {} @@",
                offset + 1,
                dir.strip_prefix(&self.config.target)
                    .unwrap_or(&dir)
                    .display()
            ),
        ];
        lines.extend(outputs_around(&c, &other, offset));
        Ok(Mode::Diff {
            scroll: 0,
            lines,
            title: format!("the C and {who} on {workload}"),
        })
    }
}

/// Where two byte strings first differ (the shorter's length when one is
/// the other's start).
fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// How far around the first difference the comparison shows.
const COMPARE_BEFORE: usize = 2048;
const COMPARE_AFTER: usize = 8192;

/// `text` with every control character but a tab escaped (`\x1b`, `\r`).
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push(ch),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The two outputs around `offset`: a unified diff of the lines there when
/// both read as text, else hex rows of 16 bytes (`-` the C, `+` the other).
/// The window never drops the difference: its head is cut to a line start
/// only before `offset` (else a partial first line is kept — the bytes
/// before `offset` are the same on both sides, so both cut alike), its
/// tail to a whole line only when the window ends before that stream does
/// (a last line with no newline is kept, so a dropped final newline shows);
/// a cut that would still hide it falls back to the hex rows.
fn outputs_around(c: &[u8], other: &[u8], offset: usize) -> Vec<String> {
    let start = offset.saturating_sub(COMPARE_BEFORE);
    let end = offset.saturating_add(COMPARE_AFTER);
    let as_text = |b: &[u8]| -> Option<String> {
        let (from_w, to_w) = (start.min(b.len()), end.min(b.len()));
        let w = &b[from_w..to_w];
        // Where the difference lies within the window.
        let at = offset.saturating_sub(from_w).min(w.len());
        let from = if start == 0 {
            0
        } else {
            w[..at]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(0, |i| i + 1)
        };
        let to = if to_w == b.len() {
            w.len()
        } else {
            match w.iter().rposition(|&x| x == b'\n') {
                Some(i) if i >= at && i >= from => i + 1,
                _ => w.len(),
            }
        };
        let text = std::str::from_utf8(&w[from..to]).ok()?;
        text.chars()
            .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\t' | '\r'))
            .then(|| text.to_string())
    };
    if let (Some(ct), Some(ot)) = (as_text(c), as_text(other)) {
        let diff = similar::TextDiff::configure()
            .timeout(std::time::Duration::from_millis(200))
            .diff_lines(&ct, &ot);
        // No file header is asked for, so every line is the diff's own (a
        // removed line "--x" reads "---x").
        let lines: Vec<String> = diff
            .unified_diff()
            .context_radius(3)
            .to_string()
            .lines()
            .map(escaped)
            .collect();
        let shows = lines
            .iter()
            .any(|l| l.starts_with('-') || l.starts_with('+'));
        if ct != ot && shows {
            return lines;
        }
    }
    let hex = |at: usize, b: &[u8]| -> String {
        let row = &b[at.min(b.len())..at.saturating_add(16).min(b.len())];
        let bytes: Vec<String> = row.iter().map(|x| format!("{x:02x}")).collect();
        let ascii: String = row
            .iter()
            .map(|&x| {
                if (0x20..0x7f).contains(&x) {
                    x as char
                } else {
                    '.'
                }
            })
            .collect();
        format!("{at:08x}  {:<47}  |{ascii}|", bytes.join(" "))
    };
    let first = (offset / 16).saturating_sub(2) * 16;
    let mut lines = Vec::new();
    let mut at = first;
    while at < first.saturating_add(16 * 12) && (at < c.len() || at < other.len()) {
        let (a, b) = (hex(at, c), hex(at, other));
        if a == b {
            lines.push(format!(" {a}"));
        } else {
            lines.push(format!("-{a}"));
            lines.push(format!("+{b}"));
        }
        at += 16;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_outputs_compare_by_lines_and_bytes_by_hex_rows() {
        let c = b"one\ntwo\nthree\n";
        let o = b"one\nTWO\x1b\nthree\n";
        let lines = outputs_around(c, o, first_difference(c, o));
        assert!(
            lines.iter().any(|l| l.starts_with("-00000000")),
            "a control character makes it bytes: {lines:?}"
        );
        let o = b"one\nTWO\nthree\n";
        let lines = outputs_around(c, o, first_difference(c, o));
        assert!(
            lines.iter().any(|l| l == "-two") && lines.iter().any(|l| l == "+TWO"),
            "{lines:?}"
        );
        let c = [0u8, 1, 2, 3];
        let o = [0u8, 1, 9, 3];
        let lines = outputs_around(&c, &o, 2);
        assert_eq!(
            lines,
            [
                format!("-00000000  {:<47}  |....|", "00 01 02 03"),
                format!("+00000000  {:<47}  |....|", "00 01 09 03"),
            ]
        );
        assert_eq!(escaped("a\x1bb\r"), "a\\x1bb\\r");
    }

    /// The comparison never hides the difference: on a last line with no
    /// newline, when only the final newline differs (each way round), on a
    /// line over 2 KiB that crosses the window's start, and on a line that
    /// itself starts with "--".
    #[test]
    fn the_comparison_always_shows_the_difference() {
        let cmp = |c: &[u8], o: &[u8]| outputs_around(c, o, first_difference(c, o));
        let has = |lines: &[String], l: &str| lines.iter().any(|x| x == l);
        let lines = cmp(b"line1\nabc", b"line1\nabd");
        assert!(has(&lines, "-abc") && has(&lines, "+abd"), "{lines:?}");
        // The Rust drops only the final newline: it did print "result".
        let lines = cmp(b"x\nresult\n", b"x\nresult");
        assert!(
            has(&lines, "-result") && has(&lines, "+result"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("No newline at end of file")),
            "{lines:?}"
        );
        let lines = cmp(b"x\nresult", b"x\nresult\n");
        assert!(
            has(&lines, "-result") && has(&lines, "+result"),
            "{lines:?}"
        );
        // One long line, the difference past its 2048th byte.
        let long = "a".repeat(3000);
        let c = format!("{long}X\nok\n");
        let o = format!("{long}Y\nok\n");
        let lines = cmp(c.as_bytes(), o.as_bytes());
        assert!(
            lines.iter().any(|l| l.starts_with('-') && l.ends_with('X'))
                && lines.iter().any(|l| l.starts_with('+') && l.ends_with('Y')),
            "{lines:?}"
        );
        // A line starting with "--" is the diff's own, never dropped.
        let lines = cmp(b"a\n--x\n", b"a\n--y\n");
        assert!(has(&lines, "---x") && has(&lines, "+--y"), "{lines:?}");
    }
}
