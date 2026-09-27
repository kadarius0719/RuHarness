//! The terminal guard (docs/COCKPIT-WRAPPER-DESIGN.md §11, review SAFE-10 and
//! CHK-5): the paths that restore the terminal never block.
//!
//! - Every enable (the initial setup, `resume` after the editor, the mouse in
//!   Build B) runs under the guard's mutex, and does nothing once the cockpit
//!   is dying.
//! - The signal path marks the cockpit dying, restores unconditionally (the
//!   restore is idempotent), waits a BOUNDED time for an enable in flight
//!   (`try_lock` in a loop — an enable stuck in a blocked write never holds
//!   it up), and restores once more before dying: a restore can no longer be
//!   undone by an enable that raced it.
//! - The panic hook marks dying and restores, never taking the mutex: a panic
//!   inside an enable, which holds it, cannot deadlock the hook.
//!
//! The mouse (docs/COCKPIT-WRAPPER-DESIGN.md §7) is one more enable:
//! [`EnableMouse`] under the guard; crossterm's `DisableMouseCapture` (every
//! mode reset) on every way out.

use ratatui::crossterm;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, TryLockError};
use std::time::{Duration, Instant};

/// How long the signal path waits for an enable in flight.
pub const ENABLE_WAIT: Duration = Duration::from_millis(200);

/// The mouse modes the cockpit uses: button presses and releases (and the
/// wheel) only, with SGR coordinates — `?1000h ?1006h`. Not crossterm's
/// `EnableMouseCapture`, which also sets `?1003h` (every movement: a flood
/// of events through the loop, each restarting a dialog's quiet time) and
/// `?1002h`/`?1015h` — which are reset first, in case a program before the
/// cockpit (an editor that crashed) left them on (review PROC-B-3). Its
/// reset is crossterm's `DisableMouseCapture`, which resets all five.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnableMouse;

impl crossterm::Command for EnableMouse {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1003l\x1b[?1002l\x1b[?1015l\x1b[?1000h\x1b[?1006h")
    }
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct TermGuard {
    dying: AtomicBool,
    enabling: Mutex<()>,
}

impl TermGuard {
    /// A guard for a live cockpit.
    pub const fn new() -> TermGuard {
        TermGuard {
            dying: AtomicBool::new(false),
            enabling: Mutex::new(()),
        }
    }

    /// The cockpit is on its way out: no enable runs any more.
    pub fn dying(&self) -> bool {
        self.dying.load(Ordering::SeqCst)
    }

    /// Mark the cockpit dying (no enable starts after this returns).
    pub fn mark_dying(&self) {
        self.dying.store(true, Ordering::SeqCst);
    }

    /// Run `enable` under the guard, unless the cockpit is dying (`None`:
    /// the caller must not touch the terminal again).
    pub fn enable<R>(&self, enable: impl FnOnce() -> R) -> Option<R> {
        let _held = self.enabling.lock().unwrap_or_else(PoisonError::into_inner);
        if self.dying() {
            return None;
        }
        Some(enable())
    }

    /// The signal path: dying, `restore`, wait at most `wait` for an enable
    /// in flight, `restore` again. Returns within `wait` plus two restores.
    pub fn restore_for_death(&self, restore: impl Fn(), wait: Duration) {
        self.mark_dying();
        restore();
        let deadline = Instant::now() + wait;
        loop {
            match self.enabling.try_lock() {
                Ok(_) | Err(TryLockError::Poisoned(_)) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => break,
            }
        }
        restore();
    }

    /// The panic hook: dying, `restore` — never the mutex.
    pub fn restore_on_panic(&self, restore: impl Fn()) {
        self.mark_dying();
        restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Arc};

    fn ansi(command: impl crossterm::Command) -> String {
        let mut out = String::new();
        command.write_ansi(&mut out).unwrap();
        out
    }

    /// §7: the mouse is button tracking with SGR coordinates only — never
    /// every movement — and crossterm's reset turns off each mode it sets.
    #[test]
    fn the_mouse_modes_and_their_reset() {
        let on = ansi(EnableMouse);
        assert_eq!(
            on, "\x1b[?1003l\x1b[?1002l\x1b[?1015l\x1b[?1000h\x1b[?1006h",
            "every movement, drags and urxvt coordinates off; buttons and SGR on"
        );
        let off = ansi(crossterm::event::DisableMouseCapture);
        for mode in ["1000", "1006", "1002", "1003", "1015"] {
            assert!(off.contains(&format!("\x1b[?{mode}l")), "{off:?}");
        }
    }

    /// §13 Terminal guard: a signal while an enable is blocked (a write the
    /// terminal never takes) still restores, twice, within the bound — and
    /// the enable that finishes afterwards cannot undo it, nor can a later one
    /// start.
    #[test]
    fn a_signal_during_a_blocked_enable_still_restores_and_returns() {
        let guard = Arc::new(TermGuard::new());
        let restores = Arc::new(AtomicUsize::new(0));
        let (started_tx, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel::<()>();
        let g = guard.clone();
        let enabler = std::thread::spawn(move || {
            g.enable(|| {
                started_tx.send(()).unwrap();
                let _ = blocked.recv();
            })
        });
        started.recv().unwrap();
        let start = Instant::now();
        let r = restores.clone();
        guard.restore_for_death(
            || {
                r.fetch_add(1, Ordering::SeqCst);
            },
            ENABLE_WAIT,
        );
        let took = start.elapsed();
        assert!(took >= ENABLE_WAIT, "it waited for the enable: {took:?}");
        assert!(took < Duration::from_secs(2), "it never blocks: {took:?}");
        assert_eq!(restores.load(Ordering::SeqCst), 2);
        release.send(()).unwrap();
        assert!(enabler.join().unwrap().is_some());
        // Dying: no enable runs any more.
        assert!(guard.enable(|| ()).is_none());
    }

    /// An enable that finishes within the bound is waited for: the second
    /// restore comes after it.
    #[test]
    fn the_second_restore_comes_after_an_enable_in_flight() {
        let guard = Arc::new(TermGuard::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let (started_tx, started) = mpsc::channel();
        let (g, o) = (guard.clone(), order.clone());
        let enabler = std::thread::spawn(move || {
            g.enable(|| {
                started_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                o.lock().unwrap().push("enable");
            })
        });
        started.recv().unwrap();
        let o = order.clone();
        guard.restore_for_death(move || o.lock().unwrap().push("restore"), ENABLE_WAIT);
        enabler.join().unwrap();
        assert_eq!(*order.lock().unwrap(), ["restore", "enable", "restore"]);
    }

    /// §13 Terminal guard: a panic inside an enable (which holds the mutex)
    /// does not hang the hook, nor a later signal.
    #[test]
    fn a_panic_during_an_enable_does_not_hang() {
        let guard = Arc::new(TermGuard::new());
        let restores = Arc::new(AtomicUsize::new(0));
        let (g, r) = (guard.clone(), restores.clone());
        let panicked = std::thread::spawn(move || {
            g.enable(|| {
                // What the panic hook does, while the enable holds the mutex.
                let r = r.clone();
                g.restore_on_panic(move || {
                    r.fetch_add(1, Ordering::SeqCst);
                });
                panic!("inside resume");
            })
        })
        .join();
        assert!(panicked.is_err());
        assert_eq!(restores.load(Ordering::SeqCst), 1);
        let start = Instant::now();
        let r = restores.clone();
        guard.restore_for_death(
            move || {
                r.fetch_add(1, Ordering::SeqCst);
            },
            ENABLE_WAIT,
        );
        assert!(
            start.elapsed() < ENABLE_WAIT,
            "a poisoned mutex is not waited on"
        );
        assert_eq!(restores.load(Ordering::SeqCst), 3);
        assert!(guard.enable(|| ()).is_none());
    }
}
