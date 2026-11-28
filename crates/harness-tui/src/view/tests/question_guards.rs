//! The guards while a question is open (docs/NEXT-SESSION.md, "Guards while
//! a question is open", task 1; the person, 2026-11-27: "we need to put
//! guards in place … to stop a user from breaking things"): the acts that
//! change what a question is posed against wait while any question is open;
//! the acts on the piece the chat is answering wait while it answers; quit,
//! a new chat and leaving the chat's screen say what is lost; and what is
//! allowed on purpose stays allowed.

use super::*;
use crate::app::asks::HandOff;
use crate::app::{ChatPause, Pending};
use crate::menu::{Action, Item, CHAT_ANSWERS_PIECE};
use crate::testutil::TmpDir;
use std::time::Duration;

const MODEL: &str = "claude-opus-5-5";
const TRY: &str = "a-0000000000c1";

const NOBODY: &str = "u-cache has a question open — this would leave its answer without a \
                      home; answer it first (open the piece)";
const ANSWERING: &str = "u-cache has a question open (the chat is answering it) — this would \
                         leave its answer without a home; wait for it, or stop the chat with \
                         Esc in the chat pane";

/// Zopfli with u-cache's try stopped at a hand-off nobody answers (the
/// chat is on, live, but between turns).
fn open_app(tag: &str, tmp: &TmpDir) -> App {
    let mut app = app_of("targets/zopfli", tag);
    waiting_try(&app, "u-cache", TRY, false);
    assert!(app.reload(true));
    app.chat_on = true;
    app.chat_column = true;
    app.chat.test_live(&tmp.0, MODEL);
    app.chat.turn = false;
    app
}

/// The chat answers u-cache's question now (its hand-off is the live
/// chat's, and the chat is in its turn).
fn answer(app: &mut App) {
    let gen = app.chat.gen;
    app.asks.table.insert(
        TRY.into(),
        HandOff {
            key: "0badf00d".into(),
            gen,
            act: "Migrate u-cache".into(),
            turn: Some(1),
            unit: "u-cache".into(),
        },
    );
    app.chat.turn = true;
    app.sync_chat_answering();
    assert!(app.chat_answering("u-cache"));
}

fn greyed(items: &[Item], label: &str) -> Option<String> {
    items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("{label} is not in the menu: {items:?}"))
        .greyed
        .clone()
}

/// The three acts' items on the project's menu.
const LIST: [(Act, &str); 3] = [
    (Act::Scan, "Scan the project"),
    (Act::Plan, "Refresh the plan"),
    (Act::Detect, "Find risky patterns in the C"),
];

/// A Pending for `act` built past every gate, as a dialog opened before
/// the question was posed would hold it.
fn unguarded(app: &App, act: Act, unit: Option<&str>) -> Pending {
    let mut p = app
        .act_argv_unchecked(Act::Scan, None, None, None)
        .expect("a scan argv");
    p.act = act;
    p.unit = unit.map(str::to_string);
    p.label = act.label().into();
    // A Re-check's dialog needs the code it shows; its confirm compares.
    p.shown_digest = Some("shown".into());
    p
}

/// Confirm `p` (its dialog armed, Run pressed): what came of it, and the
/// notice row.
fn confirm(app: &mut App, p: Pending) -> (Command, String) {
    app.ask(p);
    assert!(matches!(&app.mode, Mode::Dialog(_)), "{:?}", app.mode);
    arm(app);
    let got = key(app, 'y');
    let said = app
        .notice
        .as_ref()
        .map(|n| n.text.clone())
        .unwrap_or_default();
    (got, said)
}

/// Scan, Refresh the plan and Find risky patterns wait while a question
/// is open, with the piece named and the way out by case — greyed in the
/// menu, refused by the argv builder and again at confirm — and come back
/// once no question is open.
#[test]
fn acts_that_change_the_question_wait_while_one_is_open() {
    let tmp = TmpDir::new("guards-list");
    let mut app = open_app("guards-list", &tmp);
    app.select(Selection::Project);
    for (case, want) in [("nobody", NOBODY), ("chat", ANSWERING)] {
        if case == "chat" {
            answer(&mut app);
        }
        assert_eq!(app.question_open_refusal().as_deref(), Some(want), "{case}");
        let items = app.menu_items();
        for (act, label) in LIST {
            assert_eq!(
                greyed(&items, label).as_deref(),
                Some(want),
                "{case}: {label}"
            );
            assert_eq!(
                app.act_argv(act, None, None, None),
                Err(want.to_string()),
                "{case}: {label}"
            );
            // A dialog opened before the question was posed: its Run is
            // refused at confirm, saying why.
            let p = unguarded(&app, act, None);
            let (got, said) = confirm(&mut app, p);
            assert_eq!(got, Command::None, "{case}: {label}");
            assert!(said.contains(want), "{case}: {label}: {said}");
            assert_eq!(app.mode, Mode::Normal);
        }
    }
    // A second piece with a question open: the first in plan order, and
    // how many more.
    waiting_try(&app, "u-hash", "a-0000000000c2", false);
    assert!(app.reload(true));
    assert_eq!(
        app.question_open_refusal().as_deref(),
        Some(
            "u-cache has a question open (the chat is answering it), and 1 more — this would \
             leave their answers without a home; wait for it, or stop the chat with Esc in the \
             chat pane"
        )
    );
    // No question open any more: the three are available again.
    for unit in ["u-cache", "u-hash"] {
        let dir = harness_core::ledger::Ledger::new(&app.config.target)
            .unit_dir(unit)
            .join("attempts");
        std::fs::remove_dir_all(dir).unwrap();
    }
    app.asks.table.clear();
    assert!(app.reload(true));
    assert_eq!(app.question_open_refusal(), None);
    let items = app.menu_items();
    for (act, label) in LIST {
        assert!(app.act_argv(act, None, None, None).is_ok(), "{label}");
        let why = greyed(&items, label).unwrap_or_default();
        assert!(!why.contains("question open"), "{label}: {why}");
    }
    let p = unguarded(&app, Act::Scan, None);
    let (got, _) = confirm(&mut app, p);
    assert!(matches!(got, Command::Spawn(_)), "{got:?}");
}

/// The actions that start a command (or an editor) on one piece.
fn starting(action: &Action) -> bool {
    matches!(
        action,
        Action::Act(
            Act::Verify
                | Act::Accept
                | Act::Modify
                | Act::HandEdit
                | Act::Retry
                | Act::Resume
                | Act::Migrate
                | Act::GenDriver
                | Act::Measure
                | Act::MeasureMore
        ) | Action::ChooseAttempt(_)
            | Action::HandEdit
            | Action::Modify
            | Action::ContinueKept
            | Action::Migrate
            | Action::GenDriver
    )
}

/// While the chat answers u-cache's question, every act on u-cache waits
/// with the chat's words — on its menus, by the argv builder, at confirm,
/// for the hand edit — and reading stays free; the same acts on another
/// piece do not wait for it.
#[test]
fn acts_on_the_piece_the_chat_answers_wait() {
    let tmp = TmpDir::new("guards-piece");
    let mut app = open_app("guards-piece", &tmp);
    answer(&mut app);
    // Its rows' menus: what would start a command is greyed with the
    // chat's words; nothing that reads is.
    for sel in [
        Selection::Unit("u-cache".into()),
        Selection::Attempt("u-cache".into(), TRY.into()),
    ] {
        app.select(sel.clone());
        let items = app.menu_items();
        let mut seen = 0;
        for it in &items {
            if starting(&it.action) {
                seen += 1;
                assert_eq!(
                    it.greyed.as_deref(),
                    Some(CHAT_ANSWERS_PIECE),
                    "{sel:?}: {it:?}"
                );
            } else {
                assert_ne!(
                    it.greyed.as_deref(),
                    Some(CHAT_ANSWERS_PIECE),
                    "{sel:?}: {it:?}"
                );
            }
        }
        assert!(seen > 0, "{sel:?}: {items:?}");
        assert!(
            items
                .iter()
                .any(|i| i.action == Action::Reread && i.greyed.is_none()),
            "{sel:?}"
        );
    }
    // Every act on a piece, by the argv builder: refused on u-cache, never
    // for that reason on u001-katajainen.
    for act in [
        Act::Verify,
        Act::Accept,
        Act::Modify,
        Act::HandEdit,
        Act::Retry,
        Act::Resume,
        Act::Migrate,
        Act::GenDriver,
        Act::Measure,
        Act::MeasureMore,
    ] {
        assert_eq!(
            app.question_guard(act, Some("u-cache"), false).as_deref(),
            Some(CHAT_ANSWERS_PIECE),
            "{act:?}"
        );
        assert_eq!(
            app.act_argv(act, Some("u-cache"), Some(TRY), Some("a note")),
            Err(CHAT_ANSWERS_PIECE.to_string()),
            "{act:?}"
        );
        assert_eq!(
            app.question_guard(act, Some("u001-katajainen"), false),
            None,
            "{act:?}"
        );
        // The chat's own request is the answerer's: not refused here.
        assert_eq!(
            app.question_guard(act, Some("u-cache"), true),
            None,
            "{act:?}"
        );
    }
    // The chat's Continue is the answer itself.
    assert_eq!(
        app.question_guard(Act::Continue, Some("u-cache"), false),
        None
    );
    // A dialog opened before: refused at confirm.
    let p = unguarded(&app, Act::Verify, Some("u-cache"));
    let (got, said) = confirm(&mut app, p);
    assert_eq!(got, Command::None);
    assert!(said.contains(CHAT_ANSWERS_PIECE), "{said}");
    // The hand edit refuses before any editor opens.
    app.select(Selection::Attempt("u-cache".into(), TRY.into()));
    assert_eq!(app.hand_edit_target(), Err(CHAT_ANSWERS_PIECE.to_string()));
    // Another piece's menu carries none of it.
    app.select(Selection::Unit("u001-katajainen".into()));
    assert!(app
        .menu_items()
        .iter()
        .all(|i| i.greyed.as_deref() != Some(CHAT_ANSWERS_PIECE)));
}

/// A piece waiting for the person (nobody answering) keeps its menu: the
/// same items as while the chat answers it, none greyed with the chat's
/// words, and its way to answer (Migrate — ask in chat, the try's Retry and
/// the Re-check) as they were.
#[test]
fn a_waiting_piece_keeps_its_menu() {
    let tmp = TmpDir::new("guards-waiting");
    let mut app = open_app("guards-waiting", &tmp);
    let sels = [
        Selection::Unit("u-cache".into()),
        Selection::Attempt("u-cache".into(), TRY.into()),
    ];
    let menus = |app: &mut App| {
        sels.iter()
            .map(|sel| {
                app.select(sel.clone());
                app.menu_items()
            })
            .collect::<Vec<_>>()
    };
    let waiting = menus(&mut app);
    for (sel, items) in sels.iter().zip(&waiting) {
        assert!(
            items
                .iter()
                .all(|i| i.greyed.as_deref() != Some(CHAT_ANSWERS_PIECE)),
            "{sel:?}: {items:?}"
        );
        // Not refused by the guard on the piece: only the acts that change
        // the question wait.
        for it in items {
            if let Action::Act(act) = it.action {
                assert_eq!(
                    app.question_guard(act, Some("u-cache"), false),
                    None,
                    "{it:?}"
                );
            }
        }
    }
    answer(&mut app);
    let answering = menus(&mut app);
    let names = |items: &[Item]| items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
    for ((sel, w), a) in sels.iter().zip(&waiting).zip(&answering) {
        assert_eq!(names(w), names(a), "{sel:?}");
    }
}

/// Allowed on purpose while a question is open: Re-check and Accept of
/// other pieces, reading, and editing the features file.
#[test]
fn other_pieces_reading_and_the_features_file_stay_available() {
    let tmp = TmpDir::new("guards-allowed");
    let mut app = open_app("guards-allowed", &tmp);
    answer(&mut app);
    assert!(app
        .act_argv(Act::Verify, Some("u001-katajainen"), None, None)
        .is_ok());
    for a in &app.snapshot.unit("u001-katajainen").unwrap().attempts {
        assert_eq!(
            app.question_guard(Act::Accept, Some("u001-katajainen"), false),
            None,
            "{}",
            a.record.id
        );
    }
    // Re-check every piece behind the program as it stands: not a guarded
    // act.
    assert_eq!(app.question_guard(Act::RecheckBehind, None, false), None);
    assert_eq!(app.question_guard(Act::MapFeatures, None, false), None);
    // Reading: the checks, the fold, the card, the log.
    app.select(Selection::Unit("u001-katajainen".into()));
    for it in app.menu_items() {
        if matches!(
            it.action,
            Action::ShowChecks | Action::Fold | Action::Reread
        ) {
            assert_eq!(it.greyed, None, "{it:?}");
        }
    }
    // The features file's Edit (or Write) on the Features row.
    app.select(Selection::Features);
    let edit = app
        .menu_items()
        .into_iter()
        .find(|i| i.action == Action::EditFeatures)
        .expect("the features file's item");
    assert_eq!(edit.greyed, None, "{edit:?}");
}

/// The dialog open now: its title, body, buttons' labels and focus.
fn shown(app: &App) -> (String, Vec<String>, Vec<String>, usize) {
    let Mode::Dialog(c) = &app.mode else {
        panic!("no dialog: {:?}", app.mode);
    };
    (
        c.title.clone(),
        c.body.clone(),
        c.dialog
            .buttons
            .iter()
            .map(|b| b.label.to_string())
            .collect(),
        c.dialog.focus,
    )
}

/// The chat paused for the chat's answer 4 minutes ago.
fn four_minutes(app: &mut App) {
    app.chat_pause = Some(ChatPause {
        since: Instant::now() - Duration::from_secs(4 * 60 + 5),
        turn: Some(1),
        piece: Some("u-cache".into()),
        logged: 0,
    });
}

const LOST: &str = "The chat is answering u-cache's question (4 min so far).";

/// Quit while the chat answers: it asks, saying what is lost, the safe
/// choice first and focused; Keep it running changes nothing, Quit anyway
/// quits. While the chat is only talking, today's dialog.
#[test]
fn quit_while_the_chat_answers_asks() {
    let tmp = TmpDir::new("guards-quit");
    let mut app = open_app("guards-quit", &tmp);
    app.focus = crate::app::Focus::View;
    // Not answering: today's words.
    assert_eq!(key(&mut app, 'q'), Command::None);
    assert_eq!(shown(&app).0, "Quit the cockpit?");
    code(&mut app, KeyCode::Esc);
    answer(&mut app);
    four_minutes(&mut app);
    assert_eq!(key(&mut app, 'q'), Command::None);
    let (title, body, buttons, focus) = shown(&app);
    assert_eq!(title, "Quit while the chat is answering?");
    assert_eq!(
        body,
        vec![format!(
            "{LOST} Quitting stops it and its work so far is lost; the question stays open."
        )]
    );
    assert_eq!(buttons, vec!["Keep it running", "Quit anyway"]);
    assert_eq!(focus, 0);
    arm(&mut app);
    assert_eq!(code(&mut app, KeyCode::Enter), Command::None);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.chat_answering("u-cache"), "nothing changed");
    key(&mut app, 'q');
    arm(&mut app);
    assert_eq!(key(&mut app, 'q'), Command::Quit);
}

/// A new chat while the chat answers: the same shape; Keep it running
/// keeps the chat, Start a new chat anyway starts one, as before.
#[test]
fn a_new_chat_while_the_chat_answers_asks() {
    let tmp = TmpDir::new("guards-new");
    let mut app = open_app("guards-new", &tmp);
    app.focus = crate::app::Focus::View;
    app.chat_ask_new();
    assert_eq!(shown(&app).0, "Start a new chat?");
    code(&mut app, KeyCode::Esc);
    answer(&mut app);
    four_minutes(&mut app);
    app.chat_ask_new();
    let (title, body, buttons, focus) = shown(&app);
    assert_eq!(title, "Start a new chat while this one is answering?");
    assert_eq!(
        body,
        vec![format!(
            "{LOST} A new chat stops it and its work so far is lost; the question stays open."
        )]
    );
    assert_eq!(buttons, vec!["Keep it running", "Start a new chat anyway"]);
    assert_eq!(focus, 0);
    arm(&mut app);
    let gen = app.chat.gen;
    code(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.chat_answering("u-cache") && app.chat.gen == gen);
    app.chat_ask_new();
    arm(&mut app);
    code(&mut app, KeyCode::Right);
    code(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.chat.alive(), "the old chat ended");
    assert!(!app.chat_answering("u-cache"));
}

/// Leaving the chat's screen (opening a tool) while it answers ends the
/// chat: the confirm says so; Keep it running stays, Close it and go on
/// goes on as before. (The chat column's `[×]` only folds the column: the
/// chat answers on, nothing is lost, and nothing is asked.)
#[test]
fn closing_the_chat_while_it_answers_asks() {
    let tmp = TmpDir::new("guards-close");
    let mut app = open_app("guards-close", &tmp);
    app.focus = crate::app::Focus::View;
    let open_tool = crate::menu::item("Open tool t", Action::OpenTool("t".into()), None);
    app.choose(&open_tool);
    assert_eq!(shown(&app).0, "Open tool t?");
    code(&mut app, KeyCode::Esc);
    answer(&mut app);
    four_minutes(&mut app);
    app.choose(&open_tool);
    let (title, body, buttons, focus) = shown(&app);
    assert_eq!(title, "Close the chat while it is answering?");
    assert_eq!(
        body,
        vec![format!(
            "{LOST} Opening tool t closes the chat, which stops it and its work so far is \
             lost; the question stays open."
        )]
    );
    assert_eq!(buttons, vec!["Keep it running", "Close it and go on"]);
    assert_eq!(focus, 0);
    arm(&mut app);
    assert_eq!(code(&mut app, KeyCode::Esc), Command::None);
    assert!(app.chat_answering("u-cache"));
    app.choose(&open_tool);
    arm(&mut app);
    assert_eq!(key(&mut app, 'y'), Command::OpenTool("t".into()));
}
