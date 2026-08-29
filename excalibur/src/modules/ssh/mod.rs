mod state;
mod ui;

use super::{Module, ModuleAction, ModuleId, ModuleMetadata};
use color_eyre::Result;
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
                self.state.select_previous();
                Ok(ModuleAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.select_next();
                Ok(ModuleAction::None)
            }
            KeyCode::Enter => {
                self.state.screen = self.state.selected().screen;
                Ok(ModuleAction::None)
            }
            _ => Ok(ModuleAction::None),
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
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction> {
        match self.state.screen {
            Screen::Menu => self.handle_menu_key(key_event),
            _ => self.handle_subscreen_key(key_event),
        }
    }

    fn update(&mut self) -> Result<()> {
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
    use super::state::MENU;
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(module: &mut SshModule, code: KeyCode) -> ModuleAction {
        module
            .handle_key_event(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap()
    }

    fn rendered(module: &SshModule) -> String {
        let area = Rect::new(0, 0, 100, 24);
        let mut buf = Buffer::empty(area);
        module.render(area, &mut buf);
        buf.content.iter().map(|cell| cell.symbol()).collect()
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
        assert_eq!(SshModule::new().state.selected().screen, Screen::Dashboard);
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let mut module = SshModule::new();
        press(&mut module, KeyCode::Char('j'));
        assert_eq!(module.state.selected().screen, Screen::Config);
        press(&mut module, KeyCode::Char('k'));
        assert_eq!(module.state.selected().screen, Screen::Dashboard);
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
}
