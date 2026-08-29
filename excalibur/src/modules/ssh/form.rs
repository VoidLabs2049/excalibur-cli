use super::sshconfig::{SshConfig, proxy_command_gateway_span};
use super::tunnels::{Forward, Kind};
use color_eyre::Result;
use color_eyre::eyre::bail;
use std::path::{Path, PathBuf};
use tui_textarea::TextArea;

/// The fields the form exposes, chosen by how often they appear in real configs
/// rather than by what ssh_config supports. Everything else stays in the block
/// untouched and is surfaced read-only as "other directives".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Alias,
    HostName,
    User,
    Port,
    Gateway,
    IdentityFile,
}

impl Field {
    pub const ALL: [Field; 6] = [
        Field::Alias,
        Field::HostName,
        Field::User,
        Field::Port,
        Field::Gateway,
        Field::IdentityFile,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Alias => "Alias",
            Field::HostName => "HostName",
            Field::User => "User",
            Field::Port => "Port",
            Field::Gateway => "Jump host",
            Field::IdentityFile => "IdentityFile",
        }
    }

    /// The keyword this field writes when it maps to one directly. `Alias` is
    /// the `Host` header, and `Gateway` is either `ProxyJump` or an existing
    /// `ProxyCommand`, so both are handled on their own.
    fn keyword(self) -> Option<&'static str> {
        match self {
            Field::HostName => Some("HostName"),
            Field::User => Some("User"),
            Field::Port => Some("Port"),
            Field::IdentityFile => Some("IdentityFile"),
            Field::Alias | Field::Gateway => None,
        }
    }

    /// Fields whose value is picked from what the config already contains.
    fn is_pick(self) -> bool {
        matches!(self, Field::User | Field::Gateway | Field::IdentityFile)
    }
}

/// One line the save will touch, for the confirmation diff.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Replaced {
        line: usize,
        before: String,
        after: String,
    },
    Removed {
        line: usize,
        before: String,
    },
    Inserted {
        line: usize,
        after: String,
    },
}

/// The file contents a save would produce, plus what changed.
#[derive(Debug, Default)]
pub struct Plan {
    pub lines: Vec<String>,
    pub changes: Vec<Change>,
}

pub enum Editing {
    Text(Box<TextArea<'static>>),
    Pick { options: Vec<String>, index: usize },
}

impl std::fmt::Debug for Editing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Editing::Text(_) => f.write_str("Text(..)"),
            Editing::Pick { options, index } => f
                .debug_struct("Pick")
                .field("options", options)
                .field("index", index)
                .finish(),
        }
    }
}

#[derive(Debug)]
pub struct HostForm {
    pub host_index: usize,
    pub values: Vec<String>,
    original: Vec<String>,
    pub cursor: usize,
    pub editing: Option<Editing>,
    /// Directives the form does not model. Shown so that a block with extras is
    /// visibly more than the six fields suggest.
    pub other: Vec<String>,
    /// The `Host` header carries more than one pattern, so which one the alias
    /// field would rename is ambiguous; it is displayed read-only instead.
    pub alias_locked: bool,
    /// This block is not in the file yet, so the save appends it rather than
    /// editing lines in place. `host_index` then points one past the end and is
    /// only good for excluding "self" from the candidate lists.
    pub creating: bool,
}

impl HostForm {
    pub fn new(config: &SshConfig, host_index: usize) -> Option<Self> {
        let host = config.hosts.get(host_index)?;
        let values: Vec<String> = Field::ALL
            .iter()
            .map(|field| match field {
                Field::Alias => host.alias().to_string(),
                Field::Gateway => host.gateway().unwrap_or_default(),
                _ => field
                    .keyword()
                    .and_then(|k| host.get(k))
                    .map(|d| d.value.clone())
                    .unwrap_or_default(),
            })
            .collect();

        let modelled = [
            "HostName",
            "User",
            "Port",
            "IdentityFile",
            "ProxyJump",
            "ProxyCommand",
        ];
        let other = host
            .directives
            .iter()
            .filter(|d| !modelled.iter().any(|k| d.key.eq_ignore_ascii_case(k)))
            .map(|d| format!("{} {}", d.key, d.value))
            .collect();

        Some(HostForm {
            host_index,
            original: values.clone(),
            values,
            cursor: 0,
            editing: None,
            other,
            alias_locked: host.patterns.len() != 1,
            creating: false,
        })
    }

    /// A block that does not exist yet.
    ///
    /// Prefilled with `Port 22` and whichever `User` this config uses most --
    /// between them that is two of the six fields on almost every host here,
    /// and both are one keystroke to change.
    pub fn creating(config: &SshConfig) -> Self {
        let mut values = vec![String::new(); Field::ALL.len()];
        values[index_of(Field::Port)] = "22".to_string();
        values[index_of(Field::User)] = most_common(config, "User");
        HostForm {
            host_index: config.hosts.len(),
            // Nothing is on disk, so every filled field is a change.
            original: vec![String::new(); Field::ALL.len()],
            values,
            cursor: 0,
            editing: None,
            other: Vec::new(),
            alias_locked: false,
            creating: true,
        }
    }

    /// A copy of an existing block, minus its alias.
    ///
    /// The alias is blanked rather than suffixed: two blocks sharing one makes
    /// the second inert, which is the silent failure this module exists to
    /// surface, so it must be typed rather than guessed at.
    pub fn cloning(config: &SshConfig, host_index: usize) -> Option<Self> {
        let mut form = HostForm::new(config, host_index)?;
        form.values[index_of(Field::Alias)] = String::new();
        form.original = vec![String::new(); Field::ALL.len()];
        form.host_index = config.hosts.len();
        form.alias_locked = false;
        form.creating = true;
        // Directives the six fields do not model belong to the block being
        // copied from; carrying them over would write lines the form cannot
        // show, let alone edit.
        form.other.clear();
        form.cursor = index_of(Field::Alias);
        Some(form)
    }

    pub fn field(&self) -> Field {
        Field::ALL[self.cursor]
    }

    pub fn value(&self, field: Field) -> &str {
        &self.values[Field::ALL.iter().position(|f| *f == field).unwrap()]
    }

    pub fn is_dirty(&self) -> bool {
        self.values != self.original
    }

    pub fn next_field(&mut self) {
        self.cursor = (self.cursor + 1) % Field::ALL.len();
    }

    pub fn previous_field(&mut self) {
        self.cursor = (self.cursor + Field::ALL.len() - 1) % Field::ALL.len();
    }

    /// Open the editor for the current field: a picker when the config already
    /// supplies the plausible values, a text box otherwise.
    pub fn begin_edit(&mut self, config: &SshConfig) {
        let field = self.field();
        if field == Field::Alias && self.alias_locked {
            return;
        }
        self.editing = Some(if field.is_pick() {
            let mut options = candidates(config, field, self.host_index);
            options.insert(0, String::new()); // "(none)" clears the field
            let current = self.values[self.cursor].clone();
            let index = options.iter().position(|o| *o == current).unwrap_or(0);
            Editing::Pick { options, index }
        } else {
            Editing::Text(Box::new(text_area(&self.values[self.cursor])))
        });
    }

    /// Accept the value being edited. A picker landing on the blank entry, or a
    /// text box left empty, clears the field.
    pub fn commit_edit(&mut self) {
        let value = match self.editing.take() {
            Some(Editing::Text(area)) => area.lines().first().cloned().unwrap_or_default(),
            Some(Editing::Pick { options, index }) => {
                options.get(index).cloned().unwrap_or_default()
            }
            None => return,
        };
        self.values[self.cursor] = value.trim().to_string();
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// Switch a picker to free text, keeping whatever was highlighted.
    pub fn edit_as_text(&mut self) {
        if let Some(Editing::Pick { options, index }) = &self.editing {
            let seed = options.get(*index).cloned().unwrap_or_default();
            self.editing = Some(Editing::Text(Box::new(text_area(&seed))));
        }
    }

    /// Compute the file the form would write.
    ///
    /// Only the lines the six fields own are touched. Anything else in the block
    /// -- `ServerAliveInterval`, comments, odd indentation -- is carried through
    /// byte for byte, because dropping a directive here changes what ssh does
    /// and nothing reports it.
    pub fn plan(&self, config: &SshConfig) -> Plan {
        if self.creating {
            return self.plan_new_block(config);
        }
        let Some(host) = config.hosts.get(self.host_index) else {
            return Plan::default();
        };
        let mut replacements: Vec<(usize, String)> = Vec::new();
        let mut removals: Vec<usize> = Vec::new();
        let mut insertions: Vec<String> = Vec::new();
        let indent = block_indent(config, host.start, host.end);

        for (i, field) in Field::ALL.iter().enumerate() {
            let new = self.values[i].trim();
            if new == self.original[i].trim() {
                continue;
            }

            match field {
                Field::Alias => {
                    if !self.alias_locked {
                        let line = &config.lines[host.start];
                        replacements.push((host.start, replace_value(line, new)));
                    }
                }
                Field::Gateway => {
                    if let Some(d) = host.get("ProxyCommand") {
                        let line = &config.lines[d.line];
                        if new.is_empty() {
                            removals.push(d.line);
                        } else if let Some((start, end)) = proxy_command_gateway_span(&d.value) {
                            // Splice into the command so the ssh binary path and
                            // every other flag survive untouched.
                            let mut value = d.value.clone();
                            value.replace_range(start..end, new);
                            replacements.push((d.line, replace_value(line, &value)));
                        } else {
                            replacements.push((d.line, replace_value(line, new)));
                        }
                    } else if let Some(d) = host.get("ProxyJump") {
                        if new.is_empty() {
                            removals.push(d.line);
                        } else {
                            replacements.push((d.line, replace_value(&config.lines[d.line], new)));
                        }
                    } else if !new.is_empty() {
                        insertions.push(format!("{indent}ProxyJump {new}"));
                    }
                }
                _ => {
                    let keyword = field.keyword().expect("non-header field has a keyword");
                    match host.get(keyword) {
                        Some(d) if new.is_empty() => removals.push(d.line),
                        Some(d) => {
                            replacements.push((d.line, replace_value(&config.lines[d.line], new)))
                        }
                        None if !new.is_empty() => {
                            insertions.push(format!("{indent}{keyword} {new}"))
                        }
                        None => {}
                    }
                }
            }
        }

        let mut lines = config.lines.clone();
        let mut changes = Vec::new();

        for (line, after) in &replacements {
            changes.push(Change::Replaced {
                line: *line,
                before: lines[*line].clone(),
                after: after.clone(),
            });
            lines[*line] = after.clone();
        }
        for text in &insertions {
            changes.push(Change::Inserted {
                line: host.end,
                after: text.clone(),
            });
        }
        // Splice inserts at the block end first: they sit above nothing that the
        // removals below would shift.
        lines.splice(host.end..host.end, insertions.iter().cloned());

        removals.sort_unstable();
        for line in removals.iter().rev() {
            changes.push(Change::Removed {
                line: *line,
                before: lines[*line].clone(),
            });
            lines.remove(*line);
        }

        changes.sort_by_key(|c| match c {
            Change::Replaced { line, .. } | Change::Removed { line, .. } => *line,
            Change::Inserted { line, .. } => *line,
        });
        Plan { lines, changes }
    }
}

fn text_area(value: &str) -> TextArea<'static> {
    let mut area = TextArea::new(vec![value.to_string()]);
    area.move_cursor(tui_textarea::CursorMove::End);
    area
}

/// Values already present in this config, so that editing is a choice rather
/// than recall. `~/.ssh` is scanned for keys because a fresh one has never been
/// referenced yet.
pub fn candidates(config: &SshConfig, field: Field, host_index: usize) -> Vec<String> {
    let mut values: Vec<String> = match field {
        Field::Gateway => config
            .hosts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != host_index)
            .map(|(_, h)| h.alias().to_string())
            .collect(),
        Field::IdentityFile => {
            let mut files: Vec<String> = config
                .hosts
                .iter()
                .filter_map(|h| h.get("IdentityFile").map(|d| d.value.clone()))
                .collect();
            files.extend(private_keys());
            files
        }
        _ => {
            let keyword = field.keyword().unwrap_or("");
            config
                .hosts
                .iter()
                .filter_map(|h| h.get(keyword).map(|d| d.value.clone()))
                .collect()
        }
    };
    values.retain(|v| !v.is_empty());
    values.sort_unstable();
    values.dedup();
    values
}

fn private_keys() -> Vec<String> {
    let Some(dir) = SshConfig::default_path().and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let skip = [
        "config",
        "known_hosts",
        "known_hosts.old",
        "authorized_keys",
    ];
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.ends_with(".pub") && !skip.contains(&name.as_str()))
        .map(|name| format!("~/.ssh/{name}"))
        .collect()
}

/// Rebuild a directive line with a new value, keeping its indentation, the
/// keyword exactly as spelled, and the separator that was used.
fn replace_value(line: &str, value: &str) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    let key_len = rest.find([' ', '\t', '=']).unwrap_or(rest.len());
    let (key, after) = rest.split_at(key_len);
    let sep_len = after.len() - after.trim_start_matches([' ', '\t', '=']).len();
    let separator = if sep_len == 0 { " " } else { &after[..sep_len] };
    format!("{indent}{key}{separator}{value}")
}

/// Indentation to give a directive this block does not have yet, copied from
/// the block's own lines so an inserted line does not stand out.
impl HostForm {
    /// Append the whole block at the end of the file.
    ///
    /// Appending is what makes this safe to do at all: every existing byte is
    /// untouched by construction, so a new host cannot disturb a block it was
    /// never meant to. OpenSSH matches first-wins, so a block at the end also
    /// cannot shadow anything above it -- only be shadowed, which the list
    /// already flags.
    fn plan_new_block(&self, config: &SshConfig) -> Plan {
        let alias = self.value(Field::Alias).trim();
        if alias.is_empty() {
            return Plan::default();
        }
        let indent = file_indent(config);
        let mut block = vec![format!("Host {alias}")];
        for (i, field) in Field::ALL.iter().enumerate() {
            let value = self.values[i].trim();
            if value.is_empty() || *field == Field::Alias {
                continue;
            }
            // A gateway on a brand new block is written as ProxyJump: the
            // ProxyCommand spelling only exists here to preserve the ones
            // already in the file.
            let keyword = match field {
                Field::Gateway => "ProxyJump",
                other => other.keyword().expect("non-header field has a keyword"),
            };
            block.push(format!("{indent}{keyword} {value}"));
        }

        let mut lines = config.lines.clone();
        // Keep exactly one blank line between blocks. The file ends in an empty
        // element whenever it ends in a newline, so appending naively gives
        // either none or two depending on the file.
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        let at = lines.len();
        lines.extend(block.iter().cloned());
        // Restore the trailing newline the file had.
        lines.push(String::new());

        Plan {
            changes: block
                .into_iter()
                .enumerate()
                .map(|(offset, after)| Change::Inserted {
                    line: at + offset,
                    after,
                })
                .collect(),
            lines,
        }
    }
}

/// The indent this file uses for directives, so a new block matches the rest.
fn file_indent(config: &SshConfig) -> String {
    config
        .lines
        .iter()
        .find(|line| !line.trim().is_empty() && line.starts_with([' ', '\t']))
        .map(|line| line[..line.len() - line.trim_start().len()].to_string())
        .unwrap_or_else(|| "  ".to_string())
}

/// The value a config uses most for `keyword`, for prefilling a new block.
fn most_common(config: &SshConfig, keyword: &str) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for host in &config.hosts {
        if let Some(directive) = host.get(keyword) {
            *counts.entry(directive.value.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        // Ties broken by the value itself so the answer does not move around
        // between runs with the hash order.
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(value, _)| value.to_string())
        .unwrap_or_default()
}

fn index_of(field: Field) -> usize {
    Field::ALL.iter().position(|f| *f == field).unwrap()
}

fn block_indent(config: &SshConfig, start: usize, end: usize) -> String {
    config.lines[start + 1..end]
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| line[..line.len() - line.trim_start().len()].to_string())
        .unwrap_or_else(|| "  ".to_string())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Write the planned lines back, keeping one backup and swapping atomically.
pub fn write_config(config: &SshConfig, lines: &[String]) -> Result<PathBuf> {
    if config.read_only {
        bail!("{} is read-only", config.path.display());
    }
    let backup = sibling(&config.path, ".excalibur.bak");
    if config.path.exists() {
        std::fs::copy(&config.path, &backup)?;
    }

    let temp = sibling(&config.path, ".excalibur.tmp");
    std::fs::write(&temp, lines.join("\n"))?;
    // Carry the original mode over: ssh refuses configs it considers too open,
    // and a fresh file would silently take the umask default instead.
    if let Ok(meta) = std::fs::metadata(&config.path) {
        std::fs::set_permissions(&temp, meta.permissions())?;
    }
    std::fs::rename(&temp, &config.path)?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "Host web\n\
         \x20\x20HostName 52.82.106.84\n\
         \x20\x20User root\n\
         \x20\x20Port 22\n\
         \x20\x20IdentityFile ~/AWS/key.pem\n\
         \x20\x20ServerAliveInterval 30\n\
         \x20\x20ServerAliveCountMax 7200\n\
         \n\
         Host jumped\n\
         \x20\x20HostName 127.0.0.1\n\
         \x20\x20Port 6022\n\
         \x20\x20ProxyCommand /run/current-system/sw/bin/ssh wonder@db -W %h:%p\n";

    fn config(text: &str) -> SshConfig {
        SshConfig::parse(Path::new("/tmp/config"), text)
    }

    fn form(config: &SshConfig, alias: &str) -> HostForm {
        let index = config
            .hosts
            .iter()
            .position(|h| h.alias() == alias)
            .expect("host in fixture");
        HostForm::new(config, index).unwrap()
    }

    fn set(form: &mut HostForm, field: Field, value: &str) {
        let i = Field::ALL.iter().position(|f| *f == field).unwrap();
        form.values[i] = value.to_string();
    }

    #[test]
    fn reads_the_six_fields_off_a_block() {
        let config = config(FIXTURE);
        let form = form(&config, "web");
        assert_eq!(form.value(Field::Alias), "web");
        assert_eq!(form.value(Field::HostName), "52.82.106.84");
        assert_eq!(form.value(Field::User), "root");
        assert_eq!(form.value(Field::Port), "22");
        assert_eq!(form.value(Field::IdentityFile), "~/AWS/key.pem");
        assert_eq!(form.value(Field::Gateway), "");
    }

    #[test]
    fn directives_outside_the_six_fields_are_listed_not_swallowed() {
        let config = config(FIXTURE);
        assert_eq!(
            form(&config, "web").other,
            ["ServerAliveInterval 30", "ServerAliveCountMax 7200"]
        );
    }

    #[test]
    fn an_unedited_form_changes_nothing() {
        let config = config(FIXTURE);
        let plan = form(&config, "web").plan(&config);
        assert!(plan.changes.is_empty());
        assert_eq!(plan.lines.join("\n"), FIXTURE);
    }

    #[test]
    fn editing_a_port_touches_exactly_one_line() {
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::Port, "2222");
        let plan = form.plan(&config);

        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.lines.len(), config.lines.len());
        let differing: Vec<usize> = (0..config.lines.len())
            .filter(|i| config.lines[*i] != plan.lines[*i])
            .collect();
        assert_eq!(differing, [3]);
        assert_eq!(plan.lines[3], "  Port 2222");
    }

    #[test]
    fn unmodelled_directives_survive_an_edit() {
        // The whole point: `windows-via-reverse` style blocks must not lose
        // their extra directives when the form saves.
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::Port, "2222");
        let text = form.plan(&config).lines.join("\n");
        assert!(text.contains("  ServerAliveInterval 30"));
        assert!(text.contains("  ServerAliveCountMax 7200"));
    }

    #[test]
    fn a_new_directive_is_inserted_with_the_blocks_indentation() {
        let config = config("Host a\n    Port 22\n");
        let mut form = form(&config, "a");
        set(&mut form, Field::User, "lxb");
        assert_eq!(
            form.plan(&config).lines,
            ["Host a", "    Port 22", "    User lxb", ""]
        );
    }

    #[test]
    fn clearing_a_field_removes_its_line() {
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::IdentityFile, "");
        let text = form.plan(&config).lines.join("\n");
        assert!(!text.contains("IdentityFile"));
        assert!(text.contains("  ServerAliveInterval 30"));
    }

    #[test]
    fn changing_a_gateway_keeps_the_rest_of_the_proxycommand() {
        let config = config(FIXTURE);
        let mut form = form(&config, "jumped");
        set(&mut form, Field::Gateway, "kami");
        let text = form.plan(&config).lines.join("\n");
        assert!(
            text.contains("  ProxyCommand /run/current-system/sw/bin/ssh kami -W %h:%p"),
            "got: {text}"
        );
    }

    #[test]
    fn adding_a_gateway_to_a_plain_host_writes_proxyjump() {
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::Gateway, "kami");
        assert!(
            form.plan(&config)
                .lines
                .join("\n")
                .contains("  ProxyJump kami")
        );
    }

    #[test]
    fn clearing_a_gateway_drops_the_proxycommand() {
        let config = config(FIXTURE);
        let mut form = form(&config, "jumped");
        set(&mut form, Field::Gateway, "");
        assert!(!form.plan(&config).lines.join("\n").contains("ProxyCommand"));
    }

    #[test]
    fn renaming_writes_the_host_header() {
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::Alias, "www");
        assert_eq!(form.plan(&config).lines[0], "Host www");
    }

    #[test]
    fn a_multi_pattern_header_is_not_renamed() {
        let config = config("Host a b\n  Port 22\n");
        let mut form = form(&config, "a");
        assert!(form.alias_locked);
        set(&mut form, Field::Alias, "z");
        assert_eq!(form.plan(&config).lines[0], "Host a b");
    }

    #[test]
    fn odd_indentation_and_key_spelling_are_preserved() {
        let config = config("Host a\n\tHostname old\n");
        let mut form = form(&config, "a");
        set(&mut form, Field::HostName, "new");
        assert_eq!(form.plan(&config).lines[1], "\tHostname new");
    }

    #[test]
    fn an_equals_separator_is_preserved() {
        let config = config("Host a\n  Port=22\n");
        let mut form = form(&config, "a");
        set(&mut form, Field::Port, "23");
        assert_eq!(form.plan(&config).lines[1], "  Port=23");
    }

    #[test]
    fn two_edits_at_once_stay_aligned() {
        let config = config(FIXTURE);
        let mut form = form(&config, "web");
        set(&mut form, Field::User, "lxb");
        set(&mut form, Field::IdentityFile, "");
        let lines = form.plan(&config).lines;
        assert!(lines.contains(&"  User lxb".to_string()));
        assert!(!lines.iter().any(|l| l.contains("IdentityFile")));
        assert!(lines.contains(&"  ServerAliveInterval 30".to_string()));
    }

    #[test]
    fn gateway_candidates_exclude_the_host_being_edited() {
        let config = config(FIXTURE);
        let options = candidates(&config, Field::Gateway, 0);
        assert!(options.contains(&"jumped".to_string()));
        assert!(!options.contains(&"web".to_string()));
    }

    #[test]
    fn saving_keeps_a_backup_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("excalibur-ssh-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, FIXTURE).unwrap();

        let config = SshConfig::load_from(&path).unwrap();
        let mut form = form(&config, "web");
        set(&mut form, Field::Port, "2222");
        let backup = write_config(&config, &form.plan(&config).lines).unwrap();

        assert_eq!(std::fs::read_to_string(&backup).unwrap(), FIXTURE);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("  Port 2222"));
        assert!(saved.contains("  ServerAliveCountMax 7200"));
        assert!(!dir.join("config.excalibur.tmp").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_preserves_the_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("excalibur-ssh-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, FIXTURE).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let config = SshConfig::load_from(&path).unwrap();
        let mut form = form(&config, "web");
        set(&mut form, Field::Port, "2222");
        write_config(&config, &form.plan(&config).lines).unwrap();

        // ssh rejects configs it considers too open, and a freshly written file
        // would otherwise pick up the umask default instead.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_read_only_config_refuses_to_save() {
        let mut config = config(FIXTURE);
        config.read_only = true;
        let lines = config.lines.clone();
        assert!(write_config(&config, &lines).is_err());
    }

    #[test]
    fn user_candidates_come_from_the_config() {
        let config = config("Host a\n  User root\nHost b\n  User lxb\nHost c\n  User lxb\n");
        assert_eq!(candidates(&config, Field::User, 0), ["lxb", "root"]);
    }
}

/// The six fields of a tunnel rule. Unlike [`HostForm`] this edits a struct we
/// own, so there is no file formatting to preserve -- the whole yaml is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardField {
    Group,
    Host,
    Kind,
    Bind,
    Target,
    Note,
}

impl ForwardField {
    pub const ALL: [ForwardField; 6] = [
        ForwardField::Group,
        ForwardField::Host,
        ForwardField::Kind,
        ForwardField::Bind,
        ForwardField::Target,
        ForwardField::Note,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ForwardField::Group => "Group",
            ForwardField::Host => "Host",
            ForwardField::Kind => "Direction",
            ForwardField::Bind => "Listen on",
            ForwardField::Target => "Exit at",
            ForwardField::Note => "Note",
        }
    }

    fn is_pick(self) -> bool {
        matches!(
            self,
            ForwardField::Group | ForwardField::Host | ForwardField::Kind
        )
    }

    /// Shown in place of the value when a field is empty. `-L` takes
    /// `port:host:hostport`, so the exit needs two parts and the field has to
    /// say so -- a bare port there builds a command ssh refuses. It also has to
    /// say whose view the host is resolved in, which is the far side for `-L`.
    pub fn placeholder(self) -> &'static str {
        match self {
            ForwardField::Group => "(pick a group, or Tab to name a new one)",
            ForwardField::Host => "(pick a host)",
            ForwardField::Kind => "(local or remote)",
            ForwardField::Bind => "(port, or address:port)",
            ForwardField::Target => "(host:port, resolved from the far side)",
            ForwardField::Note => "(optional)",
        }
    }
}

#[derive(Debug)]
pub struct ForwardForm {
    /// Where the rule was read from. The group it will be *written* to is
    /// `values[0]`, which the user can change to move the rule or to name a
    /// group that does not exist yet.
    pub profile: usize,
    /// Index within the profile, or `None` while the rule is still new.
    pub index: Option<usize>,
    pub values: Vec<String>,
    pub cursor: usize,
    pub editing: Option<Editing>,
}

impl ForwardForm {
    pub fn new(profile: usize, index: Option<usize>, forward: &Forward, group: &str) -> Self {
        ForwardForm {
            profile,
            index,
            values: vec![
                group.to_string(),
                forward.host.clone(),
                forward.kind.label().to_string(),
                forward.bind.clone(),
                forward.target.clone(),
                forward.note.clone(),
            ],
            cursor: 0,
            editing: None,
        }
    }

    /// The group the rule will be saved into.
    pub fn group(&self) -> &str {
        self.values[0].trim()
    }

    pub fn field(&self) -> ForwardField {
        ForwardField::ALL[self.cursor]
    }

    pub fn next_field(&mut self) {
        self.cursor = (self.cursor + 1) % ForwardField::ALL.len();
    }

    pub fn previous_field(&mut self) {
        self.cursor = (self.cursor + ForwardField::ALL.len() - 1) % ForwardField::ALL.len();
    }

    /// The rule as it currently stands, so the direction diagram and the command
    /// line update while the form is still open.
    pub fn to_forward(&self) -> Forward {
        Forward {
            host: self.values[1].clone(),
            kind: Kind::ALL
                .into_iter()
                .find(|k| k.label() == self.values[2])
                .unwrap_or(Kind::Local),
            bind: self.values[3].clone(),
            target: self.values[4].clone(),
            note: self.values[5].clone(),
        }
    }

    /// `groups` are the profile names that already exist; typing over the pick
    /// (`Tab`) is what creates a new one, so there is no separate "new group"
    /// step and no way to end up with a group that holds nothing.
    pub fn begin_edit(&mut self, config: &SshConfig, groups: &[String]) {
        let field = self.field();
        self.editing = Some(if field.is_pick() {
            let options: Vec<String> = match field {
                ForwardField::Kind => Kind::ALL.iter().map(|k| k.label().to_string()).collect(),
                ForwardField::Group => groups.to_vec(),
                _ => config
                    .hosts
                    .iter()
                    .map(|h| h.alias().to_string())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            };
            let index = options
                .iter()
                .position(|o| *o == self.values[self.cursor])
                .unwrap_or(0);
            Editing::Pick { options, index }
        } else {
            Editing::Text(Box::new(text_area(&self.values[self.cursor])))
        });
    }

    pub fn commit_edit(&mut self) {
        let value = match self.editing.take() {
            Some(Editing::Text(area)) => area.lines().first().cloned().unwrap_or_default(),
            Some(Editing::Pick { options, index }) => {
                options.get(index).cloned().unwrap_or_default()
            }
            None => return,
        };
        let value = value.trim().to_string();
        // Expand a bare exit port here rather than at build time, so the field
        // shows what the command will actually contain.
        self.values[self.cursor] = if self.field() == ForwardField::Target && !value.is_empty() {
            Forward::normalise_target(&value)
        } else {
            value
        };
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    pub fn edit_as_text(&mut self) {
        if let Some(Editing::Pick { options, index }) = &self.editing {
            let seed = options.get(*index).cloned().unwrap_or_default();
            self.editing = Some(Editing::Text(Box::new(text_area(&seed))));
        }
    }
}
