use super::sshconfig::HostBlock;
use super::state::{MENU, Screen, SshState};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, List, ListItem, Paragraph, Widget, Wrap},
};

/// How many host rows the menu preview shows before eliding.
const MENU_PREVIEW_ROWS: usize = 12;

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
        Screen::Config => render_config(state, chunks[1], buf),
        Screen::Forward => render_placeholder("tunnel profiles", chunks[1], buf),
        Screen::Dashboard => render_placeholder("dashboard", chunks[1], buf),
    }

    render_help(state, chunks[2], buf);
}

fn render_help(state: &SshState, area: Rect, buf: &mut Buffer) {
    let help = match state.screen {
        Screen::Menu => " j/k: navigate   Enter: open   q: back to main menu".to_string(),
        Screen::Config if state.searching => {
            format!(" /{}_   Enter: keep filter   Esc: clear", state.search_query)
        }
        Screen::Config => {
            let filter = if state.search_query.is_empty() {
                String::new()
            } else {
                format!("   filter: /{}", state.search_query)
            };
            format!(" j/k: navigate   /: search   Esc: back   q: quit{filter}")
        }
        _ => " Esc: back to SSH menu   q: back to main menu".to_string(),
    };
    Paragraph::new(help)
        .block(Block::bordered().border_type(BorderType::Rounded))
        .style(Style::default().fg(Color::DarkGray))
        .render(area, buf);
}

fn split_panes(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area)
}

fn render_menu(state: &SshState, area: Rect, buf: &mut Buffer) {
    let panes = split_panes(area);

    let items: Vec<ListItem> = MENU
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == state.menu_index;
            let content = vec![
                Line::from(vec![
                    Span::raw(if selected { "> " } else { "  " }),
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

    let entry = state.menu_entry();
    let body = match entry.screen {
        Screen::Config => config_summary(state),
        _ => vec![Line::from(Span::styled(
            "  not implemented yet",
            Style::default().fg(Color::DarkGray),
        ))],
    };

    Paragraph::new(body)
        .block(
            Block::bordered()
                .title(format!(" Preview: {} ", entry.label))
                .border_type(BorderType::Rounded),
        )
        .render(panes[1], buf);
}

fn config_summary(state: &SshState) -> Vec<Line<'static>> {
    if let Some(error) = &state.config_error {
        return vec![Line::from(Span::styled(
            format!("  {error}"),
            Style::default().fg(Color::Red),
        ))];
    }

    let hosts = &state.config.hosts;
    let gateways = hosts.iter().filter(|h| h.gateway().is_some()).count();
    let shadowed = hosts.iter().filter(|h| h.shadowed_by.is_some()).count();

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "  {} hosts   {} via a gateway   {} shadowed{}",
                hosts.len(),
                gateways,
                shadowed,
                if state.config.read_only {
                    "   [read-only]"
                } else {
                    ""
                }
            ),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];

    for host in hosts.iter().take(MENU_PREVIEW_ROWS) {
        lines.push(Line::from(vec![
            Span::raw(format!("  {:<18}", truncate(host.alias(), 18))),
            Span::styled(endpoint(host), Style::default().fg(Color::DarkGray)),
        ]));
    }
    if hosts.len() > MENU_PREVIEW_ROWS {
        lines.push(Line::from(Span::styled(
            format!("  ... {} more", hosts.len() - MENU_PREVIEW_ROWS),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn render_config(state: &SshState, area: Rect, buf: &mut Buffer) {
    let panes = split_panes(area);
    render_host_list(state, panes[0], buf);
    render_host_preview(state, panes[1], buf);
}

fn render_host_list(state: &SshState, area: Rect, buf: &mut Buffer) {
    let title = format!(
        " Hosts ({}/{}) ",
        state.filtered_indices.len(),
        state.config.hosts.len()
    );
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded);

    if let Some(error) = &state.config_error {
        Paragraph::new(format!("  {error}"))
            .block(block)
            .style(Style::default().fg(Color::Red))
            .render(area, buf);
        return;
    }

    let width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = state
        .filtered_indices
        .iter()
        .enumerate()
        .filter_map(|(row, &index)| {
            let host = state.config.hosts.get(index)?;
            let shadowed = host.shadowed_by.is_some();
            let marker = if shadowed { "! " } else { "  " };

            let note = if let Some(line) = state.shadowing_line(host) {
                format!("dead: line {line}")
            } else if let Some(gateway) = host.gateway() {
                format!("via {gateway}")
            } else {
                String::new()
            };

            let mut style = if row == state.selected_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            if shadowed && row != state.selected_index {
                style = style.fg(Color::DarkGray);
            }

            Some(ListItem::new(Line::from(row_text(marker, host.alias(), &note, width))).style(style))
        })
        .collect();

    List::new(items).block(block).render(area, buf);
}

fn render_host_preview(state: &SshState, area: Rect, buf: &mut Buffer) {
    let Some(host) = state.selected_host() else {
        Paragraph::new("  no host selected")
            .block(
                Block::bordered()
                    .title(" Preview ")
                    .border_type(BorderType::Rounded),
            )
            .style(Style::default().fg(Color::DarkGray))
            .render(area, buf);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    if let Some(line) = state.shadowing_line(host) {
        lines.push(Line::from(Span::styled(
            format!("  dead: every keyword is already set by line {line}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }
    for raw in &state.config.lines[host.start..host.end] {
        lines.push(Line::from(format!("  {raw}")));
    }

    Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(format!(
                    " {}   lines {}-{} ",
                    host.alias(),
                    host.start + 1,
                    host.end
                ))
                .border_type(BorderType::Rounded),
        )
        // Wrap rather than truncate: a long ProxyCommand is exactly the line
        // worth reading, and cutting it looks like the value is short.
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

/// Lay out `marker + alias ... note` with the note flush against the right edge.
/// The note is what the pane exists to surface (`dead: line 60`, `via bastion`),
/// so a narrow terminal eats into the alias rather than truncating the note.
fn row_text(marker: &str, alias: &str, note: &str, width: usize) -> String {
    let note_len = note.chars().count();
    let alias_room = width.saturating_sub(marker.len() + note_len + 1);
    let alias = truncate(alias, alias_room);
    let pad = width.saturating_sub(marker.len() + alias.chars().count() + note_len);
    format!("{marker}{alias}{}{note}", " ".repeat(pad))
}

/// `HostName:Port` as written, for the one-line summary of a host.
fn endpoint(host: &HostBlock) -> String {
    let hostname = host.get("HostName").map(|d| d.value.as_str()).unwrap_or("-");
    match host.get("Port") {
        Some(port) => format!("{hostname}:{}", port.value),
        None => hostname.to_string(),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
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
