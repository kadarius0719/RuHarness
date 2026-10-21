//! A model proposes the features and the workloads
//! (docs/COCKPIT-WRAPPER-DESIGN.md §9.6): the chat writes a proposal
//! beside the person's file through harness-mcp
//! (`harness_features_propose`, `harness_workloads_propose`); the cockpit
//! opens it in the person's editor as their draft, and their save writes
//! the file. Writing stays the person's; proposing is the model's.

use super::{notice, App};

impl App {
    /// Tell the chat which of the person's drafts are kept, before it
    /// reads what the runtime printed: a proposal of that file is refused
    /// while one is (the draft slot is theirs).
    pub(crate) fn sync_drafts_kept(&mut self) {
        self.chat.drafts_kept = [
            self.features_draft.is_some(),
            self.workloads_draft.is_some(),
        ];
    }

    /// The chat's proposal is written: said, with the item that opens it.
    pub(crate) fn chat_proposal_written(&mut self, workloads: bool) {
        let noun = if workloads { "workloads" } else { "features" };
        self.notice = notice(format!(
            "the chat's proposal of your {noun} file is written — the {noun} row's menu opens it \
             as your draft"
        ));
    }
}
