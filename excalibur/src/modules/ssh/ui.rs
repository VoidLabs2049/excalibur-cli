use super::effective::Effective;
use super::form::{Change, Editing, Field, HostForm};
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
    if let Some((message, _)) = &state.notification {
        Paragraph::new(format!(" {message}"))
            .block(Block::bordered().border_type(BorderType::Rounded))
            .style(Style::default().fg(Color::Yellow))
            .render(area, buf);
        return;
    }

    let help = match state.screen {
        Screen::Menu => " j/k: navigate   Enter: open   q: back to main menu".to_string(),
        Screen::Config if state.searching => {
            format!(
                " /{}_   Enter: keep filter   Esc: clear",
                state.search_query
            )
        }
        // The text box opens on the current value with the cursor at its end,
        // so typing appends. Ctrl+U is what makes replacing it discoverable.
        Screen::Config
            if matches!(
                state.form.as_ref().map(|f| &f.editing),
                Some(Some(Editing::Text(_)))
            ) =>
        {
            " Enter: accept   Ctrl+U: clear   Ctrl+W: delete word   Esc: cancel".to_string()
        }
        Screen::Config if state.form.is_some() => {
            let read_only = if state.config.read_only {
                "   [read-only: saving is refused]"
            } else {
                ""
            };
            format!(" j/k: field   Enter: edit   Ctrl+S: save   Esc: close{read_only}")
        }
        Screen::Config => {
            let filter = if state.search_query.is_empty() {
                String::new()
            } else {
                format!("   filter: /{}", state.search_query)
            };
            format!(
                " j/k: navigate   Enter: edit   g: ssh -G   /: search   Esc: back   q: quit{filter}"
            )
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
    match (&state.form, &state.effective) {
        (Some(form), _) => render_form(state, form, panes[1], buf),
        (None, Some((alias, resolved))) => render_effective(state, alias, resolved, panes[1], buf),
        (None, None) => render_host_preview(state, panes[1], buf),
    }
}

/// Compare what the block says against what `ssh -G` resolves. A directive can
/// be present and inert -- an earlier matching block already set that keyword --
/// and this is the only place that difference becomes visible.
fn render_effective(
    state: &SshState,
    alias: &str,
    resolved: &Effective,
    area: Rect,
    buf: &mut Buffer,
) {
    let mut lines: Vec<Line> = Vec::new();

    if let Some(error) = &resolved.error {
        for line in error.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(Color::Red),
            )));
        }
        lines.push(Line::from(""));
    }

    match state.selected_host() {
        Some(host) if !host.directives.is_empty() => {
            let mut overridden = 0;
            for directive in &host.directives {
                // Status goes first: values vary wildly in length (a
                // ProxyCommand wraps several lines) so a trailing column would
                // not line up.
                if resolved.agrees(&directive.key, &directive.value) {
                    lines.push(Line::from(vec![
                        Span::styled("  [ok] ", Style::default().fg(Color::Green)),
                        Span::raw(format!("{} {}", directive.key, directive.value)),
                    ]));
                } else {
                    overridden += 1;
                    let actual = resolved
                        .first(&directive.key)
                        .map(|v| format!("   ssh uses: {v}"))
                        .unwrap_or_default();
                    lines.push(Line::from(vec![
                        Span::styled("  [--] ", Style::default().fg(Color::Red)),
                        Span::styled(
                            format!("{} {}", directive.key, directive.value),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(actual, Style::default().fg(Color::Red)),
                    ]));
                }
            }
            // A duplicated block whose values happen to match still reports
            // every row as "in effect" -- true of the value, but not of this
            // block. Say so, or the rows read as a clean bill of health.
            let shadow = state.shadowing_line(host);
            if shadow.is_some() || overridden > 0 {
                lines.push(Line::from(""));
                let note = match (shadow, overridden) {
                    (Some(line), 0) => {
                        format!("  line {line} sets the same values first; this block does nothing")
                    }
                    (Some(line), n) => format!("  {n} ignored; line {line} sets them first"),
                    (None, n) => format!("  {n} ignored by an earlier matching block"),
                };
                lines.push(Line::from(Span::styled(
                    note,
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        _ => lines.push(Line::from(Span::styled(
            "  this block sets nothing",
            Style::default().fg(Color::DarkGray),
        ))),
    }

    Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(format!(" ssh -G {alias} "))
                .border_type(BorderType::Rounded),
        )
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

fn render_form(state: &SshState, form: &HostForm, area: Rect, buf: &mut Buffer) {
    let title = format!(
        " {}{} ",
        form.value(Field::Alias),
        if form.is_dirty() { "  *" } else { "" }
    );
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(Field::ALL.len() as u16),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .split(inner);

    render_fields(form, chunks[0], buf);

    match &form.editing {
        Some(Editing::Pick { options, index }) => {
            render_candidates(options, *index, chunks[2], buf)
        }
        _ => render_form_footer(state, form, chunks[2], buf),
    }
}

fn render_fields(form: &HostForm, area: Rect, buf: &mut Buffer) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); Field::ALL.len()])
        .split(area);

    for (i, field) in Field::ALL.iter().enumerate() {
        let selected = i == form.cursor;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(16), Constraint::Min(0)])
            .split(rows[i]);

        let label = format!("{} {}", if selected { ">" } else { " " }, field.label());
        let label_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Paragraph::new(label)
            .style(label_style)
            .render(columns[0], buf);

        if selected && let Some(Editing::Text(textarea)) = &form.editing {
            Widget::render(&**textarea, columns[1], buf);
            continue;
        }

        let value = &form.values[i];
        let locked = *field == Field::Alias && form.alias_locked;
        let (text, style) = if locked {
            (
                format!("{value}   (multiple patterns)"),
                Style::default().fg(Color::DarkGray),
            )
        } else if value.is_empty() {
            ("(none)".to_string(), Style::default().fg(Color::DarkGray))
        } else {
            (value.clone(), Style::default())
        };
        Paragraph::new(text).style(style).render(columns[1], buf);
    }
}

fn render_candidates(options: &[String], index: usize, area: Rect, buf: &mut Buffer) {
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, option)| {
            let text = if option.is_empty() {
                "(none)".to_string()
            } else {
                option.clone()
            };
            let style = if i == index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(format!("  {text}"))).style(style)
        })
        .collect();

    List::new(items)
        .block(
            Block::bordered()
                .title(" j/k choose · Enter accept · i type · Esc cancel ")
                .border_type(BorderType::Rounded),
        )
        .render(area, buf);
}

fn render_form_footer(state: &SshState, form: &HostForm, area: Rect, buf: &mut Buffer) {
    let mut lines: Vec<Line> = Vec::new();

    if !form.other.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  kept as written ({})", form.other.len()),
            Style::default().fg(Color::DarkGray),
        )));
        for directive in &form.other {
            lines.push(Line::from(Span::styled(
                format!("    {directive}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
    }

    let plan = form.plan(&state.config);
    if plan.changes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no changes",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  will write",
            Style::default().fg(Color::Yellow),
        )));
        for change in &plan.changes {
            match change {
                Change::Replaced {
                    line,
                    before,
                    after,
                } => {
                    lines.push(diff_line('-', line + 1, before, Color::Red));
                    lines.push(diff_line('+', line + 1, after, Color::Green));
                }
                Change::Removed { line, before } => {
                    lines.push(diff_line('-', line + 1, before, Color::Red))
                }
                Change::Inserted { line, after } => {
                    lines.push(diff_line('+', line + 1, after, Color::Green))
                }
            }
        }
    }

    Paragraph::new(lines).render(area, buf);
}

fn diff_line(sign: char, number: usize, text: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {sign} {number:<4}{text}"),
        Style::default().fg(color),
    ))
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

            Some(
                ListItem::new(Line::from(row_text(marker, host.alias(), &note, width)))
                    .style(style),
            )
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
    let hostname = host
        .get("HostName")
        .map(|d| d.value.as_str())
        .unwrap_or("-");
    match host.get("Port") {
        Some(port) => format!("{hostname}:{}", port.value),
        None => hostname.to_string(),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
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
