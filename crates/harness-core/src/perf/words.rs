//! What perf says about a row (docs/PERF-DESIGN.md §3.8 and build notes
//! 10–21): computed from the stored numbers and closed values on every
//! read, the same in the CLI, the cockpit and the MCP reads. Std only.

use super::results::{Difference, Row, Run, SetupFacts};
use super::stats::{self, Shift};

/// The time margin, in percent.
pub const TIME_MARGIN: f64 = 2.0;
/// The instructions margin, in percent.
pub const INSTRUCTIONS_MARGIN: f64 = 1.5;
/// The floor of a full run: instructions.
pub const FLOOR_INSTRUCTIONS: f64 = 1e9;
/// The floor of a full run: CPU time, in microseconds.
pub const FLOOR_CPU_US: f64 = 500_000.0;
/// Above this median footprint (either side) a short run keeps its memory
/// line.
pub const MEMORY_LINE_BYTES: f64 = 4.0 * 1024.0 * 1024.0;
/// Rust's fixed start-up, in instructions (§6).
pub const STARTUP_INSTRUCTIONS: f64 = 1.04e7;
/// The start-up note's interval's highest upper end (build note 18).
pub const STARTUP_UPPER: f64 = 2.6e7;
/// The widest a short form may be, in columns.
pub const SHORT_WIDTH: usize = 26;
/// The CLI's width.
pub const CLI_WIDTH: usize = 80;

/// Which side a row measures against the C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side<'a> {
    /// The C alone.
    C,
    /// A unit's program: the C with only that unit's Rust.
    Unit(&'a str),
    /// The program as it stands.
    AsItStands,
}

impl Side<'_> {
    /// How a sentence names the other side's code.
    fn rust(&self) -> String {
        match self {
            Side::C => "the C".into(),
            Side::Unit(id) => format!("{id}'s Rust"),
            Side::AsItStands => "the program as it stands".into(),
        }
    }
}

/// What a row's words need beside the row.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The side.
    pub side: Side<'a>,
    /// The workload's id.
    pub workload: &'a str,
    /// The workload's input as written, when it has one.
    pub input: Option<&'a str>,
}

/// Every value [`RowWords::answer`] takes — the MCP's closed set: the time
/// answers, then each outcome's own.
pub const ANSWERS: &[&str] = &[
    "about-as-fast",
    "slower",
    "faster",
    "probably-slower",
    "probably-faster",
    "close-call-slower",
    "close-call-faster",
    "no-clear-difference",
    "cant-tell-estimate",
    "cant-tell-short-run",
    "cant-tell-too-few",
    "cant-tell-slow-cores",
    "baseline",
    "too-short",
    "behaves-differently",
    "stopped-by-sigkill",
    "run-failed-timeout",
    "run-failed-exit",
    "run-failed-signal",
    "c-unstable",
    "c-crashed",
    "c-timed-out",
    "output-too-large",
    "c-could-not-start",
    "could-not-start",
    "not-verified",
    "replaces-mismatch",
    "crate-does-not-build",
    "does-not-link",
    "mixed-panic",
    "input-unusable",
    "run-failed: unmeasurable",
];

/// A row's words.
#[derive(Debug, Clone, PartialEq)]
pub struct RowWords {
    /// The headline (after "perf: <side> on <workload> — ").
    pub headline: String,
    /// The details, each a fragment the CLI joins with " · ".
    pub details: Vec<String>,
    /// The short form, at most [`SHORT_WIDTH`] columns.
    pub short: String,
    /// The worst-first order's key: lower is worse; ties by `rank.1`,
    /// higher first.
    pub rank: (u8, f64),
    /// The MCP answer, from a closed set.
    pub answer: &'static str,
    /// The row's time shift and interval in percent, only when the answer
    /// is not a can't-tell kind.
    pub shift: Option<(f64, f64, f64)>,
    /// Whether "measure again with 31 runs" applies (build note 12).
    pub offers_more_runs: bool,
}

/// The time answer of a measured row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    /// Inside ±M.
    AboutAsFast,
    /// Past +M.
    Slower,
    /// Past −M.
    Faster,
    /// Near the line: narrow, excluding 0, not past M.
    CloseCall {
        /// Toward slower.
        slower: bool,
    },
    /// Excluding 0, not past M.
    Probably {
        /// Toward slower.
        slower: bool,
    },
    /// Holding 0 at 31 runs.
    NoClearDifference,
    /// Holding 0 below 31 runs.
    CantTellEstimate,
    /// A short run that would not be past the line.
    ShortRun,
    /// Fewer than five values on a side.
    TooFew,
    /// The share rule failed and the two metrics disagree.
    SlowCores,
}

/// Which branch an interval `[lo, hi]` (percent) takes against margin `m`.
pub fn branch(lo: f64, hi: f64, m: f64, runs: u32, short: bool) -> Answer {
    if lo >= -m && hi <= m {
        return if short {
            Answer::ShortRun
        } else {
            Answer::AboutAsFast
        };
    }
    if lo > m {
        return Answer::Slower;
    }
    if hi < -m {
        return Answer::Faster;
    }
    if lo > 0.0 || hi < 0.0 {
        let slower = lo > 0.0;
        return if hi - lo <= 2.0 * m {
            Answer::CloseCall { slower }
        } else {
            Answer::Probably { slower }
        };
    }
    if short {
        Answer::ShortRun
    } else if runs >= stats::MAX_RUNS as u32 {
        Answer::NoClearDifference
    } else {
        Answer::CantTellEstimate
    }
}

// ---- Numbers -----------------------------------------------------------

/// A percent at the design's precision: one decimal under 10, whole above.
fn decimals(v: f64) -> usize {
    if v.abs() < 9.95 {
        1
    } else {
        0
    }
}

fn fmt_fixed(v: f64, d: usize) -> String {
    let s = format!("{v:.d$}");
    if s == "-0.0" || s == "-0" || s == "-0.00" {
        s[1..].to_string()
    } else {
        s
    }
}

/// `v` at the design's precision, nearest.
pub fn pct(v: f64) -> String {
    fmt_fixed(v, decimals(v))
}

/// A margin: whole when it is one, else one decimal (`2`, `1.5`, `8.5`).
pub fn margin(m: f64) -> String {
    if (m - m.round()).abs() < 0.05 {
        format!("{}", m.round() as u64)
    } else {
        format!("{m:.1}")
    }
}

/// `v` rounded up at the design's precision (Y in "within ±Y %").
pub fn pct_up(v: f64) -> String {
    let d = decimals(v);
    let scale = 10f64.powi(d as i32);
    let up = (v * scale - 1e-9).ceil() / scale;
    fmt_fixed(up.max(0.0), decimals(up))
}

/// An interval end at the design's precision, unless that would put it on
/// or past `boundary` from `v`'s side: then two decimals, rounded away
/// from the boundary (build note 13).
pub fn end(v: f64, boundaries: &[f64]) -> String {
    let near = pct(v);
    let shown: f64 = near.parse().unwrap_or(v);
    for b in boundaries {
        let crossed = (v > *b && shown <= *b) || (v < *b && shown >= *b);
        if crossed {
            let away = if v > *b {
                (v * 100.0).ceil() / 100.0
            } else {
                (v * 100.0).floor() / 100.0
            };
            return fmt_fixed(away, 2);
        }
    }
    near
}

/// "(a–b %)" from two shown ends; one number when they show equal.
fn interval(a: &str, b: &str, unit: &str) -> String {
    if a == b {
        format!("({a}{unit})")
    } else {
        format!("({a}–{b}{unit})")
    }
}

/// Seconds for people: three significant figures at a second or more, two
/// decimals under, milliseconds under a tenth.
pub fn seconds(us: f64) -> String {
    let s = us / 1e6;
    if s >= 1.0 {
        format!("{} s", sig3(s))
    } else if s >= 0.1 {
        format!("{s:.2} s")
    } else {
        format!("{} ms", (us / 1e3).round().max(1.0) as u64)
    }
}

/// Three significant figures.
fn sig3(v: f64) -> String {
    if v >= 100.0 {
        format!("{}", v.round() as u64)
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

/// Bytes for people: three significant figures in kB, MB or GB (decimal).
pub fn bytes(b: f64) -> String {
    if b >= 1e9 {
        format!("{} GB", sig3(b / 1e9))
    } else if b >= 1e6 {
        format!("{} MB", sig3(b / 1e6))
    } else if b >= 1e3 {
        format!("{} kB", sig3(b / 1e3))
    } else {
        format!("{} B", b.round() as u64)
    }
}

/// A count in scientific notation, three significant figures: `1.21e10`.
pub fn count(v: f64) -> String {
    if v < 1e4 {
        return format!("{}", v.round() as u64);
    }
    let exp = v.log10().floor() as i32;
    let mut mant = v / 10f64.powi(exp);
    let mut exp = exp;
    if (mant * 100.0).round() / 100.0 >= 10.0 {
        mant /= 10.0;
        exp += 1;
    }
    format!("{mant:.2}e{exp}")
}

/// A whole number with spaces between thousands: `40 961`.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// A size factor: up to one decimal below 2 (never "1×"), whole above,
/// rounded up.
pub fn factor(f: f64) -> String {
    if f < 2.0 {
        let up = ((f * 10.0) - 1e-9).ceil() / 10.0;
        format!("{:.1}×", up.max(1.1))
    } else {
        format!("{}×", (f - 1e-9).ceil() as u64)
    }
}

// ---- Metrics -----------------------------------------------------------

/// A side's values of one metric, run by run.
fn values(runs: &[Run], f: impl Fn(&Run) -> Option<f64>) -> Vec<Option<f64>> {
    runs.iter().map(f).collect()
}

fn cycles(r: &Run) -> Option<f64> {
    r.cycles.map(|v| v as f64)
}

fn cpu(r: &Run) -> Option<f64> {
    r.cpu_us.map(|v| v as f64)
}

fn instructions(r: &Run) -> Option<f64> {
    r.instructions.map(|v| v as f64)
}

/// Performance-core-normalised cycles: P-core cycles per P-core
/// instruction × all instructions; a run with no P-core cycles has none.
fn pnorm(r: &Run) -> Option<f64> {
    let (pc, pi, i) = (r.p_cycles?, r.p_instructions?, r.instructions?);
    if pc == 0 || pi == 0 {
        return None;
    }
    Some(pc as f64 / pi as f64 * i as f64)
}

/// Whether a run had at least half its cycles on the performance cores.
fn mostly_fast(r: &Run) -> bool {
    match (r.p_cycles, r.cycles) {
        (Some(p), Some(c)) if c > 0 => p * 2 >= c,
        _ => false,
    }
}

/// The platforms' families, for [`choose_metric`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS with `RUSAGE_INFO_V6`.
    MacV6,
    /// macOS with only `RUSAGE_INFO_V4` (no P fields).
    MacV4,
    /// Linux.
    Linux,
    /// Linux on a computer with two kinds of cores (per-type counts
    /// summed).
    LinuxHybrid,
}

/// The time metric a measured row uses (§3.8, build note 10): CPU time
/// without counters; raw cycles without two kinds of cores; the share rule
/// when fewer than three quarters of the 2n runs ran mostly on the
/// performance cores; else normalised cycles when the P-core cost per
/// instruction is steady (or ≥ 97 % of the cycles were on the performance
/// cores), raw cycles ("phases") when it is not.
pub fn choose_metric(
    c: &[Run],
    other: &[Run],
    platform: Platform,
    two_kinds: bool,
) -> &'static str {
    let all: Vec<&Run> = c.iter().chain(other.iter()).collect();
    let counted = all
        .iter()
        .filter(|r| r.cycles.is_some_and(|v| v > 0))
        .count();
    if counted * 2 < all.len().max(1) {
        return "cpu-time";
    }
    match platform {
        Platform::MacV4 => return "macos-v4-cycles",
        Platform::Linux => return "linux-cycles",
        Platform::LinuxHybrid => return "linux-hybrid-summed",
        Platform::MacV6 if !two_kinds => return "macos-v6-cycles",
        Platform::MacV6 => {}
    }
    let fast = all.iter().filter(|r| mostly_fast(r)).count();
    if fast * 4 < all.len() * 3 {
        return "macos-v6-share";
    }
    let p_total: u64 = all.iter().filter_map(|r| r.p_cycles).sum();
    let total: u64 = all.iter().filter_map(|r| r.cycles).sum();
    if total > 0 && p_total as f64 >= 0.97 * total as f64 {
        return "macos-v6-pnorm";
    }
    let side_spreads = |runs: &[Run]| -> Option<(f64, f64)> {
        let cost: Vec<f64> = runs
            .iter()
            .filter_map(|r| match (r.p_cycles, r.p_instructions) {
                (Some(pc), Some(pi)) if pc > 0 && pi > 0 => Some((pc as f64 / pi as f64).ln()),
                _ => None,
            })
            .collect();
        let raw: Vec<f64> = runs.iter().filter_map(|r| cycles(r).map(f64::ln)).collect();
        Some((stats::robust_spread(&cost)?, stats::robust_spread(&raw)?))
    };
    match (side_spreads(c), side_spreads(other)) {
        (Some((pc, rc)), Some((po, ro))) if (pc + po) / 2.0 > (rc + ro) / 2.0 => {
            "macos-v6-cycles-phases"
        }
        (Some(_), Some(_)) => "macos-v6-pnorm",
        _ => "macos-v6-cycles",
    }
}

/// The detail naming the metric, when it is not the usual one.
fn metric_note(metric: &str) -> Option<&'static str> {
    match metric {
        "macos-v6-cycles" | "macos-v4-cycles" | "linux-cycles" | "linux-hybrid-summed" => {
            Some("cycles")
        }
        "macos-v6-cycles-phases" => Some("cycles — the program's phases differ in speed"),
        "cpu-time" => Some("CPU time (no counters)"),
        _ => None,
    }
}

// ---- Words -------------------------------------------------------------

/// A row's words (see [`RowWords`]).
pub fn words(row: &Row, cx: &Context) -> RowWords {
    let mut w = match row.outcome.as_str() {
        "baseline" => baseline(row, cx),
        "measured" => measured(row, cx),
        "too-short" => too_short(row, cx),
        "behaves-differently" => behaves_differently(row, cx),
        "stopped-by-sigkill" => fixed(
            format!(
                "{} was stopped by a SIGKILL perf did not send",
                side_subject(cx)
            ),
            "stopped by SIGKILL",
            (2, 0.0),
            "stopped-by-sigkill",
        ),
        o if o.starts_with("run-failed: ") && o != "run-failed: unmeasurable" => {
            run_failed(row, cx)
        }
        "c-unstable" => fixed(
            "the C ends or prints differently from one run to the next — perf cannot compare \
             against it"
                .into(),
            "C output unstable",
            (0, 0.0),
            "c-unstable",
        ),
        "c-crashed" => fixed(
            format!("the C crashes on {}{}", cx.workload, c_end_words(row)),
            "C crashed",
            (0, 0.0),
            "c-crashed",
        ),
        "c-timed-out" => fixed(
            format!("the C timed out on {}", cx.workload),
            "C timed out",
            (0, 0.0),
            "c-timed-out",
        ),
        "output-too-large" => fixed(
            "the C prints more than 64 MiB; give it an output-file argument — then perf times it \
             but no longer compares its output"
                .into(),
            "C output too large",
            (0, 0.0),
            "output-too-large",
        ),
        "c-could-not-start" => fixed(
            format!("the C never started{}", never_started(row.setup.as_ref())),
            "C could not start",
            (0, 0.0),
            "c-could-not-start",
        ),
        "could-not-start" => fixed(
            format!(
                "{} never started{}",
                side_subject(cx),
                never_started(row.setup.as_ref())
            ),
            "could not start",
            (8, 0.0),
            "could-not-start",
        ),
        o => fixed(
            set_up_words(o, row.setup.as_ref(), cx),
            "not measured",
            (9, 0.0),
            static_outcome(o),
        ),
    };
    if let Some(t) = &row.last_try {
        w.details.push(format!(
            "last try: {}",
            match t.outcome.as_str() {
                o if super::results::is_c_side(o) => c_side_words(o, cx),
                o => set_up_words(o, t.setup.as_ref(), cx),
            }
        ));
    }
    if let Some(d) = &row.found_before {
        w.details.push(format!(
            "a difference found before: {} — measure again to check",
            difference_words(d)
        ));
    }
    w
}

fn static_outcome(o: &str) -> &'static str {
    super::results::OUTCOMES
        .iter()
        .find(|x| **x == o)
        .copied()
        .unwrap_or("not-verified")
}

fn fixed(headline: String, short: &str, rank: (u8, f64), answer: &'static str) -> RowWords {
    RowWords {
        headline,
        details: Vec::new(),
        short: short.into(),
        rank,
        answer,
        shift: None,
        offers_more_runs: false,
    }
}

fn side_subject(cx: &Context) -> String {
    match cx.side {
        Side::C => "the C".into(),
        Side::Unit(id) => format!("the program with {id}'s Rust"),
        Side::AsItStands => "the program as it stands".into(),
    }
}

/// The end of the C's step-1 run that crashed — the first of the two that
/// ended by a signal, which need not be the first run.
fn c_end_words(row: &Row) -> String {
    row.step1
        .as_ref()
        .and_then(|s| {
            [&s.c_first, &s.c_second]
                .into_iter()
                .find(|r| r.end.starts_with("signal "))
        })
        .map(|r| format!(" ({})", r.end))
        .unwrap_or_default()
}

fn never_started(setup: Option<&SetupFacts>) -> String {
    match setup.and_then(|s| s.never_started.as_deref()) {
        Some("no-ready") => " (the launcher's trampoline never ran)".into(),
        Some(errno) => format!(" (exec failed: errno {errno})"),
        None => String::new(),
    }
}

fn c_side_words(o: &str, cx: &Context) -> String {
    match o {
        "c-unstable" => "the C ends or prints differently from one run to the next".into(),
        "c-crashed" => format!("the C crashes on {}", cx.workload),
        "c-timed-out" => format!("the C timed out on {}", cx.workload),
        "output-too-large" => "the C prints more than 64 MiB".into(),
        _ => "the C never started".into(),
    }
}

/// The words of a set-up outcome, rebuilt from its facts (§3.9).
pub fn set_up_words(o: &str, setup: Option<&SetupFacts>, cx: &Context) -> String {
    let unit = match cx.side {
        Side::Unit(id) => id.to_string(),
        _ => "the unit".into(),
    };
    let s = setup.cloned().unwrap_or_default();
    let log = s
        .log
        .as_deref()
        .map(|l| format!(" — see migration/build/perf-logs/{l}"))
        .unwrap_or_default();
    match o {
        "not-verified" => match s.reason.as_deref() {
            Some("replaces-changed") => {
                format!("{unit}'s replaced files changed since verify — Re-check it")
            }
            Some("rust-changed") => format!("{unit}'s Rust changed since verify — Re-check it"),
            Some("accept-interrupted") => format!(
                "{}; Measure does not",
                super::accept_interrupted_words(s.attempt.as_deref().unwrap_or("legacy"), &unit)
            ),
            _ => format!("{unit} is not verified fresh today — Re-check it"),
        },
        "replaces-mismatch" => format!(
            "{unit}'s replaces entry {} names no top-level C file of the program — Re-check it",
            s.index.map(|i| i + 1).unwrap_or(1)
        ),
        "crate-does-not-build" => format!("{unit}'s crate does not build{log}"),
        "does-not-link" => {
            let units = s.units.unwrap_or_default();
            let first = units.first().cloned().unwrap_or_else(|| unit.clone());
            match s.cause.as_deref() {
                Some("no-std") => {
                    format!("{first} uses no std — it cannot be linked beside units that use std")
                }
                Some("two-no-std") => format!(
                    "two units use no std ({}) — they cannot be linked together",
                    units.join(", ")
                ),
                Some("lto") => format!(
                    "{first} is built with lto — it cannot be linked beside another Rust unit; \
                     set lto = false in its Cargo.toml and run harness verify {first}"
                ),
                Some("two-lto") => format!(
                    "two units are built with lto ({}) — set lto = false in their Cargo.toml \
                     and verify them again",
                    units.join(", ")
                ),
                _ => format!("the program did not link{log}"),
            }
        }
        "mixed-panic" => {
            let r = s.runtimes.unwrap_or_default();
            let found: Vec<String> = r
                .iter()
                .filter(|u| u.runtime != "none")
                .map(|u| {
                    let how = if u.runtime == "abort" {
                        "aborts"
                    } else {
                        "unwinds"
                    };
                    format!("{} {how}", u.id)
                })
                .collect();
            let unwinding: Vec<&str> = r
                .iter()
                .filter(|u| u.runtime == "unwind")
                .map(|u| u.id.as_str())
                .collect();
            let first = unwinding.first().copied().unwrap_or("the unwinding unit");
            format!(
                "{} — the program as it stands would silently use one. Add [profile.release] \
                 panic = \"abort\" to {first}'s Cargo.toml, then run harness verify {first}",
                found.join("; ")
            )
        }
        "input-unusable" => {
            let reason = s
                .input
                .as_deref()
                .and_then(super::workloads::InputUnusable::from_token)
                .unwrap_or(super::workloads::InputUnusable::Unreadable);
            reason.words(cx.input.unwrap_or("its input"))
        }
        "run-failed: unmeasurable" => "the launcher stopped before measuring".into(),
        "could-not-start" => format!(
            "{} never started{}",
            side_subject(cx),
            never_started(Some(&s))
        ),
        other => other.to_string(),
    }
}

fn baseline(row: &Row, _cx: &Context) -> RowWords {
    let c = row.c.as_deref().unwrap_or_default();
    let cpu_med = stats::median(&values(c, cpu));
    let mem_med = stats::median(&values(c, |r| r.memory.map(|v| v as f64)));
    let ins_med = stats::median(&values(c, instructions));
    let mut parts = Vec::new();
    if let Some(t) = cpu_med {
        parts.push(format!(
            "CPU about {} here today (varies with load)",
            seconds(t)
        ));
    }
    if let Some(m) = mem_med {
        parts.push(bytes(m));
    }
    if let Some(i) = ins_med {
        parts.push(format!("{} instructions", count(i)));
    }
    let short = match (cpu_med, mem_med) {
        (Some(t), Some(m)) => format!("CPU {} · {}", seconds(t), bytes(m)),
        (Some(t), None) => format!("CPU {}", seconds(t)),
        _ => "measured".into(),
    };
    let mut details = vec![format!("{} runs", row.runs.unwrap_or(0))];
    if row.short == Some(true) {
        details.push(
            "a short run: under 1e9 instructions or half a second of CPU — use a bigger input"
                .into(),
        );
    }
    RowWords {
        headline: parts.join(" · "),
        details,
        short,
        rank: (14, 0.0),
        answer: "baseline",
        shift: None,
        offers_more_runs: false,
    }
}

/// The time words of a measured row.
fn measured(row: &Row, cx: &Context) -> RowWords {
    let c = row.c.as_deref().unwrap_or_default();
    let o = row.other.as_deref().unwrap_or_default();
    let runs = row.runs.unwrap_or(0);
    let short = row.short == Some(true);
    let metric = row.platform_metrics.as_deref().unwrap_or("cpu-time");
    let f: fn(&Run) -> Option<f64> = match metric {
        "macos-v6-pnorm" => pnorm,
        "cpu-time" => cpu,
        _ => cycles,
    };
    let mut details = Vec::new();
    let (answer, shift, share_note) = if metric == "macos-v6-share" {
        share_rule(c, o, row)
    } else {
        match stats::hodges_lehmann(&values(c, f), &values(o, f)) {
            None => (Answer::TooFew, None, None),
            Some(s) => {
                let (lo, hi) = s.percent_interval();
                (branch(lo, hi, TIME_MARGIN, runs, short), Some(s), None)
            }
        }
    };
    let headline = time_headline(answer, shift.as_ref(), share_note.as_deref());
    let offers = runs < stats::MAX_RUNS as u32
        && matches!(
            answer,
            Answer::CloseCall { .. } | Answer::Probably { .. } | Answer::CantTellEstimate
        );
    if let Some(n) = metric_note(metric) {
        details.push(format!("measured in {n}"));
    }
    // CPU seconds: from the headline on a time answer, else each side's own.
    let c_cpu = stats::median(&values(c, cpu));
    let o_cpu = stats::median(&values(o, cpu));
    let answered = !matches!(
        answer,
        Answer::CantTellEstimate | Answer::ShortRun | Answer::TooFew | Answer::SlowCores
    );
    match (c_cpu, o_cpu, shift.as_ref()) {
        (Some(ct), _, Some(s)) if answered => details.push(format!(
            "CPU about {} → about {} (from the estimate)",
            seconds(ct),
            seconds(ct * (1.0 + s.percent() / 100.0))
        )),
        (Some(ct), Some(ot), _) => details.push(format!(
            "CPU about {} and {} here today (they vary with load)",
            seconds(ct),
            seconds(ot)
        )),
        _ => {}
    }
    details.push(instruction_words(c, o, row.std == Some(true), cx));
    let mem_c = stats::median(&values(c, |r| r.memory.map(|v| v as f64)));
    let mem_o = stats::median(&values(o, |r| r.memory.map(|v| v as f64)));
    let big = [mem_c, mem_o]
        .into_iter()
        .flatten()
        .any(|m| m > MEMORY_LINE_BYTES);
    if !short || big {
        if let Some(m) = memory_words(c, o) {
            details.push(m);
        }
    }
    details.push(format!("{runs} runs each"));
    if short {
        details
            .push("a short run: under 1e9 instructions or half a second of CPU on one side".into());
    }
    let parallel = several_cores(c, o);
    if let Some(p) = &parallel {
        details.push(p.clone());
    }
    if row.fat_lto == Some(true) {
        details.push("built with fat LTO".into());
    }
    for p in row.profile.iter().flatten() {
        details.push(format!(
            "the crate's manifest sets {} = {}",
            p.key,
            crate::text::safe_line(&p.value)
        ));
    }
    if answer == Answer::AboutAsFast && matches!(cx.side, Side::Unit(_)) {
        details.push(format!(
            "(perf cannot tell whether {} runs this unit's code)",
            cx.workload
        ));
    }
    let (short_form, rank, answer_name) = time_short(answer, shift.as_ref(), parallel.is_some());
    RowWords {
        headline,
        details,
        short: short_form,
        rank,
        answer: answer_name,
        shift: shift.filter(|_| answered).map(|s| {
            let (lo, hi) = s.percent_interval();
            (s.percent(), lo, hi)
        }),
        offers_more_runs: offers,
    }
}

/// The share rule (§3.8, build note 11): both metrics on the same runs; a
/// difference only when both agree past the line.
fn share_rule(c: &[Run], o: &[Run], row: &Row) -> (Answer, Option<Shift>, Option<String>) {
    let raw = stats::hodges_lehmann(&values(c, cycles), &values(o, cycles));
    let norm = stats::hodges_lehmann(&values(c, pnorm), &values(o, pnorm));
    let all: Vec<&Run> = c.iter().chain(o.iter()).collect();
    let slow = all.iter().filter(|r| !mostly_fast(r)).count();
    let of = all.len();
    let (Some(raw), Some(norm)) = (raw, norm) else {
        return (Answer::TooFew, None, None);
    };
    let (rl, rh) = raw.percent_interval();
    let (nl, nh) = norm.percent_interval();
    let m = TIME_MARGIN;
    let note = format!("{slow} of the {of} runs ran mostly on the slower cores");
    if rl > m && nl > m {
        return (Answer::Slower, Some(raw), Some(format!("cycles; {note}")));
    }
    if rh < -m && nh < -m {
        return (Answer::Faster, Some(raw), Some(format!("cycles; {note}")));
    }
    let load = stats::median(
        &all.iter()
            .map(|r| r.load.map(|l| l as f64 / 100.0))
            .collect::<Vec<_>>(),
    );
    let fast = row.inputs.computer.fast_cores;
    let parallelism = [c, o]
        .iter()
        .filter_map(|side| {
            let ratios: Vec<Option<f64>> = side
                .iter()
                .map(|r| match (r.cpu_us, r.wall_us) {
                    (Some(cu), Some(w)) if w > 0 => Some(cu as f64 / w as f64),
                    _ => None,
                })
                .collect();
            stats::median(&ratios)
        })
        .fold(1.0f64, f64::max)
        .ceil();
    let busy = load.map(|l| {
        format!(
            "the computer may have been busy (load about {} on {fast} fast cores)",
            l.round() as u64
        )
    });
    let design = "the program may run there by design (several threads, a low priority)";
    let cause = match busy {
        Some(b) if load.unwrap_or(0.0) - parallelism >= fast as f64 => format!("{b}, or {design}"),
        Some(b) => format!("{design}, or {b}"),
        None => design.to_string(),
    };
    (Answer::SlowCores, None, Some(format!("{note} — {cause}")))
}

fn time_headline(answer: Answer, shift: Option<&Shift>, share: Option<&str>) -> String {
    let m = TIME_MARGIN;
    let suffix = |s: String| match share {
        Some(n) => format!("{s} — {n}"),
        None => s,
    };
    let Some(s) = shift else {
        return match answer {
            Answer::SlowCores => format!("can't tell — {}", share.unwrap_or("")),
            _ => "can't tell — too few runs gave a value; measure again".into(),
        };
    };
    let x = s.percent();
    let (lo, hi) = s.percent_interval();
    match answer {
        Answer::AboutAsFast => format!("about as fast as the C (within {} %)", margin(m)),
        Answer::Slower if x >= 99.95 => suffix(format!(
            "{}× as slow ({}–{}×)",
            fmt_fixed(1.0 + x / 100.0, 1),
            fmt_fixed(1.0 + lo / 100.0, 1),
            fmt_fixed(1.0 + hi / 100.0, 1)
        )),
        Answer::Slower => suffix(format!(
            "slower by {} % {}",
            pct(x),
            interval(&end(lo, &[m]), &end(hi, &[]), " %")
        )),
        Answer::Faster => suffix(format!(
            "faster: takes {} % less time {}",
            pct(-x),
            interval(&end(-hi, &[m]), &end(-lo, &[]), " %")
        )),
        Answer::CloseCall { slower } => {
            let (a, b, word) = if slower {
                (end(lo, &[0.0, m]), end(hi, &[]), "slower")
            } else {
                (end(-hi, &[0.0, m]), end(-lo, &[]), "faster")
            };
            format!(
                "about {} % {word} {} — too close to the {} % line to call",
                pct(x.abs()),
                interval(&a, &b, " %"),
                margin(m)
            )
        }
        Answer::Probably { slower } => {
            let (a, b, word) = if slower {
                (end(lo, &[0.0]), end(hi, &[]), "slower")
            } else {
                (end(-hi, &[0.0]), end(-lo, &[]), "faster")
            };
            format!(
                "probably {word}, by about {} % {} — not clearly past the {} % line",
                pct(x.abs()),
                interval(&a, &b, " %"),
                margin(m)
            )
        }
        Answer::NoClearDifference => {
            format!(
                "no clear difference: within ±{} %",
                pct_up(lo.abs().max(hi.abs()))
            )
        }
        Answer::CantTellEstimate => {
            format!(
                "can't tell: the estimate is ±{} %",
                pct_up(lo.abs().max(hi.abs()))
            )
        }
        Answer::ShortRun => "can't tell on a run this short — use a bigger input".into(),
        Answer::TooFew => "can't tell — too few runs gave a value; measure again".into(),
        Answer::SlowCores => format!("can't tell — {}", share.unwrap_or("")),
    }
}

/// The short form, the worst-first rank and the MCP answer of a time
/// answer.
fn time_short(
    answer: Answer,
    shift: Option<&Shift>,
    parallel: bool,
) -> (String, (u8, f64), &'static str) {
    let x = shift.map_or(0.0, Shift::percent);
    let (lo, hi) = shift.map_or((0.0, 0.0), Shift::percent_interval);
    let m = TIME_MARGIN;
    let with_parallel = |s: String| {
        let p = format!("{s} · parallel");
        if parallel && p.chars().count() <= SHORT_WIDTH {
            p
        } else {
            s
        }
    };
    match answer {
        Answer::AboutAsFast => (
            with_parallel("about as fast".into()),
            (14, 0.0),
            "about-as-fast",
        ),
        Answer::Slower if x >= 99.95 => (
            with_parallel(format!("{}× as slow", fmt_fixed(1.0 + x / 100.0, 1))),
            (4, x),
            "slower",
        ),
        Answer::Slower => {
            let full = format!(
                "slower {} % {}",
                pct(x),
                interval(&end(lo, &[m]), &end(hi, &[]), " %")
            );
            let s = if parallel {
                with_parallel(format!("slower {} %", pct(x)))
            } else {
                full
            };
            (s, (4, x), "slower")
        }
        Answer::Faster => {
            let full = format!(
                "faster {} % {}",
                pct(-x),
                interval(&end(-hi, &[m]), &end(-lo, &[]), " %")
            );
            let s = if parallel {
                with_parallel(format!("faster {} %", pct(-x)))
            } else {
                full
            };
            (s, (17, -x), "faster")
        }
        Answer::CloseCall { slower: true } => (
            format!("close call: ≈{} % slower", pct(x)),
            (7, x),
            "close-call-slower",
        ),
        Answer::CloseCall { slower: false } => (
            format!("close call: ≈{} % faster", pct(-x)),
            (15, -x),
            "close-call-faster",
        ),
        Answer::Probably { slower: true } => (
            format!("probably slower ≈{} %", pct(x)),
            (6, x),
            "probably-slower",
        ),
        Answer::Probably { slower: false } => (
            format!("probably faster ≈{} %", pct(-x)),
            (16, -x),
            "probably-faster",
        ),
        Answer::NoClearDifference => (
            format!("no clear diff ±{} %", pct_up(lo.abs().max(hi.abs()))),
            (11, 0.0),
            "no-clear-difference",
        ),
        Answer::CantTellEstimate => (
            format!("can't tell: ±{} %", pct_up(lo.abs().max(hi.abs()))),
            (10, 1.0),
            "cant-tell-estimate",
        ),
        Answer::ShortRun => (
            "short run: can't tell".into(),
            (12, 0.0),
            "cant-tell-short-run",
        ),
        Answer::TooFew => ("can't tell: too few".into(), (10, 2.0), "cant-tell-too-few"),
        Answer::SlowCores => (
            "can't tell: slow cores".into(),
            (10, 3.0),
            "cant-tell-slow-cores",
        ),
    }
}

/// The instructions detail (§3.8, build note 18).
fn instruction_words(c: &[Run], o: &[Run], std: bool, cx: &Context) -> String {
    let (ci, oi) = (values(c, instructions), values(o, instructions));
    let Some(s) = stats::hodges_lehmann(&ci, &oi) else {
        return "instructions not counted".into();
    };
    let m = INSTRUCTIONS_MARGIN;
    let (lo, hi) = s.percent_interval();
    let x = s.percent();
    let base = match branch(lo, hi, m, 0, false) {
        Answer::AboutAsFast => format!("about the same instructions (within {} %)", margin(m)),
        Answer::Slower => format!(
            "{} % more instructions {}",
            pct(x),
            interval(&end(lo, &[m]), &end(hi, &[]), " %")
        ),
        Answer::Faster => format!(
            "{} % fewer instructions {}",
            pct(-x),
            interval(&end(-hi, &[m]), &end(-lo, &[]), " %")
        ),
        Answer::CloseCall { slower } => format!(
            "about {} % {} instructions — too close to the {} % line to call",
            pct(x.abs()),
            if slower { "more" } else { "fewer" },
            margin(m)
        ),
        _ => "instructions: can't tell".into(),
    };
    let startup = std
        && !matches!(cx.side, Side::C)
        && stats::hodges_lehmann_linear(&ci, &oi).is_some_and(|d| {
            d.lo <= STARTUP_INSTRUCTIONS && STARTUP_INSTRUCTIONS <= d.hi && d.hi <= STARTUP_UPPER
        });
    if startup {
        format!("{base} (about Rust's fixed start-up)")
    } else {
        base
    }
}

/// The memory detail (§3.8): its margin max(5 %, min(1 MiB, 20 %)) of the
/// C's median; no near-the-line wording.
fn memory_words(c: &[Run], o: &[Run]) -> Option<String> {
    let mem = |r: &Run| r.memory.map(|v| v as f64);
    let c_med = stats::median(&values(c, mem))?;
    let margin = 5f64.max((1024.0 * 1024.0 / c_med * 100.0).min(20.0));
    let Some(s) = stats::hodges_lehmann(&values(c, mem), &values(o, mem)) else {
        return Some("memory: can't tell — too few runs gave a value".into());
    };
    let (lo, hi) = s.percent_interval();
    let x = s.percent();
    let shown_margin = self::margin(margin);
    Some(if lo >= -margin && hi <= margin {
        format!("about the same memory (within {shown_margin} %)")
    } else if lo > margin {
        format!(
            "uses about {} % more memory {}",
            pct(x),
            interval(&end(lo, &[margin]), &end(hi, &[]), " %")
        )
    } else if hi < -margin {
        format!(
            "uses about {} % less memory {}",
            pct(-x),
            interval(&end(-hi, &[margin]), &end(-lo, &[]), " %")
        )
    } else {
        "memory: can't tell — memory varied from run to run".into()
    })
}

/// "uses several cores …" when either side's median CPU time exceeds 1.2 ×
/// its median clock time.
fn several_cores(c: &[Run], o: &[Run]) -> Option<String> {
    let clock = |r: &Run| r.wall_us.map(|v| v as f64);
    let (cc, oc) = (
        stats::median(&values(c, cpu)),
        stats::median(&values(o, cpu)),
    );
    let (cw, ow) = (
        stats::median(&values(c, clock)),
        stats::median(&values(o, clock)),
    );
    let parallel = |cpu: Option<f64>, wall: Option<f64>| match (cpu, wall) {
        (Some(c), Some(w)) => c > 1.2 * w,
        _ => false,
    };
    if !(parallel(cc, cw) || parallel(oc, ow)) {
        return None;
    }
    let (cw, ow) = (cw?, ow?);
    Some(format!(
        "uses several cores: the words compare total CPU work, not waiting — clock time {} → {}",
        seconds(cw),
        seconds(ow)
    ))
}

fn too_short(row: &Row, cx: &Context) -> RowWords {
    let s = row.step1.as_ref();
    let c_first = s.map(|s| &s.c_first);
    let c_second = s.map(|s| &s.c_second);
    // The C's numbers from its step-1 run with fewer instructions.
    let c_run = match (c_first, c_second) {
        (Some(a), Some(b)) => {
            if b.instructions.unwrap_or(u64::MAX) < a.instructions.unwrap_or(u64::MAX) {
                Some(b)
            } else {
                Some(a)
            }
        }
        (a, b) => a.or(b),
    };
    let other = s.and_then(|s| s.other.as_ref());
    let c_cpu = c_run.and_then(|r| r.cpu_us).map(|v| v as f64);
    let o_cpu = other.and_then(|r| r.cpu_us).map(|v| v as f64);
    let ran = match (cx.side, c_cpu, o_cpu) {
        (Side::C, Some(c), _) => format!("the C ran {} of CPU", seconds(c)),
        (_, Some(c), Some(o)) => {
            format!(
                "the C ran {} of CPU and the Rust {}",
                seconds(c),
                seconds(o)
            )
        }
        _ => "the runs were too short".into(),
    };
    let smallest_ins = [
        c_run.and_then(|r| r.instructions),
        other.and_then(|r| r.instructions),
    ]
    .into_iter()
    .flatten()
    .min();
    let smallest_cpu = [c_cpu, o_cpu]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
    let half = (smallest_cpu.is_finite() && smallest_cpu > 0.0)
        .then(|| factor(FLOOR_CPU_US / smallest_cpu));
    let fix = match (smallest_ins.filter(|v| *v > 0), half) {
        (Some(i), Some(h)) => format!(
            "use an input at least about {} bigger ({h} for half a second)",
            factor(FLOOR_INSTRUCTIONS / i as f64)
        ),
        (Some(i), None) => {
            format!(
                "use an input at least about {} bigger",
                factor(FLOOR_INSTRUCTIONS / i as f64)
            )
        }
        (None, Some(h)) => format!("use an input at least about {h} bigger for half a second"),
        (None, None) => "use a bigger input".into(),
    };
    let check = c_run.is_some_and(|r| r.stdout_bytes == 0 || r.end != "exit 0");
    let mut headline = format!("too short to time: {ran} on {} — {fix}", cx.workload);
    if check {
        headline.push_str(" — check the workload's options first");
    }
    // A gap (build note 14): the Rust slower by more than 2× against both
    // step-1 C runs, both sides at least 20 ms of CPU.
    let gap = match (
        c_first.and_then(|r| r.cpu_us),
        c_second.and_then(|r| r.cpu_us),
        other.and_then(|r| r.cpu_us),
    ) {
        (Some(a), Some(b), Some(o)) if a.min(b) >= 20_000 && o >= 20_000 && o > 2 * a.max(b) => {
            Some(o as f64 / a.max(b) as f64)
        }
        _ => None,
    };
    let (short, rank) = match gap {
        Some(g) => (
            format!("too short · Rust {}× CPU", g.round() as u64),
            (5, g),
        ),
        None => ("too short to time".into(), (13, 0.0)),
    };
    RowWords {
        headline,
        details: Vec::new(),
        short,
        rank,
        answer: "too-short",
        shift: None,
        offers_more_runs: false,
    }
}

/// A difference in words: "prints differently (stdout, byte 40 961)",
/// "exits differently (…)", "the Rust crashes (signal 11)", "prints more
/// than 64 MiB …".
pub fn difference_words(d: &Difference) -> String {
    if d.over_cap {
        return format!(
            "prints more than 64 MiB where the C prints {}",
            bytes(d.c_len as f64)
        );
    }
    if d.other_end.starts_with("signal") && !d.c_end.starts_with("signal") {
        return format!("the Rust crashes ({})", d.other_end);
    }
    if d.other_end != d.c_end {
        return format!(
            "exits differently ({} where the C has {})",
            d.other_end, d.c_end
        );
    }
    if d.stream == "exit" {
        return format!(
            "exits differently ({} where the C has {})",
            d.other_end, d.c_end
        );
    }
    format!(
        "prints differently ({}, byte {})",
        d.stream,
        grouped(d.offset + 1)
    )
}

fn behaves_differently(row: &Row, cx: &Context) -> RowWords {
    let Some(d) = &row.first_difference else {
        return fixed(
            "behaves differently".into(),
            "prints differently",
            (1, 0.0),
            "behaves-differently",
        );
    };
    let (short, sub) = if d.over_cap {
        ("prints too much", 2.0)
    } else if d.other_end.starts_with("signal") && !d.c_end.starts_with("signal") {
        ("Rust crashed", 4.0)
    } else if d.other_end != d.c_end || d.stream == "exit" {
        ("exits differently", 3.0)
    } else {
        ("prints differently", 1.0)
    };
    let headline = format!(
        "with {} the program {} on {} — verify does not run this workload",
        cx.side.rust(),
        difference_words(d),
        cx.workload
    );
    RowWords {
        headline,
        details: Vec::new(),
        short: short.into(),
        rank: (1, sub),
        answer: "behaves-differently",
        shift: None,
        offers_more_runs: false,
    }
}

fn run_failed(row: &Row, cx: &Context) -> RowWords {
    let kind = row.outcome.trim_start_matches("run-failed: ");
    let which = row
        .failed_run
        .as_ref()
        .map(|f| {
            let side = if f.side == "c" {
                "the C".to_string()
            } else {
                cx.side.rust()
            };
            format!(": {side}'s timed run {} ended {}", f.index, f.end)
        })
        .unwrap_or_default();
    let answer = match kind {
        "timeout" => "run-failed-timeout",
        "exit" => "run-failed-exit",
        _ => "run-failed-signal",
    };
    RowWords {
        headline: format!("run failed{which} — unlike its first runs"),
        details: Vec::new(),
        short: format!("run failed: {kind}"),
        rank: (3, 0.0),
        answer,
        shift: None,
        offers_more_runs: false,
    }
}

/// Worst first (§3.11): by the rank's place in the design's order, then
/// its key, larger first — a unit's rows, and units by their worst row.
pub fn worst_first(a: &RowWords, b: &RowWords) -> std::cmp::Ordering {
    a.rank.0.cmp(&b.rank.0).then(b.rank.1.total_cmp(&a.rank.1))
}

/// "— measure again with 31 runs: <command> (about N s a row; on a busy
/// computer it may still not tell)", for a row whose words offer it.
pub fn more_runs_words(command: &str, seconds_a_row: u64) -> String {
    format!(
        "— measure again with 31 runs: {command} (about {seconds_a_row} s a row; on a busy \
         computer it may still not tell)"
    )
}

/// The CLI's lines for a row (§3.8): `perf: <label> on <workload> —
/// <headline>`, then the details joined with " · ", every line wrapped at
/// 80 columns with a 6-space hanging indent.
pub fn cli_lines(label: &str, workload: &str, w: &RowWords) -> Vec<String> {
    let mut lines = wrap_words(&format!("perf: {label} on {workload} — {}", w.headline));
    let indent = "      ";
    let mut line = String::new();
    for (i, d) in w.details.iter().enumerate() {
        let piece = if i == 0 { d.clone() } else { format!("· {d}") };
        let len = line.chars().count();
        if len == 0 {
            line = format!("{indent}{piece}");
        } else if len + 1 + piece.chars().count() <= CLI_WIDTH {
            line.push(' ');
            line.push_str(&piece);
        } else {
            lines.extend(wrap_words(&line));
            line = format!("{indent}{piece}");
        }
    }
    if !line.is_empty() {
        lines.extend(wrap_words(&line));
    }
    lines
}

/// Wrap `text` at 80 columns by words, continuing with a 6-space indent;
/// leading spaces are kept.
fn wrap_words(text: &str) -> Vec<String> {
    let body = text.trim_start_matches(' ');
    let lead = &text[..text.len() - body.len()];
    let mut out = Vec::new();
    let mut line = lead.to_string();
    for word in body.split(' ') {
        let len = line.chars().count();
        if len > lead.len() && len + 1 + word.chars().count() > CLI_WIDTH {
            out.push(line);
            line = format!("      {word}");
        } else if len == lead.len() {
            line.push_str(word);
        } else {
            line.push(' ');
            line.push_str(word);
        }
    }
    out.push(line);
    out
}

#[cfg(test)]
mod tests {
    use super::super::results::{Compilers, Computer, RowInputs, Step1, Step1Run};
    use super::*;

    const UNIT: Context = Context {
        side: Side::Unit("u001"),
        workload: "big-text",
        input: Some("bench/big.txt"),
    };

    fn run(cycles: f64) -> Run {
        let c = cycles.round() as u64;
        Run {
            instructions: Some(4_000_000_000),
            cycles: Some(c),
            cpu_us: Some((cycles / 3_200.0) as u64),
            wall_us: Some((cycles / 3_200.0) as u64 + 5_000),
            memory: Some(12_400_000),
            p_instructions: Some(4_000_000_000),
            p_cycles: Some(c),
            load: Some(250),
            end: "exit 0".into(),
            ..Run::default()
        }
    }

    fn row(c: Vec<Run>, other: Vec<Run>, metric: &str, short: bool) -> Row {
        let n = c.len() as u32;
        Row {
            workload: "big-text".into(),
            outcome: "measured".into(),
            short: Some(short),
            runs: Some(n),
            platform_metrics: Some(metric.into()),
            inputs: RowInputs {
                workload: format!("blake3:{}", "a".repeat(64)),
                program: format!("blake3:{}", "b".repeat(64)),
                crates: None,
                replaces: None,
                program_name: "tool".into(),
                units: None,
                left_out: None,
                recipe: super::super::PERF_RECIPE.into(),
                launcher: super::super::PERF_LAUNCHER.into(),
                computer: Computer {
                    os: "15.6".into(),
                    build: "24G84".into(),
                    arch: "arm64".into(),
                    cpu: "Apple M3".into(),
                    two_kinds: true,
                    fast_cores: 8,
                },
                compilers: Compilers {
                    cc: "cc".into(),
                    rustc: Some("rustc 1.94.1".into()),
                },
            },
            c: Some(c),
            other: Some(other),
            std: Some(true),
            fat_lto: None,
            profile: None,
            step1: None,
            failed_run: None,
            setup: None,
            first_difference: None,
            found_before: None,
            last_try: None,
        }
    }

    /// A side of `n` runs around `base` cycles, spread ±`spread` (a
    /// fraction) evenly.
    fn side(n: usize, base: f64, spread: f64) -> Vec<Run> {
        (0..n)
            .map(|i| run(base * (1.0 + spread * ((i as f64 + 0.5) / n as f64 - 0.5) * 2.0)))
            .collect()
    }

    /// A seeded xorshift for reproducible noise.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn normal(&mut self) -> f64 {
            let (u, v) = (self.next().max(1e-12), self.next());
            (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
        }
    }

    fn noisy(rng: &mut Rng, n: usize, base: f64, sigma: f64) -> Vec<Run> {
        (0..n)
            .map(|_| run(base * (1.0 + sigma * rng.normal()).max(0.1)))
            .collect()
    }

    #[test]
    fn each_branch_from_constructed_intervals() {
        let m = TIME_MARGIN;
        assert_eq!(branch(-1.0, 1.5, m, 15, false), Answer::AboutAsFast);
        assert_eq!(branch(-1.0, 1.5, m, 15, true), Answer::ShortRun);
        assert_eq!(branch(4.1, 8.3, m, 15, false), Answer::Slower);
        assert_eq!(branch(-14.0, -10.0, m, 15, false), Answer::Faster);
        assert_eq!(
            branch(0.6, 3.4, m, 15, false),
            Answer::CloseCall { slower: true }
        );
        assert_eq!(
            branch(-3.4, -0.6, m, 15, false),
            Answer::CloseCall { slower: false }
        );
        assert_eq!(
            branch(0.5, 5.5, m, 15, false),
            Answer::Probably { slower: true }
        );
        assert_eq!(
            branch(-5.5, -0.5, m, 15, false),
            Answer::Probably { slower: false }
        );
        assert_eq!(branch(-3.0, 4.0, m, 31, false), Answer::NoClearDifference);
        assert_eq!(branch(-3.0, 4.0, m, 15, false), Answer::CantTellEstimate);
        assert_eq!(branch(-3.0, 4.0, m, 15, true), Answer::ShortRun);
    }

    #[test]
    fn rounding_keeps_off_each_boundary() {
        assert_eq!(pct(6.24), "6.2");
        assert_eq!(pct(12.4), "12");
        assert_eq!(end(0.01, &[0.0]), "0.01");
        assert_eq!(end(0.004, &[0.0]), "0.01");
        assert_eq!(end(2.004, &[TIME_MARGIN]), "2.01");
        assert_eq!(end(1.996, &[0.0, TIME_MARGIN]), "1.99");
        assert_eq!(end(4.14, &[TIME_MARGIN]), "4.1");
        assert_eq!(pct_up(3.41), "3.5");
        assert_eq!(pct_up(11.2), "12");
        assert_eq!(interval("3", "3", " %"), "(3 %)");
        assert_eq!(factor(1.03), "1.1×");
        assert_eq!(factor(7.0), "7×");
        assert_eq!(factor(6.2), "7×");
        assert_eq!(seconds(1_210_000.0), "1.21 s");
        assert_eq!(seconds(12_410_000.0), "12.4 s");
        assert_eq!(seconds(700_000.0), "0.70 s");
        assert_eq!(seconds(40_000.0), "40 ms");
        assert_eq!(bytes(12_400_000.0), "12.4 MB");
        assert_eq!(bytes(124_300_000.0), "124 MB");
        assert_eq!(bytes(2_100_000_000.0), "2.10 GB");
        assert_eq!(count(1.21e10), "1.21e10");
        assert_eq!(grouped(40_961), "40 961");
    }

    #[test]
    fn a_v_a_at_half_a_percent_is_about_as_fast_at_every_n() {
        for n in [5, 7, 10, 15, 20, 31] {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ n as u64);
            let r = row(
                noisy(&mut rng, n, 4e9, 0.005),
                noisy(&mut rng, n, 4e9, 0.005),
                "macos-v6-pnorm",
                false,
            );
            let w = words(&r, &UNIT);
            assert_eq!(w.answer, "about-as-fast", "n = {n}: {}", w.headline);
            assert!(w
                .headline
                .starts_with("about as fast as the C (within 2 %)"));
            assert!(w
                .details
                .iter()
                .any(|d| d.contains("runs this unit's code")));
        }
    }

    #[test]
    fn three_percent_slower_at_high_noise_is_mostly_probably_never_faster() {
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        let mut total = 0;
        for seed in 0..12u64 {
            let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ (seed * 0x9E37_79B9));
            for i in 0..200 {
                let sigma = 0.04 + 0.01 * (i % 2) as f64;
                let r = row(
                    noisy(&mut rng, 31, 4e9, sigma),
                    noisy(&mut rng, 31, 4.12e9, sigma),
                    "macos-v6-pnorm",
                    false,
                );
                *counts.entry(words(&r, &UNIT).answer).or_insert(0) += 1;
                total += 1;
            }
        }
        let most = counts.iter().max_by_key(|(_, c)| **c).map(|(a, _)| *a);
        assert_eq!(most, Some("probably-slower"), "{counts:?}");
        let wrong = counts.get("faster").copied().unwrap_or(0)
            + counts.get("about-as-fast").copied().unwrap_or(0);
        assert!(wrong * 200 < total, "{counts:?}");
    }

    #[test]
    fn the_words_for_each_answer() {
        let slower = row(
            side(15, 4e9, 0.004),
            side(15, 4.25e9, 0.004),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&slower, &UNIT);
        assert_eq!(w.answer, "slower");
        assert!(
            w.headline.starts_with("slower by 6.2 % ("),
            "{}",
            w.headline
        );
        assert!(
            w.details[0].starts_with("CPU about 1.25 s → about 1.33 s"),
            "{:?}",
            w.details
        );
        assert!(!w.offers_more_runs);
        let twice = row(
            side(15, 4e9, 0.004),
            side(15, 1.48e10, 0.004),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&twice, &UNIT);
        assert!(w.headline.starts_with("3.7× as slow ("), "{}", w.headline);
        assert_eq!(w.short, "3.7× as slow");
        let faster = row(
            side(15, 4e9, 0.004),
            side(15, 3.52e9, 0.004),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&faster, &UNIT);
        assert!(
            w.headline.starts_with("faster: takes 12 % less time ("),
            "{}",
            w.headline
        );
        let cant = row(
            side(15, 4e9, 0.2),
            side(15, 4e9, 0.2),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&cant, &UNIT);
        assert_eq!(w.answer, "cant-tell-estimate", "{}", w.headline);
        assert!(w.offers_more_runs);
        assert!(w.shift.is_none(), "no shift exported on a can't-tell row");
        assert!(
            w.details
                .iter()
                .any(|d| d.contains("here today (they vary with load)")),
            "{:?}",
            w.details
        );
        let at31 = row(
            side(31, 4e9, 0.2),
            side(31, 4e9, 0.2),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&at31, &UNIT);
        assert_eq!(w.answer, "no-clear-difference");
        assert!(!w.offers_more_runs);
        let short = row(
            side(15, 4e8, 0.004),
            side(15, 4e8, 0.004),
            "macos-v6-pnorm",
            true,
        );
        let w = words(&short, &UNIT);
        assert_eq!(w.answer, "cant-tell-short-run");
        assert!(
            !w.offers_more_runs,
            "never on a short run's inside-the-margin row"
        );
        let mut few = row(
            side(15, 4e9, 0.01),
            side(15, 4e9, 0.01),
            "macos-v6-pnorm",
            false,
        );
        for r in few.other.as_mut().expect("runs").iter_mut().take(11) {
            r.p_cycles = Some(0);
        }
        assert_eq!(
            words(&few, &UNIT).headline,
            "can't tell — too few runs gave a value; measure again"
        );
    }

    #[test]
    fn the_share_rule_words_a_difference_only_when_both_metrics_agree() {
        // Identical programs, most runs on the slower cores: never slower.
        let mut c = side(15, 4e9, 0.01);
        let mut o = side(15, 4e9, 0.01);
        for r in c.iter_mut().chain(o.iter_mut()).take(20) {
            r.p_cycles = r.cycles.map(|v| v / 4);
            r.p_instructions = r.instructions.map(|v| v / 4);
            r.load = Some(1400);
        }
        let r = row(c.clone(), o.clone(), "macos-v6-share", false);
        let w = words(&r, &UNIT);
        assert_eq!(w.answer, "cant-tell-slow-cores", "{}", w.headline);
        assert!(
            w.headline
                .contains("20 of the 30 runs ran mostly on the slower cores"),
            "{}",
            w.headline
        );
        assert!(
            w.headline
                .contains("the computer may have been busy (load about 14 on 8 fast cores), or"),
            "{}",
            w.headline
        );
        assert_eq!(w.short, "can't tell: slow cores");
        // Scaled ×1.5, ×2 and ×3.7: both agree — slower, from raw cycles.
        for (scale, starts) in [
            (1.5, "slower by 50 %"),
            (2.0, "2.0× as slow"),
            (3.7, "3.7× as slow"),
        ] {
            let o2: Vec<Run> = c
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    r.cycles = r.cycles.map(|v| (v as f64 * scale) as u64);
                    r.p_cycles = r.p_cycles.map(|v| (v as f64 * scale) as u64);
                    r
                })
                .collect();
            let w = words(&row(c.clone(), o2, "macos-v6-share", false), &UNIT);
            assert_eq!(w.answer, "slower", "{}", w.headline);
            assert!(w.headline.starts_with(starts), "{}", w.headline);
            assert!(
                w.headline
                    .contains("— cycles; 30 of the 30 runs ran mostly on the slower cores"),
                "{}",
                w.headline
            );
        }
    }

    #[test]
    fn the_metric_choice() {
        let c = side(15, 4e9, 0.01);
        assert_eq!(
            choose_metric(&c, &c, Platform::MacV6, true),
            "macos-v6-pnorm"
        );
        assert_eq!(
            choose_metric(&c, &c, Platform::MacV6, false),
            "macos-v6-cycles"
        );
        assert_eq!(
            choose_metric(&c, &c, Platform::MacV4, false),
            "macos-v4-cycles"
        );
        assert_eq!(
            choose_metric(&c, &c, Platform::Linux, false),
            "linux-cycles"
        );
        let none: Vec<Run> = c
            .iter()
            .map(|r| Run {
                cycles: None,
                ..r.clone()
            })
            .collect();
        assert_eq!(
            choose_metric(&none, &none, Platform::MacV6, true),
            "cpu-time"
        );
        // Under three quarters mostly fast: the share rule.
        let mut slow = c.clone();
        for r in slow.iter_mut() {
            r.p_cycles = r.cycles.map(|v| v / 4);
        }
        assert_eq!(
            choose_metric(&slow, &slow, Platform::MacV6, true),
            "macos-v6-share"
        );
        // A phased program: P cost unsteady (some runs 60 % on the fast cores
        // at varying cost) while raw cycles are steady.
        let phased: Vec<Run> = c
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut r = r.clone();
                let share = 0.6 + 0.03 * (i % 5) as f64;
                r.p_cycles = r.cycles.map(|v| (v as f64 * share) as u64);
                r.p_instructions = r
                    .instructions
                    .map(|v| (v as f64 * (0.95 - 0.2 * (i % 3) as f64)) as u64);
                r
            })
            .collect();
        assert_eq!(
            choose_metric(&phased, &phased, Platform::MacV6, true),
            "macos-v6-cycles-phases"
        );
    }

    #[test]
    fn instructions_memory_and_parallel_details() {
        // The start-up note: a std unit, 1e7 more instructions on a 1e9 C.
        let mut c = side(15, 4e9, 0.004);
        let mut o = side(15, 4e9, 0.004);
        for (i, r) in c.iter_mut().enumerate() {
            r.instructions = Some(1_000_000_000 + (i as u64 % 3) * 1_000_000);
        }
        for (i, r) in o.iter_mut().enumerate() {
            r.instructions = Some(1_010_400_000 + (i as u64 % 3) * 1_000_000);
        }
        let w = words(&row(c.clone(), o.clone(), "macos-v6-pnorm", false), &UNIT);
        assert!(
            w.details
                .iter()
                .any(|d| d.ends_with("(about Rust's fixed start-up)")),
            "{:?}",
            w.details
        );
        let mut no_std = row(c, o, "macos-v6-pnorm", false);
        no_std.std = Some(false);
        assert!(!words(&no_std, &UNIT)
            .details
            .iter()
            .any(|d| d.contains("start-up")));
        // Memory: a 1 MB program's margin is 20 %.
        let mut c = side(15, 4e9, 0.004);
        let mut o = side(15, 4e9, 0.004);
        for r in c.iter_mut() {
            r.memory = Some(1_000_000);
        }
        for r in o.iter_mut() {
            r.memory = Some(1_150_000);
        }
        let w = words(&row(c, o, "macos-v6-pnorm", false), &UNIT);
        assert!(
            w.details
                .iter()
                .any(|d| d == "about the same memory (within 20 %)"),
            "{:?}",
            w.details
        );
        // Several cores.
        let c = side(15, 4e9, 0.004);
        let o: Vec<Run> = side(15, 4e9, 0.004)
            .into_iter()
            .map(|mut r| {
                r.wall_us = r.cpu_us.map(|v| v / 4);
                r
            })
            .collect();
        let w = words(&row(c, o, "macos-v6-pnorm", false), &UNIT);
        assert!(
            w.details
                .iter()
                .any(|d| d.starts_with("uses several cores")),
            "{:?}",
            w.details
        );
        assert_eq!(w.short, "about as fast · parallel");
    }

    fn step1(c_cpu: u64, c_ins: u64, o_cpu: u64, o_ins: u64) -> Step1 {
        let r = |cpu: u64, ins: u64| Step1Run {
            instructions: Some(ins),
            cpu_us: Some(cpu),
            end: "exit 0".into(),
            stdout_bytes: 10,
            stderr_bytes: 0,
        };
        Step1 {
            c_first: r(c_cpu, c_ins),
            other: Some(r(o_cpu, o_ins)),
            c_second: r(c_cpu, c_ins),
        }
    }

    #[test]
    fn too_short_words_and_factors() {
        let mut r = row(Vec::new(), Vec::new(), "macos-v6-pnorm", false);
        r.outcome = "too-short".into();
        r.c = None;
        r.other = None;
        r.short = None;
        r.runs = None;
        r.platform_metrics = None;
        r.step1 = Some(step1(40_000, 140_000_000, 360_000, 1_260_000_000));
        let cx = Context {
            workload: "small-text",
            ..UNIT
        };
        let w = words(&r, &cx);
        assert_eq!(
            w.headline,
            "too short to time: the C ran 40 ms of CPU and the Rust 0.36 s on small-text — use an \
             input at least about 8× bigger (13× for half a second)"
        );
        assert_eq!(w.short, "too short · Rust 9× CPU");
        assert_eq!(w.rank.0, 5, "a gap ranks after slower");
        // Close to the floor: 1.1×, never 1×.
        r.step1 = Some(step1(480_000, 970_000_000, 490_000, 980_000_000));
        let w = words(&r, &cx);
        assert!(w.headline.contains("1.1×"), "{}", w.headline);
        assert_eq!(w.short, "too short to time");
    }

    #[test]
    fn short_forms_fit_26_columns_with_the_longest_numbers() {
        let rows = [
            row(
                side(15, 4e9, 0.004),
                side(15, 4.38e9, 0.05),
                "macos-v6-pnorm",
                false,
            ),
            row(
                side(15, 4e9, 0.004),
                side(15, 7.9e9, 0.004),
                "macos-v6-pnorm",
                false,
            ),
            row(
                side(15, 4e9, 0.004),
                side(15, 2.1e9, 0.004),
                "macos-v6-pnorm",
                false,
            ),
            row(
                side(15, 4e9, 0.3),
                side(15, 4e9, 0.3),
                "macos-v6-pnorm",
                false,
            ),
            row(
                side(31, 4e9, 0.3),
                side(31, 4e9, 0.3),
                "macos-v6-pnorm",
                false,
            ),
        ];
        for r in &rows {
            let w = words(r, &UNIT);
            assert!(
                w.short.chars().count() <= SHORT_WIDTH,
                "{:?} ({})",
                w.short,
                w.short.chars().count()
            );
        }
        for s in [
            "can't tell: slow cores",
            "short run: can't tell",
            "too short · Rust 99× CPU",
            "close call: ≈9.9 % slower",
            "probably slower ≈9.9 %",
            "no clear diff ±99 %",
            "C output too large",
        ] {
            assert!(s.chars().count() <= SHORT_WIDTH, "{s}");
        }
    }

    #[test]
    fn behaves_differently_is_named_by_its_end_first() {
        let mut r = row(Vec::new(), Vec::new(), "macos-v6-pnorm", false);
        r.outcome = "behaves-differently".into();
        r.c = None;
        r.other = None;
        r.short = None;
        r.runs = None;
        r.platform_metrics = None;
        let d = |stream: &str, other_end: &str, over: bool| Difference {
            stream: stream.into(),
            c_len: 2_100_000,
            other_len: 2_100_000,
            offset: 40_960,
            c_end: "exit 0".into(),
            other_end: other_end.into(),
            over_cap: over,
            kept: Vec::new(),
        };
        r.first_difference = Some(d("stdout", "signal 11", false));
        assert_eq!(
            words(&r, &UNIT).short,
            "Rust crashed",
            "a crash part-way reads as a crash"
        );
        r.first_difference = Some(d("stdout", "exit 1", false));
        assert_eq!(words(&r, &UNIT).short, "exits differently");
        r.first_difference = Some(d("stdout", "exit 0", true));
        assert_eq!(words(&r, &UNIT).short, "prints too much");
        r.first_difference = Some(d("stdout", "exit 0", false));
        let w = words(&r, &UNIT);
        assert_eq!(w.short, "prints differently");
        assert_eq!(
            w.headline,
            "with u001's Rust the program prints differently (stdout, byte 40 961) on big-text — \
             verify does not run this workload"
        );
    }

    #[test]
    fn set_up_words_rebuild_from_their_facts() {
        let s = |f: SetupFacts| Some(f);
        let words = |o: &str, f: Option<SetupFacts>| set_up_words(o, f.as_ref(), &UNIT);
        assert_eq!(
            words(
                "not-verified",
                s(SetupFacts {
                    reason: Some("accept-interrupted".into()),
                    attempt: Some("a-1234".into()),
                    ..SetupFacts::default()
                })
            ),
            "an Accept of a-1234 was interrupted — Re-check u001 (or run harness verify u001) to \
             finish or undo it; Measure does not"
        );
        assert_eq!(
            words(
                "does-not-link",
                s(SetupFacts {
                    cause: Some("no-std".into()),
                    units: Some(vec!["u003".into()]),
                    ..SetupFacts::default()
                })
            ),
            "u003 uses no std — it cannot be linked beside units that use std"
        );
        assert!(words(
            "does-not-link",
            s(SetupFacts {
                cause: Some("lto".into()),
                units: Some(vec!["u002".into()]),
                ..SetupFacts::default()
            })
        )
        .ends_with("set lto = false in its Cargo.toml and run harness verify u002"));
        let mixed = words(
            "mixed-panic",
            s(SetupFacts {
                runtimes: Some(vec![
                    super::super::results::UnitRuntime {
                        id: "u001".into(),
                        runtime: "unwind".into(),
                    },
                    super::super::results::UnitRuntime {
                        id: "u002".into(),
                        runtime: "abort".into(),
                    },
                    super::super::results::UnitRuntime {
                        id: "u003".into(),
                        runtime: "none".into(),
                    },
                ]),
                ..SetupFacts::default()
            }),
        );
        assert!(mixed.starts_with("u001 unwinds; u002 aborts — "), "{mixed}");
        assert!(
            mixed.ends_with("to u001's Cargo.toml, then run harness verify u001"),
            "{mixed}"
        );
        assert_eq!(
            words(
                "input-unusable",
                s(SetupFacts {
                    input: Some("too-large".into()),
                    ..SetupFacts::default()
                })
            ),
            "bench/big.txt is over 64 MiB — use a smaller input"
        );
    }

    #[test]
    fn every_outcome_answers_from_the_closed_set() {
        let base = row(
            side(15, 1e9, 0.01),
            side(15, 1.05e9, 0.01),
            "macos-v6-cycles",
            false,
        );
        for o in super::super::results::OUTCOMES {
            let mut r = base.clone();
            r.outcome = (*o).into();
            if *o == "baseline" {
                r.other = None;
            }
            for cx in [
                UNIT,
                Context {
                    side: Side::C,
                    ..UNIT
                },
            ] {
                let w = words(&r, &cx);
                assert!(ANSWERS.contains(&w.answer), "{o}: {}", w.answer);
            }
        }
    }

    #[test]
    fn the_worst_order_is_the_designs() {
        let r = |rank: (u8, f64)| RowWords {
            headline: String::new(),
            details: Vec::new(),
            short: String::new(),
            rank,
            answer: "measured",
            shift: None,
            offers_more_runs: false,
        };
        let base = row(Vec::new(), Vec::new(), "macos-v6-pnorm", false);
        let of = |outcome: &str| {
            let mut x = base.clone();
            x.outcome = outcome.into();
            words(&x, &UNIT).rank
        };
        let t = |a: Answer| time_short(a, None, false).1;
        let order = [
            (1, 4.0), // Rust crashed
            (1, 3.0), // exits differently
            (1, 2.0), // prints too much
            (1, 1.0), // prints differently
            of("stopped-by-sigkill"),
            of("run-failed: exit"),
            t(Answer::Slower),
            (5, 3.0), // too short with a CPU gap
            t(Answer::Probably { slower: true }),
            t(Answer::CloseCall { slower: true }),
            of("could-not-start"),
            of("crate-does-not-build"),
            t(Answer::SlowCores),
            t(Answer::TooFew),
            t(Answer::CantTellEstimate),
            t(Answer::NoClearDifference),
            t(Answer::ShortRun),
            (13, 0.0), // too short
            t(Answer::AboutAsFast),
            t(Answer::CloseCall { slower: false }),
            t(Answer::Probably { slower: false }),
            t(Answer::Faster),
        ];
        for pair in order.windows(2) {
            assert_eq!(
                worst_first(&r(pair[0]), &r(pair[1])),
                std::cmp::Ordering::Less,
                "{:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn the_cli_lines_wrap_at_80_with_a_hanging_indent() {
        let r = row(
            side(15, 4e9, 0.004),
            side(15, 4.25e9, 0.004),
            "macos-v6-pnorm",
            false,
        );
        let w = words(&r, &UNIT);
        let lines = cli_lines("u001-katajainen", "big-text", &w);
        assert!(
            lines[0].starts_with("perf: u001-katajainen on big-text — slower by 6.2 % ("),
            "{lines:?}"
        );
        for l in &lines {
            assert!(l.chars().count() <= CLI_WIDTH, "{l:?}");
        }
        for l in &lines[1..] {
            assert!(l.starts_with("      "), "{l:?}");
        }
    }

    /// "The C crashes" quotes the run that crashed: when only the second
    /// step-1 run did, its signal — never the first run's normal end.
    #[test]
    fn the_c_crash_quotes_the_run_that_crashed() {
        let step = |end: &str| Step1Run {
            instructions: None,
            cpu_us: None,
            end: end.into(),
            stdout_bytes: 0,
            stderr_bytes: 0,
        };
        let mut r = row(Vec::new(), Vec::new(), "macos-v6-pnorm", false);
        r.outcome = "c-crashed".into();
        r.c = None;
        r.other = None;
        r.short = None;
        r.runs = None;
        r.platform_metrics = None;
        r.step1 = Some(Step1 {
            c_first: step("exit 0"),
            other: None,
            c_second: step("signal 11"),
        });
        let cx = Context {
            side: Side::C,
            workload: "big-text",
            input: None,
        };
        assert_eq!(
            words(&r, &cx).headline,
            "the C crashes on big-text (signal 11)"
        );
        r.step1 = Some(Step1 {
            c_first: step("signal 6"),
            other: None,
            c_second: step("signal 11"),
        });
        assert_eq!(
            words(&r, &cx).headline,
            "the C crashes on big-text (signal 6)"
        );
    }
}
