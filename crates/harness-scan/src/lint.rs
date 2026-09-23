//! The driver source lint (docs/M4-DESIGN.md R1 "Source lint").
//!
//! A differential driver runs on both sides of the oracle, so a hostile or
//! careless one could detect which side it is linked against and forge the
//! C behavior on the Rust side. [`lint_driver`] is the source half of the
//! `driver-shape` gate (the oracle adds an object-level `nm` check); it
//! rejects every construct a driver has no business using:
//!
//! - inline assembly (`asm`, `__asm__`), attributes (`__attribute__`,
//!   `[[…]]`, `__declspec`), `#pragma` / `_Pragma` and every preprocessor
//!   directive other than `#define`/`#undef`/`#if…`/`#include`/`#error`/
//!   `#warning` (so `#embed`, `#include_next`, `#import`, `#line` are out);
//! - any identifier with the reserved `__` prefix (compiler builtins,
//!   `__FILE__`/`__DATE__`, `__int128`, …);
//! - function-like macros and `##` token pasting (also as the `%:%:`
//!   digraph), and `#define`/`#undef` of a unit symbol;
//! - `#include "x"` unless `x` (or its basename) is one of the unit's own
//!   headers — and never with a `..` segment or an absolute path;
//!   `#include <x>` only from [`DRIVER_SYSTEM_INCLUDES`];
//! - a unit symbol used as anything but the callee of a call or the name in
//!   its own prototype (never address-taken, assigned, cast or passed);
//! - a `%p` conversion in any string literal (escape sequences decoded,
//!   adjacent literals joined; an unknown macro between literals is assumed
//!   to be able to supply the `p`); `uintptr_t`/`intptr_t`;
//! - source tree-sitter cannot parse cleanly.
//!
//! Object-like macro bodies are not parsed by tree-sitter, so their text is
//! token-scanned for the same identifier and string rules.
//!
//! Residual (recorded in DECISIONS.md by the caller): a format string built
//! at run time from character constants, and address-distance side channels
//! between unrelated objects, are invisible to a syntactic lint.

use std::collections::BTreeSet;

/// The system headers a driver may include (`#include <x>`).
pub const DRIVER_SYSTEM_INCLUDES: [&str; 12] = [
    "stdio.h",
    "stdint.h",
    "stddef.h",
    "string.h",
    "stdlib.h",
    "inttypes.h",
    "limits.h",
    "float.h",
    "math.h",
    "stdbool.h",
    "ctype.h",
    "errno.h",
];

/// `preproc_call` directives a driver may use (`#define`, `#include` and the
/// conditional directives have node kinds of their own).
const ALLOWED_CALL_DIRECTIVES: [&str; 3] = ["#undef", "#error", "#warning"];

/// Identifiers banned outright (beyond the `__` prefix rule).
const BANNED_IDENTIFIERS: [(&str, &str); 4] = [
    ("_Pragma", "`_Pragma` is not allowed"),
    ("asm", "inline assembly (`asm`) is not allowed"),
    (
        "uintptr_t",
        "`uintptr_t` is not allowed (no pointer-to-integer conversions)",
    ),
    (
        "intptr_t",
        "`intptr_t` is not allowed (no pointer-to-integer conversions)",
    ),
];

/// Lint a driver's source. Returns human-readable violations, sorted by line
/// and deduplicated — empty means clean. `unit_symbols` are the unit's ABI
/// symbols; `allowed_quoted_includes` the paths and/or basenames of the
/// unit's own headers.
pub fn lint_driver(
    source: &[u8],
    unit_symbols: &[String],
    allowed_quoted_includes: &[String],
) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .is_err()
    {
        return vec!["the driver could not be parsed (tree-sitter grammar unavailable)".into()];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec!["the driver could not be parsed".into()];
    };
    let mut lint = Lint {
        src: source,
        symbols: unit_symbols.iter().map(String::as_str).collect(),
        includes: allowed_quoted_includes.iter().map(String::as_str).collect(),
        found: BTreeSet::new(),
    };
    lint.visit(tree.root_node());
    lint.raw_scan();
    lint.found
        .into_iter()
        .map(|(line, msg)| format!("line {line}: {msg}"))
        .collect()
}

struct Lint<'a> {
    src: &'a [u8],
    symbols: BTreeSet<&'a str>,
    includes: BTreeSet<&'a str>,
    /// `(line, message)`, ordered and deduplicated.
    found: BTreeSet<(u32, String)>,
}

impl Lint<'_> {
    fn text(&self, node: tree_sitter::Node) -> &str {
        node.utf8_text(self.src).unwrap_or("")
    }

    fn flag(&mut self, node: tree_sitter::Node, msg: impl Into<String>) {
        let line = u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX);
        self.found.insert((line, msg.into()));
    }

    fn visit(&mut self, node: tree_sitter::Node) {
        if node.is_error() || node.is_missing() {
            self.flag(
                node,
                "the driver does not parse cleanly (syntax the linter cannot verify)",
            );
        }
        match node.kind() {
            "gnu_asm_expression" | "asm" | "__asm__" | "__asm" => {
                self.flag(node, "inline assembly (`asm`) is not allowed");
            }
            "attribute_specifier" | "attribute_declaration" | "ms_declspec_modifier" => {
                self.flag(
                    node,
                    "attributes (`__attribute__`, `[[...]]`, `__declspec`) are not allowed",
                );
            }
            "preproc_function_def" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.flag(node, format!("function-like macro `{name}` is not allowed"));
            }
            "preproc_def" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let name = self.text(name).to_string();
                    if self.symbols.contains(name.as_str()) {
                        self.flag(
                            node,
                            format!("`#define` of unit symbol `{name}` is not allowed"),
                        );
                    }
                }
            }
            "preproc_call" => self.directive(node),
            "preproc_include" => self.include(node),
            "preproc_arg" => self.macro_text(node),
            // tree-sitter-c knows `uintptr_t`, `size_t`, … as primitive types.
            "identifier"
            | "type_identifier"
            | "field_identifier"
            | "statement_identifier"
            | "primitive_type" => {
                self.identifier(node);
            }
            "string_literal" => {
                let is_part = node
                    .parent()
                    .is_some_and(|p| p.kind() == "concatenated_string");
                if !is_part && has_p_conversion(&decode_c_string(self.text(node))) {
                    self.flag(node, "`%p` (pointer formatting) is not allowed");
                }
            }
            "concatenated_string" => self.concatenated(node),
            _ => {}
        }
        let mut cursor = node.walk();
        let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
        for child in children {
            self.visit(child);
        }
    }

    fn directive(&mut self, node: tree_sitter::Node) {
        let directive = node
            .child_by_field_name("directive")
            .map(|d| self.text(d).trim().to_string())
            .unwrap_or_default();
        let argument = node
            .child_by_field_name("argument")
            .map(|a| self.text(a).trim().to_string())
            .unwrap_or_default();
        if directive == "#pragma" {
            self.flag(node, "`#pragma` is not allowed");
        } else if !ALLOWED_CALL_DIRECTIVES.contains(&directive.as_str()) {
            self.flag(
                node,
                format!("preprocessor directive `{directive}` is not allowed"),
            );
        } else if directive == "#undef" && self.symbols.contains(argument.as_str()) {
            self.flag(
                node,
                format!("`#undef` of unit symbol `{argument}` is not allowed"),
            );
        }
    }

    fn include(&mut self, node: tree_sitter::Node) {
        let Some(path) = node.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path).to_string();
        match path.kind() {
            "string_literal" => {
                let x = raw.trim_matches('"');
                let basename = x.rsplit('/').next().unwrap_or(x);
                let clean = !x.starts_with('/') && !x.split('/').any(|s| s == "..");
                let listed = self.includes.contains(x) || self.includes.contains(basename);
                if !(clean && listed) {
                    self.flag(
                        node,
                        format!("`#include \"{x}\"` is not one of the unit's own headers"),
                    );
                }
            }
            "system_lib_string" => {
                let x = raw.trim_start_matches('<').trim_end_matches('>').trim();
                if !DRIVER_SYSTEM_INCLUDES.contains(&x) {
                    self.flag(
                        node,
                        format!(
                            "`#include <{x}>` is not allowed (allowed: {})",
                            DRIVER_SYSTEM_INCLUDES.join(" ")
                        ),
                    );
                }
            }
            _ => self.flag(node, "a computed `#include` is not allowed"),
        }
    }

    fn identifier(&mut self, node: tree_sitter::Node) {
        let name = self.text(node).to_string();
        if let Some(msg) = banned_identifier(&name) {
            self.flag(node, msg);
        }
        if node.kind() == "identifier"
            && self.symbols.contains(name.as_str())
            && !allowed_symbol_use(node)
        {
            let context = node.parent().map(|p| p.kind()).unwrap_or("?");
            self.flag(
                node,
                format!(
                    "unit symbol `{name}` may only be called or prototyped (found in {context})"
                ),
            );
        }
    }

    /// Adjacent literals are one string to printf: join them (an unknown
    /// macro between parts may supply any text — assume the worst, `p`).
    fn concatenated(&mut self, node: tree_sitter::Node) {
        let mut joined = String::new();
        let mut cursor = node.walk();
        let parts: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
        for part in parts {
            match part.kind() {
                "string_literal" => joined.push_str(&decode_c_string(self.text(part))),
                "identifier" if is_format_macro(self.text(part)) => joined.push('d'),
                _ => joined.push('p'),
            }
        }
        if has_p_conversion(&joined) {
            self.flag(node, "`%p` (pointer formatting) is not allowed");
        }
    }

    /// Token-scan an unparsed macro body / directive argument.
    fn macro_text(&mut self, node: tree_sitter::Node) {
        let text = self.text(node).to_string();
        for token in c_tokens(&text) {
            match token {
                Token::Ident(name) => {
                    if let Some(msg) = banned_identifier(&name) {
                        self.flag(node, msg);
                    }
                    if self.symbols.contains(name.as_str()) {
                        self.flag(
                            node,
                            format!("unit symbol `{name}` may not appear in a macro"),
                        );
                    }
                }
                Token::Str(lit) => {
                    if has_p_conversion(&decode_c_string(&lit)) {
                        self.flag(node, "`%p` (pointer formatting) is not allowed");
                    }
                }
            }
        }
    }

    /// Byte-level scan outside comments and literals: token pasting (`##`,
    /// also the `%:%:` digraph) and the `%:` digraph for `#` in general.
    fn raw_scan(&mut self) {
        let src = self.src;
        let mut line = 1u32;
        let mut i = 0usize;
        while i < src.len() {
            let b = src[i];
            let next = src.get(i + 1).copied();
            match (b, next) {
                (b'\n', _) => line += 1,
                (b'/', Some(b'/')) => {
                    while i < src.len() && src[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                (b'/', Some(b'*')) => {
                    i += 2;
                    while i < src.len() && !(src[i] == b'*' && src.get(i + 1) == Some(&b'/')) {
                        if src[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    i += 2;
                    continue;
                }
                (b'"' | b'\'', _) => {
                    i += 1;
                    while i < src.len() && src[i] != b && src[i] != b'\n' {
                        if src[i] == b'\\' && src.get(i + 1) != Some(&b'\n') {
                            i += 1;
                        }
                        i += 1;
                    }
                    // An unterminated literal ends at the newline, which the
                    // next iteration must still count.
                    if src.get(i) == Some(&b'\n') {
                        continue;
                    }
                }
                (b'#', Some(b'#')) => {
                    self.found
                        .insert((line, "token pasting (`##`) is not allowed".into()));
                }
                (b'%', Some(b':')) => {
                    self.found
                        .insert((line, "digraph `%:` (`#`) is not allowed".into()));
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// The message for an identifier that is banned wherever it appears.
fn banned_identifier(name: &str) -> Option<String> {
    if let Some((_, msg)) = BANNED_IDENTIFIERS.iter().find(|(n, _)| *n == name) {
        return Some((*msg).to_string());
    }
    if name.starts_with("__") {
        return Some(format!(
            "identifier `{name}` uses the reserved `__` prefix (builtins/extensions are not allowed)"
        ));
    }
    None
}

/// True when a unit-symbol identifier is the callee of a call, or the name
/// declared by a prototype (a `declaration`, never a definition).
fn allowed_symbol_use(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "call_expression" => parent
            .child_by_field_name("function")
            .is_some_and(|f| f.id() == node.id()),
        "function_declarator" => {
            let is_name = parent
                .child_by_field_name("declarator")
                .is_some_and(|d| d.id() == node.id());
            if !is_name {
                return false;
            }
            let mut up = parent.parent();
            while let Some(n) = up {
                match n.kind() {
                    "pointer_declarator" | "parenthesized_declarator" => up = n.parent(),
                    "declaration" => return true,
                    _ => return false,
                }
            }
            false
        }
        _ => false,
    }
}

/// `<inttypes.h>` format macros for integers (never the pointer-sized
/// `…PTR` ones).
fn is_format_macro(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("PRI")
        .or_else(|| name.strip_prefix("SCN"))
    else {
        return false;
    };
    let mut chars = rest.chars();
    let conv_ok = chars.next().is_some_and(|c| "diouxX".contains(c));
    let width = chars.as_str();
    conv_ok
        && (matches!(width, "8" | "16" | "32" | "64" | "MAX")
            || ["LEAST", "FAST"].iter().any(|p| {
                width
                    .strip_prefix(p)
                    .is_some_and(|w| matches!(w, "8" | "16" | "32" | "64"))
            }))
}

/// The contents of a C string literal token (optional `L`/`u`/`U`/`u8`
/// prefix, surrounding quotes) with escape sequences decoded. Undecodable
/// bytes pass through as-is.
fn decode_c_string(token: &str) -> String {
    let inner = match (token.find('"'), token.rfind('"')) {
        (Some(a), Some(b)) if b > a => &token[a + 1..b],
        _ => token,
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let e = bytes[i + 1];
        i += 2;
        match e {
            b'x' => {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                let v = u32::from_str_radix(hex, 16).unwrap_or(0);
                out.push((v & 0xff) as u8);
            }
            b'0'..=b'7' => {
                let start = i - 1;
                while i < bytes.len() && i < start + 3 && (b'0'..=b'7').contains(&bytes[i]) {
                    i += 1;
                }
                let oct = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
                let v = u32::from_str_radix(oct, 8).unwrap_or(0);
                out.push((v & 0xff) as u8);
            }
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'v' => out.push(11),
            // `\u`/`\U` are never `%` or `p`; keep a neutral placeholder.
            b'u' | b'U' => out.push(b'?'),
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True when a printf-style format string contains a `%p` conversion
/// (flags, width, precision and length modifiers allowed in between).
fn has_p_conversion(fmt: &str) -> bool {
    let b = fmt.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        if b.get(i) == Some(&b'%') {
            i += 1;
            continue;
        }
        while i < b.len() && b"-+ #0'".contains(&b[i]) {
            i += 1;
        }
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'*' || b[i] == b'$') {
            i += 1;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'*') {
                i += 1;
            }
        }
        while i < b.len() && b"hljztLq".contains(&b[i]) {
            i += 1;
        }
        if b.get(i) == Some(&b'p') {
            return true;
        }
    }
    false
}

/// A token of an unparsed macro body.
enum Token {
    Ident(String),
    Str(String),
}

/// Identifiers and string literals of `text`, skipping comments, character
/// constants and everything else.
fn c_tokens(text: &str) -> Vec<Token> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            break;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            if c == b'"' {
                let end = i.min(b.len());
                out.push(Token::Str(text[start..end].to_string()));
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            // A literal prefix (`L"…"`, `u8"…"`) belongs to the string.
            if b.get(i) == Some(&b'"') && matches!(&text[start..i], "L" | "u" | "U" | "u8") {
                continue;
            }
            out.push(Token::Ident(text[start..i].to_string()));
            continue;
        }
        if c.is_ascii_digit() {
            // Skip a number (including suffixes like `10ULL`).
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| (*x).to_string()).collect()
    }

    fn lint(src: &str) -> Vec<String> {
        lint_driver(src.as_bytes(), &s(&["unit_f"]), &s(&["unit.h"]))
    }

    /// A clean driver skeleton; each violation test inserts one line.
    fn with(line: &str) -> String {
        format!(
            "#include <stdio.h>\n#include \"unit.h\"\n{line}\nint main(void) {{\n  printf(\"%d\\n\", unit_f(3));\n  return 0;\n}}\n"
        )
    }

    fn assert_flags(line: &str, needle: &str) {
        let found = lint(&with(line));
        assert!(
            found.iter().any(|v| v.contains(needle)),
            "{line:?} should be flagged with {needle:?}: {found:?}"
        );
    }

    #[test]
    fn the_committed_u001_driver_is_clean() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../targets/zopfli/migration/units/u001-katajainen/driver.c");
        let src = std::fs::read(path).expect("u001 driver");
        let found = lint_driver(
            &src,
            &s(&["ZopfliLengthLimitedCodeLengths"]),
            &s(&["katajainen.h"]),
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_clean_skeleton_is_clean() {
        assert!(lint(&with("")).is_empty(), "{:?}", lint(&with("")));
        assert!(lint(&with("int unit_f(int x);")).is_empty());
        assert!(lint(&with("#define N 10\n#undef N")).is_empty());
        assert!(lint(&with(
            "static void p(unsigned long long v) { printf(\"%\" PRIu64 \" %5.2f %%p\\n\", v, 1.0); }"
        ))
        .is_empty());
    }

    #[test]
    fn inline_assembly() {
        assert_flags("static void f(void) { asm(\"nop\"); }", "inline assembly");
        assert_flags(
            "static void f(void) { __asm__(\"nop\"); }",
            "inline assembly",
        );
    }

    #[test]
    fn attributes() {
        assert_flags("__attribute__((weak)) int unit_g(void);", "attributes");
        assert_flags("[[gnu::weak]] int unit_g(void);", "attributes");
    }

    #[test]
    fn pragmas_and_other_directives() {
        assert_flags("#pragma weak unit_f", "`#pragma`");
        assert_flags("_Pragma(\"weak unit_f\")", "`_Pragma`");
        assert_flags("#embed \"secret.bin\"", "`#embed`");
        assert_flags("#include_next <stdio.h>", "`#include_next`");
    }

    #[test]
    fn reserved_identifiers() {
        assert_flags(
            "static int f(void) { return __builtin_return_address(0) != 0; }",
            "reserved `__` prefix",
        );
        assert_flags(
            "static const char *d = __DATE__;",
            "`__DATE__` uses the reserved",
        );
        assert_flags("#define X __COUNTER__", "`__COUNTER__`");
    }

    #[test]
    fn function_like_macros_and_token_pasting() {
        assert_flags("#define CALL(f) f(1)", "function-like macro `CALL`");
        assert_flags("#define GLUE a ## b", "token pasting");
        assert_flags("%:define X 1", "digraph");
    }

    #[test]
    fn redefining_a_unit_symbol() {
        assert_flags("#define unit_f fake_f", "`#define` of unit symbol `unit_f`");
        assert_flags("#undef unit_f", "`#undef` of unit symbol `unit_f`");
    }

    #[test]
    fn includes() {
        assert_flags("#include \"other.h\"", "not one of the unit's own headers");
        assert_flags(
            "#include \"../../heldout/unit.h\"",
            "not one of the unit's own headers",
        );
        assert_flags(
            "#include \"/abs/unit.h\"",
            "not one of the unit's own headers",
        );
        assert_flags(
            "#include <unistd.h>",
            "`#include <unistd.h>` is not allowed",
        );
        assert_flags("#include HDR", "computed `#include`");
        // A path whose basename is listed is fine.
        assert!(lint(&with("#include \"sub/unit.h\"")).is_empty());
    }

    #[test]
    fn unit_symbols_only_as_callees_or_prototypes() {
        assert_flags(
            "static int (*p)(int) = unit_f;",
            "unit symbol `unit_f` may only be called",
        );
        assert_flags(
            "static void *q = (void *)&unit_f;",
            "unit symbol `unit_f` may only be called",
        );
        assert_flags(
            "static int g(int (*f)(int)) { return f(1); } static int h(void) { return g(unit_f); }",
            "unit symbol `unit_f` may only be called",
        );
        assert_flags(
            "int unit_f(int x) { return x; }",
            "unit symbol `unit_f` may only be called",
        );
        assert_flags(
            "#define ALIAS unit_f",
            "unit symbol `unit_f` may not appear in a macro",
        );
    }

    #[test]
    fn pointer_formatting() {
        assert_flags(
            "static void f(int *x) { printf(\"%p\\n\", (void *)x); }",
            "`%p`",
        );
        assert_flags(
            "static void f(int *x) { printf(\"%-18p\\n\", (void *)x); }",
            "`%p`",
        );
        assert_flags(
            "static void f(int *x) { printf(\"%\\x70\\n\", (void *)x); }",
            "`%p`",
        );
        assert_flags(
            "static void f(int *x) { printf(\"%\" \"p\\n\", (void *)x); }",
            "`%p`",
        );
        assert_flags(
            "#define PF \"p\"\nstatic void f(int *x) { printf(\"%\" PF, (void *)x); }",
            "`%p`",
        );
        assert_flags("#define FMT \"%p\"", "`%p`");
    }

    #[test]
    fn pointer_sized_integers() {
        assert_flags("static uintptr_t u;", "`uintptr_t`");
        assert_flags("static intptr_t i;", "`intptr_t`");
    }

    #[test]
    fn unparseable_source() {
        assert_flags("int x = ;", "does not parse cleanly");
    }

    #[test]
    fn violations_are_sorted_and_deduplicated() {
        let found = lint("#pragma a\n#pragma a\nasm(\"x\");\n");
        let lines: Vec<&str> = found.iter().map(String::as_str).collect();
        assert!(lines.windows(2).all(|w| w[0] <= w[1]), "{lines:?}");
        assert_eq!(
            found
                .iter()
                .filter(|v| v.contains("`#pragma` is not allowed"))
                .count(),
            2
        );
    }

    #[test]
    fn format_helpers() {
        assert!(has_p_conversion("%p"));
        assert!(has_p_conversion("x=%#lp"));
        assert!(!has_p_conversion("%%p"));
        assert!(!has_p_conversion("%s p"));
        assert_eq!(decode_c_string("\"\\x25\\160\""), "%p");
        assert_eq!(decode_c_string("L\"ab\""), "ab");
        assert!(is_format_macro("PRIu64"));
        assert!(is_format_macro("PRIxLEAST32"));
        assert!(!is_format_macro("PRIxPTR"));
        assert!(!is_format_macro("PF"));
    }
}
