//! The Project view's state and acts (docs/PROJECT-MAP-DESIGN.md §3.7; pass
//! 3 of docs/reviews/2026-10-18-cockpit-usability-critique.md): on a project
//! with no root `harness.toml` the cockpit opens on the project — its left
//! pane the rows of what the map found, its right pane the project in words
//! — with the same menus, dialogs, activity details and keys as a tool's
//! screen. Map, Ask and Accept run `harness project …` through the cockpit's
//! own armed dialog; Accept's held choices are picked in a menu, one per
//! held set, first. An accepted tool opens from its row ("Open tool"), and
//! its screen keeps a row back here.

use super::{notice, os, Act, App, Config, Focus, Menu, Mode, Pending};
use crate::menu::{item, Action, Item};
use crate::next::Advice;
use crate::project::{self, MapView, Offered, State};
use crate::tree::{Row, RowKind, Selection};
use harness_core::text::safe_line;
use std::collections::BTreeMap;
use std::path::Path;

/// What the Project view knows of the project.
#[derive(Debug, Clone)]
pub struct ProjectState {
    /// The Project view is shown (else a tool's screen is, with its row
    /// back here).
    pub shown: bool,
    /// The map as last read: none yet, or why it cannot be read.
    pub map: Result<Option<MapView>, String>,
    /// The accepted tools, by id.
    pub tools: Vec<String>,
    /// The model's advice on the held sets, by set.
    pub advice: BTreeMap<String, project::Advice>,
    /// The chat pane is wanted (on the Project view and the tools'
    /// screens alike).
    pub chat_wanted: bool,
    /// Accept's picker, while it asks.
    pub pick: Option<Pick>,
    /// The model's labels of the map's programs, by id.
    pub labels: BTreeMap<String, project::Label>,
    /// `migration/map/config.toml` was written after the map was made: the
    /// map does not use it yet (Map the project again does).
    pub config_newer: bool,
    /// The tool an Accept just made, until it is opened: the Next step.
    pub just_accepted: Option<String>,
    /// Each accepted tool's progress, read cheaply from its ledger (its plan
    /// and attempt records) when the project is read (fix pass, item 4).
    pub progress: BTreeMap<String, ToolProgress>,
}

/// An accepted tool's progress, as its card and the project's Next say it:
/// its pieces, how many are done, how many wait for the person.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolProgress {
    /// It has a plan.
    pub planned: bool,
    /// Its pieces (blocked ones left out).
    pub pieces: usize,
    /// Of them, done (verified or merged).
    pub done: usize,
    /// Of them, with a green try that waits for Accept.
    pub accept: usize,
    /// Of them, with a try that waits for an answer.
    pub answer: usize,
    /// Of them, the piece holding main: it stays C in this version (a piece
    /// that calls others moves, docs/BEYOND-LEAVES-DESIGN.md §5).
    pub stay_c: usize,
    /// The person's features file is written.
    pub features: bool,
}

impl ToolProgress {
    /// Read from `migration/tools/<id>/`: its plan's statuses and its
    /// attempt records (no hashing: a cheap look, as the card needs).
    pub fn read(root: &Path, id: &str) -> ToolProgress {
        let dir = root.join("migration/tools").join(id);
        let Ok(plan) = harness_core::plan::Plan::load(&dir.join("plan.toml")) else {
            return ToolProgress::default();
        };
        let mut p = ToolProgress {
            planned: true,
            features: dir
                .join(harness_core::features::FEATURES_DIR)
                .join(harness_core::features::FEATURES_FILE)
                .is_file(),
            ..ToolProgress::default()
        };
        for u in plan.units.iter().filter(|u| u.status.as_str() != "blocked") {
            p.pieces += 1;
            if matches!(u.status.as_str(), "verified" | "merged") {
                p.done += 1;
                continue;
            }
            if u.holds_main() {
                p.stay_c += 1;
                continue;
            }
            let attempts = dir.join("units").join(&u.id).join("attempts");
            let Ok(entries) = std::fs::read_dir(&attempts) else {
                continue;
            };
            let records: Vec<harness_core::attempts::AttemptRecord> = entries
                .flatten()
                .take(1000)
                .filter_map(|e| harness_core::attempts::AttemptRecord::load(&e.path()).ok())
                .collect();
            if records.iter().any(|r| r.outcome == "in-progress") {
                p.answer += 1;
            } else if records.iter().any(|r| r.outcome == "green" && !r.promoted) {
                p.accept += 1;
            }
        }
        p
    }

    /// Something waits for the person in it.
    pub fn waiting(&self) -> bool {
        self.accept + self.answer > 0
    }

    /// Its pieces that can move (those that stay C left out).
    pub fn movable(&self) -> usize {
        self.pieces.saturating_sub(self.stay_c)
    }

    /// Every piece that can move is done.
    pub fn moved(&self) -> bool {
        self.planned && self.done >= self.movable()
    }

    /// Its progress in the state word's place on its row: `not scanned
    /// yet`, `2/4 in Rust`, `done`.
    pub fn row_word(&self) -> String {
        if !self.planned {
            "not scanned yet".into()
        } else if self.moved() {
            "done".into()
        } else {
            format!("{}/{} in Rust", self.done, self.movable())
        }
    }

    /// What it still needs, most first, for the project's Next: `None`
    /// when it is as far as this version goes (its features written).
    pub fn to_do(&self) -> Option<(u8, String)> {
        if !self.planned {
            Some((0, "not scanned yet".into()))
        } else if !self.moved() {
            let left = self.movable() - self.done;
            Some((
                1,
                format!("{left} piece{} to move", if left == 1 { "" } else { "s" }),
            ))
        } else if !self.features {
            Some((2, "its features are not written".into()))
        } else {
            None
        }
    }

    /// In words: "3 pieces · 1 done · 1 waits for your Accept".
    pub fn words(&self) -> String {
        if !self.planned {
            return "not scanned and planned yet".into();
        }
        let mut parts = vec![format!(
            "{} piece{}",
            self.pieces,
            if self.pieces == 1 { "" } else { "s" }
        )];
        if self.done > 0 {
            parts.push(format!("{} done", self.done));
        }
        if self.accept > 0 {
            parts.push(format!("{} waits for your Accept", self.accept));
        }
        if self.answer > 0 {
            parts.push(format!("{} waits for your answer", self.answer));
        }
        if self.stay_c > 0 {
            parts.push(format!("{} stay C", self.stay_c));
        }
        parts.join(" · ")
    }
}

/// Accept's picker in progress: the held sets answered so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pick {
    /// The program or library accepted.
    pub id: String,
    /// The definers kept so far (indexes).
    pub kept: Vec<String>,
    /// The sets answered so far.
    pub asked: Vec<String>,
    /// The `--keep` values so far (`d1=d1.2`).
    pub keeps: Vec<String>,
    /// A run name of the person's is typed before the dialog.
    pub run_name: bool,
}

impl ProjectState {
    /// The project at `root` as it stands: its map, its tools, the advice.
    pub fn read(root: &Path) -> ProjectState {
        let map = project::read_map(root);
        let (advice, labels) = match &map {
            Ok(Some(m)) => (project::read_advice(root, m), project::read_labels(root, m)),
            _ => (BTreeMap::new(), BTreeMap::new()),
        };
        let modified = |rel: &str| {
            std::fs::symlink_metadata(root.join(rel))
                .ok()
                .filter(|m| m.file_type().is_file())
                .and_then(|m| m.modified().ok())
        };
        let config_newer = match (
            modified(super::features_edit::CONFIG_FILE),
            modified(project::MAP_FILE),
        ) {
            (Some(config), Some(map)) => config > map,
            _ => false,
        };
        let tools = harness_core::config::mapped_tools(root);
        let progress = tools
            .iter()
            .map(|id| (id.clone(), ToolProgress::read(root, id)))
            .collect();
        ProjectState {
            progress,
            shown: true,
            map,
            tools,
            advice,
            chat_wanted: false,
            pick: None,
            labels,
            config_newer,
            just_accepted: None,
        }
    }

    /// The map, when one was read.
    pub fn map(&self) -> Option<&MapView> {
        self.map.as_ref().ok().and_then(Option::as_ref)
    }
}

fn heading(text: String) -> Row {
    Row {
        depth: 0,
        kind: RowKind::Heading(text),
        expandable: false,
        open: false,
    }
}

/// A group's heading and its counts ("Programs (3) · 1 accepted · 2 need a
/// decision") when the whole of it fits the tree's `width` on one line (0:
/// not drawn yet), else the heading alone: a second line would read as a
/// row.
fn headings(head: String, counts: &[String], width: usize) -> Vec<Row> {
    let full = std::iter::once(head.clone())
        .chain(counts.iter().cloned())
        .collect::<Vec<_>>()
        .join(" · ");
    if width == 0 || full.chars().count() <= width {
        vec![heading(full)]
    } else {
        vec![heading(head)]
    }
}

/// A count in words: "1 needs a decision", "2 need a decision".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn node(depth: usize, sel: Selection) -> Row {
    Row {
        depth,
        kind: RowKind::Node(sel),
        expandable: false,
        open: false,
    }
}

impl App {
    /// The cockpit on the project at `config.target` (no root
    /// `harness.toml`), opened on its Project view.
    pub fn new_project(config: Config) -> App {
        let read = crate::load::Read::empty(&config.target);
        let mut app = App::new(config, read);
        app.project = Some(ProjectState::read(&app.config.target));
        app.selection = Selection::MapProject;
        app.rebuild_rows();
        app.select_first_project_row();
        app
    }

    /// A tool's screen opened on a project (`--tool`, or its only tool):
    /// its first row goes back to the Project view.
    pub fn join_project(&mut self) {
        let mut p = ProjectState::read(&self.config.target);
        p.shown = false;
        self.project = Some(p);
        self.rebuild_rows();
    }

    /// The Project view is shown.
    pub fn in_project_view(&self) -> bool {
        self.project.as_ref().is_some_and(|p| p.shown)
    }

    /// The chat pane on or off (`--chat`): on the Project view too, where
    /// the chat answers the map's open question (docs/CHAT-PANE-DESIGN.md
    /// §3.6) with no tool open.
    pub fn set_chat(&mut self, on: bool) {
        if let Some(p) = self.project.as_mut() {
            p.chat_wanted = on;
        }
        self.chat_on = on;
    }

    /// The map's program or library `id`.
    pub fn project_offered(&self, id: &str) -> Option<&Offered> {
        self.project.as_ref()?.map()?.get(id)
    }

    /// The accepted tools.
    pub fn project_tools(&self) -> &[String] {
        self.project.as_ref().map_or(&[], |p| p.tools.as_slice())
    }

    /// The Project view's map, when one was read.
    pub fn project_map(&self) -> Option<&MapView> {
        self.project.as_ref()?.map()
    }

    /// Read the map and the tools again; the selection stays when its row
    /// does, else the first row.
    pub fn reload_project(&mut self) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let fresh = ProjectState::read(&self.config.target);
        p.map = fresh.map;
        p.tools = fresh.tools;
        p.progress = fresh.progress;
        p.advice = fresh.advice;
        p.labels = fresh.labels;
        p.config_newer = fresh.config_newer;
        p.pick = None;
        // The tool just accepted stays the Next step while it is a tool.
        if p.just_accepted
            .as_ref()
            .is_some_and(|id| !p.tools.contains(id))
        {
            p.just_accepted = None;
        }
        self.rebuild_rows();
        if crate::tree::row_of(&self.rows, &self.selection).is_none() {
            self.select_first_project_row();
        }
    }

    fn select_first_project_row(&mut self) {
        if let Some(sel) = self.rows.iter().find_map(|r| r.selection().cloned()) {
            self.selection = sel;
        }
        self.scroll = 0;
        self.link = None;
    }

    /// The Project view's rows (§9.3 of the cockpit design: one list): one
    /// row "Map the project" before a map (or when the map is unreadable or
    /// offers nothing); after a map the heading with its counts, one row
    /// per program — an accepted program's row is its tool's row — then the
    /// libraries when there are any. A tool the map does not name keeps a
    /// row of its own.
    pub(crate) fn project_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let Some(p) = &self.project else {
            return rows;
        };
        let width = self.layout.tree_width;
        let map = p.map();
        if map.is_none_or(|m| m.offered.is_empty() && m.not_compiled.is_empty()) {
            rows.push(node(0, Selection::MapProject));
        }
        let accepted = |o: &Offered| p.tools.contains(&o.id);
        // Tools the map does not name (or no map is read).
        let others: Vec<&String> = p
            .tools
            .iter()
            .filter(|id| map.is_none_or(|m| m.get(id).is_none()))
            .collect();
        let Some(m) = map else {
            if !others.is_empty() {
                rows.extend(headings(format!("Tools ({})", others.len()), &[], width));
                rows.extend(
                    others
                        .iter()
                        .map(|id| node(0, Selection::Tool((*id).clone()))),
                );
            }
            return rows;
        };
        let programs: Vec<&Offered> = m.offered.iter().filter(|o| o.kind != "library").collect();
        let by = |f: &dyn Fn(&Offered) -> bool| programs.iter().filter(|o| f(o)).count();
        let n_accepted = by(&|o| accepted(o)) + others.len();
        let n_held = by(&|o| !accepted(o) && o.state == State::Held);
        let n_links = by(&|o| !accepted(o) && o.state == State::Linked);
        let n_not_compiled =
            by(&|o| !accepted(o) && o.state == State::DidNotCompile) + m.not_compiled.len();
        let n_not_linked = by(&|o| !accepted(o) && o.state == State::DidNotLink);
        let counts: Vec<String> = [
            (n_accepted, count(n_accepted, "accepted", "accepted")),
            (n_held, count(n_held, "needs a decision", "need a decision")),
            (n_links, count(n_links, "links", "link")),
            (
                n_not_compiled,
                count(n_not_compiled, "did not compile", "did not compile"),
            ),
            (
                n_not_linked,
                count(n_not_linked, "did not link", "did not link"),
            ),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(_, words)| words)
        .collect();
        rows.extend(headings(
            format!(
                "Programs ({})",
                programs.len() + m.not_compiled.len() + others.len()
            ),
            &counts,
            width,
        ));
        for o in programs {
            rows.push(node(0, Selection::Program(o.id.clone())));
        }
        for id in others {
            rows.push(node(0, Selection::Tool(id.clone())));
        }
        for path in &m.not_compiled {
            rows.push(Row {
                depth: 0,
                kind: RowKind::Failed(safe_line(path)),
                expandable: false,
                open: false,
            });
        }
        let libraries: Vec<&Offered> = m.offered.iter().filter(|o| o.kind == "library").collect();
        if !libraries.is_empty() {
            rows.extend(headings(
                format!("Libraries ({})", libraries.len()),
                &[],
                width,
            ));
            for o in libraries {
                rows.push(node(0, Selection::Program(o.id.clone())));
            }
        }
        rows
    }

    /// The project's Next step, from its state: no map → Map; a guess →
    /// state the build or Ask for a proposal; a held choice → pick it in
    /// Accept; a program ready → Accept it; a tool → open it. Its words
    /// are one line (§9.3 of the cockpit design): "<verb phrase> — <key>";
    /// the menu's reason is the verb phrase.
    pub fn project_next(&self) -> Option<Advice> {
        let p = self.project.as_ref()?;
        let say = |text: String, why: &str, action: Option<Action>| {
            Some(Advice {
                text,
                action,
                reason: Some(why.to_string()),
            })
        };
        // A command was refused because the results were made elsewhere:
        // nothing else runs until the folder is adopted.
        if self.not_adopted.is_some() {
            return say(
                "Adopt this folder — Enter; its results were made elsewhere".into(),
                "its results were made elsewhere",
                Some(Action::Adopt),
            );
        }
        // A hand-off's pause: the person's answer is what comes next.
        if let Some((act, words)) = &self.project_waiting {
            return say(
                words.clone(),
                "a model's answer is awaited",
                Some(Action::Project(*act)),
            );
        }
        let m = match &p.map {
            Err(_) => {
                return say(
                    "Map the project again — Enter; the map could not be read".into(),
                    "the map could not be read",
                    Some(Action::Project(project::Act::Map)),
                )
            }
            Ok(None) => {
                return say(
                    "Map the project — Enter".into(),
                    "nothing is mapped yet",
                    Some(Action::Project(project::Act::Map)),
                )
            }
            Ok(Some(m)) => m,
        };
        // Right after an Accept, the new tool first, before any other
        // program's held choice.
        if let Some(id) = &p.just_accepted {
            return say(
                format!("Open tool {} — Enter on its row", safe_line(id)),
                "it was just accepted",
                Some(Action::OpenTool(id.clone())),
            );
        }
        // Work waiting in a tool comes before a new program (fix pass,
        // item 4).
        if let Some((id, pr)) = p.progress.iter().find(|(_, pr)| pr.waiting()) {
            let what = if pr.accept > 0 {
                "Rust waits for your Accept"
            } else {
                "a try waits for your answer"
            };
            return say(
                format!("Open tool {} — Enter on its row; {what}", safe_line(id)),
                what,
                Some(Action::OpenTool(id.clone())),
            );
        }
        // A configuration written since the map: the map does not use it
        // until it is made again.
        if p.config_newer {
            return say(
                "Map the project again — Enter".into(),
                "the build configuration is newer than the map",
                Some(Action::Project(project::Act::Map)),
            );
        }
        let write = self.build_config_label();
        if m.proposed {
            // The project's own configuration is only proposed: the person
            // writes theirs (or adopts the project's on the command line).
            return say(
                format!("{write} — Enter (or harness project map --adopt)"),
                "the configuration came with the project",
                Some(Action::WriteBuildConfig),
            );
        }
        if m.guessed {
            // The chat's item is named only while the chat can be used
            // (the line stays under 80 characters).
            let key = if self.chat_unavailable().is_none() {
                "Enter, or Ask for a proposal in the chat"
            } else {
                "Enter, it is the first item"
            };
            return say(
                format!("{write} — {key}"),
                "the configuration is a guess",
                Some(Action::WriteBuildConfig),
            );
        }
        let open = |o: &&Offered| o.acceptable() && !p.tools.contains(&o.id);
        // The tool with the most to do: not scanned yet, then pieces to
        // move, then features not written (the person, 2026-10-20).
        let most = p
            .tools
            .iter()
            .filter_map(|id| {
                p.progress
                    .get(id)
                    .and_then(ToolProgress::to_do)
                    .map(|(rank, why)| (rank, id, why))
            })
            .min_by_key(|(rank, _, _)| *rank);
        let open_tool = |id: &String, why: &str| {
            say(
                format!("Open tool {} — Enter on its row ({why})", safe_line(id)),
                why,
                Some(Action::OpenTool(id.clone())),
            )
        };
        // A tool with a piece ready to start comes first, then a program
        // that links, then one that needs a decision (the whole-journey
        // check, item 10: the decision came first and hid the work ready in
        // the tool just accepted).
        if let Some((_, id, why)) = most.as_ref().filter(|(rank, _, _)| *rank <= 1) {
            return open_tool(id, why);
        }
        if let Some(o) = m
            .offered
            .iter()
            .filter(open)
            .find(|o| matches!(o.state, State::Linked | State::Compiles))
        {
            return say(
                format!("Accept {} — Enter on its row", safe_line(&o.id)),
                if o.state == State::Linked {
                    "it links"
                } else {
                    "it compiles"
                },
                Some(Action::AcceptProgram {
                    id: o.id.clone(),
                    run_name: false,
                }),
            );
        }
        if let Some(o) = m
            .offered
            .iter()
            .filter(open)
            .find(|o| o.state == State::Held)
        {
            return say(
                format!(
                    "Accept {} — Enter on its row, it asks for the file",
                    safe_line(&o.id)
                ),
                "it needs a decision",
                Some(Action::AcceptProgram {
                    id: o.id.clone(),
                    run_name: false,
                }),
            );
        }
        if let Some(first) = p.tools.first() {
            return match most {
                Some((_, id, why)) => open_tool(id, &why),
                None => say(
                    "Every program is as far as this version goes — write features and \
                     workloads for what matters"
                        .into(),
                    "every tool is done",
                    Some(Action::OpenTool(first.clone())),
                ),
            };
        }
        say(
            format!("{write} — Enter, then Map the project again"),
            "no program links yet",
            Some(Action::WriteBuildConfig),
        )
    }

    /// The build configuration act's words: "Write the build configuration"
    /// while there is no `migration/map/config.toml`, else "Edit the build
    /// configuration"; "Continue my build configuration draft" while one is
    /// kept.
    pub fn build_config_label(&self) -> String {
        let kind = super::features_edit::DraftKind::BuildConfig;
        if self.draft(kind).is_some() {
            return kind.continue_words();
        }
        let exists =
            std::fs::symlink_metadata(self.config.target.join(super::features_edit::CONFIG_FILE))
                .is_ok();
        if exists {
            "Edit the build configuration".into()
        } else {
            "Write the build configuration".into()
        }
    }

    /// A Project view row's Next step (the menu's recommended item), in
    /// the Next step's words.
    pub fn project_row_advice(&self, sel: &Selection) -> Option<Advice> {
        let say = |text: String, why: &str, action: Option<Action>| {
            Some(Advice {
                text,
                action,
                reason: Some(why.to_string()),
            })
        };
        let open = |id: &str| {
            say(
                format!("Open tool {} — Enter on its row", safe_line(id)),
                "it is accepted",
                Some(Action::OpenTool(id.to_string())),
            )
        };
        match sel {
            Selection::ProjectHome => say(
                "the project's programs, Map, Ask and Accept are on the project view — press \
                 Enter to go back to it"
                    .into(),
                "the project's acts are there",
                Some(Action::BackToProject),
            ),
            Selection::Tool(id) => open(id),
            Selection::Program(id) => {
                let m = self.project_map()?;
                let o = m.get(id)?;
                // An accepted program is its tool: open it, whatever the
                // map's configuration says now (fix pass, item 17).
                if self.project_tools().contains(id) {
                    return open(id);
                }
                if m.unstated() {
                    return self.project_next();
                }
                let accept = Some(Action::AcceptProgram {
                    id: id.clone(),
                    run_name: false,
                });
                match o.state {
                    State::Held => say(
                        format!(
                            "Accept {} — Enter on its row, it asks for the file",
                            safe_line(id)
                        ),
                        "it needs a decision",
                        accept,
                    ),
                    State::Linked => say(
                        format!("Accept {} — Enter on its row", safe_line(id)),
                        "it links",
                        accept,
                    ),
                    State::Compiles => say(
                        format!("Accept {} — Enter on its row", safe_line(id)),
                        "it compiles",
                        accept,
                    ),
                    State::DidNotCompile | State::DidNotLink | State::Incomplete => say(
                        format!(
                            "{} — Enter, then Map the project again",
                            self.build_config_label()
                        ),
                        o.state.words(),
                        Some(Action::WriteBuildConfig),
                    ),
                    State::Fuzzer | State::Driver => say(
                        "nothing here — a fuzzer is not accepted as a tool".into(),
                        "",
                        None,
                    ),
                }
            }
            _ => self.project_next(),
        }
    }

    /// The menu on a Project view row (or a tool's row back to it): the
    /// row's own items, then Map and Ask; the recommended one first. A menu
    /// that would hold only the Next step's own item is not shown: the item
    /// is chosen at once (its dialog opens), so one choice is one Enter.
    pub(crate) fn open_project_menu(&mut self) -> super::Command {
        let mut items = self.project_menu_items();
        let advice = self.project_row_advice(&self.selection);
        let reason = crate::menu::recommend(&mut items, advice.as_ref());
        if let ([only], Some(_)) = (items.as_slice(), &reason) {
            let only = only.clone();
            return self.choose(&only);
        }
        items.push(crate::menu::explain_item());
        self.mode = Mode::Menu(Menu {
            items,
            focus: 0,
            footer: None,
            reason,
            title: None,
            note: Vec::new(),
        });
        super::Command::None
    }

    /// The items of the selected Project view row.
    pub fn project_menu_items(&self) -> Vec<Item> {
        let sel = self.selection.clone();
        let mut items: Vec<Item> = Vec::new();
        if sel == Selection::ProjectHome {
            let mut it = item("Back to the project view", Action::BackToProject, None);
            it.greyed = self.switch_refusal();
            items.push(it);
            return items;
        }
        let busy = self.busy();
        let map = self.project_map();
        let switch = self.switch_refusal();
        match &sel {
            Selection::Tool(id) => {
                let mut it = item(
                    format!("Open tool {id}"),
                    Action::OpenTool(id.clone()),
                    None,
                );
                it.greyed = switch.clone();
                items.push(it);
            }
            Selection::Program(id) => {
                if self.project_tools().contains(id) {
                    let mut it = item(
                        format!("Open tool {id}"),
                        Action::OpenTool(id.clone()),
                        None,
                    );
                    it.greyed = switch.clone();
                    items.push(it);
                }
                if let Some(o) = map.and_then(|m| m.get(id)) {
                    let why = if !o.acceptable() {
                        Some(
                            "a fuzzer is not accepted as a tool: it is linked only with the \
                             project's driver"
                                .to_string(),
                        )
                    } else if map.is_some_and(MapView::unstated) {
                        Some(if self.chat_unavailable().is_none() {
                            project::ACCEPT_NEEDS_STATED.to_string()
                        } else {
                            // Never the chat's item while it cannot be used
                            // (the whole-journey check, item 13).
                            project::ACCEPT_NEEDS_STATED
                                .replace(" (or Ask for a proposal — ask in chat)", "")
                        })
                    } else {
                        busy.clone()
                    };
                    // An accepted one leads with Open tool; accepting it
                    // again is the second thing (fix pass, item 17).
                    let accepted = self.project_tools().contains(id);
                    let kinds: &[bool] = if accepted { &[false] } else { &[false, true] };
                    for &run_name in kinds {
                        let label = if accepted {
                            "Accept again…".to_string()
                        } else if run_name {
                            "Accept it under a run name of your own…".to_string()
                        } else {
                            format!("Accept this {}", o.noun())
                        };
                        let mut it = item(
                            label,
                            Action::AcceptProgram {
                                id: id.clone(),
                                run_name,
                            },
                            None,
                        );
                        it.greyed = why.clone();
                        items.push(it);
                    }
                }
            }
            _ => {}
        }
        // The build configuration, once a map exists (it names the example
        // to start from): the person's editor on migration/map/config.toml.
        if map.is_some() {
            let kind = super::features_edit::DraftKind::BuildConfig;
            let label = self.build_config_label();
            let mut it = if self.draft(kind).is_some() {
                item(label, Action::WriteBuildConfig, None)
            } else {
                let name = super::features_edit::editor_name(&self.features_editor());
                item(
                    format!("{label} (in {name})"),
                    Action::WriteBuildConfig,
                    None,
                )
            };
            if self.running {
                it.greyed = Some("a command is running (one at a time)".into());
            }
            items.push(it);
            if self.draft(kind).is_some() {
                items.push(item(
                    "Discard my build configuration draft",
                    Action::DiscardBuildConfigDraft,
                    None,
                ));
            }
        }
        if self.not_adopted.is_some() {
            let mut it = item("Adopt this folder…", Action::Adopt, None);
            if self.running {
                it.greyed = Some("a command is running (one at a time)".into());
            }
            items.push(it);
        }
        let mut it = item(
            if map.is_some() {
                "Map the project again"
            } else {
                "Map the project"
            },
            Action::Project(project::Act::Map),
            None,
        );
        it.greyed = busy.clone();
        items.push(it);
        // Model work goes through the chat (docs/CHAT-PANE-DESIGN.md §3.6):
        // once a map exists, its open question is asked in chat — the
        // question typed into the chat, the chat answering through the
        // harness. (`harness project ask` stays the command line's.)
        let chat_off = self.chat_unavailable();
        self.check_chat_item(&mut items);
        if map.is_some() {
            let build = project::asks_for_build(map);
            let mut it = item(
                if build {
                    "Ask for a proposal — ask in chat"
                } else {
                    "Ask for advice — ask in chat"
                },
                Action::AskInChat { build },
                None,
            );
            it.model = true;
            it.greyed = chat_off.clone().or_else(|| {
                (!self.chat.input.is_empty())
                    .then(|| "the chat has a draft — send or clear it first".to_string())
            });
            items.push(it);
        }
        if self.chat_on {
            let mut it = item("Ask in chat…", Action::AskChat, None);
            it.model = true;
            it.greyed = chat_off;
            items.push(it);
        }
        // Model-backed items last, under the separator.
        items.sort_by_key(|i| i.model);
        items
    }

    /// Why the cockpit cannot move between the Project view and a tool now,
    /// if it cannot: what the screen holds would be lost. A conversation in
    /// the chat is not a refusal: the move asks, and a new chat starts
    /// there (`Purpose::Switch`).
    pub fn switch_refusal(&self) -> Option<String> {
        if self.running {
            return Some("a command is running (one at a time)".into());
        }
        if !self.kept_edits.is_empty() {
            return Some("a hand edit is kept — record or discard it first (E)".into());
        }
        if self.features_draft.is_some()
            || self.workloads_draft.is_some()
            || self.config_draft.is_some()
        {
            return Some("a draft is kept — save or discard it first".into());
        }
        if !self.awaiting.is_empty() {
            return Some("a hand-off waits for its answer here — answer it first".into());
        }
        None
    }

    /// Accept: its held choices picked first, one menu per held set.
    pub(crate) fn start_accept(&mut self, id: &str, run_name: bool) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        p.pick = Some(Pick {
            id: id.to_string(),
            run_name,
            ..Pick::default()
        });
        self.pick_next();
    }

    /// The next held set's menu, or — every set answered — the run name or
    /// the dialog.
    fn pick_next(&mut self) {
        let Some(pick) = self.project.as_ref().and_then(|p| p.pick.clone()) else {
            return;
        };
        let Some(o) = self.project_offered(&pick.id).cloned() else {
            self.notice = notice(format!("{} is no longer in the map", pick.id));
            return;
        };
        if let Some(h) = o.next_held(&pick.kept, &pick.asked) {
            // The files numbered, none recommended; the one the model
            // advises marked, and its why above them. The set and definer
            // ids stay behind the items (the command's --keep flags).
            let advice = self.project.as_ref().and_then(|p| p.advice.get(&h.set));
            let mut note = vec![project::keep_question(h.definers.len())];
            if !h.under.is_empty() {
                let path_of = |index: &str| {
                    o.held
                        .iter()
                        .flat_map(|x| x.definers.iter())
                        .find(|(i, _)| i == index)
                        .map_or_else(|| index.to_string(), |(_, path)| safe_line(path))
                };
                let paths: Vec<String> = h.under.iter().map(|u| path_of(u)).collect();
                note.push(format!("Asked only if you keep {}.", paths.join(" or ")));
            }
            if let Some(a) = advice {
                note.push(a.why_line());
            }
            let items = h
                .definers
                .iter()
                .enumerate()
                .map(|(n, (index, path))| {
                    let advised = advice.is_some_and(|a| &a.keep == index);
                    item(
                        format!(
                            "{}  {}{}",
                            n + 1,
                            safe_line(path),
                            if advised {
                                format!("  {}", project::ADVISED)
                            } else {
                                String::new()
                            }
                        ),
                        Action::Keep(index.clone()),
                        None,
                    )
                })
                .collect();
            // The highlight starts on the advised file: nothing is picked
            // until Enter, so a second Enter never takes the other one.
            let focus = advice
                .and_then(|a| h.definers.iter().position(|(i, _)| *i == a.keep))
                .unwrap_or(0);
            self.mode = Mode::Menu(Menu {
                items,
                focus,
                footer: None,
                reason: None,
                title: Some(format!("Accept {}", o.id)),
                note,
            });
            return;
        }
        if pick.run_name {
            self.mode = Mode::RunName {
                input: String::new(),
                id: pick.id,
                keeps: pick.keeps,
            };
        } else {
            self.ask_project(project::Act::Accept, &pick.id, pick.keeps, None);
        }
    }

    /// Accept's picker: `index` kept for the set it belongs to.
    pub(crate) fn keep_chosen(&mut self, index: &str) {
        let Some(pick) = self.project.as_mut().and_then(|p| p.pick.as_mut()) else {
            return;
        };
        let Some((set, _)) = index.split_once('.') else {
            return;
        };
        pick.kept.push(index.to_string());
        pick.asked.push(set.to_string());
        pick.keeps.push(format!("{set}={index}"));
        self.pick_next();
    }

    /// Open the dialog of a project act (nothing runs yet).
    pub(crate) fn ask_project(
        &mut self,
        act: project::Act,
        id: &str,
        keeps: Vec<String>,
        run_name: Option<String>,
    ) {
        let Some(harness) = self.config.harness.clone() else {
            self.notice =
                notice("no `harness` binary found (PATH, or --harness <path>): read-only");
            return;
        };
        let words = project::argv(
            act,
            &self.config.target,
            self.project_map(),
            id,
            &keeps,
            run_name.as_deref(),
        );
        let mut argv = vec![harness.into_os_string()];
        argv.extend(words.into_iter().map(os));
        let (kind, label) = match act {
            project::Act::Map => (Act::ProjectMap, "Map the project".to_string()),
            project::Act::Ask => (Act::ProjectAsk, "Ask a model".to_string()),
            project::Act::Accept => (Act::ProjectAccept, format!("Accept {id}")),
        };
        self.ask(Pending {
            act: kind,
            argv,
            label,
            unit: (!id.is_empty()).then(|| id.to_string()),
            attempt: None,
            cleanup: None,
            expect_attempt: None,
            note: None,
            shown_digest: None,
            chat: None,
            stdin: None,
        });
    }

    /// A project act's dialog: its title, then what it does, runs, takes
    /// and writes — the command line's words (docs/PROJECT-MAP-DESIGN.md
    /// §3.7).
    pub(crate) fn project_dialog_words(&self, p: &Pending) -> (String, Vec<String>) {
        let words: Vec<String> = p
            .argv
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let map = self.project_map();
        let act = p.act.project().unwrap_or(project::Act::Map);
        let title = match act {
            project::Act::Map if map.is_some() => "Map the project again?".to_string(),
            project::Act::Map => "Map the project?".to_string(),
            project::Act::Ask if words.iter().any(|w| w == "--build") => {
                "Ask a model for a proposal?".to_string()
            }
            project::Act::Ask => "Ask a model for advice?".to_string(),
            project::Act::Accept => format!(
                "Accept {}?",
                safe_line(p.unit.as_deref().unwrap_or("the program"))
            ),
        };
        let mut body = project::dialog(act, map, &words);
        if act == project::Act::Accept {
            // Each decision in words, before the command's --keep flags.
            let picks = self.keep_words(p.unit.as_deref().unwrap_or_default(), &words);
            body.splice(1..1, picks);
        }
        (title, body)
    }

    /// Accept's decisions in words, one line each, from the command's
    /// `--keep <set>=<definer>` flags: "Keeps src/lib/decode.c for the
    /// functions both files define (the model advised this file)".
    fn keep_words(&self, id: &str, words: &[String]) -> Vec<String> {
        let Some(o) = self.project_offered(id) else {
            return Vec::new();
        };
        let advice = self.project.as_ref().map(|p| &p.advice);
        let mut out = Vec::new();
        for pair in words.windows(2).filter(|w| w[0] == "--keep") {
            let Some((set, keep)) = pair[1].split_once('=') else {
                continue;
            };
            let Some(h) = o.held.iter().find(|h| h.set == set) else {
                continue;
            };
            let path_of = |index: &str| {
                h.definers
                    .iter()
                    .find(|(i, _)| i == index)
                    .map(|(_, path)| safe_line(path))
            };
            let Some(path) = path_of(keep) else {
                continue;
            };
            let files = if h.definers.len() == 2 {
                "both files".to_string()
            } else {
                format!("these {} files", h.definers.len())
            };
            let mut line = format!("Keeps {path} for the functions {files} define");
            match advice.and_then(|a| a.get(set)) {
                Some(a) if a.keep == keep => line.push_str(" (the model advised this file)"),
                Some(a) => {
                    if let Some(other) = path_of(&a.keep) {
                        line.push_str(&format!(" (the model advised {other} instead)"));
                    }
                }
                None => {}
            }
            out.push(format!("{line}."));
        }
        out
    }

    /// Open the accepted tool `id` over what `read` found: the tool's
    /// screen, with its row back to the Project view; what the activity
    /// showed is kept.
    pub fn open_tool(&mut self, id: String, read: crate::load::Read) {
        let exe = std::env::current_exe().ok();
        let mut config = self.config.clone();
        config.tool = Some(id.clone());
        let mut next = App::new_from(config, read, exe.as_deref());
        let mut project = self.project.take();
        let chat_wanted = project.as_ref().is_some_and(|p| p.chat_wanted);
        if let Some(p) = project.as_mut() {
            p.shown = false;
            p.pick = None;
            // A tool opened: the one just accepted is no longer the Next
            // step, whichever it was.
            p.just_accepted = None;
        }
        next.project = project;
        next.not_adopted = self.not_adopted.take();
        std::mem::swap(&mut next.chat, &mut self.chat);
        next.chat.tool = Some(id.clone());
        next.chat_on = chat_wanted;
        next.run = self.run.take();
        // A screen never opens with another screen's last command (the
        // details, c, keep it).
        next.last = None;
        self.last = None;
        next.mouse = self.mouse;
        next.now = self.now;
        next.layout = self.layout.clone();
        // The colours are the session's, not the screen's (the person's
        // --theme dark went back to the terminal theme on Open tool) — and
        // so are why that theme is on, the pane width cockpit.toml fixed,
        // the chat's column, the first move made, and the editor leftovers
        // and old drafts named on quit (the whole-journey check, item 9).
        next.palette = self.palette;
        next.theme_chosen = self.theme_chosen;
        next.files_width = self.files_width;
        next.chat_column = self.chat_column;
        next.moved = self.moved;
        next.leftovers = std::mem::take(&mut self.leftovers);
        next.kept_drafts = std::mem::take(&mut self.kept_drafts);
        next.rebuild_rows();
        next.refresh_view(true);
        next.notice = notice(format!(
            "opened tool {id} — the first row goes back to the project view"
        ));
        *self = next;
    }

    /// Back from a tool's screen to the Project view, the map read again.
    pub fn show_project(&mut self) {
        let chat_on = self.chat_on;
        let Some(p) = self.project.as_mut() else {
            return;
        };
        p.shown = true;
        p.chat_wanted = chat_on;
        // The tool's last command is not the project's, nor its plan's
        // summary.
        self.last = None;
        self.plan_notice = None;
        self.last_waiting = false;
        // The next chat starts on the project, no tool open (the switch
        // waits while the chat holds a conversation).
        self.chat.tool = None;
        let tool = self.config.tool.take();
        self.focus = Focus::Files;
        self.mode = Mode::Normal;
        self.back.clear();
        self.pairs.clear();
        self.source = None;
        self.reload_project();
        // The tool just left is selected: its program's row, or its own.
        match tool
            .into_iter()
            .flat_map(|id| [Selection::Program(id.clone()), Selection::Tool(id)])
            .find(|sel| crate::tree::row_of(&self.rows, sel).is_some())
        {
            Some(sel) => self.selection = sel,
            None => self.select_first_project_row(),
        }
        self.scroll = 0;
        self.link = None;
    }
}
