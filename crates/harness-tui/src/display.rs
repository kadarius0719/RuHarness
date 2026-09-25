//! The display filter (docs/TUI-DESIGN.md §2): what any target- or
//! model-derived text becomes before it reaches the terminal. Highlighting
//! and diffing work on the raw text first; this runs last, per rendered
//! piece, carrying the display column across the styled spans of a line so
//! tabs land on their stops.
//!
//! - `\t` expands to the next multiple of [`TAB_WIDTH`] display columns;
//! - every other control character (C0, DEL, C1) and every bidirectional
//!   formatting character becomes `?` — no escape sequence in a C comment or
//!   a model reply can reach the terminal, and no text can be visually
//!   reordered;
//! - everything else is kept (`unicode-width` gives cell widths);
//! - a line is cut at [`MAX_LINE_BYTES`] of raw text, on a char boundary,
//!   and a trailing `\r` is dropped.

use unicode_width::UnicodeWidthChar;

/// Tab stops, in display columns.
pub const TAB_WIDTH: usize = 8;
/// Longest raw line rendered, in bytes.
pub const MAX_LINE_BYTES: usize = 4096;

/// Bidirectional formatting characters (Trojan-Source): rendered as `?`.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Invisible format characters (a soft hyphen, zero-width spaces and
/// joiners, word joiners, line and paragraph separators, the byte-order
/// mark, the tag block): rendered as `?`, so two target names never look
/// alike on screen while differing (review SAFE-13).
fn is_invisible_format(c: char) -> bool {
    // Unicode's Default_Ignorable_Code_Point set, less what is filtered
    // elsewhere (controls, bidi): invisible when rendered (review SAFE-13).
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200D}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// `raw` without a trailing `\r`, cut to [`MAX_LINE_BYTES`] on a char
/// boundary.
pub fn cut(raw: &str) -> &str {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    if raw.len() <= MAX_LINE_BYTES {
        return raw;
    }
    let mut end = MAX_LINE_BYTES;
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    &raw[..end]
}

/// Sanitizes one line piece by piece, carrying the display column.
#[derive(Debug, Default, Clone)]
pub struct Sanitizer {
    col: usize,
}

impl Sanitizer {
    /// A sanitizer for a line that starts at display column `col`.
    pub fn at(col: usize) -> Sanitizer {
        Sanitizer { col }
    }

    /// The display column reached so far.
    pub fn column(&self) -> usize {
        self.col
    }

    /// `text` (the next piece of the line) made safe to render.
    pub fn push(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            if c == '\t' {
                let spaces = TAB_WIDTH - self.col % TAB_WIDTH;
                out.extend(std::iter::repeat_n(' ', spaces));
                self.col += spaces;
            } else if c.is_control() || is_bidi_control(c) || is_invisible_format(c) {
                out.push('?');
                self.col += 1;
            } else {
                out.push(c);
                self.col += c.width().unwrap_or(0);
            }
        }
        out
    }
}

/// One whole raw line made safe to render.
pub fn line(raw: &str) -> String {
    Sanitizer::default().push(cut(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_land_on_stops_across_pieces() {
        assert_eq!(line("\tif (x) {"), "        if (x) {");
        assert_eq!(line("ab\tc"), "ab      c");
        // Mixed tab/space indentation.
        assert_eq!(line("  \t  x"), "          x");
        // Carried across styled spans.
        let mut s = Sanitizer::default();
        let a = s.push("int");
        let b = s.push("\tx;");
        assert_eq!(format!("{a}{b}"), "int     x;");
        assert_eq!(s.column(), 10);
        // Wide characters count their cells.
        assert_eq!(line("日\tx"), "日      x");
    }

    #[test]
    fn controls_and_bidi_never_reach_the_terminal() {
        assert_eq!(line("a\u{1b}[31mb"), "a?[31mb");
        assert_eq!(line("x\u{7f}\u{85}y"), "x??y");
        assert_eq!(line("/* \u{202E}evil */"), "/* ?evil */");
        assert_eq!(line("crlf\r"), "crlf");
        assert_eq!(line("cr\rmid"), "cr?mid");
    }

    /// SAFE-13: invisible format characters never make two names look alike.
    #[test]
    fn invisible_format_characters_are_shown() {
        assert_eq!(line("lib\u{200B}.c"), "lib?.c");
        assert_eq!(line("\u{FEFF}a\u{00AD}b\u{2060}c\u{E0041}"), "?a?b?c?");
        assert_eq!(line("a\u{2028}b"), "a?b");
        assert_eq!(line("x\u{FE0F}\u{3164}\u{E0100}y"), "x???y");
        assert_eq!(line("a\u{206A}b\u{206F}c"), "a?b?c");
    }

    #[test]
    fn long_lines_are_cut_on_a_char_boundary() {
        let raw = format!("{}é", "x".repeat(MAX_LINE_BYTES - 1));
        let cut = cut(&raw);
        assert_eq!(
            cut.len(),
            MAX_LINE_BYTES - 1,
            "the 2-byte é straddles the cut"
        );
        assert!(cut.chars().all(|c| c == 'x'));
        assert_eq!(line("short"), "short");
    }
}
