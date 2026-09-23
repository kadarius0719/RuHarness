//! The `driver-shape` gate (docs/M4-DESIGN.md §R1): a differential driver
//! runs on BOTH sides of the oracle, so it must not be able to tell them
//! apart. Run by every `verify` (human drivers included) right after the
//! symbol-set check, and by `validate_driver`; a failure ends the run
//! before any driver is linked.
//!
//! Two halves:
//! - **object check** — the driver compiled ALONE to an object (`cc -c`,
//!   the same flags as its real builds) must define exactly `{main}` as
//!   external symbols; everything it leaves undefined must be a unit symbol
//!   or on [`DRIVER_LIBC_ALLOWLIST`] (no file, env, time, dl, process or
//!   syscall function is reachable); and it may carry no weak reference or
//!   definition (a weak ref to a Rust-runtime symbol is the classic
//!   "which side am I on?" probe);
//! - **source lint** — [`harness_scan::lint_driver`] with the unit's symbols
//!   and the paths/basenames of the unit's include-closure headers.

use crate::exec::Runner;
use crate::symbols::{nm_argv, normalize, parse_nm};
use harness_core::error::Error;
use harness_core::verdict::Check;
use std::collections::BTreeSet;
use std::path::Path;

/// Name of the check in verdicts and validation records.
pub(crate) const CHECK_NAME: &str = "driver-shape";

/// How many violations a failure detail lists.
const DETAIL_LIMIT: usize = 12;

/// The C library surface a driver object may leave undefined (names as C
/// sees them — the extra leading underscore of Mach-O is stripped first).
pub(crate) struct LibcAllowlist {
    /// Names allowed verbatim.
    pub exact: &'static [&'static str],
    /// libm functions, each allowed with its `f` and `l` variants too.
    pub libm: &'static [&'static str],
}

/// The ONE list of what a driver may call besides the unit's own symbols.
/// A driver prints to stdout/stderr, computes, allocates — nothing else:
///
/// - the stdout/stderr printing family (there is no `fopen`, and `stdin` is
///   not listed, so the only reachable streams are stdout and stderr);
/// - pure memory/string/conversion functions, `qsort`/`bsearch`,
///   `malloc`/`calloc`/`realloc`/`free`, the `abs`/`div` families,
///   `exit`/`abort`;
/// - libm with its `f`/`l` variants, the errno accessor, `<ctype.h>` support;
/// - compiler-emitted helpers: stack protector, `__chkstk_darwin`,
///   `memset_pattern*`, `bzero`, macOS `__sincos_stret`-style math, glibc's
///   classification helpers and C23 `strto*` aliases.
///
/// Additionally the `_FORTIFY_SOURCE` variant `__<name>_chk` of any listed
/// name is allowed (the compiler rewrites `sprintf` to `__sprintf_chk`, …).
pub(crate) const DRIVER_LIBC_ALLOWLIST: LibcAllowlist = LibcAllowlist {
    exact: &[
        // stdout / stderr printing
        "printf",
        "puts",
        "putchar",
        "fputs",
        "fputc",
        "putc",
        "fwrite",
        "fflush",
        "fprintf",
        "snprintf",
        "sprintf",
        "vprintf",
        "vfprintf",
        "vsnprintf",
        "vsprintf",
        "__stdoutp",
        "__stderrp",
        "stdout",
        "stderr",
        // pure memory / string / conversion
        "memcpy",
        "memmove",
        "memset",
        "memcmp",
        "memchr",
        "bzero",
        "__bzero",
        "memset_pattern4",
        "memset_pattern8",
        "memset_pattern16",
        "strlen",
        "strnlen",
        "strcmp",
        "strncmp",
        "strcpy",
        "strncpy",
        "strcat",
        "strncat",
        "strchr",
        "strrchr",
        "strstr",
        "strspn",
        "strcspn",
        "strpbrk",
        "strdup",
        "strndup",
        "stpcpy",
        "stpncpy",
        "strlcpy",
        "strlcat",
        "strtol",
        "strtoul",
        "strtoll",
        "strtoull",
        "strtod",
        "strtof",
        "strtold",
        "__isoc23_strtol",
        "__isoc23_strtoul",
        "__isoc23_strtoll",
        "__isoc23_strtoull",
        "atoi",
        "atol",
        "atoll",
        "atof",
        "qsort",
        "bsearch",
        // heap
        "malloc",
        "calloc",
        "realloc",
        "free",
        // integer helpers, termination
        "abs",
        "labs",
        "llabs",
        "div",
        "ldiv",
        "lldiv",
        "exit",
        "abort",
        // errno
        "__error",
        "__errno_location",
        // <ctype.h>
        "__maskrune",
        "__tolower",
        "__toupper",
        "_DefaultRuneLocale",
        "__ctype_b_loc",
        "__ctype_tolower_loc",
        "__ctype_toupper_loc",
        "isalnum",
        "isalpha",
        "isblank",
        "iscntrl",
        "isdigit",
        "isgraph",
        "islower",
        "isprint",
        "ispunct",
        "isspace",
        "isupper",
        "isxdigit",
        "tolower",
        "toupper",
        // compiler-emitted
        "__stack_chk_fail",
        "__stack_chk_guard",
        "__chk_fail",
        "__chkstk_darwin",
        "__sincos_stret",
        "__sincosf_stret",
        "__sincospi_stret",
        "__sincospif_stret",
        "__exp10",
        "__exp10f",
        "__sinpi",
        "__sinpif",
        "__cospi",
        "__cospif",
        "__tanpi",
        "__tanpif",
        "__isnan",
        "__isnanf",
        "__isinf",
        "__isinff",
        "__finite",
        "__finitef",
        "__fpclassify",
        "__fpclassifyf",
        "__signbit",
        "__signbitf",
    ],
    libm: &[
        "acos",
        "asin",
        "atan",
        "atan2",
        "cos",
        "sin",
        "tan",
        "acosh",
        "asinh",
        "atanh",
        "cosh",
        "sinh",
        "tanh",
        "exp",
        "exp2",
        "expm1",
        "log",
        "log10",
        "log1p",
        "log2",
        "logb",
        "ilogb",
        "pow",
        "sqrt",
        "cbrt",
        "hypot",
        "erf",
        "erfc",
        "tgamma",
        "lgamma",
        "ceil",
        "floor",
        "trunc",
        "round",
        "lround",
        "llround",
        "rint",
        "lrint",
        "llrint",
        "nearbyint",
        "fmod",
        "remainder",
        "remquo",
        "fdim",
        "fmax",
        "fmin",
        "fma",
        "fabs",
        "copysign",
        "nan",
        "nextafter",
        "nexttoward",
        "frexp",
        "ldexp",
        "modf",
        "scalbn",
        "scalbln",
    ],
};

impl LibcAllowlist {
    /// True when the (normalized) undefined name `name` may be referenced.
    pub(crate) fn allows(&self, name: &str) -> bool {
        let listed = |n: &str| {
            self.exact.contains(&n)
                || self.libm.contains(&n)
                || n.strip_suffix(['f', 'l'])
                    .is_some_and(|base| self.libm.contains(&base))
        };
        listed(name)
            || name
                .strip_prefix("__")
                .and_then(|n| n.strip_suffix("_chk"))
                .is_some_and(listed)
    }
}

/// What `nm` says about a driver object, names normalized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ObjectSymbols {
    /// Defined external symbols.
    pub defined: BTreeSet<String>,
    /// Undefined symbols.
    pub undefined: BTreeSet<String>,
    /// Weak symbols (references or definitions).
    pub weak: BTreeSet<String>,
}

/// Run `nm` three ways on the driver object `obj`.
pub(crate) fn object_symbols(runner: &Runner, obj: &Path) -> Result<ObjectSymbols, Error> {
    let macos = cfg!(target_os = "macos");
    let obj_str = obj
        .to_str()
        .ok_or_else(|| Error::Invariant(format!("non-UTF-8 path: {}", obj.display())))?;
    let defined_out = runner.tool(&nm_argv(obj_str))?;
    let undefined_out = runner.tool(&["nm".to_string(), "-u".to_string(), obj_str.to_string()])?;
    let weak_argv: Vec<String> = if macos {
        vec!["nm".into(), "-m".into(), obj_str.to_string()]
    } else {
        vec!["nm".into(), obj_str.to_string()]
    };
    let weak_out = runner.tool(&weak_argv)?;
    Ok(ObjectSymbols {
        defined: parse_nm(&String::from_utf8_lossy(&defined_out))
            .into_iter()
            .map(|s| normalize(&s.name, macos).to_string())
            .collect(),
        undefined: parse_undefined(&String::from_utf8_lossy(&undefined_out))
            .into_iter()
            .map(|n| normalize(&n, macos).to_string())
            .collect(),
        weak: parse_weak(&String::from_utf8_lossy(&weak_out), macos)
            .into_iter()
            .map(|n| normalize(&n, macos).to_string())
            .collect(),
    })
}

/// Names from `nm -u` output: the last token of every line that is not an
/// archive member header (`foo.o:`). Works for both the bare-name (macOS)
/// and the `U name` (GNU) layouts.
pub(crate) fn parse_undefined(nm_u_output: &str) -> Vec<String> {
    nm_u_output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with(':'))
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect()
}

/// Weak symbols: from `nm -m` (Mach-O: a `weak` attribute token) or plain
/// `nm` (ELF: type `w`/`W`/`v`/`V`).
pub(crate) fn parse_weak(output: &str, macho: bool) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some((name, rest)) = tokens.split_last() else {
            continue;
        };
        let weak = if macho {
            rest.contains(&"weak")
        } else {
            rest.last()
                .is_some_and(|t| matches!(*t, "w" | "W" | "v" | "V"))
        };
        if weak {
            out.push((*name).to_string());
        }
    }
    out
}

/// The object half of the gate: violations of the `{main}`-only export
/// rule, the undefined-symbol allowlist and the no-weak rule.
pub(crate) fn object_violations(syms: &ObjectSymbols, unit_symbols: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let main: BTreeSet<String> = std::iter::once("main".to_string()).collect();
    if syms.defined != main {
        let extra: Vec<&str> = syms
            .defined
            .iter()
            .filter(|n| *n != "main")
            .map(String::as_str)
            .collect();
        if !syms.defined.contains("main") {
            out.push("the driver object must define `main`".to_string());
        }
        if !extra.is_empty() {
            out.push(format!(
                "the driver object may define only `main` externally, found: {} \
                 (make helpers and globals `static`)",
                extra.join(", ")
            ));
        }
    }
    let not_allowed: Vec<&str> = syms
        .undefined
        .iter()
        .filter(|n| !unit_symbols.contains(n) && !DRIVER_LIBC_ALLOWLIST.allows(n))
        .map(String::as_str)
        .collect();
    if !not_allowed.is_empty() {
        out.push(format!(
            "the driver references functions outside the unit and the driver libc allowlist: {}",
            not_allowed.join(", ")
        ));
    }
    if !syms.weak.is_empty() {
        let weak: Vec<&str> = syms.weak.iter().map(String::as_str).collect();
        out.push(format!(
            "weak symbols are not allowed in a driver: {}",
            weak.join(", ")
        ));
    }
    out
}

/// Fold object + lint violations into the `driver-shape` check.
pub(crate) fn shape_check(object: Vec<String>, lint: Vec<String>) -> Check {
    let mut all = object;
    all.extend(lint);
    if all.is_empty() {
        return Check {
            name: CHECK_NAME.into(),
            passed: true,
            detail: "driver object defines only main, references only the unit and allowlisted \
                     libc; source lint clean"
                .into(),
        };
    }
    let total = all.len();
    let mut detail = all
        .into_iter()
        .take(DETAIL_LIMIT)
        .collect::<Vec<_>>()
        .join("; ");
    if total > DETAIL_LIMIT {
        detail.push_str(&format!(" (+{} more)", total - DETAIL_LIMIT));
    }
    Check {
        name: CHECK_NAME.into(),
        passed: false,
        detail,
    }
}

/// A failed `driver-shape` check for a driver that does not even compile
/// on its own (in `verify`; `validate_driver` reports that as
/// `driver-build`).
pub(crate) fn not_compiled(stderr: &str) -> Check {
    Check {
        name: CHECK_NAME.into(),
        passed: false,
        detail: format!("the driver does not compile on its own: {stderr}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn allowlist_covers_variants_and_fortify_but_not_capabilities() {
        let a = &DRIVER_LIBC_ALLOWLIST;
        for ok in [
            "printf",
            "__sprintf_chk",
            "__memcpy_chk",
            "sqrt",
            "sqrtf",
            "sqrtl",
            "__stack_chk_fail",
            "__stdoutp",
            "_DefaultRuneLocale",
            "memset_pattern16",
        ] {
            assert!(a.allows(ok), "{ok}");
        }
        for bad in [
            "fopen",
            "open",
            "getenv",
            "time",
            "clock_gettime",
            "dlsym",
            "syscall",
            "system",
            "rand",
            "__stdinp",
            "stdin",
            "__open_chk",
            "__readlink_chk",
            "fread",
            "fscanf",
        ] {
            assert!(!a.allows(bad), "{bad}");
        }
    }

    #[test]
    fn nm_parsers_handle_both_layouts() {
        assert_eq!(
            parse_undefined("_printf\n___stack_chk_fail\n"),
            vec!["_printf".to_string(), "___stack_chk_fail".to_string()]
        );
        assert_eq!(
            parse_undefined("\nx.o:\n                 U printf\n                 w weakling\n"),
            vec!["printf".to_string(), "weakling".to_string()]
        );
        let macho = "                 (undefined) external _printf\n\
                     0000000000000000 (__TEXT,__text) external _main\n\
                     \x20                (undefined) weak external _rust_probe\n";
        assert_eq!(parse_weak(macho, true), vec!["_rust_probe".to_string()]);
        let elf = "                 U printf\n                 w rust_probe\n\
                   0000000000000000 T main\n0000000000000010 W helper\n";
        assert_eq!(
            parse_weak(elf, false),
            vec!["rust_probe".to_string(), "helper".to_string()]
        );
    }

    #[test]
    fn object_rules() {
        let unit = vec!["unit_f".to_string()];
        let clean = ObjectSymbols {
            defined: set(&["main"]),
            undefined: set(&["printf", "unit_f", "__stack_chk_guard"]),
            weak: BTreeSet::new(),
        };
        assert!(object_violations(&clean, &unit).is_empty());

        let bad = ObjectSymbols {
            defined: set(&["main", "helper", "table"]),
            undefined: set(&["printf", "fopen", "getenv"]),
            weak: set(&["rust_eh_personality"]),
        };
        let v = object_violations(&bad, &unit);
        assert_eq!(v.len(), 3, "{v:?}");
        assert!(v[0].contains("helper, table"), "{v:?}");
        assert!(v[1].contains("fopen, getenv"), "{v:?}");
        assert!(v[2].contains("rust_eh_personality"), "{v:?}");

        let no_main = ObjectSymbols {
            defined: BTreeSet::new(),
            ..clean
        };
        assert!(object_violations(&no_main, &unit)[0].contains("must define `main`"));
    }

    #[test]
    fn check_detail_is_bounded() {
        let many: Vec<String> = (0..20).map(|i| format!("v{i}")).collect();
        let c = shape_check(many, Vec::new());
        assert!(!c.passed);
        assert!(c.detail.ends_with("(+8 more)"), "{}", c.detail);
        assert!(shape_check(Vec::new(), Vec::new()).passed);
    }
}
