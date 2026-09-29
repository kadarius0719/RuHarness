//! Writing the person's features file from the cockpit
//! (docs/FEATURES-DESIGN.md §7.2): the file (or the starter) is copied into
//! a private draft, a dialog says which editor opens and how to leave it,
//! the draft is validated on return, and a confirmed `harness features save`
//! — the text on its stdin, `--expect` the bytes the edit started from —
//! writes it. The cockpit never writes the ledger itself; the draft is kept
//! (named on quit) until it is saved or discarded.

use super::{notice, Act, App, Command, Mode, Pending, Purpose};
use crate::dialog::{Choice, Dialog, Kind};
use crate::handedit;
use harness_core::features;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A features draft being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeaturesDraft {
    /// Its private temp dir (0700).
    pub tmp: PathBuf,
    /// `<tmp>/features.toml`, the file the editor opens.
    pub file: PathBuf,
    /// What `--expect` names: the blake3 of the file's bytes when the edit
    /// started, or `none` when there was no file.
    pub expect: String,
    /// The draft's text when it was last handed to the editor.
    pub before: String,
    /// The line of the last validation error, for the editor's `+N`.
    pub error_line: Option<u32>,
    /// When the editor was started.
    pub started: Option<Instant>,
}

/// Editors that take `+N` to open at a line.
const LINE_EDITORS: [&str; 7] = ["nano", "pico", "vi", "vim", "nvim", "emacs", "micro"];

/// The editor the features Edit opens: `$VISUAL`, else `$EDITOR`, else
/// `nano` when it is on `PATH`, else `vi` (the hand edit keeps its own
/// default: it opens two files).
pub fn features_editor(visual: Option<OsString>, editor: Option<OsString>) -> String {
    let chosen = [visual, editor]
        .into_iter()
        .flatten()
        .map(|v| v.to_string_lossy().into_owned())
        .find(|v| !v.trim().is_empty());
    chosen.unwrap_or_else(|| {
        if on_path("nano") {
            "nano".into()
        } else {
            "vi".into()
        }
    })
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|dir| std::fs::metadata(dir.join(name)).is_ok_and(|m| m.is_file()))
    })
}

/// The editor's name as the person reads it: the basename of the command's
/// first word.
pub fn editor_name(editor: &str) -> String {
    let trimmed = editor.trim_start();
    let first = match trimmed.chars().next() {
        Some(q @ ('"' | '\'')) => trimmed[1..].split(q).next().unwrap_or(""),
        _ => trimmed.split_whitespace().next().unwrap_or(trimmed),
    };
    Path::new(first)
        .file_name()
        .map_or_else(|| first.to_string(), |n| n.to_string_lossy().into_owned())
}

/// How to use the editor, in words (§7.2 step 3).
pub fn editor_instructions(name: &str) -> String {
    match name {
        "nano" | "pico" => "Type your changes. Ctrl-O then Enter saves; Ctrl-X leaves.".into(),
        "vi" | "vim" | "nvim" => "Press i to type. To save and leave: Esc, then :wq, then Enter. \
                                  To leave without saving: Esc, then :q!, then Enter."
            .into(),
        _ => "Save and close the file to come back here.".into(),
    }
}

/// The argv of the editor on `file`, at `line` when it takes `+N`.
pub fn editor_argv(editor: &str, file: &Path, line: Option<u32>) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from("-c"),
        OsString::from(handedit::editor_script(editor)),
        OsString::from("sh"),
    ];
    if let Some(n) = line.filter(|_| LINE_EDITORS.contains(&editor_name(editor).as_str())) {
        argv.push(OsString::from(format!("+{n}")));
    }
    argv.push(file.as_os_str().to_owned());
    argv
}

/// The line a loader message names (`… at line N, column M …`).
fn error_line(message: &str) -> Option<u32> {
    let rest = message.split("at line ").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

impl App {
    /// The editor this session's features Edit opens.
    pub fn features_editor(&self) -> String {
        features_editor(std::env::var_os("VISUAL"), std::env::var_os("EDITOR"))
    }

    /// Menu: Write / Edit the features file, or continue the kept draft —
    /// the dialog that says which editor opens and how to leave it.
    pub fn start_features_edit(&mut self) -> Command {
        if self.running {
            self.notice = notice("a command is running — edit the features file when it is done");
            return Command::None;
        }
        let root = self.config.target.clone();
        let dir = features::features_dir(&root);
        let path = features::features_path(&root);
        for p in [&dir, &path] {
            if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                self.notice = notice(format!(
                    "{} is a symlink — replace it with a regular file outside the cockpit",
                    p.strip_prefix(&root).unwrap_or(p).display()
                ));
                return Command::None;
            }
        }
        if self.features_draft.is_none() {
            let (text, expect) =
                match harness_core::ledger::read_regular(&path, features::MAX_FEATURES_BYTES) {
                    Ok(bytes) => (
                        String::from_utf8_lossy(&bytes).into_owned(),
                        harness_core::hash::bytes_hash(&bytes),
                    ),
                    Err(e) if e.is_not_found() => {
                        let starter = harness_core::TargetConfig::load(&root)
                            .map(|c| features::starter(&c))
                            .unwrap_or_default();
                        (starter, "none".to_string())
                    }
                    Err(e) => {
                        self.notice = notice(format!("the features file cannot be read: {e}"));
                        return Command::None;
                    }
                };
            let tmp = match handedit::private_dir(&std::env::temp_dir(), "harness-tui-features") {
                Ok(t) => t,
                Err(e) => {
                    self.notice = notice(format!("no private draft directory: {e}"));
                    return Command::None;
                }
            };
            let file = tmp.join(features::FEATURES_FILE);
            if let Err(e) = std::fs::write(&file, &text) {
                let _ = std::fs::remove_dir_all(&tmp);
                self.notice = notice(format!("the draft cannot be written: {e}"));
                return Command::None;
            }
            self.features_draft = Some(FeaturesDraft {
                tmp,
                file,
                expect,
                before: text,
                error_line: None,
                started: None,
            });
        }
        self.open_editor_dialog();
        Command::None
    }

    fn open_editor_dialog(&mut self) {
        let editor = self.features_editor();
        let name = editor_name(&editor);
        let title = format!("Open the features file in {name}?");
        let body = vec![
            format!(
                "The cockpit steps aside and {name} opens a private copy of \
                 migration/features/features.toml."
            ),
            editor_instructions(&name),
            "When you come back, the cockpit checks it and asks before saving it.".into(),
        ];
        let mut dialog = Dialog::new(Kind::OpenEditor, self.now);
        dialog.chat_rules = false;
        self.mode = Mode::Dialog(Box::new(super::Confirm {
            dialog,
            title,
            body,
            purpose: Purpose::OpenEditor,
        }));
    }

    /// The editor dialog closed (or the edit-again one).
    pub(super) fn close_features_dialog(&mut self, purpose: Purpose, choice: Choice) -> Command {
        match (purpose, choice) {
            (Purpose::OpenEditor | Purpose::EditAgain, Choice::Run) => {
                let Some(draft) = self.features_draft.as_mut() else {
                    return Command::None;
                };
                draft.started = Some(Instant::now());
                draft.before = std::fs::read_to_string(&draft.file).unwrap_or_default();
                Command::EditFeatures {
                    file: draft.file.clone(),
                    line: draft.error_line,
                }
            }
            (Purpose::EditAgain, Choice::Discard) => self.discard_features_draft(),
            _ => {
                if let Some(d) = &self.features_draft {
                    self.notice = notice(format!(
                        "your features draft is kept in {} — the menu offers it again",
                        d.file.display()
                    ));
                }
                Command::None
            }
        }
    }

    /// Discard the kept draft.
    pub fn discard_features_draft(&mut self) -> Command {
        match self.features_draft.take() {
            Some(d) => {
                self.notice = notice("features draft discarded");
                Command::Cleanup(d.tmp)
            }
            None => Command::None,
        }
    }

    /// The editor returned: no change, a draft that does not validate (the
    /// error, Edit again or Discard), or the confirmed save.
    pub fn features_edited(
        &mut self,
        status: std::io::Result<std::process::ExitStatus>,
    ) -> Command {
        let Some(draft) = self.features_draft.clone() else {
            return Command::None;
        };
        let quick = draft
            .started
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1));
        let text = match std::fs::read_to_string(&draft.file) {
            Ok(t) => t,
            Err(e) => {
                self.notice = notice(format!(
                    "the draft cannot be read ({e}); it is kept in {}",
                    draft.tmp.display()
                ));
                return Command::None;
            }
        };
        if let Ok(st) = &status {
            if !st.success() {
                self.notice = notice(format!(
                    "the editor exited {st}; your draft is kept in {} — the menu offers it again",
                    draft.file.display()
                ));
                return Command::None;
            }
        }
        if let Err(e) = &status {
            self.notice = notice(format!(
                "the editor could not run ({e}); set $VISUAL or $EDITOR — the draft is kept in {}",
                draft.file.display()
            ));
            return Command::None;
        }
        if text == draft.before {
            self.notice = notice(if quick {
                "The editor returned at once — if it opened a window, use its 'wait' option \
                 (for VS Code: code -w)"
                    .to_string()
            } else {
                "No change.".to_string()
            });
            // Nothing new to keep unless an earlier return changed it.
            if draft.error_line.is_none() {
                return self.discard_quietly();
            }
            return Command::None;
        }
        match features::parse(&text, &draft.file) {
            Err(e) => {
                let message = e.to_string();
                if let Some(d) = self.features_draft.as_mut() {
                    d.error_line = error_line(&message);
                }
                let mut dialog = Dialog::new(Kind::EditAgain, self.now);
                dialog.chat_rules = false;
                self.mode = Mode::Dialog(Box::new(super::Confirm {
                    dialog,
                    title: "Your features file has an error".into(),
                    body: vec![
                        message,
                        "Edit again opens your draft at that line; Discard drops it; Esc keeps \
                         it for later."
                            .into(),
                    ],
                    purpose: Purpose::EditAgain,
                }));
                Command::None
            }
            Ok(_) => {
                if let Some(d) = self.features_draft.as_mut() {
                    d.error_line = None;
                }
                match self.save_features_pending(&draft, &text) {
                    Ok(p) => {
                        self.ask(p);
                        Command::None
                    }
                    Err(why) => {
                        self.notice = notice(format!("{why}; your draft is kept"));
                        Command::None
                    }
                }
            }
        }
    }

    fn discard_quietly(&mut self) -> Command {
        match self.features_draft.take() {
            Some(d) => Command::Cleanup(d.tmp),
            None => Command::None,
        }
    }

    /// The confirmed save: `harness features save --expect … --bytes N`, the
    /// text on its stdin.
    fn save_features_pending(&self, draft: &FeaturesDraft, text: &str) -> Result<Pending, String> {
        let argv = self.harness_argv(&[
            OsString::from("features"),
            OsString::from("save"),
            OsString::from(format!("--expect={}", draft.expect)),
            OsString::from(format!("--bytes={}", text.len())),
            self.target_arg(),
        ])?;
        Ok(Pending {
            act: Act::SaveFeatures,
            argv,
            label: Act::SaveFeatures.label().to_string(),
            unit: None,
            attempt: None,
            cleanup: Some(draft.tmp.clone()),
            expect_attempt: None,
            note: None,
            shown_digest: None,
            chat: None,
            stdin: Some(text.to_string()),
        })
    }

    /// The Save dialog's words, by what changed (§7.2 step 4).
    pub(super) fn save_features_words(&self, p: &Pending) -> (String, Vec<String>) {
        let text = p.stdin.clone().unwrap_or_default();
        let Ok(new) = features::parse(&text, Path::new("features.toml")) else {
            return ("Save the features file?".into(), vec![]);
        };
        let config = harness_core::TargetConfig::load(&self.config.target).ok();
        let new_digest = config.as_ref().map(|c| features::features_digest(&new, c));
        let old_digest = match &self.snapshot.features {
            features::FeatureSnapshot::Valid { digest, .. } => Some(digest.clone()),
            _ => None,
        };
        let n = new.scenarios.len();
        let f = new.features.len();
        let mut body = vec![format!(
            "{n} scenario{} in {f} feature{}.",
            if n == 1 { "" } else { "s" },
            if f == 1 { "" } else { "s" }
        )];
        if new_digest.is_some() && new_digest == old_digest {
            body.push("Only names changed: no verdict is affected.".into());
        } else if n > 0 {
            body.push(
                "From now on every Re-check runs them; verdicts made before show \"not checked \
                 since you changed them\" until re-checked."
                    .into(),
            );
        }
        body.push("Writes migration/features/features.toml — commit it with your work.".into());
        ("Save the features file?".into(), body)
    }

    /// A save ended: saved → the draft goes; refused → it stays, said.
    pub(super) fn features_save_ended(&mut self, success: bool) -> Option<PathBuf> {
        if success {
            return self.features_draft.take().map(|d| d.tmp);
        }
        if let Some(d) = &self.features_draft {
            self.notice = notice(format!(
                "not saved — your draft is kept in {} — the menu offers Continue my features draft",
                d.file.display()
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_is_named_and_explained() {
        assert_eq!(features_editor(Some("code -w".into()), None), "code -w");
        assert_eq!(features_editor(Some(" ".into()), Some("vim".into())), "vim");
        assert_eq!(editor_name("\"/My Apps/ed\" -w"), "ed");
        assert_eq!(editor_name("/usr/bin/nano"), "nano");
        assert!(editor_instructions("nano").contains("Ctrl-X leaves"));
        assert!(editor_instructions("vim").contains("Press i to type"));
        assert!(editor_instructions("code").contains("Save and close"));
    }

    #[test]
    fn the_editor_opens_at_the_error_line_when_it_can() {
        let file = Path::new("/tmp/x/features.toml");
        let argv = editor_argv("nano", file, Some(7));
        assert_eq!(argv[3], OsString::from("+7"));
        assert_eq!(argv[4], file.as_os_str());
        let argv = editor_argv("code -w", file, Some(7));
        assert_eq!(argv.len(), 4, "no +N for an editor that does not take it");
        assert_eq!(
            error_line("…: not valid TOML at line 4, column 7: x"),
            Some(4)
        );
        assert_eq!(error_line("no line here"), None);
    }

    fn ok() -> std::io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(0))
    }

    fn open_draft(app: &mut App) -> PathBuf {
        assert_eq!(app.start_features_edit(), Command::None);
        let Mode::Dialog(c) = &app.mode else {
            panic!("the editor dialog: {:?}", app.mode)
        };
        assert_eq!(c.purpose, Purpose::OpenEditor);
        assert!(
            c.title.starts_with("Open the features file in "),
            "{}",
            c.title
        );
        let confirm = match std::mem::replace(&mut app.mode, Mode::Normal) {
            Mode::Dialog(c) => *c,
            _ => unreachable!(),
        };
        match app.close_features_dialog(confirm.purpose, Choice::Run) {
            Command::EditFeatures { file, .. } => file,
            other => panic!("{other:?}"),
        }
    }

    const GOOD: &str = "schema_version = 1\n[[feature]]\nid = \"help\"\nname = \"Show the help\"\n\
                        [[scenario]]\nfeature = \"help\"\nid = \"flag\"\nargs = [\"-h\"]\n";

    #[test]
    fn writing_a_features_file_saves_through_the_cli_with_the_text_on_stdin() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "feat-write");
        let file = open_draft(&mut app);
        let starter = std::fs::read_to_string(&file).unwrap();
        assert!(
            starter.contains("schema_version = 1"),
            "the starter: {starter}"
        );
        assert!(
            app.kept_paths().iter().any(|p| file.starts_with(p)),
            "the draft is named on quit"
        );
        std::fs::write(&file, GOOD).unwrap();
        assert_eq!(app.features_edited(ok()), Command::None);
        let Mode::Dialog(c) = &app.mode else {
            panic!("the save dialog: {:?}", app.mode)
        };
        let Purpose::Act(p) = &c.purpose else {
            panic!()
        };
        assert_eq!(p.act, Act::SaveFeatures);
        assert_eq!(p.stdin.as_deref(), Some(GOOD));
        let argv = crate::app::tests::strs(&p.argv);
        assert_eq!(
            argv[..5],
            [
                crate::app::tests::HARNESS,
                "--json",
                "features",
                "save",
                "--expect=none"
            ]
        );
        assert!(argv.contains(&format!("--bytes={}", GOOD.len())));
        assert!(
            c.body
                .iter()
                .any(|l| l.starts_with("1 scenario in 1 feature")),
            "{:?}",
            c.body
        );
        // The target's file is untouched until the CLI saves it.
        assert!(!harness_core::features::features_path(&app.config.target).exists());
        // Saved: the draft goes.
        let tmp = app.features_draft.as_ref().unwrap().tmp.clone();
        assert_eq!(app.features_save_ended(true), Some(tmp));
        assert!(app.features_draft.is_none());
    }

    #[test]
    fn a_draft_with_an_error_is_kept_and_offered_again_at_its_line() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "feat-invalid-draft");
        let file = open_draft(&mut app);
        std::fs::write(&file, "schema_version = 1\n[[feature]]\nid = \n").unwrap();
        app.features_edited(ok());
        let Mode::Dialog(c) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        assert_eq!(c.purpose, Purpose::EditAgain);
        assert!(c.body[0].contains("at line 3"), "{:?}", c.body);
        assert_eq!(app.features_draft.as_ref().unwrap().error_line, Some(3));
        app.mode = Mode::Normal;
        match app.close_features_dialog(Purpose::EditAgain, Choice::Run) {
            Command::EditFeatures { line, .. } => assert_eq!(line, Some(3)),
            other => panic!("{other:?}"),
        }
        // Esc keeps it; the menu offers it; Discard drops it.
        app.close_features_dialog(Purpose::EditAgain, Choice::Safe);
        app.selection = crate::tree::Selection::Features;
        let labels: Vec<String> = app.menu_items().into_iter().map(|i| i.label).collect();
        assert!(
            labels.contains(&"Continue my features draft".to_string()),
            "{labels:?}"
        );
        let tmp = app.features_draft.as_ref().unwrap().tmp.clone();
        assert_eq!(app.discard_features_draft(), Command::Cleanup(tmp));
        assert!(app.features_draft.is_none());
    }

    #[test]
    fn no_change_and_a_refused_save_say_so_and_keep_what_matters() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "feat-nochange");
        let _file = open_draft(&mut app);
        assert!(
            matches!(app.features_edited(ok()), Command::Cleanup(_)),
            "nothing to keep"
        );
        assert!(app.features_draft.is_none());
        let file = open_draft(&mut app);
        std::fs::write(&file, GOOD).unwrap();
        app.features_edited(ok());
        app.mode = Mode::Normal;
        assert_eq!(
            app.features_save_ended(false),
            None,
            "a refused save keeps the draft"
        );
        assert!(app.features_draft.is_some());
        assert!(
            app.notice.as_ref().unwrap().text.contains("not saved"),
            "{:?}",
            app.notice
        );
    }

    #[test]
    fn a_symlinked_features_dir_is_refused_before_anything() {
        let mut app = crate::app::tests::app_of("targets/zopfli", "feat-symlink");
        let elsewhere = app.config.target.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, app.config.target.join("migration/features"))
            .unwrap();
        assert_eq!(app.start_features_edit(), Command::None);
        assert!(app.features_draft.is_none());
        assert!(app.notice.as_ref().unwrap().text.contains("symlink"));
    }
}
