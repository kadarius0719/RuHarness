//! The chat's connection, and the Project view's model work asked in chat
//! (docs/CHAT-PANE-DESIGN.md §1.6, §3.6):
//!
//! - before the chat starts (an "ask in chat" item, or the first message
//!   typed) the cockpit checks Claude Code is installed (`claude --version`);
//!   the sign-in shows when it starts — a "not signed in" from it, or a
//!   Claude Code that stops by itself, opens the same walk-through: what
//!   failed in plain words, the four steps, `[ Check again ]`;
//! - while the chat is unavailable its items are greyed with that reason,
//!   "Check the chat's connection" opens the walk-through again, and the
//!   Next step never recommends a chat item;
//! - the Project view's "Ask for advice — ask in chat" and "Ask for a
//!   proposal — ask in chat" type the question into the chat, as "Migrate —
//!   ask in chat" does for a unit; the chat answers through harness-mcp's
//!   map tools, and the advice it records shows on the Project view and in
//!   Accept's picker.

use super::{notice, App, Focus, Mode, Purpose};
use crate::chat::{runtime, Event as ChatEvent, Trouble};
use crate::menu::Action;
use crate::next::Advice;
use crate::tree::Selection;
use harness_core::text::safe_line;
use std::time::Duration;

/// The question typed for a proposal of the build configuration.
pub const PROPOSAL_QUESTION: &str =
    "Propose the build configuration for this project from its Makefiles";

impl App {
    /// Why the chat's items cannot run now, in words — `None` when they
    /// can (the connection is checked when one is chosen).
    pub fn chat_unavailable(&self) -> Option<String> {
        if !self.chat_on {
            return Some("the chat is off (--no-chat)".into());
        }
        if let Some(p) = &self.chat.problem {
            return Some(p.reason());
        }
        self.chat.bins.as_ref().err().cloned()
    }

    /// The check before the chat starts: `true` when it may be used; else
    /// the walk-through opens (or, for a harness-mcp that is missing, the
    /// reason is said) and nothing else happens.
    pub(crate) fn chat_connected(&mut self) -> bool {
        if !self.chat_on {
            self.say("the chat is off (--no-chat)");
            return false;
        }
        if self.chat.check().is_some() {
            self.open_dialog(Purpose::Connection);
            return false;
        }
        if let Err(why) = &self.chat.bins {
            let why = why.clone();
            self.say(why);
            return false;
        }
        true
    }

    /// The chat said it is not signed in, or stopped by itself: the
    /// walk-through, unless a dialog or a prompt is open (then the notice
    /// says so, and the greyed items lead back to it).
    pub(crate) fn chat_trouble_seen(&mut self) {
        let Some(p) = self.chat.problem.clone() else {
            return;
        };
        if matches!(self.mode, Mode::Normal | Mode::Menu(_)) {
            self.open_dialog(Purpose::Connection);
        } else {
            self.notice = notice(format!(
                "{} — choose Check the chat's connection to see how to fix it",
                p.title()
            ));
        }
    }

    /// Check again: the binaries found again and `claude --version` run;
    /// after a failed sign-in or a stop, the stopped chat ended and the
    /// person's last message sent again — the sign-in shows only when
    /// Claude Code starts.
    pub(crate) fn chat_check_again(&mut self) {
        let now = self.now;
        let resend = self
            .chat
            .problem
            .as_ref()
            .is_some_and(|p| p.trouble != Trouble::NotInstalled);
        if self.chat.alive() {
            if let Some(ChatEvent::Ended { gen }) =
                self.chat
                    .end(now, Duration::ZERO, runtime::BOUNDED_KILL_AFTER)
            {
                self.chat_gen_ended(gen, now);
            }
        }
        if self.chat.check_again().is_some() {
            self.open_dialog(Purpose::Connection);
            return;
        }
        if let Err(why) = &self.chat.bins {
            let why = why.clone();
            self.say(why);
            return;
        }
        match self.chat.last_sent.clone() {
            Some(text) if resend && self.chat.input.is_empty() => {
                self.chat.input.insert(&text);
                self.move_focus(Focus::Chat);
                self.chat_send(now);
                self.notice = notice(
                    "Claude Code is installed — your last message is sent again to check the \
                     sign-in",
                );
            }
            _ => {
                self.notice = notice(
                    "Claude Code is installed — the sign-in is checked when the chat starts: \
                     send your message",
                );
            }
        }
    }

    /// `harness_map_answer` recorded the chat's answer: the advice read
    /// again for the Project view and Accept's picker.
    pub(crate) fn chat_map_answered(&mut self) {
        if self.in_project_view() {
            self.reload_project();
        } else if let Some(p) = self.project.as_mut() {
            let fresh = super::project_view::ProjectState::read(&self.config.target);
            p.advice = fresh.advice;
            p.labels = fresh.labels;
        } else {
            return;
        }
        self.notice = notice(
            "the chat's answer is recorded — its advice shows beside each choice when you Accept \
             a program; the choice stays yours",
        );
    }

    /// The question the Project view's ask-in-chat item types: a proposal
    /// of the build configuration, or which file a program's held choices
    /// keep (the selected program's, else every one).
    pub fn project_chat_question(&self, build: bool) -> String {
        if build {
            return PROPOSAL_QUESTION.to_string();
        }
        if let Selection::Program(id) = &self.selection {
            if let Some(o) = self.project_offered(id) {
                let sets: Vec<String> = o.held.iter().map(|h| safe_line(&h.set)).collect();
                let id = safe_line(id);
                match sets.as_slice() {
                    [] => {}
                    [one] => {
                        return format!("Which file should {id} keep for choice {one}, and why?")
                    }
                    [rest @ .., last] => {
                        return format!(
                            "Which file should {id} keep for choices {} and {last}, and why?",
                            rest.join(", ")
                        )
                    }
                }
            }
        }
        "Which file should each held choice keep, and why?".to_string()
    }

    /// The Project view's "Ask for advice — ask in chat" or "Ask for a
    /// proposal — ask in chat": the question typed into the input (never
    /// over a draft), the chat focused; the person reads it and sends it.
    pub(crate) fn chat_ask_project(&mut self, build: bool) {
        if !self.chat.input.is_empty() {
            self.say("the chat has a draft — send or clear it first");
            self.move_focus(Focus::Chat);
            return;
        }
        let text = self.project_chat_question(build);
        self.chat.input.insert(&text);
        self.move_focus(Focus::Chat);
        self.notice = notice(
            "the question is typed into the chat — press Enter to send it; the chat reads the \
             map through the harness and records its answer as advice",
        );
    }

    /// A Next step that recommends a chat item while the chat is
    /// unavailable says why instead, and leads to the walk-through.
    pub(crate) fn unless_chat_down(&self, a: Advice) -> Advice {
        // `--no-chat` is the person's own choice: its items stay greyed
        // with it, and the Next step's words are unchanged.
        let chat_item = self.chat_on
            && matches!(
                a.action,
                Some(Action::Migrate | Action::AskChat | Action::AskInChat { .. })
            );
        match self.chat_unavailable() {
            Some(why) if chat_item => Advice {
                text: format!(
                    "{} — the chat does that, and it is unavailable: {why}",
                    crate::menu::reason_of(&a.text)
                ),
                action: self.chat.problem.is_some().then_some(Action::CheckChat),
            },
            _ => a,
        }
    }
}
