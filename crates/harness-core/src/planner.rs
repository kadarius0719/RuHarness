//! The deterministic planner (M1): cluster files into migration units,
//! collapse dependency cycles, and propose a topological order. A plain
//! function rather than a trait until a second strategy exists (recorded in
//! DECISIONS.md). LLM refinement and the human approval gate operate on the
//! reconciled plan.toml, never here.

use crate::error::Error;
use crate::facts::Facts;
use crate::hash;
use crate::plan::ComputedUnit;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Compute unit clusters from facts.
///
/// M1 granularity: one unit per source file that defines at least one public
/// symbol (C's natural translation unit); files in a mutual dependency cycle
/// merge into one unit. Header-defined internal symbols (e.g. static inline
/// functions) belong to no unit — their source still counts toward every
/// consuming unit's `source_hash` via the include closure.
///
/// Deterministic: identical facts yield identical clusters, ids, and order.
///
/// Each unit's `source_hash` covers the facts' include closure: right for a
/// folder target. `harness plan` calls [`compute_units_in`], which hashes a
/// file-list target's compile closure instead.
pub fn compute_units(facts: &Facts) -> Result<Vec<ComputedUnit>, Error> {
    compute_units_with(facts, &mut |files| facts.include_closure(files))
}

/// [`compute_units`] for the target `ctx`: each unit's `source_hash`
/// covers the files its compile reads ([`crate::sources::unit_closure`];
/// the 2026-10-08 triage, decision 11), so for a file-list target a header
/// an ambiguous include lands on is hashed too. A folder target's hashes
/// are [`compute_units`]'s, byte for byte.
pub fn compute_units_in(
    ctx: &crate::TargetContext,
    facts: &Facts,
) -> Result<Vec<ComputedUnit>, Error> {
    let Ok(Some(resolver)) = crate::sources::Resolver::of(ctx) else {
        return compute_units(facts);
    };
    let root = resolver.root().to_path_buf();
    // Each file's include names read once for every unit.
    let mut names: BTreeMap<String, Option<Vec<(String, bool)>>> = BTreeMap::new();
    compute_units_with(facts, &mut |files| {
        resolver.closure_with(facts, files, &mut |rel| {
            names
                .entry(rel.to_string())
                .or_insert_with(|| crate::sources::names_on_disk(&root, rel))
                .clone()
        })
    })
}

fn compute_units_with(
    facts: &Facts,
    closure_of: &mut dyn FnMut(&[String]) -> Vec<String>,
) -> Result<Vec<ComputedUnit>, Error> {
    // Which file defines each symbol (canonical id -> file).
    let sym_file: BTreeMap<&str, &str> = facts
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.file.as_str()))
        .collect();

    // Unit-seed files: those defining >= 1 public symbol — never a header,
    // which no compile builds on its own: a function written in one is a
    // copy in every file that includes it (docs/HEADER-FUNCTIONS-DESIGN.md
    // §3), and a piece cut from the header alone has nothing to test.
    let unit_files: BTreeSet<&str> = facts
        .symbols
        .iter()
        .filter(|s| s.visibility == "public" && !is_header(&s.file))
        .map(|s| s.file.as_str())
        .collect();

    // File-level dependency edges between unit-seed files.
    let mut edges: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in &unit_files {
        edges.entry(f).or_default();
    }
    for r in &facts.refs {
        if !r.resolved {
            continue;
        }
        let from_file = r.file.as_str();
        if let Some(&to_file) = sym_file.get(r.to.as_str()) {
            if from_file != to_file
                && unit_files.contains(from_file)
                && unit_files.contains(to_file)
            {
                edges.entry(from_file).or_default().insert(to_file);
            }
        }
    }

    // Strongly connected components (iterative Tarjan), so cyclic file groups
    // become a single unit.
    let sccs = tarjan_sccs(&edges);

    // Build units per SCC.
    let mut file_to_unit: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, scc) in sccs.iter().enumerate() {
        for f in scc {
            file_to_unit.insert(f, i);
        }
    }
    // Stem-derived ids, made collision-proof: any id shared by two SCCs is
    // regenerated from full paths (silently dropping a unit is the one thing
    // a planner must never do).
    let mut ids: Vec<String> = sccs
        .iter()
        .map(|scc| unit_id(scc, /* full_paths= */ false))
        .collect();
    {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for id in &ids {
            *counts.entry(id.as_str()).or_insert(0) += 1;
        }
        let colliding: BTreeSet<String> = counts
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(id, _)| id.to_string())
            .collect();
        for (i, scc) in sccs.iter().enumerate() {
            if colliding.contains(&ids[i]) {
                ids[i] = unit_id(scc, /* full_paths= */ true);
            }
        }
        let unique: BTreeSet<&String> = ids.iter().collect();
        if unique.len() != ids.len() {
            return Err(Error::Invariant(format!(
                "planner id collision even with path-derived ids: {}",
                ids.join(", ")
            )));
        }
    }

    let mut units: Vec<ComputedUnit> = Vec::with_capacity(sccs.len());
    for (scc, id) in sccs.iter().zip(ids) {
        let files: Vec<String> = scc.iter().map(|f| f.to_string()).collect();
        let mut symbols = Vec::new();
        let mut interface = Vec::new();
        for s in &facts.symbols {
            if s.visibility == "public" && scc.iter().any(|f| *f == s.file) {
                symbols.push(s.name.clone());
                interface.push(s.signature.clone());
            }
        }
        let closure = closure_of(&files);
        let pairs: Vec<(String, String)> = closure
            .iter()
            .filter_map(|p| {
                facts
                    .files
                    .iter()
                    .find(|f| &f.path == p)
                    .map(|f| (f.path.clone(), f.hash.clone()))
            })
            .collect();
        units.push(ComputedUnit {
            id,
            files,
            symbols,
            interface,
            not_built: Vec::new(),
            build_only: Vec::new(),
            depends_on: Vec::new(), // filled below once all ids exist
            source_hash: hash::file_set_hash(&pairs),
        });
    }

    // Unit-level depends_on from file-level edges.
    for (i, scc) in sccs.iter().enumerate() {
        let mut deps: BTreeSet<String> = BTreeSet::new();
        for f in scc {
            if let Some(tos) = edges.get(f) {
                for to in tos {
                    let j = file_to_unit[to];
                    if j != i {
                        deps.insert(units[j].id.clone());
                    }
                }
            }
        }
        units[i].depends_on = deps.into_iter().collect();
    }

    // Advisory order: topological (deps first), unit-id tiebreak.
    topo_sort(units)
}

/// Let the build have the last word on a unit's functions, but only when it
/// clearly agrees with the scan. The scan reads the C without preprocessing,
/// so a function under an `#if` the build leaves out is planned yet never
/// compiled, and a driver asked to call it can never be written.
///
/// `built` maps a file path to the function names its compiled object
/// defines, holding only files that compiled ([`built_functions`]). A unit is
/// looked at only when every one of its files is in `built`. Then, with `S`
/// its planned symbols and `D` the functions its files' objects define:
/// planned names the build lacks are moved from `symbols` (and the matching
/// `interface` lines) to `not_built`, in their planned order, when the build
/// defines no name the plan does not know and shares at least one with it;
/// when the build does define names the plan does not know (a build flag
/// that renames them), nothing is removed and those names go to
/// `build_only`, sorted. A build that defines none of the unit's functions
/// changes nothing.
pub fn keep_built(units: &mut [ComputedUnit], built: &BTreeMap<String, BTreeSet<String>>) {
    for unit in units.iter_mut() {
        let mut defined: BTreeSet<&str> = BTreeSet::new();
        let mut all_built = !unit.files.is_empty();
        for f in &unit.files {
            match built.get(f) {
                Some(names) => defined.extend(names.iter().map(String::as_str)),
                None => all_built = false,
            }
        }
        if !all_built {
            continue;
        }
        let planned: BTreeSet<&str> = unit.symbols.iter().map(String::as_str).collect();
        let build_only: Vec<String> = defined
            .iter()
            .filter(|n| !planned.contains(*n))
            .map(|n| n.to_string())
            .collect();
        if !build_only.is_empty() {
            unit.build_only = build_only;
            continue;
        }
        let shared = unit.symbols.iter().any(|s| defined.contains(s.as_str()));
        if !shared {
            continue;
        }
        let keep: Vec<bool> = unit
            .symbols
            .iter()
            .map(|s| defined.contains(s.as_str()))
            .collect();
        let mut symbols = Vec::with_capacity(unit.symbols.len());
        let mut interface = Vec::with_capacity(unit.interface.len());
        let mut not_built = Vec::new();
        let mut lines = std::mem::take(&mut unit.interface).into_iter();
        for (name, kept) in std::mem::take(&mut unit.symbols).into_iter().zip(keep) {
            let line = lines.next();
            if kept {
                symbols.push(name);
                interface.extend(line);
            } else {
                not_built.push(name);
            }
        }
        interface.extend(lines);
        unit.symbols = symbols;
        unit.interface = interface;
        unit.not_built = not_built;
    }
}

/// The largest project map [`built_functions`] reads.
const MAX_BUILT_MAP_BYTES: u64 = 256 << 20;

/// The function names each compiled file of the project map defines, for
/// [`keep_built`]: file path to names, holding only files the map records as
/// compiled `"ok"` and whose size on disk still matches the map's (a file
/// changed since the map could define a function the map does not know, so
/// it is left out and its unit planned from the scan alone). `Ok(None)` when
/// `root` has no map; `Err` with one line when the map cannot be read.
pub fn built_functions(root: &Path) -> Result<Option<BTreeMap<String, BTreeSet<String>>>, String> {
    #[derive(Deserialize)]
    struct Defined {
        name: String,
        kind: String,
    }
    #[derive(Deserialize)]
    struct File {
        path: String,
        #[serde(default)]
        compiled: Option<serde_json::Value>,
        bytes: u64,
        #[serde(default)]
        defined_symbols: Vec<Defined>,
    }
    #[derive(Deserialize)]
    struct Map {
        files: Vec<File>,
    }
    let path = root.join(crate::ledger::PROJECT_MAP_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => {}
    }
    let unreadable = |why: String| {
        format!(
            "the project map ({}) could not be read ({why}): run `harness project map` again",
            crate::ledger::PROJECT_MAP_FILE
        )
    };
    let bytes = crate::ledger::read_regular(&path, MAX_BUILT_MAP_BYTES)
        .map_err(|e| unreadable(e.to_string()))?;
    let map: Map = serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))?;
    let mut built = BTreeMap::new();
    for file in map.files {
        if file.compiled.as_ref().and_then(|c| c.as_str()) != Some("ok") {
            continue;
        }
        let on_disk = std::fs::symlink_metadata(root.join(&file.path)).map(|m| m.len());
        if on_disk.ok() != Some(file.bytes) {
            continue;
        }
        let names: BTreeSet<String> = file
            .defined_symbols
            .into_iter()
            .filter(|d| d.kind == "function")
            .map(|d| d.name)
            .collect();
        built.insert(file.path, names);
    }
    Ok(Some(built))
}

/// A header by its extension (`.h`, `.hh`, `.hpp`, `.hxx`): a file no
/// compile builds on its own.
pub fn is_header(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "h" | "hh" | "hpp" | "hxx"))
}

/// Deterministic unit id from its file cluster: `u-<stem>` for a single
/// file, stems joined by `-` (sorted) for merged cycles. With `full_paths`
/// (the collision fallback), each file contributes its full repo-relative
/// path slug (extension dropped, separators to `-`) instead of the bare
/// stem, e.g. `u-src-x-util` vs `u-src-y-util`.
fn unit_id(files: &[&str], full_paths: bool) -> String {
    let mut parts: Vec<String> = files
        .iter()
        .map(|f| {
            if full_paths {
                let no_ext = std::path::Path::new(f)
                    .with_extension("")
                    .to_string_lossy()
                    .into_owned();
                no_ext.replace(['/', '\\'], "-")
            } else {
                std::path::Path::new(f)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| (*f).to_string())
            }
        })
        .collect();
    parts.sort();
    format!("u-{}", parts.join("-"))
}

fn topo_sort(units: Vec<ComputedUnit>) -> Result<Vec<ComputedUnit>, Error> {
    let mut by_id: BTreeMap<String, ComputedUnit> =
        units.into_iter().map(|u| (u.id.clone(), u)).collect();
    let mut indegree: BTreeMap<String, usize> = by_id.keys().map(|id| (id.clone(), 0)).collect();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for u in by_id.values() {
        for d in &u.depends_on {
            *indegree.entry(u.id.clone()).or_insert(0) += 1;
            dependents.entry(d.clone()).or_default().push(u.id.clone());
        }
    }
    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut order: Vec<ComputedUnit> = Vec::with_capacity(by_id.len());
    while let Some(id) = ready.iter().next().cloned() {
        ready.remove(&id);
        for dep in dependents.get(&id).cloned().unwrap_or_default() {
            let e = indegree
                .get_mut(&dep)
                .ok_or_else(|| Error::Invariant(format!("missing indegree for {dep}")))?;
            *e -= 1;
            if *e == 0 {
                ready.insert(dep);
            }
        }
        let unit = by_id
            .remove(&id)
            .ok_or_else(|| Error::Invariant(format!("missing unit {id}")))?;
        order.push(unit);
    }
    if !by_id.is_empty() {
        // SCC collapse precedes this sort, so a cycle here is a planner bug.
        return Err(Error::Invariant(format!(
            "internal planner cycle among: {}",
            by_id.keys().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(order)
}

/// Iterative Tarjan SCC over a BTreeMap adjacency (deterministic order).
/// Returns SCCs, each sorted, in deterministic order.
fn tarjan_sccs<'a>(edges: &BTreeMap<&'a str, BTreeSet<&'a str>>) -> Vec<Vec<&'a str>> {
    struct Frame<'a> {
        node: &'a str,
        neighbors: Vec<&'a str>,
        next: usize,
    }
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lowlink: BTreeMap<&str, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut counter = 0usize;
    let mut sccs: Vec<Vec<&str>> = Vec::new();

    for &start in edges.keys() {
        if index.contains_key(start) {
            continue;
        }
        let mut call_stack: Vec<Frame> = vec![Frame {
            node: start,
            neighbors: edges
                .get(start)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default(),
            next: 0,
        }];
        index.insert(start, counter);
        lowlink.insert(start, counter);
        counter += 1;
        stack.push(start);
        on_stack.insert(start);

        while let Some(frame) = call_stack.last_mut() {
            if frame.next < frame.neighbors.len() {
                let w = frame.neighbors[frame.next];
                frame.next += 1;
                if !index.contains_key(w) {
                    index.insert(w, counter);
                    lowlink.insert(w, counter);
                    counter += 1;
                    stack.push(w);
                    on_stack.insert(w);
                    call_stack.push(Frame {
                        node: w,
                        neighbors: edges
                            .get(w)
                            .map(|s| s.iter().copied().collect())
                            .unwrap_or_default(),
                        next: 0,
                    });
                } else if on_stack.contains(w) {
                    let wl = index[w];
                    let v = frame.node;
                    let cur = lowlink[v];
                    lowlink.insert(v, cur.min(wl));
                }
            } else {
                let v = frame.node;
                call_stack.pop();
                if let Some(parent) = call_stack.last() {
                    let vl = lowlink[v];
                    let p = parent.node;
                    let cur = lowlink[p];
                    lowlink.insert(p, cur.min(vl));
                }
                if lowlink[v] == index[v] {
                    let mut scc = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack.remove(w);
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    scc.sort();
                    sccs.push(scc);
                }
            }
        }
    }
    sccs.sort();
    sccs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(files: &[&str], symbols: &[&str]) -> ComputedUnit {
        ComputedUnit {
            id: "u-a".into(),
            files: files.iter().map(|f| f.to_string()).collect(),
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
            interface: symbols.iter().map(|s| format!("int {s}(void)")).collect(),
            not_built: Vec::new(),
            build_only: Vec::new(),
            depends_on: Vec::new(),
            source_hash: String::new(),
        }
    }

    fn built(entries: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        entries
            .iter()
            .map(|(f, names)| (f.to_string(), names.iter().map(|n| n.to_string()).collect()))
            .collect()
    }

    /// A build that defines exactly the planned functions changes nothing.
    #[test]
    fn keep_built_leaves_a_matching_unit_alone() {
        let mut units = vec![unit(&["a.c"], &["b", "a"])];
        let before = units[0].clone();
        keep_built(&mut units, &built(&[("a.c", &["a", "b"])]));
        assert_eq!(units[0].symbols, before.symbols);
        assert_eq!(units[0].interface, before.interface);
        assert!(units[0].not_built.is_empty() && units[0].build_only.is_empty());
    }

    /// A planned function the build does not compile leaves `symbols` with
    /// its `interface` line, and the rest keep their order.
    #[test]
    fn keep_built_leaves_out_a_function_the_build_lacks() {
        let mut units = vec![unit(&["a.c"], &["c", "hexOut", "a"])];
        keep_built(&mut units, &built(&[("a.c", &["a", "c"])]));
        assert_eq!(units[0].symbols, ["c", "a"]);
        assert_eq!(units[0].interface, ["int c(void)", "int a(void)"]);
        assert_eq!(units[0].not_built, ["hexOut"]);
        assert!(units[0].build_only.is_empty());
    }

    /// Names the build defines that the plan does not know mean the two do
    /// not clearly agree: nothing is removed, the names are recorded sorted.
    #[test]
    fn keep_built_removes_nothing_when_the_build_has_unknown_names() {
        let mut units = vec![unit(&["x.c"], &["XXH32", "XXH64"])];
        keep_built(
            &mut units,
            &built(&[("x.c", &["LZ4_XXH64", "LZ4_XXH32", "XXH32"])]),
        );
        assert_eq!(units[0].symbols, ["XXH32", "XXH64"]);
        assert_eq!(units[0].interface.len(), 2);
        assert!(units[0].not_built.is_empty());
        assert_eq!(units[0].build_only, ["LZ4_XXH32", "LZ4_XXH64"]);
    }

    /// A unit with a file the map does not hold as compiled is untouched.
    #[test]
    fn keep_built_skips_a_unit_with_a_file_not_built() {
        let mut units = vec![unit(&["a.c", "b.c"], &["a", "b"])];
        keep_built(&mut units, &built(&[("a.c", &["a"])]));
        assert_eq!(units[0].symbols, ["a", "b"]);
        assert!(units[0].not_built.is_empty() && units[0].build_only.is_empty());
    }

    /// A build that defines none of the unit's functions changes nothing.
    #[test]
    fn keep_built_skips_a_unit_whose_build_defines_no_function() {
        let mut units = vec![unit(&["a.c"], &["a", "b"])];
        keep_built(&mut units, &built(&[("a.c", &[])]));
        assert_eq!(units[0].symbols, ["a", "b"]);
        assert!(units[0].not_built.is_empty() && units[0].build_only.is_empty());
    }

    /// A merged unit compares its planned functions with every file's build:
    /// a function built by its other file is not left out.
    #[test]
    fn keep_built_takes_the_union_of_a_units_files() {
        let mut units = vec![unit(&["a.c", "b.c"], &["a", "b", "gone"])];
        keep_built(&mut units, &built(&[("a.c", &["a"]), ("b.c", &["b"])]));
        assert_eq!(units[0].symbols, ["a", "b"]);
        assert_eq!(units[0].not_built, ["gone"]);
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("ruharness-built-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("migration/map")).unwrap();
        root
    }

    /// The reader keeps a file compiled "ok" with its function names only,
    /// skips one not compiled (either form) or changed in size since the
    /// map, says nothing without a map and refuses one it cannot parse.
    #[test]
    fn built_functions_reads_only_compiled_files_functions() {
        let root = scratch("read");
        assert_eq!(built_functions(&root), Ok(None));
        for (name, body) in [("a.c", "12345"), ("b.c", "1"), ("c.c", "1"), ("d.c", "123")] {
            std::fs::write(root.join(name), body).unwrap();
        }
        let map = r#"{"schema": "ruharness-project-map", "files": [
            {"path": "a.c", "compiled": "ok", "bytes": 5, "extra": 1,
             "defined_symbols": [{"name": "fa", "kind": "function"},
                                 {"name": "table", "kind": "read-only"}]},
            {"path": "b.c", "compiled": null, "bytes": 1, "defined_symbols": []},
            {"path": "c.c", "compiled": {"reason": "syntax"}, "bytes": 1,
             "defined_symbols": [{"name": "fc", "kind": "function"}]},
            {"path": "d.c", "compiled": "ok", "bytes": 99,
             "defined_symbols": [{"name": "fd", "kind": "function"}]}
        ]}"#;
        std::fs::write(root.join(crate::ledger::PROJECT_MAP_FILE), map).unwrap();
        let got = built_functions(&root).unwrap().unwrap();
        let want: BTreeMap<String, BTreeSet<String>> =
            [("a.c".to_string(), ["fa".to_string()].into_iter().collect())]
                .into_iter()
                .collect();
        assert_eq!(got, want);
        std::fs::write(root.join(crate::ledger::PROJECT_MAP_FILE), "not json").unwrap();
        let err = built_functions(&root).unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        assert!(!err.contains('\n'), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
