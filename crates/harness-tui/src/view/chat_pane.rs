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
    let model = app
        .chat
        .model
        .as_deref()
        .map(|m| format!(" · {}", short_model(m)))
        .unwrap_or_default();
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
    h.push(("F1", "help"));
    h.push((
        "Ctrl-C",
        if app.chat.turn {
            "stop"
        } else if !app.chat.input.is_empty() {
            "clear"
        } else {
            "quit"
        },
    ));
    h
}
