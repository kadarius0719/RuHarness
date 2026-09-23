//! The executor's emission contract (docs/SCHEMAS.md "Emission contract"):
//! turning one untrusted model reply into exactly the files of a stage's
//! [`FileSpec`] — `src/logic.rs` and `src/ffi.rs` for migrate
//! ([`MIGRATE_SPEC`]), `driver.c` for driver generation ([`DRIVER_SPEC`]) —
//! or into a closed reason why not.
//!
//! Everything here is a pure function of its arguments (no I/O, no clock),
//! so a trajectory replayed from traces parses identically.
//!
//! - [`parse_spec`] never returns partial files: any truncation signal
//!   yields [`ParsedEmission::Truncated`], and file paths are resolved by
//!   EXACT allowlist lookup — a path is never sanitized into acceptance.
//!   [`parse_emission`] is the migrate spec's historical entry point; its
//!   behavior is byte-for-byte what it was before specs existed.
//! - [`deny_scan`] is **quality feedback only**. It tells the model early,
//!   in words, about constructs the structure rules forbid; it is trivially
//!   evadable and nothing relies on it. The security boundaries are the
//!   compiler-enforced lint structure of the harness-owned `src/lib.rs`, the
//!   oracle's symbol-set check, and the sandbox (docs/SCHEMAS.md "Trust
//!   boundaries"). It applies to the migrate stage only.
//! - [`render_spec`] (and [`render_files`] for migrate) is the inverse of
//!   the parser for well-formed files: it is how a repair turn shows the
//!   model its current candidate.

use harness_core::traits::StopKind;

/// The safe-logic file of a candidate crate (allowlisted emission path).
pub const LOGIC_PATH: &str = "src/logic.rs";
/// The C-ABI shim file of a candidate crate (allowlisted emission path).
pub const FFI_PATH: &str = "src/ffi.rs";
/// The generated differential driver (driver-stage emission path).
pub const DRIVER_PATH: &str = "driver.c";
/// The line that must end a complete reply.
pub const END_SENTINEL: &str = "RUHARNESS_END_OF_OUTPUT";

/// One stage's emission contract: the closed, ordered set of paths a reply
/// may provide and the language its fences are rendered in. The fence
/// language is never REQUIRED of a reply — paths come from labels, info
/// strings or first lines — so a `C`-tagged or untagged fence under a path
/// label is accepted exactly as a `c`-tagged one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec {
    /// Accepted paths, in the order [`ParsedEmission::Files`] returns them.
    pub paths: &'static [&'static str],
    /// Info string of rendered fences and of format messages (`rust`, `c`).
    pub fence_lang: &'static str,
    /// Language named in lint messages (`Rust`, `C`).
    pub language: &'static str,
}

/// The migrate stage: `src/logic.rs` + `src/ffi.rs` in `rust` fences.
pub const MIGRATE_SPEC: FileSpec = FileSpec {
    paths: &[LOGIC_PATH, FFI_PATH],
    fence_lang: "rust",
    language: "Rust",
};

/// The driver stage: exactly `driver.c` in a `c` fence.
pub const DRIVER_SPEC: FileSpec = FileSpec {
    paths: &[DRIVER_PATH],
    fence_lang: "c",
    language: "C",
};

/// The C stdio output functions: a C unit calling any of them prints, and a
/// migrate candidate's `src/ffi.rs` may then declare them — and nothing
/// else — in a foreign block ([`deny_scan_with`]).
pub const STDIO_OUTPUT_FNS: [&str; 9] = [
    "printf", "puts", "putchar", "fputs", "fputc", "putc", "fwrite", "fprintf", "vprintf",
];
/// A reply whose reported output tokens come within this many tokens of the
/// budget is treated as truncated.
const TOKEN_CAP_MARGIN: u32 = 8;
/// How many lines above an opening fence a path label may sit.
const PATH_LOOKBACK_LINES: usize = 2;
/// Longest `<blocked>` reason kept, in chars.
const BLOCKED_REASON_MAX_CHARS: usize = 2000;
/// Most lint hits quoted in one [`EmissionResult::Format`] message.
const MAX_LINT_HITS: usize = 5;
/// Phrases that mark an elided ("the rest is as before") file.
const ELISION_PHRASES: [&str; 4] = ["rest of the", "unchanged", "omitted", "same as before"];
/// HTML entities that mark an HTML-escaped (hence corrupted) file.
const HTML_ENTITIES: [&str; 3] = ["&lt;", "&gt;", "&amp;"];

/// What one model reply amounted to under the emission contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmissionResult {
    /// Both files, complete.
    Files {
        /// Entire content of `src/logic.rs` (`\n` line ends, trailing newline).
        logic: String,
        /// Entire content of `src/ffi.rs` (`\n` line ends, trailing newline).
        ffi: String,
        /// Notes on every leniency applied (path taken from an info string,
        /// duplicate block, missing sentinel, ignored block, …). Empty for a
        /// reply in the canonical layout.
        guesses: Vec<String>,
    },
    /// The model (or the provider) declined: `<blocked>reason</blocked>`
    /// outside any code block, or a refusal stop kind. Carries the reason.
    Blocked(String),
    /// The reply is incomplete; nothing may be written. Carries the signal.
    Truncated(String),
    /// The reply is complete but does not follow the contract. Carries a
    /// message fit to show the model in a repair turn.
    Format(String),
}

/// What one model reply amounted to under a [`FileSpec`]: the spec-generic
/// form of [`EmissionResult`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedEmission {
    /// Every file of the spec, complete, in [`FileSpec::paths`] order (`\n`
    /// line ends, trailing newline).
    Files {
        /// File contents, one per spec path.
        files: Vec<String>,
        /// Notes on every leniency applied (see [`EmissionResult::Files`]).
        guesses: Vec<String>,
    },
    /// See [`EmissionResult::Blocked`].
    Blocked(String),
    /// See [`EmissionResult::Truncated`].
    Truncated(String),
    /// See [`EmissionResult::Format`].
    Format(String),
}

/// Parse one reply under the MIGRATE emission contract ([`MIGRATE_SPEC`]);
/// [`parse_spec`] states the rules.
pub fn parse_emission(
    text: &str,
    stop: StopKind,
    output_tokens: Option<u64>,
    max_tokens: u32,
) -> EmissionResult {
    match parse_spec(text, stop, output_tokens, max_tokens, &MIGRATE_SPEC) {
        ParsedEmission::Files { files, guesses } => match <[String; 2]>::try_from(files) {
            Ok([logic, ffi]) => EmissionResult::Files {
                logic,
                ffi,
                guesses,
            },
            // Unreachable: `parse_spec` returns one file per spec path.
            Err(_) => EmissionResult::Format(format!("{FFI_PATH} is missing")),
        },
        ParsedEmission::Blocked(reason) => EmissionResult::Blocked(reason),
        ParsedEmission::Truncated(signal) => EmissionResult::Truncated(signal),
        ParsedEmission::Format(message) => EmissionResult::Format(message),
    }
}

/// Parse one reply under the emission contract of `spec`.
///
/// `stop` is the normalized stop kind of the response, `output_tokens` the
/// provider-reported output usage when known, `max_tokens` the request's
/// budget. Precedence, first match wins:
///
/// 1. `stop == MaxTokens`, or `output_tokens >= max_tokens - 8` →
///    [`ParsedEmission::Truncated`];
/// 2. `stop == Refusal` → [`ParsedEmission::Blocked`];
/// 3. the text is normalized (CRLF → LF, leading BOM dropped) and leading
///    `<think>…</think>` spans are stripped — an unclosed one is `Truncated`;
/// 4. fenced blocks are scanned: an opening fence is a column-0 line of ≥ 3
///    backticks plus an optional info string, the closing fence a column-0
///    line of exactly the same number of backticks (trailing blanks
///    tolerated). End of text inside a block → `Truncated`;
/// 5. `<blocked>reason</blocked>` outside any block → `Blocked`, judged by
///    CONTENT, not position: the contract's literal placeholder reason
///    `reason` and an echoed instruction (the tag right after `reply with
///    only`, or right before `instead of code`) are never verdicts; a tag
///    that starts its line — markdown decoration such as `- `, `> `, `1. `
///    or backticks aside — always is; a tag after ordinary prose on its
///    line is one only when the reply holds NO allowlisted file block (a
///    reply that delivers files and mentions the tag in passing is not
///    declining);
/// 6. the sentinel [`END_SENTINEL`] (outside any block) is missing: accepted
///    with a note when `stop == EndTurn`, `Truncated` when `stop == Other`;
/// 7. each block's path is, in order: the nearest non-blank line within the
///    2 lines above the fence (leading `#`, `*`, backticks and a `File:` /
///    `Path:` prefix stripped, trailing `:` / `*` / backticks stripped); a
///    path in the info string (`rust src/logic.rs`, `rust:src/logic.rs`,
///    `title="src/logic.rs"`); the block's first content line when it is
///    exactly a path (optionally as a `//` comment), which is then dropped.
///    A leading `./` is normalized away and the result must EXACTLY equal
///    one of the spec's paths — every other block is ignored. The last
///    block for a path wins;
/// 8. a missing or blank file → [`ParsedEmission::Format`] (the first such
///    path in spec order is reported);
/// 9. elision lints → `Format`: a line that is exactly `// ...`, or one
///    containing `rest of the`, `unchanged`, `omitted` or `same as before`
///    (ASCII case-insensitive); likewise the HTML entities `&lt;`, `&gt;`,
///    `&amp;`.
pub fn parse_spec(
    text: &str,
    stop: StopKind,
    output_tokens: Option<u64>,
    max_tokens: u32,
    spec: &FileSpec,
) -> ParsedEmission {
    if stop == StopKind::MaxTokens {
        return ParsedEmission::Truncated(
            "the provider reported that the output hit the token cap".into(),
        );
    }
    let cap = u64::from(max_tokens.saturating_sub(TOKEN_CAP_MARGIN));
    if let Some(used) = output_tokens {
        if used >= cap {
            return ParsedEmission::Truncated(format!(
                "the output used {used} of {max_tokens} tokens (within {TOKEN_CAP_MARGIN} of \
                 the cap)"
            ));
        }
    }
    if stop == StopKind::Refusal {
        return ParsedEmission::Blocked("the provider refused or filtered the request".into());
    }

    let normalized = text.replace("\r\n", "\n");
    let without_bom = normalized.strip_prefix('\u{feff}').unwrap_or(&normalized);
    let body = match strip_leading_think(without_bom) {
        Ok(body) => body,
        Err(signal) => return ParsedEmission::Truncated(signal),
    };

    // Fence scan. `blocks` holds (first line a label may sit on, opening
    // fence line, closing fence line) as indexes into `lines`.
    let lines: Vec<&str> = body.split('\n').collect();
    let mut blocks: Vec<(usize, usize, usize)> = Vec::new();
    let mut outside: Vec<&str> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let mut floor = 0;
    for (index, line) in lines.iter().enumerate() {
        match open {
            None => match fence_open(line) {
                Some(ticks) => open = Some((index, ticks)),
                None => outside.push(line),
            },
            Some((start, ticks)) => {
                if is_fence_close(line, ticks) {
                    blocks.push((floor, start, index));
                    floor = index + 1;
                    open = None;
                }
            }
        }
    }
    if let Some((start, _)) = open {
        return ParsedEmission::Truncated(format!(
            "the output ended inside the code block opened on line {}",
            start + 1
        ));
    }

    // Path resolution: allowlist lookup only. Done before the blocked check
    // (which needs to know whether the reply delivered any file), but its
    // notes are reported after the sentinel note, in reading order.
    let mut path_notes: Vec<String> = Vec::new();
    let mut found: Vec<Option<String>> = vec![None; spec.paths.len()];
    let mut ignored = 0usize;
    for &(floor, start, end) in &blocks {
        let mut content: &[&str] = &lines[start + 1..end];
        let ticks = lines[start].chars().take_while(|c| *c == '`').count();
        let info = lines[start][ticks..].trim();
        let path = if let Some((path, tidy)) = label_path(spec, &lines[floor..start]) {
            if !tidy {
                path_notes.push(format!("{path}: path label needed cleanup"));
            }
            Some(path)
        } else if let Some(path) = info_path(spec, info) {
            path_notes.push(format!("{path}: path taken from the fence info string"));
            Some(path)
        } else if let Some(path) = content.first().and_then(|line| first_line_path(spec, line)) {
            path_notes.push(format!(
                "{path}: path taken from the first line of the block (line dropped)"
            ));
            content = &content[1..];
            Some(path)
        } else {
            None
        };
        let Some(path) = path else {
            ignored += 1;
            path_notes.push(format!(
                "ignored the code block opened on line {}: no allowlisted path",
                start + 1
            ));
            continue;
        };
        let slot = spec
            .paths
            .iter()
            .position(|allowed| *allowed == path)
            .unwrap_or(0); // `path` IS one of `spec.paths`: allowlist lookup
        if found[slot].is_some() {
            path_notes.push(format!("{path}: emitted more than once (last one wins)"));
        }
        let mut file = content.join("\n");
        file.push('\n');
        found[slot] = Some(file);
    }

    let prose = outside.join("\n");
    let delivered_files = found.iter().any(Option::is_some);
    if let Some(reason) = blocked_reason(&prose, delivered_files) {
        return ParsedEmission::Blocked(reason);
    }

    let mut guesses: Vec<String> = Vec::new();
    if !outside.iter().any(|line| line.contains(END_SENTINEL)) {
        match stop {
            StopKind::EndTurn => guesses.push(format!(
                "{END_SENTINEL} is missing (accepted: the turn ended normally)"
            )),
            _ => {
                return ParsedEmission::Truncated(format!(
                    "{END_SENTINEL} is missing and the turn did not end normally"
                ))
            }
        }
    }
    guesses.extend(path_notes);

    let accepted = format!(
        "only {} {} accepted",
        spec.paths.join(" and "),
        if spec.paths.len() == 1 { "is" } else { "are" }
    );
    let mut files: Vec<String> = Vec::with_capacity(found.len());
    for (path, file) in spec.paths.iter().zip(found) {
        match file {
            Some(file) if !file.trim().is_empty() => files.push(file),
            Some(_) => return ParsedEmission::Format(format!("{path} is empty")),
            None => {
                return ParsedEmission::Format(format!(
                    "{path} is missing: expected the path alone on a line, then a column-0 \
                     ```{} fence holding the entire file ({} code block(s) found, {ignored} \
                     without an accepted path; {accepted})",
                    spec.fence_lang,
                    blocks.len()
                ))
            }
        }
    }

    let named: Vec<(&str, &str)> = spec
        .paths
        .iter()
        .copied()
        .zip(files.iter().map(String::as_str))
        .collect();
    let hits = lint_files(&named);
    if !hits.is_empty() {
        let shown: Vec<&str> = hits
            .iter()
            .take(MAX_LINT_HITS)
            .map(String::as_str)
            .collect();
        let more = hits.len().saturating_sub(MAX_LINT_HITS);
        let mut message = format!(
            "the {} not complete, literal {} source — {}",
            if spec.paths.len() == 1 {
                "file is"
            } else {
                "files are"
            },
            spec.language,
            shown.join("; ")
        );
        if more > 0 {
            message.push_str(&format!("; and {more} more"));
        }
        message.push_str(
            ". Emit every file in full: no placeholder comments, no abbreviation, no HTML \
             escaping",
        );
        return ParsedEmission::Format(message);
    }

    ParsedEmission::Files { files, guesses }
}

/// Render two files in the MIGRATE emission layout — [`render_spec`] over
/// [`MIGRATE_SPEC`].
pub fn render_files(logic: &str, ffi: &str) -> String {
    render_spec(&MIGRATE_SPEC, &[logic, ffi])
}

/// Render `files` (one per spec path, in order) in the emission layout
/// (path line, column-0 fence, the entire file, closing fence) — WITHOUT
/// the final [`END_SENTINEL`] line.
///
/// The fence is tagged with the spec's language and at least three
/// backticks long; it grows past the longest column-0 backtick run in any
/// file, so the rendering always parses back to the same files.
pub fn render_spec<S: AsRef<str>>(spec: &FileSpec, files: &[S]) -> String {
    let mut out = String::new();
    for (path, file) in spec.paths.iter().zip(files) {
        let file = file.as_ref();
        let longest_run = file
            .lines()
            .map(|line| line.chars().take_while(|c| *c == '`').count())
            .max()
            .unwrap_or(0);
        let fence = "`".repeat(longest_run.max(2) + 1);
        out.push_str(path);
        out.push('\n');
        out.push_str(&fence);
        out.push_str(spec.fence_lang);
        out.push('\n');
        out.push_str(file);
        if !file.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&fence);
        out.push('\n');
    }
    out
}

/// Textual deny-scan over a parsed candidate. Returns one human-readable
/// violation per (file, rule) hit; empty = clean.
///
/// **QUALITY FEEDBACK ONLY — not a security boundary.** A text scan cannot
/// constrain what compiled code does and is easy to evade; its job is to
/// tell the model, cheaply and in words, that it broke a structure rule,
/// before a build is spent on it. What actually confines model-written code
/// is the compiler (the harness-owned `src/lib.rs` forbids `unsafe` in
/// `logic` and the harness owns `Cargo.toml`), the oracle's symbol-set
/// check, and the sandbox.
///
/// Matching is whitespace-insensitive — ALL whitespace is removed before
/// matching, so `include_str !(` and `# [ path` are caught — and covers:
/// foreign `extern { … }` / `extern "ABI" { … }` blocks (but not
/// `extern "C" fn` definitions), `#[link…`, `#[path`, the macros `include!`
/// `include_str!` `include_bytes!` `env!` `option_env!` `asm!` `global_asm!`
/// `todo!` `unimplemented!` (any delimiter), `transmute`, `std::process`
/// `std::fs` `std::net` `std::env`, `#![feature`, `allow(unsafe_code)`,
/// `export_name`, `link_section`, `#[used`, and the word `unsafe` anywhere
/// in `src/logic.rs` (comments included).
pub fn deny_scan(logic: &str, ffi: &str) -> Vec<String> {
    deny_scan_with(logic, ffi, &[])
}

/// [`deny_scan`], except that `src/ffi.rs` may hold foreign blocks that
/// declare nothing but `fn`s named in `ffi_externs` (optionally `pub`,
/// `safe` or `unsafe`) — the migrate stage passes [`STDIO_OUTPUT_FNS`] for a
/// unit whose C prints, so the candidate can write through the C stdio
/// stream. Every other rule is unchanged; with an empty `ffi_externs` this
/// IS [`deny_scan`].
pub fn deny_scan_with(logic: &str, ffi: &str, ffi_externs: &[&str]) -> Vec<String> {
    /// (needle in the whitespace-free text, what to tell the model).
    const SUBSTRING_RULES: [(&str, &str); 11] = [
        (
            "#[path",
            "`#[path]` attribute — a candidate is exactly its two files",
        ),
        (
            "transmute",
            "`transmute` — use explicit, checked conversions",
        ),
        ("std::process", "`std::process` — no process control"),
        ("std::fs", "`std::fs` — no filesystem access"),
        ("std::net", "`std::net` — no network access"),
        ("std::env", "`std::env` — no environment access"),
        ("#![feature", "`#![feature(…)]` — stable Rust only"),
        (
            "allow(unsafe_code)",
            "`allow(unsafe_code)` — the harness-owned src/lib.rs decides where unsafe is allowed",
        ),
        (
            "export_name",
            "`export_name` — exports come only from `#[no_mangle]` fns named in the ABI contract",
        ),
        ("link_section", "`link_section` attribute"),
        ("#[used", "`#[used]` attribute"),
    ];
    /// (macro name, prefix that makes the hit a different macro).
    const MACRO_RULES: [(&str, &str); 9] = [
        ("include", ""),
        ("include_str", ""),
        ("include_bytes", ""),
        ("env", "option_"),
        ("option_env", ""),
        ("asm", "global_"),
        ("global_asm", ""),
        ("todo", ""),
        ("unimplemented", ""),
    ];

    let mut violations = Vec::new();
    for (path, text) in [(LOGIC_PATH, logic), (FFI_PATH, ffi)] {
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let mut hits: Vec<String> = Vec::new();
        if path == LOGIC_PATH && squeezed.contains("unsafe") {
            hits.push(
                "the word `unsafe` — src/logic.rs must be 100% safe Rust (not even in a comment)"
                    .into(),
            );
        }
        if path == FFI_PATH && !ffi_externs.is_empty() {
            let bodies = foreign_extern_bodies(&squeezed);
            let stray = bodies
                .iter()
                .any(|body| !body.is_some_and(|body| declares_only(body, ffi_externs)));
            if stray {
                hits.push(format!(
                    "foreign `extern {{ … }}` block declaring something other than the C stdio \
                     output functions allowed under [STDOUT] ({}) — only `fn` declarations of \
                     those functions",
                    ffi_externs.join(", ")
                ));
            }
        } else if has_foreign_extern_block(&squeezed) {
            hits.push(
                "foreign `extern { … }` block — declaring or calling C functions is not allowed \
                 (`extern \"C\" fn` definitions are fine)"
                    .into(),
            );
        }
        if squeezed
            .match_indices("#[link")
            .any(|(at, needle)| !squeezed[at + needle.len()..].starts_with("_section"))
        {
            hits.push("`#[link…]` attribute — no linking against anything".into());
        }
        for (needle, what) in SUBSTRING_RULES {
            if squeezed.contains(needle) {
                hits.push(what.into());
            }
        }
        for (name, other_prefix) in MACRO_RULES {
            if has_macro_call(&squeezed, name, other_prefix) {
                hits.push(format!("`{name}!` macro"));
            }
        }
        violations.extend(hits.into_iter().map(|what| format!("{path}: {what}")));
    }
    violations
}

/// Strip leading `<think>…</think>` spans (and the blank space around
/// them). `Err` = an unclosed span, i.e. the reply was cut while thinking.
fn strip_leading_think(text: &str) -> Result<&str, String> {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut body = text;
    while let Some(rest) = body.trim_start().strip_prefix(OPEN) {
        match rest.find(CLOSE) {
            Some(end) => body = &rest[end + CLOSE.len()..],
            None => return Err("the output ended inside an unclosed <think> span".into()),
        }
    }
    Ok(body)
}

/// The backtick count when `line` is an opening fence: column 0, ≥ 3
/// backticks, then an info string free of backticks.
fn fence_open(line: &str) -> Option<usize> {
    let ticks = line.chars().take_while(|c| *c == '`').count();
    (ticks >= 3 && !line[ticks..].contains('`')).then_some(ticks)
}

/// True when `line` closes a fence of `ticks` backticks: exactly that many
/// backticks at column 0, nothing else but trailing blanks.
fn is_fence_close(line: &str, ticks: usize) -> bool {
    let line = line.trim_end();
    line.len() == ticks && line.chars().all(|c| c == '`')
}

/// The reason inside the first `<blocked>…</blocked>` of `prose` (text
/// outside code blocks) that is the model's OWN verdict, trimmed and capped.
///
/// A verdict is judged by content, not by where the tag sits. Weak models
/// parrot their instructions back, including the contract's literal example
/// `reply with only <blocked>reason</blocked> instead of code` (observed
/// live, llama3.2:1b, M3 run #1) — so these are never verdicts:
/// - the placeholder reason `reason`;
/// - a tag that directly follows `reply with only` on its line, or is
///   directly followed by `instead of code` (ASCII case-insensitive;
///   whitespace, backticks and emphasis marks in between are ignored).
///
/// Everything else depends on what precedes the tag on its line:
/// - nothing, or only markdown decoration (`-`, `*`, `>`, `#`, backticks,
///   whitespace, one `1.`-style list number): a verdict — models wrap the
///   tag in lists, quotes and code spans;
/// - ordinary prose (`I cannot do this. <blocked>…`): a verdict only when
///   the reply delivered no allowlisted file block (`delivered_files` is
///   false). A reply that hands over files while mentioning the tag
///   mid-sentence is not declining.
fn blocked_reason(prose: &str, delivered_files: bool) -> Option<String> {
    const OPEN: &str = "<blocked>";
    const CLOSE: &str = "</blocked>";
    /// What the contract's instruction says right before the tag …
    const ECHO_BEFORE: &str = "reply with only";
    /// … and right after it.
    const ECHO_AFTER: &str = "instead of code";
    let is_wrapper = |c: char| c.is_whitespace() || matches!(c, '`' | '*' | '_');

    let mut search_from = 0;
    while let Some(found) = prose[search_from..].find(OPEN) {
        let open_at = search_from + found;
        let after = &prose[open_at + OPEN.len()..];
        search_from = open_at + OPEN.len();
        let close_at = after.find(CLOSE)?;
        let reason = after[..close_at].trim();
        if reason.eq_ignore_ascii_case("reason") {
            continue; // the contract's own placeholder
        }

        let line_start = prose[..open_at].rfind('\n').map_or(0, |i| i + 1);
        let before = &prose[line_start..open_at];
        let follows = &after[close_at + CLOSE.len()..];
        let echoed = before
            .trim_end_matches(is_wrapper)
            .to_ascii_lowercase()
            .ends_with(ECHO_BEFORE)
            || follows
                .trim_start_matches(is_wrapper)
                .to_ascii_lowercase()
                .starts_with(ECHO_AFTER);
        if echoed {
            continue; // the instruction, parroted back
        }
        if !is_line_decoration(before) && delivered_files {
            continue; // mentioned in passing by a reply that delivers files
        }

        if reason.is_empty() {
            return Some("(no reason given)".into());
        }
        return Some(reason.chars().take(BLOCKED_REASON_MAX_CHARS).collect());
    }
    None
}

/// True when `before` — the text preceding a tag on its line — is nothing
/// but markdown decoration: whitespace, `-`, `*`, `>`, `#`, `_`, backticks,
/// and at most one ordered-list number (`12.` or `12)`).
fn is_line_decoration(before: &str) -> bool {
    let rest: String = before
        .chars()
        .filter(|c| !(c.is_whitespace() || matches!(c, '-' | '*' | '>' | '#' | '`' | '_')))
        .collect();
    if rest.is_empty() {
        return true;
    }
    let digits = rest.trim_end_matches(['.', ')']);
    digits.len() + 1 == rest.len()
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
}

/// EXACT lookup in the spec's paths after normalizing leading `./`.
fn allowlisted(spec: &FileSpec, candidate: &str) -> Option<&'static str> {
    let mut path = candidate;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    spec.paths.iter().copied().find(|allowed| *allowed == path)
}

/// Markdown decoration a path label may be wrapped in.
fn is_label_decoration(c: char) -> bool {
    matches!(c, '#' | '*' | '`') || c.is_whitespace()
}

/// The path named by the nearest non-blank line within
/// [`PATH_LOOKBACK_LINES`] above a fence. `above` is the prose between the
/// previous block and the fence. Returns the path and whether the label was
/// already the bare path.
fn label_path(spec: &FileSpec, above: &[&str]) -> Option<(&'static str, bool)> {
    let label = above
        .iter()
        .rev()
        .take(PATH_LOOKBACK_LINES)
        .find(|line| !line.trim().is_empty())?;
    let mut cleaned = label.trim_start_matches(is_label_decoration);
    for prefix in ["file:", "path:"] {
        let is_prefixed = cleaned
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix));
        if is_prefixed {
            cleaned = cleaned[prefix.len()..].trim_start_matches(is_label_decoration);
            break;
        }
    }
    let cleaned = cleaned.trim_end_matches(|c: char| c == ':' || is_label_decoration(c));
    let path = allowlisted(spec, cleaned)?;
    Some((path, label.trim() == path))
}

/// A path in a fence info string: a whole token, the part of a token after
/// `:` (`rust:src/logic.rs`), or an attribute value (`title="src/logic.rs"`).
fn info_path(spec: &FileSpec, info: &str) -> Option<&'static str> {
    let unquote = |s: &str| allowlisted(spec, s.trim_matches(|c| c == '"' || c == '\''));
    info.split_whitespace().find_map(|token| {
        unquote(token)
            .or_else(|| token.split_once(':').and_then(|(_, rest)| unquote(rest)))
            .or_else(|| token.split_once('=').and_then(|(_, rest)| unquote(rest)))
    })
}

/// The path when a block's first content line is exactly one, bare or as a
/// `//` comment.
fn first_line_path(spec: &FileSpec, line: &str) -> Option<&'static str> {
    let line = line.trim();
    allowlisted(spec, line.strip_prefix("//").map_or(line, str::trim_start))
}

/// Elision and HTML-escaping lints over the parsed files.
fn lint_files(files: &[(&str, &str)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (path, file) in files {
        for (index, line) in file.lines().enumerate() {
            let at = format!("{path} line {}", index + 1);
            let squeezed: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if squeezed == "//..." || squeezed == "//…" {
                hits.push(format!("{at} is an elision marker (`// ...`)"));
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(phrase) = ELISION_PHRASES.iter().find(|p| lower.contains(**p)) {
                hits.push(format!("{at} contains the elision phrase \"{phrase}\""));
            } else if let Some(entity) = HTML_ENTITIES.iter().find(|e| line.contains(**e)) {
                hits.push(format!("{at} contains the HTML entity `{entity}`"));
            }
        }
    }
    hits
}

/// True when the whitespace-free `squeezed` text holds `extern`, an optional
/// ABI string, then `{` — a foreign block, not an `extern "C" fn`.
fn has_foreign_extern_block(squeezed: &str) -> bool {
    const KEYWORD: &str = "extern";
    let mut rest = squeezed;
    while let Some(at) = rest.find(KEYWORD) {
        let after = &rest[at + KEYWORD.len()..];
        let after_abi = match after.strip_prefix('"') {
            Some(abi) => abi.find('"').map_or(abi, |end| &abi[end + 1..]),
            None => after,
        };
        if after_abi.starts_with('{') {
            return true;
        }
        rest = after;
    }
    false
}

/// The body of every foreign block in the whitespace-free `squeezed` text
/// (found exactly as [`has_foreign_extern_block`] finds them); `None` for a
/// block that is never closed.
fn foreign_extern_bodies(squeezed: &str) -> Vec<Option<&str>> {
    const KEYWORD: &str = "extern";
    let mut bodies = Vec::new();
    let mut rest = squeezed;
    while let Some(at) = rest.find(KEYWORD) {
        let after = &rest[at + KEYWORD.len()..];
        let after_abi = match after.strip_prefix('"') {
            Some(abi) => abi.find('"').map_or(abi, |end| &abi[end + 1..]),
            None => after,
        };
        if let Some(body) = after_abi.strip_prefix('{') {
            bodies.push(body.find('}').map(|end| &body[..end]));
        }
        rest = after;
    }
    bodies
}

/// True when every item of a whitespace-free foreign-block `body` is a
/// `fn` declaration (optionally `pub`, then `safe` or `unsafe`) of a name
/// in `allowed`.
fn declares_only(body: &str, allowed: &[&str]) -> bool {
    body.split(';').filter(|item| !item.is_empty()).all(|item| {
        let item = item.strip_prefix("pub").unwrap_or(item);
        let item = item
            .strip_prefix("safe")
            .or_else(|| item.strip_prefix("unsafe"))
            .unwrap_or(item);
        item.strip_prefix("fn")
            .and_then(|rest| rest.split_once('('))
            .is_some_and(|(name, _)| allowed.contains(&name))
    })
}

/// True when `squeezed` invokes the macro `name` with any delimiter, not
/// counting hits that are really `<other_prefix><name>!`.
fn has_macro_call(squeezed: &str, name: &str, other_prefix: &str) -> bool {
    let needle = format!("{name}!");
    squeezed.match_indices(&needle).any(|(at, _)| {
        let invoked = squeezed[at + needle.len()..].starts_with(['(', '[', '{']);
        let other = !other_prefix.is_empty() && squeezed[..at].ends_with(other_prefix);
        invoked && !other
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIC: &str = "pub fn add(a: u32, b: u32) -> u32 {\n    a.wrapping_add(b)\n}\n";
    const FFI: &str = "#[no_mangle]\npub unsafe extern \"C\" fn add(a: u32, b: u32) -> u32 {\n    \
                       crate::logic::add(a, b)\n}\n";

    fn canonical() -> String {
        format!("{}{END_SENTINEL}\n", render_files(LOGIC, FFI))
    }

    fn parse(text: &str) -> EmissionResult {
        parse_emission(text, StopKind::EndTurn, None, 8192)
    }

    /// Parse and require `Files`; returns (logic, ffi, guesses).
    fn files(text: &str) -> (String, String, Vec<String>) {
        match parse(text) {
            EmissionResult::Files {
                logic,
                ffi,
                guesses,
            } => (logic, ffi, guesses),
            other => panic!("expected files, got {other:?}"),
        }
    }

    fn format_message(text: &str) -> String {
        match parse(text) {
            EmissionResult::Format(message) => message,
            other => panic!("expected a format failure, got {other:?}"),
        }
    }

    /// A reply with `label` / `info` / `first` variations around the logic
    /// block and a canonical ffi block.
    fn with_logic_block(label: &str, info: &str, first: &str) -> String {
        format!(
            "{label}```{info}\n{first}{LOGIC}```\nsrc/ffi.rs\n```rust\n{FFI}```\n{END_SENTINEL}\n"
        )
    }

    #[test]
    fn canonical_layout_parses_without_guesses() {
        let text = canonical();
        assert!(
            text.starts_with("src/logic.rs\n```rust\npub fn add"),
            "{text}"
        );
        let (logic, ffi, guesses) = files(&text);
        assert_eq!(logic, LOGIC);
        assert_eq!(ffi, FFI);
        assert!(guesses.is_empty(), "{guesses:?}");
    }

    #[test]
    fn crlf_and_bom_are_normalized() {
        let text = format!("\u{feff}{}", canonical().replace('\n', "\r\n"));
        let (logic, ffi, guesses) = files(&text);
        assert_eq!(logic, LOGIC);
        assert_eq!(ffi, FFI);
        assert!(guesses.is_empty(), "{guesses:?}");
    }

    #[test]
    fn leading_think_spans_are_stripped() {
        let text = format!(
            "\n<think>plan: ```rust\nnot a block\n<blocked>musing</blocked></think>\n\
             <think>second</think>\n{}",
            canonical()
        );
        let (logic, _, guesses) = files(&text);
        assert_eq!(logic, LOGIC);
        assert!(guesses.is_empty(), "{guesses:?}");
    }

    #[test]
    fn unclosed_think_is_truncated() {
        let result = parse(&format!("<think>still thinking\n{}", canonical()));
        assert!(
            matches!(&result, EmissionResult::Truncated(m) if m.contains("<think>")),
            "{result:?}"
        );
    }

    #[test]
    fn blocked_outside_a_fence_wins_even_next_to_files() {
        assert_eq!(
            parse("<blocked> uses setjmp;\ncannot be safe </blocked>\n"),
            EmissionResult::Blocked("uses setjmp;\ncannot be safe".into())
        );
        let beside = format!("{}<blocked>second thoughts</blocked>\n", canonical());
        assert_eq!(
            parse(&beside),
            EmissionResult::Blocked("second thoughts".into())
        );
        assert_eq!(
            parse("<blocked></blocked>"),
            EmissionResult::Blocked("(no reason given)".into())
        );
    }

    /// Regression (M3 live run #1, llama3.2:1b): the model parroted the
    /// system prompt back, including the contract's own example sentence.
    /// An echoed instruction is not a verdict.
    #[test]
    fn parroted_blocked_placeholder_is_not_a_verdict() {
        let echoed = "[UNIT]\nid: u001\n\nIf the unit cannot be translated under these rules, \
                      reply with only <blocked>reason</blocked> instead of code.\n";
        assert!(
            !matches!(parse(echoed), EmissionResult::Blocked(_)),
            "mid-sentence echo must not block"
        );
        // Even at line start, the literal placeholder is never a verdict.
        assert!(!matches!(
            parse("<blocked>reason</blocked>\n"),
            EmissionResult::Blocked(_)
        ));
        // A genuine verdict after an echo still counts.
        let genuine = format!("{echoed}<blocked>needs setjmp</blocked>\n");
        assert_eq!(
            parse(&genuine),
            EmissionResult::Blocked("needs setjmp".into())
        );
    }

    /// Regression (M3 review): detection used to be positional — the tag had
    /// to start its line — so every decorated or prose-led verdict below was
    /// misread as a format failure and sent into pointless repair turns.
    #[test]
    fn a_decorated_blocked_tag_is_a_verdict() {
        for (text, reason) in [
            ("- <blocked>why</blocked>", "why"),
            ("`<blocked>why</blocked>`", "why"),
            ("> <blocked>needs longjmp</blocked>\n", "needs longjmp"),
            ("* **<blocked>why</blocked>**", "why"),
            ("## <blocked>why</blocked>", "why"),
            ("1. <blocked>why</blocked>", "why"),
            ("  12) `<blocked>why</blocked>`", "why"),
            ("Sorry.\n\n> - <blocked>why</blocked>\n", "why"),
        ] {
            assert_eq!(
                parse(text),
                EmissionResult::Blocked(reason.into()),
                "{text:?}"
            );
        }
        // Decoration does not change the verdict next to delivered files.
        let beside = format!("{}- `<blocked>gave up after all</blocked>`\n", canonical());
        assert_eq!(
            parse(&beside),
            EmissionResult::Blocked("gave up after all".into())
        );
    }

    #[test]
    fn a_blocked_tag_after_prose_is_a_verdict_only_without_files() {
        assert_eq!(
            parse("I cannot do this. <blocked>needs setjmp</blocked>"),
            EmissionResult::Blocked("needs setjmp".into())
        );
        assert_eq!(
            parse("Unit 1.2 is odd: <blocked>needs setjmp</blocked>\n"),
            EmissionResult::Blocked("needs setjmp".into())
        );
        // The same sentence next to BOTH files — or even one — is a remark,
        // not a refusal: the reply delivers code.
        let remark = "Note: I did not have to answer <blocked>needs setjmp</blocked> here.\n";
        let (logic, ffi, _) = files(&format!("{remark}{}", canonical()));
        assert_eq!((logic.as_str(), ffi.as_str()), (LOGIC, FFI));
        let one_file = format!("{remark}src/logic.rs\n```rust\n{LOGIC}```\n{END_SENTINEL}\n");
        let message = format_message(&one_file);
        assert!(message.contains("src/ffi.rs is missing"), "{message}");
        // A block without an allowlisted path is not a delivered file.
        let stray = format!("{remark}```rust\n{LOGIC}```\n");
        assert_eq!(
            parse(&stray),
            EmissionResult::Blocked("needs setjmp".into())
        );
    }

    #[test]
    fn an_echoed_instruction_is_never_a_verdict_whatever_its_reason() {
        for echoed in [
            "reply with only <blocked>too hard</blocked>",
            "If you cannot, Reply With Only `<blocked>too hard</blocked>`.",
            "- reply with only **<blocked>too hard</blocked>**",
            "<blocked>too hard</blocked> instead of code",
            "`<blocked>too hard</blocked>` Instead Of Code.",
            "- <blocked>REASON</blocked>",
            "I would say <blocked> reason </blocked> but here goes.",
        ] {
            assert!(
                !matches!(parse(echoed), EmissionResult::Blocked(_)),
                "{echoed:?}"
            );
        }
        // An echo does not hide a genuine verdict later in the reply.
        assert_eq!(
            parse("reply with only <blocked>x</blocked>\nSo: <blocked>needs setjmp</blocked>"),
            EmissionResult::Blocked("needs setjmp".into())
        );
    }

    #[test]
    fn line_decoration_is_a_closed_set() {
        for decoration in [
            "", "  ", "- ", "* ", "> > ", "### ", "`", "**`", "1. ", "12) ", "_",
        ] {
            assert!(is_line_decoration(decoration), "{decoration:?}");
        }
        for prose in ["a", "1", "1.2. ", "1.) ", "- no: ", "x. ", "é"] {
            assert!(!is_line_decoration(prose), "{prose:?}");
        }
    }

    #[test]
    fn blocked_inside_a_fence_is_file_content() {
        let logic = format!("// <blocked>not a verdict</blocked>\n{LOGIC}");
        let text = format!("{}{END_SENTINEL}\n", render_files(&logic, FFI));
        assert_eq!(files(&text).0, logic);
    }

    #[test]
    fn unclosed_blocked_tag_is_not_a_verdict() {
        let message = format_message("<blocked>never closed\n");
        assert!(message.contains("src/logic.rs is missing"), "{message}");
    }

    #[test]
    fn blocked_reason_is_capped() {
        let text = format!("<blocked>{}</blocked>", "x".repeat(5000));
        match parse(&text) {
            EmissionResult::Blocked(reason) => assert_eq!(reason.len(), BLOCKED_REASON_MAX_CHARS),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refusal_stop_is_blocked() {
        let result = parse_emission(&canonical(), StopKind::Refusal, None, 8192);
        assert!(
            matches!(&result, EmissionResult::Blocked(m) if m.contains("refused")),
            "{result:?}"
        );
    }

    #[test]
    fn longer_fences_carry_backtick_lines() {
        let logic = format!("/// ```\n/// add(1, 2);\n/// ```\n{LOGIC}```\nlet x = 1;\n```\n");
        let rendered = render_files(&logic, FFI);
        assert!(
            rendered.starts_with("src/logic.rs\n````rust\n"),
            "{rendered}"
        );
        let (parsed, ffi, guesses) = files(&format!("{rendered}{END_SENTINEL}\n"));
        assert_eq!(parsed, logic);
        assert_eq!(ffi, FFI);
        assert!(guesses.is_empty(), "{guesses:?}");
    }

    #[test]
    fn closing_fence_must_match_the_count_exactly() {
        // A 4-backtick line inside a 3-backtick fence is content; trailing
        // blanks on the real closing fence are tolerated.
        let text = format!(
            "src/logic.rs\n```rust\n{LOGIC}````\n// still logic\n```  \nsrc/ffi.rs\n```rust\n\
             {FFI}```\n{END_SENTINEL}\n"
        );
        let (logic, _, _) = files(&text);
        assert_eq!(logic, format!("{LOGIC}````\n// still logic\n"));
    }

    #[test]
    fn indented_fences_are_not_fences() {
        let text = canonical().replace("```", "  ```");
        let message = format_message(&text);
        assert!(message.contains("0 code block(s) found"), "{message}");
    }

    #[test]
    fn inline_triple_backticks_do_not_open_a_block() {
        let text = format!("```not a fence``` just prose\n{}", canonical());
        assert_eq!(files(&text).0, LOGIC);
    }

    #[test]
    fn decorated_path_labels_are_accepted_with_a_note() {
        for label in [
            "### src/logic.rs\n",
            "**src/logic.rs**\n",
            "`src/logic.rs`:\n",
            "File: src/logic.rs\n",
            "**File:** `./src/logic.rs`\n",
            "path: ././src/logic.rs:\n",
            "  src/logic.rs  \n\n",
        ] {
            let (logic, _, guesses) = files(&with_logic_block(label, "rust", ""));
            assert_eq!(logic, LOGIC, "label {label:?}");
            assert!(
                guesses.iter().all(|g| g.contains("cleanup")) && guesses.len() <= 1,
                "label {label:?}: {guesses:?}"
            );
        }
        // A bare label two lines up (one blank between) is canonical enough.
        let (_, _, guesses) = files(&with_logic_block("src/logic.rs\n\n", "rust", ""));
        assert!(guesses.is_empty(), "{guesses:?}");
    }

    #[test]
    fn a_label_more_than_two_lines_up_is_not_seen() {
        let message = format_message(&with_logic_block("src/logic.rs\n\n\n", "rust", ""));
        assert!(message.contains("src/logic.rs is missing"), "{message}");
        // Only the NEAREST non-blank line counts, even when a path sits
        // just above it.
        let message = format_message(&with_logic_block("src/logic.rs\nHere it is:\n", "rust", ""));
        assert!(message.contains("src/logic.rs is missing"), "{message}");
    }

    #[test]
    fn a_label_is_never_read_out_of_the_previous_block() {
        // The ffi block has no label; the line above its fence is the logic
        // block's closing fence, and the line above that is logic CONTENT
        // that happens to be a path.
        let text = format!(
            "src/logic.rs\n```rust\n{LOGIC}// see\nsrc/ffi.rs\n```\n```rust\n{FFI}```\n\
             {END_SENTINEL}\n"
        );
        let message = format_message(&text);
        assert!(message.contains("src/ffi.rs is missing"), "{message}");
    }

    #[test]
    fn info_string_paths_are_accepted_with_a_note() {
        for info in [
            "rust src/logic.rs",
            "rust:src/logic.rs",
            "rust title=\"src/logic.rs\"",
            "rust title='./src/logic.rs'",
            "src/logic.rs",
        ] {
            let (logic, _, guesses) = files(&with_logic_block("", info, ""));
            assert_eq!(logic, LOGIC, "info {info:?}");
            assert!(
                guesses.iter().any(|g| g.contains("info string")),
                "info {info:?}: {guesses:?}"
            );
        }
    }

    #[test]
    fn first_content_line_path_is_accepted_and_dropped() {
        for first in ["src/logic.rs\n", "// src/logic.rs\n", "//./src/logic.rs\n"] {
            let (logic, _, guesses) = files(&with_logic_block("", "rust", first));
            assert_eq!(logic, LOGIC, "first line {first:?}");
            assert!(
                guesses.iter().any(|g| g.contains("first line")),
                "first line {first:?}: {guesses:?}"
            );
        }
    }

    #[test]
    fn label_beats_info_string_beats_first_line() {
        // Label says logic; the info string and first line say ffi.
        let text = format!(
            "src/logic.rs\n```rust src/ffi.rs\n{LOGIC}```\nsrc/ffi.rs\n```rust\n{FFI}```\n\
             {END_SENTINEL}\n"
        );
        let (logic, ffi, _) = files(&text);
        assert_eq!((logic.as_str(), ffi.as_str()), (LOGIC, FFI));
    }

    #[test]
    fn only_exact_allowlisted_paths_are_accepted() {
        for label in [
            "src/main.rs",
            "../src/logic.rs",
            "/src/logic.rs",
            "src/logic.rs.bak",
            "src//logic.rs",
            "SRC/LOGIC.RS",
            "src/../src/logic.rs",
            "Cargo.toml",
            "1. src/logic.rs",
        ] {
            let message = format_message(&with_logic_block(&format!("{label}\n"), "rust", ""));
            assert!(
                message.contains("src/logic.rs is missing") && message.contains("1 without"),
                "label {label:?}: {message}"
            );
        }
    }

    #[test]
    fn extra_blocks_are_ignored_with_a_note() {
        let text = format!(
            "Cargo.toml\n```toml\n[package]\n```\n{}build.rs\n```rust\nfn main() {{}}\n```\n",
            canonical()
        );
        let (logic, ffi, guesses) = files(&text);
        assert_eq!((logic.as_str(), ffi.as_str()), (LOGIC, FFI));
        assert_eq!(
            guesses.iter().filter(|g| g.contains("ignored")).count(),
            2,
            "{guesses:?}"
        );
    }

    #[test]
    fn duplicate_paths_last_wins_with_a_note() {
        let text = format!(
            "src/logic.rs\n```rust\npub fn old() {{}}\n```\n{}",
            canonical()
        );
        let (logic, _, guesses) = files(&text);
        assert_eq!(logic, LOGIC);
        assert!(
            guesses
                .iter()
                .any(|g| g.contains("src/logic.rs") && g.contains("more than once")),
            "{guesses:?}"
        );
    }

    #[test]
    fn max_tokens_stop_is_truncated_even_for_a_complete_reply() {
        let result = parse_emission(&canonical(), StopKind::MaxTokens, None, 8192);
        assert!(matches!(result, EmissionResult::Truncated(_)), "{result:?}");
    }

    #[test]
    fn output_tokens_near_the_cap_are_truncated() {
        let at =
            |used: u64, max: u32| parse_emission(&canonical(), StopKind::EndTurn, Some(used), max);
        assert!(matches!(at(8184, 8192), EmissionResult::Truncated(_)));
        assert!(matches!(at(9000, 8192), EmissionResult::Truncated(_)));
        assert!(matches!(at(8183, 8192), EmissionResult::Files { .. }));
        // A budget below the margin never underflows.
        assert!(matches!(at(1, 4), EmissionResult::Truncated(_)));
    }

    #[test]
    fn end_of_text_inside_a_block_is_truncated() {
        let text = format!("src/logic.rs\n```rust\n{LOGIC}```\nsrc/ffi.rs\n```rust\n{FFI}");
        let result = parse(&text);
        assert!(
            matches!(&result, EmissionResult::Truncated(m) if m.contains("line 8")),
            "{result:?}"
        );
        // …and it outranks a <blocked> seen earlier: the reply is cut.
        let result = parse(&format!("<blocked>x</blocked>\n{text}"));
        assert!(matches!(result, EmissionResult::Truncated(_)), "{result:?}");
    }

    #[test]
    fn missing_or_empty_files_are_format_failures() {
        let only_logic = format!("src/logic.rs\n```rust\n{LOGIC}```\n{END_SENTINEL}\n");
        assert!(format_message(&only_logic).contains("src/ffi.rs is missing"));
        assert!(format_message("I cannot help with that.").contains("0 code block(s)"));
        assert!(format_message("").contains("src/logic.rs is missing"));
        let empty = format!(
            "src/logic.rs\n```rust\n\n  \n```\nsrc/ffi.rs\n```rust\n{FFI}```\n{END_SENTINEL}\n"
        );
        assert_eq!(format_message(&empty), "src/logic.rs is empty");
    }

    #[test]
    fn missing_sentinel_depends_on_the_stop_kind() {
        let text = render_files(LOGIC, FFI);
        let (_, _, guesses) = files(&text);
        assert!(
            guesses.iter().any(|g| g.contains(END_SENTINEL)),
            "{guesses:?}"
        );
        let result = parse_emission(&text, StopKind::Other, None, 8192);
        assert!(
            matches!(&result, EmissionResult::Truncated(m) if m.contains(END_SENTINEL)),
            "{result:?}"
        );
        // With the sentinel, an unknown stop kind is fine.
        let result = parse_emission(&canonical(), StopKind::Other, None, 8192);
        assert!(matches!(result, EmissionResult::Files { .. }), "{result:?}");
    }

    #[test]
    fn a_sentinel_inside_a_block_does_not_count() {
        let logic = format!("{LOGIC}// {END_SENTINEL}\n");
        let result = parse_emission(&render_files(&logic, FFI), StopKind::Other, None, 8192);
        assert!(matches!(result, EmissionResult::Truncated(_)), "{result:?}");
    }

    #[test]
    fn elision_markers_are_format_failures() {
        for (line, expect) in [
            ("    // ...", "elision marker"),
            ("//...", "elision marker"),
            ("// …", "elision marker"),
            ("// Rest of the function as above", "rest of the"),
            ("// (helpers UNCHANGED)", "unchanged"),
            ("/* body omitted for brevity */", "omitted"),
            ("// same as before", "same as before"),
        ] {
            let logic = format!("{LOGIC}{line}\n");
            let text = format!("{}{END_SENTINEL}\n", render_files(&logic, FFI));
            let message = format_message(&text);
            assert!(
                message.contains(expect) && message.contains("src/logic.rs line 4"),
                "line {line:?}: {message}"
            );
        }
        // An ordinary comment with dots is not an elision marker.
        let logic = format!("{LOGIC}// wraps... deliberately\n");
        let text = format!("{}{END_SENTINEL}\n", render_files(&logic, FFI));
        assert!(matches!(parse(&text), EmissionResult::Files { .. }));
    }

    #[test]
    fn html_entities_are_format_failures() {
        for entity in ["&lt;", "&gt;", "&amp;"] {
            let ffi = format!("{FFI}fn g(v: Vec{entity}u8>) {{}}\n");
            let text = format!("{}{END_SENTINEL}\n", render_files(LOGIC, &ffi));
            let message = format_message(&text);
            assert!(
                message.contains(entity) && message.contains("src/ffi.rs line 5"),
                "{entity}: {message}"
            );
        }
    }

    #[test]
    fn lint_messages_are_bounded() {
        let logic = format!("{LOGIC}{}", "// ...\n".repeat(40));
        let text = format!("{}{END_SENTINEL}\n", render_files(&logic, FFI));
        let message = format_message(&text);
        assert!(message.contains("and 35 more"), "{message}");
        assert!(message.len() < 800, "{}", message.len());
    }

    #[test]
    fn render_files_terminates_unterminated_files() {
        let rendered = render_files("pub fn a() {}", "pub fn b() {}");
        let (logic, ffi, _) = files(&format!("{rendered}{END_SENTINEL}\n"));
        assert_eq!(logic, "pub fn a() {}\n");
        assert_eq!(ffi, "pub fn b() {}\n");
    }

    #[test]
    fn deny_scan_passes_a_clean_candidate() {
        assert_eq!(deny_scan(LOGIC, FFI), Vec::<String>::new());
        // `unsafe`, raw pointers and `extern "C" fn` items and types are the
        // whole point of ffi.rs.
        let ffi = "use core::ffi::c_int;\ntype Callback = extern \"C\" fn(c_int) -> c_int;\n\
                   #[no_mangle]\npub unsafe extern \"C\" fn run(p: *const u8, n: usize) -> c_int {\n    \
                   let s = unsafe { core::slice::from_raw_parts(p, n) };\n    \
                   crate::logic::run(s)\n}\n";
        assert_eq!(deny_scan(LOGIC, ffi), Vec::<String>::new());
    }

    #[test]
    fn deny_scan_flags_every_rule() {
        for (snippet, expect) in [
            (
                "extern \"C\" { fn puts(s: *const u8) -> i32; }",
                "foreign `extern",
            ),
            ("extern { fn abort() -> !; }", "foreign `extern"),
            ("unsafe extern \"system\" { fn f(); }", "foreign `extern"),
            ("#[link(name = \"c\")]", "`#[link…]`"),
            ("#[link_name = \"puts\"]", "`#[link…]`"),
            ("#[path = \"../../x.rs\"] mod x;", "`#[path]`"),
            ("include!(\"x.rs\");", "`include!`"),
            (
                "const S: &str = include_str!(\"/etc/passwd\");",
                "`include_str!`",
            ),
            (
                "const B: &[u8] = include_bytes!(\"k\");",
                "`include_bytes!`",
            ),
            ("const H: &str = env!(\"HOME\");", "`env!`"),
            (
                "const H: Option<&str> = option_env!(\"HOME\");",
                "`option_env!`",
            ),
            ("core::arch::asm!(\"nop\");", "`asm!`"),
            ("core::arch::global_asm!(\".globl x\");", "`global_asm!`"),
            ("let y: u32 = core::mem::transmute(x);", "`transmute`"),
            ("std::process::exit(0);", "`std::process`"),
            ("let _ = std::fs::read(\"x\");", "`std::fs`"),
            ("use std::net::TcpStream;", "`std::net`"),
            ("let _ = ::std::env::var(\"X\");", "`std::env`"),
            ("todo!()", "`todo!`"),
            ("unimplemented!(\"later\")", "`unimplemented!`"),
            ("#![feature(asm)]", "`#![feature(…)]`"),
            ("#![allow(unsafe_code)]", "`allow(unsafe_code)`"),
            ("#[export_name = \"printf\"]", "`export_name`"),
            ("#[unsafe(export_name = \"printf\")]", "`export_name`"),
            ("#[link_section = \".init_array\"]", "`link_section`"),
            ("#[used]", "`#[used]`"),
        ] {
            let ffi = format!("{FFI}{snippet}\n");
            let violations = deny_scan(LOGIC, &ffi);
            assert!(
                violations
                    .iter()
                    .any(|v| v.starts_with("src/ffi.rs: ") && v.contains(expect)),
                "snippet {snippet:?}: {violations:?}"
            );
            // The same rules hold in logic.rs.
            let logic = format!("{LOGIC}{snippet}\n");
            assert!(
                deny_scan(&logic, FFI)
                    .iter()
                    .any(|v| v.starts_with("src/logic.rs: ") && v.contains(expect)),
                "snippet {snippet:?} in logic"
            );
        }
    }

    #[test]
    fn deny_scan_is_whitespace_insensitive() {
        for (snippet, expect) in [
            ("include_str !\n(\"x\")", "`include_str!`"),
            ("# [ path = \"x.rs\" ]", "`#[path]`"),
            ("extern\n\"C\"\n{ fn f(); }", "foreign `extern"),
            ("std :: fs :: read(\"x\")", "`std::fs`"),
            ("# ! [ feature ( x ) ]", "`#![feature(…)]`"),
            ("#[allow( unsafe_code )]", "`allow(unsafe_code)`"),
            ("env ! [\"HOME\"]", "`env!`"),
            ("todo! {}", "`todo!`"),
        ] {
            let violations = deny_scan(LOGIC, &format!("{FFI}{snippet}\n"));
            assert!(
                violations.iter().any(|v| v.contains(expect)),
                "snippet {snippet:?}: {violations:?}"
            );
        }
    }

    #[test]
    fn deny_scan_reports_each_construct_once() {
        let only = |snippet: &str| deny_scan(LOGIC, &format!("{FFI}{snippet}\n"));
        assert_eq!(
            only("option_env!(\"X\")").len(),
            1,
            "{:?}",
            only("option_env!(\"X\")")
        );
        assert_eq!(only("global_asm!(\"\")").len(), 1);
        assert_eq!(only("#[link_section = \"x\"]").len(), 1);
        // …but does not let one construct hide another.
        assert_eq!(only("option_env!(\"X\"); env!(\"Y\")").len(), 2);
        assert_eq!(
            only("#[link_section = \"x\"] #[link(name = \"c\")]").len(),
            2
        );
    }

    #[test]
    fn deny_scan_flags_unsafe_anywhere_in_logic_only() {
        let logic = format!("{LOGIC}// this is not unsafe at all\n");
        let violations = deny_scan(&logic, FFI);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(
            violations[0].starts_with("src/logic.rs: the word `unsafe`"),
            "{violations:?}"
        );
        let logic = format!("{LOGIC}pub fn f(p: *const u8) -> u8 {{ un\tsafe {{ *p }} }}\n");
        assert_eq!(deny_scan(&logic, FFI).len(), 1);
    }

    const DRIVER: &str = "#include <stdio.h>\n#include \"unit.h\"\n\
                          int main(void) {\n    printf(\"%d\\n\", add(1, 2));\n    return 0;\n}\n";

    fn parse_driver(text: &str) -> ParsedEmission {
        parse_spec(text, StopKind::EndTurn, None, 8192, &DRIVER_SPEC)
    }

    fn driver_of(text: &str) -> (String, Vec<String>) {
        match parse_driver(text) {
            ParsedEmission::Files { mut files, guesses } => {
                assert_eq!(files.len(), 1);
                (files.remove(0), guesses)
            }
            other => panic!("expected the driver, got {other:?}"),
        }
    }

    #[test]
    fn the_migrate_spec_through_parse_spec_is_parse_emission() {
        let text = canonical();
        assert_eq!(
            parse_spec(&text, StopKind::EndTurn, None, 8192, &MIGRATE_SPEC),
            ParsedEmission::Files {
                files: vec![LOGIC.to_string(), FFI.to_string()],
                guesses: vec![],
            }
        );
        assert_eq!(
            render_spec(&MIGRATE_SPEC, &[LOGIC, FFI]),
            render_files(LOGIC, FFI)
        );
    }

    #[test]
    fn the_driver_spec_accepts_exactly_driver_c() {
        let rendered = render_spec(&DRIVER_SPEC, &[DRIVER]);
        assert!(
            rendered.starts_with("driver.c\n```c\n#include"),
            "{rendered}"
        );
        let (driver, guesses) = driver_of(&format!("{rendered}{END_SENTINEL}\n"));
        assert_eq!(driver, DRIVER);
        assert!(guesses.is_empty(), "{guesses:?}");

        // `C`-tagged, untagged, and info-string/first-line paths are
        // tolerated exactly as the rust spec tolerates them.
        for text in [
            format!("driver.c\n```C\n{DRIVER}```\n{END_SENTINEL}\n"),
            format!("driver.c\n```\n{DRIVER}```\n{END_SENTINEL}\n"),
            format!("**driver.c**\n```c\n{DRIVER}```\n{END_SENTINEL}\n"),
            format!("```c driver.c\n{DRIVER}```\n{END_SENTINEL}\n"),
            format!("```c\n// driver.c\n{DRIVER}```\n{END_SENTINEL}\n"),
            format!("```c\n./driver.c\n{DRIVER}```\n{END_SENTINEL}\n"),
        ] {
            assert_eq!(driver_of(&text).0, DRIVER, "{text}");
        }

        // The migrate paths are not driver paths, and vice versa.
        let message = match parse_driver(&canonical()) {
            ParsedEmission::Format(message) => message,
            other => panic!("{other:?}"),
        };
        assert!(
            message.starts_with(
                "driver.c is missing: expected the path alone on a line, then a column-0 ```c \
                 fence"
            ),
            "{message}"
        );
        assert!(message.ends_with("only driver.c is accepted)"), "{message}");
        let driver_reply = format!("{rendered}{END_SENTINEL}\n");
        assert!(format_message(&driver_reply).contains("src/logic.rs is missing"));
        for label in ["src/driver.c", "driver.h", "../driver.c", "DRIVER.C"] {
            let text = format!("{label}\n```c\n{DRIVER}```\n{END_SENTINEL}\n");
            assert!(
                matches!(parse_driver(&text), ParsedEmission::Format(_)),
                "{label}"
            );
        }
    }

    #[test]
    fn the_driver_spec_shares_blocked_truncated_think_and_lints() {
        let canonical_driver = format!("{}{END_SENTINEL}\n", render_spec(&DRIVER_SPEC, &[DRIVER]));
        assert_eq!(
            parse_driver("<blocked>needs a callback</blocked>"),
            ParsedEmission::Blocked("needs a callback".into())
        );
        assert!(matches!(
            parse_spec(
                &canonical_driver,
                StopKind::MaxTokens,
                None,
                8192,
                &DRIVER_SPEC
            ),
            ParsedEmission::Truncated(_)
        ));
        assert!(matches!(
            parse_driver(&format!("driver.c\n```c\n{DRIVER}")),
            ParsedEmission::Truncated(_)
        ));
        let thinking = format!("<think>plan</think>\n{canonical_driver}");
        assert_eq!(driver_of(&thinking).0, DRIVER);
        assert!(matches!(
            parse_driver(&format!("<think>still\n{canonical_driver}")),
            ParsedEmission::Truncated(_)
        ));
        let elided = format!(
            "{}{END_SENTINEL}\n",
            render_spec(&DRIVER_SPEC, &[format!("{DRIVER}// ...\n")])
        );
        match parse_driver(&elided) {
            ParsedEmission::Format(message) => {
                assert!(
                    message.starts_with(
                        "the file is not complete, literal C source — driver.c line 7"
                    ),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stdio_externs_are_allowed_in_ffi_only_when_passed() {
        let block = "extern \"C\" {\n    fn putchar(c: i32) -> i32;\n    pub fn printf(f: *const u8, ...) -> i32;\n}\n";
        let ffi = format!("{FFI}{block}");
        assert!(deny_scan(LOGIC, &ffi)
            .iter()
            .any(|v| v.contains("foreign `extern")));
        assert_eq!(
            deny_scan_with(LOGIC, &ffi, &STDIO_OUTPUT_FNS),
            Vec::<String>::new()
        );
        let unsafe_block = "unsafe extern \"C\" { pub safe fn putchar(c: i32) -> i32; }\n";
        assert_eq!(
            deny_scan_with(LOGIC, &format!("{FFI}{unsafe_block}"), &STDIO_OUTPUT_FNS),
            Vec::<String>::new()
        );
        // Anything else in the block — another function, a static — and any
        // foreign block in logic.rs stays a violation.
        for stray in [
            "extern \"C\" { fn system(c: *const u8) -> i32; }\n",
            "extern \"C\" { fn putchar(c: i32) -> i32; static stdout: *mut u8; }\n",
            "extern \"C\" { fn putchar(c: i32) -> i32;\n",
        ] {
            let violations = deny_scan_with(LOGIC, &format!("{FFI}{stray}"), &STDIO_OUTPUT_FNS);
            assert!(
                violations
                    .iter()
                    .any(|v| v.starts_with("src/ffi.rs: foreign `extern { … }` block declaring")),
                "{stray}: {violations:?}"
            );
        }
        let violations = deny_scan_with(&format!("{LOGIC}{block}"), FFI, &STDIO_OUTPUT_FNS);
        assert!(
            violations
                .iter()
                .any(|v| v.starts_with("src/logic.rs: foreign `extern")),
            "{violations:?}"
        );
    }

    #[test]
    fn extern_fn_definitions_and_extern_crate_are_not_foreign_blocks() {
        assert!(!has_foreign_extern_block("pubunsafeextern\"C\"fnf(){}"));
        assert!(!has_foreign_extern_block("pubexternfnf(){}"));
        assert!(!has_foreign_extern_block("externcratecore;"));
        assert!(!has_foreign_extern_block("typeF=extern\"C\"fn(i32)->i32;"));
        assert!(has_foreign_extern_block(
            "extern\"C\"fnf(){}extern\"C\"{fng();}"
        ));
        assert!(!has_foreign_extern_block("extern\"C"));
    }
}
