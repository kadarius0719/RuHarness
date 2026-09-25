//! Confirmation dialogs and their arming latch (docs/COCKPIT-WRAPPER-DESIGN.md
//! §5): no act, quit or cancel happens without an ARMED dialog.
//!
//! A dialog arms once three things hold — it was drawn whole (its argv seen
//! to the end), at least [`ARM_QUIET`] passed since the last input event was
//! READ, and no input is pending at that moment — and then stays armed (a
//! latch). Before it arms, `Esc`, `n` and `Ctrl-C` take the safe choice,
//! scroll keys scroll (and restart the wait), and every other key is DROPPED
//! — never queued — and restarts the wait, with visible feedback. Focus
//! starts on the first button, the safe one. After arming, a button's letter
//! acts, or a focus move (`←`, `→`, `Tab`) followed by `Enter` activates the
//! focused button; `Enter` on the safe button takes the safe choice. So a
//! held `Enter` (auto-repeat arrives as `Press`) can never run anything: its
//! repeats keep restarting the wait, and once armed it sits on the safe
//! button. The clock and the pending-input flag are passed in: the logic is
//! tested without a terminal.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// The quiet time a dialog needs before it arms.
pub const ARM_QUIET: Duration = Duration::from_millis(300);

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The safe choice: Cancel, Keep for later, Stay, Keep running.
    Safe,
    /// Run the act (`y`).
    Run,
    /// A hand edit's override: record it (`y`).
    Record,
    /// A hand edit: discard it (`D`).
    Discard,
    /// Quit and let the running command finish (`q`).
    QuitLeave,
    /// Stop the running command and quit (`x`).
    QuitStop,
    /// Stop the running command (`x`).
    Stop,
}

/// One button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// Its words.
    pub label: &'static str,
    /// The key shown beside it (`Esc`, `y`, …).
    pub key: &'static str,
    /// The letters that press it once armed (none for the safe button,
    /// which `Esc`/`n` take at any time).
    pub letters: &'static [char],
    /// What it does.
    pub choice: Choice,
}

const fn button(
    label: &'static str,
    key: &'static str,
    letters: &'static [char],
    choice: Choice,
) -> Button {
    Button {
        label,
        key,
        letters,
        choice,
    }
}

/// The kinds of dialog, each with its buttons (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An act: Cancel, Run.
    Act,
    /// A staged hand edit's override: Keep for later, Record, Discard.
    Override,
    /// Quit while a command runs: Stay, Quit (let it finish), Stop it and quit.
    Quit,
    /// Cancel the running command: Keep running, Stop it.
    Cancel,
}

impl Kind {
    /// Its buttons, the safe one first.
    pub fn buttons(self) -> Vec<Button> {
        match self {
            Kind::Act => vec![
                button("Cancel", "Esc", &[], Choice::Safe),
                button("Run", "y", &['y', 'Y'], Choice::Run),
            ],
            Kind::Override => vec![
                button("Keep for later", "Esc", &[], Choice::Safe),
                button("Record", "y", &['y', 'Y'], Choice::Record),
                button("Discard", "D", &['D'], Choice::Discard),
            ],
            Kind::Quit => vec![
                button("Stay", "Esc", &[], Choice::Safe),
                button("Quit, let it finish", "q", &['q', 'Q'], Choice::QuitLeave),
                button("Stop it and quit", "x", &['x'], Choice::QuitStop),
            ],
            Kind::Cancel => vec![
                button("Keep running", "Esc", &[], Choice::Safe),
                button("Stop it", "x", &['x'], Choice::Stop),
            ],
        }
    }
}

/// What a key did to a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It stays open.
    Stay,
    /// It closes with this choice.
    Close(Choice),
}

/// A dialog's state (its words live with its owner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// Which dialog.
    pub kind: Kind,
    /// Its buttons, the safe one first.
    pub buttons: Vec<Button>,
    /// The focused button (starts on the safe one).
    pub focus: usize,
    /// First row shown (a long argv scrolls).
    pub scroll: usize,
    /// Drawn whole, its argv seen to the end (set by the view).
    pub seen: bool,
    /// The latch: once armed, it stays armed.
    pub armed: bool,
    /// When the last input event was read (the quiet time runs from here).
    pub quiet_since: Instant,
    /// A key was dropped since the last draw: "Too soon — wait for ready".
    pub too_soon: bool,
}

impl Dialog {
    /// A fresh dialog, opened by an input read at `now`.
    pub fn new(kind: Kind, now: Instant) -> Dialog {
        Dialog {
            kind,
            buttons: kind.buttons(),
            focus: 0,
            scroll: 0,
            seen: false,
            armed: false,
            quiet_since: now,
            too_soon: false,
        }
    }

    /// An input event was read at `now` (a key, a paste, a resize, the
    /// mouse): the quiet time starts again (until armed).
    pub fn input(&mut self, now: Instant) {
        if !self.armed {
            self.quiet_since = now;
        }
    }

    /// The event loop's check after a draw: arm when drawn whole, quiet for
    /// [`ARM_QUIET`], with no input `pending`. Returns whether it is armed.
    pub fn arm(&mut self, now: Instant, pending: bool) -> bool {
        if !self.armed
            && self.seen
            && !pending
            && now.saturating_duration_since(self.quiet_since) >= ARM_QUIET
        {
            self.armed = true;
            self.too_soon = false;
        }
        self.armed
    }

    /// Whether `arm` could still change something (the loop polls for
    /// pending input only then).
    pub fn waiting(&self) -> bool {
        !self.armed && self.seen
    }

    /// One key press, read at `now` (the caller already reported it through
    /// [`Dialog::input`] or not — this restarts the quiet time itself).
    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Outcome {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        self.input(now);
        // The safe choice is always one key away.
        if ctrl_c
            || key.code == KeyCode::Esc
            || (plain && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N')))
        {
            return Outcome::Close(Choice::Safe);
        }
        let scroll = match key.code {
            KeyCode::Up | KeyCode::Char('k') if plain => Some(-1),
            KeyCode::Down | KeyCode::Char('j') if plain => Some(1),
            KeyCode::PageUp => Some(-10),
            KeyCode::PageDown | KeyCode::Char(' ') => Some(10),
            KeyCode::Home => Some(isize::MIN / 2),
            KeyCode::End => Some(isize::MAX / 2),
            _ => None,
        };
        if let Some(by) = scroll {
            // The view clamps it to what the rows need.
            self.scroll = self.scroll.saturating_add_signed(by);
            return Outcome::Stay;
        }
        if !self.armed {
            self.too_soon = true;
            return Outcome::Stay;
        }
        let n = self.buttons.len();
        match key.code {
            KeyCode::Right | KeyCode::Tab => self.focus = (self.focus + 1) % n,
            KeyCode::Left | KeyCode::BackTab => self.focus = (self.focus + n - 1) % n,
            KeyCode::Enter => return Outcome::Close(self.buttons[self.focus].choice),
            KeyCode::Char(c) if plain => {
                if let Some(b) = self.buttons.iter().find(|b| b.letters.contains(&c)) {
                    return Outcome::Close(b.choice);
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// The status the dialog shows beside its buttons.
    pub fn state_text(&self) -> String {
        if self.armed {
            let letter = self.buttons.get(1).map_or("y", |b| b.key);
            format!("ready: → then Enter, or {letter}")
        } else if self.too_soon {
            "Too soon — wait for ready".into()
        } else if !self.seen {
            "↓ more below — scroll to the end".into()
        } else {
            "reading…".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: KeyCode) -> KeyEvent {
        KeyEvent::from(c)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A dialog drawn whole at `t0`, as the view leaves it.
    fn drawn(kind: Kind, t0: Instant) -> Dialog {
        let mut d = Dialog::new(kind, t0);
        d.seen = true;
        d
    }

    /// Mutation-checked rule: the latch drops early input. A key before
    /// arming is dropped, says so, and restarts the wait.
    #[test]
    fn a_key_before_arming_is_dropped_and_restarts_the_wait() {
        let t0 = Instant::now();
        let mut d = drawn(Kind::Act, t0);
        assert!(!d.arm(t0 + ms(299), false), "not before 300 ms");
        assert_eq!(
            d.on_key(press(KeyCode::Char('y')), t0 + ms(250)),
            Outcome::Stay
        );
        assert!(d.too_soon);
        assert_eq!(d.state_text(), "Too soon — wait for ready");
        assert!(!d.arm(t0 + ms(400), false), "the wait restarted at 250 ms");
        assert!(d.arm(t0 + ms(550), false));
        assert_eq!(
            d.on_key(press(KeyCode::Char('y')), t0 + ms(600)),
            Outcome::Close(Choice::Run)
        );
    }

    /// Pending input blocks arming; an unseen argv blocks arming; once
    /// armed, the latch holds whatever comes.
    #[test]
    fn pending_input_and_an_unseen_argv_block_arming_and_the_latch_holds() {
        let t0 = Instant::now();
        let mut d = Dialog::new(Kind::Act, t0);
        assert!(!d.arm(t0 + ms(1000), false), "not seen to the end");
        assert_eq!(d.state_text(), "↓ more below — scroll to the end");
        d.seen = true;
        assert!(!d.arm(t0 + ms(1000), true), "input pending");
        assert!(d.arm(t0 + ms(1000), false));
        d.input(t0 + ms(1001));
        assert!(d.arm(t0 + ms(1002), true), "a latch");
        assert_eq!(
            d.on_key(press(KeyCode::Char('z')), t0 + ms(1003)),
            Outcome::Stay
        );
        assert!(d.armed);
    }

    /// Mutation-checked rule: a held `Enter` never runs anything — one
    /// press, 600 ms of silence (the OS repeat delay), then repeats every
    /// 30 ms, the loop arming between reads as it does.
    #[test]
    fn a_held_enter_never_runs_anything() {
        for kind in [Kind::Act, Kind::Override, Kind::Quit, Kind::Cancel] {
            let t0 = Instant::now();
            // The press that opened the dialog was read at t0.
            let mut d = drawn(kind, t0);
            let mut t = t0;
            let mut outcome = Outcome::Stay;
            for i in 0..40 {
                t = if i == 0 { t0 + ms(600) } else { t + ms(30) };
                d.arm(t - ms(1), false);
                outcome = d.on_key(press(KeyCode::Enter), t);
                if outcome != Outcome::Stay {
                    break;
                }
            }
            assert_eq!(outcome, Outcome::Close(Choice::Safe), "{kind:?}");
        }
        // Held from before arming: its repeats keep restarting the wait.
        let t0 = Instant::now();
        let mut d = drawn(Kind::Act, t0);
        let mut t = t0;
        for _ in 0..100 {
            t += ms(30);
            assert!(!d.arm(t, false));
            assert_eq!(d.on_key(press(KeyCode::Enter), t), Outcome::Stay);
        }
    }

    /// After arming: `Enter` on the safe button is the safe choice; a move
    /// plus `Enter`, or the letter, runs.
    #[test]
    fn after_arming_a_move_and_enter_or_the_letter_acts() {
        let t0 = Instant::now();
        let armed = |kind| {
            let mut d = drawn(kind, t0);
            assert!(d.arm(t0 + ms(300), false));
            d
        };
        let mut d = armed(Kind::Act);
        assert_eq!(
            d.on_key(press(KeyCode::Enter), t0),
            Outcome::Close(Choice::Safe)
        );
        let mut d2 = armed(Kind::Act);
        d2.on_key(press(KeyCode::Right), t0);
        assert_eq!(
            d2.on_key(press(KeyCode::Enter), t0),
            Outcome::Close(Choice::Run)
        );
        let _ = &mut d;
        let mut d = armed(Kind::Override);
        d.on_key(press(KeyCode::Left), t0);
        assert_eq!(
            d.on_key(press(KeyCode::Enter), t0),
            Outcome::Close(Choice::Discard)
        );
        let mut d = armed(Kind::Override);
        assert_eq!(
            d.on_key(press(KeyCode::Char('D')), t0),
            Outcome::Close(Choice::Discard)
        );
        let mut d = armed(Kind::Quit);
        assert_eq!(
            d.on_key(press(KeyCode::Char('q')), t0),
            Outcome::Close(Choice::QuitLeave)
        );
        let mut d = armed(Kind::Quit);
        assert_eq!(
            d.on_key(press(KeyCode::Char('x')), t0),
            Outcome::Close(Choice::QuitStop)
        );
        let mut d = armed(Kind::Cancel);
        assert_eq!(
            d.on_key(press(KeyCode::Char('x')), t0),
            Outcome::Close(Choice::Stop)
        );
        // A letter of another dialog does nothing; Ctrl-Y is not y.
        let mut d = armed(Kind::Cancel);
        assert_eq!(d.on_key(press(KeyCode::Char('y')), t0), Outcome::Stay);
        let mut d = armed(Kind::Act);
        let ctrl_y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(d.on_key(ctrl_y, t0), Outcome::Stay);
    }

    /// `D`, `q` and `x` need arming inside their dialogs; `Esc`, `n` and
    /// `Ctrl-C` take the safe choice at any time.
    #[test]
    fn every_non_safe_letter_needs_arming_and_the_safe_choice_never_does() {
        let t0 = Instant::now();
        for (kind, letter) in [
            (Kind::Act, 'y'),
            (Kind::Override, 'D'),
            (Kind::Override, 'y'),
            (Kind::Quit, 'q'),
            (Kind::Quit, 'x'),
            (Kind::Cancel, 'x'),
        ] {
            let mut d = drawn(kind, t0);
            assert_eq!(
                d.on_key(press(KeyCode::Char(letter)), t0 + ms(100)),
                Outcome::Stay,
                "{kind:?} {letter}"
            );
            assert!(!d.armed);
        }
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for key in [press(KeyCode::Esc), press(KeyCode::Char('n')), ctrl_c] {
            let mut d = Dialog::new(Kind::Act, t0);
            assert_eq!(d.on_key(key, t0), Outcome::Close(Choice::Safe));
        }
    }

    /// Scroll keys scroll before arming, and restart the wait: the argv must
    /// be seen to its end.
    #[test]
    fn scroll_keys_scroll_and_restart_the_wait() {
        let t0 = Instant::now();
        let mut d = drawn(Kind::Act, t0);
        assert_eq!(d.on_key(press(KeyCode::Down), t0 + ms(200)), Outcome::Stay);
        assert_eq!(d.scroll, 1);
        assert!(!d.too_soon, "a scroll is not dropped");
        assert!(!d.arm(t0 + ms(400), false));
        assert!(d.arm(t0 + ms(500), false));
        assert_eq!(d.state_text(), "ready: → then Enter, or y");
    }
}
