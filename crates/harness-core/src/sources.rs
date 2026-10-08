//! A target's C sources, confined (docs/PROJECT-MAP-DESIGN.md §3.7 "The
//! scanner" and "Confinement, restated"): the folders no scan, prompt or
//! compile reads as project C (`<root>/migration/`, every tool's ledger and
//! the map), a file-list target's files checked against the root and the
//! ledger, the include search of §3.1 step 3 for one file, and a lexical
//! reader of include names for the staleness rule (the scanner reads names
//! with its parser; this crate has none).
//!
//! The search, for an include name N written in file F built with the
//! include folders D: a quoted include tries F's own folder first, then each
//! of D in order; an angle-bracket include tries D only; the first folder
//! holding N as a regular file is the one the compiler takes. A file it
//! takes that lies outside the project root or under `migration/` is never
//! project C: the include resolves to no project file. Otherwise the system
//! holds N, or nothing does.

use crate::config::TargetContext;
use crate::error::Error;
use std::path::{Path, PathBuf};

/// Largest source file a scan parses (docs/PROJECT-MAP-DESIGN.md §3.10): a
/// larger one is recorded with its hash, never parsed.
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

/// The folders every scan, detect and prompt read leaves out:
/// `<root>/migration/` (every tool's ledger and the map) and the target's
/// own ledger, when it lies elsewhere.
pub fn pruned(ctx: &TargetContext) -> Vec<PathBuf> {
    let mut out = vec![ctx.root.join(crate::ledger::MIGRATION_DIR)];
    if !out.contains(&ctx.ledger) {
        out.push(ctx.ledger.clone());
    }
    out
}

/// The project root and the pruned folders, canonical: where project C may
/// be read from.
#[derive(Debug, Clone)]
pub struct Confine {
    root: PathBuf,
    pruned: Vec<PathBuf>,
}

/// One listed file of a file-list target, by the path a scan records it
/// under (its links resolved, relative to the root, `/`-joined), with its
/// include folders the same way (`""` is the root itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFile {
    /// The file.
    pub path: String,
    /// Its include folders, in order.
    pub include_dirs: Vec<String>,
}

impl Confine {
    /// The confinement of `ctx`; `Err` when the root cannot be resolved.
    pub fn new(ctx: &TargetContext) -> Result<Confine, Error> {
        let root = ctx
            .root
            .canonicalize()
            .map_err(|e| Error::io(&ctx.root, e))?;
        let pruned = pruned(ctx)
            .iter()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
            .collect();
        Ok(Confine { root, pruned })
    }

    /// The canonical project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the canonical `path` lies inside the root and outside every
    /// pruned folder.
    pub fn allows(&self, path: &Path) -> bool {
        path.starts_with(&self.root) && !self.pruned.iter().any(|p| path.starts_with(p))
    }

    /// The canonical `path` relative to the root, `/`-joined (`""` for the
    /// root); `None` outside it.
    pub fn rel(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    /// `rel` (a clean relative path, or `.`) as a scan records it: its
    /// links resolved when it exists, else as written. `Err(true)` when it
    /// leads under a pruned folder, `Err(false)` when it leads outside the
    /// root. The deepest part that exists decides, as for the config.
    fn place(&self, rel: &str) -> Result<String, bool> {
        let lexical = rel.trim_end_matches('/');
        let lexical = if lexical == "." { "" } else { lexical };
        let under_ledger = lexical
            .split('/')
            .next()
            .is_some_and(|first| first == crate::ledger::MIGRATION_DIR);
        if lexical.starts_with('/')
            || lexical.split('/').any(|s| s == "..")
            || (!lexical.is_empty() && !crate::plan::is_clean_relative_path(lexical))
        {
            return Err(false);
        }
        if under_ledger {
            return Err(true);
        }
        let mut probe = self.root.join(lexical);
        let mut rest: Vec<std::ffi::OsString> = Vec::new();
        loop {
            match probe.canonicalize() {
                Ok(real) => {
                    if !real.starts_with(&self.root) {
                        return Err(false);
                    }
                    if !self.allows(&real) {
                        return Err(true);
                    }
                    let mut full = real;
                    full.extend(rest.iter().rev());
                    return self.rel(&full).ok_or(false);
                }
                Err(_) => match probe.file_name() {
                    Some(name) => {
                        rest.push(name.to_os_string());
                        probe.pop();
                    }
                    None => return Err(false),
                },
            }
        }
    }

    /// A file-list target's files and their include folders, as a scan
    /// records them; `Ok(None)` for a folder target. A file or a folder that
    /// leads outside the project root or under `migration/` is refused in
    /// one sentence naming it (a `harness.toml` pointing into the ledger
    /// would put model-written files into prompts and builds).
    pub fn listed_files(&self, ctx: &TargetContext) -> Result<Option<Vec<ListedFile>>, Error> {
        let Some(files) = ctx.config.target.files() else {
            return Ok(None);
        };
        let config = ctx.ledger.join(crate::config::CONFIG_FILE);
        let refuse = |what: String, ledger: bool| {
            let why = if ledger {
                "lies under migration/, the harness's own folder, whose files are never read as \
                 project C; name the project's own file or folder"
            } else {
                "leads outside the project root; name a path inside the project"
            };
            Error::parse(&config, format!("{what} {why}"))
        };
        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let shown = crate::text::safe_line(&file.path);
            let path = self
                .place(&file.path)
                .map_err(|ledger| refuse(format!("the listed file `{shown}`"), ledger))?;
            let mut include_dirs = Vec::with_capacity(file.include_dirs.len());
            for dir in &file.include_dirs {
                include_dirs.push(self.place(dir).map_err(|ledger| {
                    refuse(
                        format!(
                            "the include folder `{}` of `{shown}`",
                            crate::text::safe_line(dir)
                        ),
                        ledger,
                    )
                })?);
            }
            out.push(ListedFile { path, include_dirs });
        }
        Ok(Some(out))
    }

    /// Where the include `name` written in `including` (a path as a scan
    /// records it) lands when that file is built with the include folders
    /// `dirs` (see the module docs): the project file, as a scan records
    /// it, or `None` when no project file is taken.
    pub fn resolve_include(
        &self,
        including: &str,
        name: &str,
        quoted: bool,
        dirs: &[String],
    ) -> Option<String> {
        if name.is_empty() || name.starts_with('/') {
            return None;
        }
        let own = match including.rsplit_once('/') {
            Some((dir, _)) => dir,
            None => "",
        };
        let search = quoted
            .then_some(own)
            .into_iter()
            .chain(dirs.iter().map(String::as_str));
        for dir in search {
            let candidate = self.root.join(dir).join(name);
            let Ok(real) = candidate.canonicalize() else {
                continue;
            };
            if !real.is_file() {
                continue;
            }
            // The compiler takes this file: project C only when confined.
            return self.allows(&real).then(|| self.rel(&real)).flatten();
        }
        None
    }
}

/// The include names a file's bytes write — `(name, quoted)`, sorted, each
/// once — read lexically: comments removed, continued lines joined, then
/// every line `# include "name"` or `# include <name>`, from every `#if`
/// branch alike, as the scanner's parser reads them.
pub fn include_names(src: &[u8]) -> Vec<(String, bool)> {
    let mut names = std::collections::BTreeSet::new();
    for line in logical_lines(&strip_comments(src)) {
        let line = trim_start(&line);
        let Some(rest) = line.strip_prefix(b"#") else {
            continue;
        };
        let Some(rest) = trim_start(rest).strip_prefix(b"include") else {
            continue;
        };
        let rest = trim_start(rest);
        let (close, quoted) = match rest.first() {
            Some(b'"') => (b'"', true),
            Some(b'<') => (b'>', false),
            _ => continue,
        };
        let body = &rest[1..];
        if let Some(end) = body.iter().position(|&b| b == close) {
            if let Ok(name) = std::str::from_utf8(&body[..end]) {
                if !name.is_empty() {
                    names.insert((name.to_string(), quoted));
                }
            }
        }
    }
    names.into_iter().collect()
}

fn trim_start(s: &[u8]) -> &[u8] {
    let at = s
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'))
        .unwrap_or(s.len());
    &s[at..]
}

/// The bytes with every `/* … */` and `// …` comment replaced by one space
/// (newlines inside a block comment kept), string and character literals
/// passed through as they are.
fn strip_comments(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'/' if src.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < src.len() && !(src[i] == b'*' && src.get(i + 1) == Some(&b'/')) {
                    if src[i] == b'\n' {
                        out.push(b'\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(src.len());
                out.push(b' ');
            }
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    // A continued `//` comment runs on to the next line.
                    if src[i] == b'\\' && src.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    i += 1;
                }
                out.push(b' ');
            }
            q @ (b'"' | b'\'') => {
                out.push(q);
                i += 1;
                while i < src.len() && src[i] != q && src[i] != b'\n' {
                    if src[i] == b'\\' && i + 1 < src.len() {
                        out.push(src[i]);
                        i += 1;
                    }
                    out.push(src[i]);
                    i += 1;
                }
                if i < src.len() && src[i] == q {
                    out.push(q);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// The lines of `src` with each backslash-newline continuation joined.
fn logical_lines(src: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\\' if src.get(i + 1) == Some(&b'\n') => i += 2,
            b'\\' if src.get(i + 1) == Some(&b'\r') && src.get(i + 2) == Some(&b'\n') => i += 3,
            b'\n' => {
                lines.push(std::mem::take(&mut current));
                i += 1;
            }
            b => {
                current.push(b);
                i += 1;
            }
        }
    }
    lines.push(current);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_names_reads_both_forms_and_skips_comments() {
        let src = b"/* #include \"no.h\" */\n#include \"a.h\"\n  #  include <b/c.h>\n\
            // #include \"no2.h\"\n#ifdef X\n#include \"d.h\"\n#endif\n#include NAME\n\
            #inc\\\nlude \"e.h\"\nconst char *s = \"#include \\\"no3.h\\\"\";\n";
        assert_eq!(
            include_names(src),
            [
                ("a.h".to_string(), true),
                ("b/c.h".to_string(), false),
                ("d.h".to_string(), true),
                ("e.h".to_string(), true),
            ]
        );
    }
}
