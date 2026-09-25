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
//! never removed on the way out: its path is printed. The restores never
//! block (the terminal guard, `harness_tui::termguard`), and the ledger is
//! read on a loader thread (`harness_tui::load`), the preflight first.

#![forbid(unsafe_code)]

use harness_tui::app::{Act, App, Command, Config, LayoutMode, LoadWhy};
use harness_tui::handedit;
use harness_tui::load::{self, Loader};
use harness_tui::spawn::{self, ChildSlot, Running};
use harness_tui::termguard::{TermGuard, ENABLE_WAIT};
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
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The terminal guard: every enable runs under it; the signal path and the
/// panic hook restore through it without ever blocking. Once it is dying
/// the loop stops drawing.
static GUARD: TermGuard = TermGuard::new();

const USAGE: &str = "\
usage: harness-tui [--target DIR] [--harness PATH] [--provider NAME]... [--allow-unsandboxed]
                   [--layout split|stacked]

The review cockpit over a target's migration ledger. Every write is a spawned
`harness --json …` command whose exact argv is shown and confirmed first.

  --target DIR          the target repository root (default: .)
  --harness PATH        the harness binary acts spawn (default: `harness` on PATH,
                        else the one next to this binary)
  --provider NAME       a provider profile model acts may use (repeatable;
                        default: external). Modify passes the first; Retry
                        runs only for an attempt whose provider is listed.
                        The target's harness.toml never chooses it.
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
    providers: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        target: PathBuf::from("."),
        harness: None,
        allow_unsandboxed: false,
        layout: LayoutMode::Auto,
        providers: Vec::new(),
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
            "--provider" => {
                let name = value("--provider")?.to_string_lossy().into_owned();
                // A profile name travels attached in one argv element: a
                // plain word, never a flag or a path.
                if name.is_empty()
                    || name.len() > 64
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                    || name.starts_with(['-', '.'])
                {
                    return Err(format!("--provider {name:?}: not a provider profile name"));
                }
                if !args.providers.contains(&name) {
                    args.providers.push(name);
                }
            }
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
    if args.providers.is_empty() {
        args.providers
            .push(harness_tui::app::EXTERNAL_PROVIDER.to_string());
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
            GUARD.mark_dying();
            // The CLI's 250 ms courtesy budget plus its group kill.
            let _ = spawn::interrupt_and_wait(&slot, Duration::from_secs(1));
            GUARD.restore_for_death(restore_terminal, ENABLE_WAIT);
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
/// terminal that never answers would stall it and fail). Under the guard:
/// once the cockpit is dying it never re-enables anything (the signal path
/// owns the terminal; this thread parks until the process ends).
fn resume(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    let enabled = GUARD.enable(|| -> std::io::Result<()> {
        std::io::stdout().execute(EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;
        std::io::stdout().execute(EnableBracketedPaste)?;
        terminal.hide_cursor()?;
        let size = terminal.size()?;
        terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))
    });
    match enabled {
        Some(result) => result,
        None => park(),
    }
}

/// The signal path is ending the process: wait for it.
fn park() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(5));
    }
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
    if let Err(e) = suspend(terminal) {
        let _ = resume(terminal);
        let _ = std::fs::remove_dir_all(&session.tmp);
        return Err(format!("hand edit: {e}"));
    }
    let (status, signal) = edit_in_editor(&session);
    finish_edit(app, unit, &session, status, signal, || {
        let resumed = resume(terminal);
        // Keys typed into the cooked terminal while the editor ran answer
        // nothing (a buffered Esc or `n` must not decide about this edit).
        while event::poll(Duration::ZERO).unwrap_or(false) {
            if event::read().is_err() {
                break;
            }
        }
        resumed
    })
}

/// After the editor: what it saved, or left beside the files, joins the
/// kept list the signal path prints BEFORE anything else happens — staging,
/// or `resume`, which a signal may interrupt (SAFE-10, CHK-5) — then the
/// edit is staged and the terminal taken back.
fn finish_edit(
    app: &mut App,
    unit: &str,
    session: &handedit::Session,
    status: std::io::Result<ExitStatus>,
    signal: Option<i32>,
    resume: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), String> {
    let discard = |why: String| {
        let _ = std::fs::remove_dir_all(&session.tmp);
        why
    };
    let changed = session.changed().unwrap_or(true);
    // What the editor saved, or left beside the files (its recovery data
    // after a hangup): never removed.
    let keep = changed || session.has_leftovers();
    if keep {
        let edit = session.tmp.join("edit");
        let mut kept = guard(&KEPT_EDITS);
        if !kept.contains(&edit) {
            kept.push(edit);
        }
    }
    if let Some(sig) = signal {
        // TERM/HUP while editing: the edit is kept, the cockpit dies by it.
        if !keep {
            let _ = std::fs::remove_dir_all(&session.tmp);
        }
        GUARD.restore_for_death(restore_terminal, ENABLE_WAIT);
        announce_kept_edits();
        die_by(sig);
    }
    let staged = match status {
        Ok(status) if status.success() => session.stage().map_err(|e| format!("hand edit: {e}")),
        Ok(status) => Err(format!("hand edit aborted: the editor exited {status}")),
        Err(e) => Err(format!("hand edit: the editor: {e}")),
    };
    let resumed = resume();
    let result = match staged {
        Ok(Some(stage)) => {
            app.edit_staged(unit.to_string(), stage, session.tmp.clone());
            if let Err(e) = resumed {
                app.say(format!(
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

/// Start the loads the app asked for, and hand it the ones that finished:
/// the requests in flight fold (the loader answers the latest), so each
/// finished load carries the strongest reason asked for up to it.
fn pump_loads(app: &mut App, loader: &mut Loader, whys: &mut Vec<(u64, LoadWhy)>) {
    if let Some(why) = app.load_request.take() {
        let seq = loader.request(&app.config.target);
        whys.push((seq, why));
    }
    if let Some(loaded) = loader.poll() {
        let why = whys
            .iter()
            .filter(|(seq, _)| *seq <= loaded.seq)
            .map(|(_, why)| *why)
            .max()
            .unwrap_or(LoadWhy::Tick);
        whys.retain(|(seq, _)| *seq > loaded.seq);
        app.on_loaded(loaded.result, why);
    }
    app.loading = loader.loading();
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    slot: &ChildSlot,
    loader: &mut Loader,
) -> std::io::Result<()> {
    let mut running: Option<Running> = None;
    let mut last_tick = Instant::now();
    let mut whys: Vec<(u64, LoadWhy)> = Vec::new();
    loop {
        if GUARD.dying() {
            // The signal path owns the terminal now.
            park();
        }
        pump_loads(app, loader, &mut whys);
        app.expire_notice(Instant::now());
        terminal.draw(|f| view::draw(f, app))?;
        // A dialog drawn whole, quiet for 300 ms, with no input pending,
        // arms: typed-ahead, pasted or auto-repeated input never answers it.
        if app.dialog_waiting() {
            let pending = event::poll(Duration::ZERO)?;
            app.arm(Instant::now(), pending);
        }
        let mut command = Command::None;
        if event::poll(Duration::from_millis(60))? {
            let event = event::read()?;
            let now = Instant::now();
            // Every input read restarts an open dialog's quiet time.
            app.on_input(now);
            match event {
                TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    command = app.on_key(key, now);
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
                        Ok(true) => app.say("SIGINT sent; the command cancels"),
                        Ok(false) => app.say("the command already ended"),
                        Err(e) => app.say(format!("cancel failed: {e}")),
                    }
                }
            }
            Command::Reload => app.request_load(LoadWhy::Key),
            Command::Edit { unit, crate_dir } => {
                if let Err(why) = hand_edit(terminal, app, &unit, &crate_dir) {
                    app.say(why);
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
    if !target.join("harness.toml").is_file() {
        eprintln!(
            "harness-tui: {} is not a harness target (no harness.toml); start with `--target \
             <target dir>`",
            target.display()
        );
        return ExitCode::from(2);
    }
    // The first read runs here, before the terminal is taken: a target the
    // preflight refuses is refused in words.
    let snapshot = match load::read(&target) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("harness-tui: {} is unreadable: {e}", target.display());
            return ExitCode::from(1);
        }
    };
    let mut loader = match Loader::spawn(load::read) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("harness-tui: the loader thread: {e}");
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
            providers: args.providers,
        },
        snapshot,
    );
    if harness.is_none() {
        app.say("no `harness` binary found (PATH, or --harness <path>): read-only, acts disabled");
    }
    // Under the guard: a signal during the setup restores after it.
    let Some(mut terminal) = GUARD.enable(|| {
        let terminal = ratatui::init();
        let _ = std::io::stdout().execute(EnableBracketedPaste);
        terminal
    }) else {
        park();
    };
    // ratatui's hook restores raw mode and the screen; ours also turns
    // bracketed paste off, shows the cursor and names kept edits — without
    // the guard's mutex (a panic inside an enable holds it).
    let ratatui_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        GUARD.restore_on_panic(restore_terminal);
        announce_kept_edits();
        ratatui_hook(info);
    }));
    let result = run(&mut terminal, &mut app, &slot, &mut loader);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    /// SAFE-10, CHK-5: a changed hand edit is on the kept list the signal
    /// path prints BEFORE `resume` runs — a signal that lands during
    /// `resume` names it.
    #[test]
    fn a_changed_edit_is_kept_before_the_terminal_is_taken_back() {
        let base = std::env::temp_dir().join(format!("harness-tui-finish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let crate_dir = base.join("crate");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        for f in handedit::EDIT_FILES {
            std::fs::write(crate_dir.join(f), "pub fn f() {}\n").unwrap();
        }
        let session = handedit::prepare(&crate_dir, &base).unwrap();
        std::fs::write(&session.files[0], "pub fn f() { /* edited */ }\n").unwrap();
        let case = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib")
            .canonicalize()
            .unwrap();
        let mut app = App::new(
            Config {
                target: case.clone(),
                harness: None,
                allow_unsandboxed: false,
                layout: LayoutMode::Auto,
                providers: vec!["external".into()],
            },
            load::read(&case).unwrap(),
        );
        let edit = session.tmp.join("edit");
        let mut kept_at_resume = None;
        let result = finish_edit(
            &mut app,
            "u-lib",
            &session,
            Ok(ExitStatus::from_raw(0)),
            None,
            || {
                kept_at_resume = Some(guard(&KEPT_EDITS).contains(&edit));
                Ok(())
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            kept_at_resume,
            Some(true),
            "the edit was not kept before resume"
        );
        assert!(guard(&KEPT_EDITS).contains(&edit));
        let _ = std::fs::remove_dir_all(&base);
    }
}
