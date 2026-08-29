use super::effective::{self, Effective};
use super::form::{Field, HostForm, write_config};
use super::sshconfig::{HostBlock, SshConfig};
use std::time::{Duration, Instant};

/// How long a save / error message stays on screen.
const NOTIFICATION_TTL: Duration = Duration::from_secs(5);

/// Top-level screens inside the SSH module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Landing menu with a preview pane.
    Menu,
    /// Host blocks of `~/.ssh/config`.
    Config,
    /// Tunnel profiles in `~/.config/excalibur/tunnels.yaml`.
    Forward,
    /// Live tunnel dashboard.
    Dashboard,
}

/// One entry of the landing menu.
#[derive(Debug, Clone, Copy)]
pub struct MenuEntry {
    pub screen: Screen,
    pub label: &'static str,
    pub hint: &'static str,
}

/// Menu entries, in display order.
pub const MENU: [MenuEntry; 3] = [
    MenuEntry {
        screen: Screen::Config,
        label: "Edit ssh config",
        hint: "host blocks in ~/.ssh/config",
    },
    MenuEntry {
        screen: Screen::Forward,
        label: "Edit tunnel profiles",
        hint: "~/.config/excalibur/tunnels.yaml",
    },
    MenuEntry {
        screen: Screen::Dashboard,
        label: "Start ssh forwarding",
        hint: "live tunnels and connectivity",
    },
];

/// The dashboard is the high-frequency entry and its preview doubles as the
/// connectivity summary, so the cursor starts there rather than at the top.
const DEFAULT_MENU_INDEX: usize = 2;

#[derive(Debug)]
pub struct SshState {
    pub screen: Screen,
    pub menu_index: usize,

    pub config: SshConfig,
    /// Why the last load failed. The host list shows this instead of rows.
    pub config_error: Option<String>,

    pub search_query: String,
    pub searching: bool,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,

    /// The form for the selected host, replacing the preview pane while open.
    pub form: Option<HostForm>,
    /// `ssh -G` output for an alias, shown instead of the raw block preview.
    pub effective: Option<(String, Effective)>,
    pub notification: Option<(String, Instant)>,
}

impl SshState {
    pub fn new() -> Self {
        Self {
            screen: Screen::Menu,
            menu_index: DEFAULT_MENU_INDEX,
            config: SshConfig::default(),
            config_error: None,
            search_query: String::new(),
            searching: false,
            filtered_indices: Vec::new(),
            selected_index: 0,
            form: None,
            effective: None,
            notification: None,
        }
    }

    /// Re-read the config from disk. `~/.ssh/config` is the single source of
    /// truth and is edited outside this tool too, so it is never cached across
    /// entries into the module.
    pub fn load_config(&mut self) {
        match SshConfig::load() {
            Ok(config) => {
                self.config = config;
                self.config_error = None;
            }
            Err(e) => {
                self.config = SshConfig::default();
                self.config_error = Some(e.to_string());
            }
        }
        self.apply_filters();
    }

    pub fn apply_filters(&mut self) {
        self.filtered_indices = if self.search_query.is_empty() {
            (0..self.config.hosts.len()).collect()
        } else {
            let query = self.search_query.to_lowercase();
            self.config
                .hosts
                .iter()
                .enumerate()
                .filter(|(_, host)| host.alias().to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect()
        };
        self.selected_index = 0;
    }

    pub fn menu_next(&mut self) {
        self.menu_index = (self.menu_index + 1) % MENU.len();
    }

    pub fn menu_previous(&mut self) {
        self.menu_index = (self.menu_index + MENU.len() - 1) % MENU.len();
    }

    pub fn menu_entry(&self) -> MenuEntry {
        MENU[self.menu_index]
    }

    pub fn host_next(&mut self) {
        // The panel belongs to one alias; moving off it would leave a stale
        // answer attached to a different host.
        self.effective = None;
        if !self.filtered_indices.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.filtered_indices.len();
        }
    }

    pub fn host_previous(&mut self) {
        self.effective = None;
        if !self.filtered_indices.is_empty() {
            let len = self.filtered_indices.len();
            self.selected_index = (self.selected_index + len - 1) % len;
        }
    }

    pub fn selected_host(&self) -> Option<&HostBlock> {
        let index = *self.filtered_indices.get(self.selected_index)?;
        self.config.hosts.get(index)
    }

    /// Line number, 1-based, of the block that shadows `host` -- what the user
    /// needs in order to find the block that is actually in effect.
    pub fn shadowing_line(&self, host: &HostBlock) -> Option<usize> {
        let by = host.shadowed_by?;
        Some(self.config.hosts.get(by)?.start + 1)
    }

    /// Needs the config for the candidate list, so it lives here where the two
    /// fields can be borrowed apart.
    pub fn form_begin_edit(&mut self) {
        let config = &self.config;
        if let Some(form) = self.form.as_mut() {
            form.begin_edit(config);
        }
    }

    /// Ask ssh what it resolves for the selected host, or put the panel away.
    pub fn toggle_effective(&mut self) {
        let Some(alias) = self.selected_host().map(|h| h.alias().to_string()) else {
            return;
        };
        if self
            .effective
            .as_ref()
            .is_some_and(|(shown, _)| *shown == alias)
        {
            self.effective = None;
            return;
        }
        let resolved = effective::resolve(&alias, &self.config.lines);
        self.effective = Some((alias, resolved));
    }

    pub fn open_form(&mut self) {
        if let Some(&index) = self.filtered_indices.get(self.selected_index) {
            self.form = HostForm::new(&self.config, index);
        }
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.notification = Some((message.into(), Instant::now()));
    }

    pub fn expire_notification(&mut self) {
        if let Some((_, at)) = &self.notification
            && at.elapsed() > NOTIFICATION_TTL
        {
            self.notification = None;
        }
    }

    /// Write the open form back to disk, then re-read so the line numbers the
    /// UI shows match the file again.
    pub fn save_form(&mut self) {
        let Some(form) = &self.form else { return };
        let plan = form.plan(&self.config);
        if plan.changes.is_empty() {
            self.notify("No changes");
            return;
        }
        let alias = form.value(Field::Alias).to_string();
        match write_config(&self.config, &plan.lines) {
            Ok(backup) => {
                self.form = None;
                self.load_config();
                self.select_alias(&alias);
                self.notify(format!("Saved. Backup: {}", backup.display()));
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    fn select_alias(&mut self, alias: &str) {
        if let Some(row) = self
            .filtered_indices
            .iter()
            .position(|i| self.config.hosts.get(*i).map(HostBlock::alias) == Some(alias))
        {
            self.selected_index = row;
        }
    }
}

impl Default for SshState {
    fn default() -> Self {
        Self::new()
    }
}
