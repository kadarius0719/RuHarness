//! Parse one preserved C signature (a plan `interface` line) into what the
//! boundary check's generated call wrapper needs (docs/ORACLE-HARDENING.md
//! §B.6): the symbol, its parameter names, which parameters are data
//! pointers, and whether it returns `void`.
//!
//! Interface lines are target-derived (hostile input): a line is accepted only
//! when it parses as exactly one function declaration of a plain identifier,
//! with no preprocessor, brace, semicolon or attribute syntax, and every
//! parameter named. Anything else is refused with a reason — the boundary
//! stage then refuses the unit rather than guessing. The caller matches the
//! declared name against the plan's symbols.

/// Longest interface line accepted (the prompt's contract-line bound).
pub const MAX_INTERFACE_LINE: usize = 512;

/// One parameter of a parsed interface line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceParam {
    /// The parameter's declared name.
    pub name: String,
    /// True for a pointer or array parameter that is not a function pointer:
    /// the parameters whose pointees the boundary check guards.
    pub data_pointer: bool,
}

/// A parsed interface line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSig {
    /// The declared function name (a plain C identifier).
    pub symbol: String,
    /// Parameters in declaration order (`(void)` and `()` = none).
    pub params: Vec<InterfaceParam>,
    /// True when the declared return type is exactly `void`.
    pub returns_void: bool,
}

/// True iff `name` is a plain C identifier (`^[A-Za-z_][A-Za-z0-9_]*$`).
pub fn is_c_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse `line` as one function declaration. `Err` carries a short,
/// harness-worded reason (never the line itself).
pub fn parse_interface(line: &str) -> Result<InterfaceSig, String> {
    if line.len() > MAX_INTERFACE_LINE {
        return Err(format!(
            "the interface line is longer than {MAX_INTERFACE_LINE} bytes"
        ));
    }
    if !line.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Err("the interface line is not printable ASCII on one line".into());
    }
    if line.contains(['{', '}', ';', '#', '\\'])
        || line.contains("__attribute__")
        || line.contains("[[")
        || line.contains("__declspec")
        || line.contains("...")
    {
        return Err(
            "the interface line contains syntax the wrapper does not support (braces, \
             semicolons, preprocessor, attributes, or a variadic parameter list)"
                .into(),
        );
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|_| "the C grammar is unavailable".to_string())?;
    let source = format!("{line};");
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| "the interface line does not parse".to_string())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("the interface line does not parse cleanly as a C declaration".into());
    }
    let src = source.as_bytes();
    let mut cursor = root.walk();
    let decls: Vec<tree_sitter::Node> = root.named_children(&mut cursor).collect();
    let [decl] = decls.as_slice() else {
        return Err("the interface line is not exactly one declaration".into());
    };
    if decl.kind() != "declaration" {
        return Err("the interface line is not a declaration".into());
    }
    let mut declarators = Vec::new();
    let mut c = decl.walk();
    for d in decl.children_by_field_name("declarator", &mut c) {
        declarators.push(d);
    }
    let [declarator] = declarators.as_slice() else {
        return Err("the interface line declares more than one name".into());
    };
    // The return type is `void` iff the type is `void` and the function
    // declarator is not wrapped in a pointer (`void *f(...)`).
    let (func, pointer_return) = match declarator.kind() {
        "function_declarator" => (*declarator, false),
        "pointer_declarator" => {
            let mut node = *declarator;
            while node.kind() == "pointer_declarator" {
                node = node
                    .child_by_field_name("declarator")
                    .ok_or_else(|| "the interface line has no function declarator".to_string())?;
            }
            if node.kind() != "function_declarator" {
                return Err("the interface line does not declare a function".into());
            }
            (node, true)
        }
        _ => return Err("the interface line does not declare a function".into()),
    };
    let name_node = func
        .child_by_field_name("declarator")
        .ok_or_else(|| "the function declarator has no name".to_string())?;
    let symbol = text(name_node, src);
    if name_node.kind() != "identifier" || !is_c_identifier(symbol) {
        return Err("the declared function name is not a plain C identifier".into());
    }
    let type_text = decl
        .child_by_field_name("type")
        .map(|t| text(t, src))
        .unwrap_or("");
    let mut qualifiers = decl.walk();
    let qualified = decl
        .children(&mut qualifiers)
        .any(|n| n.kind() == "type_qualifier" || n.kind() == "storage_class_specifier");
    let returns_void = type_text == "void" && !pointer_return && !qualified;

    let list = func
        .child_by_field_name("parameters")
        .ok_or_else(|| "the function declarator has no parameter list".to_string())?;
    let mut params = Vec::new();
    let mut pc = list.walk();
    let items: Vec<tree_sitter::Node> = list.named_children(&mut pc).collect();
    for item in &items {
        match item.kind() {
            "parameter_declaration" => {}
            "variadic_parameter" => return Err("variadic functions are not supported".into()),
            "comment" => continue,
            _ => return Err("the parameter list has an unsupported element".into()),
        }
        let Some(pdecl) = item.child_by_field_name("declarator") else {
            // `(void)` alone is the empty list; any other unnamed parameter
            // cannot be forwarded by name.
            let ptype = item.child_by_field_name("type").map(|t| text(t, src));
            if items.len() == 1 && ptype == Some("void") {
                break;
            }
            return Err("every parameter must be named".into());
        };
        let (name, data_pointer) = param_shape(pdecl, src)?;
        if params.iter().any(|p: &InterfaceParam| p.name == name) {
            return Err("two parameters share a name".into());
        }
        params.push(InterfaceParam { name, data_pointer });
    }
    Ok(InterfaceSig {
        symbol: symbol.to_string(),
        params,
        returns_void,
    })
}

/// A parameter declarator's name and whether it declares a data pointer:
/// any pointer or array declarator on the way to the name makes it one,
/// unless a function declarator appears (a function pointer).
fn param_shape(node: tree_sitter::Node, src: &[u8]) -> Result<(String, bool), String> {
    let mut node = node;
    let mut pointer = false;
    let mut function = false;
    loop {
        match node.kind() {
            "identifier" => {
                let name = text(node, src).to_string();
                if !is_c_identifier(&name) {
                    return Err("a parameter name is not a plain C identifier".into());
                }
                return Ok((name, pointer && !function));
            }
            "pointer_declarator" | "array_declarator" => {
                pointer = true;
                node = node
                    .child_by_field_name("declarator")
                    .ok_or_else(|| "every parameter must be named".to_string())?;
            }
            "function_declarator" => {
                function = true;
                node = node
                    .child_by_field_name("declarator")
                    .ok_or_else(|| "every parameter must be named".to_string())?;
            }
            "parenthesized_declarator" => {
                let mut c = node.walk();
                let inner = node.named_children(&mut c).find(|n| n.kind() != "comment");
                node = inner.ok_or_else(|| "every parameter must be named".to_string())?;
            }
            k if k.starts_with("abstract_") => return Err("every parameter must be named".into()),
            _ => return Err("a parameter declarator has an unsupported shape".into()),
        }
    }
}

fn text<'a>(node: tree_sitter::Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(line: &str, sym: &str) -> Vec<(String, bool)> {
        let sig = parse_interface(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert_eq!(sig.symbol, sym);
        parse_interface(line)
            .unwrap_or_else(|e| panic!("{line}: {e}"))
            .params
            .into_iter()
            .map(|p| (p.name, p.data_pointer))
            .collect()
    }

    #[test]
    fn corpus_shapes_parse() {
        assert_eq!(
            params(
                "void read_scalefactors(bs_t *bs, uint8_t *pba, uint8_t *scfcod, int bands, float *scf)",
                "read_scalefactors"
            ),
            vec![
                ("bs".into(), true),
                ("pba".into(), true),
                ("scfcod".into(), true),
                ("bands".into(), false),
                ("scf".into(), true)
            ]
        );
        assert_eq!(
            params(
                "void md5_digest(const tflac_md5 *m, tflac_u8 out[16])",
                "md5_digest"
            ),
            vec![("m".into(), true), ("out".into(), true)]
        );
        assert_eq!(
            params(
                "int hex2bin(uint8_t *bin, size_t bin_maxlen, const char *hex, size_t hex_len, \
                 const char *ignore, const char **hex_end_p)",
                "hex2bin"
            )
            .iter()
            .filter(|p| p.1)
            .count(),
            4
        );
        assert_eq!(
            params("void printLine (const char * line)", "printLine"),
            vec![("line".into(), true)]
        );
        let sig = parse_interface("char *bin2hex(char *hex, size_t n)").unwrap();
        assert!(!sig.returns_void && sig.symbol == "bin2hex");
        let sig = parse_interface("int* static_alias(int *outer)").unwrap();
        assert!(!sig.returns_void);
        let sig = parse_interface("void good()").unwrap();
        assert!(sig.returns_void && sig.params.is_empty());
        let sig = parse_interface("int f(void)").unwrap();
        assert!(!sig.returns_void && sig.params.is_empty());
        let sig = parse_interface("void *g(int x)").unwrap();
        assert!(!sig.returns_void);
        let sig = parse_interface("const int *h(int x)").unwrap();
        assert!(!sig.returns_void);
    }

    #[test]
    fn function_pointers_are_not_data_pointers() {
        assert_eq!(
            params("void f(void (*cb)(int), int *p, int *(*mk)(void))", "f"),
            vec![
                ("cb".into(), false),
                ("p".into(), true),
                ("mk".into(), false)
            ]
        );
    }

    #[test]
    fn hostile_or_unsupported_lines_are_refused() {
        for line in [
            "int f(int *p) { return 0; }",
            "int f(int *p); int g(void)",
            "int f(int *p)\n#define X",
            "int f(int *, int)",
            "int f(int, ...)",
            "int f(int *p) __attribute__((x))",
            "int f(int p, int p)",
            "int f(int *p",
            "int x",
            "int f(int *p), g(int *q)",
            "int (*f)(int *p)",
        ] {
            assert!(parse_interface(line).is_err(), "accepted: {line}");
        }
    }
}
