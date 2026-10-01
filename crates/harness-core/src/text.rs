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
}
