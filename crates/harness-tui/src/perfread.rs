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

/// The most names of results files of units no longer in the plan kept
/// (the rest are counted).
pub const MAX_ORPHANS_LISTED: usize = 100;

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
    /// Results files of units no longer in the plan: the first
    /// [`MAX_ORPHANS_LISTED`] names, sorted.
    pub orphans: Vec<String>,
    /// How many more such files there are.
    pub orphans_more: usize,
    /// perf's folders that could not be read, in words (a linked
    /// `migration/perf/units` is refused, never followed).
    pub errors: Vec<String>,
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
            orphans_more: 0,
            errors: Vec::new(),
            inputs: BTreeMap::new(),
            crates: BTreeMap::new(),
            measuring: false,
            program_now: None,
        }
    }
}

/// (device, inode, size, modification time in ns, change time in ns, the
/// workload's id, args and input name) → the workload's digest.
type CacheKey = (u64, u64, u64, i128, i128, String);

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

/// The cache key of the input `rel` under `root`, or `None` when its
/// digest is never taken from the cache: a link anywhere on its path (the
/// file itself or a folder on the way — perf's own checks then decide,
/// each time, whether it may be used), or a path that does not resolve to
/// a regular file. The key is the file's own (device, inode, size,
/// modification time) — what §3.11 names — and its change time, which a
/// chmod, or an in-place rewrite that puts the old modification time back,
/// moves; and the workload's identity.
fn file_key(root: &Path, rel: &str, w: &wl::Workload) -> Option<CacheKey> {
    if wl::input_problem(rel).is_some() {
        return None;
    }
    let canonical = root.join(rel).canonicalize().ok()?;
    if canonical != root.canonicalize().ok()?.join(rel) {
        return None;
    }
    let m = std::fs::symlink_metadata(&canonical).ok()?;
    if !m.file_type().is_file() {
        return None;
    }
    let ns = |s: i64, n: i64| s as i128 * 1_000_000_000 + n as i128;
    Some((
        m.dev(),
        m.ino(),
        m.size(),
        ns(m.mtime(), m.mtime_nsec()),
        ns(m.ctime(), m.ctime_nsec()),
        identity(w),
    ))
}

/// Read perf's files under `root` for the `units` of the plan (ids) — the
/// caller read the plan; `holder_command` is the live lock holder's.
pub fn read(
    root: &Path,
    units: &[(String, Option<String>)],
    holder_command: Option<&str>,
) -> PerfRead {
    read_with_budget(root, units, holder_command, MAX_INPUT_HASH_BYTES)
}

/// [`read`], hashing at most `budget` bytes of inputs.
fn read_with_budget(
    root: &Path,
    units: &[(String, Option<String>)],
    holder_command: Option<&str>,
    budget: u64,
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
        // The units' folder is never followed through a link: a file read
        // through it could be outside the target (§3.9).
        let entries = match std::fs::symlink_metadata(&units_dir) {
            Ok(m) if m.is_dir() => std::fs::read_dir(&units_dir).ok(),
            Ok(_) => {
                read.errors.push(format!(
                    "{}: must be a directory (a link is refused)",
                    units_dir.display()
                ));
                None
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                read.errors.push(format!("{}: {e}", units_dir.display()));
                None
            }
        };
        let mut orphans = std::collections::BTreeSet::new();
        let mut orphans_seen = 0usize;
        for e in entries.into_iter().flatten().flatten() {
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
                // The first names in order, the rest counted.
                orphans_seen += 1;
                orphans.insert(id);
                if orphans.len() > MAX_ORPHANS_LISTED {
                    orphans.pop_last();
                }
            }
        }
        read.orphans_more = orphans_seen - orphans.len();
        read.orphans = orphans.into_iter().collect();
    }
    // The crates' digests, for the units perf measured — and those an
    // as-it-stands row left out with their crate's digest, which currency
    // compares (build note 24).
    let ledger = Ledger::new(root);
    for (id, krate) in units {
        if !read.units.contains_key(id) && !program_names(&read.program, id) {
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
        let mut budget = budget;
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        for w in &workloads.workloads {
            let now = match &w.input {
                None => InputNow::Digest(wl::digest(w, None)),
                Some(rel) => match file_key(root, rel, w) {
                    Some(key) if cache.contains_key(&key) => InputNow::Digest(cache[&key].clone()),
                    // One perf itself refuses, in its own words — known
                    // without reading it.
                    Some(key) if key.2 > wl::MAX_INPUT_BYTES => {
                        InputNow::Unusable(InputUnusable::TooLarge)
                    }
                    _ if measuring => InputNow::WhileMeasuring,
                    Some(key) if key.2 > budget => InputNow::TooLarge,
                    key => match wl::read_input(root, rel) {
                        Ok(bytes) => {
                            budget = budget.saturating_sub(bytes.len() as u64);
                            let d = wl::digest(w, Some(&bytes));
                            // Kept only while the file is still the one
                            // keyed: one swapped during the read is hashed
                            // again next time.
                            if let Some(key) =
                                key.filter(|k| file_key(root, rel, w).as_ref() == Some(k))
                            {
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

/// Whether an as-it-stands row names unit `id`: held, or left out with its
/// crate's digest (the build, link and replaces reasons, judged by it).
fn program_names(program: &Result<Option<ProgramResults>, String>, id: &str) -> bool {
    match program {
        Ok(Some(p)) => p.as_it_stands.iter().any(|r| {
            r.inputs.units.iter().flatten().any(|u| u.id == id)
                || r.inputs
                    .left_out
                    .iter()
                    .flatten()
                    .any(|l| l.id == id && !l.crate_digest.is_empty())
        }),
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

    /// A scratch root with `migration/perf/workloads.toml` holding one
    /// workload per (id, input), and an `outside/` folder beside it (outside
    /// the project).
    fn scratch(tag: &str, inputs: &[(&str, &str)]) -> (std::path::PathBuf, std::path::PathBuf) {
        let tmp =
            std::env::temp_dir().join(format!("harness-tui-perfread-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("root/migration/perf")).unwrap();
        std::fs::create_dir_all(tmp.join("outside")).unwrap();
        let tmp = tmp.canonicalize().unwrap();
        let root = tmp.join("root");
        let mut toml = String::from("schema_version = 1\n");
        for (id, input) in inputs {
            toml.push_str(&format!(
                "[[workload]]\nid = \"{id}\"\nargs = [\"{{input}}\"]\ninput = \"{input}\"\n"
            ));
        }
        std::fs::write(root.join("migration/perf/workloads.toml"), toml).unwrap();
        (tmp, root)
    }

    /// The cache never answers for perf's checks: a file moved out and
    /// linked back, a folder moved out and linked back, a chmod, another
    /// file of the same size renamed over it, or bytes rewritten in place —
    /// the old modification time put back each time — each reads as perf
    /// would read it now, never the cached digest.
    #[test]
    fn the_digest_cache_never_trusts_a_link_or_an_old_change_time() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (tmp, root) = scratch("cache", &[("w", "bench/in.txt")]);
        let outside = tmp.join("outside");
        std::fs::create_dir_all(root.join("bench")).unwrap();
        let input = root.join("bench/in.txt");
        std::fs::write(&input, b"hello").unwrap();
        let w = |r: &PerfRead| r.inputs.get("w").cloned();
        let Some(InputNow::Digest(d)) = w(&read(&root, &[], None)) else {
            panic!("a digest")
        };
        // The file moved out of the project (the same inode) and linked back.
        std::fs::rename(&input, outside.join("in.txt")).unwrap();
        symlink(outside.join("in.txt"), &input).unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Unusable(InputUnusable::Link))
        );
        std::fs::remove_file(&input).unwrap();
        std::fs::rename(outside.join("in.txt"), &input).unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Digest(d.clone()))
        );
        // Its folder moved out and linked back.
        std::fs::rename(root.join("bench"), outside.join("bench")).unwrap();
        symlink(outside.join("bench"), root.join("bench")).unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Unusable(InputUnusable::Outside))
        );
        std::fs::remove_file(root.join("bench")).unwrap();
        std::fs::rename(outside.join("bench"), root.join("bench")).unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Digest(d.clone()))
        );
        // Its permissions refuse the read (unless the tests run as root).
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&input).is_err() {
            assert_eq!(
                w(&read(&root, &[], None)),
                Some(InputNow::Unusable(InputUnusable::PermissionDenied))
            );
        }
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Digest(d.clone()))
        );
        // Another file of the same size renamed over it, the old
        // modification time put back: another inode, hashed again.
        let mtime = std::fs::metadata(&input).unwrap().modified().unwrap();
        let other = root.join("bench/other.txt");
        std::fs::write(&other, b"HELLO").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        std::fs::rename(&other, &input).unwrap();
        let Some(InputNow::Digest(swapped)) = w(&read(&root, &[], None)) else {
            panic!("a digest")
        };
        assert_ne!(swapped, d, "another file: hashed again");
        std::fs::write(&input, b"hello").unwrap();
        assert_eq!(
            w(&read(&root, &[], None)),
            Some(InputNow::Digest(d.clone()))
        );
        // Other bytes of the same size, the old modification time put back.
        let mtime = std::fs::metadata(&input).unwrap().modified().unwrap();
        std::fs::write(&input, b"HELLO").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&input)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let Some(InputNow::Digest(again)) = w(&read(&root, &[], None)) else {
            panic!("a digest")
        };
        assert_ne!(again, d, "the change time moved: hashed again");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The cache keys the workload as well as the file: two workloads on
    /// one input, and a workload whose options or input name changed (the
    /// same file under another name, a hard link), each read perf's own
    /// digest today — never one cached for another workload.
    #[test]
    fn the_digest_cache_keys_the_workload_too() {
        let (tmp, root) = scratch("workload", &[]);
        std::fs::write(root.join("in.txt"), b"hello").unwrap();
        std::fs::hard_link(root.join("in.txt"), root.join("n.txt")).unwrap();
        for workloads in [
            // Two workloads on one input.
            "id = \"w\"\nargs = [\"{input}\"]\ninput = \"in.txt\"\n\
             [[workload]]\nid = \"v\"\nargs = [\"{input}\"]\ninput = \"in.txt\"\n",
            // The same file under another name.
            "id = \"w\"\nargs = [\"{input}\"]\ninput = \"n.txt\"\n",
            // Its options changed: added, then split another way.
            "id = \"w\"\nargs = [\"-a\", \"-b\", \"{input}\"]\ninput = \"in.txt\"\n",
            "id = \"w\"\nargs = [\"-a-b\", \"{input}\"]\ninput = \"in.txt\"\n",
            // Its last option and its input's name changed together, run
            // into one another the same as before.
            "id = \"w\"\nargs = [\"{input}\", \"-\"]\ninput = \"in.txt\"\n",
            "id = \"w\"\nargs = [\"{input}\", \"-i\"]\ninput = \"n.txt\"\n",
        ] {
            std::fs::write(
                root.join("migration/perf/workloads.toml"),
                format!("schema_version = 1\n[[workload]]\n{workloads}"),
            )
            .unwrap();
            let r = read(&root, &[], None);
            let Ok(WorkloadsState::Ready(file)) = &r.workloads else {
                panic!("{:?}", r.workloads)
            };
            let perf: BTreeMap<String, InputNow> = file
                .workloads
                .iter()
                .map(|w| {
                    let d = wl::digest(w, Some(b"hello"));
                    (w.id.clone(), InputNow::Digest(d))
                })
                .collect();
            assert_eq!(r.inputs, perf, "{workloads}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Inputs past the load's budget read "can't check" (the load goes on);
    /// one over perf's own 64 MiB cap reads perf's words, unread, and so
    /// does a folder named as an input — never "can't check".
    #[test]
    fn an_input_over_the_budget_reads_cant_check() {
        let (tmp, root) = scratch(
            "budget",
            &[
                ("a", "a.txt"),
                ("b", "b.txt"),
                ("c", "c.txt"),
                ("huge", "huge.bin"),
                ("folder", "folder"),
            ],
        );
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(root.join(name), b"12345").unwrap();
        }
        std::fs::create_dir_all(root.join("folder")).unwrap();
        std::fs::write(root.join("folder/in.txt"), b"12345").unwrap();
        // Sparse: nothing is written, and nothing must be read.
        std::fs::File::create(root.join("huge.bin"))
            .unwrap()
            .set_len(wl::MAX_INPUT_BYTES + 1)
            .unwrap();
        let r = read_with_budget(&root, &[], None, 10);
        assert!(matches!(r.inputs["a"], InputNow::Digest(_)));
        assert!(matches!(r.inputs["b"], InputNow::Digest(_)));
        assert_eq!(r.inputs["c"], InputNow::TooLarge);
        assert_eq!(
            r.inputs["huge"],
            InputNow::Unusable(InputUnusable::TooLarge)
        );
        assert_eq!(
            r.inputs["folder"],
            InputNow::Unusable(InputUnusable::NotAFile)
        );
        // Cached inputs cost the next load nothing: c is hashed then.
        let r = read_with_budget(&root, &[], None, 10);
        assert!(matches!(r.inputs["c"], InputNow::Digest(_)));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A linked `migration/perf/units` is refused, never read through;
    /// stray results files are named up to a cap and counted beyond it.
    #[test]
    fn a_linked_units_folder_is_refused_and_strays_are_capped() {
        let (tmp, root) = scratch("units", &[]);
        let outside = tmp.join("outside");
        std::fs::write(outside.join("u001.json"), "{\"secret\": 1}").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("migration/perf/units")).unwrap();
        let r = read(&root, &[("u001".into(), None)], None);
        assert!(r.units.is_empty(), "{:?}", r.units);
        assert!(r.orphans.is_empty());
        assert_eq!(r.errors.len(), 1);
        assert!(
            r.errors[0].ends_with("migration/perf/units: must be a directory (a link is refused)"),
            "{:?}",
            r.errors
        );
        std::fs::remove_file(root.join("migration/perf/units")).unwrap();
        std::fs::create_dir(root.join("migration/perf/units")).unwrap();
        for i in 0..MAX_ORPHANS_LISTED + 5 {
            std::fs::write(
                root.join(format!("migration/perf/units/u{i:03}.json")),
                "{}",
            )
            .unwrap();
        }
        let r = read(&root, &[], None);
        assert_eq!(r.orphans.len(), MAX_ORPHANS_LISTED);
        assert_eq!(r.orphans[0], "u000");
        assert_eq!(r.orphans_more, 5);
        assert!(r.errors.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A linked `migration/perf` is never read through either: the results
    /// files behind it are outside the project.
    #[test]
    fn a_linked_perf_folder_is_never_read_through() {
        let (tmp, root) = scratch("perf-link", &[]);
        let outside = tmp.join("outside");
        std::fs::create_dir_all(outside.join("units")).unwrap();
        std::fs::write(outside.join("units/u001.json"), "{\"secret\": 1}").unwrap();
        std::fs::write(outside.join("program.json"), "{\"secret\": 1}").unwrap();
        std::fs::remove_dir_all(root.join("migration/perf")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("migration/perf")).unwrap();
        let r = read(&root, &[("u001".into(), None)], None);
        assert!(r.units.is_empty(), "{:?}", r.units);
        assert!(matches!(r.program, Ok(None)), "{:?}", r.program);
        assert!(
            r.workloads.as_ref().is_err_and(
                |e| e.ends_with("migration/perf: must be a directory (a link is refused)")
            ),
            "{:?}",
            r.workloads
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
