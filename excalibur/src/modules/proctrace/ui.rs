use super::collector::Supervisor;
use super::network::{ConnectionState, Protocol};
use super::state::{InputMode, ProcessTracerState};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, StatefulWidget, Widget,
    },
};

/// Render the process tracer UI
pub fn render(state: &ProcessTracerState, area: Rect, buf: &mut Buffer) {
    match state.input_mode {
        InputMode::Query => render_query_mode(state, area, buf),
        InputMode::ViewResults => render_results_mode(state, area, buf),
    }

    // Render notification if present
    if let Some((ref msg, _)) = state.notification {
        render_notification(msg, area, buf);
    }
}

/// Render query mode (input + help)
fn render_query_mode(state: &ProcessTracerState, area: Rect, buf: &mut Buffer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(5), // Input box (larger)
            Constraint::Min(8),    // Help text
            Constraint::Length(3), // Status bar
        ])
        .split(area);

    // Header
    let header = Block::bordered()
        .title(" Process Tracer - Why Is This Running? ")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Cyan));
    header.render(chunks[0], buf);

    // Input box
    let input_text = format!(" {}", state.query_input);
    let input = Paragraph::new(input_text)
        .block(
            Block::bordered()
                .title(" Enter Query ")
                .border_type(BorderType::Rounded)
                .style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .style(Style::default().fg(Color::White));
    input.render(chunks[1], buf);

    // Render cursor
    let cursor_x = chunks[1].x + 1 + state.query_input.len() as u16 + 1;
    let cursor_y = chunks[1].y + 1;
    if cursor_x < chunks[1].x + chunks[1].width - 1 {
        if let Some(cell) = buf.cell_mut((cursor_x, cursor_y)) {
            cell.set_char('█');
            cell.set_fg(Color::Yellow);
        }
    }

    // Help text
    let help_lines = vec![
        Line::from(vec![Span::styled(
            "Query by:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Green)),
            Span::styled("Process name: ", Style::default().fg(Color::Yellow)),
            Span::raw("nginx"),
        ]),
        Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Green)),
            Span::styled("PID: ", Style::default().fg(Color::Yellow)),
            Span::raw("12345"),
        ]),
        Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::Green)),
            Span::styled("Port: ", Style::default().fg(Color::Yellow)),
            Span::raw(":8080"),
            Span::styled(" (may need root)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Note: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Port queries for root processes require sudo",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("History: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{} queries", state.query_history.len())),
        ]),
    ];

    let help = Paragraph::new(help_lines).block(
        Block::bordered()
            .title(" Help ")
            .border_type(BorderType::Rounded)
            .style(Style::default().fg(Color::DarkGray)),
    );
    help.render(chunks[2], buf);

    // Status bar
    let status_text = "[Enter] Search  [↑/↓] History  [Esc] Exit";
    let status = Paragraph::new(status_text)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::DarkGray)),
        )
        .centered()
        .style(Style::default().fg(Color::Gray));
    status.render(chunks[3], buf);
}

/// Render results mode (results list + details)
fn render_results_mode(state: &ProcessTracerState, area: Rect, buf: &mut Buffer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(10), // Results list
            Constraint::Min(15),    // Details panel (scrollable)
            Constraint::Length(3),  // Status bar
        ])
        .split(area);

    // Header
    let result_count = state.query_results.len();
    let title = format!(
        " Results: {} match{} ",
        result_count,
        if result_count == 1 { "" } else { "es" }
    );
    let header = Block::bordered()
        .title(title)
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Cyan));
    header.render(chunks[0], buf);

    // Results list
    render_results_list(state, chunks[1], buf);

    // Details panel
    render_detailed_analysis(state, chunks[2], buf);

    // Status bar
    let status_text =
        "[j/k] Navigate  [/] New Query  [l] logs  [r] restart  [x] kill  [c] cd  [Esc] Back";
    let status = Paragraph::new(status_text)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::DarkGray)),
        )
        .centered()
        .style(Style::default().fg(Color::Gray));
    status.render(chunks[3], buf);
}

/// Render results list
fn render_results_list(state: &ProcessTracerState, area: Rect, buf: &mut Buffer) {
    if state.query_results.is_empty() {
        let msg = Paragraph::new("No processes found")
            .block(
                Block::bordered()
                    .title(" Results ")
                    .border_type(BorderType::Rounded),
            )
            .centered()
            .style(Style::default().fg(Color::DarkGray));
        msg.render(area, buf);
        return;
    }

    let items: Vec<ListItem> = state
        .query_results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            let proc = &result.process;
            let supervisor_str = match &proc.supervisor {
                Supervisor::Systemd { unit } => format!("systemd: {}", unit),
                Supervisor::Docker { container_id } => format!("docker: {}", container_id),
                Supervisor::Shell => "shell".to_string(),
                Supervisor::Unknown => "unknown".to_string(),
            };

            let is_selected = idx == state.selected_result;
            let symbol = if is_selected { "▶ " } else { "  " };

            let line = Line::from(vec![
                Span::raw(symbol),
                Span::styled(
                    &proc.name,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::raw(format!(" (PID {}) - ", proc.pid)),
                Span::styled(supervisor_str, Style::default().fg(Color::Cyan)),
            ]);

            ListItem::new(line).style(if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            })
        })
        .collect();

    let list = List::new(items).block(
        Block::bordered()
            .title(" Results ")
            .border_type(BorderType::Rounded),
    );

    Widget::render(list, area, buf);
}

/// Render detailed analysis panel
fn render_detailed_analysis(state: &ProcessTracerState, area: Rect, buf: &mut Buffer) {
    let result = match state.get_selected_result() {
        Some(r) => r,
        None => {
            let msg = Paragraph::new("No result selected")
                .block(
                    Block::bordered()
                        .title(" Details ")
                        .border_type(BorderType::Rounded),
                )
                .centered()
                .style(Style::default().fg(Color::DarkGray));
            msg.render(area, buf);
            return;
        }
    };

    let mut lines = Vec::new();

    // === PROCESS ===
    lines.push(Line::from(vec![Span::styled(
        "=== PROCESS ===",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled("Name:    ", Style::default().fg(Color::Cyan)),
        Span::raw(&result.process.name),
    ]));
    lines.push(Line::from(vec![
        Span::styled("PID:     ", Style::default().fg(Color::Cyan)),
        Span::raw(result.process.pid.to_string()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("User:    ", Style::default().fg(Color::Cyan)),
        Span::raw(&result.process.user),
    ]));

    let cmdline = if result.process.cmdline.is_empty() {
        format!("[{}]", result.process.name)
    } else {
        result.process.cmdline.join(" ")
    };
    lines.push(Line::from(vec![
        Span::styled("Command: ", Style::default().fg(Color::Cyan)),
        Span::raw(&cmdline),
    ]));

    if let Some(ref cwd) = result.working_directory {
        lines.push(Line::from(vec![
            Span::styled("CWD:     ", Style::default().fg(Color::Cyan)),
            Span::raw(cwd),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("Uptime:  ", Style::default().fg(Color::Cyan)),
        Span::raw(result.process.uptime_str()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Memory:  ", Style::default().fg(Color::Cyan)),
        Span::raw(result.process.memory_str()),
    ]));

    lines.push(Line::from(""));

    // === ANCESTOR CHAIN ===
    if !result.ancestor_chain.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "=== ANCESTOR CHAIN ===",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        for (idx, ancestor) in result.ancestor_chain.iter().enumerate() {
            let supervisor_str = match &ancestor.supervisor {
                Supervisor::Systemd { unit } => format!(" (Systemd: {})", unit),
                Supervisor::Docker { container_id } => format!(" (Docker: {})", container_id),
                Supervisor::Shell => " (Shell)".to_string(),
                Supervisor::Unknown => "".to_string(),
            };

            let prefix = if idx == 0 {
                "".to_string()
            } else {
                format!("{}", "  ".repeat(idx - 1) + "└─ ")
            };

            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    format!("PID {}", ancestor.pid),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(" "),
                Span::styled(&ancestor.name, Style::default().fg(Color::Yellow)),
                Span::styled(supervisor_str, Style::default().fg(Color::Cyan)),
            ]));
        }

        lines.push(Line::from(""));
    }

    // === NETWORK ===
    if !result.network_bindings.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "=== NETWORK ===",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        for binding in &result.network_bindings {
            let protocol_str = match binding.protocol {
                Protocol::Tcp => "TCP",
                Protocol::Udp => "UDP",
            };

            let state_str = match binding.state {
                ConnectionState::Listen => "[LISTEN]",
                ConnectionState::Established => "[ESTABLISHED]",
                ConnectionState::TimeWait => "[TIME_WAIT]",
                ConnectionState::CloseWait => "[CLOSE_WAIT]",
                ConnectionState::Unknown => "[UNKNOWN]",
            };

            let is_public = binding.local_addr.to_string() == "0.0.0.0"
                || binding.local_addr.to_string() == "::";

            let warning = if is_public && binding.state == ConnectionState::Listen {
                Span::styled(" ⚠ PUBLIC", Style::default().fg(Color::Red))
            } else {
                Span::raw("")
            };

            let remote_str = if let Some(ref remote_addr) = binding.remote_addr {
                if let Some(remote_port) = binding.remote_port {
                    format!(" → {}:{}", remote_addr, remote_port)
                } else {
                    format!(" → {}", remote_addr)
                }
            } else {
                String::new()
            };

            lines.push(Line::from(vec![
                Span::raw(format!(
                    "{} {}:{} {}",
                    protocol_str, binding.local_addr, binding.local_port, state_str
                )),
                Span::raw(remote_str),
                warning,
            ]));
        }

        lines.push(Line::from(""));
    }

    // === SYSTEMD ===
    if let Some(ref systemd) = result.systemd_metadata {
        lines.push(Line::from(vec![Span::styled(
            "=== SYSTEMD ===",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::styled("Unit:        ", Style::default().fg(Color::Cyan)),
            Span::raw(&systemd.unit_name),
        ]));

        if let Some(ref desc) = systemd.description {
            lines.push(Line::from(vec![
                Span::styled("Description: ", Style::default().fg(Color::Cyan)),
                Span::raw(desc),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled("State:       ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}/{}", systemd.active_state, systemd.sub_state)),
        ]));

        if let Some(ref restart) = systemd.restart_policy {
            lines.push(Line::from(vec![
                Span::styled("Restart:     ", Style::default().fg(Color::Cyan)),
                Span::raw(restart),
            ]));
        }

        if let Some(ref exec_start) = systemd.exec_start {
            lines.push(Line::from(vec![
                Span::styled("ExecStart:   ", Style::default().fg(Color::Cyan)),
                Span::raw(exec_start),
            ]));
        }

        lines.push(Line::from(""));
    }

    // === ENVIRONMENT ===
    if !result.environment.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "=== ENVIRONMENT ===",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        let mut env_vars: Vec<_> = result.environment.iter().collect();
        env_vars.sort_by_key(|(k, _)| *k);

        for (key, value) in env_vars {
            lines.push(Line::from(vec![
                Span::styled(key, Style::default().fg(Color::Green)),
                Span::raw("="),
                Span::raw(value),
            ]));
        }

        lines.push(Line::from(""));
    }

    // === WARNINGS ===
    if !result.process.warnings.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "=== WARNINGS ===",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        for warning in &result.process.warnings {
            lines.push(Line::from(vec![
                Span::styled(warning.symbol(), Style::default().fg(warning.color())),
                Span::raw(" "),
                Span::styled(warning.description(), Style::default().fg(Color::Gray)),
            ]));
        }

        lines.push(Line::from(""));
    }

    // Apply scroll offset
    let total_lines = lines.len(); // Save before consuming
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(state.scroll_offset as usize)
        .collect();

    let details = Paragraph::new(visible_lines).block(
        Block::bordered()
            .title(format!(
                " Details - {} (PID {}) ",
                result.process.name, result.process.pid
            ))
            .border_type(BorderType::Rounded),
    );

    details.render(area, buf);

    // Render scrollbar if needed
    let visible_height = area.height.saturating_sub(2) as usize;
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(total_lines)
            .position(state.scroll_offset as usize);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));

        StatefulWidget::render(scrollbar, area, buf, &mut scrollbar_state);
    }
}

/// Render notification popup
fn render_notification(message: &str, area: Rect, buf: &mut Buffer) {
    let notification_width = (message.len() + 4).min(area.width as usize) as u16;
    let notification_height = 3;

    let x = area.width.saturating_sub(notification_width) / 2;
    let y = area.height.saturating_sub(notification_height + 2);

    let notification_area = Rect {
        x: area.x + x,
        y: area.y + y,
        width: notification_width,
        height: notification_height,
    };

    // Clear background
    for py in notification_area.y..notification_area.y + notification_area.height {
        for px in notification_area.x..notification_area.x + notification_area.width {
            if let Some(cell) = buf.cell_mut((px, py)) {
                cell.set_char(' ');
                cell.set_bg(Color::Black);
            }
        }
    }

    let notification = Paragraph::new(message)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::Green).bg(Color::Black)),
        )
        .centered()
        .style(Style::default().fg(Color::Green).bg(Color::Black));

    notification.render(notification_area, buf);
}
