//! The adoption question offered again inside the cockpit: a command that
//! failed because the folder's results were made elsewhere (a copy, another
//! checkout, a record that could not be written here) is no dead end — the
//! cockpit asks the same question it asks at the start
//! ([`crate::adoption`]), in its own armed dialog, and adopts on the
//! person's yes (docs/PROJECT-MAP-DESIGN.md §3.7).

use super::{notice, App, Confirm, Mode, Purpose};
use crate::dialog::{Choice, Dialog, Kind};

impl App {
    /// After a command that did not succeed: when the folder is now refused
    /// as made elsewhere, remember it (the menus offer Adopt this folder)
    /// and ask at once when nothing else is open.
    pub(super) fn offer_adoption_after_failure(&mut self) {
        let refused = match harness_core::adopt::check(&self.config.target) {
            Err(harness_core::Error::NotAdopted {
                root,
                units,
                verified,
            }) => harness_core::adopt::made_elsewhere(&root, units, verified),
            _ => return,
        };
        self.not_adopted = Some(refused);
        if matches!(self.mode, Mode::Normal) {
            self.ask_adopt();
        }
    }

    /// The adoption dialog: the start's three plain lines; what adopting
    /// does in full and the folder's claim are its card (`i`).
    pub fn ask_adopt(&mut self) {
        if self.not_adopted.is_none() {
            self.notice = notice("this folder needs no adoption");
            return;
        }
        if self.running {
            self.notice = notice("a command is running — adopt the folder when it is done");
            return;
        }
        // Three plain lines, as at the start; the rest, with the folder's
        // claim, is its card, `i` (the whole-journey check, item 11).
        let body = crate::adoption::short_words();
        let mut dialog = Dialog::new(Kind::Act, self.now);
        dialog.chat_rules = false;
        if let Some(b) = dialog.buttons.iter_mut().find(|b| b.choice == Choice::Run) {
            b.label = "Adopt".into();
        }
        self.mode = Mode::Dialog(Box::new(Confirm {
            dialog,
            title: "Adopt this folder?".into(),
            body,
            purpose: Purpose::Adopt,
        }));
    }

    /// The adoption dialog closed: adopted on Adopt, the screen read again;
    /// else said how to come back to it.
    pub(super) fn close_adopt_dialog(&mut self, choice: Choice) {
        if choice != Choice::Run {
            self.notice = notice(
                "not adopted; nothing was changed — Enter's menu offers Adopt this folder when \
                 you want to",
            );
            return;
        }
        match harness_core::adopt::adopt(&self.config.target) {
            Ok(done) => {
                self.not_adopted = None;
                // Read again (the Project view at once), then said.
                self.request_load(super::LoadWhy::Reaped);
                self.notice = notice(done.describe(self.config.tool.as_deref()).join("; "));
            }
            Err(e) => self.notice = notice(format!("not adopted: {e}")),
        }
    }
}
