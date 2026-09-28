//! The chat pane (docs/CHAT-PANE-DESIGN.md §5): its title and state, the
//! transcript, the request or waiting line, the context line and the input;
//! the tab strip wherever the chat has no column of its own. Every string
//! goes through the display filter; every button is recorded as a
//! [`Hit::Chat`] as it is drawn, whole or not at all.

use super::*;
use crate::chat::transcript::Tone as T;
use crate::chat::{short_model, Phase};

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
    if app.asks.shown().is_some() || app.asks.waiting.is_some() {
        return "waiting for you";
    }
    let chat_act = app.running && app.run.as_ref().is_some_and(|r| r.pending.chat.is_some());
    if chat_act {
        return "running a command for the chat";
    }
    match app.chat.phase() {
        Phase::Idle if app.chat.gen == 0 => "not started",
        Phase::Idle => "ended",
        Phase::Starting => "starting…",
        Phase::Thinking => "thinking…",
        Phase::Ready => "ready",
    }
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

/// The tab strip (§5.1): `View │ Chat` on the right column's top border,
/// wherever the chat has no column of its own — "Chat ●" when it has a
/// request waiting or new output. Returns the column left of it.
pub fn tab_strip(frame: &mut Frame, app: &mut App, area: Rect) -> u16 {
    if area.width < 20 || area.height == 0 {
        return area.x + area.width;
    }
    let on_chat = app.focus == Focus::Chat;
    let news = app.asks.shown().is_some() || app.asks.waiting.is_some() || app.chat.unseen;
    let chat_label = if news && !on_chat {
        " Chat ● "
    } else {
        " Chat "
    };
    let active = Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let (view_style, chat_style) = if on_chat {
        (dim(), active)
    } else {
        (
            active,
            if news {
                Style::default().fg(Color::Yellow)
            } else {
                dim()
            },
        )
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
    let title = format!(" Chat — {}{model} ", state_word(app));
    let block = pane_block(display::line(&title), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.push((area, Hit::Pane(Focus::Chat)));
    // The title's buttons, then the strip, from the right.
    let mut end = if strip {
        tab_strip(frame, app, area)
    } else {
        area.x + area.width - 1
    };
    let mut buttons: Vec<(String, &'static str, Style)> = Vec::new();
    if app.chat.turn {
        buttons.push(("[Stop]".into(), "stop", bold()));
    }
    if app.chat.has_conversation() {
        buttons.push(("[New]".into(), "new", bold()));
    }
    buttons.push(("[?]".into(), "help", bold()));
    if column {
        buttons.push(("[×]".into(), "close", bold()));
    }
    let title_end = area.x + 2 + width_of(&display::line(&title)) as u16;
    if end > title_end + 1 {
        end = border_buttons(frame, app, area.y, title_end, end, &buttons);
    }
    let _ = end;
    if inner.width < 4 || inner.height < 3 {
        return;
    }
    let width = inner.width as usize;
    // The bottom: the request or waiting line, the context line, the input.
    let (input_rows, cursor, first) = app.chat.input.layout(width.saturating_sub(2));
    let shown_input = input_rows.len().clamp(1, crate::chat::input::MAX_ROWS);
    let mut bottom: Vec<Line<'static>> = Vec::new();
    let mut bottom_hits: Vec<(u16, u16, u16, &'static str)> = Vec::new();
    let now = app.now;
    if let Some(r) = app.asks.shown().cloned() {
        let settled = app.asks.settled(now);
        let lead = format!("Asks: {}", r.words);
        for l in wrapped(
            &lead,
            width,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .into_iter()
        .take(2)
        {
            bottom.push(l);
        }
        let review_ok = settled && !app.running;
        let style = |ok: bool| if ok { bold() } else { dim() };
        let mut labels: Vec<(String, &'static str, Style)> = vec![
            (
                if app.running {
                    "[Review — after the running command]".to_string()
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
        let mut spans = Vec::new();
        let mut x = 0u16;
        for (label, key, st) in labels {
            let w = width_of(&label) as u16;
            if x + w > inner.width {
                break;
            }
            bottom_hits.push((bottom.len() as u16, x, w, key));
            spans.push(Span::styled(label, st));
            spans.push(Span::raw("  "));
            x += w + 2;
        }
        bottom.push(Line::from(spans));
    } else if let Some(w) = app.asks.waiting.clone() {
        let lead = format!("{} — ", w.words);
        let lead = ellipsis(&lead, width.saturating_sub(12));
        let x = width_of(&lead) as u16;
        bottom_hits.push((bottom.len() as u16, x, 10, "hold"));
        bottom.push(Line::from(vec![
            Span::styled(lead, Style::default().fg(Color::Yellow)),
            Span::styled("[Hold Esc]", bold()),
        ]));
    }
    bottom.push(Line::from(Span::styled(
        ellipsis(&format!("About: {}", app.about()), width),
        dim(),
    )));
    let input_top = bottom.len();
    for (i, row) in input_rows.iter().skip(first).take(shown_input).enumerate() {
        let lead = if i == 0 && first == 0 { "› " } else { "  " };
        bottom.push(Line::from(vec![
            Span::styled(lead, bold()),
            Span::raw(row.clone()),
        ]));
    }
    if app.chat.input.is_empty() && focused {
        if let Some(l) = bottom.last_mut() {
            *l = Line::from(vec![
                Span::styled("› ", bold()),
                Span::styled("type here — Enter sends", dim()),
            ]);
        }
    }
    let bottom_h = (bottom.len() as u16).min(inner.height.saturating_sub(1));
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
    let skip = bottom.len().saturating_sub(bottom_h as usize);
    frame.render_widget(
        Paragraph::new(bottom.into_iter().skip(skip).collect::<Vec<_>>()),
        b_area,
    );
    for (row, x, w, key) in bottom_hits {
        if (row as usize) < skip {
            continue;
        }
        let y = b_area.y + row - skip as u16;
        if y < b_area.y + b_area.height && x + w <= b_area.width {
            app.hits
                .push((Rect::new(b_area.x + x, y, w, 1), Hit::Chat(key)));
        }
    }
    if focused && matches!(app.mode, Mode::Normal) {
        let row = input_top + cursor.0.saturating_sub(first);
        if row >= skip {
            let y = b_area.y + (row - skip) as u16;
            let x = b_area.x + 2 + cursor.1 as u16;
            if y < b_area.y + b_area.height && x < b_area.x + b_area.width {
                frame.set_cursor_position((x, y));
            }
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
    } else if app.asks.shown().is_some() {
        h.push(("Enter", "review"));
    }
    let chat_act = app.running && app.run.as_ref().is_some_and(|r| r.pending.chat.is_some());
    if app.asks.shown().is_some() {
        h.push(("Esc", "decline"));
    } else if app.asks.waiting.is_some() {
        h.push(("Esc", "hold"));
    } else if app.chat.turn && !chat_act {
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
            if app.chat.turn {
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
                    "review" => &["[Review Enter]", "[Review — after the running command]"],
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
