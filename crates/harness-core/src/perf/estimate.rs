//! About how long a perf run takes (docs/PERF-DESIGN.md §6 *Cost*, build
//! note 27): the figure the cockpit's dialogs and the CLI's "measure again
//! with 31 runs" words show. Runs per workload: the C alone 2 + n, each
//! row 3 + 2n; seconds = runs × (the C's clock time + 0.1 s) + 10 s per new
//! binary + the builds. A row that turns out too short stops after its 3
//! step-1 runs, so the figure is an upper guess, never a promise.

use super::results::Row;
use super::stats::median;
use std::path::Path;
use std::time::SystemTime;

/// A new binary's first exec on a loaded Mac (0.25–14 s), counted as this.
pub const FIRST_EXEC_SECONDS: f64 = 10.0;
/// One compile of the C (about 1–5 s).
pub const C_COMPILE_SECONDS: f64 = 3.0;
/// A unit's crate build when warm (about 5–30 s).
pub const CRATE_BUILD_SECONDS: f64 = 15.0;
/// One link.
pub const LINK_SECONDS: f64 = 1.0;
/// What each run costs beyond the program's own clock time.
pub const RUN_OVERHEAD_SECONDS: f64 = 0.1;

/// The C's clock time on a row's workload, in seconds: the median of its
/// timed runs' clock time, else twice its step-1 CPU time; `None` when the
/// row holds neither.
pub fn c_clock(row: &Row) -> Option<f64> {
    if let Some(runs) = row.c.as_deref() {
        let wall: Vec<Option<f64>> = runs.iter().map(|r| r.wall_us.map(|w| w as f64)).collect();
        if let Some(m) = median(&wall) {
            return Some(m / 1e6);
        }
        let cpu: Vec<Option<f64>> = runs.iter().map(|r| r.cpu_us.map(|c| c as f64)).collect();
        if let Some(m) = median(&cpu) {
            return Some(2.0 * m / 1e6);
        }
    }
    let s = row.step1.as_ref()?;
    let cpu = [s.c_first.cpu_us, s.c_second.cpu_us]
        .into_iter()
        .flatten()
        .min()?;
    Some(2.0 * cpu as f64 / 1e6)
}

/// Runs of the C alone on one workload at `n` runs a side.
pub fn c_alone_runs(n: u32) -> u32 {
    2 + n
}

/// Runs of one row on one workload at `n` runs a side.
pub fn row_runs(n: u32) -> u32 {
    3 + 2 * n
}

/// What one perf command measures.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// Each workload measured: its runs a side and the C's clock time in
    /// seconds from its last stored row (`None`: never measured).
    pub workloads: Vec<(u32, Option<f64>)>,
    /// The C alone is measured.
    pub c_alone: bool,
    /// Rows per workload (each unit, and the program as it stands).
    pub rows: u32,
    /// Crates built (each warm).
    pub crates: u32,
    /// Links (each unit's, the program as it stands's).
    pub links: u32,
}

/// The estimate of a [`Job`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Estimate {
    /// About this many seconds, builds included.
    Seconds(u64),
    /// The C's time is not known on some workload: the runs only.
    Runs {
        /// The C alone's runs per workload, when every workload has the
        /// same n (else the formula's words).
        c_alone: Option<u32>,
        /// A row's runs per workload, likewise.
        row: Option<u32>,
    },
}

impl Job {
    /// The runs of the program in all.
    pub fn runs(&self) -> u32 {
        self.workloads
            .iter()
            .map(|(n, _)| {
                (if self.c_alone { c_alone_runs(*n) } else { 0 }) + self.rows * row_runs(*n)
            })
            .sum()
    }

    /// About how long it takes.
    pub fn estimate(&self) -> Estimate {
        let mut seconds = 0.0;
        for (n, clock) in &self.workloads {
            let Some(clock) = clock else {
                let same = self.workloads.iter().all(|(m, _)| m == n);
                return Estimate::Runs {
                    c_alone: same.then(|| c_alone_runs(*n)),
                    row: same.then(|| row_runs(*n)),
                };
            };
            let runs = (if self.c_alone { c_alone_runs(*n) } else { 0 }) + self.rows * row_runs(*n);
            seconds += runs as f64 * (clock + RUN_OVERHEAD_SECONDS);
        }
        // A new binary a side: the C's and each row's.
        seconds += FIRST_EXEC_SECONDS * (1 + self.rows) as f64;
        seconds += C_COMPILE_SECONDS
            + CRATE_BUILD_SECONDS * self.crates as f64
            + LINK_SECONDS * self.links as f64;
        Estimate::Seconds(seconds.ceil() as u64)
    }
}

impl Estimate {
    /// In words: "about 56 s", "about 4 minutes", or the runs.
    pub fn words(&self) -> String {
        match self {
            Estimate::Seconds(s) if *s < 120 => format!("about {s} s"),
            Estimate::Seconds(s) => format!("about {} minutes", (*s as f64 / 60.0).round() as u64),
            Estimate::Runs {
                c_alone: Some(c),
                row: Some(r),
            } => format!(
                "the C's time is not known yet: about {c} runs of your program, then {r} for each \
                 row, per workload"
            ),
            Estimate::Runs { .. } => "the C's time is not known yet: about 2 + n runs of your \
                                      program, then 3 + 2n for each row, per workload"
                .into(),
        }
    }
}

/// The most source entries [`crate_cold`] looks at before it calls a
/// build cold.
const MAX_SOURCE_ENTRIES: usize = 4096;

/// Whether a unit crate's build is cold (build note 27): its
/// `target/release` is missing, or the newest file directly in it is older
/// than the newest of `Cargo.toml`, `Cargo.lock` and the files under `src/`
/// (files compared, not folders; links not followed). Too many sources to
/// look at reads as cold.
pub fn crate_cold(crate_dir: &Path) -> bool {
    let modified = |p: &Path| -> Option<SystemTime> {
        let m = std::fs::symlink_metadata(p).ok()?;
        m.is_file().then(|| m.modified().ok()).flatten()
    };
    let release = crate_dir.join("target").join("release");
    let Ok(entries) = std::fs::read_dir(&release) else {
        return true;
    };
    let built = entries.flatten().filter_map(|e| modified(&e.path())).max();
    let Some(built) = built else {
        return true;
    };
    let mut newest = [crate_dir.join("Cargo.toml"), crate_dir.join("Cargo.lock")]
        .iter()
        .filter_map(|p| modified(p))
        .max();
    let mut stack = vec![crate_dir.join("src")];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_SOURCE_ENTRIES {
                return true;
            }
            let Ok(m) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if m.is_dir() {
                stack.push(e.path());
            } else if m.is_file() {
                if let Ok(t) = m.modified() {
                    newest = newest.max(Some(t));
                }
            }
        }
    }
    newest.is_some_and(|n| built < n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_designs_example() {
        // A 1.2 s C (clock 1.3 s) at n = 15: the C alone ≈ 34 s with its
        // first exec, plus the C compile.
        let c = Job {
            workloads: vec![(15, Some(1.3))],
            c_alone: true,
            rows: 0,
            crates: 0,
            links: 0,
        };
        assert_eq!(c.runs(), 17);
        assert_eq!(c.estimate(), Estimate::Seconds(37));
        let unit = Job {
            workloads: vec![(31, Some(1.3))],
            c_alone: false,
            rows: 1,
            crates: 1,
            links: 1,
        };
        // 65 runs ≈ 91 s + 20 s of first execs + the C compile, the crate
        // build and a link.
        assert_eq!(unit.runs(), 65);
        assert_eq!(unit.estimate(), Estimate::Seconds(91 + 20 + 3 + 15 + 1));
        assert_eq!(unit.estimate().words(), "about 2 minutes");
        assert_eq!(Estimate::Seconds(56).words(), "about 56 s");
    }

    #[test]
    fn a_crate_is_cold_without_a_build_or_with_newer_sources() {
        let dir = std::env::temp_dir().join(format!("perf-cold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        assert!(crate_cold(&dir), "no target/release");
        std::fs::create_dir_all(dir.join("target/release")).unwrap();
        assert!(crate_cold(&dir), "an empty target/release");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("target/release/libx.a"), "!<arch>\n").unwrap();
        assert!(!crate_cold(&dir));
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("src/inner/deep.rs"), "").unwrap();
        assert!(crate_cold(&dir), "a newer source, however deep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_c_gives_the_runs() {
        let job = Job {
            workloads: vec![(15, Some(1.0)), (15, None)],
            c_alone: true,
            rows: 2,
            crates: 2,
            links: 3,
        };
        assert_eq!(job.runs(), 2 * (17 + 2 * 33));
        assert_eq!(
            job.estimate().words(),
            "the C's time is not known yet: about 17 runs of your program, then 33 for each row, \
             per workload"
        );
        let mixed = Job {
            workloads: vec![(15, None), (5, None)],
            ..job
        };
        assert!(mixed.estimate().words().contains("2 + n runs"));
    }
}
