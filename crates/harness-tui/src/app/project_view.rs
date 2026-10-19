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
        ProjectState {
            shown: true,
            map,
            tools: harness_core::config::mapped_tools(root),
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
/// decision"), wrapped at the counts to the tree's `width` (0: one row);
/// the rows after the first indented.
fn headings(head: String, counts: &[String], width: usize) -> Vec<Row> {
    let mut lines = vec![head];
    for c in counts {
        let last = lines.last_mut().expect("one line at least");
        let joined = format!("{last} · {c}");
        if width == 0 || joined.chars().count() <= width {
            *last = joined;
        } else {
            lines.push(format!("  {c}"));
        }
    }
    lines.into_iter().map(heading).collect()
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
                        .map(|id| node(1, Selection::Tool((*id).clone()))),
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
            rows.push(node(1, Selection::Program(o.id.clone())));
        }
        for id in others {
            rows.push(node(1, Selection::Tool(id.clone())));
        }
        for path in &m.not_compiled {
            rows.push(Row {
                depth: 1,
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
                rows.push(node(1, Selection::Program(o.id.clone())));
            }
        }
        rows
    }

    /// The project's Next step, from its state: no map → Map; a guess →
    /// state the build or Ask for a proposal; a held choice → pick it in
    /// Accept; a program ready → Accept it; a tool → open it.
    pub fn project_next(&self) -> Option<Advice> {
        let p = self.project.as_ref()?;
        let say = |text: String, action: Option<Action>| Some(Advice { text, action });
        // A command was refused because the results were made elsewhere:
        // nothing else runs until the folder is adopted.
        if self.not_adopted.is_some() {
            return say(
                "this folder's results were made elsewhere and are not trusted here yet — press \
                 Enter and choose Adopt this folder"
                    .into(),
                Some(Action::Adopt),
            );
        }
        // A hand-off's pause: the person's answer is what comes next.
        if let Some((act, words)) = &self.project_waiting {
            return say(words.clone(), Some(Action::Project(*act)));
        }
        let m = match &p.map {
            Err(_) => {
                return say(
                    "the map could not be read — press Enter to map the project again".into(),
                    Some(Action::Project(project::Act::Map)),
                )
            }
            Ok(None) => {
                return say(
                    "No map yet — press Enter to map the project".into(),
                    Some(Action::Project(project::Act::Map)),
                )
            }
            Ok(Some(m)) => m,
        };
        // Right after an Accept, the new tool first, before any other
        // program's held choice.
        if let Some(id) = &p.just_accepted {
            return say(
                format!(
                    "{} is accepted — open it and scan it: press Enter on its row and choose \
                     Open tool {}",
                    safe_line(id),
                    safe_line(id)
                ),
                Some(Action::OpenTool(id.clone())),
            );
        }
        // A configuration written since the map: the map does not use it
        // until it is made again.
        if p.config_newer {
            return say(
                "migration/map/config.toml is written but this map was made before it — press \
                 Enter and choose Map the project again to use it"
                    .into(),
                Some(Action::Project(project::Act::Map)),
            );
        }
        let write = self.build_config_label();
        if m.proposed {
            return say(
                format!(
                    "the configuration came with the project and is only proposed — state it \
                     with `harness project map --adopt`, or press Enter and choose {write}, then \
                     Map the project again"
                ),
                Some(Action::Project(project::Act::Map)),
            );
        }
        if m.guessed {
            // The chat's item is named only while the chat can be used.
            let or_ask = if self.chat_unavailable().is_none() {
                " (or Ask for a proposal — ask in chat)"
            } else {
                ""
            };
            return say(
                format!(
                    "the configuration is a guess — press Enter and choose {write}{or_ask}, \
                     then Map the project again"
                ),
                Some(Action::WriteBuildConfig),
            );
        }
        let open = |o: &&Offered| o.acceptable() && !p.tools.contains(&o.id);
        if let Some(o) = m
            .offered
            .iter()
            .filter(open)
            .find(|o| o.state == State::Held)
        {
            return say(
                format!(
                    "{} holds a choice — press Enter on it and choose Accept this program to \
                     pick which file to keep",
                    safe_line(&o.id)
                ),
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
            .find(|o| matches!(o.state, State::Linked | State::Compiles))
        {
            return say(
                format!(
                    "{} {} and is ready — press Enter on it and choose Accept this {}",
                    safe_line(&o.id),
                    if o.state == State::Linked {
                        "links"
                    } else {
                        "compiles"
                    },
                    o.noun()
                ),
                Some(Action::AcceptProgram {
                    id: o.id.clone(),
                    run_name: false,
                }),
            );
        }
        if let Some(id) = p.tools.first() {
            return say(
                format!(
                    "{} is accepted — press Enter on its row and choose Open tool {} to scan, \
                     plan and migrate it",
                    safe_line(id),
                    safe_line(id)
                ),
                Some(Action::OpenTool(id.clone())),
            );
        }
        say(
            format!(
                "no program links yet — press Enter and choose {write} to state the build's \
                 flags, then Map the project again"
            ),
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

    /// A Project view row's Next step (the menu's recommended item).
    pub fn project_row_advice(&self, sel: &Selection) -> Option<Advice> {
        let say = |text: String, action: Option<Action>| Some(Advice { text, action });
        match sel {
            Selection::ProjectHome => say(
                "the project's programs, Map, Ask and Accept are on the project view — press \
                 Enter to go back to it"
                    .into(),
                Some(Action::BackToProject),
            ),
            // The tool just accepted: the project's own Next step names it.
            Selection::Tool(id)
                if self
                    .project
                    .as_ref()
                    .is_some_and(|p| p.just_accepted.as_ref() == Some(id)) =>
            {
                self.project_next()
            }
            Selection::Tool(id) => say(
                format!("open tool {} to scan, plan and migrate it", safe_line(id)),
                Some(Action::OpenTool(id.clone())),
            ),
            Selection::Program(id) => {
                let m = self.project_map()?;
                let o = m.get(id)?;
                if m.unstated() {
                    return self.project_next();
                }
                if self.project_tools().contains(id) {
                    return say(
                        format!(
                            "{} is accepted — choose Open tool {}",
                            safe_line(id),
                            safe_line(id)
                        ),
                        Some(Action::OpenTool(id.clone())),
                    );
                }
                let accept = Some(Action::AcceptProgram {
                    id: id.clone(),
                    run_name: false,
                });
                match o.state {
                    State::Held => say(
                        format!(
                            "{} holds a choice — choose Accept this program to pick which file \
                             to keep",
                            safe_line(id)
                        ),
                        accept,
                    ),
                    State::Linked => say(
                        format!(
                            "{} links and is ready — choose Accept this program",
                            safe_line(id)
                        ),
                        accept,
                    ),
                    State::Compiles => say(
                        format!(
                            "{} compiles and is ready — choose Accept this library",
                            safe_line(id)
                        ),
                        accept,
                    ),
                    State::DidNotCompile | State::DidNotLink | State::Incomplete => say(
                        format!(
                            "{} {} — choose {} to state the build's flags, then Map the \
                             project again",
                            safe_line(id),
                            o.state.words(),
                            self.build_config_label()
                        ),
                        Some(Action::WriteBuildConfig),
                    ),
                    State::Fuzzer | State::Driver => say(
                        "a fuzzer is not accepted as a tool: it is linked only with the \
                         project's driver"
                            .into(),
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
                        Some(project::ACCEPT_NEEDS_STATED.to_string())
                    } else {
                        busy.clone()
                    };
                    for run_name in [false, true] {
                        let label = if run_name {
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
            // The definers listed, none suggested; the model's advice, when
            // there is some, beside them.
            let mut note = vec![project::keep_question(&o.id, &h.set)];
            if let Some(a) = self.project.as_ref().and_then(|p| p.advice.get(&h.set)) {
                note.push(a.words());
            }
            let items = h
                .definers
                .iter()
                .map(|(index, path)| {
                    item(
                        format!("{} {}", safe_line(index), safe_line(path)),
                        Action::Keep(index.clone()),
                        None,
                    )
                })
                .collect();
            self.mode = Mode::Menu(Menu {
                items,
                focus: 0,
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
        (title, project::dialog(act, map, &words))
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
            // Opened: no longer the Next step.
            if p.just_accepted.as_ref() == Some(&id) {
                p.just_accepted = None;
            }
        }
        next.project = project;
        next.not_adopted = self.not_adopted.take();
        std::mem::swap(&mut next.chat, &mut self.chat);
        next.chat.tool = Some(id.clone());
        next.chat_on = chat_wanted;
        next.run = self.run.take();
        next.last = self.last.take();
        next.mouse = self.mouse;
        next.now = self.now;
        next.layout = self.layout.clone();
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
