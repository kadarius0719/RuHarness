//! `harness-tui` — the review cockpit's terminal front end
//! (docs/TUI-DESIGN.md §3–§6). It reads the ledger with the library's read
//! model and spawns `harness --json …` for every write; it never takes the
//! writer lock and never writes the ledger itself.
//!
//! Signals (§4 "The TUI's own signals"): SIGINT, SIGTERM and SIGHUP →
//! interrupt the running command (it leads its own process group, so the
//! terminal's signal never reached it), wait ≤ 1 s, restore the terminal,
//! die BY the signal. In raw mode Ctrl-C is a key, not a signal. While the
//! editor of a hand edit runs in the foreground, SIGINT belongs to the
//! editor; TERM and HUP are forwarded to it, and the cockpit dies by them
//! once it is gone — never under it. A hand edit that was not recorded is
//! never removed on the way out: its path is printed.

#![forbid(unsafe_code)]

use harness_tui::app::{Act, App, Command, Config, LayoutMode};
use harness_tui::handedit;
use harness_tui::model::Snapshot;
use harness_tui::spawn::{self, ChildSlot, Running};
use harness_tui::view;
use ratatui::crossterm::cursor;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as TermEvent, KeyEventKind,
};
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::ExecutableCommand;
use ratatui::DefaultTerminal;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A terminal signal arrived: the loop stops drawing.
static DYING: AtomicBool = AtomicBool::new(false);

const USAGE: &str = "\
usage: harness-tui [--target DIR] [--harness PATH] [--allow-unsandboxed] [--layout split|stacked]

The review cockpit over a target's migration ledger. Every write is a spawned
`harness --json …` command whose exact argv is shown and confirmed first.

  --target DIR          the target repository root (default: .)
  --harness PATH        the harness binary acts spawn (default: `harness` on PATH,
                        else the one next to this binary)
  --allow-unsandboxed   pass --allow-unsandboxed to acts that run code
  --layout split|stacked
                        force side-by-side pairs, or stacked ones (default: side by
                        side at 110 columns and wider)
";

struct Args {
    target: PathBuf,
    harness: Option<PathBuf>,
    allow_unsandboxed: bool,
    layout: LayoutMode,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        target: PathBuf::from("."),
        harness: None,
        allow_unsandboxed: false,
        layout: LayoutMode::Auto,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let text = arg.to_string_lossy().into_owned();
        let (flag, attached) = match text.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (text.clone(), None),
        };
        let mut value = |name: &str| -> Result<std::ffi::OsString, String> {
            match &attached {
                Some(v) => Ok(v.into()),
                None => it.next().ok_or(format!("{name} needs a value")),
            }
        };
        match flag.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--target" => args.target = PathBuf::from(value("--target")?),
            "--harness" => args.harness = Some(PathBuf::from(value("--harness")?)),
            "--allow-unsandboxed" => args.allow_unsandboxed = true,
            "--layout" => {
                args.layout = match value("--layout")?.to_str() {
                    Some("split") => LayoutMode::Split,
                    Some("stacked") => LayoutMode::Stacked,
                    _ => return Err("--layout is split or stacked".into()),
                }
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    Ok(args)
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `--harness`, else `harness` on PATH, else the `harness` next to this
/// binary (a workspace build) — resolved to a path, so the argv each act
/// shows is exactly what runs.
fn resolve_harness(flag: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    if let Some(path) = flag {
        let path = path
            .canonicalize()
            .map_err(|e| format!("--harness {}: {e}", path.display()))?;
        if !executable(&path) {
            return Err(format!(
                "--harness {}: not an executable file",
                path.display()
            ));
        }
        return Ok(Some(path));
    }
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("harness"))
            .find(|p| executable(p))
    });
    let sibling = || {
        std::env::current_exe()
            .ok()?
            .parent()
            .map(|dir| dir.join("harness"))
            .filter(|p| executable(p))
    };
    Ok(on_path.or_else(sibling))
}

/// Where the kept hand edits live (mirrors `App::kept_paths`) — for the
/// signal path, which cannot reach the `App`: they are never removed there,
/// only named, so a signal never costs the user their edit.
static KEPT_EDITS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// The hand edit's editor while it runs: TERM/HUP are forwarded to it and
/// recorded here; the main thread, which owns it, dies by the signal once
/// the editor is gone.
static EDITOR: Mutex<Option<Editing>> = Mutex::new(None);

#[derive(Default)]
struct Editing {
    child: Option<Child>,
    signal: Option<i32>,
}

fn guard<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Tell the user where every kept hand edit is (stderr, after the
/// terminal is restored).
fn announce_kept_edits() {
    for dir in guard(&KEPT_EDITS).iter() {
        let _ = writeln!(
            std::io::stderr(),
            "harness-tui: a hand edit that was not recorded is kept in {}",
            dir.display()
        );
    }
}

/// Signal the editor with `sig` — only while it is unreaped (never a pid
/// the system may have reused).
fn forward(child: &mut Child, sig: i32) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = std::process::Command::new("/bin/kill")
            .args([format!("-{sig}"), child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Leave the terminal as the shell expects it: no bracketed paste, cursor
/// shown, main screen, cooked mode.
fn restore_terminal() {
    let mut out = std::io::stdout();
    let _ = out.execute(DisableBracketedPaste);
    let _ = out.execute(cursor::Show);
    let _ = ratatui::try_restore();
}

fn install_signal_path(slot: ChildSlot) -> std::io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP])?;
    std::thread::spawn(move || {
        for sig in signals.forever() {
            {
                let mut editing = guard(&EDITOR);
                if let Some(e) = editing.as_mut() {
                    // The editor owns the terminal: SIGINT is its own; TERM
                    // and HUP reach it (never a reaped pid), and the main
                    // thread dies by them once it is gone.
                    if sig != SIGINT {
                        e.signal.get_or_insert(sig);
                        if let Some(child) = e.child.as_mut() {
                            forward(child, sig);
                        }
                    }
                    continue;
                }
            }
            DYING.store(true, Ordering::SeqCst);
            // The CLI's 250 ms courtesy budget plus its group kill.
            let _ = spawn::interrupt_and_wait(&slot, Duration::from_secs(1));
            restore_terminal();
            announce_kept_edits();
            die_by(sig);
        }
    });
    Ok(())
}

fn die_by(sig: i32) -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(sig);
    std::process::exit(128 + sig);
}

/// Hand the terminal to the editor: cooked mode, main screen, cursor shown.
fn suspend(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    std::io::stdout().execute(DisableBracketedPaste)?;
    terminal.show_cursor()?;
    terminal::disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Take the terminal back and repaint everything — through `resize`, not
/// `Terminal::clear`, which asks the terminal for the cursor position (a
/// terminal that never answers would stall it and fail).
fn resume(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    std::io::stdout().execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    std::io::stdout().execute(EnableBracketedPaste)?;
    terminal.hide_cursor()?;
    let size = terminal.size()?;
    terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))
}

/// Run the editor on the session's files as a tracked child (so TERM/HUP
/// can be forwarded to it), and wait. The signal that arrived meanwhile, if
/// any, is returned: the caller dies by it once the edit is safe.
fn edit_in_editor(session: &handedit::Session) -> (std::io::Result<ExitStatus>, Option<i32>) {
    let editor = handedit::editor_command(std::env::var_os("VISUAL"), std::env::var_os("EDITOR"));
    *guard(&EDITOR) = Some(Editing::default());
    let status = match session.spawn_editor(&editor) {
        Ok(child) => {
            let pid = child.id();
            if let Some(e) = guard(&EDITOR).as_mut() {
                // A TERM/HUP that arrived while the editor was starting is
                // forwarded now.
                let child = e.child.insert(child);
                if let Some(sig) = e.signal {
                    forward(child, sig);
                }
            }
            loop {
                let done = match guard(&EDITOR).as_mut().and_then(|e| e.child.as_mut()) {
                    Some(child) => child.try_wait(),
                    None => Err(std::io::Error::other(format!("editor {pid} lost"))),
                };
                match done {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(e) => break Err(e),
                }
            }
        }
        Err(e) => Err(e),
    };
    let signal = guard(&EDITOR).take().and_then(|e| e.signal);
    (status, signal)
}

/// The hand edit: copy, edit (the terminal handed to the editor), stage —
/// the staged edit comes first, so a terminal that does not come back
/// cleanly never costs the user their edit, and an editor that exits
/// non-zero after saving keeps it too. `Ok` = staged (its note, then the
/// argv, are asked for next).
fn hand_edit(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    unit: &str,
    crate_dir: &Path,
) -> Result<(), String> {
    let session = handedit::prepare(crate_dir, &std::env::temp_dir())
        .map_err(|e| format!("hand edit: {e}"))?;
    let discard = |why: String| {
        let _ = std::fs::remove_dir_all(&session.tmp);
        why
    };
    if let Err(e) = suspend(terminal) {
        let _ = resume(terminal);
        return Err(discard(format!("hand edit: {e}")));
    }
    let (status, signal) = edit_in_editor(&session);
    let changed = session.changed().unwrap_or(true);
    // What the editor saved, or left beside the files (its recovery data
    // after a hangup): never removed.
    let keep = changed || session.has_leftovers();
    if let Some(sig) = signal {
        // TERM/HUP while editing: the edit is kept, the cockpit dies by it.
        if keep {
            guard(&KEPT_EDITS).push(session.tmp.join("edit"));
        } else {
            let _ = std::fs::remove_dir_all(&session.tmp);
        }
        restore_terminal();
        announce_kept_edits();
        die_by(sig);
    }
    let staged = match status {
        Ok(status) if status.success() => session.stage().map_err(|e| format!("hand edit: {e}")),
        Ok(status) => Err(format!("hand edit aborted: the editor exited {status}")),
        Err(e) => Err(format!("hand edit: the editor: {e}")),
    };
    let resumed = resume(terminal);
    // Keys typed into the cooked terminal while the editor ran answer
    // nothing (a buffered Esc or `n` must not decide about this edit).
    while event::poll(Duration::ZERO).unwrap_or(false) {
        if event::read().is_err() {
            break;
        }
    }
    let result = match staged {
        Ok(Some(stage)) => {
            app.edit_staged(unit.to_string(), stage, session.tmp.clone());
            if let Err(e) = resumed {
                app.notice = Some(format!(
                    "the terminal did not come back cleanly ({e}); the edit is kept"
                ));
            }
            Ok(())
        }
        Ok(None) if session.has_leftovers() => {
            app.leftovers.push(session.tmp.clone());
            Err("no change to record; what the editor left is kept (printed on quit)".into())
        }
        Ok(None) => Err(discard("no change; nothing to record".into())),
        // An abort is not a discard: macOS `vi` exits 1 after any error
        // message even when `:wq` wrote the files — a changed edit is kept
        // (E offers it), recovery data too.
        Err(why) if changed => match session.stage() {
            Ok(Some(stage)) => {
                app.keep_edit(unit.to_string(), stage, session.tmp.clone());
                Err(format!("{why}; the edit is kept — E offers it"))
            }
            _ => {
                app.leftovers.push(session.tmp.clone());
                Err(format!(
                    "{why}; the edited files are kept (printed on quit)"
                ))
            }
        },
        Err(why) if keep => {
            app.leftovers.push(session.tmp.clone());
            Err(format!(
                "{why}; what the editor left is kept (printed on quit)"
            ))
        }
        Err(why) => Err(discard(why)),
    };
    *guard(&KEPT_EDITS) = app.kept_paths();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App, slot: &ChildSlot) -> std::io::Result<()> {
    let mut running: Option<Running> = None;
    let mut last_tick = Instant::now();
    loop {
        if DYING.load(Ordering::SeqCst) {
            // The signal path owns the terminal now.
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        terminal.draw(|f| view::draw(f, app))?;
        // A prompt drawn whole with no input pending may take its `y`:
        // typed-ahead or pasted input never answers it.
        if app.confirm_waiting() && !event::poll(Duration::ZERO)? {
            app.confirm_armed = true;
        }
        let mut command = Command::None;
        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    command = app.on_key(key);
                }
                TermEvent::Paste(text) => app.on_paste(&text),
                _ => {}
            }
        }
        if let Some(r) = running.as_mut() {
            for msg in r.drain() {
                app.on_child_msg(msg);
            }
            r.poll_exit()?;
            if let Some(status) = r.finished() {
                running = None;
                if let Some(tmp) = app.on_child_exit(status) {
                    let _ = std::fs::remove_dir_all(tmp);
                }
            }
        }
        if last_tick.elapsed() >= Duration::from_secs(2) {
            app.tick();
            last_tick = Instant::now();
        }
        match command {
            Command::None => {}
            Command::Quit => break,
            Command::CancelAndQuit => {
                let _ = spawn::interrupt_and_wait(slot, Duration::from_secs(1));
                // A reaped child is finished the normal way (a recorded
                // hand edit is removed).
                if let Some(r) = running.as_mut() {
                    let deadline = Instant::now() + Duration::from_millis(500);
                    loop {
                        for msg in r.drain() {
                            app.on_child_msg(msg);
                        }
                        r.poll_exit()?;
                        if let Some(status) = r.finished() {
                            if let Some(tmp) = app.on_child_exit(status) {
                                let _ = std::fs::remove_dir_all(tmp);
                            }
                            break;
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                break;
            }
            Command::Spawn(pending) => match Running::spawn(pending.argv.clone(), slot.clone()) {
                Ok(r) => {
                    app.on_spawned(&pending);
                    running = Some(r);
                }
                Err(e) => app.on_spawn_failed(pending, &e.to_string()),
            },
            Command::Cancel => {
                if let Some(r) = running.as_mut() {
                    match r.interrupt() {
                        Ok(true) => app.notice = Some("SIGINT sent; the command cancels".into()),
                        Ok(false) => app.notice = Some("the command already ended".into()),
                        Err(e) => app.notice = Some(format!("cancel failed: {e}")),
                    }
                }
            }
            Command::Reload => {
                if app.reload(true) {
                    app.notice = Some("ledger re-read".into());
                }
            }
            Command::Edit { unit, crate_dir } => {
                if let Err(why) = hand_edit(terminal, app, &unit, &crate_dir) {
                    app.notice = Some(why);
                }
            }
            Command::Cleanup(tmp) => {
                let _ = std::fs::remove_dir_all(tmp);
            }
        }
        *guard(&KEPT_EDITS) = app.kept_paths();
    }
    *guard(&KEPT_EDITS) = app.kept_paths();
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(why) if why.is_empty() => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("harness-tui: {why}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let target = match args.target.canonicalize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("harness-tui: --target {}: {e}", args.target.display());
            return ExitCode::from(2);
        }
    };
    let snapshot = match Snapshot::load(&target) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("harness-tui: {e}");
            return ExitCode::from(1);
        }
    };
    let harness = match resolve_harness(args.harness) {
        Ok(h) => h,
        Err(why) => {
            eprintln!("harness-tui: {why}");
            return ExitCode::from(2);
        }
    };
    let slot = ChildSlot::default();
    if let Err(e) = install_signal_path(slot.clone()) {
        eprintln!("harness-tui: signals: {e}");
        return ExitCode::from(1);
    }
    let mut app = App::new(
        Config {
            target,
            harness: harness.clone(),
            allow_unsandboxed: args.allow_unsandboxed,
            layout: args.layout,
        },
        snapshot,
    );
    if harness.is_none() {
        app.notice = Some(
            "no `harness` binary found (PATH, or --harness <path>): read-only, acts disabled"
                .into(),
        );
    }
    let mut terminal = ratatui::init();
    // ratatui's hook restores raw mode and the screen; ours also turns
    // bracketed paste off, shows the cursor and names kept edits.
    let ratatui_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        announce_kept_edits();
        ratatui_hook(info);
    }));
    let _ = std::io::stdout().execute(EnableBracketedPaste);
    let result = run(&mut terminal, &mut app, &slot);
    restore_terminal();
    announce_kept_edits();
    if app.running && app.run.as_ref().is_some_and(|r| r.act == Act::HandEdit) {
        eprintln!(
            "harness-tui: the hand-edit override is still running and may yet record the latest \
             of these (`harness state status` shows it)"
        );
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("harness-tui: {e}");
            ExitCode::from(1)
        }
    }
}
