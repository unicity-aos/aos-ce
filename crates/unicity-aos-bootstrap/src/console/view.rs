//! Responsive terminal presentation; never renders secret values.
use super::{App, model::Kind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

const INK: Color = Color::Rgb(223, 231, 239);
const MUTED: Color = Color::Rgb(139, 156, 174);
const ACCENT: Color = Color::Rgb(112, 218, 195);
const BG: Color = Color::Rgb(18, 24, 32);
const PANEL: Color = Color::Rgb(24, 32, 43);

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    if app.updates_open {
        super::updates_view::draw(frame, app);
        return;
    }
    let viewport = frame.area();
    let area = Rect::new(
        viewport.x + viewport.width.saturating_sub(88) / 2,
        viewport.y + viewport.height.saturating_sub(32) / 2,
        viewport.width.min(88),
        viewport.height.min(32),
    );
    app.rows.clear();
    app.buttons.clear();
    app.detail_area = Rect::default();
    frame.render_widget(
        Block::default().style(Style::default().bg(BG).fg(INK)),
        viewport,
    );
    if area.width < 64 || area.height < 24 {
        frame.render_widget(
            Paragraph::new("AOS Console\n\nPlease resize to at least 64 × 24.\nCtrl+C to close.")
                .style(Style::default().fg(INK)),
            area,
        );
        return;
    }
    let outer = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(12),
        Constraint::Length(3),
    ])
    .margin(1)
    .split(area);
    let title = if app.preview {
        " AOS   Command Center                         PREVIEW "
    } else {
        " AOS   Command Center "
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                title,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                " Review requests from your agents",
                Style::default().fg(MUTED),
            )),
        ]),
        outer[0],
    );
    let inbox = outer[1];
    let visible = usize::from(inbox.width / 26).max(1);
    let first = app.selected.saturating_sub(visible - 1);
    app.rows.resize(app.requests.len(), Rect::default());
    for (i, request) in app.requests.iter().enumerate().skip(first).take(visible) {
        let rect = Rect::new(inbox.x + (i - first) as u16 * 26, inbox.y, 24, 1);
        app.rows[i] = rect;
        let active = i == app.selected;
        let style = Style::default()
            .bg(if active { Color::Rgb(35, 60, 65) } else { BG })
            .fg(if active { ACCENT } else { INK });
        let kind = match request.kind {
            Kind::Approval(_) => "Permission",
            Kind::Secret => "Private input",
            _ => "Input request",
        };
        frame.render_widget(
            Paragraph::new(Line::from(format!(
                " {} {}  {kind}",
                if active { "›" } else { " " },
                i + 1
            )))
            .style(style),
            rect,
        );
    }
    if app.requests.is_empty() {
        frame.render_widget(
            Paragraph::new("All caught up · no pending requests")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(MUTED)),
            inbox,
        );
    }
    let panel = Block::default()
        .padding(ratatui::widgets::Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(PANEL));
    let inner = panel.inner(outer[2]);
    app.detail_area = outer[2];
    frame.render_widget(panel, outer[2]);
    if let Some(request) = app.requests.get(app.selected) {
        let options = request.options();
        let columns = if inner.width >= 64 { 2 } else { 1 };
        let button_rows = ((inner.height.saturating_sub(7)) / 3)
            .clamp(1, 3)
            .min((options.len() + 1).div_ceil(columns) as u16);
        let visible_choices = usize::from(button_rows) * columns;
        let input_height = if matches!(request.kind, Kind::Text | Kind::Secret | Kind::Array) {
            3
        } else {
            1
        };
        let heading_height = 2;
        let body = request.detail.clone();
        let body_lines = body
            .lines()
            .map(|line| {
                line.chars()
                    .count()
                    .max(1)
                    .div_ceil(usize::from(inner.width.saturating_sub(2).max(1)))
            })
            .sum::<usize>()
            .clamp(1, 7) as u16;
        let body_lines = body_lines.min(
            inner
                .height
                .saturating_sub(2 + heading_height + input_height + button_rows * 3),
        );
        let panes = Layout::vertical([
            Constraint::Length(heading_height),
            Constraint::Length(body_lines),
            Constraint::Length(input_height),
            Constraint::Length(button_rows * 3),
            Constraint::Min(0),
        ])
        .margin(1)
        .split(inner);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    &request.message,
                    Style::default().fg(INK).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    if request.principal.is_empty() {
                        request.title.clone()
                    } else {
                        format!("{}  ·  {}", request.title, request.principal)
                    },
                    Style::default().fg(MUTED),
                )),
            ]),
            panes[0],
        );
        frame.render_widget(
            Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .scroll((app.scroll, 0))
                .style(Style::default().fg(INK)),
            panes[1],
        );
        if matches!(request.kind, Kind::Text | Kind::Secret | Kind::Array) {
            let value = if request.private() {
                "•".repeat(app.input.chars().count().min(48))
            } else {
                app.input.to_string()
            };
            let label = if request.private() {
                " Secret · hidden "
            } else if matches!(request.kind, Kind::Array) {
                " Items · Alt+Enter adds a line "
            } else {
                " Your response "
            };
            frame.render_widget(
                Paragraph::new(if value.is_empty() {
                    "Type here…".to_owned()
                } else {
                    value
                })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(ratatui::widgets::BorderType::Rounded)
                        .title(label)
                        .border_style(Style::default().fg(ACCENT)),
                )
                .style(Style::default().fg(INK)),
                Rect::new(
                    panes[2].x,
                    panes[2].y,
                    panes[2].width.min(52),
                    panes[2].height,
                ),
            );
        } else {
            frame.render_widget(
                Paragraph::new("Choose how to respond").style(Style::default().fg(MUTED)),
                panes[2],
            );
        }
        let choices = std::iter::once("Cancel · Esc".to_owned())
            .chain(options)
            .collect::<Vec<_>>();
        // Keep long enum lists navigable by moving the visible window with Tab.
        let offset = (app.action / visible_choices) * visible_choices;
        for (i, label) in choices
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_choices)
        {
            let slot = i - offset;
            let width = (panes[3].width / columns as u16).min(32);
            let row = Rect::new(
                panes[3].x + (slot % columns) as u16 * width,
                panes[3].y + (slot / columns) as u16 * 3,
                width.saturating_sub(u16::from(columns > 1)),
                3,
            );
            if row.bottom() > panes[3].bottom() {
                break;
            }
            while app.buttons.len() < i {
                app.buttons.push(Rect::default());
            }
            app.buttons.push(row);
            let selected = app.action == i;
            frame.render_widget(
                Paragraph::new(format!("{}{}", if selected { "› " } else { "" }, label))
                    .alignment(ratatui::layout::Alignment::Center)
                    .block(
                        Block::bordered()
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(Style::default().fg(if selected {
                                ACCENT
                            } else {
                                MUTED
                            })),
                    )
                    .style(
                        Style::default()
                            .fg(if selected { ACCENT } else { INK })
                            .bg(PANEL),
                    ),
                row,
            );
        }
    } else {
        let content = if app.preview {
            "Ready when you need it.\n\nYou've finished the sample requests.\nNo approval or private input was sent to a runtime."
        } else {
            "Ready when you need it.\n\nKeep this console open while your agents work. Requests appear automatically.\n\nSee the connection status below for private-input availability."
        };
        frame.render_widget(
            Paragraph::new(content)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(MUTED)),
            inner.inner(ratatui::layout::Margin {
                horizontal: 2,
                vertical: 2,
            }),
        );
    }
    let keys = if app.show_help && area.width < 100 {
        " ↑↓ Requests · Tab Action · Enter Confirm · Esc Cancel"
    } else if app.show_help {
        " ↑↓ Requests · Tab Action · Enter Confirm · Esc Cancel · PgUp/PgDn Scroll · Ctrl+C Quit"
    } else {
        " Click to choose · F2 Updates · F1 Help · Ctrl+C Quit"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                if app.show_help
                    || app.requests.is_empty()
                    || app
                        .requests
                        .get(app.selected)
                        .is_some_and(|r| !matches!(r.kind, Kind::Approval(_)))
                {
                    format!(" {}", app.private_status)
                } else {
                    " Waiting for your decision on this request".into()
                },
                Style::default().fg(MUTED),
            )),
            Line::from(format!(" {}", app.notice)),
            Line::from(Span::styled(keys, Style::default().fg(MUTED))),
        ]),
        outer[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn every_focused_action_remains_clickable_at_supported_sizes() {
        for (width, height) in [(112, 34), (80, 26), (64, 24)] {
            let mut app = App::new(true);
            for action in 0..4 {
                app.action = action;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| draw(f, &mut app)).unwrap();
                assert!(
                    app.buttons.get(action).is_some_and(|r| !r.is_empty()),
                    "{width}x{height}: action {action} hidden"
                );
            }
        }
    }
    #[test]
    fn actions_are_bounded_non_overlapping_buttons() {
        for (width, height) in [(112, 34), (80, 26), (64, 20)] {
            let mut app = App::new(true);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            for (index, button) in app.buttons.iter().enumerate() {
                assert_eq!(button.height, 3);
                assert!(button.right() <= width && button.bottom() <= height);
                for other in &app.buttons[index + 1..] {
                    assert!(button.intersection(*other).is_empty());
                    assert_eq!(button.width, other.width);
                }
            }
            if width == 112 {
                assert_eq!(app.buttons.len(), 4);
                assert_eq!(app.buttons[0].y, app.buttons[1].y);
                assert!(app.buttons[0].y < 22, "actions should follow the content");
            }
        }
    }
    #[test]
    fn full_approval_choices_include_deny_without_tab_paging() {
        let mut app = App::new(true);
        app.requests[0].detail = "A capsule tool requests approval.\n\nAction: network access\nResource: api.github.com\nReason: Read pull requests".into();
        app.requests[0].kind = Kind::Approval(vec![
            "Approve Once".into(),
            "Approve for Session".into(),
            "Always Approve".into(),
            "Deny".into(),
        ]);
        let mut terminal = Terminal::new(TestBackend::new(112, 34)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(app.buttons.len(), 5);
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Deny"));
        assert!(text.contains("Resource: api.github.com"));
        assert!(text.contains("Reason: Read pull requests"));
        assert!(text.contains("Waiting for your decision"));
        assert!(!text.contains("native-setup"));
        assert!(!text.contains("PERMISSION"));
    }
    #[test]
    fn private_connection_status_remains_available_in_help() {
        let mut app = App::new(true);
        app.private_status = "Private input unavailable".into();
        app.show_help = true;
        let mut terminal = Terminal::new(TestBackend::new(112, 34)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Private input unavailable"));
    }
    #[test]
    fn secret_never_appears_in_rendered_buffer() {
        let mut app = App::new(true);
        app.select(1);
        app.append("SENSITIVE-test-value");
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!text.contains("SENSITIVE"));
        assert!(text.contains("•"));
        assert!(text.contains("PREVIEW"));
    }
    #[test]
    fn small_terminal_has_no_invisible_click_targets() {
        let mut app = App::new(true);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert!(app.buttons.is_empty());
        assert!(app.rows.is_empty());
    }
}
