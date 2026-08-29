mod form;
mod sshconfig;
mod state;
mod ui;

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
        // Decide first, act second: the picker's own state is borrowed while it
        // is being read, so commit/cancel cannot run inside the match.
        enum Then {
            Nothing,
            Commit,
            Cancel,
            AsText,
        }

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

    fn handle_subscreen_key(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        match key.code {
            KeyCode::Esc => {
                self.state.screen = Screen::Menu;
                Ok(ModuleAction::None)
            }
            KeyCode::Char('q') => Ok(ModuleAction::Exit),
            _ => Ok(ModuleAction::None),
        }
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
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction> {
        match self.state.screen {
            Screen::Menu => self.handle_menu_key(key_event),
            Screen::Config => self.handle_config_key(key_event),
            _ => self.handle_subscreen_key(key_event),
        }
    }

    fn update(&mut self) -> Result<()> {
        self.state.expire_notification();
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
    fn the_menu_preview_summarises_the_config() {
        let mut module = on_config_screen("Host kami\n  Port 22\nHost kami\n  Port 22\n");
        module.state.screen = Screen::Menu;
        module.state.menu_index = 0;
        let out = rendered(&module);
        assert!(out.contains("2 hosts"), "missing host count");
        assert!(out.contains("1 shadowed"), "missing shadow count");
    }
}
