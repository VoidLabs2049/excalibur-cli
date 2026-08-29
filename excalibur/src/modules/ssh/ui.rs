use super::effective::Effective;
use super::form::{Change, Editing, Field, ForwardField, ForwardForm, HostForm};
use super::probe::{Health, Light};
use super::sshconfig::HostBlock;
use super::state::{MENU, Screen, SshState};
use super::tunnels::Forward;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, List, ListItem, ListState, Paragraph, StatefulWidget, Widget, Wrap,
    },
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
        Screen::Forward => render_forward(state, chunks[1], buf),
        Screen::Dashboard => render_dashboard(state, chunks[1], buf),
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
        Screen::Forward
            if matches!(
                state.forward_form.as_ref().map(|f| &f.editing),
                Some(Some(Editing::Text(_)))
            ) =>
        {
            " Enter: accept   Ctrl+U: clear   Esc: cancel".to_string()
        }
        // Naming a group that does not exist yet is the only way to create one,
        // and it goes through Tab. The placeholder cannot say so -- the group
        // field is never empty -- so the hint has to live here.
        Screen::Forward
            if matches!(
                state.forward_form.as_ref().map(|f| &f.editing),
                Some(Some(Editing::Pick { .. }))
            ) =>
        {
            " j/k: choose   Enter: accept   Tab: type a new one   Esc: cancel".to_string()
        }
        Screen::Forward if state.forward_form.is_some() => {
            " j/k: field   Enter: edit   Ctrl+S: save   Esc: close".to_string()
        }
        Screen::Forward => {
            " j/k: navigate   Enter: edit   n: new   c: clone   d: delete   Esc: back   q: quit"
                .to_string()
        }
        // `s`/`S` act on the marks when there are any and on the cursor when
        // there are not. That is one key meaning two things, so the scope is
        // spelled out here rather than left to be remembered.
        Screen::Dashboard if !state.marked.is_empty() => {
            let n = state.marked.len();
            format!(
                " s: start {n} marked   S: stop {n} marked   Space/g: mark   u: clear   \
                 Enter: this one   r: refresh"
            )
        }
        // An orphan cannot be marked or started -- there is no rule to do either
        // to -- so say what the one key that does work will do.
        Screen::Dashboard if state.selected_orphan().is_some() => {
            " unclaimed process   Enter: stop it   j/k: navigate   r: refresh   Esc: back"
                .to_string()
        }
        Screen::Dashboard => {
            " j/k: navigate   Enter: start/stop   Space: mark   a: start all   A: stop all   r: refresh   Esc: back"
                .to_string()
        }
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

    Widget::render(
        List::new(items).block(
            Block::bordered()
                .title(" Menu ")
                .border_type(BorderType::Rounded),
        ),
        panes[0],
        buf,
    );

    let entry = state.menu_entry();
    let body = match entry.screen {
        Screen::Config => config_summary(state),
        Screen::Forward => forward_summary(state),
        Screen::Dashboard => dashboard_summary(state),
        // MENU never lists itself, so this is unreachable in practice.
        Screen::Menu => Vec::new(),
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

fn forward_summary(state: &SshState) -> Vec<Line<'static>> {
    if let Some(error) = &state.tunnels_error {
        return vec![Line::from(Span::styled(
            format!("  {error}"),
            Style::default().fg(Color::Red),
        ))];
    }
    if state.tunnels.profiles.is_empty() {
        return vec![Line::from(Span::styled(
            "  no profiles yet",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "  {} profiles   {} forwards",
                state.tunnels.profiles.len(),
                state.tunnels.count()
            ),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for profile in &state.tunnels.profiles {
        lines.push(Line::from(Span::styled(
            format!("  {}", profile.name),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for forward in &profile.forwards {
            lines.push(Line::from(Span::styled(
                format!("    {}", forward.summary()),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    lines
}

fn dashboard_summary(state: &SshState) -> Vec<Line<'static>> {
    let orphans = state.orphans();
    if state.tunnels.count() == 0 && orphans.is_empty() {
        return vec![Line::from(Span::styled(
            "  no forwards defined yet",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    let mut lines = vec![
        Line::from(Span::styled(
            format!("  {} of {} up", state.live_count(), state.tunnels.count()),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for slot in state.tunnels.all() {
        let Some(forward) = state.tunnels.get(slot.0, slot.1) else {
            continue;
        };
        let health = state.health_at(slot);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", health.lights()),
                Style::default().fg(light_colour(&health)),
            ),
            Span::raw(forward.summary()),
        ]));
    }
    // The preview is where the dashboard is picked from, so a port held by
    // something the rules do not mention has to be visible before opening it.
    if !orphans.is_empty() {
        lines.push(Line::from(""));
        for running in orphans {
            lines.push(Line::from(Span::styled(
                format!("  ?      {}   unclaimed", running.summary()),
                Style::default().fg(Color::Yellow),
            )));
        }
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

/// Rendered stateful so the list scrolls to the highlighted row. With a plain
/// `List` the selection silently disappears once the options outrun the pane --
/// 35 host aliases into three visible rows.
fn render_candidates(options: &[String], index: usize, area: Rect, buf: &mut Buffer) {
    let items: Vec<ListItem> = options
        .iter()
        .map(|option| {
            let text = if option.is_empty() {
                "(none)"
            } else {
                option.as_str()
            };
            ListItem::new(Line::from(format!("  {text}")))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(format!(
                    " {}/{}  j/k choose · Enter accept · i type · Esc cancel ",
                    index + 1,
                    options.len()
                ))
                .border_type(BorderType::Rounded),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let mut list_state = ListState::default();
    list_state.select(Some(index));
    StatefulWidget::render(list, area, buf, &mut list_state);
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

    Widget::render(List::new(items).block(block), area, buf);
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

fn render_forward(state: &SshState, area: Rect, buf: &mut Buffer) {
    let panes = split_panes(area);
    render_forward_list(state, panes[0], buf);
    match &state.forward_form {
        Some(form) => render_forward_form(form, panes[1], buf),
        None => render_forward_detail(state, panes[1], buf),
    }
}

fn render_forward_form(form: &ForwardForm, area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .title(if form.index.is_some() {
            " Edit forward "
        } else {
            " New forward "
        })
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ForwardField::ALL.len() as u16),
            Constraint::Length(1),
            Constraint::Min(3),
        ])
        .split(inner);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); ForwardField::ALL.len()])
        .split(chunks[0]);

    for (i, field) in ForwardField::ALL.iter().enumerate() {
        let selected = i == form.cursor;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(14), Constraint::Min(0)])
            .split(rows[i]);

        let label_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Paragraph::new(format!(
            "{} {}",
            if selected { ">" } else { " " },
            field.label()
        ))
        .style(label_style)
        .render(columns[0], buf);

        if selected && let Some(Editing::Text(textarea)) = &form.editing {
            Widget::render(&**textarea, columns[1], buf);
            continue;
        }
        let value = &form.values[i];
        let (text, style) = if value.is_empty() {
            (
                field.placeholder().to_string(),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (value.clone(), Style::default())
        };
        Paragraph::new(text).style(style).render(columns[1], buf);
    }

    match &form.editing {
        Some(Editing::Pick { options, index }) => {
            render_candidates(options, *index, chunks[2], buf)
        }
        // The diagram is live: flipping the direction swaps which side listens,
        // which is the field people get wrong.
        _ => Paragraph::new(forward_explainer(&form.to_forward()))
            .wrap(Wrap { trim: false })
            .render(chunks[2], buf),
    }
}

/// Why the rule cannot run, if it cannot, then the direction diagram, then the
/// command. The problem comes first because it is the only line that must not
/// be the one clipped when the pane is short.
fn forward_explainer(forward: &Forward) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(problem) = forward.problem() {
        lines.push(Line::from(Span::styled(
            format!("  {problem}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    } else if let Some(port) = forward.privileged_bind() {
        lines.push(Line::from(Span::styled(
            format!("  port {port} is below 1024 -- binding it needs root"),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    let (listen, exit) = forward.explain();
    lines.push(Line::from(Span::styled(
        format!("  {listen}"),
        Style::default().fg(Color::Green),
    )));
    lines.push(Line::from(Span::styled(
        format!("  {exit}"),
        Style::default().fg(Color::Yellow),
    )));

    if forward.problem().is_none() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", forward.command_line()),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn render_forward_list(state: &SshState, area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .title(format!(" Forwards ({}) ", state.tunnels.count()))
        .border_type(BorderType::Rounded);

    if let Some(error) = &state.tunnels_error {
        Paragraph::new(format!("  {error}"))
            .block(block)
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: false })
            .render(area, buf);
        return;
    }

    // The block is applied by the List, so its inner width is two less than the pane.
    let width = area.width.saturating_sub(2) as usize;
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item = None;
    let mut row = 0;
    for (index, profile) in state.tunnels.profiles.iter().enumerate() {
        items.push(profile_heading(state, index, &profile.name, width));
        for forward in &profile.forwards {
            let selected = row == state.forward_index;
            if selected {
                selected_item = Some(items.len());
            }
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let summary = Line::from(format!("   {}", forward.summary()));
            items.push(ListItem::new(rule_lines(summary, &forward.note, 5)).style(style));
            row += 1;
        }
    }

    if items.is_empty() {
        let path = super::tunnels::Tunnels::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.config/excalibur/tunnels.yaml".to_string());
        Paragraph::new(format!(
            "  no profiles yet\n\n  will be read from\n  {path}"
        ))
        .block(block)
        .style(Style::default().fg(Color::DarkGray))
        .wrap(Wrap { trim: false })
        .render(area, buf);
        return;
    }

    // Stateful so the selection scrolls into view: with notes the rows are twice
    // as tall, so a plain List drops the highlight off the bottom much sooner.
    let mut list_state = ListState::default();
    list_state.select(selected_item);
    StatefulWidget::render(List::new(items).block(block), area, buf, &mut list_state);
}

fn render_forward_detail(state: &SshState, area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .title(" Detail ")
        .border_type(BorderType::Rounded);

    let Some(forward) = state.selected_forward() else {
        Paragraph::new(EMPTY_FORWARD_HELP)
            .block(block)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false })
            .render(area, buf);
        return;
    };

    let (listen, exit) = forward.explain();
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  direction   "),
            Span::styled(
                format!("{}  ({})", forward.kind.label(), forward.kind.flag()),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {listen}"),
            Style::default().fg(Color::Green),
        )),
        Line::from(Span::styled(
            format!("  {exit}"),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
    ];

    if !forward.note.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", forward.note),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    if !state.config.hosts.iter().any(|h| h.alias() == forward.host) {
        lines.push(Line::from(Span::styled(
            format!("  `{}` is not a host in ~/.ssh/config", forward.host),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        format!("  {}", forward.command_line()),
        Style::default().fg(Color::DarkGray),
    )));

    Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

const EMPTY_FORWARD_HELP: &str = "\
  A forward describes one tunnel.

    profiles:
      - name: daily
        forwards:
          - host: kami          # the machine ssh connects to
            kind: local         # local (-L) or remote (-R)
            bind: 29001         # port opened on this machine
            target: 10.0.0.5:9001
            note: reached through kami, not from here

  local   opens the port here; the exit is resolved from the far side,
          so it can name anything that host can reach
  remote  opens the port on the far side; the exit is resolved here";

fn render_dashboard(state: &SshState, area: Rect, buf: &mut Buffer) {
    let orphans = state.orphans();
    let unclaimed = match orphans.len() {
        0 => String::new(),
        n => format!("  +{n} unclaimed"),
    };
    let block = Block::bordered()
        .title(format!(
            " Tunnels   {} of {} up{unclaimed} ",
            state.live_count(),
            state.tunnels.count()
        ))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    block.render(area, buf);

    // An empty tunnels.yaml with tunnels running by hand is exactly the case
    // the orphan list is for, so the placeholder must not hide it.
    if state.tunnels.count() == 0 && orphans.is_empty() {
        Paragraph::new("  no forwards defined -- add some under `Edit tunnel profiles`")
            .style(Style::default().fg(Color::DarkGray))
            .render(inner, buf);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(inner);

    // Rules are grouped under their profile, so the item index is no longer the
    // rule index -- headings and notes both take rows. The selected item is
    // recorded while building instead of being computed back out.
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item = None;
    let mut row = 0;
    for (index, profile) in state.tunnels.profiles.iter().enumerate() {
        items.push(profile_heading(
            state,
            index,
            &profile.name,
            chunks[0].width as usize,
        ));
        for (within, forward) in profile.forwards.iter().enumerate() {
            let slot = (index, within);
            let health = state.health_at(slot);
            if row == state.forward_index {
                selected_item = Some(items.len());
            }

            let broken = forward.problem().is_some();
            let mut line = vec![
                // `+` rather than `*`: the lights already use `*` for "up", and
                // a marker sharing that symbol reads as a fourth light.
                Span::styled(
                    if state.marked.contains(&slot) {
                        " + "
                    } else {
                        "   "
                    },
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    format!("{}  ", health.lights()),
                    Style::default().fg(if broken {
                        Color::Red
                    } else {
                        light_colour(&health)
                    }),
                ),
                Span::raw(format!("{:<34}", truncate(&forward.summary(), 34))),
            ];
            line.push(match (broken, state.pid_at(slot)) {
                (true, _) => Span::styled("incomplete", Style::default().fg(Color::Red)),
                (false, Some(pid)) => {
                    Span::styled(format!("pid {pid}"), Style::default().fg(Color::DarkGray))
                }
                (false, None) => Span::styled("stopped", Style::default().fg(Color::DarkGray)),
            });

            let mut style = Style::default();
            if row == state.forward_index {
                style = style.add_modifier(Modifier::REVERSED);
            }
            // 10 = 3 of indent, 5 of lights, 2 of gap.
            items.push(ListItem::new(rule_lines(Line::from(line), &forward.note, 10)).style(style));
            row += 1;
        }
    }

    if !orphans.is_empty() {
        items.push(orphan_heading(orphans.len(), chunks[0].width as usize));
        for running in &orphans {
            if row == state.forward_index {
                selected_item = Some(items.len());
            }
            let mut style = Style::default();
            if row == state.forward_index {
                style = style.add_modifier(Modifier::REVERSED);
            }
            // Deliberately not the three lights: they belong to a rule, and only
            // the first of the three could be answered here anyway. `?` keeps the
            // column width so the summary and the pid stay in line.
            items.push(
                ListItem::new(Line::from(vec![
                    Span::styled("   ?      ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!("{:<34}", truncate(&running.summary(), 34))),
                    Span::styled(
                        format!("pid {}", running.pid),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
                .style(style),
            );
            row += 1;
        }
    }

    let mut list_state = ListState::default();
    list_state.select(selected_item);
    StatefulWidget::render(List::new(items), chunks[0], buf, &mut list_state);

    // The legend earns its space: the three lights only mean anything if you
    // know which layer each one is.
    // A rule that cannot be built never gets a process, so its lights stay dark
    // and the reason has to come from the rule itself.
    let detail = match state.selected_orphan() {
        // The whole point of the row: it holds a port and no rule says so.
        Some(_) => "no rule describes this -- a rule edited while running leaves \
                    its process here"
            .to_string(),
        None => state
            .selected_slot()
            .and_then(|slot| {
                let forward = state.tunnels.get(slot.0, slot.1)?;
                Some(
                    forward
                        .problem()
                        .unwrap_or_else(|| state.health_at(slot).detail),
                )
            })
            .unwrap_or_default(),
    };
    let legend = vec![
        Line::from(Span::styled(
            "  process / listening / path        * up   x failed   - not observable   o stopped",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format!("  {detail}"),
            Style::default().fg(Color::Yellow),
        )),
    ];
    Paragraph::new(legend)
        .wrap(Wrap { trim: false })
        .render(chunks[1], buf);
}

/// Where a dashboard rule's own status starts: 3 of indent, 5 of lights, 2 of
/// gap, 34 of summary.
const STATUS_COLUMN: usize = 44;

/// A profile heading with how many of its rules are up.
///
/// The heading is not selectable -- the cursor still walks rules only -- so the
/// one the cursor sits under is highlighted instead, which is what says which
/// group a group action would hit.
///
/// `width` is the pane's inner width. The count lines up with each rule's own
/// status where there is room, and slides left where there is not -- the
/// forward pane is 40% wide and would otherwise clip it away entirely.
fn profile_heading(state: &SshState, index: usize, name: &str, width: usize) -> ListItem<'static> {
    let (live, total) = state.profile_status(index);
    let here = state.selected_profile() == Some(index);
    let mut style = Style::default().add_modifier(Modifier::BOLD);
    if here {
        style = style.fg(Color::Cyan);
    }
    let count = format!("{live}/{total} up");
    // Less one for the leading space, which is part of the same column budget.
    let pad = (STATUS_COLUMN - 1).min(width.saturating_sub(count.chars().count() + 1));
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {:<pad$}", truncate(name, pad)), style),
        Span::styled(
            count,
            Style::default().fg(if live == total && total > 0 {
                Color::Green
            } else if live == 0 {
                Color::DarkGray
            } else {
                Color::Yellow
            }),
        ),
    ]))
}

/// Heading for the processes no rule claims.
///
/// Yellow rather than red: an orphan is not necessarily wrong -- a tunnel
/// started by hand is one too -- it is just unaccounted for.
fn orphan_heading(count: usize, width: usize) -> ListItem<'static> {
    let note = format!("{count} not in any rule");
    let pad = (STATUS_COLUMN - 1).min(width.saturating_sub(note.chars().count() + 1));
    ListItem::new(Line::from(vec![
        Span::styled(
            format!(" {:<pad$}", truncate("unclaimed", pad)),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(note, Style::default().fg(Color::Yellow)),
    ]))
}

/// The rule row, with its note underneath when it has one.
///
/// The note is usually the only thing that says what a rule is for, and the
/// summary line has no room left for it. `indent` lines the note up with the
/// start of the summary text, which differs by screen because the dashboard
/// carries the three lights in front of it.
fn rule_lines(summary: Line<'static>, note: &str, indent: usize) -> Vec<Line<'static>> {
    if note.is_empty() {
        return vec![summary];
    }
    vec![
        summary,
        Line::from(Span::styled(
            format!("{:indent$}{note}", "", indent = indent),
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

fn light_colour(health: &Health) -> Color {
    if health.process == Light::Off {
        Color::DarkGray
    } else if health.port == Light::Bad || health.path == Light::Bad {
        Color::Red
    } else if health.port == Light::Ok && health.path == Light::Ok {
        Color::Green
    } else {
        Color::Yellow
    }
}
