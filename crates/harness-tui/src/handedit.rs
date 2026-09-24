//! The labelled hand edit's file flow (docs/TUI-DESIGN.md §4 `e`, §R2 9):
//! the crate's `src/logic.rs` and `src/ffi.rs` are copied into a fresh
//! private `<tmp>/edit/`, hashed, opened in the user's editor, hashed again;
//! when they changed, EXACTLY those two files are staged in a new
//! `<tmp>/stage/src/` — so nothing an editor leaves beside them (`*~`,
//! `.*.swp`, `#*#`) ever reaches the directory `harness override` reads.
//! The caller removes `<tmp>` only once the edit was RECORDED (the
//! override's `attempt` event) or the user discarded it: a refused override,
//! a failed spawn or an editor that exits non-zero after saving must never
//! cost the user their edit.

use harness_core::hash;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// The two files a hand edit consists of, crate-relative.
pub const EDIT_FILES: [&str; 2] = ["src/logic.rs", "src/ffi.rs"];

/// The editor command: `$VISUAL`, else `$EDITOR`, else `vi`, taken as
/// shell text the way git takes it (`EDITOR='"/My Apps/ed" -w'` works).
pub fn editor_command(visual: Option<OsString>, editor: Option<OsString>) -> String {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(|v| v.to_string_lossy().into_owned())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".into())
}

/// The `sh -c` script that runs `editor` on `"$@"` — git's rule
/// (`<editor> "$@"`). When the text is one plain command, `exec` is
/// prepended, so the child's pid is the editor's own and a forwarded
/// TERM/HUP reaches it; a leading `VAR=value` or a compound command (`;`,
/// `&`, `|`, `(`, `)`, backquote, newline, redirection) is run exactly as
/// git runs it, because `exec` would break it.
pub fn editor_script(editor: &str) -> String {
    let first = editor.split_whitespace().next().unwrap_or("");
    let compound = editor.contains([';', '&', '|', '(', ')', '`', '\n', '<', '>']);
    let assignment = first.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    if compound || assignment {
        format!("{editor} \"$@\"")
    } else {
        format!("exec {editor} \"$@\"")
    }
}

/// One hand edit in progress.
#[derive(Debug)]
pub struct Session {
    /// The private temp dir holding everything (`edit/`, later `stage/`).
    pub tmp: PathBuf,
    /// The files as the editor sees them: `<tmp>/edit/logic.rs`, `…/ffi.rs`.
    pub files: [PathBuf; 2],
    before: [String; 2],
}

/// Whether `crate_dir` is in the executor layout a hand edit needs.
pub fn editable(crate_dir: &Path) -> bool {
    EDIT_FILES.iter().all(|rel| {
        std::fs::symlink_metadata(crate_dir.join(rel)).is_ok_and(|m| m.file_type().is_file())
    })
}

/// A fresh private directory under `base` (mode 0700, never an existing
/// path).
fn fresh_dir(base: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    for n in 0..1000u32 {
        let dir = base.join(format!("harness-tui-edit-{}-{n}", std::process::id()));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temp directory name"))
}

fn digest(path: &Path) -> std::io::Result<String> {
    hash::file_hash(path).map_err(|e| std::io::Error::other(e.to_string()))
}

/// Largest file copied into an edit (the CLI's own limit).
const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;

/// Copy the crate's two files into a fresh `<base>/harness-tui-edit-…/edit/`
/// and hash them. The crate is ledger content (target-owned): a symlink, a
/// non-regular or an oversized file is refused, and a failure leaves no
/// directory behind.
pub fn prepare(crate_dir: &Path, base: &Path) -> std::io::Result<Session> {
    let tmp = fresh_dir(base)?;
    let fill = || -> std::io::Result<Session> {
        let edit = tmp.join("edit");
        std::fs::create_dir(&edit)?;
        let files = EDIT_FILES.map(|rel| edit.join(Path::new(rel).file_name().unwrap_or_default()));
        for (rel, to) in EDIT_FILES.iter().zip(&files) {
            let from = crate_dir.join(rel);
            let meta = std::fs::symlink_metadata(&from)?;
            if !meta.file_type().is_file() || meta.len() > MAX_EDIT_FILE_BYTES {
                return Err(std::io::Error::other(format!(
                    "{rel} is not a regular file of at most {MAX_EDIT_FILE_BYTES} bytes"
                )));
            }
            std::fs::copy(&from, to)?;
        }
        let before = [digest(&files[0])?, digest(&files[1])?];
        Ok(Session {
            tmp: tmp.clone(),
            files,
            before,
        })
    };
    fill().inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&tmp);
    })
}

impl Session {
    /// Start `editor` (shell text, see [`editor_command`]) on the two files,
    /// inheriting the terminal; the caller waits for it.
    pub fn spawn_editor(&self, editor: &str) -> std::io::Result<Child> {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(editor_script(editor))
            .arg("sh")
            .args(&self.files)
            .spawn()
    }

    /// Whether the editor left anything beside the two files in `<tmp>/edit/`
    /// — vim's `.logic.rs.swp`, nano's `logic.rs.save`, written when it is
    /// hung up or terminated: the unsaved buffer. An unreadable dir counts.
    pub fn has_leftovers(&self) -> bool {
        let Some(edit) = self.files[0].parent() else {
            return true;
        };
        std::fs::read_dir(edit).map_or(true, |entries| {
            entries.flatten().any(|e| {
                !self
                    .files
                    .iter()
                    .any(|f| f.file_name() == Some(e.file_name().as_os_str()))
            })
        })
    }

    /// Whether either file differs from what was copied in.
    pub fn changed(&self) -> std::io::Result<bool> {
        Ok([digest(&self.files[0])?, digest(&self.files[1])?] != self.before)
    }

    /// After the editor exited 0: `None` when neither file changed
    /// (nothing to record); else the staged DIR for `harness override`,
    /// holding exactly `src/logic.rs` and `src/ffi.rs`.
    pub fn stage(&self) -> std::io::Result<Option<PathBuf>> {
        if !self.changed()? {
            return Ok(None);
        }
        let stage = self.tmp.join("stage");
        let src = stage.join("src");
        std::fs::create_dir(&stage)?;
        std::fs::create_dir(&src)?;
        for (from, rel) in self.files.iter().zip(EDIT_FILES) {
            std::fs::copy(from, stage.join(rel))?;
        }
        Ok(Some(stage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_dir(base: &Path) -> PathBuf {
        let dir = base.join("crate");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/logic.rs"), "pub fn f() {}\n").unwrap();
        std::fs::write(dir.join("src/ffi.rs"), "// ffi\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "mod logic;\n").unwrap();
        dir
    }

    fn base(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harness-tui-handedit-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An "editor" (a shell script run exactly as `$EDITOR` would be) that
    /// appends to the first file and leaves backup and swap files beside it.
    fn editor(base: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = base.join("fake-editor.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn run_with_editor(session: &Session, editor: &Path) -> std::process::ExitStatus {
        let editor = editor_command(None, Some(editor.as_os_str().to_owned()));
        session.spawn_editor(&editor).unwrap().wait().unwrap()
    }

    /// §R2 9: editor artefacts never reach the DIR `override` reads.
    #[test]
    fn only_the_two_files_are_staged() {
        let base = base("stage");
        let krate = crate_dir(&base);
        assert!(editable(&krate));
        let session = prepare(&krate, &base).unwrap();
        let ed = editor(
            &base,
            r#"echo '// edited' >> "$1"; cp "$1" "$1~"; touch "$(dirname "$1")/.logic.rs.swp" "$(dirname "$1")/#logic.rs#""#,
        );
        assert!(run_with_editor(&session, &ed).success());
        let stage = session.stage().unwrap().expect("a changed edit is staged");
        let mut names: Vec<String> = std::fs::read_dir(stage.join("src"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["ffi.rs", "logic.rs"]);
        let entries: Vec<_> = std::fs::read_dir(&stage).unwrap().collect();
        assert_eq!(entries.len(), 1, "only src/ in the staged DIR");
        assert!(std::fs::read_to_string(stage.join("src/logic.rs"))
            .unwrap()
            .ends_with("// edited\n"));
        // The original crate is untouched.
        assert_eq!(
            std::fs::read_to_string(krate.join("src/logic.rs")).unwrap(),
            "pub fn f() {}\n"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// §7: an unchanged hand edit stages (and so spawns) nothing.
    #[test]
    fn an_unchanged_edit_stages_nothing() {
        let base = base("unchanged");
        let krate = crate_dir(&base);
        let session = prepare(&krate, &base).unwrap();
        let ed = editor(&base, r#"touch "$1~""#);
        assert!(run_with_editor(&session, &ed).success());
        assert_eq!(session.stage().unwrap(), None);
        // The temp dir is private.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&session.tmp)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The editor is shell text (git's rule): quoted paths and arguments in
    /// `$EDITOR` work, `$VISUAL` wins, `vi` is the default; the script
    /// `exec`s it so a forwarded signal reaches the editor itself.
    #[test]
    fn the_editor_command_follows_shell_quoting() {
        let base = base("quoting");
        let dir = base.join("My Editor");
        std::fs::create_dir_all(&dir).unwrap();
        let ed = editor(
            &dir,
            r#"echo "$1|$2" > "$(dirname "$2")/args.txt"; echo '//' >> "$2""#,
        );
        let krate = crate_dir(&base);
        let session = prepare(&krate, &base).unwrap();
        let text = format!("'{}' --wait", ed.display());
        let status = session.spawn_editor(&text).unwrap().wait().unwrap();
        assert!(status.success());
        let args = std::fs::read_to_string(session.files[0].with_file_name("args.txt")).unwrap();
        assert!(args.starts_with("--wait|"), "{args}");
        assert!(session.changed().unwrap());
        assert_eq!(
            editor_command(Some("nano".into()), Some("vim".into())),
            "nano"
        );
        assert_eq!(editor_command(Some(" ".into()), Some("vim".into())), "vim");
        assert_eq!(editor_command(None, None), "vi");
        assert!(editor_script("x --wait").starts_with("exec x --wait "));
        // git's form, untouched, where `exec` would break it.
        for text in ["NVIM_APPNAME=rust nvim", "clear; vim", "a | b"] {
            assert_eq!(editor_script(text), format!("{text} \"$@\""));
        }
        let out = Command::new("/bin/sh")
            .args(["-c", &editor_script("FOO=1 /bin/echo"), "sh", "a", "b"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b\n");
        // Leftovers: anything beside the two files.
        assert!(
            !session.has_leftovers()
                || std::fs::read_dir(session.files[0].parent().unwrap())
                    .unwrap()
                    .count()
                    > 2
        );
        std::fs::write(session.files[0].with_file_name(".logic.rs.swp"), "buffer").unwrap();
        assert!(session.has_leftovers());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Ledger content is hostile: a symlinked crate file is refused, and a
    /// refused prepare leaves no temp dir behind.
    #[test]
    fn a_symlinked_crate_file_is_refused_without_a_leftover_dir() {
        let base = base("symlink");
        let krate = crate_dir(&base);
        std::fs::remove_file(krate.join("src/ffi.rs")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", krate.join("src/ffi.rs")).unwrap();
        let tmp_base = base.join("tmp");
        std::fs::create_dir_all(&tmp_base).unwrap();
        assert!(prepare(&krate, &tmp_base).is_err());
        assert_eq!(std::fs::read_dir(&tmp_base).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_crate_without_the_executor_layout_is_not_editable() {
        let base = base("layout");
        let krate = crate_dir(&base);
        std::fs::remove_file(krate.join("src/ffi.rs")).unwrap();
        assert!(!editable(&krate));
        let _ = std::fs::remove_dir_all(&base);
    }
}
