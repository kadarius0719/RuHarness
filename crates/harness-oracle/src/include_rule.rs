//! The include rule the oracle's readers follow for a file-list target
//! (docs/PROJECT-MAP-DESIGN.md §3.7; the 2026-10-08 triage, decision 1):
//! the compiler's own search order, which the oracle's argument order
//! ([`crate::cc_argv`]: the judge's flags, the configuration's flags, then
//! the file's `-I` folders) gives every compile. The order itself is
//! harness-core's shared resolver ([`Resolver::search_order`]): quoted —
//! the own folder, the configuration's `-iquote`, `-I`, the listed file's
//! `include_dirs`, `-isystem`; angle-bracket — the same without the own
//! folder and the `-iquote` folders; each `-include` file first; the
//! `-idirafter` folders after the system. The oracle's readers
//! (`unit_headers`, `driver_folders`, `unit_header_names`, the features
//! mirror) and `compute_inputs`' `unit_source` take the unit's closure
//! from harness-core ([`unit_closure`], the 2026-10-08 triage, decision
//! 11), the one the planner and `state status` hash too.

use harness_core::config::TargetContext;
use harness_core::sources::{unit_closure, Resolver};
use harness_core::Facts;

/// The files the compile of `start` (project-relative, as a scan records
/// them) reads from the project, starts included: harness-core's
/// [`unit_closure`] (for the folder form the facts' include closure as
/// ever; for a file list the resolver's closure joined with the facts', so
/// a forged or stale record is still checked by the callers'
/// confinement).
pub(crate) fn closure(target: &TargetContext, facts: &Facts, start: &[String]) -> Vec<String> {
    unit_closure(target, facts, start)
}

/// The folders the unit's own `.c` files search, in the rule's order and
/// without repeats (each listed `.c`'s quoted order: its own folder, the
/// configuration's `-iquote` and `-I`, its `include_dirs`, `-isystem`; then
/// the `-idirafter` folders, searched after the system); empty for a folder
/// target.
pub(crate) fn unit_search_folders(target: &TargetContext, unit_files: &[String]) -> Vec<String> {
    let Ok(Some(resolver)) = Resolver::of(target) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for c in unit_files.iter().filter(|f| f.ends_with(".c")) {
        for dir in resolver
            .search_order(c, c, true)
            .into_iter()
            .chain(resolver.after_system().iter().cloned())
        {
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
    use std::collections::BTreeSet;
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

    /// A tool over `root` listing `files` (TOML array items) under `flags`
    /// (TOML strings).
    fn tool(root: &Path, files: &str, flags: &str) -> TargetContext {
        let config: harness_core::TargetConfig = toml::from_str(&format!(
            "schema_version = 2\n[target]\nname = \"t\"\nfiles = [{files}]\n\
             configuration = {{ name = \"make\", from = \"stated\", flags = [{flags}] }}\n\
             [oracle]\nallowlist = [\"cc\"]\n"
        ))
        .expect("config");
        TargetContext {
            ledger: root.join("migration/tools/t-x"),
            tool: Some("t-x".into()),
            root: root.to_path_buf(),
            config,
        }
    }

    /// What `cc -M` reads compiling the listed `rel` of `target`, and what
    /// the rule predicts.
    fn cc_and_rule(root: &Path, target: &TargetContext, rel: &str) -> [BTreeSet<String>; 2] {
        let base = Base::resolve(target, "u", &["cc"]).expect("base");
        let file = root.join(rel);
        let by_cc = read_by_cc(root, &base.file_args(&file).expect("args"), &file);
        let predicted = closure(target, &harness_core::Facts::default(), &[rel.into()])
            .into_iter()
            .collect();
        [by_cc, predicted]
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    /// A folder named by both `-I` (or a file's `include_dirs`) and
    /// `-isystem` is searched at its `-isystem` place; an `-idirafter`
    /// folder after the system, so the system's `<stdio.h>` wins over its
    /// own and a name the system lacks is taken from it.
    #[test]
    fn the_rule_reads_what_cc_reads_after_the_system_and_for_a_doubled_folder() {
        let tmp = TempDir::new("include-rule-after");
        let root = tmp.path().to_path_buf();
        put(
            &root,
            "m/main.c",
            "#include \"x.h\"\n#include \"ua.h\"\n#include <stdio.h>\n\
             #include \"after.h\"\n#include <after2.h>\nint main(void) { return 0; }\n",
        );
        // `a` is -I and -isystem: searched after the file's `b`.
        put(&root, "a/x.h", "/* a */\n");
        put(&root, "b/x.h", "/* b */\n");
        // `u` is an include_dirs and -isystem: searched after `a`.
        put(&root, "a/ua.h", "/* a */\n");
        put(&root, "u/ua.h", "/* u */\n");
        // `c` is -idirafter: after the system.
        put(&root, "c/stdio.h", "#error not the system's\n");
        put(&root, "c/after.h", "/* c */\n");
        put(&root, "c/after2.h", "/* c */\n");
        let target = tool(
            &root,
            "{ path = \"m/main.c\", include_dirs = [\"b\", \"u\"] }",
            "\"-Ia\", \"-isystemt\", \"-isystema\", \"-isystemu\", \"-idirafterc\"",
        );
        std::fs::create_dir_all(root.join("t")).expect("t");
        let [by_cc, predicted] = cc_and_rule(&root, &target, "m/main.c");
        let expected = set(&["m/main.c", "b/x.h", "a/ua.h", "c/after.h", "c/after2.h"]);
        assert_eq!(by_cc, expected, "what cc reads");
        assert_eq!(predicted, expected, "what the rule predicts");
        // The driver searches `c` after the system too, never as an `-I`.
        let unit: harness_core::Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"m/main.c\"]\nsymbols = [\"main\"]\n",
        )
        .expect("unit");
        let folders = crate::driver_folders(&target, &harness_core::Facts::default(), &unit);
        assert!(!folders.contains(&"c".to_string()), "{folders:?}");
        let names = crate::unit_header_names(&target, &harness_core::Facts::default(), &unit);
        assert!(names.contains(&"after.h".to_string()), "{names:?}");
    }

    /// The forms the 2026-10-08 check found unread — `#import`,
    /// `#include_next` (a header wrapping a same-named one in a later
    /// folder), `%:include`, a lone carriage return as a line end and a
    /// backslash with blanks before the line end — are read as `cc -M`
    /// reads them.
    #[test]
    fn the_reader_reads_every_form_cc_reads() {
        let tmp = TempDir::new("include-rule-forms");
        let root = tmp.path().to_path_buf();
        put(
            &root,
            "r/main.c",
            "#import \"imp.h\"\n%:include \"dig.h\"\n#inc\\  \nlude \"spl.h\"\n\
             #include \"cr.h\"\n#include <nx.h>\nint main(void) { return 0; }\n",
        );
        put(&root, "r/imp.h", "/* imp */\n");
        put(&root, "r/dig.h", "/* dig */\n");
        put(&root, "r/spl.h", "/* spl */\n");
        put(&root, "r/cr.h", "int q;\r#include \"crx.h\"\rint z;\r");
        put(&root, "r/crx.h", "/* crx */\n");
        put(&root, "k/nx.h", "#include_next <nx.h>\n");
        put(&root, "l/nx.h", "/* the next one */\n");
        let target = tool(
            &root,
            "{ path = \"r/main.c\", include_dirs = [\"k\", \"l\"] }",
            "",
        );
        let [by_cc, predicted] = cc_and_rule(&root, &target, "r/main.c");
        let expected = set(&[
            "r/main.c", "r/imp.h", "r/dig.h", "r/spl.h", "r/cr.h", "r/crx.h", "k/nx.h", "l/nx.h",
        ]);
        assert_eq!(by_cc, expected, "what cc reads");
        assert_eq!(predicted, expected, "what the rule predicts");
    }

    /// A header an ambiguous include lands on is in the unit's closure (the
    /// one `compute_inputs` hashes), though the scan records no edge for it.
    #[test]
    fn the_closure_holds_the_header_an_ambiguous_include_lands_on() {
        let tmp = TempDir::new("include-rule-ambiguous");
        let root = tmp.path().to_path_buf();
        put(
            &root,
            "a.c",
            "#include \"common.h\"\nint main(void) { return 0; }\n",
        );
        put(
            &root,
            "b.c",
            "#include \"common.h\"\nint b(void) { return 0; }\n",
        );
        put(&root, "inc/common.h", "#include \"cfg.h\"\n");
        put(&root, "d1/cfg.h", "/* d1 */\n");
        put(&root, "d2/cfg.h", "/* d2 */\n");
        let target = tool(
            &root,
            "{ path = \"a.c\", include_dirs = [\"inc\", \"d1\"] }, \
             { path = \"b.c\", include_dirs = [\"inc\", \"d2\"] }",
            "",
        );
        let resolver = Resolver::of(&target).expect("resolver").expect("a list");
        let program = resolver.walk(|rel| harness_core::sources::names_on_disk(&root, rel));
        assert_eq!(program.ambiguous.len(), 1, "{:?}", program.ambiguous);
        let [by_cc, predicted] = cc_and_rule(&root, &target, "a.c");
        assert_eq!(by_cc, set(&["a.c", "inc/common.h", "d1/cfg.h"]));
        assert_eq!(predicted, by_cc);
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
