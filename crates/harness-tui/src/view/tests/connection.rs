//! The chat's connection (docs/CHAT-PANE-DESIGN.md §1.6) and the Project
//! view's model work asked in chat (§3.6): the walk-through in each of its
//! cases, the greyed items and their reason, a Next step that never
//! recommends the chat while it is unavailable, Check again, the question
//! typed for a held choice, and the chat's recorded advice on the Project
//! view and in Accept's picker.

use super::project::{cockpit, project, with_chat};
use super::*;
use crate::app::tests::{arm, code, key};
use crate::app::{Focus, Purpose};
use crate::chat::{Problem, Trouble};
use crate::menu::Action;
use crate::project::tests::{map_text, write_map};
use std::path::Path;

/// An executable `sh` script at `dir/name`.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The Project view over a map with one held choice (t-unlzg's d1), the
/// chat on.
fn held(tag: &str) -> (crate::testutil::TmpDir, PathBuf, App) {
    let (tmp, root) = project(tag);
    write_map(&root, &map_text("stated", false));
    let mut app = cockpit(&root);
    with_chat(&mut app);
    (tmp, root, app)
}

fn labels(app: &App) -> Vec<(String, Option<String>)> {
    match &app.mode {
        Mode::Menu(m) => m
            .items
            .iter()
            .map(|i| (i.label.clone(), i.greyed.clone()))
            .collect(),
        other => panic!("no menu: {other:?}"),
    }
}

/// Choose the open menu's item labelled `label`.
fn choose(app: &mut App, label: &str) -> Command {
    let at = labels(app)
        .iter()
        .position(|(l, _)| l == label)
        .unwrap_or_else(|| panic!("no item {label:?} in {:?}", labels(app)));
    for _ in 0..at {
        code(app, KeyCode::Down);
    }
    code(app, KeyCode::Enter)
}

fn dialog(app: &App) -> (String, Vec<String>) {
    match &app.mode {
        Mode::Dialog(c) if c.purpose == Purpose::Connection => (c.title.clone(), c.body.clone()),
        other => panic!("no connection dialog: {other:?}"),
    }
}

const STEPS: [&str; 4] = [
    "1. Open a new Terminal window.",
    "2. Type claude and press Return.",
    "3. Follow the sign-in it shows.",
    "4. Come back here and choose Check again.",
];

/// The walk-through in each case, verbatim, opened from "Ask for advice —
/// ask in chat" (not installed: `claude` missing, or its `--version`
/// failing) and from the chat's own stream (not signed in, stopped); its
/// two buttons; nothing typed into the chat while it stands.
#[test]
fn the_walk_through_says_what_failed_and_the_steps() {
    // Not installed: no claude at all, found at the start — the chat's
    // items are greyed with it, and Check the chat's connection opens the
    // walk-through.
    let (tmp, root, _) = held("conn-missing");
    let mut app = cockpit(&root);
    app.set_chat(true);
    app.chat = crate::chat::Chat::new(
        Err("claude not found: install Claude Code or pass --chat-runtime".into()),
        &[],
        None,
        root.clone(),
        Default::default(),
    );
    app.select(Selection::Program("t-unlzg".into()));
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Check the chat's connection…");
    let (title, body) = dialog(&app);
    assert_eq!(title, "Claude Code is not installed");
    assert_eq!(
        body,
        [
            "The chat (and every \"ask in chat\" item) runs Claude Code, and the cockpit could \
             not run it. Install Claude Code first, then:",
            STEPS[0],
            STEPS[1],
            STEPS[2],
            STEPS[3],
            "What the cockpit saw: claude not found: install Claude Code or pass --chat-runtime",
        ]
    );
    assert!(app.chat.input.is_empty(), "nothing typed while it stands");
    let screen = text(&render(&mut app, 120, 40));
    for words in [
        "Claude Code is not installed",
        "1. Open a new Terminal window.",
        "4. Come back here and choose Check again.",
        "Check again",
        "Cancel",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    // Not installed: `claude --version` fails.
    let broken = script(
        &tmp.0,
        "claude",
        "echo 'dyld: Library not loaded' >&2; exit 134",
    );
    app.mode = Mode::Normal;
    app.chat.bins = Ok(crate::chat::runtime::Binaries {
        claude: broken,
        mcp: "/bin/sh".into(),
    });
    app.chat.problem = None;
    app.chat.installed = false;
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Ask for advice — ask in chat");
    let (title, body) = dialog(&app);
    assert_eq!(title, "Claude Code is not installed");
    assert_eq!(
        body.last().unwrap(),
        "What the cockpit saw: `claude --version` failed: dyld: Library not loaded"
    );
    // Not signed in, from the chat's stream.
    for (trouble, title, lead, seen) in [
        (
            Trouble::NotSignedIn,
            "Claude Code is not signed in",
            "The chat (and every \"ask in chat\" item) runs Claude Code, and it says it is not \
             signed in:",
            "Claude Code said: Not logged in · Please run /login",
        ),
        (
            Trouble::Stopped,
            "Claude Code stopped",
            "The chat's Claude Code stopped (what it said is below). When it is the sign-in:",
            "Error: something broke",
        ),
    ] {
        app.mode = Mode::Normal;
        app.chat.problem = Some(Problem {
            trouble,
            seen: seen.into(),
            fix: None,
        });
        app.chat_trouble_seen();
        let (t, body) = dialog(&app);
        assert_eq!(t, title);
        assert_eq!(body[0], lead);
        assert_eq!(&body[1..5], &STEPS);
        assert_eq!(body[5], format!("What the cockpit saw: {seen}"));
        let Mode::Dialog(c) = &app.mode else { panic!() };
        let buttons: Vec<String> = c
            .dialog
            .buttons
            .iter()
            .map(|b| format!("{} {}", b.label, b.key))
            .collect();
        assert_eq!(buttons, ["Cancel Esc", "Check again y"]);
    }
    // An old harness-mcp beside a newer cockpit: a rebuild, said as one —
    // its own title, the line to paste as step 2, never the sign-in steps.
    app.mode = Mode::Normal;
    app.chat.problem = Some(Problem {
        trouble: Trouble::OldTools,
        seen: "harness-mcp at /w/target/debug/harness-mcp lacks harness_map_questions".into(),
        fix: Some("cd /w && cargo build -p harness-mcp".into()),
    });
    app.chat_trouble_seen();
    let (t, body) = dialog(&app);
    assert_eq!(t, "The chat's harness tools are out of date");
    assert_eq!(
        body,
        [
            "The chat reads the project through harness-mcp, and the one the cockpit found is \
             older than the cockpit: it lacks a tool the chat needs. Rebuild it:",
            "1. Open a new Terminal window.",
            "2. Paste this line and press Return: cd /w && cargo build -p harness-mcp",
            "3. Wait for it to finish (about a minute).",
            "4. Come back here and choose Check again.",
            "What the cockpit saw: harness-mcp at /w/target/debug/harness-mcp lacks \
             harness_map_questions",
        ]
    );
    assert_eq!(
        app.chat_unavailable().as_deref(),
        Some(
            "The chat's harness tools are out of date — choose Check the chat's connection to \
             see how to fix it"
        )
    );
    // While a dialog of another kind is open, the notice says it instead.
    app.chat.problem = Some(Problem {
        trouble: Trouble::Stopped,
        seen: "Error: something broke".into(),
        fix: None,
    });
    app.mode = Mode::Help { scroll: 0 };
    app.chat_trouble_seen();
    assert!(matches!(app.mode, Mode::Help { .. }));
    assert!(
        app.notice.as_ref().is_some_and(|n| n
            .text
            .contains("Claude Code stopped — choose Check the chat's connection")),
        "{:?}",
        app.notice
    );
}

/// While the chat is unavailable: its items are greyed with the reason,
/// "Check the chat's connection…" opens the walk-through, and the Next
/// step never recommends a chat item — on the Project view and on a
/// tool's screen.
#[test]
fn while_the_chat_is_unavailable_its_items_are_greyed_and_never_recommended() {
    let (_tmp, _root, mut app) = held("conn-greyed");
    app.chat.problem = Some(Problem {
        trouble: Trouble::NotSignedIn,
        seen: "Claude Code said authentication_failed".into(),
        fix: None,
    });
    let reason =
        "Claude Code is not signed in — choose Check the chat's connection to see how to fix it";
    app.select(Selection::Program("t-unlzg".into()));
    code(&mut app, KeyCode::Enter);
    let items = labels(&app);
    let find = |label: &str| {
        items
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("{label}: {items:?}"))
            .1
            .clone()
    };
    assert_eq!(find("Check the chat's connection…"), None);
    assert_eq!(
        find("Ask for advice — ask in chat").as_deref(),
        Some(reason)
    );
    assert_eq!(find("Ask in chat…").as_deref(), Some(reason));
    let screen = text(&render(&mut app, 120, 40));
    assert!(screen.contains("Claude Code is not signed in"), "{screen}");
    choose(&mut app, "Check the chat's connection…");
    assert_eq!(dialog(&app).0, "Claude Code is not signed in");

    // A tool's screen: the Next step of a unit with no test program says
    // why, and leads to the walk-through; Make the test program is greyed
    // with it and not recommended, Migrate greyed for want of a test
    // program.
    let mut tool = crate::app::tests::app_of("targets/zopfli", "conn-tool");
    tool.chat_on = true;
    tool.chat.bins = Ok(crate::chat::runtime::Binaries {
        claude: "/bin/sh".into(),
        mcp: "/bin/sh".into(),
    });
    tool.chat.problem = app.chat.problem.clone();
    tool.select(Selection::Unit("u-cache".into()));
    let next = tool.advice(&tool.selection.clone()).unwrap();
    assert_eq!(
        next.text,
        format!("Make the test program — the chat does that, and it is unavailable: {reason}")
    );
    assert_eq!(next.action, Some(Action::CheckChat));
    let project = tool.next_step().map(|a| tool.unless_chat_down(a)).unwrap();
    assert_ne!(project.action, Some(Action::Migrate), "{}", project.text);
    code(&mut tool, KeyCode::Enter);
    let Mode::Menu(m) = &tool.mode else {
        panic!("{:?}", tool.mode)
    };
    assert_eq!(m.items[0].label, "Check the chat's connection…");
    let migrate = m
        .items
        .iter()
        .find(|i| i.label == "Migrate — ask in chat")
        .unwrap();
    assert_eq!(
        migrate.greyed.as_deref(),
        Some("it has no test program yet — Make the test program first")
    );
    let make = m
        .items
        .iter()
        .find(|i| i.label == "Make the test program — ask in chat")
        .unwrap();
    assert_eq!(make.greyed.as_deref(), Some(reason));
}

/// Check again: `claude` found and answering — the problem goes; after a
/// failed sign-in the stopped chat ends and the last message is sent again
/// (a new chat starts, which shows the sign-in); still missing, the
/// walk-through stays.
#[test]
fn check_again_looks_again_and_sends_the_last_message_again() {
    let (tmp, _root, mut app) = held("conn-again");
    app.chat.temp = tmp.0.clone();
    app.chat.problem = Some(Problem {
        trouble: Trouble::NotInstalled,
        seen: "claude not found".into(),
        fix: None,
    });
    app.chat.bins = Err("claude not found: install Claude Code or pass --chat-runtime".into());
    app.open_menu();
    choose(&mut app, "Check the chat's connection…");
    arm(&mut app);
    key(&mut app, 'y');
    // Still missing (no resolver here): the walk-through again.
    assert_eq!(dialog(&app).0, "Claude Code is not installed");
    // Installed now: the problem goes; nothing to send again.
    let good = script(
        &tmp.0,
        "claude",
        "[ \"$1\" = --version ] && { echo '2.1.274 (Claude Code)'; exit 0; }\nexec cat > /dev/null",
    );
    app.chat.resolver = Some(crate::chat::Resolver {
        runtime: Some(good),
        mcp: Some("/bin/sh".into()),
        exe: None,
        path: None,
    });
    arm(&mut app);
    key(&mut app, 'y');
    assert!(matches!(app.mode, Mode::Normal), "{:?}", app.mode);
    assert!(app.chat.problem.is_none() && app.chat.installed);
    assert!(
        app.notice.as_ref().is_some_and(|n| n.text
            == "Claude Code is installed — the sign-in is checked when the chat starts: send \
                your message"),
        "{:?}",
        app.notice
    );
    // After a failed sign-in: the last message goes again, in a new chat.
    app.chat.last_sent = Some("Which file should t-unlzg keep for choice d1, and why?".into());
    app.chat.problem = Some(Problem {
        trouble: Trouble::NotSignedIn,
        seen: "Claude Code said authentication_failed".into(),
        fix: None,
    });
    app.focus = Focus::Files;
    app.mode = Mode::Normal;
    app.open_menu();
    choose(&mut app, "Check the chat's connection…");
    arm(&mut app);
    key(&mut app, 'y');
    assert!(app.chat.alive(), "a new chat started");
    assert_eq!(app.focus, Focus::Chat);
    assert!(
        app.notice.as_ref().is_some_and(|n| n
            .text
            .starts_with("Claude Code is installed — your last message is sent again")),
        "{:?}",
        app.notice
    );
    app.chat.end(
        std::time::Instant::now(),
        std::time::Duration::ZERO,
        std::time::Duration::ZERO,
    );
    crate::chat::runtime::kill_all(&app.chat.procs);
}

/// "Ask for advice — ask in chat" on a held program types its question;
/// the chat's answer, once recorded (as harness-mcp's `harness_map_answer`
/// writes it), shows on the Project view and beside the choice in Accept's
/// picker.
#[test]
fn the_question_is_typed_and_the_recorded_advice_shows() {
    let (_tmp, root, mut app) = held("conn-advice");
    app.select(Selection::Program("t-unlzg".into()));
    code(&mut app, KeyCode::Enter);
    let cmd = choose(&mut app, "Ask for advice — ask in chat");
    assert!(matches!(cmd, Command::None));
    assert_eq!(app.focus, Focus::Chat);
    assert_eq!(
        app.chat.input.text,
        "Which file should t-unlzg keep for choices d1 and d2, and why?"
    );
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|n| n.text.starts_with("the question is typed into the chat")),
        "{:?}",
        app.notice
    );
    // Elsewhere on the view: every held choice.
    app.select(Selection::MapProject);
    assert_eq!(
        app.project_chat_question(false),
        "Which file should each held choice keep, and why?"
    );
    // The chat records its answer (the file as `record_answers` writes it).
    std::fs::write(
        root.join(crate::project::REPLY_FILE),
        r#"{
  "schema": "ruharness-project-map-reply",
  "root_hash": "blake3:aa",
  "inputs_hash": "blake3:bb",
  "items": {
    "d1": {
      "keep": "d1.2",
      "reason": "alternative-implementation",
      "model": "claude-opus-5-5",
      "provider": "chat"
    }
  }
}
"#,
    )
    .unwrap();
    app.chat_map_answered();
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|n| n.text.starts_with("the chat's answer is recorded")),
        "{:?}",
        app.notice
    );
    app.focus = Focus::Files;
    app.select(Selection::Program("t-unlzg".into()));
    let screen = text(&render(&mut app, 220, 44));
    for words in [
        "│  2  src/mini.c  ← the model advises this one ",
        "│Why (claude-opus-5-5): they are two implementations of the same thing. ",
    ] {
        assert!(screen.contains(words), "{words}\n{screen}");
    }
    code(&mut app, KeyCode::Enter);
    choose(&mut app, "Accept this program");
    let Mode::Menu(m) = &app.mode else {
        panic!("{:?}", app.mode)
    };
    assert_eq!(
        m.note[1],
        "Why (claude-opus-5-5): they are two implementations of the same thing."
    );
}
