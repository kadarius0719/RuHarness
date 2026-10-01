//! What the read model knows about perf (docs/PERF-DESIGN.md §3.11 *Inputs
//! read by the cockpit*): the workloads file's state, the results files,
//! each workload's digest today — its input hashed with perf's own confined,
//! bounded read, cached across loads by (device, inode, size, modification
//! time), and never while a perf run holds the lock — and the crates'
//! digests. Shared by the cockpit and the MCP reads through
//! [`crate::model::Snapshot`]. Std and harness-core only.

use harness_core::ledger::Ledger;
use harness_core::perf::results::{self as res, ProgramResults, UnitResults};
use harness_core::perf::workloads::{self as wl, InputUnusable, WorkloadsState};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;

/// The most input bytes one load hashes (the rest read "can't check").
pub const MAX_INPUT_HASH_BYTES: u64 = 256 * 1024 * 1024;

/// A workload's state today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputNow {
    /// Its digest today (with or without an input).
    Digest(String),
    /// Its input cannot be used: the CLI's own words come from this.
    Unusable(InputUnusable),
    /// A perf run holds the lock and the input changed since it was last
    /// hashed here: "can't check while measuring".
    WhileMeasuring,
    /// Over this load's hashing budget: "can't check: inputs too large to
    /// hash here".
    TooLarge,
}

/// perf as the read model sees it.
#[derive(Debug, Clone)]
pub struct PerfRead {
    /// The workloads file: its state, or why it could not be read.
    pub workloads: Result<WorkloadsState, String>,
    /// `program.json`, or why it could not be read.
    pub program: Result<Option<ProgramResults>, String>,
    /// Each plan unit's results file, when it has one (or why it could not
    /// be read).
    pub units: BTreeMap<String, Result<UnitResults, String>>,
    /// Results files of units no longer in the plan.
    pub orphans: Vec<String>,
    /// Each workload's state today, by id.
    pub inputs: BTreeMap<String, InputNow>,
    /// Each plan unit's crate digest today (`unit_crate_file_set_hash`).
    pub crates: BTreeMap<String, String>,
    /// A perf run holds the writer lock now.
    pub measuring: bool,
    /// The C program's digest today (with facts and a workloads file).
    pub program_now: Option<String>,
}

impl Default for PerfRead {
    fn default() -> Self {
        PerfRead {
            workloads: Ok(WorkloadsState::NoFile),
            program: Ok(None),
            units: BTreeMap::new(),
            orphans: Vec::new(),
            inputs: BTreeMap::new(),
            crates: BTreeMap::new(),
            measuring: false,
            program_now: None,
        }
    }
}

/// (device, inode, size, modification time in ns, the workload's id, args
/// and input name) → the workload's digest.
type CacheKey = (u64, u64, u64, i128, String);

static CACHE: Mutex<Option<HashMap<CacheKey, String>>> = Mutex::new(None);

fn identity(w: &wl::Workload) -> String {
    let mut s = w.id.clone();
    for a in &w.args {
        s.push('\0');
        s.push_str(a);
    }
    s.push('\u{1}');
    s.push_str(w.input.as_deref().unwrap_or(""));
    s
}

fn file_key(root: &Path, rel: &str, w: &wl::Workload) -> Option<CacheKey> {
    let m = std::fs::metadata(root.join(rel)).ok()?;
    let mtime = m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128;
    Some((m.dev(), m.ino(), m.size(), mtime, identity(w)))
}

/// Read perf's files under `root` for the `units` of the plan (ids) — the
/// caller read the plan; `holder_command` is the live lock holder's.
pub fn read(
    root: &Path,
    units: &[(String, Option<String>)],
    holder_command: Option<&str>,
) -> PerfRead {
    let measuring =
        holder_command.is_some_and(|c| c.starts_with(harness_core::perf::PERF_RUN_LOCK));
    let mut read = PerfRead {
        measuring,
        ..PerfRead::default()
    };
    read.workloads = wl::load(root).map_err(|e| e.to_string());
    let dir = harness_core::perf::perf_dir(root);
    let dir_ok = std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir());
    if dir_ok {
        read.program = res::read_program(&res::program_path(&dir)).map_err(|e| e.to_string());
        let units_dir = dir.join(res::UNITS_DIR);
        if let Ok(entries) = std::fs::read_dir(&units_dir) {
            for e in entries.flatten() {
                let Some(id) = e
                    .file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".json"))
                    .map(str::to_string)
                else {
                    continue;
                };
                if !harness_core::plan::is_clean_segment(&id) {
                    continue;
                }
                if units.iter().any(|(u, _)| *u == id) {
                    let file = res::read_unit(&res::unit_path(&dir, &id), &id)
                        .map_err(|e| e.to_string())
                        .and_then(|f| f.ok_or_else(|| "gone".to_string()));
                    read.units.insert(id, file);
                } else {
                    read.orphans.push(id);
                }
            }
        }
        read.orphans.sort();
    }
    // The crates' digests, for the units perf measured.
    let ledger = Ledger::new(root);
    for (id, krate) in units {
        if !read.units.contains_key(id) && !program_holds(&read.program, id) {
            continue;
        }
        if let Some(krate) = krate {
            if let Ok(d) =
                harness_core::hash::unit_crate_file_set_hash(root, &ledger.unit_dir(id).join(krate))
            {
                read.crates.insert(id.clone(), d);
            }
        }
    }
    // Each workload's digest today.
    if let Ok(WorkloadsState::Ready(workloads)) = &read.workloads {
        let mut budget = MAX_INPUT_HASH_BYTES;
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        for w in &workloads.workloads {
            let now = match &w.input {
                None => InputNow::Digest(wl::digest(w, None)),
                Some(rel) => match file_key(root, rel, w) {
                    Some(key) if cache.contains_key(&key) => InputNow::Digest(cache[&key].clone()),
                    _ if measuring => InputNow::WhileMeasuring,
                    Some(key) if key.2 > budget => InputNow::TooLarge,
                    key => match wl::read_input(root, rel) {
                        Ok(bytes) => {
                            budget = budget.saturating_sub(bytes.len() as u64);
                            let d = wl::digest(w, Some(&bytes));
                            if let Some(key) = key {
                                cache.insert(key, d.clone());
                            }
                            InputNow::Digest(d)
                        }
                        Err(reason) => InputNow::Unusable(reason),
                    },
                },
            };
            read.inputs.insert(w.id.clone(), now);
        }
    }
    read
}

fn program_holds(program: &Result<Option<ProgramResults>, String>, id: &str) -> bool {
    match program {
        Ok(Some(p)) => p
            .as_it_stands
            .iter()
            .any(|r| r.inputs.units.iter().flatten().any(|u| u.id == id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_hashed_once_and_never_while_measuring() {
        let tmp = std::env::temp_dir().join(format!("harness-tui-perfread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("tmp");
        let root = tmp.canonicalize().expect("root");
        std::fs::create_dir_all(root.join("migration/perf")).expect("dir");
        std::fs::create_dir_all(root.join("bench")).expect("dir");
        std::fs::write(root.join("bench/in.txt"), b"hello").expect("write");
        std::fs::write(
            root.join("migration/perf/workloads.toml"),
            "schema_version = 1\n[[workload]]\nid = \"w\"\nargs = [\"{input}\"]\ninput = \"bench/in.txt\"\n\
             [[workload]]\nid = \"gone\"\nargs = [\"{input}\"]\ninput = \"bench/gone.txt\"\n",
        )
        .expect("write");
        let r = read(&root, &[], None);
        let d = match r.inputs.get("w") {
            Some(InputNow::Digest(d)) => d.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            r.inputs.get("gone"),
            Some(&InputNow::Unusable(InputUnusable::Missing))
        );
        // While a perf run holds the lock: the cached digest, never a read.
        let r = read(&root, &[], Some("perf run --target ."));
        assert!(r.measuring);
        assert_eq!(r.inputs.get("w"), Some(&InputNow::Digest(d)));
        // A changed input then cannot be checked.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("bench/in.txt"), b"hello, again").expect("write");
        let r = read(&root, &[], Some("perf run"));
        assert_eq!(r.inputs.get("w"), Some(&InputNow::WhileMeasuring));
        let r = read(&root, &[], Some("verify u001"));
        assert!(
            matches!(r.inputs.get("w"), Some(InputNow::Digest(_))),
            "another writer is no perf run"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
