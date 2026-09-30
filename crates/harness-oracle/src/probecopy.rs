//! The features map's scratch copy and its notes
//! (docs/FEATURES-PROBE-REDESIGN.md §3.1–§3.3): which functions carry a note
//! and why the others do not; the listing runs (`cc -E -MD -MF -H`); what the
//! copy's preprocessed text says (notes turned into text, notes in a skipped
//! branch, files read by `.incbin`); and the check that the copy, apart from
//! its notes, preprocesses to the program's own code.

use harness_core::error::Error;
use harness_scan::{NoNote, PlacedNote, ProbeOptions};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Why a function has no note (docs/FEATURES-PROBE-REDESIGN.md §3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// The parser could not read its definition.
    Parser,
    /// Its body is not a `{ }` block.
    NotABlock,
    /// A `#` line between its head and its body.
    ConditionalBrace,
    /// Its body's brace is inside `#if` (the end-token count).
    SkippedBranch,
    /// A naked function (gcc).
    Naked,
    /// Its body is inside a macro argument that becomes a string.
    Stringized,
    /// Its file is read as data.
    Data,
    /// A note at its start does not compile.
    Compile,
    /// Its note broke the build, found by building without it.
    Elimination,
    /// The program does not link with its note.
    Link,
    /// The scratch copy of its file did not build with notes.
    FileLimit,
    /// The notes check stopped at its limit before its file.
    NotChecked,
}

impl Kind {
    /// The kind's name in `map.json`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Parser => "parser",
            Kind::NotABlock => "not-a-block",
            Kind::ConditionalBrace => "conditional-brace",
            Kind::SkippedBranch => "skipped-branch",
            Kind::Naked => "naked",
            Kind::Stringized => "stringized",
            Kind::Data => "data",
            Kind::Compile => "compile",
            Kind::Elimination => "elimination",
            Kind::Link => "link",
            Kind::FileLimit => "file-limit",
            Kind::NotChecked => "not-checked",
        }
    }

    fn of(rule: NoNote) -> Kind {
        match rule {
            NoNote::Parser => Kind::Parser,
            NoNote::NotABlock => Kind::NotABlock,
            NoNote::ConditionalBrace => Kind::ConditionalBrace,
            NoNote::Naked => Kind::Naked,
        }
    }
}

/// Why a function has no note: the kind and its detail (a file, a
/// compiler's message, a symbol), at most [`DETAIL_MAX`] bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reason {
    pub kind: Kind,
    pub detail: String,
}

/// The longest detail kept (§3.7).
pub(crate) const DETAIL_MAX: usize = 160;

impl Reason {
    pub(crate) fn new(kind: Kind, detail: &str) -> Reason {
        Reason {
            kind,
            detail: detail_text(detail),
        }
    }
}

/// `text` as a reason's detail: Unicode control characters as `?`, cut on a
/// character boundary to at most [`DETAIL_MAX`] bytes.
pub(crate) fn detail_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() { '?' } else { c };
        if out.len() + c.len_utf8() > DETAIL_MAX {
            break;
        }
        out.push(c);
    }
    out
}

/// One file of `source_dir` whose functions the facts record.
#[derive(Debug, Clone)]
struct File {
    original: Vec<u8>,
    /// Every canonical id the file defines (noted or not).
    ids: Vec<String>,
    /// Ids that carry no note: the rules' and the pass's.
    skip: Vec<String>,
    /// The whole file goes back unprobed.
    unprobed: bool,
    /// The notes of the last compile copy written.
    notes: Vec<PlacedNote>,
}

/// The scratch copy's probed files and every function's reason for having
/// no note.
#[derive(Debug, Default)]
pub(crate) struct Probe {
    files: BTreeMap<String, File>,
    /// `(file, id)` → why it has no note.
    pub reasons: BTreeMap<(String, String), Reason>,
    gcc: bool,
}

impl Probe {
    pub(crate) fn new(gcc: bool) -> Probe {
        Probe {
            gcc,
            ..Probe::default()
        }
    }

    /// Add `rel` (its original bytes) and write its copy to `mirror/rel`.
    pub(crate) fn add(
        &mut self,
        mirror: &Path,
        rel: &str,
        original: Vec<u8>,
        index_of: &dyn Fn(&str) -> Option<u32>,
        end_tokens: bool,
    ) -> Result<(), Error> {
        self.files.insert(
            rel.to_string(),
            File {
                original,
                ids: Vec::new(),
                skip: Vec::new(),
                unprobed: false,
                notes: Vec::new(),
            },
        );
        self.write(mirror, rel, index_of, end_tokens)
    }

    /// The probed files' repo-relative paths.
    pub(crate) fn rels(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    /// Whether `rel` is a probed file still carrying notes.
    pub(crate) fn is_probed(&self, rel: &str) -> bool {
        self.files.get(rel).is_some_and(|f| !f.unprobed)
    }

    /// The notes `rel`'s compile copy carries (empty for an unprobed file).
    pub(crate) fn notes(&self, rel: &str) -> &[PlacedNote] {
        self.files.get(rel).map_or(&[], |f| f.notes.as_slice())
    }

    /// Write `rel`'s copy — the compile copy, or with `end_tokens` the
    /// listing copy — recording the rules' reasons and the notes placed.
    pub(crate) fn write(
        &mut self,
        mirror: &Path,
        rel: &str,
        index_of: &dyn Fn(&str) -> Option<u32>,
        end_tokens: bool,
    ) -> Result<(), Error> {
        self.write_with(mirror, rel, index_of, end_tokens, &[])
    }

    /// [`Probe::write`] with `extra` ids also left without a note, for this
    /// write only (the search's trial compiles, §3.4 step 4).
    pub(crate) fn write_with(
        &mut self,
        mirror: &Path,
        rel: &str,
        index_of: &dyn Fn(&str) -> Option<u32>,
        end_tokens: bool,
        extra: &[String],
    ) -> Result<(), Error> {
        let file = self
            .files
            .get_mut(rel)
            .ok_or_else(|| Error::Invariant(format!("{rel} is not a probed file")))?;
        let skip: Vec<String> = file.skip.iter().chain(extra).cloned().collect();
        let (bytes, notes) = if file.unprobed {
            (file.original.clone(), Vec::new())
        } else {
            let probed = harness_scan::probe_source(
                rel,
                &file.original,
                index_of,
                ProbeOptions {
                    skip: &skip,
                    gcc: self.gcc,
                    end_tokens,
                },
            )?;
            for (id, rule) in &probed.unwatched {
                self.reasons
                    .entry((rel.to_string(), id.clone()))
                    .or_insert_with(|| Reason::new(Kind::of(*rule), ""));
                if !file.skip.contains(id) {
                    file.skip.push(id.clone());
                }
            }
            for id in probed
                .notes
                .iter()
                .map(|n| &n.id)
                .chain(probed.unwatched.iter().map(|(id, _)| id))
            {
                if !file.ids.contains(id) {
                    file.ids.push(id.clone());
                }
            }
            (probed.source, probed.notes)
        };
        if !end_tokens && extra.is_empty() {
            file.notes = notes;
        }
        crate::featuremap::write_file(&mirror.join(rel), &bytes)
    }

    /// Take `id`'s note out of `rel` (the caller rewrites the file).
    /// Returns whether it still had one.
    pub(crate) fn take_out(&mut self, rel: &str, id: &str, reason: Reason) -> bool {
        let Some(file) = self.files.get_mut(rel) else {
            return false;
        };
        if file.unprobed || file.skip.iter().any(|s| s == id) {
            return false;
        }
        file.skip.push(id.to_string());
        self.reasons
            .entry((rel.to_string(), id.to_string()))
            .or_insert(reason);
        true
    }

    /// Put `id`'s note back in `rel` (the search's restore pass).
    pub(crate) fn put_back(&mut self, rel: &str, id: &str) {
        if let Some(file) = self.files.get_mut(rel) {
            file.skip.retain(|s| s != id);
            self.reasons.remove(&(rel.to_string(), id.to_string()));
        }
    }

    /// Send `rel` back unprobed: every function in it gets `reason`.
    /// Returns whether it was probed.
    pub(crate) fn unprobe(&mut self, rel: &str, reason: Reason) -> bool {
        let Some(file) = self.files.get_mut(rel) else {
            return false;
        };
        if file.unprobed {
            return false;
        }
        file.unprobed = true;
        for id in &file.ids {
            self.reasons
                .entry((rel.to_string(), id.clone()))
                .or_insert_with(|| reason.clone());
        }
        true
    }

    /// Whether `rel`'s original holds a `#line` or line-marker directive
    /// (gcc reports such a file's errors at presumed places, §3.4 step 3).
    pub(crate) fn has_line_directives(&self, rel: &str) -> bool {
        self.files.get(rel).is_some_and(|f| {
            f.original.split(|b| *b == b'\n').any(|line| {
                let t = trim_start(line);
                t.strip_prefix(b"#").is_some_and(|r| {
                    let r = trim_start(r);
                    r.starts_with(b"line") || r.first().is_some_and(u8::is_ascii_digit)
                })
            })
        })
    }

    /// The unwatched pairs, sorted.
    pub(crate) fn unwatched(&self) -> Vec<(String, String)> {
        self.reasons.keys().cloned().collect()
    }
}

/// What a listing run's preprocessed text says (§3.2, §3.3).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TextScan {
    /// Note number → how often its note appears as code.
    pub notes: BTreeMap<u32, usize>,
    /// Note numbers whose note text sits inside a string or character
    /// literal.
    pub in_literals: BTreeSet<u32>,
    /// Note number → how often its end token appears.
    pub ends: BTreeMap<u32, usize>,
    /// The files `.incbin` names (`None`: one whose name cannot be read).
    pub incbins: Vec<Option<String>>,
}

/// Read a preprocessed text: line markers and `#pragma` lines skipped,
/// string and character literals told apart from code.
pub(crate) fn scan_text(text: &[u8]) -> TextScan {
    let mut scan = TextScan::default();
    for line in text.split(|b| *b == b'\n') {
        let trimmed = trim_start(line);
        if trimmed.starts_with(b"#") {
            continue;
        }
        let mut code: Vec<u8> = Vec::new();
        let mut literals: Vec<Vec<u8>> = Vec::new();
        let mut run: Option<Vec<u8>> = None;
        let mut i = 0;
        while i < line.len() {
            let b = line[i];
            if b == b'"' || b == b'\'' {
                let quote = b;
                let start = i + 1;
                i += 1;
                while i < line.len() && line[i] != quote {
                    i += if line[i] == b'\\' { 2 } else { 1 };
                }
                let body = &line[start..i.min(line.len())];
                if quote == b'"' {
                    let decoded = decode_c_string(body);
                    run.get_or_insert_with(Vec::new).extend_from_slice(&decoded);
                }
                literals.push(body.to_vec());
                i += 1;
                code.push(b' ');
                continue;
            }
            if !b.is_ascii_whitespace() {
                if let Some(joined) = run.take() {
                    note_incbin(&joined, &mut scan);
                }
            }
            code.push(b);
            i += 1;
        }
        if let Some(joined) = run.take() {
            note_incbin(&joined, &mut scan);
        }
        for n in note_numbers(&code) {
            *scan.notes.entry(n).or_insert(0) += 1;
        }
        for n in end_numbers(&code) {
            *scan.ends.entry(n).or_insert(0) += 1;
        }
        for literal in &literals {
            for n in note_numbers(&decode_c_string(literal)) {
                scan.in_literals.insert(n);
            }
            for n in note_numbers(literal) {
                scan.in_literals.insert(n);
            }
        }
    }
    scan
}

fn trim_start(line: &[u8]) -> &[u8] {
    let at = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(line.len());
    &line[at..]
}

/// A joined run of string literals that mentions `incbin` (any case): the
/// quoted name after it, or `None` when there is none to read.
fn note_incbin(joined: &[u8], scan: &mut TextScan) {
    let lower: Vec<u8> = joined.iter().map(u8::to_ascii_lowercase).collect();
    let mut from = 0;
    while let Some(at) = find(&lower[from..], b"incbin") {
        let after = from + at + b"incbin".len();
        let rest = trim_start(&joined[after..]);
        let name = rest.strip_prefix(b"\"").and_then(|r| {
            r.iter()
                .position(|b| *b == b'"')
                .map(|end| String::from_utf8_lossy(&r[..end]).into_owned())
        });
        scan.incbins.push(name);
        from = after;
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The note numbers in `text`: `__ruharness_seen[N] = 1;`, spaces allowed.
fn note_numbers(text: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let key = b"__ruharness_seen";
    let mut from = 0;
    while let Some(at) = find(&text[from..], key) {
        let mut i = from + at + key.len();
        from = i;
        let skip_ws = |i: &mut usize| {
            while *i < text.len() && text[*i].is_ascii_whitespace() {
                *i += 1;
            }
        };
        skip_ws(&mut i);
        if text.get(i) != Some(&b'[') {
            continue;
        }
        i += 1;
        skip_ws(&mut i);
        let digits_from = i;
        while i < text.len() && text[i].is_ascii_digit() {
            i += 1;
        }
        let Ok(n) = std::str::from_utf8(&text[digits_from..i])
            .unwrap_or("")
            .parse::<u32>()
        else {
            continue;
        };
        skip_ws(&mut i);
        let expect = |c: u8, i: &mut usize| -> bool {
            skip_ws(i);
            if text.get(*i) == Some(&c) {
                *i += 1;
                true
            } else {
                false
            }
        };
        if expect(b']', &mut i)
            && expect(b'=', &mut i)
            && expect(b'1', &mut i)
            && expect(b';', &mut i)
        {
            out.push(n);
        }
    }
    out
}

/// The end-token numbers in `text`: `__ruharness_end_N` as a whole word.
fn end_numbers(text: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let key = b"__ruharness_end_";
    let mut from = 0;
    while let Some(at) = find(&text[from..], key) {
        let start = from + at;
        let mut i = start + key.len();
        from = i;
        let before_ok = start == 0 || !is_word(text[start - 1]);
        let digits_from = i;
        while i < text.len() && text[i].is_ascii_digit() {
            i += 1;
        }
        let after_ok = i >= text.len() || !is_word(text[i]);
        if before_ok && after_ok && i > digits_from {
            if let Ok(n) = std::str::from_utf8(&text[digits_from..i])
                .unwrap_or("")
                .parse()
            {
                out.push(n);
            }
        }
    }
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A C string literal's body decoded to bytes: the simple escapes and octal
/// and hex escapes; anything else kept as it is.
pub(crate) fn decode_c_string(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] != b'\\' || i + 1 >= body.len() {
            out.push(body[i]);
            i += 1;
            continue;
        }
        let e = body[i + 1];
        i += 2;
        match e {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'0'..=b'7' => {
                let mut v: u32 = u32::from(e - b'0');
                let mut k = 0;
                while k < 2 && i < body.len() && (b'0'..=b'7').contains(&body[i]) {
                    v = v * 8 + u32::from(body[i] - b'0');
                    i += 1;
                    k += 1;
                }
                out.push((v & 0xff) as u8);
            }
            b'x' => {
                let mut v: u32 = 0;
                while i < body.len() && body[i].is_ascii_hexdigit() {
                    v = v * 16 + (body[i] as char).to_digit(16).unwrap_or(0);
                    i += 1;
                }
                out.push((v & 0xff) as u8);
            }
            other => out.push(other),
        }
    }
    out
}

/// A line marker (`# 12 "file" 1 3`): the file name decoded, and its flags.
fn line_marker(line: &[u8]) -> Option<(Vec<u8>, Vec<u32>)> {
    let rest = trim_start(line).strip_prefix(b"#")?;
    let rest = trim_start(rest);
    let rest = rest.strip_prefix(b"line").map_or(rest, trim_start);
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = trim_start(&rest[digits..]);
    let body = rest.strip_prefix(b"\"")?;
    let mut i = 0;
    while i < body.len() && body[i] != b'"' {
        i += if body[i] == b'\\' { 2 } else { 1 };
    }
    let name = decode_c_string(&body[..i.min(body.len())]);
    let flags = String::from_utf8_lossy(body.get(i + 1..).unwrap_or(&[]))
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .collect();
    Some((name, flags))
}

/// §3.3 step 4: the copy's preprocessed text, with the probe header's region
/// and every note and end token taken out, must be the program's code — line
/// markers and blank lines dropped on both sides, whitespace runs read as one
/// space. `Err` carries the first line that differs, as the copy has it.
pub(crate) fn same_code(program: &[u8], copy: &[u8], header: &Path) -> Result<(), String> {
    let header_name = header.as_os_str().as_encoded_bytes().to_vec();
    let program_lines = code_lines(program, None);
    let copy_lines = code_lines(copy, Some(&header_name));
    let n = program_lines.len().max(copy_lines.len());
    for i in 0..n {
        let (a, b) = (program_lines.get(i), copy_lines.get(i));
        if a != b {
            let shown = match (b, a) {
                (Some(line), _) => line.clone(),
                (None, Some(line)) => format!("(the copy ends; the program has: {line})"),
                (None, None) => String::new(),
            };
            return Err(detail_text(&shown));
        }
    }
    Ok(())
}

/// The code lines of a preprocessed text: markers and blank lines dropped,
/// the probe header's region (for the copy) skipped, notes and end tokens
/// taken out, whitespace runs as one space.
fn code_lines(text: &[u8], header: Option<&[u8]>) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth: usize = 0;
    let mut in_header: Option<usize> = None;
    for line in text.split(|b| *b == b'\n') {
        if let Some((name, flags)) = line_marker(line) {
            if flags.contains(&1) {
                depth += 1;
                if in_header.is_none() && header.is_some_and(|h| name == h) {
                    in_header = Some(depth - 1);
                }
            } else if flags.contains(&2) {
                depth = depth.saturating_sub(1);
                if in_header == Some(depth) {
                    in_header = None;
                }
            }
            continue;
        }
        if in_header.is_some() {
            continue;
        }
        let mut code = line.to_vec();
        if header.is_some() {
            code = strip_notes(&code);
        }
        let tokens = token_text(&code);
        if tokens.is_empty() {
            continue;
        }
        out.push(tokens);
    }
    out
}

/// `line` as its tokens: whitespace kept (as one space) only where it
/// separates two word characters, and inside literals as it is — so taking
/// a note or end token out of `{}` compares equal, and `int x` never reads as
/// `intx`.
fn token_text(line: &[u8]) -> String {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let mut out: Vec<u8> = Vec::with_capacity(line.len());
    let mut pending_space = false;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if b.is_ascii_whitespace() {
            pending_space = true;
            i += 1;
            continue;
        }
        if pending_space && out.last().is_some_and(|l| word(*l)) && word(b) {
            out.push(b' ');
        }
        pending_space = false;
        if b == b'"' || b == b'\'' {
            let start = i;
            i += 1;
            while i < line.len() && line[i] != b {
                i += if line[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(line.len());
            out.extend_from_slice(&line[start..i]);
            continue;
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `line` with every note (`__ruharness_seen[N] = 1;`) and end token
/// (`__ruharness_end_N`) taken out, outside literals.
fn strip_notes(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if b == b'"' || b == b'\'' {
            let start = i;
            i += 1;
            while i < line.len() && line[i] != b {
                i += if line[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(line.len());
            out.extend_from_slice(&line[start..i]);
            continue;
        }
        if line[i..].starts_with(b"__ruharness_seen") {
            if let Some(end) = note_end(&line[i..]) {
                i += end;
                continue;
            }
        }
        if line[i..].starts_with(b"__ruharness_end_") && (i == 0 || !is_word(line[i - 1])) {
            let mut j = i + b"__ruharness_end_".len();
            while j < line.len() && line[j].is_ascii_digit() {
                j += 1;
            }
            if j < line.len() && is_word(line[j]) {
                out.push(b);
                i += 1;
                continue;
            }
            i = j;
            continue;
        }
        out.push(b);
        i += 1;
    }
    out
}

/// The length of the note at the start of `text`, if one is there.
fn note_end(text: &[u8]) -> Option<usize> {
    let mut i = b"__ruharness_seen".len();
    let ws = |i: &mut usize| {
        while *i < text.len() && text[*i].is_ascii_whitespace() {
            *i += 1;
        }
    };
    for part in [&b"["[..], b"", b"]", b"=", b"1", b";"] {
        ws(&mut i);
        if part.is_empty() {
            let from = i;
            while i < text.len() && text[i].is_ascii_digit() {
                i += 1;
            }
            if i == from {
                return None;
            }
            continue;
        }
        if !text[i..].starts_with(part) {
            return None;
        }
        i += part.len();
    }
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_says_where_notes_are() {
        let text =
            b"# 1 \"a.c\"\nint f(void) {__ruharness_seen[0] = 1; return 1;  __ruharness_end_0 }\n\
                     const char *s = \"void g(void) {__ruharness_seen[1] = 1; }\";\n\
                     int h(void) { __ruharness_end_2 }\n\
                     __asm__(\".\" \"INCBIN \\\"unit.c\\\"\");\n\
                     __asm__(\".incbin \" MACRO);\n";
        let scan = scan_text(text);
        assert_eq!(scan.notes, BTreeMap::from([(0, 1)]));
        assert_eq!(scan.in_literals, BTreeSet::from([1]));
        assert_eq!(scan.ends, BTreeMap::from([(0, 1), (2, 1)]));
        assert_eq!(scan.incbins, vec![Some("unit.c".to_string()), None]);
    }

    #[test]
    fn the_copy_is_the_programs_code_without_its_notes() {
        let program = b"# 1 \"/t/src/a.c\"\n# 1 \"<built-in>\" 1\n# 1 \"/t/src/a.c\" 2\n\
                        int f(void) { return 1; }\n\nint x = 3;\n";
        let copy = b"# 1 \"/m/src/a.c\"\n# 1 \"<built-in>\" 1\n# 1 \"/o/fnprobe.h\" 1\n\
                     extern unsigned char *volatile __ruharness_seen;\n# 2 \"<built-in>\" 2\n\
                     # 1 \"/m/src/a.c\" 2\n\
                     int f(void) {__ruharness_seen[0] = 1; return 1;  __ruharness_end_0 }\n\n\n\
                     int x = 3;\n";
        assert_eq!(same_code(program, copy, Path::new("/o/fnprobe.h")), Ok(()));
        let different =
            b"# 1 \"/m/src/a.c\"\nint f(void) {__ruharness_seen[0] = 1; return 2; }\nint x = 3;\n";
        assert_eq!(
            same_code(program, different, Path::new("/o/fnprobe.h")),
            Err("int f(void){return 2;}".to_string())
        );
        // An empty body: the note and end token out, `{ }` is `{}`.
        let program = b"void f(void) {}\nint x;\n";
        let copy = b"void f(void) {__ruharness_seen[0] = 1; __ruharness_end_0 }\nint  x;\n";
        assert_eq!(same_code(program, copy, Path::new("/o/fnprobe.h")), Ok(()));
        // Words stay apart; spaces inside a literal count.
        assert!(same_code(b"int x;\n", b"intx;\n", Path::new("/o/h")).is_err());
        assert!(same_code(
            b"char *s = \"a b\";\n",
            b"char *s = \"a  b\";\n",
            Path::new("/o/h")
        )
        .is_err());
        // A note turned into text is not taken out: the texts differ.
        let program = b"const char *s = \"{ }\";\n";
        let copy = b"const char *s = \"{__ruharness_seen[0] = 1; }\";\n";
        assert!(same_code(program, copy, Path::new("/o/fnprobe.h")).is_err());
    }

    #[test]
    fn details_are_cut_on_a_character_boundary() {
        let long = "é".repeat(100);
        let d = detail_text(&long);
        assert!(d.len() <= DETAIL_MAX && d.len() >= DETAIL_MAX - 1);
        assert_eq!(detail_text("a\u{1b}b\nc"), "a?b?c");
    }

    #[test]
    fn line_markers_decode_their_names() {
        let (name, flags) = line_marker(b"# 12 \"/t/caf\\303\\251 dir/a\\\\b.h\" 1 3").unwrap();
        assert_eq!(name, "/t/café dir/a\\b.h".as_bytes());
        assert_eq!(flags, vec![1, 3]);
        assert!(line_marker(b"#pragma once").is_none());
    }
}
