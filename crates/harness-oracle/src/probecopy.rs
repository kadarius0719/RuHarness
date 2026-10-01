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
/// string and character literals — raw strings and C23 digit separators
/// too — told apart from code by [`lex`].
pub(crate) fn scan_text(text: &[u8]) -> TextScan {
    let mut scan = TextScan::default();
    // Adjacent string literals, joined and decoded: across lines and line
    // markers, as the compiler joins them (§3.3 step 1).
    let mut run: Option<Vec<u8>> = None;
    let flush = |run: &mut Option<Vec<u8>>, scan: &mut TextScan| {
        if let Some(joined) = run.take() {
            note_incbin(&joined, scan);
        }
    };
    for line in lex(text) {
        if line.directive {
            if line_marker(line.row(text)).is_none() {
                flush(&mut run, &mut scan);
            }
            continue;
        }
        let toks = &line.toks;
        let mut k = 0;
        while k < toks.len() {
            let tok = &toks[k];
            if let TokKind::Literal { string: true, raw } = tok.kind {
                let body = tok.body(text);
                let decoded = if raw {
                    body.to_vec()
                } else {
                    decode_c_string(body)
                };
                run.get_or_insert_with(Vec::new).extend_from_slice(&decoded);
            } else {
                flush(&mut run, &mut scan);
            }
            if let Some((n, len)) = note_at(text, toks, k) {
                *scan.notes.entry(n).or_insert(0) += 1;
                k += len;
                continue;
            }
            if let Some(n) = end_token(text, tok) {
                *scan.ends.entry(n).or_insert(0) += 1;
            }
            if let TokKind::Literal { raw, .. } = tok.kind {
                let body = tok.body(text);
                let mut found = note_numbers(body);
                if !raw {
                    found.extend(note_numbers(&decode_c_string(body)));
                }
                scan.in_literals.extend(found);
            }
            k += 1;
        }
    }
    flush(&mut run, &mut scan);
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

/// The note numbers in a literal's text: `__ruharness_seen[N] = 1;`, spaces
/// allowed.
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
/// markers and blank lines dropped on both sides, each line compared as its
/// tokens (so spacing never counts, and `- -` is not `--`). `Err` carries the
/// first line that differs, as the copy has it.
pub(crate) fn same_code(program: &[u8], copy: &[u8], header: &Path) -> Result<(), String> {
    let header_name = header.as_os_str().as_encoded_bytes().to_vec();
    let program_lines = code_lines(program, None);
    let copy_lines = code_lines(copy, Some(&header_name));
    let n = program_lines.len().max(copy_lines.len());
    for i in 0..n {
        let (a, b) = (program_lines.get(i), copy_lines.get(i));
        if a != b {
            let shown = match (b, a) {
                (Some(line), _) => String::from_utf8_lossy(line).into_owned(),
                (None, Some(line)) => format!(
                    "(the copy ends; the program has: {})",
                    String::from_utf8_lossy(line)
                ),
                (None, None) => String::new(),
            };
            return Err(detail_text(&shown));
        }
    }
    Ok(())
}

/// The code lines of a preprocessed text, each as its tokens joined by one
/// space: markers and blank lines dropped, the probe header's region (for
/// the copy) skipped, notes and end tokens taken out.
fn code_lines(text: &[u8], header: Option<&[u8]>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut depth: usize = 0;
    let mut in_header: Option<usize> = None;
    for line in lex(text) {
        if line.directive {
            if let Some((name, flags)) = line_marker(line.row(text)) {
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
        }
        if in_header.is_some() {
            continue;
        }
        let mut joined: Vec<u8> = Vec::new();
        let mut k = 0;
        while k < line.toks.len() {
            if header.is_some() {
                if let Some((_, len)) = note_at(text, &line.toks, k) {
                    k += len;
                    continue;
                }
                if end_token(text, &line.toks[k]).is_some() {
                    k += 1;
                    continue;
                }
            }
            if !joined.is_empty() {
                joined.push(b' ');
            }
            joined.extend_from_slice(line.toks[k].text(text));
            k += 1;
        }
        if !joined.is_empty() {
            out.push(joined);
        }
    }
    out
}

/// A token of preprocessed C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    /// An identifier or keyword.
    Word,
    /// A preprocessing number (C23 digit separators included).
    Number,
    /// A string (`string`) or character literal, with any encoding prefix;
    /// `raw`: `R"delim(…)delim"`, which may span lines.
    Literal { string: bool, raw: bool },
    /// A punctuator, the longest that fits (`--` is one, `- -` two).
    Punct,
}

/// A token: its kind and its bytes in the text (`start..end`); a literal's
/// body is `body.0..body.1` (between the quotes, or a raw string's
/// parentheses).
#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: TokKind,
    start: usize,
    end: usize,
    body: (usize, usize),
}

impl Tok {
    fn text<'t>(&self, text: &'t [u8]) -> &'t [u8] {
        &text[self.start..self.end]
    }

    fn body<'t>(&self, text: &'t [u8]) -> &'t [u8] {
        &text[self.body.0..self.body.1]
    }
}

/// A line of preprocessed text: where it starts, whether it is a directive
/// (its first token `#`: a line marker or a `#pragma`), and its tokens — a
/// raw string that spans lines stays in the line it starts on.
struct Line {
    start: usize,
    directive: bool,
    toks: Vec<Tok>,
}

impl Line {
    /// The line's first physical row, for reading a line marker.
    fn row<'t>(&self, text: &'t [u8]) -> &'t [u8] {
        let rest = &text[self.start..];
        &rest[..rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len())]
    }
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Split preprocessed C into lines of tokens, as the compiler reads them:
/// encoding prefixes (`L`, `u`, `U`, `u8`), raw strings, C23 digit
/// separators (`1'000`) and the longest punctuator.
fn lex(text: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = Line {
        start: 0,
        directive: false,
        toks: Vec::new(),
    };
    let mut i = 0;
    while i < text.len() {
        let b = text[i];
        if b == b'\n' {
            let next = Line {
                start: i + 1,
                directive: false,
                toks: Vec::new(),
            };
            lines.push(std::mem::replace(&mut line, next));
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if line.toks.is_empty() && b == b'#' {
            line.directive = true;
        }
        let tok = if ident_byte(b) && !b.is_ascii_digit() {
            let mut j = i + 1;
            while j < text.len() && ident_byte(text[j]) {
                j += 1;
            }
            let word = &text[i..j];
            let raw = matches!(word, b"R" | b"LR" | b"uR" | b"UR" | b"u8R");
            let prefix = matches!(word, b"L" | b"u" | b"U" | b"u8");
            match text.get(j) {
                Some(b'"') if raw => raw_string(text, i, j).unwrap_or(Tok {
                    kind: TokKind::Word,
                    start: i,
                    end: j,
                    body: (j, j),
                }),
                Some(q @ (b'"' | b'\'')) if prefix => quoted(text, i, j, *q == b'"'),
                _ => Tok {
                    kind: TokKind::Word,
                    start: i,
                    end: j,
                    body: (j, j),
                },
            }
        } else if b.is_ascii_digit()
            || (b == b'.' && text.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let mut j = i + 1;
            loop {
                match text.get(j) {
                    Some(b'e' | b'E' | b'p' | b'P')
                        if matches!(text.get(j + 1), Some(b'+' | b'-')) =>
                    {
                        j += 2
                    }
                    Some(c) if ident_byte(*c) || *c == b'.' => j += 1,
                    // A C23 digit separator: `'` then a digit or a letter.
                    Some(b'\'')
                        if text
                            .get(j + 1)
                            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') =>
                    {
                        j += 2
                    }
                    _ => break,
                }
            }
            Tok {
                kind: TokKind::Number,
                start: i,
                end: j,
                body: (j, j),
            }
        } else if b == b'"' || b == b'\'' {
            quoted(text, i, i, b == b'"')
        } else {
            let len = PUNCTUATORS
                .iter()
                .find(|p| text[i..].starts_with(p))
                .map_or(1, |p| p.len());
            Tok {
                kind: TokKind::Punct,
                start: i,
                end: i + len,
                body: (i, i),
            }
        };
        line.toks.push(tok);
        i = tok.end;
    }
    lines.push(line);
    lines
}

/// Punctuators longer than one byte, longest first.
const PUNCTUATORS: &[&[u8]] = &[
    b"%:%:", b"...", b"<<=", b">>=", b"->", b"++", b"--", b"<<", b">>", b"<=", b">=", b"==", b"!=",
    b"&&", b"||", b"*=", b"/=", b"%=", b"+=", b"-=", b"&=", b"^=", b"|=", b"##", b"<:", b":>",
    b"<%", b"%>", b"%:", b"::",
];

/// A string or character literal from `start` (its prefix), its quote at
/// `quote`; an unterminated one ends at the line's end.
fn quoted(text: &[u8], start: usize, quote: usize, string: bool) -> Tok {
    let q = text[quote];
    let mut j = quote + 1;
    while j < text.len() && text[j] != q && text[j] != b'\n' {
        j += if text[j] == b'\\' && text.get(j + 1).is_some_and(|c| *c != b'\n') {
            2
        } else {
            1
        };
    }
    let body_end = j.min(text.len());
    let end = if text.get(j) == Some(&q) {
        j + 1
    } else {
        body_end
    };
    Tok {
        kind: TokKind::Literal { string, raw: false },
        start,
        end,
        body: (quote + 1, body_end),
    }
}

/// A raw string from `start` (its prefix), its quote at `quote`:
/// `"delim(` … `)delim"`, the delimiter at most 16 bytes. `None` when it is
/// not one.
fn raw_string(text: &[u8], start: usize, quote: usize) -> Option<Tok> {
    let from = quote + 1;
    let open = text[from..]
        .iter()
        .take(17)
        .position(|b| *b == b'(')
        .map(|at| from + at)?;
    let delim = &text[from..open];
    if delim
        .iter()
        .any(|b| b.is_ascii_whitespace() || matches!(b, b')' | b'\\' | b'"'))
    {
        return None;
    }
    let mut close: Vec<u8> = vec![b')'];
    close.extend_from_slice(delim);
    close.push(b'"');
    let at = find(&text[open + 1..], &close)? + open + 1;
    Some(Tok {
        kind: TokKind::Literal {
            string: true,
            raw: true,
        },
        start,
        end: at + close.len(),
        body: (open + 1, at),
    })
}

/// The note that starts at token `k` (`__ruharness_seen [ N ] = 1 ;`): its
/// number and how many tokens it takes.
fn note_at(text: &[u8], toks: &[Tok], k: usize) -> Option<(u32, usize)> {
    let t = |d: usize| toks.get(k + d).map(|tok| tok.text(text));
    if t(0)? != b"__ruharness_seen"
        || t(1)? != b"["
        || t(3)? != b"]"
        || t(4)? != b"="
        || t(5)? != b"1"
        || t(6)? != b";"
    {
        return None;
    }
    let digits = t(2)?;
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = std::str::from_utf8(digits).ok()?.parse().ok()?;
    Some((n, 7))
}

/// The number of an end token (`__ruharness_end_N`, a whole word).
fn end_token(text: &[u8], tok: &Tok) -> Option<u32> {
    if tok.kind != TokKind::Word {
        return None;
    }
    let digits = tok.text(text).strip_prefix(b"__ruharness_end_")?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
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
            Err("int f ( void ) { return 2 ; }".to_string())
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
        // Tokens, not spacing: `- -` is two tokens, `--` one.
        assert!(same_code(b"x = a - -b;\n", b"x = a--b;\n", Path::new("/o/h")).is_err());
        assert_eq!(
            same_code(b"x = a - -b;\n", b"x = a- - b;\n", Path::new("/o/h")),
            Ok(())
        );
        // Review (non-UTF-8 lines): a Latin-1 byte in a literal compares as
        // the byte it is, and the end token's spaces do not count.
        let program = b"int greet(void) { return puts(\"caf\xe9\"); }\n";
        let copy = b"int greet(void) {__ruharness_seen[0] = 1; return puts(\"caf\xe9\");  __ruharness_end_0 }\n";
        assert_eq!(same_code(program, copy, Path::new("/o/h")), Ok(()));
        let other = b"int greet(void) {__ruharness_seen[0] = 1; return puts(\"caf\xe8\");  __ruharness_end_0 }\n";
        assert!(same_code(program, other, Path::new("/o/h")).is_err());
    }

    /// Review (a raw string or a C23 digit separator on the same line): a
    /// stringized note after one is still inside its literal — never code
    /// the check takes out.
    #[test]
    fn raw_strings_and_digit_separators_keep_literals_in_step() {
        let copy = b"static const char *pre = R\"(\")\"; static const char frag[] = \
                     \"void shade(void) {__ruharness_seen[3] = 1; go(); __ruharness_end_3}\";\n";
        let scan = scan_text(copy);
        assert_eq!(scan.in_literals, BTreeSet::from([3]));
        assert!(scan.notes.is_empty() && scan.ends.is_empty(), "{scan:?}");
        let program = b"static const char *pre = R\"(\")\"; static const char frag[] = \
                        \"void shade(void) { go(); }\";\n";
        assert!(same_code(program, copy, Path::new("/o/h")).is_err());
        // A raw string over two lines, with a delimiter and a quote inside.
        let copy =
            b"const char *r = R\"x(a \" )\"\nb)x\"; int f(void) {__ruharness_seen[1] = 1; }\n";
        let scan = scan_text(copy);
        assert_eq!(scan.notes, BTreeMap::from([(1, 1)]));
        assert!(scan.in_literals.is_empty());
        // C23 digit separators: `1'000` is one number, not a character literal.
        let copy = b"int k = 1'000; const char *s = \"[1'0] {__ruharness_seen[2] = 1; }\";\n";
        assert_eq!(scan_text(copy).in_literals, BTreeSet::from([2]));
        let copy = b"int k = 1'000'000; int f(void) {__ruharness_seen[4] = 1; }\n";
        assert_eq!(scan_text(copy).notes, BTreeMap::from([(4, 1)]));
        // Encoding prefixes, and a quote in a character literal.
        let copy = b"char c = '\"'; const char *u = u8\"{__ruharness_seen[5] = 1;\"; wchar_t w = L'\\'';\n";
        let scan = scan_text(copy);
        assert_eq!(scan.in_literals, BTreeSet::from([5]));
        assert!(scan.notes.is_empty());
    }

    /// Review (`.incbin` split across lines): string literals on several
    /// lines, with a line marker between them, are one run.
    #[test]
    fn an_incbin_split_across_lines_is_read() {
        let text = b"__asm__(\".data\\n.inc\"\n\"bin \\\"unit.c\\\"\\n\");\n\
                     __asm__(\".incbin \"\n# 7 \"a.c\"\n\"\\\"b.c\\\"\");\n";
        assert_eq!(
            scan_text(text).incbins,
            vec![Some("unit.c".to_string()), Some("b.c".to_string())]
        );
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
