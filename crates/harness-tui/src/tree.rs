//! The navigator's tree (docs/COCKPIT-WRAPPER-DESIGN.md §2.2): the
//! [`Selection`] — keyed by path and id, never by row index — and the rows
//! the tree flattens into (only when the snapshot, the expansion or the
//! layout changes; the view draws only the visible ones).
//!
//! From the top: the project (the root, expanded and selected at start);
//! the group **Units (n)**, under each unit its crate node, then its
//! attempts in the defined order (TUI-DESIGN §2); the group **Features**;
//! **Speed**; then the source tree — its directories, then files, under a
//! file its functions in span order. The sections come first so that a
//! 30-row terminal shows all three headings (pass 2 of
//! docs/reviews/2026-10-18-cockpit-usability-critique.md): the source
//! folder is folded by default once the project has units (its row sums up
//! their marks), and a group whose rows would push the others off the
//! screen starts folded ([`defaults`]).

use crate::featmap::FeatureModel;
use crate::files::{Files, TreeWalk};
use crate::model::Snapshot;
use harness_core::walk::Why;
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
    /// The group of the person's features (docs/FEATURES-DESIGN.md §8.3).
    Features,
    /// A feature, by id.
    Feature(String),
    /// The C-vs-Rust speed (docs/PERF-DESIGN.md §3.11).
    Speed,
    /// The Project view's one row before a map: Map the project
    /// (docs/PROJECT-MAP-DESIGN.md §3.7).
    MapProject,
    /// A program or library of the project map, by id.
    Program(String),
    /// An accepted tool of the project, by id: Enter opens it.
    Tool(String),
    /// A tool's row back to the Project view.
    ProjectHome,
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
            Selection::Features => Some(Selection::Project),
            Selection::Feature(_) => Some(Selection::Features),
            Selection::Speed => Some(Selection::Project),
            Selection::MapProject
            | Selection::Program(_)
            | Selection::Tool(_)
            | Selection::ProjectHome => None,
        }
    }
}

/// Which nodes start folded: what [`defaults`] decides from the snapshot
/// and the tree's height. All open is the default (a height not known
/// yet, a test's tree).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Defaults {
    /// The top-level folders of the source tree (the project has units:
    /// the Units list is where a unit is, the folders sum up their marks).
    pub source_folded: bool,
    /// The units group (its rows would push Features and Speed off).
    pub units_folded: bool,
    /// The features group (its rows would push Speed and the source
    /// folder off).
    pub features_folded: bool,
}

impl Defaults {
    /// Whether `sel` is expanded unless folded (the project, directories,
    /// the groups — each as these defaults say), or folded unless opened
    /// (files, units).
    fn open(&self, sel: &Selection) -> bool {
        match sel {
            Selection::Project => true,
            // A nested folder opens with its parent.
            Selection::Dir(d) => !self.source_folded || d.contains('/'),
            Selection::Units => !self.units_folded,
            Selection::Features => !self.features_folded,
            _ => false,
        }
    }

    /// Whether the default of `sel` differs between `self` and `other`.
    fn differs(&self, other: &Defaults, sel: &Selection) -> bool {
        self.open(sel) != other.open(sel)
    }
}

/// The folds a tree of `height` rows starts with (0: not known yet, all
/// open): the source folder folded when the project has units; the units
/// group folded when its rows, with the other headings, do not fit; then
/// the features group, likewise. The headings are the project, Units,
/// Features, Speed and the source folder.
pub fn defaults(
    snapshot: &Snapshot,
    files: &Files,
    features: &FeatureModel,
    height: usize,
) -> Defaults {
    let units = snapshot.units.len();
    let source = usize::from(!files.files.is_empty());
    let headings = 1 + usize::from(units > 0) + 1 + 1 + source;
    let fits = |rows: usize| height == 0 || rows <= height;
    let units_folded = units > 0 && !fits(headings + units);
    let shown_units = if units_folded { 0 } else { units };
    let features_folded = !fits(headings + shown_units + features.features.len());
    Defaults {
        source_folded: units > 0,
        units_folded,
        features_folded,
    }
}

/// Which nodes differ from their default expansion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    changed: BTreeSet<Selection>,
    defaults: Defaults,
}

impl Expansion {
    /// Whether `sel` is expanded.
    pub fn is_open(&self, sel: &Selection) -> bool {
        self.defaults.open(sel) != self.changed.contains(sel)
    }

    /// Open or fold `sel`.
    pub fn set(&mut self, sel: &Selection, open: bool) {
        if open == self.defaults.open(sel) {
            self.changed.remove(sel);
        } else {
            self.changed.insert(sel.clone());
        }
    }

    /// Take new defaults: a node the person opened or folded keeps how it
    /// is; one left alone follows the new default. `true` when they
    /// changed.
    pub fn set_defaults(&mut self, defaults: Defaults) -> bool {
        if defaults == self.defaults {
            return false;
        }
        let old = self.defaults;
        // A recorded change against the old default is the new default.
        self.changed.retain(|sel| !old.differs(&defaults, sel));
        self.defaults = defaults;
        true
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
    /// A group's heading in the Project view ("Programs (3)"): never
    /// selected.
    Heading(String),
    /// A file holding a `main` that did not compile, in the Project view:
    /// drawn as a program's row (`✗ <file>`, its state at the right), never
    /// selected (it has no id to act on).
    Failed(String),
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
            RowKind::Note(_) | RowKind::Heading(_) | RowKind::Failed(_) => None,
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
    features: &FeatureModel,
    walk: &TreeWalk,
    expansion: &Expansion,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let open = expansion.is_open(&Selection::Project);
    push_node(&mut rows, 0, Selection::Project, true, open);
    if !open {
        return rows;
    }
    // The sections first: a unit is in one place, the Units list.
    if !snapshot.units.is_empty() {
        let open = expansion.is_open(&Selection::Units);
        push_node(&mut rows, 1, Selection::Units, true, open);
        if open {
            for u in &snapshot.units {
                let id = u.unit.id.clone();
                let sel = Selection::Unit(id.clone());
                let open = expansion.is_open(&sel);
                // A ▸ only when something is under it (the whole-journey
                // check, item 18): its Rust, or its tries.
                let children = u.crate_dir.is_some() || !u.attempts.is_empty();
                push_node(&mut rows, 2, sel, children, open && children);
                if open && children {
                    // Its Rust, only once it exists: a row saying "no Rust
                    // yet" read as a promise, and never kept for a piece
                    // that stays C (2026-10-20).
                    if u.crate_dir.is_some() {
                        push_node(&mut rows, 3, Selection::Crate(id.clone()), false, false);
                    }
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
    // The person's features, always: the group says when there are none.
    let open = expansion.is_open(&Selection::Features);
    let expandable = !features.features.is_empty();
    push_node(&mut rows, 1, Selection::Features, expandable, open);
    if open {
        for f in &features.features {
            push_node(&mut rows, 2, Selection::Feature(f.id.clone()), false, false);
        }
    }
    // Speed, always beside Features: the label says its state.
    push_node(&mut rows, 1, Selection::Speed, false, false);
    // Then the source tree, and what its walk could not list.
    let dirs = dirs_of(files);
    dir_rows(&mut rows, 1, "", &dirs, files, expansion);
    for (path, files) in &walk.skipped_folders {
        rows.push(Row {
            depth: 1,
            kind: RowKind::Note(format!(
                "skipped folder {path}: {files} C file(s), not read"
            )),
            expandable: false,
            open: false,
        });
    }
    for (path, why) in &walk.issues {
        let note = match why {
            Why::Unreadable(error) => format!("unreadable {path}: {error}"),
            why => format!("skipped {path}: {}", why.words()),
        };
        rows.push(Row {
            depth: 1,
            kind: RowKind::Note(note),
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
    rows
}

/// The row of `sel`, if it is shown.
pub fn row_of(rows: &[Row], sel: &Selection) -> Option<usize> {
    rows.iter().position(|r| r.selection() == Some(sel))
}

/// Whether `sel` names a node that exists in this snapshot and tree
/// (shown or folded away).
pub fn exists(
    snapshot: &Snapshot,
    files: &Files,
    features: &FeatureModel,
    sel: &Selection,
) -> bool {
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
        Selection::Unit(id) => snapshot.unit(id).is_some(),
        Selection::Crate(id) => snapshot.unit(id).is_some_and(|u| u.crate_dir.is_some()),
        Selection::Attempt(u, a) => snapshot.unit(u).and_then(|u| u.attempt(a)).is_some(),
        Selection::Features => true,
        Selection::Feature(id) => features.feature(id).is_some(),
        Selection::Speed => true,
        // The Project view's rows and the row back to it are the App's to
        // keep (they come from the map, not the snapshot).
        Selection::MapProject
        | Selection::Program(_)
        | Selection::Tool(_)
        | Selection::ProjectHome => true,
    }
}

/// `sel` if it still exists, else its nearest existing ancestor.
pub fn surviving(
    snapshot: &Snapshot,
    files: &Files,
    features: &FeatureModel,
    sel: &Selection,
) -> Selection {
    let mut at = sel.clone();
    while !exists(snapshot, files, features, &at) {
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
