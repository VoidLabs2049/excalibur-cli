use super::query::{QueryResult, QueryType};
use color_eyre::Result;
use std::time::Instant;

/// Input mode for the process tracer
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Query,       // Entering query
    ViewResults, // Browsing results
}

/// State for the process tracer module (query-driven)
#[derive(Debug)]
pub struct ProcessTracerState {
    /// Current input mode
    pub input_mode: InputMode,

    /// Query input buffer
    pub query_input: String,

    /// Query results
    pub query_results: Vec<QueryResult>,

    /// Selected result index
    pub selected_result: usize,

    /// Scroll offset for details panel
    pub scroll_offset: u16,

    /// Notification message with timestamp
    pub notification: Option<(String, Instant)>,

    /// Query history (for up/down arrow navigation)
    pub query_history: Vec<String>,

    /// Current position in history
    pub history_index: usize,
}

impl ProcessTracerState {
    pub fn new() -> Self {
        Self {
            input_mode: InputMode::Query,
            query_input: String::new(),
            query_results: Vec::new(),
            selected_result: 0,
            scroll_offset: 0,
            notification: None,
            query_history: Vec::new(),
            history_index: 0,
        }
    }

    /// Parse query input into QueryType
    pub fn parse_query(&self) -> Result<QueryType> {
        let input = self.query_input.trim();

        // Try to parse as PID (pure number)
        if let Ok(pid) = input.parse::<u32>() {
            return Ok(QueryType::ByPid(pid));
        }

        // Try to parse as port (":8080" format)
        if let Some(port_str) = input.strip_prefix(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                return Ok(QueryType::ByPort(port));
            }
        }

        // Default: treat as process name
        Ok(QueryType::ByName(input.to_string()))
    }

    /// Add query to history
    pub fn add_to_history(&mut self, query: String) {
        if !query.is_empty() && self.query_history.last() != Some(&query) {
            self.query_history.push(query);
            self.history_index = self.query_history.len();
        }
    }

    /// Navigate history up
    pub fn history_up(&mut self) {
        if !self.query_history.is_empty() && self.history_index > 0 {
            self.history_index -= 1;
            self.query_input = self.query_history[self.history_index].clone();
        }
    }

    /// Navigate history down
    pub fn history_down(&mut self) {
        if self.history_index < self.query_history.len().saturating_sub(1) {
            self.history_index += 1;
            self.query_input = self.query_history[self.history_index].clone();
        } else if self.history_index == self.query_history.len().saturating_sub(1) {
            // At the end of history, clear input
            self.history_index = self.query_history.len();
            self.query_input.clear();
        }
    }

    /// Navigate to previous result
    pub fn select_previous(&mut self) {
        if self.selected_result > 0 {
            self.selected_result -= 1;
            self.scroll_offset = 0; // Reset scroll when changing selection
        }
    }

    /// Navigate to next result
    pub fn select_next(&mut self) {
        if self.selected_result < self.query_results.len().saturating_sub(1) {
            self.selected_result += 1;
            self.scroll_offset = 0; // Reset scroll when changing selection
        }
    }

    /// Navigate to first result
    pub fn select_first(&mut self) {
        self.selected_result = 0;
        self.scroll_offset = 0;
    }

    /// Navigate to last result
    pub fn select_last(&mut self) {
        if !self.query_results.is_empty() {
            self.selected_result = self.query_results.len() - 1;
            self.scroll_offset = 0;
        }
    }

    /// Page up (10 lines)
    pub fn page_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(10);
    }

    /// Page down (10 lines)
    pub fn page_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(10);
    }

    /// Set notification message
    pub fn set_notification(&mut self, message: String) {
        self.notification = Some((message, Instant::now()));
    }

    /// Clear expired notifications (> 3 seconds)
    pub fn clear_expired_notifications(&mut self) {
        if let Some((_, timestamp)) = &self.notification {
            if timestamp.elapsed().as_secs() > 3 {
                self.notification = None;
            }
        }
    }

    /// Get currently selected query result
    pub fn get_selected_result(&self) -> Option<&QueryResult> {
        self.query_results.get(self.selected_result)
    }
}
