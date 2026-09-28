//! The chat's transcript (docs/CHAT-PANE-DESIGN.md §5.2): cells in order —
//! the person's messages, one cell per assistant message (its text blocks
//! streaming in place), tool lines, cockpit lines — wrapped by display
//! width through the display filter, the wrap cached per cell and width.
//! Bounded to the last [`MAX_TEXT_BYTES`] of text (older cells dropped, with
//! a line saying so). It follows the newest line unless the person scrolled
//! up; a scrolled view is anchored to a cell (by its id, which never
//! changes) and a row in it, so lines do not move under the reader as cells
//! change or drop.

use crate::display;

/// Most text kept, in bytes.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// How a row is drawn (the view picks the colours).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The person's words.
    You,
    /// The model's words.
    Claude,
    /// A read, a background line.
    Dim,
    /// Worth a look: a request, a notice.
    Warn,
    /// Went well.
    Good,
    /// Went badly.
    Bad,
    /// The cockpit's own line.
    Cockpit,
}

/// What a cell is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A message the person sent (its `uuid`).
    You {
        /// The message's uuid (the runtime echoes it).
        uuid: String,
        /// A Stop cancelled it before the model read it.
        undelivered: bool,
    },
    /// One assistant message, by id; its text blocks by index.
    Claude {
        /// The message id.
        id: String,
        /// (index, text) of its text blocks, in index order.
        blocks: Vec<(u64, String)>,
    },
    /// A tool line or a cockpit line.
    Line(Tone),
}

/// One cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Its id: never reused, never changed.
    pub id: u64,
    /// What it is.
    pub kind: Kind,
    /// Its raw text (for Claude, the blocks joined).
    pub text: String,
    wrap: Option<(usize, Vec<(Tone, String)>)>,
}

impl Cell {
    fn tone(&self) -> Tone {
        match &self.kind {
            Kind::You { .. } => Tone::You,
            Kind::Claude { .. } => Tone::Claude,
            Kind::Line(t) => *t,
        }
    }

    /// Its rows at `width` (cached): filtered, word-wrapped; the person's
    /// behind `› `, tool and cockpit lines indented; a blank row after the
    /// person's and the model's cells.
    fn rows(&mut self, width: usize) -> &[(Tone, String)] {
        if self.wrap.as_ref().is_none_or(|(w, _)| *w != width) {
            let tone = self.tone();
            let (first, rest) = match &self.kind {
                Kind::You { undelivered, .. } => {
                    let _ = undelivered;
                    ("› ", "  ")
                }
                Kind::Claude { .. } => ("", ""),
                Kind::Line(_) => ("  ", "  "),
            };
            let inner = width.saturating_sub(2).max(8);
            let mut rows = Vec::new();
            let mut text = self.text.clone();
            if let Kind::You {
                undelivered: true, ..
            } = self.kind
            {
                text.push_str(" (not delivered)");
            }
            for (i, raw) in text.split('\n').enumerate() {
                for (j, row) in wrap(raw, if first.is_empty() { width } else { inner })
                    .into_iter()
                    .enumerate()
                {
                    let lead = if i == 0 && j == 0 { first } else { rest };
                    rows.push((tone, format!("{lead}{row}")));
                }
            }
            if matches!(self.kind, Kind::You { .. } | Kind::Claude { .. }) {
                rows.push((tone, String::new()));
            }
            self.wrap = Some((width, rows));
        }
        self.wrap.as_ref().map_or(&[], |(_, r)| r.as_slice())
    }

    fn touched(&mut self) {
        self.wrap = None;
    }
}

/// Word-wrap one raw line to `width` display columns (filtered first).
pub fn wrap(raw: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    let text = display::line(raw);
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let candidate = if row.is_empty() {
            word.to_string()
        } else {
            format!("{row} {word}")
        };
        if candidate.width() <= width {
            row = candidate;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if word.width() > width {
            let mut cur = String::new();
            for c in word.chars() {
                let mut next = cur.clone();
                next.push(c);
                if next.width() > width && !cur.is_empty() {
                    rows.push(std::mem::take(&mut cur));
                    cur.push(c);
                } else {
                    cur = next;
                }
            }
            row = cur;
        } else {
            row = word.to_string();
        }
    }
    rows.push(row);
    rows
}

/// Where the view is: following the newest line, or at a row of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    /// The newest line at the bottom.
    Follow,
    /// The top row is row `row` of cell `cell` (by id).
    At {
        /// The cell's id.
        cell: u64,
        /// The row in it.
        row: usize,
    },
}

/// The transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    /// The cells, oldest first.
    pub cells: Vec<Cell>,
    next_id: u64,
    bytes: usize,
    /// Where the view is.
    pub scroll: Scroll,
    /// Cells dropped to keep the bound.
    pub dropped: usize,
}

impl Default for Transcript {
    fn default() -> Self {
        Transcript {
            cells: Vec::new(),
            next_id: 1,
            bytes: 0,
            scroll: Scroll::Follow,
            dropped: 0,
        }
    }
}

impl Transcript {
    fn push(&mut self, kind: Kind, text: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.bytes += text.len();
        self.cells.push(Cell {
            id,
            kind,
            text,
            wrap: None,
        });
        self.bound();
        id
    }

    /// Drop the oldest cells beyond the bound; a view anchored in one
    /// dropped moves to the first cell kept.
    fn bound(&mut self) {
        let mut cut = 0;
        while self.bytes > MAX_TEXT_BYTES && cut + 1 < self.cells.len() {
            self.bytes -= self.cells[cut].text.len();
            cut += 1;
        }
        if cut > 0 {
            self.cells.drain(..cut);
            self.dropped += cut;
            if let Scroll::At { cell, .. } = self.scroll {
                if self.cells.first().is_some_and(|c| c.id > cell) {
                    self.scroll = Scroll::At {
                        cell: self.cells[0].id,
                        row: 0,
                    };
                }
            }
        }
    }

    /// A message the person sent.
    pub fn you(&mut self, uuid: &str, text: &str) -> u64 {
        self.push(
            Kind::You {
                uuid: uuid.to_string(),
                undelivered: false,
            },
            text.to_string(),
        )
    }

    /// Mark the person's message `uuid` as not delivered.
    pub fn undelivered(&mut self, uuid: &str) -> bool {
        for c in self.cells.iter_mut().rev() {
            if let Kind::You {
                uuid: u,
                undelivered,
            } = &mut c.kind
            {
                if u == uuid {
                    *undelivered = true;
                    c.touched();
                    return true;
                }
            }
        }
        false
    }

    /// A tool or cockpit line; its id.
    pub fn line(&mut self, tone: Tone, text: impl Into<String>) -> u64 {
        self.push(Kind::Line(tone), text.into())
    }

    /// Replace line `id`'s text and tone (a read's "done").
    pub fn update_line(&mut self, id: u64, tone: Tone, text: impl Into<String>) {
        if let Some(c) = self.cells.iter_mut().find(|c| c.id == id) {
            let text = text.into();
            self.bytes = self.bytes - c.text.len() + text.len();
            c.text = text;
            c.kind = Kind::Line(tone);
            c.touched();
        }
    }

    fn claude_cell(&mut self, msg: &str) -> usize {
        if let Some(i) = self
            .cells
            .iter()
            .rposition(|c| matches!(&c.kind, Kind::Claude { id, .. } if id == msg))
        {
            return i;
        }
        self.push(
            Kind::Claude {
                id: msg.to_string(),
                blocks: Vec::new(),
            },
            String::new(),
        );
        self.cells.len() - 1
    }

    fn set_block(&mut self, i: usize, index: u64, f: impl FnOnce(&mut String)) {
        let c = &mut self.cells[i];
        if let Kind::Claude { blocks, .. } = &mut c.kind {
            let at = match blocks.iter().position(|(n, _)| *n == index) {
                Some(at) => at,
                None => {
                    blocks.push((index, String::new()));
                    blocks.sort_by_key(|(n, _)| *n);
                    blocks.iter().position(|(n, _)| *n == index).unwrap_or(0)
                }
            };
            f(&mut blocks[at].1);
            let text = blocks
                .iter()
                .map(|(_, t)| t.trim_end_matches('\n'))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            self.bytes = self.bytes - c.text.len() + text.len();
            c.text = text;
            c.touched();
        }
        self.bound();
    }

    /// A message starts: its cell (created now, or later by its first
    /// text).
    pub fn message_start(&mut self, msg: &str) {
        self.claude_cell(msg);
    }

    /// Streamed text of block `index` of message `msg`.
    pub fn delta(&mut self, msg: &str, index: u64, text: &str) {
        let i = self.claude_cell(msg);
        self.set_block(i, index, |b| b.push_str(text));
    }

    /// The whole text of a block of message `msg` (its `assistant` line):
    /// it replaces what streamed. The block's index is where streamed text
    /// for it went; without one, a new block after the last.
    pub fn block(&mut self, msg: &str, index: Option<u64>, text: &str) {
        let i = self.claude_cell(msg);
        let index = index.unwrap_or_else(|| match &self.cells[i].kind {
            Kind::Claude { blocks, .. } => blocks.last().map_or(0, |(n, _)| n + 1),
            _ => 0,
        });
        self.set_block(i, index, |b| *b = text.to_string());
    }

    /// Remove message `msg`'s cell if it holds no text (a message of tool
    /// calls only).
    pub fn prune_empty(&mut self, msg: &str) {
        if let Some(i) = self
            .cells
            .iter()
            .rposition(|c| matches!(&c.kind, Kind::Claude { id, .. } if id == msg))
        {
            if self.cells[i].text.is_empty() {
                let id = self.cells[i].id;
                self.cells.remove(i);
                if let Scroll::At { cell, .. } = self.scroll {
                    if cell == id {
                        self.scroll = Scroll::Follow;
                    }
                }
            }
        }
    }

    /// Every row at `width` (the whole transcript: tests, and the view's
    /// row count).
    pub fn all_rows(&mut self, width: usize) -> Vec<(Tone, String)> {
        let mut out = Vec::new();
        if self.dropped > 0 {
            out.push((Tone::Dim, "  (earlier lines dropped)".to_string()));
        }
        for c in &mut self.cells {
            out.extend_from_slice(c.rows(width));
        }
        out
    }

    /// Rows in total at `width` (the header line included).
    fn total(&mut self, width: usize) -> usize {
        usize::from(self.dropped > 0)
            + self
                .cells
                .iter_mut()
                .map(|c| c.rows(width).len())
                .sum::<usize>()
    }

    /// The rows the view shows: `height` rows at `width`, following or
    /// from the anchor — only those are copied.
    pub fn window(&mut self, width: usize, height: usize) -> Vec<(Tone, String)> {
        let total = self.total(width);
        let top = self.top(width, height, total);
        let mut out = Vec::new();
        let mut at = 0;
        if self.dropped > 0 {
            if top == 0 {
                out.push((Tone::Dim, "  (earlier lines dropped)".to_string()));
            }
            at = 1;
        }
        for c in &mut self.cells {
            if out.len() >= height {
                break;
            }
            let rows = c.rows(width);
            if at + rows.len() <= top {
                at += rows.len();
                continue;
            }
            let skip = top.saturating_sub(at);
            out.extend(rows.iter().skip(skip).take(height - out.len()).cloned());
            at += rows.len();
        }
        out
    }

    /// The index of the top row at `width` for a view `height` high.
    fn top(&mut self, width: usize, height: usize, total: usize) -> usize {
        let bottom = total.saturating_sub(height);
        match self.scroll {
            Scroll::Follow => bottom,
            Scroll::At { cell, row } => {
                let mut at = usize::from(self.dropped > 0);
                for c in &mut self.cells {
                    if c.id >= cell {
                        let n = c.rows(width).len();
                        return (at + row.min(n.saturating_sub(1))).min(bottom);
                    }
                    at += c.rows(width).len();
                }
                bottom
            }
        }
    }

    /// Whether the view follows the newest line.
    pub fn following(&self) -> bool {
        self.scroll == Scroll::Follow
    }

    /// Scroll by `by` rows (up is negative) at `width`, `height`: reaching
    /// the bottom follows again.
    pub fn scroll_by(&mut self, by: isize, width: usize, height: usize) {
        let total = self.total(width);
        let bottom = total.saturating_sub(height);
        let top = self.top(width, height, total);
        let new = (top as isize + by).clamp(0, bottom as isize) as usize;
        if new >= bottom {
            self.scroll = Scroll::Follow;
            return;
        }
        self.anchor_at(new, width);
    }

    /// The top: the first row.
    pub fn to_top(&mut self, width: usize) {
        self.anchor_at(0, width);
    }

    /// The bottom: follow again.
    pub fn follow(&mut self) {
        self.scroll = Scroll::Follow;
    }

    fn anchor_at(&mut self, mut row: usize, width: usize) {
        if self.dropped > 0 {
            row = row.saturating_sub(1);
        }
        for c in &mut self.cells {
            let n = c.rows(width).len();
            if row < n {
                self.scroll = Scroll::At { cell: c.id, row };
                return;
            }
            row -= n;
        }
        self.scroll = Scroll::Follow;
    }

    /// No cell yet.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Text held, in bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(t: &mut Transcript, width: usize) -> Vec<String> {
        t.all_rows(width).into_iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn cells_by_message_and_block_stream_in_place() {
        let mut t = Transcript::default();
        t.you("u1", "migrate this");
        t.message_start("m1");
        t.delta("m1", 1, "Checking ");
        t.delta("m1", 1, "the status.");
        t.line(Tone::Dim, "· read the project status");
        t.message_start("m2");
        t.delta("m2", 0, "It is ");
        // The assistant line replaces what streamed for its block.
        t.block("m2", Some(0), "It is pending.");
        t.block("m1", Some(1), "Checking the status.");
        assert_eq!(
            texts(&mut t, 40),
            [
                "› migrate this",
                "",
                "Checking the status.",
                "",
                "  · read the project status",
                "It is pending.",
                ""
            ]
        );
        // A message of tool calls only leaves no empty cell.
        t.message_start("m3");
        t.prune_empty("m3");
        assert_eq!(t.cells.len(), 4);
        assert!(t.undelivered("u1"));
        assert_eq!(texts(&mut t, 40)[0], "› migrate this (not delivered)");
    }

    #[test]
    fn text_is_filtered_and_wrapped_by_display_width() {
        let mut t = Transcript::default();
        t.message_start("m");
        t.block("m", Some(0), "a\u{1b}[31mb 漢字漢字漢字 end\nnext line");
        let rows = texts(&mut t, 8);
        assert!(rows.iter().all(|r| !r.contains('\u{1b}')), "{rows:?}");
        assert!(rows.iter().any(|r| r == "next"), "{rows:?}");
        use unicode_width::UnicodeWidthStr;
        assert!(rows.iter().all(|r| r.width() <= 8), "{rows:?}");
    }

    /// Mutation-checked rule (§5.2): a scrolled view stays on its lines as
    /// cells grow and new ones arrive; reaching the bottom follows again.
    #[test]
    fn a_scrolled_view_is_anchored_and_follow_comes_back() {
        let mut t = Transcript::default();
        for i in 0..20 {
            t.line(Tone::Cockpit, format!("line {i}"));
        }
        assert_eq!(t.window(20, 3).last().unwrap().1, "  line 19");
        t.scroll_by(-5, 20, 3);
        assert!(!t.following());
        let seen = t.window(20, 3);
        assert_eq!(seen[0].1, "  line 12");
        t.line(Tone::Cockpit, "line 20");
        t.message_start("m");
        t.delta("m", 0, "streaming");
        assert_eq!(t.window(20, 3), seen, "nothing moved under the reader");
        t.scroll_by(100, 20, 3);
        assert!(t.following());
        assert_eq!(t.window(20, 3)[1].1, "streaming");
        t.to_top(20);
        assert_eq!(t.window(20, 2)[0].1, "  line 0");
    }

    #[test]
    fn the_text_is_bounded_and_says_so() {
        let mut t = Transcript::default();
        let big = "x".repeat(700 * 1024);
        for _ in 0..4 {
            t.line(Tone::Dim, big.clone());
        }
        assert!(t.bytes() <= MAX_TEXT_BYTES);
        assert_eq!(t.cells.len(), 2);
        assert_eq!(t.dropped, 2);
        assert_eq!(t.all_rows(40)[0].1, "  (earlier lines dropped)");
        // An anchor in a dropped cell moves to the first kept.
        let mut t = Transcript::default();
        t.line(Tone::Dim, big.clone());
        t.line(Tone::Dim, "short");
        t.scroll = Scroll::At { cell: 1, row: 0 };
        t.line(Tone::Dim, big.clone());
        t.line(Tone::Dim, big.clone());
        assert_eq!(t.scroll, Scroll::At { cell: 2, row: 0 });
    }
}
