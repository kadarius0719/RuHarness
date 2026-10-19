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

    /// The adoption dialog: one plain sentence first, then what adopting
    /// does — the start's own words — and the folder's claim.
    pub fn ask_adopt(&mut self) {
        let Some(refused) = self.not_adopted.clone() else {
            self.notice = notice("this folder needs no adoption");
            return;
        };
        if self.running {
            self.notice = notice("a command is running — adopt the folder when it is done");
            return;
        }
        let file = harness_core::adopt::adoption_file()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| e.to_string());
        let mut body = vec![crate::adoption::LEAD.to_string()];
        // The explanation's lines, joined back into paragraphs: one per
        // dash item.
        let mut paragraph = String::new();
        for line in crate::adoption::explanation(&file).lines() {
            let line = line.trim();
            if line.starts_with("- ") && !paragraph.is_empty() {
                body.push(std::mem::take(&mut paragraph));
            }
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
        }
        if !paragraph.is_empty() {
            body.push(paragraph);
        }
        body.push(format!("The refusal: {refused}."));
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
