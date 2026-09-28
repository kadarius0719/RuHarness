//! The chat pane (docs/CHAT-PANE-DESIGN.md §5): its title and state, the
//! transcript, the request or waiting line, the context line and the input;
//! the tab strip wherever the chat has no column of its own. Every string
//! goes through the display filter; every button is recorded as a
//! [`Hit::Chat`] as it is drawn, whole or not at all.

use super::*;
use crate::chat::transcript::Tone as T;
use crate::chat::{short_model, Phase};
use std::time::Instant;

/// From this many columns the chat has a column of its own (§5.1).
pub const CHAT_COLUMN_FROM: u16 = 156;
/// The chat's column is at most this wide.
pub const CHAT_COLUMN_MAX: u16 = 64;
/// The View keeps at least this many columns beside the chat's column.
pub const VIEW_KEEPS: u16 = 80;

fn row_style(t: T) -> Style {
    match t {
        T::You => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        T::Claude => Style::default(),
        T::Dim => dim(),
        T::Warn => Style::default().fg(Color::Yellow),
        T::Good => Style::default().fg(Color::Green),
        T::Bad => Style::default().fg(Color::Red),
        T::Cockpit => Style::default().fg(Color::Magenta),
    }
}

/// The chat's state word for its title (§5.1).
pub fn state_word(app: &App) -> &'static str {
    if app.chat.bins.is_err() {
        return "unavailable";
    }
    if app.asks.shown().is_some() {
        return "waiting for you";
    }
    if app.asks.waiting.is_some() {
        return "continuing…";
    }
    let chat_act = app.running && app.run.as_ref().is_some_and(|r| r.pending.chat.is_some());
    if chat_act {
        return "running a command for the chat";
    }
    match app.chat.phase() {
        Phase::Idle if app.chat.gen == 0 || !app.chat.ended_by_itself => "not started",
        Phase::Idle => "ended",
        Phase::Starting => "starting…",
        Phase::Thinking if app.chat.stopping() => "stopping…",
        Phase::Thinking => "thinking…",
        Phase::Ready if app.chat.stopped_last => "stopped",
        Phase::Ready => "ready",
    }
}

/// A key drawn in a bottom row: its column, width and name.
type KeyAt = (u16, u16, &'static str);

/// The strip's Chat tab: "Chat ●" outside the chat when it has a request
/// waiting or new output.
fn chat_tab(app: &App) -> &'static str {
    let news = app.asks.shown().is_some() || app.asks.waiting.is_some() || app.chat.unseen;
    if news && app.focus != Focus::Chat {
        " Chat ● "
    } else {
        " Chat "
    }
}

/// The tab strip's width as drawn now, its border gap included (the title
/// is cut only as far as it needs — fix check).
fn strip_width(app: &App) -> u16 {
    [" View ", chat_tab(app)]
        .iter()
        .map(|l| width_of(l) as u16 + 1)
        .sum::<u16>()
        + 1
}

/// Buttons laid on a border row from the right edge leftwards (`end`
/// exclusive), each whole or not at all; their hits recorded. Returns the
/// column left of the leftmost drawn.
fn border_buttons(
    frame: &mut Frame,
    app: &mut App,
    y: u16,
    left: u16,
    end: u16,
    buttons: &[(String, &'static str, Style)],
) -> u16 {
    let mut x = end;
    for (label, key, style) in buttons.iter().rev() {
        let w = width_of(label) as u16;
        if x < left + w + 1 {
            break;
        }
        x -= w;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(label.clone(), *style))),
            Rect::new(x, y, w, 1),
        );
        app.hits.push((Rect::new(x, y, w, 1), Hit::Chat(key)));
        x -= 1;
    }
    x
}

/// The tab strip (§5.1): `View │ Chat` on the top border of the pane
/// `on` — wherever the chat has no column of its own — its tab shown
/// active when it is that pane's (neither on the files: fix check N6);
/// "Chat ●" when it has a request waiting or new output. Returns the
/// column left of it.
pub fn tab_strip(frame: &mut Frame, app: &mut App, area: Rect, on: Focus) -> u16 {
    if area.width < 20 || area.height == 0 {
        return area.x + area.width;
    }
    let chat_label = chat_tab(app);
    let active = Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let view_style = if on == Focus::View { active } else { dim() };
    let chat_style = if on == Focus::Chat {
        active
    } else if chat_label.contains('●') {
        Style::default().fg(Color::Yellow)
    } else {
        dim()
    };
    let end = area.x + area.width - 1;
    let buttons = [
        (" View ".to_string(), "tab-view", view_style),
        (chat_label.to_string(), "tab-chat", chat_style),
    ];
    // Both or neither: the strip is one control.
    let need: u16 = buttons.iter().map(|(l, ..)| width_of(l) as u16 + 1).sum();
    if end < area.x + need + 2 {
        return area.x + area.width;
    }
    border_buttons(frame, app, area.y, area.x + 1, end, &buttons)
}

/// Draw the chat in `area`; `column`: it has a column of its own (its
/// `[×]` closes it).
pub fn draw_chat(frame: &mut Frame, app: &mut App, area: Rect, column: bool, strip: bool) {
    let focused =
        app.focus == Focus::Chat && matches!(app.mode, Mode::Normal | Mode::Details { .. });
    let mut model = app
        .chat
        .model
        .as_deref()
        .map(|m| format!(" · {}", short_model(m)))
        .unwrap_or_default();
    // After `init`, what signed it in (§1.1) — said when it is not the
    // subscription.
    if let Some(src) = app.chat.api_key_source.as_deref().filter(|s| *s != "none") {
        model.push_str(&format!(" · {src}"));
    }
    // The title's buttons — [Stop] and [?] first when room is short
    // (review USE-10) — then the strip, from the right; the title is cut
    // with "…" before them, never overwritten.
    let right = area.x + area.width.saturating_sub(1);
    let strip_w = if strip && area.width >= 20 {
        strip_width(app)
    } else {
        0
    };
    let mut wanted: Vec<(u8, String, &'static str)> = Vec::new();
    if app.chat.turn && !app.chat.stopping() {
        wanted.push((0, "[Stop]".into(), "stop"));
    }
    wanted.push((1, "[?]".into(), "help"));
    if column {
        wanted.push((2, "[×]".into(), "close"));
    }
    if app.chat.has_conversation() {
        wanted.push((3, "[New]".into(), "new"));
    }
    let min_title = 10u16;
    let mut room = right
        .saturating_sub(area.x + 1 + min_title)
        .saturating_sub(strip_w);
    let mut chosen: Vec<(u8, String, &'static str)> = Vec::new();
    for w in wanted {
        let need = width_of(&w.1) as u16 + 1;
        if need <= room {
            room -= need;
            chosen.push(w);
        }
    }
    // Shown in a steady order: [Stop] [New] [?] [×].
    chosen.sort_by_key(|(p, ..)| match p {
        0 => 0,
        3 => 1,
        1 => 2,
        _ => 3,
    });
    let buttons_w: u16 = chosen.iter().map(|(_, l, _)| width_of(l) as u16 + 1).sum();
    let title_room = (right.saturating_sub(area.x + 2 + strip_w + buttons_w)) as usize;
    let title = ellipsis(
        &format!(" Chat — {}{model} ", state_word(app)),
        title_room.max(4),
    );
    let block = pane_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.push((area, Hit::Pane(Focus::Chat)));
    let end = if strip {
        tab_strip(frame, app, area, Focus::Chat)
    } else {
        right
    };
    let styled: Vec<(String, &'static str, Style)> =
        chosen.into_iter().map(|(_, l, k)| (l, k, bold())).collect();
    border_buttons(frame, app, area.y, area.x + 1, end, &styled);
    if inner.width < 4 || inner.height < 3 {
        return;
    }
    let width = inner.width as usize;
    // The bottom block, by priority when rows are short (review USE-5): the
    // request's buttons (or the waiting line), the input's cursor row, the
    // request's first words, the context line, more input rows, the rest
    // of the words — the request is never the first thing cut.
    let (input_rows, cursor, _) = app.chat.input.layout(width.saturating_sub(2));
    let shown_input = input_rows.len().clamp(1, crate::chat::input::MAX_ROWS);
    let now = Instant::now();
    let yellow = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut words: Vec<Line<'static>> = Vec::new();
    let mut must: Vec<(Line<'static>, Vec<KeyAt>)> = Vec::new();
    if let Some(r) = app.asks.shown().cloned() {
        let settled = app.asks.settled(now);
        let all = wrapped(&format!("Asks: {}", r.words), width, yellow);
        let cut = all.len() > 2;
        words = all.into_iter().take(2).collect();
        if cut {
            if let Some(last) = words.pop() {
                let text: String = last.spans.iter().map(|s| s.content.to_string()).collect();
                words.push(Line::from(Span::styled(
                    ellipsis(&format!("{text} …"), width),
                    yellow,
                )));
            }
        }
        let review_ok = settled && !app.running;
        let style = |ok: bool| if ok { bold() } else { dim() };
        let mut labels: Vec<(String, &'static str, Style)> = vec![
            (
                if app.running {
                    "[Review — later]".to_string()
                } else {
                    "[Review Enter]".to_string()
                },
                "review",
                style(review_ok),
            ),
            ("[Decline Esc]".into(), "decline", style(settled)),
        ];
        if !app.chat.input.is_empty() {
            labels.push((
                "[Decline with my draft]".into(),
                "decline-draft",
                style(settled),
            ));
        }
        // The buttons wrap onto a second row rather than drop one.
        let mut row: (Vec<Span<'static>>, Vec<KeyAt>) = (Vec::new(), Vec::new());
        let mut x = 0u16;
        for (label, key, st) in labels {
            let w = width_of(&label) as u16;
            if x > 0 && x + w > inner.width {
                must.push((
                    Line::from(std::mem::take(&mut row.0)),
                    std::mem::take(&mut row.1),
                ));
                x = 0;
            }
            if w > inner.width {
                continue;
            }
            row.1.push((x, w, key));
            row.0.push(Span::styled(label, st));
            row.0.push(Span::raw("  "));
            x += w + 2;
        }
        must.push((Line::from(row.0), row.1));
    } else if let Some(w) = app.asks.waiting.clone() {
        let lead = ellipsis(&format!("{} — ", w.words), width.saturating_sub(12));
        let x = width_of(&lead) as u16;
        must.push((
            Line::from(vec![
                Span::styled(lead, Style::default().fg(Color::Yellow)),
                Span::styled("[Hold Esc]", bold()),
            ]),
            vec![(x, 10, "hold")],
        ));
    }
    let about = Line::from(Span::styled(
        ellipsis(&format!("About: {}", app.about()), width),
        dim(),
    ));
    let avail = inner.height.saturating_sub(1) as usize;
    let mut left = avail;
    let mut take = |want: usize| {
        let n = want.min(left);
        left -= n;
        n
    };
    let n_must = take(must.len());
    let n_cursor = take(1);
    let n_words1 = take(words.len().min(1));
    let n_about = take(1);
    let n_input_more = take(shown_input.saturating_sub(1));
    let n_words2 = take(words.len().saturating_sub(1));
    // The input's rows: a window ending at the cursor's row.
    let n_input = n_cursor + n_input_more;
    let start = (cursor.0 + 1).saturating_sub(n_input.max(1));
    let mut bottom: Vec<Line<'static>> = Vec::new();
    let mut hits: Vec<(usize, u16, u16, &'static str)> = Vec::new();
    bottom.extend(words.iter().take(n_words1 + n_words2).cloned());
    for (line, row_hits) in must.into_iter().take(n_must) {
        for (x, w, k) in row_hits {
            hits.push((bottom.len(), x, w, k));
        }
        bottom.push(line);
    }
    if n_about > 0 {
        bottom.push(about);
    }
    let input_top = bottom.len();
    let placeholder = if app.asks.shown().is_some() {
        "Enter reviews the request — or type"
    } else {
        "type here — Enter sends"
    };
    for (i, row) in input_rows.iter().enumerate().skip(start).take(n_input) {
        let lead = if i == 0 { "› " } else { "  " };
        if app.chat.input.is_empty() && focused {
            bottom.push(Line::from(vec![
                Span::styled(lead, bold()),
                Span::styled(placeholder, dim()),
            ]));
        } else {
            bottom.push(Line::from(vec![
                Span::styled(lead, bold()),
                Span::raw(row.clone()),
            ]));
        }
    }
    let bottom_h = bottom.len() as u16;
    let transcript_h = inner.height - bottom_h;
    let t_area = Rect::new(inner.x, inner.y, inner.width, transcript_h);
    let b_area = Rect::new(inner.x, inner.y + transcript_h, inner.width, bottom_h);
    app.asks.transcript_size = (width, transcript_h as usize);
    // The transcript, or what the pane is before the first message.
    let lines: Vec<Line<'static>> = if app.chat.transcript.is_empty() {
        let intro = match &app.chat.bins {
            Ok(bins) => format!(
                "Ask for model work here — for example \"migrate this\". The chat reads the \
                 project through the harness and asks before it runs anything; you confirm \
                 every act. It will use {} (claude at {}).",
                app.chat.sign_in,
                bins.claude.display()
            ),
            Err(why) => format!("The chat is unavailable: {why}"),
        };
        wrapped(&intro, width, dim())
            .into_iter()
            .take(transcript_h as usize)
            .collect()
    } else {
        app.chat
            .transcript
            .window(width, transcript_h as usize)
            .into_iter()
            .map(|(t, text)| Line::from(Span::styled(text, row_style(t))))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), t_area);
    frame.render_widget(Paragraph::new(bottom), b_area);
    for (row, x, w, key) in hits {
        let y = b_area.y + row as u16;
        if y < b_area.y + b_area.height && x + w <= b_area.width {
            app.hits
                .push((Rect::new(b_area.x + x, y, w, 1), Hit::Chat(key)));
        }
    }
    if focused && matches!(app.mode, Mode::Normal) && n_input > 0 {
        let row = input_top + cursor.0.saturating_sub(start);
        let y = b_area.y + row as u16;
        let x = b_area.x + 2 + cursor.1 as u16;
        if y < b_area.y + b_area.height && x < b_area.x + b_area.width {
            frame.set_cursor_position((x, y));
        }
    }
    if focused {
        app.chat.unseen = false;
    }
}

/// The chat's keys for the hint bar (§5.4): what `Enter` and `Esc` do now,
/// the line break, scrolling, the pane, help, and `Ctrl-C`'s meaning.
pub fn chat_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut h: Vec<(&'static str, &'static str)> = Vec::new();
    if app.running {
        h.push(("Ctrl-X", "cancel"));
    }
    if !app.chat.input.is_empty() {
        h.push(("Enter", "send"));
    } else if app.asks.shown().is_some() && !app.running {
        h.push(("Enter", "review"));
    }
    let chat_act = app.running && app.run.as_ref().is_some_and(|r| r.pending.chat.is_some());
    if app.asks.shown().is_some() {
        h.push(("Esc", "decline"));
    } else if app.asks.waiting.is_some() {
        h.push(("Esc", "hold"));
    } else if app.chat.turn && !chat_act && !app.chat.stopping() {
        h.push(("Esc", "stop"));
    }
    h.push(("Ctrl-J", "new line"));
    if app.chat.input.lines() <= 1 {
        h.push(("↑↓", "scroll"));
    }
    h.push(("Tab", "pane"));
    h
}

/// The chat's hint bar tail, never dropped (as `? help` and `q quit` in the
/// panes): help, and what `Ctrl-C` means now.
pub fn chat_hint_tail(app: &App) -> Vec<(&'static str, &'static str)> {
    vec![
        ("F1", "help"),
        (
            "Ctrl-C",
            if app.chat.turn && !app.chat.stopping() {
                "stop"
            } else if !app.chat.input.is_empty() {
                "clear"
            } else {
                "quit"
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::asks::{ChatTag, Request};
    use crate::app::tests::app;
    use crate::app::{Act, Pending};
    use crate::testutil::TmpDir;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use std::time::{Duration, Instant};

    const MODEL: &str = "claude-haiku-4-5-20251001";

    fn render(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    fn cells(buffer: &Buffer, r: Rect) -> String {
        let mut s = String::new();
        for x in r.x..r.x + r.width {
            s.push_str(buffer[(x, r.y)].symbol());
        }
        s
    }

    /// The golden of `name`, the scratch copy's path masked (compare
    /// `view::tests::golden`).
    fn golden(name: &str, app: &App, buffer: &Buffer) {
        let got = text(buffer).replace(&app.config.target.display().to_string(), "<target>");
        let got: String = {
            let mut out = String::new();
            let mut rest = got.as_str();
            while let Some(i) = rest.find("harness-tui-") {
                out.push_str(&rest[..i + "harness-tui-".len()]);
                rest = &rest[i + "harness-tui-".len()..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                    .unwrap_or(rest.len());
                out.extend(
                    rest[..end]
                        .chars()
                        .map(|c| if c.is_ascii_digit() { '0' } else { c }),
                );
                rest = &rest[end..];
            }
            out.push_str(rest);
            out
        };
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name);
        if std::env::var("RUHARNESS_UPDATE_TUI_GOLDENS").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &got).unwrap();
            return;
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing golden {}", path.display()));
        assert!(want == got, "golden {name} differs:\n{got}");
    }

    /// Every chat key drawn shows its words under it (§5.5) — the hits are
    /// what the person sees.
    fn hits_match(app: &App, buffer: &Buffer) {
        for (r, h) in &app.hits {
            if let Hit::Chat(k) = h {
                let under = cells(buffer, *r);
                let want: &[&str] = match *k {
                    "review" => &["[Review Enter]", "[Review — later]"],
                    "decline" => &["[Decline Esc]"],
                    "decline-draft" => &["[Decline with my draft]"],
                    "hold" => &["[Hold Esc]"],
                    "stop" => &["[Stop]"],
                    "new" => &["[New]"],
                    "help" => &["[?]"],
                    "close" => &["[×]"],
                    "tab-view" => &[" View "],
                    "tab-chat" => &[" Chat ", " Chat ● "],
                    other => panic!("an unknown chat key {other}"),
                };
                assert!(want.contains(&under.as_str()), "{k}: {under:?} at {r:?}");
            }
        }
    }

    fn chat_app(tag: &str, tmp: &TmpDir) -> App {
        let mut a = app(tag);
        a.chat_on = true;
        a.chat.test_live(&tmp.0, MODEL);
        a.chat.turn = false;
        a.chat.transcript.you("u1", "Please migrate u-lib.");
        a.chat.transcript.message_start("m1");
        a.chat
            .transcript
            .block("m1", Some(0), "I will ask to migrate u-lib.");
        a.chat.transcript.line(
            crate::chat::transcript::Tone::Dim,
            "· read the project's status — done",
        );
        a
    }

    fn request(a: &mut App, settled: bool) {
        let tag = ChatTag {
            gen: a.chat.gen,
            request_id: "r1".into(),
            tool: "harness_migrate".into(),
            model: MODEL.into(),
            grant: true,
            answer: None,
            key: None,
            permitted: false,
            attempt: None,
            epoch: 0,
        };
        a.asks.requests.push_back(Request {
            pending: Pending {
                act: Act::Migrate,
                argv: Vec::new(),
                label: "Migrate u-lib (asked in chat)".into(),
                unit: Some("u-lib".into()),
                attempt: None,
                cleanup: None,
                expect_attempt: None,
                note: None,
                shown_digest: None,
                chat: Some(Box::new(tag)),
            },
            shown_at: Some(if settled {
                Instant::now() - Duration::from_secs(2)
            } else {
                Instant::now()
            }),
            words: "Migrate u-lib — a model call, answered here in chat".into(),
            continue_asks: false,
        });
        a.now = Instant::now();
    }

    /// §5.1 goldens: at 79 one pane at a time; 80–155 the right column shows
    /// the chat only while it is focused, the tab strip always; from 156 a
    /// column of its own (the View keeping 80), `[×]` closing it.
    #[test]
    fn goldens_of_the_chat_by_width() {
        let tmp = TmpDir::new("chat-goldens");
        let mut a = chat_app("chat-goldens", &tmp);
        request(&mut a, true);
        for w in [79u16, 80, 120, 155, 156, 200] {
            a.chat_column = false;
            a.focus = crate::app::Focus::Files;
            let b = render(&mut a, w, 24);
            hits_match(&a, &b);
            let screen = text(&b);
            if w >= 80 {
                assert!(
                    screen.lines().next().unwrap().contains(" View "),
                    "{w}: {screen}"
                );
                assert!(
                    screen.lines().next().unwrap().contains("Chat ●"),
                    "{w}: a request waits"
                );
            }
            a.focus = crate::app::Focus::Chat;
            a.chat_column = true;
            let b = render(&mut a, w, 24);
            hits_match(&a, &b);
            golden(&format!("chat-{w}.txt"), &a, &b);
            let screen = text(&b);
            assert!(screen.contains("Asks: Migrate u-lib"), "{w}");
            assert!(screen.contains("[Review Enter]"), "{w}");
            if w >= CHAT_COLUMN_FROM {
                assert!(screen.contains("[×]"), "{w}: a column of its own");
                assert!(screen.lines().next().unwrap().contains("Files"), "{w}");
            } else {
                assert!(!screen.contains("[×]"), "{w}");
            }
        }
    }

    /// The View keeps 80 columns beside the chat's column.
    #[test]
    fn the_view_keeps_eighty_columns() {
        let tmp = TmpDir::new("chat-width");
        let mut a = chat_app("chat-width", &tmp);
        a.focus = crate::app::Focus::Chat;
        a.chat_column = true;
        for w in [156u16, 170, 200, 250] {
            render(&mut a, w, 24);
            let view = a
                .hits
                .iter()
                .find(|(_, h)| *h == Hit::Pane(crate::app::Focus::View))
                .map(|(r, _)| *r)
                .unwrap();
            let chat = a
                .hits
                .iter()
                .find(|(_, h)| *h == Hit::Pane(crate::app::Focus::Chat))
                .map(|(r, _)| *r)
                .unwrap();
            assert!(view.width >= VIEW_KEEPS, "{w}: {view:?}");
            assert!(chat.width <= CHAT_COLUMN_MAX, "{w}: {chat:?}");
        }
    }

    /// A request settling is shown greyed; a waiting Continue shows its
    /// Hold; a long transcript follows its newest line.
    /// Review USE-12, fix check N3: the title says where the chat is — and
    /// New chat after a chat that ended leaves "ended".
    #[test]
    fn the_title_says_where_the_chat_is() {
        let tmp = TmpDir::new("chat-words");
        let mut a = chat_app("chat-words", &tmp);
        assert_eq!(state_word(&a), "ready");
        a.chat.turn = true;
        assert_eq!(state_word(&a), "thinking…");
        assert!(a.chat.stop());
        assert_eq!(state_word(&a), "stopping…");
        a.chat.turn = false;
        a.chat.stopped_last = true;
        assert_eq!(state_word(&a), "stopped");
        let _ = a.chat.new_chat(Instant::now());
        assert_eq!(state_word(&a), "not started");
        a.chat.ended_by_itself = true;
        assert_eq!(state_word(&a), "ended");
        let _ = a.chat.new_chat(Instant::now());
        assert_eq!(state_word(&a), "not started", "New chat after an ended one");
    }

    /// Review USE-18: the request's Review waits for a running command in
    /// words — "later" — and the hint bar offers no `Enter review` then.
    #[test]
    fn review_waits_for_the_running_command() {
        let tmp = TmpDir::new("chat-later");
        let mut a = chat_app("chat-later", &tmp);
        a.focus = crate::app::Focus::Chat;
        request(&mut a, true);
        assert!(chat_hints(&a).contains(&("Enter", "review")));
        a.running = true;
        assert!(!chat_hints(&a).iter().any(|h| h.1 == "review"));
        let b = render(&mut a, 120, 24);
        hits_match(&a, &b);
        assert!(text(&b).contains("[Review — later]"));
    }

    /// Review USE-4, USE-5, USE-10, USE-11; fix check N6: what the pane
    /// keeps when room is short — the request's buttons before anything
    /// else below the transcript, [Stop] and [?] before [New] on the
    /// title; the settle read from the clock, not the last input's time;
    /// the strip on the files pane with neither tab shown active.
    #[test]
    fn the_pane_keeps_what_matters_when_room_is_short() {
        let tmp = TmpDir::new("chat-short");
        let mut a = chat_app("chat-short", &tmp);
        a.focus = crate::app::Focus::Chat;
        request(&mut a, true);
        // The last input long before the request was shown: settled by the
        // clock all the same (USE-4).
        a.now = Instant::now() - Duration::from_secs(60);
        a.chat.input.insert("a draft");
        // Three rows inside the pane: the buttons and the cursor's row
        // below one row of transcript.
        let b = render(&mut a, 120, 8);
        let screen = text(&b);
        assert!(screen.contains("[Review Enter]"), "{screen}");
        assert!(
            screen.contains("› a draft") || screen.contains("a draft"),
            "{screen}"
        );
        assert!(
            !screen.contains("About:"),
            "the context line goes first: {screen}"
        );
        let review = a
            .hits
            .iter()
            .find(|(_, h)| *h == Hit::Chat("review"))
            .map(|(r, _)| *r)
            .expect("the Review button");
        assert!(
            b[(review.x, review.y)].modifier.contains(Modifier::BOLD),
            "settled: the button is live"
        );
        // A narrow pane with a turn running: [Stop] and [?] kept, [New]
        // dropped (USE-10).
        a.asks.requests.clear();
        a.chat.turn = true;
        let b = render(&mut a, 40, 20);
        let top = text(&b).lines().next().unwrap_or_default().to_string();
        assert!(top.contains("[Stop]") && top.contains("[?]"), "{top}");
        assert!(!top.contains("[New]"), "{top}");
        // The files pane alone: the strip on it, neither tab active
        // (USE-11, N6).
        a.focus = crate::app::Focus::Files;
        let b = render(&mut a, 60, 20);
        for key in ["tab-view", "tab-chat"] {
            let r = a
                .hits
                .iter()
                .find(|(_, h)| *h == Hit::Chat(key))
                .map(|(r, _)| *r)
                .unwrap_or_else(|| panic!("the strip's {key} on the files pane"));
            assert!(
                !b[(r.x, r.y)].modifier.contains(Modifier::REVERSED),
                "{key} shown active over the files"
            );
        }
    }

    /// Review USE-14: Help says why the chat is unavailable.
    #[test]
    fn help_says_why_the_chat_is_unavailable() {
        let mut a = app("chat-help-why");
        a.chat_on = true;
        a.chat.bins = Err("no `claude` on PATH".into());
        a.mode = Mode::Help { scroll: 0 };
        let b = render(&mut a, 120, 200);
        assert!(
            text(&b).contains("The chat is unavailable here: no `claude` on PATH."),
            "{}",
            text(&b)
        );
    }

    #[test]
    fn request_waiting_and_long_transcript() {
        let tmp = TmpDir::new("chat-lines");
        let mut a = chat_app("chat-lines", &tmp);
        a.focus = crate::app::Focus::Chat;
        request(&mut a, false);
        let b = render(&mut a, 120, 24);
        hits_match(&a, &b);
        let r = a.asks.requests.pop_front().unwrap();
        a.asks.waiting = Some(Request {
            words: "Continues a-d2e5513c… turn 2".into(),
            ..r
        });
        for i in 0..200 {
            a.chat
                .transcript
                .line(crate::chat::transcript::Tone::Dim, format!("line {i}"));
        }
        let b = render(&mut a, 120, 24);
        hits_match(&a, &b);
        let screen = text(&b);
        assert!(screen.contains("[Hold Esc]"), "{screen}");
        assert!(
            screen.contains("line 199"),
            "follows the newest line: {screen}"
        );
        golden("chat-waiting-120.txt", &a, &b);
    }
}
