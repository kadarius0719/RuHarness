//! Measuring speed from the cockpit (docs/PERF-DESIGN.md §3.11): the
//! Measure acts' argv and gates, and their confirm dialogs' words — what
//! runs, how many times, about how long (§6, perf's launcher's build
//! counted when its cache is not current), what it writes, no verdict, the
//! ledger's lock, that Cancel keeps finished rows, and to keep the computer
//! quiet.

use super::{os, Act, App, Mode, Pending};
use crate::speed::{Group, SideKey};
use harness_core::perf::estimate::{self, Estimate, Job};
use harness_oracle::LauncherCache;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// How long a Measure dialog waits, at most, to learn whether perf's
/// launcher cache is current (§3.11). The check runs on its own thread:
/// finding the compiler runs `clang --version`, which a loaded Mac can make
/// slow on a new compiler's first exec (§6: 0.25–14 s). Past this wait the
/// dialog opens without the answer.
pub(super) const LAUNCHER_WAIT: Duration = Duration::from_millis(250);

/// `probe`'s answer, asked on a thread of its own and waited for at most
/// `wait`: `None` when it did not come in time (the thread runs on and its
/// answer is dropped) or the thread could not start.
fn ask_within<T: Send + 'static>(probe: fn() -> T, wait: Duration) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("harness-tui-launcher".into())
        .spawn(move || {
            // The dialog may have stopped waiting: a send nobody takes is
            // fine.
            let _ = tx.send(probe());
        })
        .ok()?;
    rx.recv_timeout(wait).ok()
}

/// Whether `harness` is this cockpit's own build: the `harness` next to
/// `exe` (the cockpit's binary), the two compared canonically. Only then
/// does the cockpit's own answer about perf's launcher cache hold for the
/// run: the cache's version folder is named from the launcher sources
/// built into each binary, and the `harness` on PATH may be another build
/// (§3.11). A copy installed apart beside the cockpit still counts as its
/// own — the rule is where it lies, not how it was built.
pub(super) fn same_build(harness: &Path, exe: &Path) -> bool {
    let Some(sibling) = exe.parent().map(|dir| dir.join("harness")) else {
        return false;
    };
    match (harness.canonicalize(), sibling.canonicalize()) {
        (Ok(h), Ok(s)) => h == s,
        _ => false,
    }
}

/// What building perf's launcher adds to §6's estimate, in seconds: the
/// difference it makes to a job that measures nothing (the build and the
/// first execs of perfrun and perfgo).
fn launcher_seconds() -> u64 {
    let nothing = Job {
        workloads: Vec::new(),
        c_alone: false,
        rows: 0,
        crates: 0,
        links: 0,
    };
    match (nothing.estimate_building_launcher(), nothing.estimate()) {
        (Estimate::Seconds(with), Estimate::Seconds(without)) => with.saturating_sub(without),
        // A job with no workload is always a figure in seconds.
        _ => 0,
    }
}

impl App {
    /// Why perf cannot measure now, from the workloads file's state (and
    /// the platform), then the plan's size, in the words `perf run` refuses
    /// with (§3.1 *States*: "Measure is greyed with the same words"; §3.2:
    /// a plan over 999 units); `None` when it can.
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
            _ => self.speed.plan_refused.clone(),
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
        // refuses before it builds anything (§3.10) — said here, before
        // the act is offered.
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

    /// The Measure dialogs' words (§3.11), with what the run makes of
    /// perf's launcher cache asked of `probe` (the dialog passes
    /// [`harness_oracle::perf_launcher_cache`]) off the UI thread, waited
    /// for at most `wait` (the dialog's [`LAUNCHER_WAIT`]). Asked only when
    /// the `harness` the act runs is the cockpit's own build
    /// ([`App::harness_own_build`]): another build may name another version
    /// folder, so its answer is not known here.
    pub(super) fn measure_words(
        &self,
        p: &Pending,
        probe: fn() -> LauncherCache,
        wait: Duration,
    ) -> (String, Vec<String>) {
        let launcher = if self.harness_own_build {
            ask_within(probe, wait)
        } else {
            None
        };
        self.measure_words_with(p, launcher)
    }

    /// [`Self::measure_words`] given what the run makes of perf's launcher
    /// cache (`None`: not known — late, or another build's). Will build: the
    /// estimate counts the launcher's build and first start and says so,
    /// and the dialog names the cache it builds; current: neither (§3.11
    /// "and — when stale — the launcher cache"); cannot use: the plain
    /// estimate, and that perf will refuse and why; not known: the plain
    /// estimate and the hedge "the first time, or after an update".
    fn measure_words_with(
        &self,
        p: &Pending,
        launcher: Option<LauncherCache>,
    ) -> (String, Vec<String>) {
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
        let (estimate, building) = match launcher {
            Some(LauncherCache::WillBuild) => {
                (job.estimate_building_launcher(), Some(launcher_seconds()))
            }
            _ => (job.estimate(), None),
        };
        body.push(match estimate {
            e @ Estimate::Seconds(_) => {
                let launcher_words = building.map_or(String::new(), |s| {
                    format!(" — about {s} s of it for perf's launcher (its build and first start)")
                });
                format!(
                    "Takes {}, builds included{launcher_words}{cold_words}.",
                    e.words()
                )
            }
            e => {
                let launcher_words = building.map_or(String::new(), |s| {
                    format!(", plus about {s} s for perf's launcher (its build and first start)")
                });
                let w = e.words();
                let mut c = w.chars();
                let first = c.next().map(|f| f.to_uppercase().collect::<String>());
                format!(
                    "{}{}{launcher_words}{cold_words}.",
                    first.unwrap_or_default(),
                    c.as_str()
                )
            }
        });
        let cache_words = match &launcher {
            Some(LauncherCache::Current) | Some(LauncherCache::CannotUse(_)) => "",
            Some(LauncherCache::WillBuild) => {
                "; first it builds perf's launcher into ~/Library/Caches/ruharness/perf (its \
                 cache is missing or out of date)"
            }
            None => {
                "; the first time, or after an update, it builds perf's launcher into \
                 ~/Library/Caches/ruharness/perf"
            }
        };
        body.push(format!(
            "Writes migration/perf/ (its rows replace the ones they measure again) and scratch \
             folders under migration/build/ (.perf, .perf-out with the kept outputs, \
             perf-logs){cache_words}."
        ));
        if let Some(LauncherCache::CannotUse(why)) = &launcher {
            body.push(format!(
                "perf will refuse before it measures: {}{}",
                why.trim_end(),
                if why.trim_end().ends_with(['.', '!', '?']) {
                    ""
                } else {
                    "."
                }
            ));
        }
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

    /// zopfli with a workloads file and nothing measured yet: the Measure
    /// acts are offered (on macOS), u001 the one verified unit.
    fn measurable_app(tag: &str) -> App {
        let target = crate::testutil::scratch_target("targets/zopfli", tag);
        let perf = harness_core::perf::perf_dir(&target);
        std::fs::create_dir_all(target.join("bench")).unwrap();
        std::fs::write(target.join("bench/big.txt"), "big ".repeat(1000)).unwrap();
        std::fs::write(target.join("bench/small.txt"), "small\n").unwrap();
        std::fs::create_dir_all(&perf).unwrap();
        std::fs::write(
            perf.join("workloads.toml"),
            "schema_version = 1\n\
             [[workload]]\nid = \"big-text\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/big.txt\"\n\
             [[workload]]\nid = \"many-small\"\nargs = [\"-c\", \"{input}\"]\ninput = \"bench/small.txt\"\n",
        )
        .unwrap();
        crate::app::tests::app_of_path(&target)
    }

    /// A plan over perf's 999 slots: Measure is greyed with perf's own
    /// refusal, before any dialog — u001, measurable in a smaller plan,
    /// included.
    #[test]
    fn a_plan_over_999_units_greys_measure_with_perfs_words() {
        if !cfg!(target_os = "macos") {
            // Greyed off macOS already, with the platform's words.
            return;
        }
        let app = measurable_app("gate-1000-units");
        assert_eq!(app.speed_gate(), None);
        let target = app.config.target.clone();
        let plan = target.join("migration/plan.toml");
        let mut text = std::fs::read_to_string(&plan).unwrap();
        let have = text.matches("[[unit]]").count();
        for i in 0..(1000 - have) {
            text.push_str(&format!(
                "\n[[unit]]\nid = \"u-pad-{i:04}\"\nstatus = \"pending\"\nfiles = []\n"
            ));
        }
        std::fs::write(&plan, text).unwrap();
        let app = crate::app::tests::app_of_path(&target);
        let _ = std::fs::remove_dir_all(&target);
        let words = "perf measures a plan of at most 999 units — this plan has 1000";
        assert_eq!(app.speed_gate().as_deref(), Some(words));
        assert!(app.speed.measurable.is_empty());
        for (act, unit) in [
            (Act::Measure, None),
            (Act::Measure, Some("u001-katajainen")),
            (Act::MeasureProgram, None),
        ] {
            assert_eq!(
                app.measure_argv(act, unit).map(|(_, label)| label),
                Err(words.to_string())
            );
        }
    }

    /// The seconds of a "Takes about N s, …" line.
    fn seconds(takes: &str) -> u64 {
        takes
            .strip_prefix("Takes about ")
            .and_then(|r| r.split_once(" s,"))
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or_else(|| panic!("not a figure in seconds: {takes}"))
    }

    /// §3.11, §6: perf's launcher's build is in the Measure dialogs' figure,
    /// and its cache named, only when the run will build it — the answer
    /// injected, never the machine's own cache; not known in time, the
    /// figure leaves it out and the words keep the hedge; a cache perf
    /// cannot use, the plain figure and perf's refusal in its own words.
    #[test]
    fn the_measure_dialog_counts_the_launcher_build_only_when_it_is_not_built() {
        if !cfg!(target_os = "macos") {
            // Measure is greyed off macOS: no dialog opens.
            return;
        }
        let mut app = measurable_app("speed-launcher");
        assert_eq!(app.speed.measurable, ["u001-katajainen"]);
        let launcher = launcher_seconds();
        assert_eq!(
            launcher as f64,
            estimate::LAUNCHER_BUILD_SECONDS + 2.0 * estimate::FIRST_EXEC_SECONDS
        );
        let says_launcher = |body: &[String]| body.iter().any(|l| l.contains("launcher"));
        let refusal = "your TMPDIR holds perf's launcher folder, which every build tool may \
                       write — set TMPDIR elsewhere";
        let cannot_use = || Some(LauncherCache::CannotUse(refusal.into()));
        // u001's crate may be cold in the copy: its build is said apart.
        let cold = ", plus building u001-katajainen's Rust, which may take minutes";
        for (act, unit) in [
            (Act::Measure, None),
            (Act::Measure, Some("u001-katajainen")),
        ] {
            // The C's time known on both workloads: a figure in seconds.
            app.speed.c_clock = [("big-text".to_string(), 0.01), ("many-small".into(), 0.01)]
                .into_iter()
                .collect();
            let p = app.act_argv(act, unit, None, None).expect("offered");
            let (_, built) = app.measure_words_with(&p, Some(LauncherCache::Current));
            let (_, not_built) = app.measure_words_with(&p, Some(LauncherCache::WillBuild));
            let (_, unknown) = app.measure_words_with(&p, None);
            let (_, cannot) = app.measure_words_with(&p, cannot_use());
            assert!(!says_launcher(&built), "built: {built:?}");
            assert_eq!(
                not_built[1].replace(cold, ""),
                format!(
                    "Takes about {} s, builds included — about {launcher} s of it for perf's \
                     launcher (its build and first start).",
                    seconds(&built[1]) + launcher
                ),
                "{not_built:?}"
            );
            assert_eq!(not_built[1].contains(cold), built[1].contains(cold));
            assert!(
                not_built[2].ends_with(
                    "; first it builds perf's launcher into ~/Library/Caches/ruharness/perf (its \
                     cache is missing or out of date)."
                ),
                "{not_built:?}"
            );
            assert_eq!(unknown[1], built[1], "not known: the figure leaves it out");
            assert!(
                unknown[2].ends_with(
                    "; the first time, or after an update, it builds perf's launcher into \
                     ~/Library/Caches/ruharness/perf."
                ),
                "{unknown:?}"
            );
            // Cannot use: the plain figure, no cache written, and perf's
            // refusal said — the rest as when it is current.
            assert_eq!(cannot[1], built[1], "cannot use: no build counted");
            assert_eq!(cannot[2], built[2], "cannot use: no cache written");
            assert_eq!(
                cannot
                    .iter()
                    .filter(|l| !built.contains(l))
                    .collect::<Vec<_>>(),
                [&format!("perf will refuse before it measures: {refusal}.")],
                "{cannot:?}"
            );
            // The C's time not known: no figure, the launcher's build still
            // said with its seconds.
            app.speed.c_clock.clear();
            let (_, built) = app.measure_words_with(&p, Some(LauncherCache::Current));
            let (_, not_built) = app.measure_words_with(&p, Some(LauncherCache::WillBuild));
            let (_, cannot) = app.measure_words_with(&p, cannot_use());
            assert!(
                built[1].starts_with("The C's time is not known yet"),
                "{built:?}"
            );
            assert!(!says_launcher(&built), "built: {built:?}");
            assert_eq!(
                not_built[1].replace(cold, ""),
                format!(
                    "{}, plus about {launcher} s for perf's launcher (its build and first \
                     start).",
                    built[1].replace(cold, "").trim_end_matches('.')
                ),
                "{not_built:?}"
            );
            assert_eq!(cannot[1], built[1], "cannot use: no build counted");
        }
    }

    /// The launcher check never holds a dialog: a slow answer is waited
    /// for at most the dialog's own wait, then the dialog goes on without
    /// it — through the dialog's own words, so a check made on the UI
    /// thread fails here.
    #[test]
    fn a_slow_launcher_check_never_holds_the_dialog() {
        fn slow() -> bool {
            std::thread::sleep(Duration::from_secs(5));
            true
        }
        fn slow_cache() -> LauncherCache {
            std::thread::sleep(Duration::from_secs(5));
            LauncherCache::Current
        }
        let started = std::time::Instant::now();
        assert_eq!(ask_within(slow, LAUNCHER_WAIT), None);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(ask_within(|| false, Duration::from_secs(30)), Some(false));
        assert_eq!(ask_within(|| true, Duration::from_secs(30)), Some(true));
        if !cfg!(target_os = "macos") {
            // Measure is greyed off macOS: no dialog opens.
            return;
        }
        let mut app = measurable_app("speed-launcher-slow");
        app.harness_own_build = true;
        let p = app
            .act_argv(Act::Measure, None, None, None)
            .expect("offered");
        let started = std::time::Instant::now();
        let (_, words) = app.measure_words(&p, slow_cache, LAUNCHER_WAIT);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the dialog waited {:?}",
            started.elapsed()
        );
        assert_eq!(words, app.measure_words_with(&p, None).1, "the hedge");
    }

    /// The Measure dialog asks the machine's own launcher cache and reads
    /// its answer the right way round: its words are those for the answer
    /// `perf_launcher_cache` gives (or, not known in time, the hedge) —
    /// never those for another answer.
    #[test]
    fn the_measure_dialog_reads_the_machines_launcher_cache() {
        if !cfg!(target_os = "macos") {
            // Measure is greyed off macOS: no dialog opens.
            return;
        }
        let mut app = measurable_app("speed-launcher-own");
        // The harness the act runs is the cockpit's own build.
        app.harness_own_build = true;
        let p = app
            .act_argv(Act::Measure, None, None, None)
            .expect("offered");
        // Asked first, so the dialog's own ask finds the compiler warm.
        let cached = harness_oracle::perf_launcher_cache();
        let other = match cached {
            LauncherCache::Current => LauncherCache::WillBuild,
            _ => LauncherCache::Current,
        };
        let (_, right) = app.measure_words_with(&p, Some(cached));
        let (_, wrong) = app.measure_words_with(&p, Some(other));
        let (_, late) = app.measure_words_with(&p, None);
        assert_ne!(right, wrong);
        // A loaded machine may answer late: the dialog is asked again.
        let words = (0..5)
            .map(|_| {
                app.measure_words(&p, harness_oracle::perf_launcher_cache, LAUNCHER_WAIT)
                    .1
            })
            .find(|w| *w != late)
            .unwrap_or(late.clone());
        assert_ne!(words, wrong, "the answer read the wrong way round");
        assert!(words == right || words == late, "{words:?}");
    }

    /// The cockpit and `perf show` read perf's launcher cache alike: when
    /// `perf show` checks the computer through it, the cockpit calls it
    /// current (both are built on the one read of the cache, which never
    /// builds it).
    #[test]
    fn the_cockpit_reads_the_launcher_cache_as_perf_show_does() {
        let computer = harness_oracle::perf_computer_if_cached();
        let cached = ask_within(harness_oracle::perf_launcher_cache, Duration::from_secs(60));
        if computer.is_some() {
            assert_eq!(cached, Some(LauncherCache::Current), "{computer:?}");
        } else {
            assert!(cached.is_some(), "the check answers");
        }
    }

    /// The run's `harness` another build (§3.11): the cockpit's own answer
    /// about the launcher cache does not hold for it, so the dialog keeps
    /// the hedge whatever the answer — and gives the answer for its own.
    #[test]
    fn another_harness_build_gets_the_hedge() {
        if !cfg!(target_os = "macos") {
            // Measure is greyed off macOS: no dialog opens.
            return;
        }
        fn current() -> LauncherCache {
            LauncherCache::Current
        }
        // The test app's harness is /opt/ruharness/bin/harness, never the
        // one next to the test binary.
        let mut app = measurable_app("speed-launcher-other");
        assert!(!app.harness_own_build);
        let p = app
            .act_argv(Act::Measure, None, None, None)
            .expect("offered");
        let hedge = app.measure_words_with(&p, None).1;
        let current_words = app.measure_words_with(&p, Some(LauncherCache::Current)).1;
        assert_ne!(hedge, current_words);
        let words = app.measure_words(&p, current, Duration::from_secs(30)).1;
        assert_eq!(words, hedge, "another build: not known");
        app.harness_own_build = true;
        let words = app.measure_words(&p, current, Duration::from_secs(30)).1;
        assert_eq!(words, current_words, "its own build: the answer");
    }

    /// Own build: the `harness` next to the cockpit's binary, compared
    /// canonically (a link to it counts); any other path, or none there,
    /// does not.
    #[test]
    fn own_build_is_the_harness_next_to_the_cockpit() {
        let dir = crate::testutil::TmpDir::new("own-build");
        let bin = dir.0.join("bin");
        let elsewhere = dir.0.join("elsewhere");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let exe = bin.join("harness-tui");
        std::fs::write(&exe, "").unwrap();
        let other = elsewhere.join("harness");
        std::fs::write(&other, "").unwrap();
        // No harness next to it yet.
        assert!(!same_build(&other, &exe));
        let sibling = bin.join("harness");
        std::fs::write(&sibling, "").unwrap();
        assert!(same_build(&sibling, &exe));
        assert!(!same_build(&other, &exe));
        let link = elsewhere.join("harness-link");
        std::os::unix::fs::symlink(&sibling, &link).unwrap();
        assert!(same_build(&link, &exe), "a link to it is it");
        assert!(same_build(&bin.join("../bin/harness"), &exe));
        let this = std::env::current_exe().unwrap();
        assert!(!same_build(Path::new(crate::app::tests::HARNESS), &this));
    }

    /// `App::new` records the run's `harness` as the cockpit's own build
    /// when it is the one next to the cockpit's binary — and not otherwise
    /// (the cockpit's path given, so no file is put beside the test binary).
    #[test]
    fn the_harness_next_to_the_cockpit_is_its_own_build() {
        let dir = crate::testutil::TmpDir::new("own-build-app");
        let exe = dir.0.join("harness-tui");
        let sibling = dir.0.join("harness");
        std::fs::write(&exe, "").unwrap();
        std::fs::write(&sibling, "").unwrap();
        let target = crate::testutil::scratch_target("targets/zopfli", "own-build-target");
        let app = |harness: &Path, exe: Option<&Path>| {
            let read = crate::load::read(&target).unwrap();
            App::new_from(
                crate::app::Config {
                    target: target.clone(),
                    harness: Some(harness.to_path_buf()),
                    allow_unsandboxed: false,
                    layout: crate::app::LayoutMode::Auto,
                    providers: vec!["external".into()],
                },
                read,
                exe,
            )
        };
        let own = app(&sibling, Some(&exe)).harness_own_build;
        let other = app(Path::new(crate::app::tests::HARNESS), Some(&exe)).harness_own_build;
        let unknown = app(&sibling, None).harness_own_build;
        let _ = std::fs::remove_dir_all(&target);
        assert!(own, "the harness next to the cockpit is its own build");
        assert!(!other, "another harness is not");
        assert!(!unknown, "no cockpit path: not known");
    }
}
