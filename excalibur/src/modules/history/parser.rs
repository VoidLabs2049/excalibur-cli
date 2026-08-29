use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// A single command entry in the history
#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub cmd: String,
    pub timestamp: i64,
    pub paths: Vec<String>,
    pub count: usize,
}

impl CommandEntry {
    /// Format the timestamp as a human-readable string
    pub fn format_timestamp(&self) -> String {
        if let Some(dt) = DateTime::from_timestamp(self.timestamp, 0) {
            let now = Utc::now();
            let duration = now.signed_duration_since(dt);

            if duration.num_days() == 0 {
                "today".to_string()
            } else if duration.num_days() == 1 {
                "yesterday".to_string()
            } else if duration.num_days() < 7 {
                format!("{} days ago", duration.num_days())
            } else if duration.num_days() < 30 {
                format!("{} weeks ago", duration.num_weeks())
            } else if duration.num_days() < 365 {
                format!("{} months ago", duration.num_days() / 30)
            } else {
                format!("{} years ago", duration.num_days() / 365)
            }
        } else {
            "unknown".to_string()
        }
    }
}

/// Statistics for a command (first usage)
#[derive(Debug, Clone)]
pub struct HistoryStats {
    pub first_used: i64,
}

/// Raw entry from Fish history file
#[derive(Debug, Deserialize)]
struct RawEntry {
    cmd: String,
    when: i64,
    #[serde(default)]
    paths: Vec<String>,
}

/// Parser for Fish shell history
#[derive(Debug)]
pub struct FishHistoryParser {
    history_path: PathBuf,
}

impl FishHistoryParser {
    /// Create a new Fish history parser
    pub fn new() -> Result<Self> {
        let history_path = dirs::data_local_dir()
            .ok_or_else(|| eyre!("Failed to find local data directory"))?
            .join("fish")
            .join("fish_history");

        Ok(Self { history_path })
    }

    /// Check if the history file exists
    pub fn exists(&self) -> bool {
        self.history_path.exists()
    }

    /// Parse the Fish history file and return aggregated commands
    pub fn parse(&self) -> Result<Vec<CommandEntry>> {
        if !self.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.history_path)?;
        let raw_entries = self.parse_raw_entries(&content)?;
        Ok(self.aggregate_commands(raw_entries))
    }

    /// Parse raw entries from the history file
    fn parse_raw_entries(&self, content: &str) -> Result<Vec<RawEntry>> {
        let mut entries = Vec::new();
        let mut current_cmd: Option<String> = None;
        let mut current_when: i64 = 0;
        let mut current_paths: Vec<String> = Vec::new();
        let mut in_paths = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("- cmd: ") {
                // Save previous entry if exists
                if let Some(cmd) = current_cmd.take() {
                    entries.push(RawEntry {
                        cmd,
                        when: current_when,
                        paths: current_paths.clone(),
                    });
                    current_paths.clear();
                }

                // Start new entry
                current_cmd = Some(trimmed.trim_start_matches("- cmd: ").to_string());
                current_when = 0;
                in_paths = false;
            } else if trimmed.starts_with("when: ") {
                if let Ok(timestamp) = trimmed.trim_start_matches("when: ").parse::<i64>() {
                    current_when = timestamp;
                }
                in_paths = false;
            } else if trimmed == "paths:" {
                in_paths = true;
            } else if in_paths && trimmed.starts_with("- ") {
                let path = trimmed.trim_start_matches("- ").to_string();
                current_paths.push(path);
            }
        }

        // Don't forget the last entry
        if let Some(cmd) = current_cmd {
            entries.push(RawEntry {
                cmd,
                when: current_when,
                paths: current_paths,
            });
        }

        Ok(entries)
    }

    /// Aggregate commands by counting occurrences and tracking timestamps
    fn aggregate_commands(&self, raw: Vec<RawEntry>) -> Vec<CommandEntry> {
        let mut command_map: HashMap<String, Vec<i64>> = HashMap::new();
        let mut path_map: HashMap<String, Vec<String>> = HashMap::new();

        // Group by command
        for entry in raw {
            command_map
                .entry(entry.cmd.clone())
                .or_default()
                .push(entry.when);

            if !entry.paths.is_empty() {
                path_map
                    .entry(entry.cmd.clone())
                    .or_default()
                    .extend(entry.paths);
            }
        }

        // Convert to CommandEntry
        let mut result: Vec<CommandEntry> = command_map
            .into_iter()
            .map(|(cmd, timestamps)| {
                let last_timestamp = *timestamps.iter().max().unwrap_or(&0);
                let paths = path_map.get(&cmd).cloned().unwrap_or_default();

                // Deduplicate paths
                let mut unique_paths = paths;
                unique_paths.sort();
                unique_paths.dedup();

                CommandEntry {
                    cmd,
                    timestamp: last_timestamp,
                    paths: unique_paths,
                    count: timestamps.len(),
                }
            })
            .collect();

        // Sort by usage count (descending) by default
        result.sort_by(|a, b| b.count.cmp(&a.count));

        result
    }

    /// Get statistics for all commands
    pub fn get_stats(&self) -> Result<HashMap<String, HistoryStats>> {
        if !self.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(&self.history_path)?;
        let raw_entries = self.parse_raw_entries(&content)?;

        let mut stats_map: HashMap<String, Vec<i64>> = HashMap::new();

        for entry in raw_entries {
            stats_map.entry(entry.cmd).or_default().push(entry.when);
        }

        let stats = stats_map
            .into_iter()
            .map(|(cmd, timestamps)| {
                let first = *timestamps.iter().min().unwrap_or(&0);
                (cmd, HistoryStats { first_used: first })
            })
            .collect();

        Ok(stats)
    }
}
