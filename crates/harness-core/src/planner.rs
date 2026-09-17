//! The deterministic planner (M1): cluster files into migration units,
//! collapse dependency cycles, and propose a topological order. A plain
//! function rather than a trait until a second strategy exists (recorded in
//! DECISIONS.md). LLM refinement and the human approval gate operate on the
//! reconciled plan.toml, never here.

use crate::error::Error;
use crate::facts::Facts;
use crate::hash;
use crate::plan::ComputedUnit;
use std::collections::{BTreeMap, BTreeSet};

/// Compute unit clusters from facts.
///
/// M1 granularity: one unit per source file that defines at least one public
/// symbol (C's natural translation unit); files in a mutual dependency cycle
/// merge into one unit. Header-defined internal symbols (e.g. static inline
/// functions) belong to no unit — their source still counts toward every
/// consuming unit's `source_hash` via the include closure.
///
/// Deterministic: identical facts yield identical clusters, ids, and order.
pub fn compute_units(facts: &Facts) -> Result<Vec<ComputedUnit>, Error> {
    // Which file defines each symbol (canonical id -> file).
    let sym_file: BTreeMap<&str, &str> = facts
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.file.as_str()))
        .collect();

    // Unit-seed files: those defining >= 1 public symbol.
    let unit_files: BTreeSet<&str> = facts
        .symbols
        .iter()
        .filter(|s| s.visibility == "public")
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
        let closure = facts.include_closure(&files);
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
