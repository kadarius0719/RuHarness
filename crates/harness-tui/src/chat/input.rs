//! The chat's input box (docs/CHAT-PANE-DESIGN.md §5.3): a draft with a
//! cursor, line breaks kept, bounded; 1 to 5 rows on screen. Hand-rolled
//! like the note input — no crate.

use unicode_width::UnicodeWidthChar;

/// Longest draft, in bytes.
pub const MAX_DRAFT_BYTES: usize = 64 * 1024;
/// Longest paste taken at once, in bytes.
pub const MAX_PASTE_BYTES: usize = 16 * 1024;
/// Rows the box grows to.
pub const MAX_ROWS: usize = 5;

/// The draft and its cursor (a byte index on a char boundary).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    /// The text.
    pub text: String,
    /// The cursor.
    pub cursor: usize,
}

impl Input {
    /// Nothing typed.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Insert `s` at the cursor, as far as it fits the bound; the bytes
    /// dropped.
    pub fn insert(&mut self, s: &str) -> usize {
        let room = MAX_DRAFT_BYTES.saturating_sub(self.text.len());
        let mut end = s.len().min(room);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.text.insert_str(self.cursor, &s[..end]);
        self.cursor += end;
        s.len() - end
    }

    /// A pasted text: control characters but line breaks and tabs become
    /// spaces (`\r\n` and `\r` a line break); at most [`MAX_PASTE_BYTES`].
    /// The bytes dropped.
    pub fn paste(&mut self, s: &str) -> usize {
        let clean: String = s
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .map(|c| {
                if c.is_control() && c != '\n' && c != '\t' {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let mut end = clean.len().min(MAX_PASTE_BYTES);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        let over = clean.len() - end;
        over + self.insert(&clean[..end])
    }

    /// Delete the char before the cursor.
    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
            self.text.remove(self.cursor);
        }
    }

    /// Delete the char at the cursor.
    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// One char left.
    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    /// One char right.
    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
    }

    /// The start of the cursor's line.
    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// The end of the cursor's line.
    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    /// Up one line: `false` on the first line (the caller scrolls the
    /// transcript instead).
    pub fn up(&mut self) -> bool {
        let start = self.line_start();
        if start == 0 {
            return false;
        }
        let col = self.text[start..self.cursor].chars().count();
        let prev_start = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        self.cursor = column(&self.text, prev_start, start - 1, col);
        true
    }

    /// Down one line: `false` on the last line.
    pub fn down(&mut self) -> bool {
        let end = self.line_end();
        if end == self.text.len() {
            return false;
        }
        let col = self.text[self.line_start()..self.cursor].chars().count();
        let next_start = end + 1;
        let next_end = self.text[next_start..]
            .find('\n')
            .map_or(self.text.len(), |i| next_start + i);
        self.cursor = column(&self.text, next_start, next_end, col);
        true
    }

    /// Take the draft (the box empties).
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    /// Lines in the draft.
    pub fn lines(&self) -> usize {
        self.text.split('\n').count()
    }

    /// The draft's rows at `width` (each line hard-wrapped, control
    /// characters shown as `?`), the cursor's row and column, and the first
    /// row shown so the cursor is in view (at most [`MAX_ROWS`] rows).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize), usize) {
        let width = width.max(2);
        let mut rows = vec![String::new()];
        let mut col = 0;
        let mut cursor = (0, 0);
        for (i, c) in self.text.char_indices() {
            if i == self.cursor {
                cursor = (rows.len() - 1, col);
            }
            if c == '\n' {
                rows.push(String::new());
                col = 0;
                continue;
            }
            let shown = if c.is_control() || crate::display::line(&c.to_string()) != c.to_string() {
                '?'
            } else {
                c
            };
            let w = shown.width().unwrap_or(1);
            if col + w > width {
                rows.push(String::new());
                col = 0;
            }
            if let Some(r) = rows.last_mut() {
                r.push(shown);
            }
            col += w;
        }
        if self.cursor >= self.text.len() {
            if col >= width {
                rows.push(String::new());
                col = 0;
            }
            cursor = (rows.len() - 1, col);
        }
        let first = (cursor.0 + 1).saturating_sub(MAX_ROWS);
        (rows, cursor, first)
    }
}

/// The byte index of char column `col` in `text[start..end]` (its end when
/// the line is shorter).
fn column(text: &str, start: usize, end: usize, col: usize) -> usize {
    text[start..end]
        .char_indices()
        .nth(col)
        .map_or(end, |(i, _)| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_moves_and_bounds() {
        let mut i = Input::default();
        i.insert("héllo");
        i.left();
        i.left();
        i.backspace();
        assert_eq!(i.text, "hélo");
        i.home();
        i.delete();
        assert_eq!(i.text, "élo");
        i.end();
        i.insert("\nworld");
        assert!(i.up());
        assert!(!i.up(), "the first line");
        assert!(i.down());
        assert!(!i.down(), "the last line");
        assert_eq!(i.lines(), 2);
        let dropped = i.paste(&"x".repeat(MAX_PASTE_BYTES + 10));
        assert_eq!(dropped, 10);
        i.end();
        i.paste("a\r\nb\u{1b}c");
        let taken = i.take();
        assert!(taken.ends_with("a\nb c"), "{taken:?}");
        assert!(i.is_empty());
        assert_eq!(i.cursor, 0);
        let mut big = Input::default();
        assert_eq!(big.insert(&"y".repeat(MAX_DRAFT_BYTES + 3)), 3);
    }

    #[test]
    fn the_layout_keeps_the_cursor_in_view() {
        let mut i = Input::default();
        i.insert("abcdef\n1\n2\n3\n4\n5\n6");
        let (rows, cursor, first) = i.layout(4);
        assert_eq!(rows[0], "abcd");
        assert_eq!(rows[1], "ef");
        assert_eq!(cursor, (rows.len() - 1, 1));
        assert_eq!(first, rows.len() - MAX_ROWS);
        i.insert("\u{7}");
        assert!(i.layout(10).0.last().unwrap().ends_with('?'));
    }
}
