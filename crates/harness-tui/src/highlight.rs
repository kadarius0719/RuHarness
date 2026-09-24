//! Syntax highlighting of the two sides of a pair (docs/TUI-DESIGN.md §1):
//! tree-sitter-highlight over the grammars' own queries (tree-sitter-c's
//! `HIGHLIGHT_QUERY`, tree-sitter-rust's `HIGHLIGHTS_QUERY`). It works on the
//! RAW text; the display filter runs afterwards, per span. Anything the
//! highlighter cannot handle renders plain — highlighting is never a reason
//! not to show code.

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent};

/// The source language of a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// The C side.
    C,
    /// The Rust side.
    Rust,
}

/// What a span of code is, as far as colour goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Anything unclassified.
    Plain,
    /// `@attribute`.
    Attribute,
    /// `@comment`.
    Comment,
    /// `@constant`, `@number`, `@escape`.
    Constant,
    /// `@function`, `@constructor`.
    Function,
    /// `@keyword`, `@label`.
    Keyword,
    /// `@operator`, `@punctuation`.
    Punctuation,
    /// `@property`, `@variable`.
    Name,
    /// `@string`.
    String,
    /// `@type`.
    Type,
}

/// The capture names this module recognises, in [`Class`] order of
/// [`class_of`] (tree-sitter-highlight matches `function.method` to
/// `function`).
const NAMES: [&str; 15] = [
    "attribute",
    "comment",
    "constant",
    "number",
    "escape",
    "function",
    "constructor",
    "keyword",
    "label",
    "operator",
    "punctuation",
    "property",
    "variable",
    "string",
    "type",
];

fn class_of(index: usize) -> Class {
    match NAMES.get(index).copied() {
        Some("attribute") => Class::Attribute,
        Some("comment") => Class::Comment,
        Some("constant" | "number" | "escape") => Class::Constant,
        Some("function" | "constructor") => Class::Function,
        Some("keyword" | "label") => Class::Keyword,
        Some("operator" | "punctuation") => Class::Punctuation,
        Some("property" | "variable") => Class::Name,
        Some("string") => Class::String,
        Some("type") => Class::Type,
        _ => Class::Plain,
    }
}

/// One highlighted line: `(class, raw text)` pieces, in order.
pub type Pieces = Vec<(Class, String)>;

/// Reusable highlighter for both languages.
pub struct Highlighter {
    engine: tree_sitter_highlight::Highlighter,
    c: Option<HighlightConfiguration>,
    rust: Option<HighlightConfiguration>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("c", &self.c.is_some())
            .field("rust", &self.rust.is_some())
            .finish()
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

fn configure(
    language: tree_sitter::Language,
    name: &str,
    query: &str,
) -> Option<HighlightConfiguration> {
    let mut config = HighlightConfiguration::new(language, name, query, "", "").ok()?;
    config.configure(&NAMES);
    Some(config)
}

impl Highlighter {
    /// A highlighter; a grammar whose query fails to load renders plain.
    pub fn new() -> Highlighter {
        Highlighter {
            engine: tree_sitter_highlight::Highlighter::new(),
            c: configure(
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY,
            ),
            rust: configure(
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
        }
    }

    /// `lines` highlighted as one snippet of `lang`, one [`Pieces`] per
    /// line (the same number of lines; each line's pieces concatenate to
    /// exactly that raw line).
    pub fn lines(&mut self, lang: Lang, lines: &[String]) -> Vec<Pieces> {
        let source = lines.join("\n");
        let config = match lang {
            Lang::C => self.c.as_ref(),
            Lang::Rust => self.rust.as_ref(),
        };
        let highlighted = config.and_then(|config| {
            let events = self
                .engine
                .highlight(config, source.as_bytes(), None, None, |_| None)
                .ok()?;
            let mut out: Vec<Pieces> = vec![Vec::new()];
            let mut stack: Vec<Class> = Vec::new();
            for event in events {
                match event.ok()? {
                    HighlightEvent::HighlightStart(h) => stack.push(class_of(h.0)),
                    HighlightEvent::HighlightEnd => {
                        stack.pop();
                    }
                    HighlightEvent::Source { start, end } => {
                        let class = stack.last().copied().unwrap_or(Class::Plain);
                        let text = source.get(start..end)?;
                        for (i, part) in text.split('\n').enumerate() {
                            if i > 0 {
                                out.push(Vec::new());
                            }
                            if !part.is_empty() {
                                let line = out.last_mut()?;
                                match line.last_mut() {
                                    Some((c, t)) if *c == class => t.push_str(part),
                                    _ => line.push((class, part.to_string())),
                                }
                            }
                        }
                    }
                }
            }
            (out.len() == lines.len().max(1)).then_some(out)
        });
        match highlighted {
            Some(mut out) => {
                out.truncate(lines.len());
                out
            }
            None => lines
                .iter()
                .map(|l| {
                    if l.is_empty() {
                        Vec::new()
                    } else {
                        vec![(Class::Plain, l.clone())]
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(pieces: &Pieces) -> String {
        pieces.iter().map(|(_, t)| t.as_str()).collect()
    }

    fn class_of_word(pieces: &[Pieces], word: &str) -> Option<Class> {
        pieces
            .iter()
            .flatten()
            .find(|(_, t)| t.trim() == word)
            .map(|(c, _)| *c)
    }

    #[test]
    fn both_languages_classify_and_keep_every_byte() {
        let mut h = Highlighter::new();
        let c: Vec<String> = [
            "static int add(int a, int b) {",
            "\t/* sum */",
            "\treturn a + b; // \u{1b}[31m",
            "}",
        ]
        .map(String::from)
        .to_vec();
        let out = h.lines(Lang::C, &c);
        assert_eq!(out.len(), c.len());
        for (pieces, raw) in out.iter().zip(&c) {
            assert_eq!(&joined(pieces), raw, "raw text survives highlighting");
        }
        assert_eq!(class_of_word(&out, "return"), Some(Class::Keyword));
        assert_eq!(class_of_word(&out, "/* sum */"), Some(Class::Comment));
        let rust: Vec<String> = [
            "pub fn add(a: i32, b: i32) -> i32 {",
            "    let s = \"x\";",
            "    a.wrapping_add(b)",
            "}",
        ]
        .map(String::from)
        .to_vec();
        let out = h.lines(Lang::Rust, &rust);
        for (pieces, raw) in out.iter().zip(&rust) {
            assert_eq!(&joined(pieces), raw);
        }
        assert_eq!(class_of_word(&out, "fn"), Some(Class::Keyword));
        assert_eq!(class_of_word(&out, "\"x\""), Some(Class::String));
    }

    #[test]
    fn blank_lines_and_garbage_keep_their_shape() {
        let mut h = Highlighter::new();
        let lines: Vec<String> = ["", "}}} (( garbage", ""].map(String::from).to_vec();
        let out = h.lines(Lang::Rust, &lines);
        assert_eq!(out.len(), 3);
        assert_eq!(joined(&out[1]), lines[1]);
        assert!(out[0].is_empty() && out[2].is_empty());
        assert!(h.lines(Lang::C, &[]).is_empty());
    }
}
