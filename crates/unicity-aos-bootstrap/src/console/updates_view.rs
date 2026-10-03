//! Updates remain separate from approval responses and secret-entry buffers.
use super::App;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    app.rows.clear();
    app.buttons.clear();
    app.detail_area = Rect::default();
    let area = frame.area();
    let style = Style::default()
        .bg(Color::Rgb(18, 24, 32))
        .fg(Color::Rgb(223, 231, 239));
    frame.render_widget(Block::default().style(style), area);
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .margin(2)
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            "AOS  /  Software updates\n{} pending requests · F2 returns to Requests",
            app.requests.len()
        ))
        .style(style),
        regions[0],
    );
    let mut lines = Vec::new();
    if let Some(inventory) = &app.updates {
        lines.push(Line::from(format!("Channel: {}", inventory.channel)));
        for item in &inventory.items {
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "{}  {}{}",
                item.name,
                item.installed_version,
                item.candidate_version
                    .as_ref()
                    .map(|v| format!(" → {v}"))
                    .unwrap_or_default()
            )));
            lines.push(Line::from(item.availability.replace('_', " ")));
            lines.push(Line::from(item.message.clone()));
        }
    } else {
        lines.push(Line::from(
            "Check for updates to see authenticated release information.",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((app.updates_scroll, 0))
            .style(style),
        regions[1],
    );
    let message = if app.updates_confirm {
        "Install all available updates? Sessions may need reconnection. Enter confirms; Esc cancels."
    } else {
        &app.updates_notice
    };
    frame.render_widget(
        Paragraph::new(message)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::Rgb(112, 218, 195))),
        regions[2],
    );
    let buttons = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(regions[3]);
    for (i, label) in [
        if app.updates_busy {
            "Working…"
        } else {
            "[R] Check / Retry"
        },
        if app.updates_confirm {
            "[Enter] Confirm installation"
        } else {
            "[U] Update all"
        },
    ]
    .iter()
    .enumerate()
    {
        frame.render_widget(
            Paragraph::new(*label)
                .block(Block::default().borders(Borders::ALL))
                .style(style),
            buttons[i],
        );
        app.buttons.push(buttons[i]);
    }
    frame.render_widget(
        Paragraph::new("F2 Requests / Updates    Ctrl+C Close when idle").style(style),
        regions[4],
    );
}
