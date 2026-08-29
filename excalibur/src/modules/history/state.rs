use super::parser::{CommandEntry, HistoryStats};
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::time::Instant;

/// Sort mode for command history
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    /// Sort by usage count (most used first)
    UsageCount,
    /// Sort by timestamp (most recent first)
    Timestamp,
    /// Sort alphabetically
    Alphabetical,
}

impl SortMode {
    /// Get the next sort mode (cycle through)
    pub fn next(&self) -> Self {
        match self {
            Self::UsageCount => Self::Timestamp,
            Self::Timestamp => Self::Alphabetical,
            Self::Alphabetical => Self::UsageCount,
        }
    }

    /// Get display name with indicator
    pub fn display(&self) -> String {
        match self {
            Self::UsageCount => "Usage ↓".to_string(),
            Self::Timestamp => "Recent ↓".to_string(),
            Self::Alphabetical => "A-Z ↑".to_string(),
        }
    }
}

/// Input mode for the history module
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation mode
    Normal,
    /// Search input mode
    Search,
}

/// State for the history module
#[derive(Debug)]
pub struct HistoryState {
    /// All commands
    pub commands: Vec<CommandEntry>,
    /// Filtered indices (after search)
    pub filtered_indices: Vec<usize>,
    /// Statistics for each command
    pub stats: HashMap<String, HistoryStats>,

    /// Currently selected index in filtered list
    pub selected_index: usize,
    /// Table state for rendering
    pub table_state: TableState,

    /// Search query
    pub search_query: String,
    /// Current input mode
    pub input_mode: InputMode,
    /// Current sort mode
    pub sort_mode: SortMode,

    /// Notification message and timestamp
    pub notification: Option<(String, Instant)>,
}

impl HistoryState {
    /// Create a new history state
    pub fn new(commands: Vec<CommandEntry>, stats: HashMap<String, HistoryStats>) -> Self {
        let filtered_indices: Vec<usize> = (0..commands.len()).collect();

        let mut state = Self {
            commands,
            filtered_indices,
            stats,
            selected_index: 0,
            table_state: TableState::default(),
            search_query: String::new(),
            input_mode: InputMode::Normal,
            sort_mode: SortMode::UsageCount,
            notification: None,
        };

        // Select first item
        if !state.filtered_indices.is_empty() {
            state.table_state.select(Some(0));
        }

        state
    }

    /// Apply filters based on search query
    pub fn apply_filters(&mut self) {
        if self.search_query.is_empty() {
            // No filter, show all
            self.filtered_indices = (0..self.commands.len()).collect();
        } else {
            // Case-insensitive substring search
            let query_lower = self.search_query.to_lowercase();
            self.filtered_indices = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| cmd.cmd.to_lowercase().contains(&query_lower))
                .map(|(idx, _)| idx)
                .collect();
        }

        self.apply_sort();
        self.selected_index = 0;
        self.table_state
            .select(if self.filtered_indices.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    /// Apply current sort mode
    pub fn apply_sort(&mut self) {
        match self.sort_mode {
            SortMode::UsageCount => {
                self.filtered_indices
                    .sort_by_key(|&idx| std::cmp::Reverse(self.commands[idx].count));
            }
            SortMode::Timestamp => {
                self.filtered_indices
                    .sort_by_key(|&idx| std::cmp::Reverse(self.commands[idx].timestamp));
            }
            SortMode::Alphabetical => {
                self.filtered_indices
                    .sort_by_key(|&idx| self.commands[idx].cmd.to_lowercase());
            }
        }
    }

    /// Get the currently selected command
    pub fn get_selected_command(&self) -> Option<&CommandEntry> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.commands.get(idx))
    }

    /// Move selection to next command
    pub fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        self.selected_index = (self.selected_index + 1) % self.filtered_indices.len();
        self.table_state.select(Some(self.selected_index));
    }

    /// Move selection to previous command
    pub fn select_previous(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        if self.selected_index == 0 {
            self.selected_index = self.filtered_indices.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.table_state.select(Some(self.selected_index));
    }

    /// Move selection down by a page
    pub fn page_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        let page_size = 10;
        self.selected_index =
            (self.selected_index + page_size).min(self.filtered_indices.len() - 1);
        self.table_state.select(Some(self.selected_index));
    }

    /// Move selection up by a page
    pub fn page_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        let page_size = 10;
        self.selected_index = self.selected_index.saturating_sub(page_size);
        self.table_state.select(Some(self.selected_index));
    }

    /// Jump to the first command
    pub fn select_first(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        self.selected_index = 0;
        self.table_state.select(Some(0));
    }

    /// Jump to the last command
    pub fn select_last(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        self.selected_index = self.filtered_indices.len() - 1;
        self.table_state.select(Some(self.selected_index));
    }

    /// Cycle to the next sort mode
    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.apply_sort();
        self.selected_index = 0;
        self.table_state
            .select(if self.filtered_indices.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    /// Set a notification message
    pub fn set_notification(&mut self, message: String) {
        self.notification = Some((message, Instant::now()));
    }

    /// Clear expired notifications
    pub fn clear_expired_notifications(&mut self) {
        if let Some((_, time)) = &self.notification {
            if time.elapsed().as_secs() >= 3 {
                self.notification = None;
            }
        }
    }

    /// Get total command count
    pub fn total_count(&self) -> usize {
        self.commands.len()
    }

    /// Get filtered command count
    pub fn filtered_count(&self) -> usize {
        self.filtered_indices.len()
    }
}
