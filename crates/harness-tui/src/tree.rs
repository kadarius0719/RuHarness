//! The navigator's tree (docs/COCKPIT-WRAPPER-DESIGN.md §2.2): the
//! [`Selection`] — keyed by path and id, never by row index — and the rows
//! the tree flattens into (only when the snapshot, the expansion or the
//! layout changes; the view draws only the visible ones).
//!
//! From the top: the project (the root, expanded and selected at start);
//! its directories, then files; under a file its functions in span order;
//! after the source tree the group **Units (n)**, and under each unit its
//! crate node, then its attempts in the defined order (TUI-DESIGN §2).

use crate::files::{Files, TreeWalk};
use crate::model::Snapshot;
use harness_core::walk::Skip;
use std::collections::BTreeSet;

/// What is selected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selection {
    /// The project (the root).
    Project,
    /// A directory, repo-relative.
    Dir(String),
    /// A file, repo-relative.
    File(String),
    /// A function: its file, its name.
    Function(String, String),
    /// The group of units.
    Units,
    /// A unit, by id.
    Unit(String),
    /// A unit's crate.
    Crate(String),
    /// An attempt: its unit, its id.
    Attempt(String, String),
}

impl Selection {
    /// Where the selection moves when its node vanished.
    pub fn parent(&self) -> Option<Selection> {
        fn dir_of(path: &str) -> Selection {
            match path.rsplit_once('/') {
                Some((dir, _)) => Selection::Dir(dir.to_string()),
                None => Selection::Project,
            }
        }
        match self {
            Selection::Project => None,
            Selection::Dir(d) | Selection::File(d) => Some(dir_of(d)),
            Selection::Function(file, _) => Some(Selection::File(file.clone())),
            Selection::Units => Some(Selection::Project),
            Selection::Unit(_) => Some(Selection::Units),
            Selection::Crate(u) | Selection::Attempt(u, _) => Some(Selection::Unit(u.clone())),
        }
    }

    /// Expanded unless folded (the project, directories, the units group),
    /// or folded unless opened (files, units).
    fn open_by_default(&self) -> bool {
        matches!(
            self,
            Selection::Project | Selection::Dir(_) | Selection::Units
        )
    }
}

/// Which nodes differ from their default expansion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion(BTreeSet<Selection>);

impl Expansion {
    /// Whether `sel` is expanded.
    pub fn is_open(&self, sel: &Selection) -> bool {
        sel.open_by_default() != self.0.contains(sel)
    }

    /// Open or fold `sel`.
    pub fn set(&mut self, sel: &Selection, open: bool) {
        if open == sel.open_by_default() {
            self.0.remove(sel);
        } else {
            self.0.insert(sel.clone());
        }
    }
}

/// What a row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// A node: selectable.
    Node(Selection),
    /// A note in the tree (a walk error, a skipped entry, a truncation):
    /// never selected.
    Note(String),
}

/// One row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Indent level (the project is 0).
    pub depth: usize,
    /// What it is.
    pub kind: RowKind,
    /// It has children.
    pub expandable: bool,
    /// Its children are shown.
    pub open: bool,
}

impl Row {
    /// Its selection, for a node.
    pub fn selection(&self) -> Option<&Selection> {
        match &self.kind {
            RowKind::Node(sel) => Some(sel),
            RowKind::Note(_) => None,
        }
    }
}

/// The directories holding listed files: every proper prefix of a path.
fn dirs_of(files: &Files) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for f in &files.files {
        let mut at = 0;
        while let Some(i) = f.path[at..].find('/') {
            dirs.insert(f.path[..at + i].to_string());
            at += i + 1;
        }
    }
    dirs
}

fn push_node(rows: &mut Vec<Row>, depth: usize, sel: Selection, expandable: bool, open: bool) {
    rows.push(Row {
        depth,
        kind: RowKind::Node(sel),
        expandable,
        open: expandable && open,
    });
}

/// The children of directory `dir` (`""` = the root): its directories,
/// then its files, each sorted.
fn dir_rows(
    rows: &mut Vec<Row>,
    depth: usize,
    dir: &str,
    dirs: &BTreeSet<String>,
    files: &Files,
    expansion: &Expansion,
) {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let direct = |path: &str| {
        path.strip_prefix(prefix.as_str())
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    };
    for d in dirs.iter().filter(|d| direct(d)) {
        let sel = Selection::Dir(d.clone());
        let open = expansion.is_open(&sel);
        push_node(rows, depth, sel, true, open);
        if open {
            dir_rows(rows, depth + 1, d, dirs, files, expansion);
        }
    }
    for f in files.files.iter().filter(|f| direct(&f.path)) {
        let sel = Selection::File(f.path.clone());
        let open = expansion.is_open(&sel);
        push_node(rows, depth, sel, !f.functions.is_empty(), open);
        if open {
            for func in &f.functions {
                push_node(
                    rows,
                    depth + 1,
                    Selection::Function(f.path.clone(), func.name.clone()),
                    false,
                    false,
                );
            }
        }
    }
}

/// Flatten the tree into rows.
pub fn rows(
    snapshot: &Snapshot,
    files: &Files,
    walk: &TreeWalk,
    expansion: &Expansion,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let open = expansion.is_open(&Selection::Project);
    push_node(&mut rows, 0, Selection::Project, true, open);
    if !open {
        return rows;
    }
    let dirs = dirs_of(files);
    dir_rows(&mut rows, 1, "", &dirs, files, expansion);
    for (path, why) in &walk.skipped {
        let why = match why {
            Skip::NotRegular => "not a regular file",
        };
        rows.push(Row {
            depth: 1,
            kind: RowKind::Note(format!("skipped {path}: {why}")),
            expandable: false,
            open: false,
        });
    }
    for (path, error) in &walk.errors {
        rows.push(Row {
            depth: 1,
            kind: RowKind::Note(format!("unreadable {path}: {error}")),
            expandable: false,
            open: false,
        });
    }
    if walk.truncated {
        rows.push(Row {
            depth: 1,
            kind: RowKind::Note("… more files not listed (limit 20 000 files, 32 levels)".into()),
            expandable: false,
            open: false,
        });
    }
    if !snapshot.units.is_empty() {
        let open = expansion.is_open(&Selection::Units);
        push_node(&mut rows, 1, Selection::Units, true, open);
        if open {
            for u in &snapshot.units {
                let id = u.unit.id.clone();
                let sel = Selection::Unit(id.clone());
                let open = expansion.is_open(&sel);
                push_node(&mut rows, 2, sel, true, open);
                if open {
                    push_node(&mut rows, 3, Selection::Crate(id.clone()), false, false);
                    for a in &u.attempts {
                        push_node(
                            &mut rows,
                            3,
                            Selection::Attempt(id.clone(), a.record.id.clone()),
                            false,
                            false,
                        );
                    }
                }
            }
        }
    }
    rows
}

/// The row of `sel`, if it is shown.
pub fn row_of(rows: &[Row], sel: &Selection) -> Option<usize> {
    rows.iter().position(|r| r.selection() == Some(sel))
}

/// Whether `sel` names a node that exists in this snapshot and tree
/// (shown or folded away).
pub fn exists(snapshot: &Snapshot, files: &Files, sel: &Selection) -> bool {
    match sel {
        Selection::Project => true,
        Selection::Dir(d) => {
            let prefix = format!("{d}/");
            files.files.iter().any(|f| f.path.starts_with(&prefix))
        }
        Selection::File(p) => files.file(p).is_some(),
        Selection::Function(p, name) => files
            .file(p)
            .is_some_and(|f| f.functions.iter().any(|func| &func.name == name)),
        Selection::Units => !snapshot.units.is_empty(),
        Selection::Unit(id) | Selection::Crate(id) => snapshot.unit(id).is_some(),
        Selection::Attempt(u, a) => snapshot.unit(u).and_then(|u| u.attempt(a)).is_some(),
    }
}

/// `sel` if it still exists, else its nearest existing ancestor.
pub fn surviving(snapshot: &Snapshot, files: &Files, sel: &Selection) -> Selection {
    let mut at = sel.clone();
    while !exists(snapshot, files, &at) {
        match at.parent() {
            Some(p) => at = p,
            None => return Selection::Project,
        }
    }
    at
}

/// Open every ancestor of `sel` so that its row is shown.
pub fn reveal(expansion: &mut Expansion, sel: &Selection) {
    let mut at = sel.parent();
    while let Some(p) = at {
        expansion.set(&p, true);
        at = p.parent();
    }
}
