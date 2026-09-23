//! Mutation-site discovery for driver adequacy (docs/M4-DESIGN.md §5.4, R5).
//!
//! [`mutants`] is a pure function of one `.c` file's bytes: every site of
//! every operator class, sorted by byte offset, with no RNG and no I/O. The
//! oracle samples from this list ([`harness_core::driver::sample_mutants`])
//! and runs the sampled mutants against a generated driver.
//!
//! Operator classes (the `operator` field of [`Mutant`]):
//!
//! | operator | site | replacement |
//! |---|---|---|
//! | `arith` | binary `+ - * / %` | `+`↔`-`, `*`→`/`, `/`→`*`, `%`→`*` |
//! | `relational` | binary `< <= > >= == !=` | `<`→`<=`, `<=`→`<`, `>`→`>=`, `>=`→`>`, `==`↔`!=` |
//! | `logical` | binary `&& \|\|` | `&&`↔`\|\|` |
//! | `bitwise` | binary `& \| ^` | `&`→`\|`, `\|`→`&`, `^`→`&` |
//! | `shift` | binary `<< >>` | `<<`↔`>>` |
//! | `literal` | decimal / hex integer literal | `n`→`n+1`, suffix and format kept |
//! | `not-delete` | unary `!x` | `x` |
//! | `cast-delete` | `(T)x` | `x` |
//! | `signedness` | `unsigned`/`signed` keyword of a body declaration; plain `char` there | `unsigned`↔`signed`; `char`→`unsigned char` |
//! | `table-element` | integer literal in a FILE-SCOPE initializer list | `n`→`n+1` (`function` = `""`) |
//! | `string-literal` | first content byte of a body string literal, when ASCII alphanumeric | next alnum, wrapping `z`→`a`, `Z`→`A`, `9`→`0` |
//!
//! Scope rules:
//! - every class except `table-element` applies only inside function bodies,
//!   and such sites carry the enclosing function's (raw) name;
//! - nothing inside a preprocessor construct is mutated — directive lines
//!   AND the bodies of `#if`/`#ifdef` blocks, since without preprocessing it
//!   is unknowable whether a conditional branch is compiled at all (a
//!   mutant in a dead branch would be an unkillable, unfair survivor);
//! - nothing inside a tree-sitter `ERROR` node is mutated.
//!
//! Integer literals: the unsigned digit part is incremented, keeping any
//! sign token tree-sitter folded into the literal, the `0x`/`0X` prefix, the
//! hex-digit case and the `u`/`l` suffix. Octal (`0NN`), binary, floating and
//! digit-separated literals, and any value that would exceed `u64::MAX`, are
//! not sites.

use harness_core::driver::Mutant;
use harness_core::error::Error;

/// Every mutation site of the `.c` file `rel_path` (repo-relative, recorded
/// verbatim in each [`Mutant::file`]), sorted by start offset (ties by end,
/// operator, replacement). `Err` only when tree-sitter cannot parse at all.
pub fn mutants(rel_path: &str, source: &[u8]) -> Result<Vec<Mutant>, Error> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|e| Error::Invariant(format!("tree-sitter C grammar mismatch: {e}")))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::parse(rel_path, "tree-sitter parse failed"))?;
    let mut sites = Sites {
        file: rel_path,
        src: source,
        line_starts: line_starts(source),
        out: Vec::new(),
    };
    sites.visit(tree.root_node(), None);
    let mut out = sites.out;
    out.sort_by(|a, b| {
        (a.start, a.end, &a.operator, &a.replacement).cmp(&(
            b.start,
            b.end,
            &b.operator,
            &b.replacement,
        ))
    });
    out.dedup();
    Ok(out)
}

/// Byte offsets at which each line starts (line 1 starts at 0).
fn line_starts(src: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        src.iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i + 1),
    );
    starts
}

/// The site collector for one file.
struct Sites<'a> {
    file: &'a str,
    src: &'a [u8],
    line_starts: Vec<usize>,
    out: Vec<Mutant>,
}

impl Sites<'_> {
    /// 1-based line of byte `offset`.
    fn line_of(&self, offset: usize) -> u32 {
        let idx = self.line_starts.partition_point(|s| *s <= offset);
        u32::try_from(idx).unwrap_or(u32::MAX)
    }

    fn push(
        &mut self,
        start: usize,
        end: usize,
        replacement: &str,
        operator: &str,
        function: &str,
    ) {
        self.out.push(Mutant {
            file: self.file.to_string(),
            start,
            end,
            replacement: replacement.to_string(),
            operator: operator.to_string(),
            line: self.line_of(start),
            function: function.to_string(),
        });
    }

    fn text(&self, node: tree_sitter::Node) -> &str {
        node.utf8_text(self.src).unwrap_or("")
    }

    /// Walk `node`; `function` is the enclosing function's name inside a
    /// body, `None` at file scope.
    fn visit(&mut self, node: tree_sitter::Node, function: Option<&str>) {
        let kind = node.kind();
        if kind.starts_with("preproc_") || node.is_error() || node.is_missing() {
            return;
        }
        if kind == "function_definition" {
            // The signature and parameter list are never mutated; only the
            // body is, attributed to this function.
            let name = crate::function_name(node, self.src).unwrap_or_default();
            if let Some(body) = node.child_by_field_name("body") {
                self.visit(body, Some(&name));
            }
            return;
        }
        match function {
            None => {
                if kind == "declaration" {
                    self.file_scope_tables(node);
                    return;
                }
            }
            Some(f) => self.body_site(node, f),
        }
        let mut cursor = node.walk();
        let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
        for child in children {
            self.visit(child, function);
        }
    }

    /// Sites anchored at `node` itself, inside the body of `function`.
    fn body_site(&mut self, node: tree_sitter::Node, function: &str) {
        match node.kind() {
            "binary_expression" => {
                let Some(op) = node.child_by_field_name("operator") else {
                    return;
                };
                if let Some((operator, replacement)) = binary_swap(op.kind()) {
                    self.push(
                        op.start_byte(),
                        op.end_byte(),
                        replacement,
                        operator,
                        function,
                    );
                }
            }
            "unary_expression" => {
                if let Some(op) = node.child_by_field_name("operator") {
                    if op.kind() == "!" {
                        self.push(op.start_byte(), op.end_byte(), "", "not-delete", function);
                    }
                }
            }
            "cast_expression" => {
                if let Some(value) = node.child_by_field_name("value") {
                    // A single space keeps the neighbouring tokens apart
                    // (`return(int)x` → `return x`, `a-(int)-b` → `a- -b`).
                    self.push(
                        node.start_byte(),
                        value.start_byte(),
                        " ",
                        "cast-delete",
                        function,
                    );
                }
            }
            "number_literal" => {
                if let Some(bumped) = bump_int_literal(self.text(node)) {
                    self.push(
                        node.start_byte(),
                        node.end_byte(),
                        &bumped,
                        "literal",
                        function,
                    );
                }
            }
            "declaration" => self.signedness(node, function),
            "string_literal" => self.string_first_char(node, function),
            _ => {}
        }
    }

    /// `signedness` sites of a body declaration's type.
    fn signedness(&mut self, decl: tree_sitter::Node, function: &str) {
        let Some(ty) = decl.child_by_field_name("type") else {
            return;
        };
        match ty.kind() {
            "sized_type_specifier" => {
                let mut cursor = ty.walk();
                let keywords: Vec<(usize, usize, &str)> = ty
                    .children(&mut cursor)
                    .filter_map(|c| match c.kind() {
                        "unsigned" => Some((c.start_byte(), c.end_byte(), "signed")),
                        "signed" => Some((c.start_byte(), c.end_byte(), "unsigned")),
                        _ => None,
                    })
                    .collect();
                for (start, end, replacement) in keywords {
                    self.push(start, end, replacement, "signedness", function);
                }
            }
            "primitive_type" if self.text(ty) == "char" => {
                self.push(
                    ty.start_byte(),
                    ty.end_byte(),
                    "unsigned char",
                    "signedness",
                    function,
                );
            }
            _ => {}
        }
    }

    /// `string-literal` site: the first content byte, when alphanumeric.
    fn string_first_char(&mut self, lit: tree_sitter::Node, function: &str) {
        let text = self.text(lit).as_bytes();
        let Some(quote) = text.iter().position(|b| *b == b'"') else {
            return;
        };
        let offset = lit.start_byte() + quote + 1;
        // The content must be non-empty: the byte is not the closing quote.
        if offset + 1 >= lit.end_byte() {
            return;
        }
        if let Some(next) = self.src.get(offset).and_then(|b| next_alnum(*b)) {
            self.push(
                offset,
                offset + 1,
                &char::from(next).to_string(),
                "string-literal",
                function,
            );
        }
    }

    /// `table-element` sites of a file-scope declaration: integer literals
    /// inside the initializer lists of its declarators (designator indices
    /// excluded — they select a slot, they are not an element).
    fn file_scope_tables(&mut self, decl: tree_sitter::Node) {
        let mut cursor = decl.walk();
        let inits: Vec<tree_sitter::Node> = decl
            .children(&mut cursor)
            .filter(|c| c.kind() == "init_declarator")
            .filter_map(|c| c.child_by_field_name("value"))
            .filter(|v| v.kind() == "initializer_list")
            .collect();
        for list in inits {
            self.table_literals(list);
        }
    }

    fn table_literals(&mut self, node: tree_sitter::Node) {
        let kind = node.kind();
        if kind.starts_with("preproc_")
            || node.is_error()
            || node.is_missing()
            || kind.ends_with("_designator")
        {
            return;
        }
        if kind == "number_literal" {
            if let Some(bumped) = bump_int_literal(self.text(node)) {
                self.push(
                    node.start_byte(),
                    node.end_byte(),
                    &bumped,
                    "table-element",
                    "",
                );
            }
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
        for child in children {
            self.table_literals(child);
        }
    }
}

/// `(operator class, replacement)` for a binary operator token.
fn binary_swap(op: &str) -> Option<(&'static str, &'static str)> {
    Some(match op {
        "+" => ("arith", "-"),
        "-" => ("arith", "+"),
        "*" => ("arith", "/"),
        "/" => ("arith", "*"),
        "%" => ("arith", "*"),
        "<" => ("relational", "<="),
        "<=" => ("relational", "<"),
        ">" => ("relational", ">="),
        ">=" => ("relational", ">"),
        "==" => ("relational", "!="),
        "!=" => ("relational", "=="),
        "&&" => ("logical", "||"),
        "||" => ("logical", "&&"),
        "&" => ("bitwise", "|"),
        "|" => ("bitwise", "&"),
        "^" => ("bitwise", "&"),
        "<<" => ("shift", ">>"),
        ">>" => ("shift", "<<"),
        _ => return None,
    })
}

/// The next ASCII alphanumeric byte, wrapping within each class.
fn next_alnum(b: u8) -> Option<u8> {
    match b {
        b'z' => Some(b'a'),
        b'Z' => Some(b'A'),
        b'9' => Some(b'0'),
        b'a'..=b'y' | b'A'..=b'Y' | b'0'..=b'8' => Some(b + 1),
        _ => None,
    }
}

/// `n` → `n+1` for a decimal or hex integer literal, keeping a leading sign,
/// the prefix, the hex-digit case and the suffix. `None` for anything else.
pub(crate) fn bump_int_literal(text: &str) -> Option<String> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(r) => ("-", r.trim_start()),
        None => ("", text),
    };
    let digits_end = rest.trim_end_matches(['u', 'U', 'l', 'L']).len();
    let (body, suffix) = rest.split_at(digits_end);
    let suffix_ok = matches!(
        suffix.to_ascii_lowercase().as_str(),
        "" | "u" | "l" | "ul" | "lu" | "ll" | "ull" | "llu"
    );
    if !suffix_ok || body.is_empty() {
        return None;
    }
    let (prefix, digits, radix) = if let Some(h) = body.strip_prefix("0x") {
        ("0x", h, 16)
    } else if let Some(h) = body.strip_prefix("0X") {
        ("0X", h, 16)
    } else {
        ("", body, 10)
    };
    let valid = !digits.is_empty()
        && digits.chars().all(|c| c.is_digit(radix))
        // A leading 0 with more digits is octal in C.
        && !(radix == 10 && digits.len() > 1 && digits.starts_with('0'));
    if !valid {
        return None;
    }
    let value = u128::from_str_radix(digits, radix).ok()? + 1;
    if value > u128::from(u64::MAX) {
        return None;
    }
    let rendered = if radix == 16 {
        if digits.chars().any(|c| c.is_ascii_uppercase()) {
            format!("{value:X}")
        } else {
            format!("{value:x}")
        }
    } else {
        value.to_string()
    };
    Some(format!("{sign}{prefix}{rendered}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(operator, line, function, original text, replacement)` per mutant.
    fn describe(src: &str) -> Vec<(String, u32, String, String, String)> {
        mutants("src/t.c", src.as_bytes())
            .expect("parses")
            .into_iter()
            .map(|m| {
                (
                    m.operator.clone(),
                    m.line,
                    m.function.clone(),
                    src[m.start..m.end].to_string(),
                    m.replacement.clone(),
                )
            })
            .collect()
    }

    fn ops(src: &str, operator: &str) -> Vec<(String, String)> {
        describe(src)
            .into_iter()
            .filter(|d| d.0 == operator)
            .map(|d| (d.3, d.4))
            .collect()
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    #[test]
    fn arith_swaps_binary_operators_only() {
        let src = "int f(int a, int b) { return -a + b - a * b / a % b; }\n";
        assert_eq!(
            ops(src, "arith"),
            pairs(&[("+", "-"), ("-", "+"), ("*", "/"), ("/", "*"), ("%", "*")])
        );
    }

    #[test]
    fn relational_logical_bitwise_and_shift() {
        let src = "int f(int a, int b) {\n  int r = (a < b) + (a <= b) + (a > b) + (a >= b) + (a == b) + (a != b);\n\
                   r += (a && b) + (a || b) + (a & b) + (a | b) + (a ^ b) + (a << 1) + (a >> 1);\n  return r;\n}\n";
        assert_eq!(
            ops(src, "relational"),
            pairs(&[
                ("<", "<="),
                ("<=", "<"),
                (">", ">="),
                (">=", ">"),
                ("==", "!="),
                ("!=", "==")
            ])
        );
        assert_eq!(ops(src, "logical"), pairs(&[("&&", "||"), ("||", "&&")]));
        assert_eq!(
            ops(src, "bitwise"),
            pairs(&[("&", "|"), ("|", "&"), ("^", "&")])
        );
        assert_eq!(ops(src, "shift"), pairs(&[("<<", ">>"), (">>", "<<")]));
    }

    #[test]
    fn literals_keep_format_and_suffix() {
        let src =
            "unsigned long f(void) { return 0 + 7u + 0x1F + 0xffUL + 10LL + 017 + 1.5 + 0b1; }\n";
        assert_eq!(
            ops(src, "literal"),
            pairs(&[
                ("0", "1"),
                ("7u", "8u"),
                ("0x1F", "0x20"),
                ("0xffUL", "0x100UL"),
                ("10LL", "11LL")
            ])
        );
        assert_eq!(bump_int_literal("18446744073709551615"), None);
        assert_eq!(bump_int_literal("-3"), Some("-4".into()));
        assert_eq!(bump_int_literal("1e3"), None);
        assert_eq!(bump_int_literal("1'000"), None);
    }

    #[test]
    fn not_and_cast_deletion() {
        let src = "int f(int a) { return !a + (int)(unsigned)a; }\n";
        assert_eq!(ops(src, "not-delete"), pairs(&[("!", "")]));
        assert_eq!(
            ops(src, "cast-delete"),
            pairs(&[("(int)", " "), ("(unsigned)", " ")])
        );
    }

    #[test]
    fn signedness_flips_body_declarations_only() {
        let src = "unsigned g;\nint f(unsigned p) {\n  unsigned int a = p;\n  signed char s = 1;\n  char c = 2;\n  long z = 3;\n  for (unsigned i = 0; i < 2; i++) a += i;\n  return a + s + c + z;\n}\n";
        let found: Vec<(String, u32)> = describe(src)
            .into_iter()
            .filter(|d| d.0 == "signedness")
            .map(|d| (format!("{}->{}", d.3, d.4), d.1))
            .collect();
        assert_eq!(
            found,
            vec![
                ("unsigned->signed".to_string(), 3),
                ("signed->unsigned".to_string(), 4),
                ("char->unsigned char".to_string(), 5),
                ("unsigned->signed".to_string(), 7),
            ]
        );
    }

    #[test]
    fn table_elements_are_file_scope_initializer_literals() {
        let src = "static const int t[2][2] = {{1, 0x10}, {[1] = 5, -3}};\nstatic int x = 4;\n\
                   int f(void) { static const int local[] = {9}; return t[0][0] + local[0] + x; }\n";
        let tables: Vec<(String, String, String)> = describe(src)
            .into_iter()
            .filter(|d| d.0 == "table-element")
            .map(|d| (d.2, d.3, d.4))
            .collect();
        assert_eq!(
            tables,
            vec![
                (String::new(), "1".to_string(), "2".to_string()),
                (String::new(), "0x10".to_string(), "0x11".to_string()),
                (String::new(), "5".to_string(), "6".to_string()),
                (String::new(), "-3".to_string(), "-4".to_string()),
            ]
        );
        // A function-local table is an ordinary `literal` site of `f`.
        assert!(describe(src)
            .iter()
            .any(|d| d.0 == "literal" && d.2 == "f" && d.3 == "9"));
    }

    #[test]
    fn string_literal_first_char_wraps_within_its_class() {
        let src = "const char *f(int i) {\n  if (i == 1) return \"zebra\";\n  if (i == 2) return \"Zulu\";\n  if (i == 3) return \"9lives\";\n  if (i == 4) return \"%d\";\n  return L\"abc\" \"\";\n}\n";
        assert_eq!(
            ops(src, "string-literal"),
            pairs(&[("z", "a"), ("Z", "A"), ("9", "0"), ("a", "b")])
        );
    }

    #[test]
    fn sites_carry_function_and_line_and_skip_preprocessor_code() {
        let src = "#define TWICE(x) ((x) + (x))\n#if 1 + 1\nint dead(int a) { return a + 1; }\n#endif\n\
                   static int helper(int a)\n{\n  return a - 1;\n}\nint api(int a) {\n#ifdef FAST\n  a = a * 2;\n#endif\n  return helper(a) + 2;\n}\n";
        let found = describe(src);
        assert_eq!(
            found,
            vec![
                (
                    "arith".to_string(),
                    7,
                    "helper".to_string(),
                    "-".to_string(),
                    "+".to_string()
                ),
                (
                    "literal".to_string(),
                    7,
                    "helper".to_string(),
                    "1".to_string(),
                    "2".to_string()
                ),
                (
                    "arith".to_string(),
                    13,
                    "api".to_string(),
                    "+".to_string(),
                    "-".to_string()
                ),
                (
                    "literal".to_string(),
                    13,
                    "api".to_string(),
                    "2".to_string(),
                    "3".to_string()
                ),
            ]
        );
    }

    #[test]
    fn mutants_are_sorted_deterministic_and_apply_cleanly() {
        let src = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../targets/zopfli/src/zopfli/katajainen.c"),
        )
        .expect("katajainen.c");
        let a = mutants("src/zopfli/katajainen.c", &src).expect("parses");
        let b = mutants("src/zopfli/katajainen.c", &src).expect("parses");
        assert_eq!(a, b);
        assert!(a.len() > 50, "{} sites", a.len());
        assert!(a.windows(2).all(|w| w[0].start <= w[1].start));
        assert!(a
            .iter()
            .any(|m| m.function == "ZopfliLengthLimitedCodeLengths"));
        assert!(a.iter().any(|m| m.function == "BoundaryPM"));
        for m in &a {
            let mutated = m.apply(&src).expect("span fits");
            assert_ne!(mutated, src, "{m:?} changes nothing");
            assert_eq!(m.file, "src/zopfli/katajainen.c");
        }
    }
}
