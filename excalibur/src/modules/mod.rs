pub mod history;
pub mod manager;
#[cfg(target_os = "linux")]
pub mod proctrace;
pub mod ssh;

use color_eyre::Result;
use ratatui::{buffer::Buffer, crossterm::event::KeyEvent, layout::Rect};

/// Unique identifier for each module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleId {
    History,
    #[cfg(target_os = "linux")]
    ProcessTracer,
    Ssh,
}

impl ModuleId {
    /// Convert a CLI command name to ModuleId
    /// Accepts both full names and shortcuts (case-insensitive)
    pub fn from_command_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "history" | "h" => Some(ModuleId::History),
            #[cfg(target_os = "linux")]
            "proctrace" | "pt" => Some(ModuleId::ProcessTracer),
            "ssh" | "t" => Some(ModuleId::Ssh),
            _ => None,
        }
    }
}

/// Metadata describing a module
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub id: ModuleId,
    pub name: String,
    pub description: String,
    pub shortcut: Option<char>,
}

/// Action returned by module key event handlers
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleAction {
    /// No action needed
    None,
    /// Exit module and return to main menu
    Exit,
    /// Quit entire application
    Quit,
    /// Show a notification message
    Notification(String),
    /// Output a command to stdout and exit (for Fish integration)
    Output(String),
    /// Output a command and execute immediately
    OutputAndExecute(String),
}

/// Trait that all modules must implement
pub trait Module: std::fmt::Debug {
    /// Get module metadata
    fn metadata(&self) -> ModuleMetadata;

    /// Initialize the module (called when entering)
    fn init(&mut self) -> Result<()>;

    /// Handle key events specific to this module
    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction>;

    /// Update module state (called on tick)
    fn update(&mut self) -> Result<()>;

    /// Render the module UI
    fn render(&self, area: Rect, buf: &mut Buffer);

    /// Cleanup when exiting module
    fn cleanup(&mut self) -> Result<()>;
}
