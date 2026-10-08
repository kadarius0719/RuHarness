//! Integration tests for the c-treesitter-v1 detector suite: a synthetic
//! fixture exercising every category (including the ones zopfli cannot),
//! ground truth against the vendored zopfli target, and id stability under
//! line shifts.

use harness_core::config::TargetContext;
use harness_core::facts::{Facts, FileRecord, RefRecord, SymbolRecord};
use harness_core::hash::{file_hash, file_set_hash};
use harness_core::observer::{Finding, FindingsFile};
use harness_core::traits::Detector;
use harness_detect::CTreeSitterSuite;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn zopfli_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli")
}

const FIXTURE_H: &str = r#"#ifndef FIXTURE_H_
#define FIXTURE_H_

#define GLUE(a, b) a##b
#ifdef FIXTURE_FAST
#define APPEND(v, d, s) { (d)[(s)] = (v); (s) += 1; }
#else
#define APPEND(v, d, s) { (d)[(s)] = (v); (s) += 1; }
#endif
#define TAKE(p) { (p) = malloc(1); }
#define SMALL(x) ((x) + 1)

int guard(void);

#endif
"#;

const FIXTURE_C: &str = r#"#include "fixture.h"

static jmp_buf env;

union Both {
  int i;
  float f;
};

struct Flags {
  unsigned a : 1;
  unsigned b : 3;
};

static void worker(void) {}

static void handler(int sig) { (void)sig; }

int guard(void) {
  if (setjmp(env)) return 1;
  return 0;
}

void install(void) {
  signal(2, handler);
}

void spawn_thread(void) {
  pthread_t t;
  pthread_create(&t, 0, worker, 0);
}

int sum_all(int n, ...) {
  (void)n;
  return 0;
}

int vlog_all(int fmt, va_list ap) {
  va_list dst;
  va_copy(dst, ap);
  (void)fmt;
  return 0;
}

char* dup_bytes(int n) {
  return malloc((unsigned long)n);
}

void release(char* p) {
  free(p);
}

void rows_ok(int (*rows)[4]) {
  (void)rows;
}

int sig_guard(void) {
  sigjmp_buf senv;
  if (sigsetjmp(senv, 1)) return 1;
  siglongjmp(senv, 1);
  return 0;
}

void notify(void) {
  cnd_signal(0);
  call_once(0, 0);
}
"#;

/// Write the fixture target into `root`, with `c_prefix` prepended to
/// fixture.c (used by the id-stability test to shift every line).
fn write_fixture(root: &Path, c_prefix: &str) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("create fixture src dir");
    std::fs::write(
        root.join("harness.toml"),
        "schema_version = 1\n\n[target]\nname = \"fixture\"\nsource_dir = \"src\"\n",
    )
    .expect("write harness.toml");
    std::fs::write(src.join("fixture.h"), FIXTURE_H).expect("write fixture.h");
    std::fs::write(src.join("fixture.c"), format!("{c_prefix}{FIXTURE_C}"))
        .expect("write fixture.c");
}

/// Hand-built facts for the fixture (harness-detect cannot depend on
/// harness-scan). `line_shift` mirrors the prefix applied to fixture.c.
fn fixture_facts(root: &Path, line_shift: u32) -> Facts {
    let d = line_shift;
    let sym = |name: &str, vis: &str, sig: &str, span: (u32, u32)| SymbolRecord {
        name: name.to_string(),
        kind: "function".to_string(),
        file: "src/fixture.c".to_string(),
        visibility: vis.to_string(),
        signature: sig.to_string(),
        span: (span.0 + d, span.1 + d),
    };
    let call = |from: &str, to: &str, resolved: bool| RefRecord {
        from: from.to_string(),
        file: "src/fixture.c".to_string(),
        to: to.to_string(),
        refkind: "call".to_string(),
        resolved,
    };
    Facts {
        frontend: "c-tree-sitter".to_string(),
        files: vec![
            FileRecord {
                path: "src/fixture.c".to_string(),
                hash: file_hash(&root.join("src/fixture.c")).expect("hash fixture.c"),
                includes: vec!["src/fixture.h".to_string()],
            },
            FileRecord {
                path: "src/fixture.h".to_string(),
                hash: file_hash(&root.join("src/fixture.h")).expect("hash fixture.h"),
                includes: vec![],
            },
        ],
        symbols: vec![
            sym(
                "src/fixture.c::worker",
                "internal",
                "static void worker(void)",
                (15, 15),
            ),
            sym(
                "src/fixture.c::handler",
                "internal",
                "static void handler(int sig)",
                (17, 17),
            ),
            sym("guard", "public", "int guard(void)", (19, 22)),
            sym("install", "public", "void install(void)", (24, 26)),
            sym(
                "spawn_thread",
                "public",
                "void spawn_thread(void)",
                (28, 31),
            ),
            sym("sum_all", "public", "int sum_all(int n, ...)", (33, 36)),
            sym(
                "vlog_all",
                "public",
                "int vlog_all(int fmt, va_list ap)",
                (38, 43),
            ),
            sym("dup_bytes", "public", "char* dup_bytes(int n)", (45, 47)),
            sym("release", "public", "void release(char* p)", (49, 51)),
            sym(
                "rows_ok",
                "public",
                "void rows_ok(int (*rows)[4])",
                (53, 55),
            ),
            sym("sig_guard", "public", "int sig_guard(void)", (57, 62)),
            sym("notify", "public", "void notify(void)", (64, 67)),
        ],
        refs: vec![
            call("guard", "setjmp", false),
            call("install", "signal", false),
            call("spawn_thread", "pthread_create", false),
            call("vlog_all", "va_copy", false),
            call("dup_bytes", "malloc", false),
            call("release", "free", false),
            call("sig_guard", "sigsetjmp", false),
            call("sig_guard", "siglongjmp", false),
            call("notify", "cnd_signal", false),
            call("notify", "call_once", false),
        ],
    }
}

/// Write a one-file target (`src/snip.c`) and run the suite over it with
/// hand-built symbols/refs (all attributed to `src/snip.c`).
fn detect_snippet(
    name: &str,
    c_src: &str,
    symbols: Vec<SymbolRecord>,
    refs: Vec<RefRecord>,
) -> Vec<Finding> {
    let root = temp_target(name);
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("create snippet src dir");
    std::fs::write(
        root.join("harness.toml"),
        "schema_version = 1\n\n[target]\nname = \"snippet\"\nsource_dir = \"src\"\n",
    )
    .expect("write harness.toml");
    std::fs::write(src.join("snip.c"), c_src).expect("write snip.c");
    let target = TargetContext::load(&root).expect("load snippet target");
    let facts = Facts {
        frontend: "c-tree-sitter".to_string(),
        files: vec![FileRecord {
            path: "src/snip.c".to_string(),
            hash: file_hash(&src.join("snip.c")).expect("hash snip.c"),
            includes: vec![],
        }],
        symbols,
        refs,
    };
    let findings = CTreeSitterSuite
        .detect(&target, &facts)
        .expect("detect snippet");
    let _ = std::fs::remove_dir_all(&root);
    findings
}

fn snip_fn(name: &str, signature: &str, span: (u32, u32)) -> SymbolRecord {
    SymbolRecord {
        name: name.to_string(),
        kind: "function".to_string(),
        file: "src/snip.c".to_string(),
        visibility: "public".to_string(),
        signature: signature.to_string(),
        span,
    }
}

fn snip_call(from: &str, to: &str) -> RefRecord {
    RefRecord {
        from: from.to_string(),
        file: "src/snip.c".to_string(),
        to: to.to_string(),
        refkind: "call".to_string(),
        resolved: false,
    }
}

/// The evidence strings of every finding in `category`.
fn evidence_of<'a>(findings: &'a [Finding], category: &str) -> BTreeSet<&'a str> {
    findings
        .iter()
        .filter(|f| f.category == category)
        .map(|f| f.evidence.as_str())
        .collect()
}

fn temp_target(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("harness-detect-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn by_category<'a>(findings: &'a [Finding], category: &str) -> Vec<&'a Finding> {
    findings.iter().filter(|f| f.category == category).collect()
}

#[test]
fn synthetic_fixture_covers_every_category() {
    let root = temp_target("fixture");
    write_fixture(&root, "");
    let target = TargetContext::load(&root).expect("load fixture target");
    let facts = fixture_facts(&root, 0);
    let findings = CTreeSitterSuite.detect(&target, &facts).expect("detect");

    // nonlocal: setjmp/sigsetjmp/siglongjmp + signal, all blocker +
    // human-mandatory, one finding per (callee, caller).
    for (category, callees) in [
        ("setjmp-longjmp", &["setjmp", "sigsetjmp", "siglongjmp"][..]),
        ("signal-handler", &["signal"][..]),
    ] {
        let hits = by_category(&findings, category);
        assert_eq!(hits.len(), callees.len(), "{category}: {hits:?}");
        for callee in callees {
            assert!(
                hits.iter()
                    .any(|f| f.message.starts_with(&format!("call to {callee}:"))),
                "{category}: no finding for {callee}: {hits:?}"
            );
        }
        for f in hits {
            assert_eq!(f.detector, "nonlocal");
            assert_eq!(f.severity, "high");
            assert!(f.blocker, "{category} must be a blocker");
            assert!(f.human_mandatory, "{category} must be human-mandatory");
            assert_eq!(f.file, "src/fixture.c");
        }
    }

    // concurrency: pthread_create, cnd_signal, call_once — blocker +
    // human-mandatory.
    let threading = by_category(&findings, "threading-api");
    assert_eq!(threading.len(), 3, "{threading:?}");
    for f in &threading {
        assert_eq!(f.detector, "concurrency");
        assert_eq!(f.severity, "high");
        assert!(f.blocker && f.human_mandatory);
    }

    // layout: union + bitfield, human-mandatory but NOT blockers.
    let unions = by_category(&findings, "union-decl");
    assert_eq!(unions.len(), 1, "{unions:?}");
    assert_eq!(unions[0].detector, "layout");
    assert_eq!(unions[0].severity, "high");
    assert!(!unions[0].blocker && unions[0].human_mandatory);
    assert!(unions[0].evidence.contains("union Both"));

    let bitfields = by_category(&findings, "bitfield");
    assert!(!bitfields.is_empty(), "expected bitfield findings");
    for f in &bitfields {
        assert_eq!(f.detector, "layout");
        assert_eq!(f.severity, "high");
        assert!(!f.blocker && f.human_mandatory);
    }

    // variadic: sum_all (ellipsis) + vlog_all (va_copy ref, folded) — one
    // finding per function, never a separate va_* finding.
    let variadic = by_category(&findings, "variadic-function");
    assert_eq!(variadic.len(), 2, "{variadic:?}");
    for f in &variadic {
        assert_eq!(f.detector, "variadic");
        assert_eq!(f.severity, "medium");
        assert!(!f.blocker && !f.human_mandatory);
    }
    assert!(variadic.iter().any(|f| f.evidence.contains("sum_all")));
    assert!(variadic.iter().any(|f| f.evidence.contains("vlog_all")));

    // macros: token pasting (high), byte-identical statement-body twins with
    // occurrence 0/1, alloc-carrying statement body (high), plain (low).
    let pasting = by_category(&findings, "macro-token-pasting");
    assert_eq!(pasting.len(), 1, "{pasting:?}");
    assert_eq!(pasting[0].detector, "macros");
    assert_eq!(pasting[0].severity, "high");
    assert!(!pasting[0].blocker && !pasting[0].human_mandatory);
    assert!(pasting[0].evidence.contains("GLUE"));

    let stmt = by_category(&findings, "macro-statement-body");
    let twins: Vec<&&Finding> = stmt
        .iter()
        .filter(|f| f.evidence.contains("APPEND"))
        .collect();
    assert_eq!(twins.len(), 2, "{twins:?}");
    let mut occurrences: Vec<u32> = twins.iter().map(|f| f.occurrence).collect();
    occurrences.sort_unstable();
    assert_eq!(
        occurrences,
        vec![0, 1],
        "byte-identical macro twins disambiguate via occurrence"
    );
    assert_ne!(twins[0].id, twins[1].id);
    for f in &twins {
        assert_eq!(f.severity, "medium", "no allocation in APPEND body");
    }
    let take: Vec<&&Finding> = stmt
        .iter()
        .filter(|f| f.evidence.contains("TAKE"))
        .collect();
    assert_eq!(take.len(), 1);
    assert_eq!(take[0].severity, "high", "malloc in body upgrades severity");

    let plain = by_category(&findings, "macro-function-like");
    assert_eq!(plain.len(), 1, "{plain:?}");
    assert_eq!(plain[0].severity, "low");
    assert!(plain[0].evidence.contains("SMALL"));

    // global: exactly the mutable static (unions/structs/functions excluded).
    let globals = by_category(&findings, "global-mutable");
    assert_eq!(globals.len(), 1, "{globals:?}");
    assert_eq!(globals[0].detector, "global");
    assert_eq!(globals[0].severity, "high");
    assert!(globals[0].evidence.contains("env"));

    // alloc: dup_bytes (malloc + pointer return) and release (free).
    let alloc = by_category(&findings, "alloc-ownership");
    assert_eq!(alloc.len(), 2, "{alloc:?}");
    for f in &alloc {
        assert_eq!(f.detector, "alloc");
        assert_eq!(f.severity, "info");
        assert!(!f.blocker && !f.human_mandatory);
    }

    // fn-pointer: handler + worker passed as bare-identifier arguments; the
    // pointer-to-array parameter `int (*rows)[4]` must NOT produce a decl
    // finding (explicit negative).
    let args = by_category(&findings, "function-pointer-arg");
    assert_eq!(args.len(), 2, "{args:?}");
    assert!(args.iter().any(|f| f.evidence.contains("handler")));
    assert!(args.iter().any(|f| f.evidence.contains("worker")));
    assert!(
        by_category(&findings, "function-pointer-decl").is_empty(),
        "pointer-to-array parameter must not fire function-pointer-decl"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ids_survive_line_shifts() {
    let root_a = temp_target("shift-a");
    write_fixture(&root_a, "");
    let target_a = TargetContext::load(&root_a).expect("load fixture A");
    let facts_a = fixture_facts(&root_a, 0);
    let findings_a = CTreeSitterSuite
        .detect(&target_a, &facts_a)
        .expect("detect A");

    // Same fixture with two comment lines prepended to fixture.c: every span
    // in the .c shifts, the facts record its new hash and spans, ids hold.
    let root_b = temp_target("shift-b");
    write_fixture(&root_b, "/* shifted */\n/* shifted again */\n");
    let target_b = TargetContext::load(&root_b).expect("load fixture B");
    let facts_b = fixture_facts(&root_b, 2);
    let findings_b = CTreeSitterSuite
        .detect(&target_b, &facts_b)
        .expect("detect B");

    let ids = |fs: &[Finding], file: &str| -> BTreeSet<String> {
        fs.iter()
            .filter(|f| f.file == file)
            .map(|f| f.id.clone())
            .collect()
    };
    assert_eq!(
        ids(&findings_a, "src/fixture.c"),
        ids(&findings_b, "src/fixture.c"),
        "ids in the shifted file must not change"
    );
    assert_eq!(
        ids(&findings_a, "src/fixture.h"),
        ids(&findings_b, "src/fixture.h"),
        "ids in the untouched header must not change"
    );

    // Not vacuous: the spans really did shift (tree-driven and facts-driven).
    let union_a = &by_category(&findings_a, "union-decl")[0].span;
    let union_b = &by_category(&findings_b, "union-decl")[0].span;
    assert_eq!((union_a.0 + 2, union_a.1 + 2), *union_b);
    let setjmp_a = &by_category(&findings_a, "setjmp-longjmp")[0].span;
    let setjmp_b = &by_category(&findings_b, "setjmp-longjmp")[0].span;
    assert_eq!((setjmp_a.0 + 2, setjmp_a.1 + 2), *setjmp_b);
    // And the file hash binding did change with the content.
    assert_ne!(
        by_category(&findings_a, "union-decl")[0].file_hash,
        by_category(&findings_b, "union-decl")[0].file_hash
    );

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[test]
fn nonlocal_and_concurrency_name_lists_cover_sig_and_c11_apis() {
    let root = temp_target("name-lists");
    write_fixture(&root, "");
    let target = TargetContext::load(&root).expect("load fixture target");
    let facts = fixture_facts(&root, 0);
    let findings = CTreeSitterSuite.detect(&target, &facts).expect("detect");
    let _ = std::fs::remove_dir_all(&root);

    let find = |category: &str, callee: &str| -> Finding {
        by_category(&findings, category)
            .into_iter()
            .find(|f| f.message.starts_with(&format!("call to {callee}:")))
            .cloned()
            .unwrap_or_else(|| panic!("no {category} finding for {callee}"))
    };
    // sigsetjmp/siglongjmp pair in one caller: two findings, distinct ids
    // (identity bytes are callee ‖ NUL ‖ caller), same span.
    let sigset = find("setjmp-longjmp", "sigsetjmp");
    let siglong = find("setjmp-longjmp", "siglongjmp");
    assert_ne!(sigset.id, siglong.id);
    assert_eq!(sigset.span, (57, 62));
    assert_eq!(siglong.span, (57, 62));
    assert!(sigset.evidence.contains("sig_guard"));
    // C11 threads.h: `cnd_` prefix and the exact name `call_once`.
    let cnd = find("threading-api", "cnd_signal");
    let once = find("threading-api", "call_once");
    assert_ne!(cnd.id, once.id);
    assert_eq!(cnd.span, (64, 67));
    assert!(once.evidence.contains("notify"));
}

const GLOBALS_C: &str = r#"const char *p1;
char *const p2 = "x";
extern int t1;
extern int t2 = 5;
static const unsigned long table[4] = {1, 2, 3, 4};
static int counter;
const char *const *pp;
void (*const cb)(int) = 0;
char *const arr[3];
int proto(int);
"#;

#[test]
fn global_mutable_is_decided_at_the_declarator_binding_the_name() {
    let findings = detect_snippet("global-const", GLOBALS_C, vec![], vec![]);
    let expected: BTreeSet<&str> = [
        // Top-level `const` qualifies the pointee; the pointer is mutable.
        "const char *p1;",
        // `extern` WITH an initializer is a definition.
        "extern int t2 = 5;",
        "static int counter;",
        // The pointer layer binding `pp` (the rightmost `*`) is unqualified.
        "const char *const *pp;",
    ]
    .into_iter()
    .collect();
    // Quiet: `char *const p2` (const object), `extern int t1;` (declaration
    // only), `static const ... table[4]` (const array, the zopfli
    // crc32_table shape), `void (*const cb)(int)` (const function
    // pointer), `char *const arr[3]` (array of const pointers), and the
    // `proto` prototype.
    assert_eq!(evidence_of(&findings, "global-mutable"), expected);
}

const FN_POINTER_C: &str = r#"typedef cmp_t chained_t;
typedef int cmp_t(const void *, const void *);
void f(void (*)(int));
void g(chained_t *c);
void rows_ok(int (*rows)[4]);
struct vtable {
  int (*read)(void *);
  int n;
};
union events {
  void (*cb)(void);
  int i;
};
typedef struct {
  int (*write)(void *);
  int len;
} vt_t;
"#;

#[test]
fn fn_pointer_decl_covers_unnamed_params_chained_typedefs_and_fields() {
    let findings = detect_snippet("fn-pointer-holes", FN_POINTER_C, vec![], vec![]);
    let expected: BTreeSet<&str> = [
        "typedef int cmp_t(const void *, const void *);",
        // (b) alias of a function typedef — recorded even though it is
        // declared before its base, and flagged itself.
        "typedef cmp_t chained_t;",
        // (a) unnamed parameter parsed as abstract_function_declarator.
        "void (*)(int)",
        // (b) parameter of the chained alias type.
        "chained_t *c",
        // (c) struct/union fields outside any typedef.
        "int (*read)(void *);",
        "void (*cb)(void);",
        // (c) inside a typedef'd struct: the field is the one finding — the
        // enclosing `typedef struct {...} vt_t;` must not fire again.
        "int (*write)(void *);",
    ]
    .into_iter()
    .collect();
    assert_eq!(evidence_of(&findings, "function-pointer-decl"), expected);
    let alias = findings
        .iter()
        .find(|f| f.evidence == "typedef cmp_t chained_t;")
        .expect("alias finding");
    assert!(
        alias.message.contains("chained_t aliases a function type"),
        "message: {}",
        alias.message
    );
}

#[test]
fn alloc_pointer_return_is_read_from_the_tree_not_the_signature() {
    let c = "__attribute__((malloc)) char *adup(int n) {\n  return malloc((unsigned long)n);\n}\n\nint count(int n) {\n  char *p = malloc(1);\n  (void)p;\n  return n;\n}\n";
    let findings = detect_snippet(
        "alloc-attribute",
        c,
        vec![
            // Signature as the scanner records it: the attribute's `(`
            // comes before the `*`, which defeated the old string check.
            snip_fn("adup", "__attribute__((malloc)) char *adup(int n)", (1, 3)),
            snip_fn("count", "int count(int n)", (5, 9)),
            // Not in the tree at all: exercises the signature fallback,
            // which must see through the attribute and the comment.
            snip_fn(
                "phantom",
                "__attribute__((malloc)) /* owner */ char *phantom(void)",
                (1, 1),
            ),
        ],
        vec![
            snip_call("adup", "malloc"),
            snip_call("count", "malloc"),
            snip_call("phantom", "malloc"),
        ],
    );
    let expected: BTreeSet<&str> = [
        "__attribute__((malloc)) char *adup(int n)",
        "__attribute__((malloc)) /* owner */ char *phantom(void)",
    ]
    .into_iter()
    .collect();
    assert_eq!(evidence_of(&findings, "alloc-ownership"), expected);
    for f in by_category(&findings, "alloc-ownership") {
        assert!(
            f.message.contains("pointer return"),
            "message: {}",
            f.message
        );
    }
}

#[test]
fn macro_spans_are_end_inclusive_everywhere() {
    let c = "#define MID(a, b) \\\n  ((a) + (b))\n#define ONE(a) (a)\nint x;\n#define TAIL(a) \\\n  ((a) * 2)\n";
    for (name, src) in [
        ("macro-span-nl", c),
        ("macro-span-eof", c.trim_end_matches('\n')),
    ] {
        let findings = detect_snippet(name, src, vec![], vec![]);
        let span_of = |macro_name: &str| -> (u32, u32) {
            findings
                .iter()
                .find(|f| f.detector == "macros" && f.evidence.contains(macro_name))
                .unwrap_or_else(|| panic!("{name}: no macro finding for {macro_name}"))
                .span
        };
        // Mid-file continued, single-line, and EOF continued macros all
        // report their last physical line inclusively.
        assert_eq!(span_of("MID("), (1, 2), "{name}");
        assert_eq!(span_of("ONE("), (3, 3), "{name}");
        assert_eq!(span_of("TAIL("), (5, 6), "{name}");
    }
}

#[test]
fn ground_truth_zopfli() {
    harness_core::adopt::testing::adopt(zopfli_root());
    let target = TargetContext::load(zopfli_root()).expect("load zopfli target");
    let facts =
        Facts::load(&zopfli_root().join("migration/facts.jsonl")).expect("load committed facts");
    let suite = CTreeSitterSuite;
    let findings = suite.detect(&target, &facts).expect("detect");

    // ZOPFLI_APPEND_DATA: exactly two statement-body findings in util.h,
    // both high (embedded malloc/realloc). The two #ifdef branches differ in
    // body bytes (reinterpret_cast vs plain assignment), so per the
    // docs/SCHEMAS.md id rule each is occurrence 0 of its own spanned-bytes
    // key — occurrence disambiguation of byte-identical twins is covered by
    // the synthetic fixture test.
    let util_stmt: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.file == "src/zopfli/util.h" && f.category == "macro-statement-body")
        .collect();
    assert_eq!(util_stmt.len(), 2, "{util_stmt:?}");
    for f in &util_stmt {
        assert_eq!(f.detector, "macros");
        assert_eq!(
            f.severity, "high",
            "embedded malloc/realloc upgrades severity"
        );
        assert!(f.evidence.contains("ZOPFLI_APPEND_DATA"));
    }
    assert_ne!(util_stmt[0].id, util_stmt[1].id);
    let mut occ: Vec<u32> = util_stmt.iter().map(|f| f.occurrence).collect();
    occ.sort_unstable();
    assert_eq!(
        occ,
        vec![0, 0],
        "distinct spanned bytes each start at occurrence 0"
    );
    // Spans are end-inclusive physical lines: util.h defines the two
    // branches on 135-144 and 146-154 (the `#else` on 145 is not part of
    // either definition).
    let spans: BTreeSet<(u32, u32)> = util_stmt.iter().map(|f| f.span).collect();
    assert_eq!(spans, [(135, 144), (146, 154)].into_iter().collect());

    // Function-type typedefs: FindMinimumFun (blocksplitter.c), CostModelFun
    // (squeeze.c) — the non-(*)-style typedef shape must be caught.
    for (file, name) in [
        ("src/zopfli/blocksplitter.c", "FindMinimumFun"),
        ("src/zopfli/squeeze.c", "CostModelFun"),
    ] {
        assert!(
            findings.iter().any(|f| f.file == file
                && f.category == "function-pointer-decl"
                && f.evidence.contains(name)),
            "expected function-pointer-decl for {name} in {file}"
        );
    }

    // LeafComparator passed to qsort in katajainen.c.
    assert!(
        findings.iter().any(|f| f.file == "src/zopfli/katajainen.c"
            && f.category == "function-pointer-arg"
            && f.evidence.contains("LeafComparator")),
        "expected function-pointer-arg for LeafComparator"
    );
    // Explicit negative: `Node* (*lists)[2]` is a pointer-to-array parameter
    // and must not produce any function-pointer-decl in katajainen.c.
    assert!(
        !findings
            .iter()
            .any(|f| f.file == "src/zopfli/katajainen.c" && f.category == "function-pointer-decl"),
        "pointer-to-array parameter fired function-pointer-decl"
    );

    // Categories zopfli must not trigger: crc32_table is static const (not a
    // mutable global); no bitfields, unions, setjmp, signals, or threading.
    for category in [
        "global-mutable",
        "bitfield",
        "union-decl",
        "setjmp-longjmp",
        "signal-handler",
        "threading-api",
    ] {
        let hits = by_category(&findings, category);
        assert!(hits.is_empty(), "unexpected {category} findings: {hits:?}");
    }

    // Alloc ownership: one finding per function, spread over >= 8 files.
    let alloc_files: BTreeSet<&str> = findings
        .iter()
        .filter(|f| f.category == "alloc-ownership")
        .map(|f| f.file.as_str())
        .collect();
    assert!(alloc_files.len() >= 8, "alloc files: {alloc_files:?}");
    for f in findings.iter().filter(|f| f.category == "alloc-ownership") {
        assert_eq!(f.severity, "info");
        assert_eq!(
            f.occurrence, 0,
            "alloc findings are per-function (canonical-id keyed)"
        );
    }

    // file_hash freshness binding matches the committed facts records.
    for f in &findings {
        let record = facts
            .files
            .iter()
            .find(|r| r.path == f.file)
            .unwrap_or_else(|| panic!("finding in unscanned file {}", f.file));
        assert_eq!(f.file_hash, record.hash, "hash mismatch for {}", f.file);
    }

    // Determinism: two runs produce byte-identical canonical findings files.
    let findings_again = suite.detect(&target, &facts).expect("second detect");
    let facts_hash = file_set_hash(
        &facts
            .files
            .iter()
            .map(|r| (r.path.clone(), r.hash.clone()))
            .collect::<Vec<_>>(),
    );
    let file_a = FindingsFile {
        detector_suite: suite.name().to_string(),
        facts_hash: facts_hash.clone(),
        findings,
    };
    let file_b = FindingsFile {
        detector_suite: suite.name().to_string(),
        facts_hash,
        findings: findings_again,
    };
    assert_eq!(
        file_a.to_canonical_bytes().expect("canonical A"),
        file_b.to_canonical_bytes().expect("canonical B"),
        "detect must be deterministic"
    );
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn record(root: &Path, path: &str, includes: &[&str]) -> FileRecord {
    FileRecord {
        path: path.into(),
        hash: file_hash(&root.join(path)).unwrap(),
        includes: includes.iter().map(|s| s.to_string()).collect(),
    }
}

/// The files the findings are in, sorted, each once.
fn files_of(findings: &[Finding]) -> Vec<&str> {
    findings
        .iter()
        .map(|f| f.file.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// docs/PROJECT-MAP-DESIGN.md §3.7: a file-list target is read through its
/// listed files and the headers the facts record them reaching — a hazard
/// in a reached header is found; a header nothing includes, and the
/// ledger's model-written file, are never read.
#[test]
fn a_file_list_target_is_read_through_its_files_and_their_reached_headers() {
    let root = temp_target("file-list");
    put(
        &root,
        "src/lib/encode.c",
        "#include \"internal.h\"\nint lzg_level = 3;\n",
    );
    put(
        &root,
        "src/lib/internal.h",
        "#include \"../include/lzg.h\"\n",
    );
    put(
        &root,
        "src/include/lzg.h",
        "#define LZG_MAX(a, b) ((a) > (b) ? (a) : (b))\n",
    );
    put(
        &root,
        "src/tools/lzg.c",
        "#include <lzg.h>\nstatic void say(const char *fmt, ...) { (void)fmt; }\n",
    );
    put(
        &root,
        "src/include/unused.h",
        "#define NEVER_READ(x) ((x) + 1)\nint unused_global;\n",
    );
    let root = root.canonicalize().unwrap();
    let ledger = harness_core::config::tool_dir(&root, "t-lzg");
    put(&ledger, "driver.c", "int model_global;\n");
    put(
        &ledger,
        "harness.toml",
        "schema_version = 2\n[target]\nname = \"lzg\"\nfiles = [\n\
         { path = \"src/lib/encode.c\", include_dirs = [\"src/include\"] },\n\
         { path = \"src/tools/lzg.c\", include_dirs = [\"src/include\"] },\n]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [] }\n",
    );
    let config =
        harness_core::config::TargetConfig::load_file(&ledger.join("harness.toml"), &root).unwrap();
    let ctx = TargetContext {
        root: root.clone(),
        ledger,
        tool: Some("t-lzg".into()),
        config,
    };
    // What the scanner records for it (harness-scan's own tests prove that).
    let mut facts = Facts {
        frontend: "c-tree-sitter".into(),
        files: vec![
            record(&root, "src/include/lzg.h", &[]),
            record(&root, "src/lib/encode.c", &["src/lib/internal.h"]),
            record(&root, "src/lib/internal.h", &["src/include/lzg.h"]),
            record(&root, "src/tools/lzg.c", &["src/include/lzg.h"]),
        ],
        ..Facts::default()
    };
    facts.symbols.push(SymbolRecord {
        name: "src/tools/lzg.c::say".into(),
        kind: "function".into(),
        file: "src/tools/lzg.c".into(),
        visibility: "internal".into(),
        signature: "static void say(const char *fmt, ...)".into(),
        span: (2, 2),
    });
    let findings = CTreeSitterSuite.detect(&ctx, &facts).expect("detect");
    assert_eq!(
        files_of(&findings),
        ["src/include/lzg.h", "src/lib/encode.c", "src/tools/lzg.c"],
        "{findings:#?}"
    );
    for (file, group) in [
        ("src/include/lzg.h", "macros"),
        ("src/lib/encode.c", "global"),
        ("src/tools/lzg.c", "variadic"),
    ] {
        assert!(
            findings
                .iter()
                .any(|f| f.file == file && f.detector == group),
            "{file} {group}: {findings:#?}"
        );
    }
    // A record the facts hold beyond the list and its headers is not read.
    facts.files.push(record(&root, "src/include/unused.h", &[]));
    let again = CTreeSitterSuite.detect(&ctx, &facts).expect("detect");
    assert_eq!(files_of(&again), files_of(&findings));
    let _ = std::fs::remove_dir_all(&root);
}

/// With `source_dir = "."` the ledger lies inside the folder: the
/// detectors prune it, as the scanner does.
#[test]
fn the_detectors_prune_the_ledger_when_source_dir_is_the_root() {
    let root = temp_target("dot");
    put(&root, "a.c", "int a(void) { return 0; }\n");
    put(&root, "migration/driver.c", "int model_global;\n");
    put(
        &root,
        "harness.toml",
        "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \".\"\n",
    );
    let root = root.canonicalize().unwrap();
    let ctx = TargetContext::folder_form(
        root.clone(),
        harness_core::config::TargetConfig::load(&root).unwrap(),
    );
    let facts = Facts {
        frontend: "c-tree-sitter".into(),
        files: vec![record(&root, "a.c", &[])],
        ..Facts::default()
    };
    let findings = CTreeSitterSuite.detect(&ctx, &facts).expect("detect");
    assert!(findings.is_empty(), "{findings:#?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The readers review, finding 5: detect reads what the include rule
/// reaches today, never `facts.jsonl`'s edges, which the target owns. A
/// forged edge from a listed file to another project file, or into a
/// dot-folder, reads nothing.
#[test]
fn a_forged_include_edge_reads_nothing() {
    let root = temp_target("forged");
    put(&root, "src/a.c", "#include \"a.h\"\nint a_global;\n");
    put(
        &root,
        "src/a.h",
        "#define A_MAX(a, b) ((a) > (b) ? (a) : (b))\n",
    );
    put(&root, "notes/secret.h", "int secret_global;\n");
    put(&root, ".env/secret.h", "int env_global;\n");
    let root = root.canonicalize().unwrap();
    let ledger = harness_core::config::tool_dir(&root, "t-a");
    put(
        &ledger,
        "harness.toml",
        "schema_version = 2\n[target]\nname = \"a\"\nfiles = [{ path = \"src/a.c\" }]\n\
         configuration = { name = \"make\", from = \"stated\", flags = [] }\n",
    );
    let config =
        harness_core::config::TargetConfig::load_file(&ledger.join("harness.toml"), &root).unwrap();
    let ctx = TargetContext {
        root: root.clone(),
        ledger,
        tool: Some("t-a".into()),
        config,
    };
    let facts = Facts {
        frontend: "c-tree-sitter".into(),
        files: vec![
            record(
                &root,
                "src/a.c",
                &["src/a.h", "notes/secret.h", ".env/secret.h"],
            ),
            record(&root, "src/a.h", &[]),
            record(&root, "notes/secret.h", &[]),
            record(&root, ".env/secret.h", &[]),
        ],
        ..Facts::default()
    };
    let findings = CTreeSitterSuite.detect(&ctx, &facts).expect("detect");
    assert_eq!(files_of(&findings), ["src/a.c", "src/a.h"], "{findings:#?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The readers review, finding 4: a file that cannot be read is skipped
/// with the scan's note ("cannot be read: …"), never a stop.
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_skipped_with_a_note() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_target("unreadable");
    put(&root, "src/a.c", "int a_global;\n");
    put(&root, "src/b.c", "int b_global;\n");
    put(
        &root,
        "harness.toml",
        "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n",
    );
    let root = root.canonicalize().unwrap();
    let b = root.join("src/b.c");
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&b).is_ok() {
        // Run as root: nothing is unreadable.
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let ctx = TargetContext::folder_form(
        root.clone(),
        harness_core::config::TargetConfig::load(&root).unwrap(),
    );
    let facts = Facts {
        frontend: "c-tree-sitter".into(),
        files: vec![record(&root, "src/a.c", &[])],
        ..Facts::default()
    };
    let result = CTreeSitterSuite.detect_reporting(&ctx, &facts);
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (findings, skipped) = result.expect("detect goes on");
    assert_eq!(files_of(&findings), ["src/a.c"], "{findings:#?}");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(skipped[0].0, b);
    // The scan's note word for word (harness-core's `unreadable_note`;
    // harness-scan's `a_non_utf8_file_and_an_unreadable_folder_are_noted…`
    // checks the same words), with no machine path in it.
    assert_eq!(
        skipped[0].1, "cannot be read: Permission denied (os error 13); recorded as unreadable",
        "{skipped:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The 2026-10-08 check: a 400 KB line `int x = 0 < 1 < 1 …;` nests the
/// tree tens of thousands deep. Every detector walk keeps its own stack, so
/// detect reads the file (a mutable global, and a function pointer after
/// it) instead of overflowing the thread's.
#[test]
fn a_deeply_nested_expression_is_walked_without_overflowing() {
    let mut deep = String::from("int x = 0");
    deep.push_str(&" < 1".repeat(100_000));
    deep.push_str(";\nvoid take(void (*cb)(int));\n");
    assert!(deep.len() > 400_000);
    let findings = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || detect_snippet("deep", &deep, vec![], vec![]))
        .expect("thread")
        .join()
        .expect("no overflow");
    assert!(
        !by_category(&findings, "global-mutable").is_empty(),
        "{findings:#?}"
    );
    assert!(
        !by_category(&findings, "function-pointer-decl").is_empty(),
        "{findings:#?}"
    );
}
