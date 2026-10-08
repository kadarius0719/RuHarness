//! The include rule the oracle's readers follow for a file-list target
//! (docs/PROJECT-MAP-DESIGN.md §3.7; the 2026-10-08 triage, decision 1):
//! the compiler's own search order, which the oracle's argument order
//! ([`crate::cc_argv`]: the judge's flags, the configuration's flags, then
//! the file's `-I` folders) gives every compile. The order itself is
//! harness-core's shared resolver ([`Resolver::search_order`]): quoted —
//! the own folder, the configuration's `-iquote`, `-I`, the listed file's
//! `include_dirs`, `-isystem`; angle-bracket — the same without the own
//! folder and the `-iquote` folders; each `-include` file first. This
//! module walks it for the oracle's readers (`unit_headers`,
//! `driver_folders`, `unit_header_names`, the features mirror).

use harness_core::config::TargetContext;
use harness_core::sources::{names_on_disk, Resolver};
use harness_core::Facts;
use std::collections::BTreeSet;

/// The files the compile of `start` (project-relative, as a scan records
/// them) reads from the project, starts included, under the rule: for the
/// folder form the facts' include closure as ever; for a file list, the
/// files the resolver reaches from every listed `.c` of `start` (each as
/// its own compile, the configuration's `-include` files first), joined
/// with the facts' closure — a header the facts name stays in, so a forged
/// or stale record is still checked by the callers' confinement. Each file
/// is read as a scan reads it ([`names_on_disk`]); one that cannot be read
/// adds no includes.
pub(crate) fn closure(target: &TargetContext, facts: &Facts, start: &[String]) -> Vec<String> {
    let mut out: BTreeSet<String> = facts.include_closure(start).into_iter().collect();
    let Ok(Some(resolver)) = Resolver::of(target) else {
        return out.into_iter().collect();
    };
    // Each listed `.c` of the start is a compile of its own; a header named
    // in the start is read under each of them.
    let mut compiles: Vec<&str> = start
        .iter()
        .filter(|f| f.ends_with(".c"))
        .filter(|c| resolver.listed().iter().any(|l| l.path == **c))
        .map(String::as_str)
        .collect();
    if compiles.is_empty() {
        compiles.push("");
    }
    for unit in compiles {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = start.iter().rev().cloned().collect();
        stack.extend(resolver.forced_includes().iter().rev().cloned());
        while let Some(file) = stack.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            for (name, quoted) in names_on_disk(resolver.root(), &file).unwrap_or_default() {
                if let Some(found) = resolver.resolve(unit, &file, &name, quoted) {
                    stack.push(found);
                }
            }
        }
        out.extend(seen);
    }
    out.into_iter().collect()
}

/// The folders the unit's own `.c` files search, in the rule's order and
/// without repeats (each listed `.c`'s quoted order: its own folder, the
/// configuration's `-iquote` and `-I`, its `include_dirs`, `-isystem`);
/// empty for a folder target.
pub(crate) fn unit_search_folders(target: &TargetContext, unit_files: &[String]) -> Vec<String> {
    let Ok(Some(resolver)) = Resolver::of(target) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for c in unit_files.iter().filter(|f| f.ends_with(".c")) {
        for dir in resolver.search_order(c, c, true) {
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
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
