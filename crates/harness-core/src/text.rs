//! Which characters of target- or compiler-derived text never reach a
//! terminal as themselves: shown as `?` by the cockpit's display filter, the
//! CLI's human lines, and the features map's reasons (written and read).

/// A control character (C0, DEL, C1), a bidirectional formatting character
/// (Trojan Source: it would reorder the rest of the line) or an invisible
/// format character (Unicode's Default_Ignorable set: two names that differ
/// would look alike).
pub fn unsafe_to_show(c: char) -> bool {
    c.is_control() || is_bidi_control(c) || is_invisible_format(c)
}

/// `s` as it may be shown: every [`unsafe_to_show`] character as `?`.
pub fn safe_line(s: &str) -> String {
    s.chars()
        .map(|c| if unsafe_to_show(c) { '?' } else { c })
        .collect()
}

/// `json` with every character [`unsafe_to_show`] names written as a
/// `\uXXXX` escape (a pair of them past U+FFFF). serde_json escapes only
/// U+0000–U+001F, `"` and `\`; DEL, the C1 controls, the bidi characters
/// and the invisible format characters would otherwise reach a terminal
/// that shows the stream raw (`tail -f`, a `--json` consumer's log).
/// Outside strings JSON holds only ASCII punctuation, digits and letters,
/// so every such character is inside a string, where the escape means the
/// same character.
pub fn json_safe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if unsafe_to_show(c) {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Bidirectional formatting characters.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Invisible format characters (a soft hyphen, zero-width spaces and
/// joiners, word joiners, line and paragraph separators, the byte-order
/// mark, the tag block): Default_Ignorable_Code_Point, less the controls
/// and bidi characters filtered above.
fn is_invisible_format(c: char) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_bidi_and_invisible_characters_are_unsafe() {
        for c in [
            '\u{1b}', '\n', '\u{7f}', '\u{9b}', '\u{202E}', '\u{2066}', '\u{200B}', '\u{FEFF}',
        ] {
            assert!(unsafe_to_show(c), "{c:?}");
        }
        for c in ['a', 'é', '日', ' ', '\u{a0}', '€'] {
            assert!(!unsafe_to_show(c), "{c:?}");
        }
    }

    #[test]
    fn json_is_escaped_past_what_serde_does() {
        let json = serde_json::to_string(&"a\u{7f}b\u{9b}c\u{202E}d\u{E0001}e é").unwrap();
        let safe = json_safe(&json);
        assert_eq!(safe, r#""a\u007fb\u009bc\u202ed\udb40\udc01e é""#);
        assert_eq!(
            serde_json::from_str::<String>(&safe).unwrap(),
            "a\u{7f}b\u{9b}c\u{202E}d\u{E0001}e é"
        );
        assert_eq!(json_safe(r#"{"k":"plain"}"#), r#"{"k":"plain"}"#);
    }
}
