use super::{
    Module, ModuleAction, ModuleId, ModuleMetadata, history::HistoryModule,
    settings::SettingsModule, ssh::SshModule,
};
#[cfg(target_os = "linux")]
use super::proctrace::ProcessTracerModule;
use color_eyre::Result;
use ratatui::{buffer::Buffer, crossterm::event::KeyEvent, layout::Rect};
use std::collections::HashMap;

/// Manages all application modules
#[derive(Debug)]
pub struct ModuleManager {
    modules: HashMap<ModuleId, Box<dyn Module>>,
    active_module: Option<ModuleId>,
}

impl ModuleManager {
    /// Create a new module manager with all available modules
    pub fn new() -> Self {
        let mut modules: HashMap<ModuleId, Box<dyn Module>> = HashMap::new();

        // Register history module
        let history = HistoryModule::new();
        modules.insert(ModuleId::History, Box::new(history));

        // Register process tracer module (Linux only)
        #[cfg(target_os = "linux")]
        {
            let proctrace = ProcessTracerModule::new();
            modules.insert(ModuleId::ProcessTracer, Box::new(proctrace));
        }

        // Register settings module
        let settings = SettingsModule::new();
        modules.insert(ModuleId::Settings, Box::new(settings));

        // Register ssh module
        let ssh = SshModule::new();
        modules.insert(ModuleId::Ssh, Box::new(ssh));

        Self {
            modules,
            active_module: None,
        }
    }

    /// Activate a module by its ID
    pub fn activate(&mut self, id: ModuleId) -> Result<()> {
        if let Some(module) = self.modules.get_mut(&id) {
            module.init()?;
            self.active_module = Some(id);
            Ok(())
        } else {
            Err(color_eyre::eyre::eyre!("Module {:?} not found", id))
        }
    }

    /// Deactivate the current module
    pub fn deactivate(&mut self) -> Result<()> {
        if let Some(id) = self.active_module {
            if let Some(module) = self.modules.get_mut(&id) {
                module.cleanup()?;
            }
            self.active_module = None;
        }
        Ok(())
    }

    /// Get the currently active module
    pub fn get_active(&self) -> Option<&dyn Module> {
        self.active_module
            .and_then(|id| self.modules.get(&id).map(|b| b.as_ref()))
    }

    /// Get the currently active module (mutable)
    pub fn get_active_mut(&mut self) -> Option<&mut dyn Module> {
        if let Some(id) = self.active_module {
            if let Some(module) = self.modules.get_mut(&id) {
                return Some(module.as_mut());
            }
        }
        None
    }

    /// List all available modules
    pub fn list_modules(&self) -> Vec<ModuleMetadata> {
        self.modules
            .values()
            .map(|module| module.metadata())
            .collect()
    }

    /// Handle key event for the active module
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction> {
        if let Some(module) = self.get_active_mut() {
            module.handle_key_event(key_event)
        } else {
            Ok(ModuleAction::None)
        }
    }

    /// Update the active module
    pub fn update(&mut self) -> Result<()> {
        if let Some(module) = self.get_active_mut() {
            module.update()
        } else {
            Ok(())
        }
    }

    /// Render the active module
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if let Some(module) = self.get_active() {
            module.render(area, buf);
        }
    }
}

impl Default for ModuleManager {
    fn default() -> Self {
        Self::new()
    }
}
