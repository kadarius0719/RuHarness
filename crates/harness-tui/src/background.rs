//! The terminal's background, asked for once before the first frame
//! (docs/COCKPIT-WRAPPER-DESIGN.md §9.7): the cockpit writes the standard
//! query `ESC ] 11 ; ? BEL` to the tty and reads the answer,
//! `ESC ] 11 ; rgb:RRRR/GGGG/BBBB` ended by `BEL` or `ESC \`, within 150 ms.
//! A terminal that does not answer simply stays silent, and the theme falls
//! back to `COLORFGBG`, then to the terminal theme ([`crate::theme::Theme::choose`]).
//!
//! How the tty is read with a timeout, with no crate added and no unsafe
//! code: `/dev/tty` is opened a second time, by itself, with the
//! non-blocking flag (`OpenOptionsExt::custom_flags`), so a read with
//! nothing waiting returns at once ("would block") and the loop sleeps 2 ms
//! between tries until the reply ends or the time is up. The flag belongs
//! to this one opening only: the descriptor crossterm reads keys from later
//! stays blocking. No reader thread is left behind (a blocked thread would
//! steal the person's first key), and nothing reaches crossterm's event
//! reader, which has not started yet. A reply that has begun when the time
//! is up is read to its end (bounded) and dropped, so none of its bytes
//! become keys.
//!
//! The parser ([`parse`]) and the rule ([`is_dark`]) are pure; [`ask`] runs
//! the exchange over any reader and writer, so the tests drive it without a
//! terminal.

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

/// What the background is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// Relative luminance under 0.5.
    Dark,
    /// Relative luminance 0.5 and over.
    Light,
}

/// The query: OSC 11 with `?`, ended by BEL.
pub const QUERY: &[u8] = b"\x1b]11;?\x07";

/// How long the answer is waited for.
pub const TIMEOUT: Duration = Duration::from_millis(150);

/// How long a reply that has begun is still read, to drop it whole.
pub const DRAIN: Duration = Duration::from_millis(100);

/// The reply's start.
const START: &[u8] = b"\x1b]11;";

/// What the bytes read so far say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reply {
    /// A whole reply, its colour as red, green and blue from 0 to 1.
    Colour([f64; 3]),
    /// A reply has begun and has not ended yet.
    Partial,
    /// A whole reply whose colour cannot be read.
    Unreadable,
    /// No reply in these bytes.
    Nothing,
}

/// The reply in `bytes`, wherever it starts (a key typed meanwhile may
/// come first).
pub fn parse(bytes: &[u8]) -> Reply {
    let Some(at) = find(bytes, START) else {
        // The reply's first bytes may be at the end.
        let tail_begins = (1..START.len()).any(|n| bytes.ends_with(&START[..n]));
        return if tail_begins {
            Reply::Partial
        } else {
            Reply::Nothing
        };
    };
    let body = &bytes[at + START.len()..];
    let end = body.iter().enumerate().find_map(|(i, b)| match b {
        0x07 => Some(i),
        0x1b if body.get(i + 1) == Some(&b'\\') => Some(i),
        _ => None,
    });
    let Some(end) = end else {
        return Reply::Partial;
    };
    match std::str::from_utf8(&body[..end]).ok().and_then(colour) {
        Some(c) => Reply::Colour(c),
        None => Reply::Unreadable,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `rgb:R/G/B` (1 to 4 hex digits each; `rgba:` with an alpha too), or
/// `#rgb`, `#rrggbb`, `#rrrgggbbb`, `#rrrrggggbbbb`.
fn colour(text: &str) -> Option<[f64; 3]> {
    let channel = |h: &str| -> Option<f64> {
        if h.is_empty() || h.len() > 4 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let max = (1u32 << (4 * h.len())) - 1;
        Some(f64::from(u32::from_str_radix(h, 16).ok()?) / f64::from(max))
    };
    let parts: Vec<&str> = if let Some(rest) = text.strip_prefix("rgb:") {
        rest.split('/').collect()
    } else if let Some(rest) = text.strip_prefix("rgba:") {
        let p: Vec<&str> = rest.split('/').collect();
        if p.len() != 4 {
            return None;
        }
        p[..3].to_vec()
    } else if let Some(hex) = text.strip_prefix('#') {
        if hex.is_empty() || hex.len() % 3 != 0 || !hex.is_ascii() {
            return None;
        }
        let n = hex.len() / 3;
        vec![&hex[..n], &hex[n..2 * n], &hex[2 * n..]]
    } else {
        return None;
    };
    if parts.len() != 3 {
        return None;
    }
    Some([channel(parts[0])?, channel(parts[1])?, channel(parts[2])?])
}

/// The rule: relative luminance (WCAG 2's, from sRGB) under 0.5 is dark.
pub fn is_dark([r, g, b]: [f64; 3]) -> bool {
    let lin = |c: f64| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) < 0.5
}

/// The background a reply names, if it names one.
pub fn of(reply: Reply) -> Option<Background> {
    match reply {
        Reply::Colour(c) if is_dark(c) => Some(Background::Dark),
        Reply::Colour(_) => Some(Background::Light),
        _ => None,
    }
}

/// `COLORFGBG` (`<fg>;<bg>`, or `<fg>;<other>;<bg>`): a background of 0–6
/// or 8 is dark, any other number light; anything else says nothing.
pub fn from_colorfgbg(value: &str) -> Option<Background> {
    let bg: u8 = value.rsplit(';').next()?.trim().parse().ok()?;
    Some(if bg <= 6 || bg == 8 {
        Background::Dark
    } else {
        Background::Light
    })
}

/// The exchange over `tty`, which must not block (a read with nothing
/// waiting fails with `WouldBlock`): write the query, read until a reply
/// ends or `timeout` passes, then read a reply that has begun to its end
/// (at most [`DRAIN`] more) and whatever else is already waiting, so none
/// of it becomes keys.
pub fn ask(tty: &mut (impl Read + Write), timeout: Duration) -> Option<Background> {
    if tty.write_all(QUERY).and_then(|()| tty.flush()).is_err() {
        return None;
    }
    let mut got = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut reply = read_until(tty, &mut got, deadline);
    if reply == Reply::Partial {
        reply = read_until(tty, &mut got, Instant::now() + DRAIN);
    }
    // Whatever is still waiting: never left for the keys.
    let mut chunk = [0u8; 256];
    let mut spare = 64;
    while spare > 0 && matches!(tty.read(&mut chunk), Ok(n) if n > 0) {
        spare -= 1;
    }
    of(reply)
}

/// Read into `got` until the reply ends or `deadline`; what it says.
fn read_until(tty: &mut impl Read, got: &mut Vec<u8>, deadline: Instant) -> Reply {
    let mut chunk = [0u8; 256];
    loop {
        match tty.read(&mut chunk) {
            Ok(0) => return parse(got),
            Ok(n) => {
                got.extend_from_slice(&chunk[..n]);
                let reply = parse(got);
                if matches!(reply, Reply::Colour(_) | Reply::Unreadable) {
                    return reply;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return parse(got);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return parse(got),
        }
    }
}

/// The non-blocking flag, by platform (no libc in the tree; these are the
/// kernels' own values).
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
const O_NONBLOCK: Option<i32> = Some(0x0004);
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))
))]
const O_NONBLOCK: Option<i32> = Some(0o4000);
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    all(
        any(target_os = "linux", target_os = "android"),
        not(any(
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "sparc",
            target_arch = "sparc64"
        ))
    )
)))]
const O_NONBLOCK: Option<i32> = None;

/// Ask the terminal (see the module docs): `None` without a terminal on
/// stdin and stdout, without `/dev/tty`, or without an answer in time. Raw
/// mode is on for the exchange (turned on here and off again if it was
/// off), so the reply is not echoed and arrives byte by byte. Before the
/// event reader starts: the main thread, once.
#[cfg(unix)]
pub fn query(timeout: Duration) -> Option<Background> {
    use ratatui::crossterm::terminal;
    use std::io::IsTerminal;
    use std::os::unix::fs::OpenOptionsExt;
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return None;
    }
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK?)
        .open("/dev/tty")
        .ok()?;
    let was_raw = terminal::is_raw_mode_enabled().unwrap_or(false);
    if !was_raw {
        terminal::enable_raw_mode().ok()?;
    }
    let answer = ask(&mut tty, timeout);
    if !was_raw {
        let _ = terminal::disable_raw_mode();
    }
    answer
}

/// No tty to ask.
#[cfg(not(unix))]
pub fn query(_timeout: Duration) -> Option<Background> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;

    fn reply(body: &str) -> Vec<u8> {
        format!("\x1b]11;{body}\x07").into_bytes()
    }

    #[test]
    fn the_reply_in_each_form() {
        let dark = of(parse(&reply("rgb:1e1e/1e1e/1e1e")));
        assert_eq!(dark, Some(Background::Dark));
        assert_eq!(
            of(parse(&reply("rgb:ffff/ffff/ffff"))),
            Some(Background::Light)
        );
        assert_eq!(of(parse(&reply("rgb:1e/1e/1e"))), Some(Background::Dark));
        assert_eq!(
            of(parse(&reply("rgb:fff/fff/fff"))),
            Some(Background::Light)
        );
        assert_eq!(of(parse(&reply("#1e1e1e"))), Some(Background::Dark));
        assert_eq!(of(parse(&reply("#ffffff"))), Some(Background::Light));
        assert_eq!(
            of(parse(&reply("rgba:ffff/ffff/ffff/ffff"))),
            Some(Background::Light)
        );
        // Ended by ESC \ instead of BEL.
        assert_eq!(
            of(parse(b"\x1b]11;rgb:0000/0000/0000\x1b\\")),
            Some(Background::Dark)
        );
        // Exact channels: 2-digit ff and 4-digit ffff are both 1.0.
        assert_eq!(
            parse(&reply("rgb:ff/0/ffff")),
            Reply::Colour([1.0, 0.0, 1.0])
        );
        // A key typed before the reply does not hide it.
        let mut keyed = b"j".to_vec();
        keyed.extend(reply("rgb:ffff/ffff/ffff"));
        assert_eq!(of(parse(&keyed)), Some(Background::Light));
    }

    #[test]
    fn a_truncated_reply_is_partial_and_garbage_says_nothing() {
        assert_eq!(parse(b"\x1b]11;rgb:1e1e/1e"), Reply::Partial);
        assert_eq!(parse(b"\x1b]1"), Reply::Partial);
        assert_eq!(parse(b"\x1b"), Reply::Partial);
        assert_eq!(parse(b""), Reply::Nothing);
        assert_eq!(parse(b"hello"), Reply::Nothing);
        assert_eq!(parse(&reply("rgb:zz/zz/zz")), Reply::Unreadable);
        assert_eq!(parse(&reply("rgb:1e/1e")), Reply::Unreadable);
        assert_eq!(parse(&reply("rgb:12345/0/0")), Reply::Unreadable);
        assert_eq!(parse(&reply("#12345")), Reply::Unreadable);
        assert_eq!(parse(&reply("black")), Reply::Unreadable);
        assert_eq!(of(Reply::Unreadable), None);
        assert_eq!(of(Reply::Partial), None);
    }

    #[test]
    fn luminance_under_a_half_is_dark() {
        assert!(is_dark([0.0, 0.0, 0.0]));
        assert!(is_dark([0x1e as f64 / 255.0; 3]));
        // Mid grey is dark by luminance (0.22).
        assert!(is_dark([0.5, 0.5, 0.5]));
        assert!(!is_dark([1.0, 1.0, 1.0]));
        // Solarized light's page (#fdf6e3) and a pale yellow are light.
        assert!(!is_dark([
            0xfd as f64 / 255.0,
            0xf6 as f64 / 255.0,
            0xe3 as f64 / 255.0
        ]));
        // Solarized dark's page (#002b36) is dark.
        assert!(is_dark([0.0, 0x2b as f64 / 255.0, 0x36 as f64 / 255.0]));
    }

    #[test]
    fn colorfgbg_reads_the_last_field() {
        assert_eq!(from_colorfgbg("15;0"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;15"), Some(Background::Light));
        assert_eq!(from_colorfgbg("12;8"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;7"), Some(Background::Light));
        assert_eq!(from_colorfgbg("15;default;0"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("default"), None);
        assert_eq!(from_colorfgbg(""), None);
    }

    /// A tty that never blocks: each read takes the next chunk, or "would
    /// block" between them; what was written is kept.
    struct FakeTty {
        chunks: VecDeque<Option<Vec<u8>>>,
        written: Vec<u8>,
    }

    impl FakeTty {
        fn new(chunks: &[Option<&[u8]>]) -> FakeTty {
            FakeTty {
                chunks: chunks.iter().map(|c| c.map(<[u8]>::to_vec)).collect(),
                written: Vec::new(),
            }
        }
    }

    impl Read for FakeTty {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.chunks.pop_front() {
                Some(Some(c)) => {
                    buf[..c.len()].copy_from_slice(&c);
                    Ok(c.len())
                }
                _ => Err(io::Error::from(ErrorKind::WouldBlock)),
            }
        }
    }

    impl Write for FakeTty {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_exchange_asks_once_and_reads_a_reply_in_pieces() {
        let mut tty = FakeTty::new(&[
            None,
            Some(b"\x1b]11;rgb:1e1e/"),
            None,
            Some(b"1e1e/1e1e\x1b"),
            Some(b"\\"),
        ]);
        assert_eq!(ask(&mut tty, TIMEOUT), Some(Background::Dark));
        assert_eq!(tty.written, QUERY);
    }

    #[test]
    fn silence_is_no_answer_within_the_timeout() {
        let mut tty = FakeTty::new(&[]);
        let start = Instant::now();
        assert_eq!(ask(&mut tty, Duration::from_millis(30)), None);
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(30), "{took:?}");
        assert!(took < Duration::from_secs(1), "{took:?}");
    }

    #[test]
    fn a_late_reply_begun_in_time_is_read_to_its_end_and_dropped_whole() {
        // The first half within the time; the rest arrives while draining
        // (every read from here on is past the deadline of 0).
        let mut tty = FakeTty::new(&[
            Some(b"\x1b]11;rgb:ff"),
            None,
            Some(b"ff/ffff/ffff\x07"),
            Some(b"x"),
        ]);
        assert_eq!(ask(&mut tty, Duration::ZERO), Some(Background::Light));
        assert!(tty.chunks.is_empty(), "what followed was read away too");
    }

    /// The live query, against the terminal running the tests: skipped
    /// without one (and under cargo, whose test harness keeps stdin a
    /// terminal only when run by hand).
    #[test]
    fn the_live_query_answers_or_stays_silent_in_time() {
        use std::io::IsTerminal;
        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            eprintln!("skipped: no terminal");
            return;
        }
        let start = Instant::now();
        let _ = query(TIMEOUT);
        assert!(start.elapsed() < TIMEOUT + DRAIN + Duration::from_millis(200));
    }
}
