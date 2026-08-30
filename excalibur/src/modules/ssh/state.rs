use super::discover::Listener;
use super::effective::{self, Effective};
use super::form::{BlockEdit, Field, ForwardForm, HostForm, write_config};
use super::probe::{self, Health, Light};
use super::sshconfig::{HostBlock, SshConfig};
use super::supervisor::{self, Running, Usage};
use super::tunnels::{Forward, Kind, Profile, Protocol, Tunnels};
use super::worker::{Job, Outcome, Slot, Worker};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// How long a save / error message stays on screen.
const NOTIFICATION_TTL: Duration = Duration::from_secs(5);
/// Reading /proc is cheap, but not 30 times a second.
const SCAN_INTERVAL: Duration = Duration::from_secs(1);

/// One process's last measurement, plus the rate that came out of comparing it
/// with the one before.
///
/// A rate needs two samples, so the previous reading has to be kept somewhere;
/// this is that somewhere.
#[derive(Debug, Clone, Copy)]
pub struct Meter {
    pub usage: Usage,
    /// Bytes per second since the previous sample. `None` until there has been
    /// a previous sample -- which is not the same as zero, and must not be
    /// drawn as if it were.
    pub rate: Option<f64>,
    pub at: Instant,
}

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
    /// What one host is listening on, with the ports pickable as new rules.
    Discover,
}

/// One host's listening ports, and which of them are wanted as rules.
#[derive(Debug)]
pub struct Discovery {
    pub host: String,
    /// Loopback-only first, since those are the ones `-L` exists for.
    pub listeners: Vec<Listener>,
    /// Ports, not indices: the list is rebuilt on every refresh, so a position
    /// would name a different port afterwards.
    pub marked: HashSet<u16>,
    pub cursor: usize,
    /// Set while the ssh round trip is out. Nothing else says the screen is
    /// waiting rather than empty.
    pub asking: bool,
    pub error: Option<String>,
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
    /// The file as it was on the way in, for `U`.
    ///
    /// The on-disk backup only ever holds the state before the *last* save, so
    /// two saves put the file the user walked in with out of reach. This is the
    /// copy that does not get overwritten.
    pub pristine: Option<Vec<String>>,

    pub search_query: String,
    pub searching: bool,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,

    /// The form for the selected host, replacing the preview pane while open.
    pub form: Option<HostForm>,
    /// Free-text editing of the selected host's own lines, the supplement to
    /// the form for the blocks its six fields cannot express.
    pub block: Option<BlockEdit>,
    /// `ssh -G` output for an alias, shown instead of the raw block preview.
    pub effective: Option<(String, Effective)>,
    pub notification: Option<(String, Instant)>,

    pub tunnels: Tunnels,
    /// Why the last tunnels load failed -- usually a hand-edited yaml.
    pub tunnels_error: Option<String>,
    /// Index into `tunnels.all()`, so it walks profiles transparently.
    pub forward_index: usize,
    pub forward_form: Option<ForwardForm>,
    /// Rules picked out for the next start/stop. A `Slot` is a position, not an
    /// identity, so this is emptied whenever the file underneath it can shift.
    pub marked: HashSet<Slot>,
    /// The open port-discovery screen, if any.
    pub discovery: Option<Discovery>,

    /// Tunnel processes found on this machine, refreshed on a timer.
    pub running: Vec<Running>,
    pub health: HashMap<Slot, Health>,
    /// Keyed by pid, not by slot: what is measured is a process, and the rule it
    /// belongs to can change under it.
    pub meters: HashMap<u32, Meter>,
    worker: Worker,
    last_scan: Option<Instant>,
}

impl SshState {
    pub fn new() -> Self {
        Self {
            screen: Screen::Menu,
            menu_index: DEFAULT_MENU_INDEX,
            config: SshConfig::default(),
            config_error: None,
            pristine: None,
            search_query: String::new(),
            searching: false,
            filtered_indices: Vec::new(),
            selected_index: 0,
            form: None,
            block: None,
            effective: None,
            notification: None,
            tunnels: Tunnels::default(),
            tunnels_error: None,
            forward_index: 0,
            forward_form: None,
            marked: HashSet::new(),
            discovery: None,
            running: Vec::new(),
            health: HashMap::new(),
            meters: HashMap::new(),
            worker: Worker::spawn(),
            last_scan: None,
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
        // Only the first load of the session. Every later one runs after a save,
        // and taking the snapshot again there would make `U` a no-op.
        if self.pristine.is_none() && self.config_error.is_none() {
            self.pristine = Some(self.config.lines.clone());
        }
        self.apply_filters();
    }

    pub fn load_tunnels(&mut self) {
        match Tunnels::load() {
            Ok(tunnels) => {
                self.tunnels = tunnels;
                self.tunnels_error = None;
            }
            Err(e) => {
                self.tunnels = Tunnels::default();
                self.tunnels_error = Some(e.to_string());
            }
        }
        self.forward_index = 0;
        self.marked.clear();
    }

    /// Move to another screen, keeping the cursor on a row the new screen has.
    ///
    /// The dashboard has rows after the last rule -- the orphans -- and the
    /// forward list does not, so a cursor parked on one would arrive over there
    /// selecting nothing at all, and `n`/`c`/`d` would quietly do nothing.
    pub fn goto(&mut self, screen: Screen) {
        self.screen = screen;
        self.forward_index = self.forward_index.min(self.cursor_span().saturating_sub(1));
    }

    /// How many rows the cursor can land on, which is screen-dependent.
    fn cursor_span(&self) -> usize {
        match self.screen {
            Screen::Dashboard => self.tunnels.count() + self.orphans().len(),
            _ => self.tunnels.count(),
        }
    }

    pub fn forward_next(&mut self) {
        let span = self.cursor_span();
        if span > 0 {
            self.forward_index = (self.forward_index + 1) % span;
        }
    }

    pub fn forward_previous(&mut self) {
        let span = self.cursor_span();
        if span > 0 {
            self.forward_index = (self.forward_index + span - 1) % span;
        }
    }

    pub fn selected_forward(&self) -> Option<&Forward> {
        let (profile, forward) = *self.tunnels.all().get(self.forward_index)?;
        self.tunnels.get(profile, forward)
    }

    /// Open the editor on the selected rule, or on a blank one in its group.
    ///
    /// A blank rule with no group at all still opens: the group field defaults
    /// to `default`, and the group itself only comes into existence when the
    /// rule is saved.
    pub fn open_forward_form(&mut self, blank: bool) {
        if blank {
            let profile = self.selected_profile().unwrap_or(0);
            let group = self.group_name(profile);
            self.forward_form = Some(ForwardForm::new(
                profile,
                None,
                &Forward {
                    host: String::new(),
                    kind: Kind::Local,
                    bind: String::new(),
                    target: String::new(),
                    protocol: Protocol::Tcp,
                    note: String::new(),
                },
                &group,
            ));
            return;
        }
        let Some(&(profile, index)) = self.tunnels.all().get(self.forward_index) else {
            return;
        };
        let Some(forward) = self.tunnels.get(profile, index) else {
            return;
        };
        let group = self.group_name(profile);
        self.forward_form = Some(ForwardForm::new(profile, Some(index), forward, &group));
    }

    /// Open the editor on a copy of the selected rule, on the next free port.
    ///
    /// A group of ports to one host differs only in the port, so cloning turns
    /// "add another one" from filling six fields into changing one.
    pub fn clone_forward(&mut self) {
        let Some(&(profile, index)) = self.tunnels.all().get(self.forward_index) else {
            return;
        };
        let Some(source) = self.tunnels.get(profile, index) else {
            return;
        };
        let mut copy = source.clone();
        copy.bind = self.next_free_bind(&source.bind);
        let group = self.group_name(profile);
        self.forward_form = Some(ForwardForm::new(profile, None, &copy, &group));
    }

    /// The next listening port no other rule already claims.
    ///
    /// Two rules cannot bind the same port, so a clone that reused it would be
    /// refused the moment both are started.
    fn next_free_bind(&self, bind: &str) -> String {
        let (prefix, port) = match bind.rsplit_once(':') {
            Some((address, port)) => (format!("{address}:"), port.trim().parse::<u16>().ok()),
            None => (String::new(), bind.trim().parse::<u16>().ok()),
        };
        let Some(port) = port else {
            return bind.to_string();
        };
        let taken = self.taken_binds();
        let mut candidate = port;
        loop {
            let Some(next) = candidate.checked_add(1) else {
                return bind.to_string();
            };
            candidate = next;
            if !taken.contains(&candidate) {
                return format!("{prefix}{candidate}");
            }
        }
    }

    /// Exit ports already forwarded **on this host**, for flagging a discovered
    /// port that is already covered.
    ///
    /// Filtered by host on purpose: kami's 8080 and thor's 8080 are different
    /// services, and a bare port match would tell you one is already covered
    /// when nothing forwards it at all.
    pub fn exits_on(&self, host: &str) -> HashSet<u16> {
        self.tunnels
            .profiles
            .iter()
            .flat_map(|profile| &profile.forwards)
            .filter(|forward| forward.host == host)
            .filter_map(|forward| super::tunnels::port_of(&forward.target))
            .collect()
    }

    /// Every port some rule already listens on. Two rules cannot share one.
    fn taken_binds(&self) -> HashSet<u16> {
        self.tunnels
            .profiles
            .iter()
            .flat_map(|profile| &profile.forwards)
            .filter_map(|forward| super::tunnels::port_of(&forward.bind))
            .collect()
    }

    fn group_name(&self, profile: usize) -> String {
        self.tunnels
            .profiles
            .get(profile)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| "default".to_string())
    }

    pub fn forward_form_begin_edit(&mut self) {
        let config = &self.config;
        let groups: Vec<String> = self
            .tunnels
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect();
        if let Some(form) = self.forward_form.as_mut() {
            form.begin_edit(config, &groups);
        }
    }

    /// Fold the open rule back into its group, then write the whole file.
    pub fn save_forward_form(&mut self) {
        if let Err(problem) = self.apply_forward_form() {
            self.notify(problem);
            return;
        }
        match self.tunnels.save() {
            Ok(path) => {
                self.forward_form = None;
                self.notify(format!("Saved {}", path.display()));
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    /// Fold the open rule back into its group, in memory only.
    ///
    /// Separate from the write so the placement rules can be tested without a
    /// test run overwriting the real `tunnels.yaml`.
    ///
    /// The group is a value on the form, so this may also move the rule to
    /// another group or bring a new group into being. Groups are never created
    /// on their own: one exists exactly as long as it holds a rule.
    pub(super) fn apply_forward_form(&mut self) -> Result<(), String> {
        let Some(form) = &self.forward_form else {
            return Err("nothing to save".to_string());
        };
        let forward = form.to_forward();
        if let Some(problem) = forward.problem() {
            return Err(problem);
        }
        let group = form.group().to_string();
        if group.is_empty() {
            return Err("group is required".to_string());
        }
        let (origin, index) = (form.profile, form.index);

        let stays = self
            .tunnels
            .profiles
            .get(origin)
            .is_some_and(|entry| entry.name == group);
        match index {
            Some(i) if stays && i < self.tunnels.profiles[origin].forwards.len() => {
                self.tunnels.profiles[origin].forwards[i] = forward;
            }
            _ => {
                // Take it out of where it was before looking up where it goes,
                // so a move cannot leave a copy behind.
                if let Some(i) = index
                    && let Some(entry) = self.tunnels.profiles.get_mut(origin)
                    && i < entry.forwards.len()
                {
                    entry.forwards.remove(i);
                }
                let target = match self
                    .tunnels
                    .profiles
                    .iter()
                    .position(|entry| entry.name == group)
                {
                    Some(found) => found,
                    None => {
                        self.tunnels.profiles.push(Profile {
                            name: group,
                            forwards: Vec::new(),
                        });
                        self.tunnels.profiles.len() - 1
                    }
                };
                self.tunnels.profiles[target].forwards.push(forward);
                self.tunnels
                    .profiles
                    .retain(|entry| !entry.forwards.is_empty());
                self.forward_index = self
                    .forward_index
                    .min(self.tunnels.count().saturating_sub(1));
                // A move reorders the rules around it, and marks name positions.
                self.marked.clear();
            }
        }
        Ok(())
    }

    pub fn delete_forward(&mut self) {
        self.remove_selected_forward();
        match self.tunnels.save() {
            Ok(_) => self.notify("Deleted"),
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    /// Drop the selected rule from the model. Split from the write for the same
    /// reason as [`Self::apply_forward_form`].
    pub(super) fn remove_selected_forward(&mut self) {
        let Some(&(profile, index)) = self.tunnels.all().get(self.forward_index) else {
            return;
        };
        if let Some(target) = self.tunnels.profiles.get_mut(profile) {
            target.forwards.remove(index);
        }
        // Dropping the last rule of a profile leaves an empty heading behind.
        self.tunnels.profiles.retain(|p| !p.forwards.is_empty());
        self.forward_index = self
            .forward_index
            .min(self.tunnels.count().saturating_sub(1));
        // Everything past the hole shifted down one, so every mark now names a
        // different rule than the one that was picked.
        self.marked.clear();
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

    /// Open the selected block's own lines for free-text editing.
    pub fn open_block_edit(&mut self) {
        let Some(&index) = self.filtered_indices.get(self.selected_index) else {
            self.notify("No host selected");
            return;
        };
        self.effective = None;
        self.block = BlockEdit::new(&self.config, index);
    }

    /// Write the edited block back, through the same syntax check as the form.
    ///
    /// Free text is exactly where a misspelling like `ServerAliveCountMx` gets
    /// in, and OpenSSH stops reading the file at that line -- so this is the
    /// path that needs [`effective::check`] most.
    pub fn save_block_edit(&mut self) {
        let Some(block) = &self.block else { return };
        let plan = block.plan(&self.config);
        if plan.changes.is_empty() {
            self.notify("No changes");
            return;
        }
        let alias = block.alias.clone();
        let caveat = match effective::check(&self.config, &plan.lines, &alias) {
            effective::Verdict::Rejected(why) => {
                self.notify(format!("Not saved -- ssh will not parse it: {why}"));
                return;
            }
            effective::Verdict::Skipped(why) => format!("   (unchecked: {why})"),
            effective::Verdict::Ok => String::new(),
        };
        match write_config(&self.config, &plan.lines) {
            Ok(backup) => {
                self.block = None;
                self.load_config();
                self.select_alias(&alias);
                self.notify(format!(
                    "Saved. U: undo the session. {} = before this save{caveat}",
                    backup.display()
                ));
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    pub fn open_new_host(&mut self) {
        self.effective = None;
        self.form = Some(HostForm::creating(&self.config));
    }

    /// Open a copy of the selected host, for a family that differs in one field.
    pub fn clone_host(&mut self) {
        let Some(&index) = self.filtered_indices.get(self.selected_index) else {
            self.notify("No host selected");
            return;
        };
        self.effective = None;
        self.form = HostForm::cloning(&self.config, index);
    }

    /// Why a new block's alias will not do.
    ///
    /// A duplicate is the one that matters: OpenSSH takes the first matching
    /// block, so the new one would be written, appear in the list, and do
    /// absolutely nothing -- the exact silent failure this module exists to
    /// surface, and it would be this tool creating it.
    fn new_alias_problem(&self, form: &HostForm) -> Option<String> {
        let alias = form.value(Field::Alias).trim();
        if alias.is_empty() {
            return Some("Not saved -- the host needs a name".to_string());
        }
        if alias.split_whitespace().count() > 1 {
            return Some("Not saved -- one name per block here".to_string());
        }
        let clash = self
            .config
            .hosts
            .iter()
            .position(|host| host.patterns.iter().any(|p| p == alias))?;
        Some(format!(
            "Not saved -- line {} already matches `{alias}`; the new block would do nothing",
            self.config.hosts[clash].start + 1
        ))
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
        if form.creating
            && let Some(problem) = self.new_alias_problem(form)
        {
            self.notify(problem);
            return;
        }
        let plan = form.plan(&self.config);
        if plan.changes.is_empty() {
            self.notify("No changes");
            return;
        }
        let alias = form.value(Field::Alias).to_string();

        // Checked before the write, not after: OpenSSH stops reading a config at
        // the first line it cannot parse, so one bad value silently disables
        // every block below it and nothing says so until the next connection.
        let caveat = match effective::check(&self.config, &plan.lines, &alias) {
            effective::Verdict::Rejected(why) => {
                self.notify(format!("Not saved -- ssh will not parse it: {why}"));
                return;
            }
            // Saving still goes ahead, but silence here would turn "saves are
            // checked" into a promise that is quietly not kept.
            effective::Verdict::Skipped(why) => format!("   (unchecked: {why})"),
            effective::Verdict::Ok => String::new(),
        };

        match write_config(&self.config, &plan.lines) {
            Ok(backup) => {
                self.form = None;
                self.load_config();
                self.select_alias(&alias);
                // "Backup: <path>" read as "your original is safe there". It is
                // overwritten every save, so after the second one it is not.
                self.notify(format!(
                    "Saved. U: undo the session. {} = before this save{caveat}",
                    backup.display()
                ));
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    /// Ask the selected host what it is listening on.
    ///
    /// The answer takes a network round trip, so it goes through the worker and
    /// the screen opens straight away in its waiting state -- a UI that freezes
    /// for several seconds is indistinguishable from one that has hung.
    pub fn discover_selected_host(&mut self) {
        let Some(host) = self.selected_host().map(|h| h.alias().to_string()) else {
            self.notify("No host selected");
            return;
        };
        self.worker.submit(Job::Discover(host.clone()));
        self.discovery = Some(Discovery {
            host,
            listeners: Vec::new(),
            marked: HashSet::new(),
            cursor: 0,
            asking: true,
            error: None,
        });
        self.screen = Screen::Discover;
    }

    /// Take an answer that came back over the channel.
    ///
    /// Dropped unless the screen is still open on that same host: an ssh round
    /// trip can outlive the screen that asked, and a late answer must not
    /// repopulate one already closed or now asking about somebody else.
    pub(super) fn apply_discovery(&mut self, host: String, found: Result<Vec<Listener>, String>) {
        let Some(open) = &mut self.discovery else {
            return;
        };
        if open.host != host {
            return;
        }
        open.asking = false;
        match found {
            Ok(listeners) => {
                open.listeners = listeners;
                open.cursor = 0;
            }
            Err(e) => open.error = Some(e),
        }
    }

    pub fn discover_next(&mut self) {
        if let Some(found) = &mut self.discovery
            && !found.listeners.is_empty()
        {
            found.cursor = (found.cursor + 1) % found.listeners.len();
        }
    }

    pub fn discover_previous(&mut self) {
        if let Some(found) = &mut self.discovery
            && !found.listeners.is_empty()
        {
            let len = found.listeners.len();
            found.cursor = (found.cursor + len - 1) % len;
        }
    }

    pub fn toggle_discovered(&mut self) {
        let Some(found) = &mut self.discovery else {
            return;
        };
        let Some(port) = found.listeners.get(found.cursor).map(|l| l.port) else {
            return;
        };
        if !found.marked.remove(&port) {
            found.marked.insert(port);
        }
    }

    /// Mark every loopback-only port, which is usually the whole reason for
    /// being on this screen.
    pub fn mark_loopback_ports(&mut self) {
        let Some(found) = &mut self.discovery else {
            return;
        };
        let loopback: HashSet<u16> = found
            .listeners
            .iter()
            .filter(|l| l.loopback)
            .map(|l| l.port)
            .collect();
        if loopback.iter().all(|port| found.marked.contains(port)) {
            found.marked.retain(|port| !loopback.contains(port));
        } else {
            found.marked.extend(loopback);
        }
    }

    /// Turn the marked ports into `-L` rules in a group named after the host.
    ///
    /// The whole point of the screen: five ports found on kami become five
    /// rules in one keystroke, instead of six fields typed five times.
    pub fn adopt_discovered(&mut self) {
        let count = match self.take_discovered() {
            Ok(count) => count,
            Err(why) => return self.notify(why),
        };
        let host = self.discovery.take().map(|f| f.host).unwrap_or_default();
        match self.tunnels.save() {
            Ok(_) => {
                self.screen = Screen::Forward;
                self.notify(format!("Added {count} rule(s) to group `{host}`"));
            }
            Err(e) => self.notify(format!("Save failed: {e}")),
        }
    }

    /// Fold the marked ports into the model. Split from the write for the same
    /// reason as [`Self::apply_forward_form`].
    pub(super) fn take_discovered(&mut self) -> Result<usize, String> {
        let Some(found) = &self.discovery else {
            return Err("nothing to add".to_string());
        };
        if found.marked.is_empty() {
            return Err("Nothing marked -- Space picks a port".to_string());
        }
        let host = found.host.clone();
        // Sorted, so the rules land in the order they were shown rather than in
        // whatever order the set iterates.
        let mut ports: Vec<u16> = found.marked.iter().copied().collect();
        ports.sort_unstable();

        let group = match self.tunnels.profiles.iter().position(|p| p.name == host) {
            Some(existing) => existing,
            None => {
                self.tunnels.profiles.push(Profile {
                    name: host.clone(),
                    forwards: Vec::new(),
                });
                self.tunnels.profiles.len() - 1
            }
        };
        for port in &ports {
            // Same number locally where nothing has claimed it -- one number to
            // remember instead of two.
            let bind = self.free_bind_from(*port);
            self.tunnels.profiles[group].forwards.push(Forward {
                host: host.clone(),
                kind: Kind::Local,
                bind,
                target: format!("localhost:{port}"),
                protocol: Protocol::Tcp,
                note: format!("found listening on {host}"),
            });
        }
        // Positions after the new rules have all shifted.
        self.marked.clear();
        Ok(ports.len())
    }

    /// `port` itself when no rule listens on it, else the next one that is free.
    fn free_bind_from(&self, port: u16) -> String {
        let taken = self.taken_binds();
        let mut candidate = port;
        while taken.contains(&candidate) {
            let Some(next) = candidate.checked_add(1) else {
                return port.to_string();
            };
            candidate = next;
        }
        candidate.to_string()
    }

    /// Whether the file differs from the one this session started on.
    pub fn can_undo(&self) -> bool {
        self.pristine
            .as_ref()
            .is_some_and(|original| *original != self.config.lines)
    }

    /// Put the file back the way it was when the module was entered.
    ///
    /// Deliberately not gated on [`effective::check`]: the content being
    /// restored is what was already on disk, so it cannot be worse than what is
    /// there now -- and a config that was broken before this session started
    /// would otherwise be impossible to get back to.
    ///
    /// This is itself undoable: `write_config` backs up the current file first,
    /// so the edits being discarded land in `config.excalibur.bak`.
    pub fn undo_config(&mut self) {
        let Some(original) = self.pristine.clone() else {
            self.notify("Nothing to undo -- the config was never read");
            return;
        };
        if !self.can_undo() {
            self.notify("Nothing to undo -- unchanged since you came in");
            return;
        }
        match write_config(&self.config, &original) {
            Ok(backup) => {
                self.form = None;
                self.effective = None;
                self.load_config();
                self.notify(format!("Undone. Your edits are in {}", backup.display()));
            }
            Err(e) => self.notify(format!("Undo failed: {e}")),
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

/// Dashboard: process discovery, probing, and start/stop.
impl SshState {
    /// Called every tick. Process and local socket observation are cheap enough
    /// to do inline; only SSH start/stop and remote discovery use the worker.
    pub fn poll_tunnels(&mut self) {
        for outcome in self.worker.drain() {
            match outcome {
                Outcome::Started(slot, Err(e)) => {
                    let what = self.describe(slot);
                    self.notify(format!("{what} failed to start: {e}"));
                }
                Outcome::Stopped(slot, Err(e)) => {
                    let what = self.describe(slot);
                    self.notify(format!("{what} failed to stop: {e}"));
                }
                Outcome::StoppedOrphan(pid, Err(e)) => {
                    self.notify(format!("pid {pid} failed to stop: {e}"));
                }
                Outcome::Started(..) | Outcome::Stopped(..) | Outcome::StoppedOrphan(..) => {
                    // Reflect it immediately rather than waiting out the timer.
                    self.last_scan = None;
                }
                Outcome::Discovered(host, found) => self.apply_discovery(host, found),
            }
        }

        if due(self.last_scan, SCAN_INTERVAL) {
            self.running = supervisor::scan();
            self.last_scan = Some(Instant::now());
            self.sample_meters();
            self.refresh_health();
            // A rule with no process has nothing left to measure. Collected
            // first because `pid_at` borrows the state the retain would own.
            let live: Vec<Slot> = self
                .tunnels
                .all()
                .into_iter()
                .filter(|slot| self.pid_at(*slot).is_some())
                .collect();
            self.health.retain(|slot, _| live.contains(slot));
            // Orphans come and go with the scan, so the last row can disappear
            // out from under the cursor.
            self.forward_index = self.forward_index.min(self.cursor_span().saturating_sub(1));
        }
        // Health is intentionally local-only. Do not connect to the forwarded
        // service just to decide whether the dashboard may show its URL.
    }

    /// Re-read every claimed process and turn the difference into a rate.
    ///
    /// Only pids that came out of `scan()` are read, so every measurement is of
    /// a process already confirmed by its argv to be an ssh forward. Reading a
    /// pid from anywhere else would risk charting a process that is not the one
    /// meant -- and nothing about the numbers would look wrong.
    pub(super) fn sample_meters(&mut self) {
        let now = Instant::now();
        let mut fresh = HashMap::with_capacity(self.running.len());
        for process in &self.running {
            let Some(usage) = supervisor::usage(process.pid) else {
                continue;
            };
            // A pid reused by a different process would show as a huge negative
            // delta; `checked_sub` drops it rather than inventing a rate. A
            // platform with no byte counter drops out here for the same reason.
            let rate = self.meters.get(&process.pid).and_then(|last| {
                let bytes = usage.read?.checked_sub(last.usage.read?)?;
                let elapsed = now.duration_since(last.at).as_secs_f64();
                (elapsed > 0.0).then(|| bytes as f64 / elapsed)
            });
            fresh.insert(
                process.pid,
                Meter {
                    usage,
                    rate,
                    at: now,
                },
            );
        }
        // Whatever is not in `fresh` has exited; keeping it would leave a dead
        // process's last rate on screen looking live.
        self.meters = fresh;
    }

    /// What the process behind `pid` has been doing, once there is a reading.
    pub fn meter(&self, pid: u32) -> Option<&Meter> {
        self.meters.get(&pid)
    }

    pub fn refresh_health(&mut self) {
        let rules: Vec<(Slot, Forward)> = self
            .tunnels
            .all()
            .into_iter()
            .filter(|slot| self.pid_at(*slot).is_some())
            .filter_map(|slot| Some((slot, self.tunnels.get(slot.0, slot.1)?.clone())))
            .collect();
        self.health = rules
            .into_iter()
            .map(|(slot, forward)| (slot, probe::observe(&forward)))
            .collect();
    }

    /// The process serving the rule at `slot`, if any.
    pub fn pid_at(&self, slot: Slot) -> Option<u32> {
        let forward = self.tunnels.get(slot.0, slot.1)?;
        supervisor::find(&self.running, forward).map(|r| r.pid)
    }

    pub fn health_at(&self, slot: Slot) -> Health {
        match self.pid_at(slot) {
            None => Health::stopped(),
            Some(_) => self
                .health
                .get(&slot)
                .cloned()
                .unwrap_or_else(Health::measuring),
        }
    }

    /// Open the selected local HTTP forward in the default browser.
    pub fn open_selected_http(&mut self) {
        let Some(slot) = self.selected_slot() else {
            self.notify("No forward selected");
            return;
        };
        let Some(port) = self
            .tunnels
            .get(slot.0, slot.1)
            .filter(|forward| {
                forward.protocol == Protocol::Http
                    && forward.kind == Kind::Local
                    && self.health_at(slot).port == Light::Ok
            })
            .and_then(|forward| super::tunnels::port_of(&forward.bind))
        else {
            self.notify("Selected forward is not identified as HTTP");
            return;
        };
        let url = format!("http://127.0.0.1:{port}");
        match std::process::Command::new("open").arg(&url).status() {
            Ok(status) if status.success() => self.notify(format!("Opened {url}")),
            Ok(status) => self.notify(format!("Browser failed to open {url}: {status}")),
            Err(error) => self.notify(format!("Could not open {url}: {error}")),
        }
    }

    /// Copy the selected local HTTP forward's URL to the system clipboard.
    pub fn copy_selected_http(&mut self) {
        let Some(slot) = self.selected_slot() else {
            self.notify("No forward selected");
            return;
        };
        let Some(port) = self
            .tunnels
            .get(slot.0, slot.1)
            .filter(|forward| {
                forward.protocol == Protocol::Http
                    && forward.kind == Kind::Local
                    && self.health_at(slot).port == Light::Ok
            })
            .and_then(|forward| super::tunnels::port_of(&forward.bind))
        else {
            self.notify("Selected forward is not identified as HTTP");
            return;
        };
        let url = format!("http://127.0.0.1:{port}");
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(&url)) {
            Ok(()) => self.notify(format!("Copied {url}")),
            Err(error) => self.notify(format!("Could not copy {url}: {error}")),
        }
    }

    /// Tunnel processes that no rule accounts for.
    ///
    /// Editing a rule while it runs strands its process: `find()` matches on
    /// `(kind, spec, host)`, so changing any of the three leaves the old process
    /// alive and still holding the port while the rule reads as stopped. The
    /// port is taken and nothing on screen says by what -- which is the failure
    /// this module exists to make visible.
    ///
    /// Defined as what `scan()` saw minus what `find()` claimed. There is no
    /// pattern-matching fallback for the rest, and there must not be: a pattern
    /// broad enough to catch them also catches the process doing the matching
    /// (see ~/.claude/remote-ops.md).
    pub fn orphans(&self) -> Vec<&Running> {
        let claimed: HashSet<u32> = self
            .tunnels
            .all()
            .into_iter()
            .filter_map(|slot| self.pid_at(slot))
            .collect();
        self.running
            .iter()
            .filter(|running| !claimed.contains(&running.pid))
            .collect()
    }

    /// The orphan under the cursor, which is any row past the last rule.
    pub fn selected_orphan(&self) -> Option<&Running> {
        let past = self.forward_index.checked_sub(self.tunnels.count())?;
        self.orphans().into_iter().nth(past)
    }

    /// Stop the orphan under the cursor. Reports whether there was one, so the
    /// caller can fall through to the rule action when there was not.
    pub fn stop_selected_orphan(&mut self) -> bool {
        let Some(pid) = self.selected_orphan().map(|running| running.pid) else {
            return false;
        };
        self.worker.submit(Job::StopOrphan(pid));
        self.notify(format!("Stopping pid {pid}"));
        true
    }

    pub fn selected_slot(&self) -> Option<Slot> {
        self.tunnels.all().get(self.forward_index).copied()
    }

    /// Start the rule if it is down, stop it if it is up.
    pub fn toggle_selected_tunnel(&mut self) {
        let Some(slot) = self.selected_slot() else {
            return;
        };
        match self.pid_at(slot) {
            Some(pid) => {
                self.worker.submit(Job::Stop(slot, pid));
                self.notify("Stopping");
            }
            None => {
                let Some(forward) = self.tunnels.get(slot.0, slot.1).cloned() else {
                    return;
                };
                // Say why up front rather than relaying ssh's complaint about a
                // malformed forward specification.
                if let Some(problem) = forward.problem() {
                    self.notify(problem);
                    return;
                }
                self.worker.submit(Job::Start(slot, forward));
                self.notify("Starting");
            }
        }
    }

    /// Mark or unmark the rule under the cursor.
    pub fn toggle_mark(&mut self) {
        let Some(slot) = self.selected_slot() else {
            return;
        };
        if !self.marked.remove(&slot) {
            self.marked.insert(slot);
        }
    }

    /// Mark the cursor's whole group, or clear it if it is already all marked.
    pub fn toggle_group_mark(&mut self) {
        let Some(profile) = self.selected_profile() else {
            return;
        };
        let group: Vec<Slot> = self
            .tunnels
            .all()
            .into_iter()
            .filter(|slot| slot.0 == profile)
            .collect();
        if group.iter().all(|slot| self.marked.contains(slot)) {
            group.iter().for_each(|slot| {
                self.marked.remove(slot);
            });
        } else {
            self.marked.extend(group);
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    /// What a start/stop acts on: everything marked, or the cursor when nothing
    /// is.
    ///
    /// Walked through `all()` so the order is the one on screen and so a mark
    /// left pointing past the end of a shortened file simply drops out.
    pub fn scope(&self) -> Vec<Slot> {
        if self.marked.is_empty() {
            return self.selected_slot().into_iter().collect();
        }
        self.tunnels
            .all()
            .into_iter()
            .filter(|slot| self.marked.contains(slot))
            .collect()
    }

    pub fn start_scope(&mut self) {
        let scope = self.scope();
        self.start_slots(&scope);
    }

    pub fn stop_scope(&mut self) {
        let scope = self.scope();
        self.stop_slots(&scope);
    }

    pub fn start_all(&mut self) {
        let all = self.tunnels.all();
        self.start_slots(&all);
    }

    pub fn stop_all(&mut self) {
        let all = self.tunnels.all();
        self.stop_slots(&all);
    }

    fn start_slots(&mut self, slots: &[Slot]) {
        let (mut queued, mut skipped) = (0, 0);
        for &slot in slots {
            if self.pid_at(slot).is_some() {
                continue;
            }
            let Some(forward) = self.tunnels.get(slot.0, slot.1).cloned() else {
                continue;
            };
            if forward.problem().is_some() {
                skipped += 1;
                continue;
            }
            self.worker.submit(Job::Start(slot, forward));
            queued += 1;
        }
        // Never silently drop one: a rule that is simply skipped reads as a rule
        // that failed for no reason.
        self.notify(match (queued, skipped) {
            (0, 0) => "Nothing to start -- already up".to_string(),
            (0, s) => format!("{s} rule(s) are incomplete; nothing to start"),
            (q, 0) => format!("Starting {q}"),
            (q, s) => format!("Starting {q}, skipped {s} incomplete"),
        });
    }

    fn stop_slots(&mut self, slots: &[Slot]) {
        let mut queued = 0;
        for &slot in slots {
            if let Some(pid) = self.pid_at(slot) {
                self.worker.submit(Job::Stop(slot, pid));
                queued += 1;
            }
        }
        self.notify(format!("Stopping {queued}"));
    }

    pub fn refresh_tunnels(&mut self) {
        self.running = supervisor::scan();
        self.last_scan = Some(Instant::now());
        self.refresh_health();
    }

    /// How many rules have a process behind them.
    pub fn live_count(&self) -> usize {
        self.tunnels
            .all()
            .into_iter()
            .filter(|slot| self.pid_at(*slot).is_some())
            .count()
    }

    /// Rules that are up, down, and unrunnable.
    ///
    /// Incomplete is counted apart from stopped because "1 of 3 up" makes a rule
    /// ssh would refuse look exactly like one you simply have not started.
    pub fn tallies(&self) -> (usize, usize, usize) {
        let (mut up, mut stopped, mut incomplete) = (0, 0, 0);
        for slot in self.tunnels.all() {
            match self.tunnels.get(slot.0, slot.1) {
                Some(rule) if rule.problem().is_some() => incomplete += 1,
                Some(_) if self.pid_at(slot).is_some() => up += 1,
                Some(_) => stopped += 1,
                None => {}
            }
        }
        (up, stopped, incomplete)
    }

    /// Bytes per second across every ssh forward on this machine, once at least
    /// one of them has been measured twice.
    pub fn total_rate(&self) -> Option<f64> {
        let rates: Vec<f64> = self.meters.values().filter_map(|m| m.rate).collect();
        (!rates.is_empty()).then(|| rates.iter().sum())
    }

    /// Live and total rule counts for one profile, for its heading.
    pub fn profile_status(&self, profile: usize) -> (usize, usize) {
        let Some(entry) = self.tunnels.profiles.get(profile) else {
            return (0, 0);
        };
        let live = (0..entry.forwards.len())
            .filter(|forward| self.pid_at((profile, *forward)).is_some())
            .count();
        (live, entry.forwards.len())
    }

    /// The profile the cursor currently sits in, so its heading can say so.
    pub fn selected_profile(&self) -> Option<usize> {
        self.selected_slot().map(|slot| slot.0)
    }

    /// Names a rule in a message, so a failure says which one.
    fn describe(&self, slot: Slot) -> String {
        self.tunnels
            .get(slot.0, slot.1)
            .map(|f| f.summary())
            .unwrap_or_else(|| "tunnel".to_string())
    }
}

fn due(last: Option<Instant>, interval: Duration) -> bool {
    last.is_none_or(|at| at.elapsed() >= interval)
}
