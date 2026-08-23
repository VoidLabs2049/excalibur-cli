mod collector;
mod network;
mod query;
mod state;
mod systemd;
mod ui;

use crate::modules::{Module, ModuleAction, ModuleId, ModuleMetadata};
use collector::Supervisor;
use color_eyre::Result;
use query::QueryEngine;
use ratatui::{buffer::Buffer, crossterm::event::KeyEvent, layout::Rect};
use state::{InputMode, ProcessTracerState};

/// Process Tracer module (query-driven)
#[derive(Debug)]
pub struct ProcessTracerModule {
    state: ProcessTracerState,
    query_engine: QueryEngine,
}

impl ProcessTracerModule {
    pub fn new() -> Self {
        Self {
            state: ProcessTracerState::new(),
            query_engine: QueryEngine::new(),
        }
    }

    /// Execute the current query
    fn execute_query(&mut self) -> Result<()> {
        // Parse query input
        let query = match self.state.parse_query() {
            Ok(q) => q,
            Err(e) => {
                self.state.set_notification(format!("Invalid query: {}", e));
                return Ok(());
            }
        };

        // Execute query
        match self.query_engine.execute(query) {
            Ok(results) => {
                if results.is_empty() {
                    self.state
                        .set_notification("No processes found".to_string());
                } else {
                    // Store results and switch to results mode
                    self.state.query_results = results;
                    self.state.selected_result = 0;
                    self.state.scroll_offset = 0;
                    self.state.input_mode = InputMode::ViewResults;

                    // Add to history
                    self.state.add_to_history(self.state.query_input.clone());

                    self.state.set_notification(format!(
                        "Found {} result(s)",
                        self.state.query_results.len()
                    ));
                }
            }
            Err(e) => {
                self.state.set_notification(format!("Query error: {}", e));
            }
        }

        Ok(())
    }

    /// Handle key events in query mode
    fn handle_query_mode(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            // Execute query
            KeyCode::Enter => {
                self.execute_query()?;
                Ok(ModuleAction::None)
            }

            // Input character
            KeyCode::Char(c) => {
                self.state.query_input.push(c);
                Ok(ModuleAction::None)
            }

            // Backspace
            KeyCode::Backspace => {
                self.state.query_input.pop();
                Ok(ModuleAction::None)
            }

            // History navigation
            KeyCode::Up => {
                self.state.history_up();
                Ok(ModuleAction::None)
            }
            KeyCode::Down => {
                self.state.history_down();
                Ok(ModuleAction::None)
            }

            // Exit
            KeyCode::Esc => Ok(ModuleAction::Exit),

            _ => Ok(ModuleAction::None),
        }
    }

    /// Build a command from the selected process and hand it back to the shell.
    ///
    /// The query already resolved the parts that are hard to recall — the PID,
    /// the systemd unit, the working directory — so emitting is just formatting.
    /// Always `Output` (editable) rather than `OutputAndExecute`: these kill or
    /// restart things, and the unit is worth a glance before hitting enter.
    fn emit_from_selection(&mut self, key: char) -> ModuleAction {
        let Some(result) = self.state.get_selected_result() else {
            return ModuleAction::None;
        };

        let unit = match &result.process.supervisor {
            Supervisor::Systemd { unit } => Some(unit.clone()),
            _ => None,
        };

        let command = match key {
            'x' => Some(format!("kill {}", result.process.pid)),
            'c' => result
                .working_directory
                .as_ref()
                .map(|dir| format!("cd {}", dir)),
            'l' => unit.map(|u| format!("journalctl -u {} -f", u)),
            'r' => unit.map(|u| format!("systemctl restart {}", u)),
            _ => None,
        };

        match command {
            Some(command) => ModuleAction::Output(command),
            None => {
                let reason = match key {
                    'c' => "No working directory for this process",
                    _ => "Not supervised by systemd",
                };
                self.state.set_notification(reason.to_string());
                ModuleAction::None
            }
        }
    }

    /// Handle key events in results mode
    fn handle_results_mode(&mut self, key: KeyEvent) -> Result<ModuleAction> {
        use ratatui::crossterm::event::KeyCode;

        match key.code {
            // Return to query mode
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Query;
                self.state.query_input.clear();
                self.state.scroll_offset = 0;
                Ok(ModuleAction::None)
            }

            // New query
            KeyCode::Char('/') => {
                self.state.input_mode = InputMode::Query;
                self.state.query_input.clear();
                self.state.scroll_offset = 0;
                Ok(ModuleAction::None)
            }

            // Navigate results
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.select_previous();
                Ok(ModuleAction::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.select_next();
                Ok(ModuleAction::None)
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.state.select_first();
                Ok(ModuleAction::None)
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.state.select_last();
                Ok(ModuleAction::None)
            }

            // Scroll details panel
            KeyCode::PageUp => {
                self.state.page_up();
                Ok(ModuleAction::None)
            }
            KeyCode::PageDown => {
                self.state.page_down();
                Ok(ModuleAction::None)
            }

            // Emit a command built from the selected process (shell integration)
            KeyCode::Char(c @ ('l' | 'r' | 'x' | 'c')) => Ok(self.emit_from_selection(c)),

            // Quit (also exits)
            KeyCode::Char('q') => Ok(ModuleAction::Exit),

            _ => Ok(ModuleAction::None),
        }
    }
}

impl Module for ProcessTracerModule {
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata {
            id: ModuleId::ProcessTracer,
            name: "Process Tracer".to_string(),
            description: "Query and analyze running processes - Why is this running?".to_string(),
            shortcut: Some('p'),
        }
    }

    fn init(&mut self) -> Result<()> {
        // Reset to query mode on entry
        self.state.input_mode = InputMode::Query;
        self.state.query_input.clear();
        self.state.query_results.clear();
        self.state.selected_result = 0;
        self.state.scroll_offset = 0;
        self.state.notification = None;

        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<ModuleAction> {
        match self.state.input_mode {
            InputMode::Query => self.handle_query_mode(key_event),
            InputMode::ViewResults => self.handle_results_mode(key_event),
        }
    }

    fn update(&mut self) -> Result<()> {
        // Only clear expired notifications (no auto-refresh)
        self.state.clear_expired_notifications();
        Ok(())
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        ui::render(&self.state, area, buf);
    }

    fn cleanup(&mut self) -> Result<()> {
        // Clear state on module exit
        self.state.input_mode = InputMode::Query;
        self.state.query_input.clear();
        self.state.query_results.clear();
        self.state.notification = None;

        Ok(())
    }
}
