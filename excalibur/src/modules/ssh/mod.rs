mod effective;
mod form;
mod probe;
mod sshconfig;
mod state;
mod supervisor;
mod tunnels;
mod ui;
mod worker;

use super::{Module, ModuleAction, ModuleId, ModuleMetadata};
use color_eyre::Result;
use form::Editing;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
};
use state::{Screen, SshState};

/// Decide first, act second: the picker's own state is borrowed while it is
/// being read, so commit/cancel cannot run inside the match that reads it.
enum Then {
    Nothing,
    Commit,
    Cancel,
    AsText,
}

#[derive(Debug)]
pub struct SshModule {
    state: SshState,
}

impl SshModule {
    pub fn new() -> Self {
        Self {
            state: SshState::new(),
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Ok(ModuleAction::Exit),
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.menu_previous();
                Ok(ModuleAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.menu_next();
                Ok(ModuleAction::None)
            }
            KeyCode::Enter => {
                self.state.screen = self.state.menu_entry().screen;
                Ok(ModuleAction::None)
            }
            _ => Ok(ModuleAction::None),
        }
    }

    fn handle_config_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        // While typing a filter every character belongs to the query, so the
        // usual `q`-quits and `j/k`-navigate bindings must not fire here.
        if self.state.searching {
            match key.code {
                KeyCode::Esc => {
                    self.state.searching = false;
                    self.state.search_query.clear();
                    self.state.apply_filters();
                }
                KeyCode::Enter => self.state.searching = false,
                KeyCode::Backspace => {
                    self.state.search_query.pop();
                    self.state.apply_filters();
                }
                KeyCode::Char(c) => {
                    self.state.search_query.push(c);
                    self.state.apply_filters();
                }
                _ => {}
            }
            return Ok(ModuleAction::None);
        }

        if self.state.form.is_some() {
            return self.handle_form_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.state.screen = Screen::Menu;
                Ok(ModuleAction::None)
            }
            KeyCode::Char('q') => Ok(ModuleAction::Exit),
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.host_previous();
                Ok(ModuleAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.host_next();
                Ok(ModuleAction::None)
            }
            KeyCode::Char('/') => {
                self.state.searching = true;
                Ok(ModuleAction::None)
            }
            KeyCode::Char('g') => {
                self.state.toggle_effective();
                Ok(ModuleAction::None)
            }
            KeyCode::Enter => {
                self.state.open_form();
                Ok(ModuleAction::None)
            }
            _ => Ok(ModuleAction::None),
        }
    }

    fn handle_form_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        if self
            .state
            .form
            .as_ref()
            .is_some_and(|form| form.editing.is_some())
        {
            self.handle_field_edit_key(key);
            return Ok(ModuleAction::None);
        }

        match key.code {
            KeyCode::Esc => self.state.form = None,
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.save_form()
            }
            KeyCode::Enter => self.state.form_begin_edit(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(form) = &mut self.state.form {
                    form.previous_field();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(form) = &mut self.state.form {
                    form.next_field();
                }
            }
            _ => {}
        }
        Ok(ModuleAction::None)
    }

    fn handle_field_edit_key(&mut self, key: KeyEvent) {
        let Some(form) = &mut self.state.form else {
            return;
        };
        let then = match &mut form.editing {
            Some(Editing::Pick { options, index }) => match key.code {
                KeyCode::Esc => Then::Cancel,
                KeyCode::Enter => Then::Commit,
                KeyCode::Tab | KeyCode::Char('i') => Then::AsText,
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = index.saturating_sub(1);
                    Then::Nothing
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(options.len().saturating_sub(1));
                    Then::Nothing
                }
                _ => Then::Nothing,
            },
            Some(Editing::Text(textarea)) => match key.code {
                KeyCode::Esc => Then::Cancel,
                KeyCode::Enter => Then::Commit,
                _ => {
                    textarea.input(key);
                    Then::Nothing
                }
            },
            None => Then::Nothing,
        };

        match then {
            Then::Nothing => {}
            Then::Commit => form.commit_edit(),
            Then::Cancel => form.cancel_edit(),
            Then::AsText => form.edit_as_text(),
        }
    }

    fn handle_forward_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        if self.state.forward_form.is_some() {
            return self.handle_forward_form_key(key);
        }
        match key.code {
            KeyCode::Esc => self.state.screen = Screen::Menu,
            KeyCode::Char('q') => return Ok(ModuleAction::Exit),
            KeyCode::Up | KeyCode::Char('k') => self.state.forward_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.state.forward_next(),
            KeyCode::Enter => self.state.open_forward_form(false),
            KeyCode::Char('n') => self.state.open_forward_form(true),
            KeyCode::Char('c') => self.state.clone_forward(),
            KeyCode::Char('d') => self.state.delete_forward(),
            _ => {}
        }
        Ok(ModuleAction::None)
    }

    fn handle_forward_form_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        if self
            .state
            .forward_form
            .as_ref()
            .is_some_and(|form| form.editing.is_some())
        {
            self.handle_forward_field_key(key);
            return Ok(ModuleAction::None);
        }
        match key.code {
            KeyCode::Esc => self.state.forward_form = None,
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.save_forward_form()
            }
            KeyCode::Enter => self.state.forward_form_begin_edit(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(form) = &mut self.state.forward_form {
                    form.previous_field();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(form) = &mut self.state.forward_form {
                    form.next_field();
                }
            }
            _ => {}
        }
        Ok(ModuleAction::None)
    }

    fn handle_forward_field_key(&mut self, key: KeyEvent) {
        let Some(form) = &mut self.state.forward_form else {
            return;
        };
        let then = match &mut form.editing {
            Some(Editing::Pick { options, index }) => match key.code {
                KeyCode::Esc => Then::Cancel,
                KeyCode::Enter => Then::Commit,
                KeyCode::Tab | KeyCode::Char('i') => Then::AsText,
                KeyCode::Up | KeyCode::Char('k') => {
                    *index = index.saturating_sub(1);
                    Then::Nothing
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(options.len().saturating_sub(1));
                    Then::Nothing
                }
                _ => Then::Nothing,
            },
            Some(Editing::Text(textarea)) => match key.code {
                KeyCode::Esc => Then::Cancel,
                KeyCode::Enter => Then::Commit,
                _ => {
                    textarea.input(key);
                    Then::Nothing
                }
            },
            None => Then::Nothing,
        };
        match then {
            Then::Nothing => {}
            Then::Commit => form.commit_edit(),
            Then::Cancel => form.cancel_edit(),
            Then::AsText => form.edit_as_text(),
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        match key.code {
            KeyCode::Esc => self.state.screen = Screen::Menu,
            KeyCode::Char('q') => return Ok(ModuleAction::Exit),
            KeyCode::Up | KeyCode::Char('k') => self.state.forward_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.state.forward_next(),
            // Enter stays a toggle because it acts on exactly one rule. The
            // scope keys cannot: with a mixed set of running and stopped rules
            // there is no answer to which way a toggle should go.
            KeyCode::Enter => self.state.toggle_selected_tunnel(),
            KeyCode::Char(' ') => self.state.toggle_mark(),
            KeyCode::Char('g') => self.state.toggle_group_mark(),
            KeyCode::Char('u') => self.state.clear_marks(),
            KeyCode::Char('s') => self.state.start_scope(),
            KeyCode::Char('S') => self.state.stop_scope(),
            KeyCode::Char('a') => self.state.start_all(),
            KeyCode::Char('A') => self.state.stop_all(),
            KeyCode::Char('r') => self.state.refresh_tunnels(),
            _ => {}
        }
        Ok(ModuleAction::None)
    }
}

impl Default for SshModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for SshModule {
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata {
            id: ModuleId::Ssh,
            name: "SSH Tunnels".to_string(),
            description: "Manage ssh config and port-forward tunnels".to_string(),
            shortcut: Some('t'),
        }
    }

    fn init(&mut self) -> Result<()> {
        self.state = SshState::new();
        self.state.load_config();
        self.state.load_tunnels();
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction> {
        match self.state.screen {
            Screen::Menu => self.handle_menu_key(key_event),
            Screen::Config => self.handle_config_key(key_event),
            Screen::Forward => self.handle_forward_key(key_event),
            Screen::Dashboard => self.handle_dashboard_key(key_event),
        }
    }

    fn update(&mut self) -> Result<()> {
        self.state.expire_notification();
        self.state.poll_tunnels();
        Ok(())
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        ui::render(&self.state, area, buf);
    }

    fn cleanup(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::sshconfig::SshConfig;
    use super::state::MENU;
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;
    use std::path::Path;

    fn press(module: &mut SshModule, code: KeyCode) -> ModuleAction {
        module
            .handle_key_event(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap()
    }

    fn rendered(module: &SshModule) -> String {
        // Wide enough that the right-hand note on a host row is not truncated.
        let area = Rect::new(0, 0, 160, 30);
        let mut buf = Buffer::empty(area);
        module.render(area, &mut buf);
        buf.content.iter().map(|cell| cell.symbol()).collect()
    }

    /// A module sitting on the host list of a config that never touches disk.
    fn on_config_screen(text: &str) -> SshModule {
        let mut module = SshModule::new();
        module.state.config = SshConfig::parse(Path::new("/tmp/config"), text);
        module.state.apply_filters();
        module.state.screen = Screen::Config;
        module
    }

    #[test]
    fn menu_renders_every_entry() {
        let out = rendered(&SshModule::new());
        for entry in MENU {
            assert!(out.contains(entry.label), "missing entry: {}", entry.label);
        }
    }

    #[test]
    fn cursor_starts_on_the_dashboard() {
        assert_eq!(
            SshModule::new().state.menu_entry().screen,
            Screen::Dashboard
        );
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let mut module = SshModule::new();
        press(&mut module, KeyCode::Char('j'));
        assert_eq!(module.state.menu_entry().screen, Screen::Config);
        press(&mut module, KeyCode::Char('k'));
        assert_eq!(module.state.menu_entry().screen, Screen::Dashboard);
    }

    #[test]
    fn enter_opens_the_selected_screen_and_esc_returns() {
        let mut module = SshModule::new();
        press(&mut module, KeyCode::Enter);
        assert_eq!(module.state.screen, Screen::Dashboard);
        press(&mut module, KeyCode::Esc);
        assert_eq!(module.state.screen, Screen::Menu);
    }

    #[test]
    fn esc_on_the_menu_exits_the_module() {
        let mut module = SshModule::new();
        assert_eq!(press(&mut module, KeyCode::Esc), ModuleAction::Exit);
    }

    #[test]
    fn a_shadowed_host_is_flagged_with_the_line_that_beats_it() {
        let module = on_config_screen("Host kami\n  Port 22\nHost kami\n  Port 22\n");
        assert!(rendered(&module).contains("dead: line 1"));
    }

    #[test]
    fn a_row_note_survives_a_narrow_terminal() {
        // The note is the reason the pane exists; at 80 columns the alias must
        // give way instead.
        let module = on_config_screen(
            "Host a-very-long-host-alias-indeed\n  Port 22\n\
             Host a-very-long-host-alias-indeed\n  Port 22\n",
        );
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        module.render(area, &mut buf);
        let out: String = buf.content.iter().map(|cell| cell.symbol()).collect();
        assert!(out.contains("dead: line 1"), "note was truncated");
    }

    #[test]
    fn a_gateway_is_shown_next_to_its_host() {
        let module = on_config_screen("Host a\n  ProxyCommand ssh bastion -W %h:%p\n  Port 22\n");
        assert!(rendered(&module).contains("via bastion"));
    }

    #[test]
    fn the_filter_narrows_the_list() {
        let mut module = on_config_screen("Host kami\n  Port 22\nHost thor\n  Port 22\n");
        press(&mut module, KeyCode::Char('/'));
        for c in "kam".chars() {
            press(&mut module, KeyCode::Char(c));
        }
        assert_eq!(module.state.filtered_indices.len(), 1);
        assert_eq!(module.state.selected_host().unwrap().alias(), "kami");
    }

    #[test]
    fn typing_q_into_the_filter_does_not_quit() {
        let mut module = on_config_screen("Host q1\n  Port 22\n");
        press(&mut module, KeyCode::Char('/'));
        assert_eq!(press(&mut module, KeyCode::Char('q')), ModuleAction::None);
        assert_eq!(module.state.search_query, "q");
    }

    #[test]
    fn esc_clears_the_filter_before_leaving_the_screen() {
        let mut module = on_config_screen("Host kami\n  Port 22\nHost thor\n  Port 22\n");
        press(&mut module, KeyCode::Char('/'));
        press(&mut module, KeyCode::Char('k'));
        press(&mut module, KeyCode::Esc);
        assert_eq!(module.state.screen, Screen::Config);
        assert_eq!(module.state.filtered_indices.len(), 2);
        press(&mut module, KeyCode::Esc);
        assert_eq!(module.state.screen, Screen::Menu);
    }

    #[test]
    fn enter_opens_the_form_and_esc_closes_it() {
        let mut module = on_config_screen("Host kami\n  Port 22\n");
        press(&mut module, KeyCode::Enter);
        assert!(module.state.form.is_some());
        press(&mut module, KeyCode::Esc);
        assert!(module.state.form.is_none());
        assert_eq!(module.state.screen, Screen::Config);
    }

    #[test]
    fn a_picker_field_offers_the_values_already_in_the_config() {
        let mut module = on_config_screen("Host a\n  User root\nHost b\n  User lxb\n");
        press(&mut module, KeyCode::Enter); // open form on `a`
        press(&mut module, KeyCode::Char('j')); // HostName
        press(&mut module, KeyCode::Char('j')); // User
        press(&mut module, KeyCode::Enter); // open the picker
        let out = rendered(&module);
        assert!(
            out.contains("lxb"),
            "picker missing a value from the config"
        );
        assert!(out.contains("(none)"), "picker cannot clear the field");
    }

    #[test]
    fn typing_into_a_field_shows_up_in_the_diff() {
        let mut module = on_config_screen("Host a\n  Port 22\n");
        press(&mut module, KeyCode::Enter); // form
        for _ in 0..3 {
            press(&mut module, KeyCode::Char('j')); // to Port
        }
        press(&mut module, KeyCode::Enter); // text box
        press(&mut module, KeyCode::Char('3'));
        press(&mut module, KeyCode::Enter); // commit
        let out = rendered(&module);
        assert!(out.contains("will write"), "no diff shown");
        assert!(
            out.contains("Port 223"),
            "edited value missing from the diff"
        );
    }

    #[test]
    fn g_toggles_the_effective_panel() {
        let mut module = on_config_screen("Host kami\n  Port 22\n");
        press(&mut module, KeyCode::Char('g'));
        assert!(module.state.effective.is_some());
        assert!(rendered(&module).contains("ssh -G kami"));
        press(&mut module, KeyCode::Char('g'));
        assert!(module.state.effective.is_none());
    }

    #[test]
    fn moving_off_a_host_drops_its_effective_panel() {
        // Otherwise the answer for one alias stays pinned next to another.
        let mut module = on_config_screen("Host a\n  Port 22\nHost b\n  Port 23\n");
        press(&mut module, KeyCode::Char('g'));
        assert!(module.state.effective.is_some());
        press(&mut module, KeyCode::Char('j'));
        assert!(module.state.effective.is_none());
    }

    #[test]
    fn a_long_candidate_list_scrolls_to_the_selection() {
        // With a plain List the highlight silently falls off the bottom once the
        // options outrun the pane, and 35 aliases always will.
        let config: String = (0..30)
            .map(|i| format!("Host host{i:02}\n  Port 22\n"))
            .collect();
        let mut module = on_config_screen(&config);
        press(&mut module, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut module, KeyCode::Char('j')); // to the jump-host field
        }
        press(&mut module, KeyCode::Enter); // open the picker
        for _ in 0..28 {
            press(&mut module, KeyCode::Char('j'));
        }
        let out = rendered(&module);
        assert!(
            out.contains("host27"),
            "the selected candidate scrolled out of view"
        );
    }

    fn with_tunnels(module: &mut SshModule, forwards: Vec<tunnels::Forward>) {
        module.state.tunnels = tunnels::Tunnels {
            profiles: vec![tunnels::Profile {
                name: "test".into(),
                forwards,
            }],
        };
        module.state.screen = Screen::Dashboard;
    }

    fn with_profiles(module: &mut SshModule, profiles: Vec<(&str, Vec<tunnels::Forward>)>) {
        module.state.tunnels = tunnels::Tunnels {
            profiles: profiles
                .into_iter()
                .map(|(name, forwards)| tunnels::Profile {
                    name: name.into(),
                    forwards,
                })
                .collect(),
        };
        module.state.screen = Screen::Dashboard;
    }

    fn forward(bind: &str) -> tunnels::Forward {
        tunnels::Forward {
            host: "nowhere".into(),
            kind: tunnels::Kind::Local,
            bind: bind.into(),
            target: "127.0.0.1:1".into(),
            note: String::new(),
        }
    }

    fn noted(bind: &str, note: &str) -> tunnels::Forward {
        tunnels::Forward {
            note: note.into(),
            ..forward(bind)
        }
    }

    fn incomplete_rule() -> tunnels::Forward {
        // What the author actually wrote: an exit with no host, which builds
        // `-L 22:6022` and ssh refuses.
        tunnels::Forward {
            host: "kami".into(),
            kind: tunnels::Kind::Local,
            bind: "22".into(),
            target: "6022".into(),
            note: String::new(),
        }
    }

    #[test]
    fn the_form_names_the_missing_host_and_whose_view_resolves_it() {
        let mut module = SshModule::new();
        with_tunnels(&mut module, vec![incomplete_rule()]);
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Enter);
        let out = rendered(&module);
        assert!(out.contains("localhost:6022"), "no suggested fix");
        assert!(out.contains("kami can reach"), "does not say whose view");
    }

    #[test]
    fn committing_a_bare_exit_port_expands_it_in_the_field() {
        // Expanded on commit rather than at build time, so the field shows what
        // the command will contain.
        let mut module = SshModule::new();
        with_tunnels(&mut module, vec![incomplete_rule()]);
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Enter);
        // Found by name rather than hardcoded, so adding a field ahead of the
        // exit does not silently point this at a different one.
        let exit = form::ForwardField::ALL
            .iter()
            .position(|f| *f == form::ForwardField::Target)
            .unwrap();
        module.state.forward_form.as_mut().unwrap().cursor = exit;
        press(&mut module, KeyCode::Enter); // open the text box
        press(&mut module, KeyCode::Enter); // accept unchanged
        assert_eq!(
            module.state.forward_form.as_ref().unwrap().values[exit],
            "localhost:6022"
        );
    }

    #[test]
    fn an_incomplete_rule_is_marked_and_refused_rather_than_launched() {
        let mut module = SshModule::new();
        with_tunnels(&mut module, vec![incomplete_rule()]);
        assert!(rendered(&module).contains("incomplete"));

        press(&mut module, KeyCode::Enter); // try to start it
        let (message, _) = module.state.notification.clone().expect("a reason");
        assert!(message.contains("localhost:6022"), "got: {message}");
    }

    #[test]
    fn a_rule_with_no_process_shows_three_dark_lights() {
        let mut module = SshModule::new();
        with_tunnels(&mut module, vec![forward("39001")]);
        let out = rendered(&module);
        assert!(out.contains("o o o"), "expected a stopped row");
        assert!(out.contains("stopped"));
        assert!(out.contains("0 of 1 up"));
    }

    #[test]
    fn the_dashboard_explains_its_own_lights() {
        // Three symbols mean nothing without saying which layer each one is.
        let mut module = SshModule::new();
        with_tunnels(&mut module, vec![forward("39001")]);
        let out = rendered(&module);
        assert!(out.contains("process / listening / path"));
    }

    #[test]
    fn the_dashboard_groups_rules_under_their_profile_with_a_count() {
        // Without the heading there is no way to tell which group a rule is in,
        // and the whole point of a group is acting on it as one.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001")]),
                ("lab", vec![forward("39002"), forward("39003")]),
            ],
        );
        let out = rendered(&module);
        assert!(out.contains("daily"), "missing the first heading");
        assert!(out.contains("lab"), "missing the second heading");
        assert!(out.contains("0/1 up"), "missing the first count");
        assert!(out.contains("0/2 up"), "missing the second count");
    }

    #[test]
    fn the_group_count_follows_the_processes_behind_its_rules() {
        // A count that is always 0/n would render and pass every layout test
        // while telling you nothing.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![forward("39001"), forward("39002")])],
        );
        let live = forward("39001");
        module.state.running = vec![supervisor::Running {
            pid: 4242,
            kind: live.kind,
            spec: live.spec(),
            host: live.host.clone(),
        }];
        let out = rendered(&module);
        assert!(out.contains("1/2 up"), "the count ignores live processes");
        assert!(out.contains("pid 4242"), "the live rule is not marked");
    }

    #[test]
    fn a_note_shows_under_its_rule_on_both_screens() {
        // The summary line is all flags and ports; the note is the only part
        // that says what the rule is for.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                (
                    "daily",
                    vec![noted("39001", "minio console"), forward("39002")],
                ),
                ("lab", vec![noted("39003", "grafana behind kami")]),
            ],
        );
        assert!(
            rendered(&module).contains("minio console"),
            "note missing from the dashboard"
        );

        module.state.screen = Screen::Forward;
        assert!(
            rendered(&module).contains("minio console"),
            "note missing from the forward list"
        );
    }

    #[test]
    fn a_rule_without_a_note_takes_one_row() {
        // An empty note must not leave a blank line pushing the list apart.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![forward("39001"), forward("39002")])],
        );
        let area = Rect::new(0, 0, 160, 30);
        let mut buf = Buffer::empty(area);
        module.render(area, &mut buf);
        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        let first = rows.iter().position(|r| r.contains("39001")).unwrap();
        let second = rows.iter().position(|r| r.contains("39002")).unwrap();
        assert_eq!(second, first + 1, "a blank row crept in between the rules");
    }

    #[test]
    fn the_forward_list_scrolls_to_the_selection() {
        // Notes make every row twice as tall, so a plain List drops the
        // highlight off the bottom far sooner than it used to.
        let mut module = SshModule::new();
        let rules: Vec<tunnels::Forward> = (0..20)
            .map(|i| noted(&format!("39{i:03}"), "why this one exists"))
            .collect();
        with_profiles(&mut module, vec![("daily", rules)]);
        module.state.screen = Screen::Forward;
        for _ in 0..19 {
            press(&mut module, KeyCode::Char('j'));
        }
        assert!(
            rendered(&module).contains("39019"),
            "the selected rule scrolled out of view"
        );
    }

    /// Move the form cursor onto a field by name.
    fn focus(module: &mut SshModule, field: form::ForwardField) {
        let at = form::ForwardField::ALL
            .iter()
            .position(|f| *f == field)
            .unwrap();
        module.state.forward_form.as_mut().unwrap().cursor = at;
    }

    fn group_of(module: &SshModule, profile: usize) -> &str {
        &module.state.tunnels.profiles[profile].name
    }

    #[test]
    fn the_group_is_a_field_offering_the_ones_that_exist() {
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001")]),
                ("lab", vec![forward("39002")]),
            ],
        );
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Enter); // open the form on the first rule
        focus(&mut module, form::ForwardField::Group);
        press(&mut module, KeyCode::Enter); // open the picker
        let out = rendered(&module);
        assert!(out.contains("daily"), "the current group is not offered");
        assert!(out.contains("lab"), "the other group is not offered");
        // Tab is the only route to a group that does not exist yet, and the
        // placeholder cannot advertise it -- the field is never empty.
        assert!(
            out.contains("Tab: type a new one"),
            "no way to discover how to name a new group"
        );
    }

    #[test]
    fn typing_a_group_that_does_not_exist_creates_it_on_save() {
        // This is the only way a group comes into being: there is no separate
        // "new group" step, so a group with nothing in it cannot be produced.
        let mut module = SshModule::new();
        with_profiles(&mut module, vec![("daily", vec![forward("39001")])]);
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Char('n')); // blank rule
        focus(&mut module, form::ForwardField::Group);
        press(&mut module, KeyCode::Enter); // picker
        press(&mut module, KeyCode::Tab); // type over it instead
        for c in "lab".chars() {
            press(&mut module, KeyCode::Char(c));
        }
        press(&mut module, KeyCode::Enter); // commit the name

        let form = module.state.forward_form.as_mut().unwrap();
        form.values = vec![
            "lab".into(),
            "kami".into(),
            "local".into(),
            "39002".into(),
            "localhost:22".into(),
            String::new(),
        ];
        module.state.apply_forward_form().unwrap();

        assert_eq!(module.state.tunnels.profiles.len(), 2, "no group was made");
        assert_eq!(group_of(&module, 1), "lab");
        assert_eq!(module.state.tunnels.profiles[1].forwards.len(), 1);
    }

    #[test]
    fn changing_the_group_moves_the_rule_instead_of_copying_it() {
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001"), forward("39002")]),
                ("lab", vec![forward("39003")]),
            ],
        );
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Enter); // form on daily's first rule
        module.state.forward_form.as_mut().unwrap().values[0] = "lab".into();
        module.state.apply_forward_form().unwrap();

        assert_eq!(module.state.tunnels.count(), 3, "a rule was duplicated");
        assert_eq!(module.state.tunnels.profiles[0].forwards.len(), 1);
        assert_eq!(module.state.tunnels.profiles[1].forwards.len(), 2);
    }

    #[test]
    fn emptying_a_group_by_moving_its_last_rule_out_removes_it() {
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001")]),
                ("lab", vec![forward("39002")]),
            ],
        );
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Enter);
        module.state.forward_form.as_mut().unwrap().values[0] = "lab".into();
        module.state.apply_forward_form().unwrap();

        assert_eq!(
            module.state.tunnels.profiles.len(),
            1,
            "an empty group survived"
        );
        assert_eq!(group_of(&module, 0), "lab");
    }

    #[test]
    fn cloning_a_rule_lands_on_the_next_port_nobody_has_claimed() {
        // 39002 is taken, so the clone of 39001 has to skip to 39003 -- two
        // rules on one port cannot both be up.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![noted("39001", "minio"), forward("39002")])],
        );
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Char('c'));
        let form = module.state.forward_form.as_ref().expect("a form opened");
        assert_eq!(form.group(), "daily", "the clone left its group");
        assert_eq!(form.to_forward().bind, "39003");
        assert_eq!(form.to_forward().note, "minio", "the rest was not carried");
        assert!(form.index.is_none(), "the clone would overwrite its source");
    }

    #[test]
    fn cloning_keeps_the_bind_address_and_only_moves_the_port() {
        let mut module = SshModule::new();
        let mut rule = forward("39001");
        rule.bind = "0.0.0.0:39001".into();
        with_profiles(&mut module, vec![("daily", vec![rule])]);
        module.state.screen = Screen::Forward;
        press(&mut module, KeyCode::Char('c'));
        assert_eq!(
            module
                .state
                .forward_form
                .as_ref()
                .unwrap()
                .to_forward()
                .bind,
            "0.0.0.0:39002"
        );
    }

    #[test]
    fn space_marks_the_cursor_and_the_scope_follows_the_marks() {
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![(
                "daily",
                vec![forward("39001"), forward("39002"), forward("39003")],
            )],
        );
        // Nothing marked: the scope is the one under the cursor.
        assert_eq!(module.state.scope(), vec![(0, 0)]);

        press(&mut module, KeyCode::Char(' '));
        press(&mut module, KeyCode::Char('j'));
        press(&mut module, KeyCode::Char('j'));
        press(&mut module, KeyCode::Char(' '));
        assert_eq!(module.state.scope(), vec![(0, 0), (0, 2)]);

        // ...and marking is a toggle, not an accumulate-only.
        press(&mut module, KeyCode::Char(' '));
        assert_eq!(module.state.scope(), vec![(0, 0)]);
    }

    #[test]
    fn the_scope_comes_out_in_screen_order_not_hash_order() {
        // It drives both what runs and what the message counts, so a set
        // iteration order would make either one unpredictable.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001"), forward("39002")]),
                ("lab", vec![forward("39003")]),
            ],
        );
        module.state.marked = [(1, 0), (0, 1), (0, 0)].into_iter().collect();
        assert_eq!(module.state.scope(), vec![(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn g_marks_the_whole_group_and_a_second_press_clears_it() {
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![
                ("daily", vec![forward("39001"), forward("39002")]),
                ("lab", vec![forward("39003")]),
            ],
        );
        press(&mut module, KeyCode::Char('g'));
        assert_eq!(
            module.state.scope(),
            vec![(0, 0), (0, 1)],
            "group not marked"
        );

        press(&mut module, KeyCode::Char('g'));
        assert!(module.state.marked.is_empty(), "second press did not clear");

        // A partly marked group fills rather than clears -- otherwise one
        // stray mark would make `g` look like it does nothing.
        press(&mut module, KeyCode::Char(' '));
        press(&mut module, KeyCode::Char('g'));
        assert_eq!(module.state.scope(), vec![(0, 0), (0, 1)]);
    }

    #[test]
    fn u_clears_every_mark() {
        let mut module = SshModule::new();
        with_profiles(&mut module, vec![("daily", vec![forward("39001")])]);
        press(&mut module, KeyCode::Char(' '));
        press(&mut module, KeyCode::Char('u'));
        assert!(module.state.marked.is_empty());
    }

    #[test]
    fn starting_a_scope_reports_the_incomplete_rules_it_skipped() {
        // Inherited from `a`: a rule that is quietly left out reads as one that
        // failed for no reason.
        //
        // The complete rule is given a process so nothing is actually launched
        // -- a startable rule here would have this test spawning real ssh.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![forward("39001"), incomplete_rule()])],
        );
        let up = forward("39001");
        module.state.running = vec![supervisor::Running {
            pid: 4242,
            kind: up.kind,
            spec: up.spec(),
            host: up.host.clone(),
        }];
        press(&mut module, KeyCode::Char('g')); // mark both
        press(&mut module, KeyCode::Char('s'));
        let (message, _) = module.state.notification.clone().expect("a report");
        assert!(message.contains("incomplete"), "got: {message}");
    }

    #[test]
    fn the_footer_says_what_the_scope_keys_will_act_on() {
        // s/S mean one thing with marks and another without, so the screen has
        // to say which -- an invisible mode otherwise.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![forward("39001"), forward("39002")])],
        );
        assert!(
            rendered(&module).contains("Space: mark"),
            "no way to discover marking"
        );

        press(&mut module, KeyCode::Char(' '));
        let out = rendered(&module);
        assert!(out.contains("start 1 marked"), "scope not stated: {out:?}");
        assert!(out.contains(" + "), "the marked row is not flagged");
    }

    #[test]
    fn deleting_a_rule_drops_the_marks_that_pointed_past_it() {
        // Marks are positions. After a delete every one of them names a
        // different rule than the one that was picked.
        let mut module = SshModule::new();
        with_profiles(
            &mut module,
            vec![("daily", vec![forward("39001"), forward("39002")])],
        );
        press(&mut module, KeyCode::Char('g'));
        assert_eq!(module.state.marked.len(), 2);
        module.state.remove_selected_forward();
        assert!(
            module.state.marked.is_empty(),
            "a mark survived a delete and now names a different rule"
        );
    }

    #[test]
    fn an_empty_dashboard_points_at_where_rules_come_from() {
        let mut module = SshModule::new();
        module.state.screen = Screen::Dashboard;
        assert!(rendered(&module).contains("Edit tunnel profiles"));
    }

    #[test]
    fn the_menu_preview_summarises_the_config() {
        let mut module = on_config_screen("Host kami\n  Port 22\nHost kami\n  Port 22\n");
        module.state.screen = Screen::Menu;
        module.state.menu_index = 0;
        let out = rendered(&module);
        assert!(out.contains("2 hosts"), "missing host count");
        assert!(out.contains("1 shadowed"), "missing shadow count");
    }
}
