//! The include rule the oracle's readers follow for a file-list target
//! (docs/PROJECT-MAP-DESIGN.md §3.7; the 2026-10-08 triage, decision 1):
//! the compiler's own search order, which the oracle's argument order
//! ([`crate::cc_argv`]: the judge's flags, the configuration's flags, then
//! the file's `-I` folders) gives every compile.
//!
//! For an include written in a file reached while compiling the listed file
//! L:
//! - a quoted include searches the including file's own folder, then the
//!   configuration's `-iquote` folders in order, its `-I` folders in order,
//!   L's `include_dirs` in order, the configuration's `-isystem` folders,
//!   then the system;
//! - an angle-bracket include searches the same without the own folder and
//!   without the `-iquote` folders (the compiler reads `-iquote` for quoted
//!   includes only);
//! - each configuration `-include` file is the first include of every
//!   listed file, and its own includes are followed like any header's.
//!
//! [`search_order`] is the one function that computes the order; harness-core
//! is to hold the shared resolver (fix pass A), and [`closure`] then calls it
//! in place of this module's walk.

use harness_core::config::flags::{check_flag, Flag};
use harness_core::config::{Form, TargetContext};
use harness_core::sources::{include_names, Confine, MAX_SOURCE_BYTES};
use harness_core::Facts;
use std::collections::BTreeSet;

/// The configuration's path flags, by kind, as written (project-relative,
/// `.` the root), in argument order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConfigFolders {
    /// `-iquote` folders.
    pub iquote: Vec<String>,
    /// `-I` folders.
    pub angled: Vec<String>,
    /// `-isystem` folders.
    pub system: Vec<String>,
    /// `-include` files.
    pub forced: Vec<String>,
}

impl ConfigFolders {
    /// The path flags of `flags` (the configuration's, each checked by the
    /// flag grammar; a flag outside it is left out — loading refused it).
    pub(crate) fn of(flags: &[String]) -> ConfigFolders {
        let mut out = ConfigFolders::default();
        for flag in flags {
            let Ok(Flag::Path(rel)) = check_flag(flag) else {
                continue;
            };
            let rel = rel.to_string();
            match &flag[..flag.len() - rel.len()] {
                "-iquote" => out.iquote.push(rel),
                "-I" => out.angled.push(rel),
                "-isystem" => out.system.push(rel),
                "-include" => out.forced.push(rel),
                _ => {}
            }
        }
        out
    }
}

/// The folders, in order, the compiler searches for an include written in
/// `includer` (project-relative) while compiling a listed file whose own
/// `include_dirs` are `file_dirs` — see the module docs. `""` is the root.
pub(crate) fn search_order(
    config: &ConfigFolders,
    file_dirs: &[String],
    includer: &str,
    quoted: bool,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if quoted {
        out.push(match includer.rsplit_once('/') {
            Some((dir, _)) => dir.to_string(),
            None => String::new(),
        });
        out.extend(config.iquote.iter().cloned());
    }
    out.extend(config.angled.iter().cloned());
    out.extend(file_dirs.iter().cloned());
    out.extend(config.system.iter().cloned());
    out
}

/// The files the compile of `start` (project-relative, as a scan records
/// them) reads from the project, starts included, under the rule: for the
/// folder form the facts' include closure as ever; for a file list, the
/// files the rule reaches from every listed `.c` of `start` (each under its
/// own folders, the configuration's `-include` files first), joined with
/// the facts' closure — a header the facts name stays in, so a forged or
/// stale record is still checked by the callers' confinement. Each file
/// is read lexically ([`include_names`]), at most [`MAX_SOURCE_BYTES`],
/// as a regular file; one that cannot be read adds no includes.
pub(crate) fn closure(target: &TargetContext, facts: &Facts, start: &[String]) -> Vec<String> {
    let mut out: BTreeSet<String> = facts.include_closure(start).into_iter().collect();
    let Form::FileList(list) = &target.config.target.form else {
        return out.into_iter().collect();
    };
    let Ok(confine) = Confine::new(target) else {
        return out.into_iter().collect();
    };
    let Ok(Some(listed)) = confine.listed_files(target) else {
        return out.into_iter().collect();
    };
    let config = ConfigFolders::of(&list.configuration.flags);
    let forced: Vec<String> = config
        .forced
        .iter()
        .filter_map(|rel| {
            let real = confine.root().join(rel).canonicalize().ok()?;
            confine.allows(&real).then(|| confine.rel(&real)).flatten()
        })
        .collect();
    // Each listed `.c` of the start is a compile with its own folders; a
    // header named in the start is read under each of them.
    let mut compiles: Vec<&[String]> = start
        .iter()
        .filter(|f| f.ends_with(".c"))
        .filter_map(|c| listed.iter().find(|l| l.path == *c))
        .map(|l| l.include_dirs.as_slice())
        .collect();
    if compiles.is_empty() {
        compiles.push(&[]);
    }
    for dirs in compiles {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = start.iter().rev().cloned().collect();
        stack.extend(forced.iter().rev().cloned());
        while let Some(file) = stack.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            let Ok(bytes) =
                harness_core::ledger::read_regular(&confine.root().join(&file), MAX_SOURCE_BYTES)
            else {
                continue;
            };
            for (name, quoted) in include_names(&bytes) {
                let order = search_order(&config, dirs, &file, quoted);
                if let Some(found) = confine.resolve_include(&file, &name, false, &order) {
                    stack.push(found);
                }
            }
        }
        out.extend(seen);
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crate::{cc_argv, Base, CcInvocation};
    use std::path::{Path, PathBuf};

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
        std::fs::write(&p, text).expect("write");
    }

    /// Every flag kind and a same-named header in two of its folders:
    /// `app/main.c` (listed with `d/`), configuration
    /// `-iquote q -I i -isystem s -include f/forced.h`.
    fn fixture(root: &Path) -> TargetContext {
        put(
            root,
            "app/main.c",
            "#include \"own.h\"\n#include \"same.h\"\n#include <ang.h>\n\
             #include \"dup.h\"\n#include \"only_dir.h\"\n#include \"sysq.h\"\n\
             int main(void) { return 0; }\n",
        );
        // Own folder before the -iquote folder.
        put(root, "app/own.h", "/* app */\n");
        put(root, "q/own.h", "/* q */\n");
        // -iquote before -I, for a quoted include.
        put(root, "q/same.h", "/* q */\n");
        put(root, "i/same.h", "/* i */\n");
        // An angle-bracket include never searches -iquote.
        put(root, "q/ang.h", "/* q */\n");
        put(root, "i/ang.h", "/* i */\n");
        // The configuration's -I before the file's own folders.
        put(root, "i/dup.h", "/* i */\n");
        put(root, "d/dup.h", "/* d */\n");
        // The file's own folders before -isystem.
        put(root, "d/only_dir.h", "/* d */\n");
        put(root, "s/only_dir.h", "/* s */\n");
        // -isystem, last before the system.
        put(root, "s/sysq.h", "/* s */\n");
        // The forced include, first, and its own neighbour (its own folder
        // before the -iquote folder).
        put(root, "f/forced.h", "#include \"fneighbor.h\"\n");
        put(root, "f/fneighbor.h", "/* f */\n");
        put(root, "q/fneighbor.h", "/* q */\n");
        let config: harness_core::TargetConfig = toml::from_str(
            "schema_version = 2\n[target]\nname = \"t\"\n\
             files = [{ path = \"app/main.c\", include_dirs = [\"d\"] }]\n\
             configuration = { name = \"make\", from = \"stated\", flags = \
             [\"-iquoteq\", \"-Ii\", \"-isystems\", \"-includef/forced.h\", \"-DX=1\"] }\n\
             [oracle]\nallowlist = [\"cc\"]\n",
        )
        .expect("config");
        TargetContext {
            ledger: root.join("migration/tools/t-x"),
            tool: Some("t-x".into()),
            root: root.to_path_buf(),
            config,
        }
    }

    /// The project files `cc -M` reads compiling `source` with `args`,
    /// relative to `root`.
    fn read_by_cc(root: &Path, args: &crate::FileArgs, source: &Path) -> BTreeSet<String> {
        let deps = root.join("deps.d");
        let argv = cc_argv(&CcInvocation {
            args,
            cflags: &["-M".to_string()],
            quiet: true,
            out: &deps,
            inputs: &[source.to_path_buf()],
            libs: &[],
        })
        .expect("argv");
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .expect("cc runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = std::fs::read_to_string(&deps).expect("deps");
        let _ = std::fs::remove_file(&deps);
        let (_, rest) = text.split_once(':').expect("a make rule");
        rest.split_whitespace()
            .filter(|t| *t != "\\")
            .filter_map(|t| Path::new(t).strip_prefix(root).ok().map(PathBuf::from))
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// The rule predicts exactly the project files the compiler reads with
    /// the oracle's own argument order, for every flag kind.
    #[test]
    fn the_rule_reads_what_cc_reads() {
        let tmp = TempDir::new("include-rule");
        let root = tmp.path().to_path_buf();
        let target = fixture(&root);
        let base = Base::resolve(&target, "u", &["cc"]).expect("base");
        let main = root.join("app/main.c");
        let by_cc = read_by_cc(&root, &base.file_args(&main).expect("args"), &main);
        let predicted: BTreeSet<String> = closure(
            &target,
            &harness_core::Facts::default(),
            &["app/main.c".into()],
        )
        .into_iter()
        .collect();
        let expected: BTreeSet<String> = [
            "app/main.c",
            "app/own.h",
            "q/same.h",
            "i/ang.h",
            "i/dup.h",
            "d/only_dir.h",
            "s/sysq.h",
            "f/forced.h",
            "f/fneighbor.h",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        assert_eq!(by_cc, expected, "what cc reads");
        assert_eq!(predicted, expected, "what the rule predicts");
        // The driver's names and folders follow the same rule.
        let unit: harness_core::Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"app/main.c\"]\nsymbols = [\"main\"]\n",
        )
        .expect("unit");
        let facts = harness_core::Facts::default();
        let folders = crate::driver_folders(&target, &facts, &unit);
        for dir in ["app", "d", "q", "i", "s", "f"] {
            assert!(folders.contains(&dir.to_string()), "{dir}: {folders:?}");
        }
        let names = crate::unit_header_names(&target, &facts, &unit);
        for name in ["same.h", "q/same.h", "sysq.h", "forced.h", "f/fneighbor.h"] {
            assert!(names.contains(&name.to_string()), "{name}: {names:?}");
        }
        let headers = base.unit_headers(&target, &facts, &unit).expect("headers");
        assert!(headers.contains(&root.join("f/forced.h")), "{headers:?}");
        assert!(!headers.contains(&root.join("q/own.h")), "{headers:?}");
    }

    /// A mutant's copy, compiled away from its original's folder, reads the
    /// headers the original reads: its folder is searched by quoted
    /// includes before the configuration's `-iquote` and `-I` folders.
    #[test]
    fn a_mutant_reads_its_originals_headers() {
        let tmp = TempDir::new("include-rule-mutant");
        let root = tmp.path().to_path_buf();
        let target = fixture(&root);
        let base = Base::resolve(&target, "u", &["cc"]).expect("base");
        let main = root.join("app/main.c");
        let copy = root.join("elsewhere/main.c");
        put(
            &root,
            "elsewhere/main.c",
            &std::fs::read_to_string(&main).expect("main"),
        );
        let args = crate::validate::mutant_args(&base, &main).expect("args");
        let original = read_by_cc(&root, &args, &main);
        let mut mutant = read_by_cc(&root, &args, &copy);
        assert!(mutant.remove("elsewhere/main.c"));
        mutant.insert("app/main.c".into());
        assert_eq!(mutant, original);
        assert!(original.contains("app/own.h"), "{original:?}");
    }
}
