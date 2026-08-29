use super::state::{MENU, Screen, SshState};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, List, ListItem, Paragraph, Widget},
};

pub fn render(state: &SshState, area: Rect, buf: &mut Buffer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Content
            Constraint::Length(3), // Help
        ])
        .split(area);

    Block::bordered()
        .title(" Excalibur SSH ")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Cyan))
        .render(chunks[0], buf);

    match state.screen {
        Screen::Menu => render_menu(state, chunks[1], buf),
        Screen::Config => render_placeholder("ssh config", chunks[1], buf),
        Screen::Forward => render_placeholder("tunnel profiles", chunks[1], buf),
        Screen::Dashboard => render_placeholder("dashboard", chunks[1], buf),
    }

    let help = match state.screen {
        Screen::Menu => " j/k: navigate   Enter: open   q: back to main menu",
        _ => " Esc: back to SSH menu   q: back to main menu",
    };
    Paragraph::new(help)
        .block(Block::bordered().border_type(BorderType::Rounded))
        .style(Style::default().fg(Color::DarkGray))
        .render(chunks[2], buf);
}

fn render_menu(state: &SshState, area: Rect, buf: &mut Buffer) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let items: Vec<ListItem> = MENU
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == state.menu_index;
            let marker = if selected { "> " } else { "  " };
            let content = vec![
                Line::from(vec![
                    Span::raw(marker),
                    Span::styled(
                        format!("{}  {}", i + 1, entry.label),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("     {}", entry.hint),
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            let style = if selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    List::new(items)
        .block(
            Block::bordered()
                .title(" Menu ")
                .border_type(BorderType::Rounded),
        )
        .render(panes[0], buf);

    Paragraph::new("")
        .block(
            Block::bordered()
                .title(format!(" Preview: {} ", state.selected().label))
                .border_type(BorderType::Rounded),
        )
        .render(panes[1], buf);
}

fn render_placeholder(name: &str, area: Rect, buf: &mut Buffer) {
    Paragraph::new(format!("\n  {name} — not implemented yet"))
        .block(
            Block::bordered()
                .title(format!(" {name} "))
                .border_type(BorderType::Rounded),
        )
        .style(Style::default().fg(Color::DarkGray))
        .render(area, buf);
}
