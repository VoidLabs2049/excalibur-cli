use color_eyre::Result;
use std::path::{Path, PathBuf};

/// One `Key Value` line inside a host block.
#[derive(Debug, Clone)]
pub struct Directive {
    /// 0-based index into [`SshConfig::lines`].
    pub line: usize,
    /// Key as written, so that `Hostname` / `HostName` round-trip unchanged.
    pub key: String,
    pub value: String,
}

/// One `Host <patterns>` block.
#[derive(Debug, Clone)]
pub struct HostBlock {
    pub patterns: Vec<String>,
    /// 0-based index of the `Host` header line.
    pub start: usize,
    /// Exclusive end: one past the last *directive* line. Blank lines and
    /// comments trailing the block are left outside, so rewriting `[start, end)`
    /// never disturbs the separators between blocks.
    pub end: usize,
    pub directives: Vec<Directive>,
    /// Index of an earlier block that already sets every keyword this one sets,
    /// which makes this block dead. OpenSSH keeps the *first* value it obtains
    /// for each keyword, so a later block still takes effect for any keyword the
    /// earlier ones left unset -- being duplicated is not by itself fatal.
    pub shadowed_by: Option<usize>,
}

impl HostBlock {
    /// The name the user types after `ssh`.
    pub fn alias(&self) -> &str {
        self.patterns.first().map(String::as_str).unwrap_or("")
    }

    pub fn get(&self, key: &str) -> Option<&Directive> {
        self.directives
            .iter()
            .find(|d| d.key.eq_ignore_ascii_case(key))
    }

    /// The first jump host, from either `ProxyJump` or a legacy `ProxyCommand`.
    pub fn gateway(&self) -> Option<String> {
        if let Some(d) = self.get("ProxyJump") {
            return d
                .value
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        proxy_command_gateway(&self.get("ProxyCommand")?.value)
    }
}

#[derive(Debug, Default)]
pub struct SshConfig {
    pub path: PathBuf,
    /// Every line verbatim, split on `\n` only -- a trailing `\r` stays in the
    /// line, so CRLF files round-trip byte for byte.
    pub lines: Vec<String>,
    pub hosts: Vec<HostBlock>,
    /// Set when the file must not be written: a symlink into the nix store, or
    /// no write permission. Callers degrade to read-only rather than discovering
    /// this at save time.
    pub read_only: bool,
}

impl SshConfig {
    pub fn default_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".ssh").join("config"))
    }

    pub fn load() -> Result<Self> {
        let path = SshConfig::default_path()
            .ok_or_else(|| color_eyre::eyre::eyre!("cannot resolve home directory"))?;
        SshConfig::load_from(&path)
    }

    /// A missing file is an empty config, not an error -- a machine that has
    /// never had one is a legitimate starting state.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = if path.exists() {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        let mut config = SshConfig::parse(path, &text);
        config.read_only = detect_read_only(path);
        Ok(config)
    }

    pub fn parse(path: &Path, text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let mut hosts: Vec<HostBlock> = Vec::new();

        for (i, raw) in lines.iter().enumerate() {
            let Some((key, value)) = split_directive(raw) else {
                continue;
            };

            if key.eq_ignore_ascii_case("host") {
                hosts.push(HostBlock {
                    patterns: value.split_whitespace().map(str::to_string).collect(),
                    start: i,
                    end: i + 1,
                    directives: Vec::new(),
                    shadowed_by: None,
                });
            } else if key.eq_ignore_ascii_case("match") {
                // Not modelled in v1. Ending the current block here keeps its
                // directives from swallowing the conditional block's lines.
                hosts.push(HostBlock {
                    patterns: Vec::new(),
                    start: i,
                    end: i + 1,
                    directives: Vec::new(),
                    shadowed_by: None,
                });
            } else if let Some(block) = hosts.last_mut() {
                block.directives.push(Directive {
                    line: i,
                    key: key.to_string(),
                    value: value.to_string(),
                });
                block.end = i + 1;
            }
        }

        // A `Match` block is only a fence; drop it so callers never see a
        // patternless entry in the host list.
        hosts.retain(|b| !b.patterns.is_empty());
        mark_shadowed(&mut hosts);

        SshConfig {
            path: path.to_path_buf(),
            lines,
            hosts,
            read_only: false,
        }
    }

    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }
}

/// Split `Key Value` or `Key=Value`, returning `None` for blanks and comments.
fn split_directive(raw: &str) -> Option<(&str, &str)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let split = line.find([' ', '\t', '=']).unwrap_or(line.len());
    let (key, rest) = line.split_at(split);
    let value = rest.trim_start_matches([' ', '\t']);
    let value = value.strip_prefix('=').unwrap_or(value).trim();
    Some((key, value))
}

/// Pull the jump host out of a `ProxyCommand ... -W %h:%p` line. Configs written
/// before `ProxyJump` existed express gateways this way; without this the
/// gateway of such a host reads as unset and its jump chain becomes invisible.
fn proxy_command_gateway(value: &str) -> Option<String> {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    if !tokens.iter().any(|t| *t == "-W") {
        return None;
    }
    let mut skip_next = false;
    // Token 0 is the ssh binary, which may be an absolute store path.
    for token in tokens.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if let Some(flag) = token.strip_prefix('-') {
            skip_next = matches!(flag, "W" | "o" | "p" | "i" | "l" | "F");
            continue;
        }
        return Some((*token).to_string());
    }
    None
}

fn mark_shadowed(hosts: &mut [HostBlock]) {
    for i in 0..hosts.len() {
        let alias = hosts[i].alias().to_string();
        if hosts[i].directives.is_empty() {
            continue;
        }
        let mut covered: Vec<&str> = Vec::new();
        let mut shadowed_by = None;
        for (j, earlier) in hosts[..i].iter().enumerate() {
            if !block_matches(&earlier.patterns, &alias) {
                continue;
            }
            for d in &earlier.directives {
                covered.push(&d.key);
            }
            if shadowed_by.is_none() {
                shadowed_by = Some(j);
            }
        }
        let all_covered = hosts[i]
            .directives
            .iter()
            .all(|d| covered.iter().any(|k| k.eq_ignore_ascii_case(&d.key)));
        hosts[i].shadowed_by = if all_covered { shadowed_by } else { None };
    }
}

fn block_matches(patterns: &[String], name: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob_match(negated, name) {
                return false;
            }
        } else if glob_match(pattern, name) {
            matched = true;
        }
    }
    matched
}

/// `ssh_config(5)` globbing: `*` spans any run, `?` one character.
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut backtrack) = (None, 0usize);

    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            backtrack = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            backtrack += 1;
            ni = backtrack;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

fn detect_read_only(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(path)
            .map(|target| target.starts_with("/nix/store"))
            .unwrap_or(false),
        Ok(meta) => meta.permissions().readonly(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the quirks of the author's real file: one-space and two-space
    /// indentation, `Hostname` next to `HostName`, a trailing space inside a
    /// `Host` pattern, a whitespace-only line, and a duplicated alias.
    const FIXTURE: &str = "Host github.com\n\
         \x20Hostname ssh.github.com\n\
         \x20Port 443\n\
         \x20ProxyCommand /run/current-system/sw/bin/ssh lxb@kami -W %h:%p\n\
         \n\
         \x20Host kami\n\
         \x20\x20Hostname 192.168.110.134\n\
         \x20\x20User lxb\n\
         \x20\x20Port 22\n\
         \x20\x20\n\
         Host xx-trade-wsl1 \n\
         \x20\x20HostName sisyphus\n\
         \x20\x20Port 12221\n\
         \x20\x20User nixos\n\
         \n\
         Host kami\n\
         \x20\x20Hostname 192.168.110.134\n\
         \x20\x20User lxb\n\
         \x20\x20Port 22\n";

    fn parse(text: &str) -> SshConfig {
        SshConfig::parse(Path::new("/tmp/config"), text)
    }

    fn find<'a>(config: &'a SshConfig, alias: &str) -> Option<&'a HostBlock> {
        config.hosts.iter().find(|b| b.alias() == alias)
    }

    #[test]
    fn round_trips_byte_for_byte() {
        assert_eq!(parse(FIXTURE).to_text(), FIXTURE);
    }

    #[test]
    fn round_trips_crlf_and_a_missing_final_newline() {
        let text = "Host a\r\n  Port 22\r\nHost b\r\n  Port 23";
        assert_eq!(parse(text).to_text(), text);
    }

    #[test]
    fn finds_every_host_block_in_order() {
        let config = parse(FIXTURE);
        let aliases: Vec<&str> = config.hosts.iter().map(HostBlock::alias).collect();
        assert_eq!(aliases, ["github.com", "kami", "xx-trade-wsl1", "kami"]);
    }

    #[test]
    fn indented_host_header_still_starts_a_block() {
        // ` Host kami` is indented by one space in the real file.
        assert!(find(&parse(FIXTURE), "kami").is_some());
    }

    #[test]
    fn trailing_space_in_a_host_pattern_is_trimmed() {
        let block = find(&parse(FIXTURE), "xx-trade-wsl1").cloned();
        assert!(block.is_some(), "pattern with a trailing space was not found");
    }

    #[test]
    fn key_lookup_ignores_case_but_the_written_key_is_kept() {
        let config = parse(FIXTURE);
        let wsl = find(&config, "xx-trade-wsl1").unwrap();
        assert_eq!(wsl.get("hostname").unwrap().key, "HostName");
        let gh = find(&config, "github.com").unwrap();
        assert_eq!(gh.get("HOSTNAME").unwrap().key, "Hostname");
    }

    #[test]
    fn gateway_is_recovered_from_a_legacy_proxycommand() {
        let config = parse(FIXTURE);
        assert_eq!(
            find(&config, "github.com").unwrap().gateway().as_deref(),
            Some("lxb@kami")
        );
    }

    #[test]
    fn gateway_is_recovered_when_the_host_follows_the_w_flag() {
        let config = parse("Host a\n  ProxyCommand ssh -W %h:%p bastion\n");
        assert_eq!(config.hosts[0].gateway().as_deref(), Some("bastion"));
    }

    #[test]
    fn proxyjump_takes_the_first_hop_of_a_chain() {
        let config = parse("Host a\n  ProxyJump g1,g2\n");
        assert_eq!(config.hosts[0].gateway().as_deref(), Some("g1"));
    }

    #[test]
    fn a_host_without_a_gateway_reports_none() {
        assert_eq!(parse(FIXTURE).hosts[1].gateway(), None);
    }

    #[test]
    fn a_duplicate_block_adding_nothing_new_is_shadowed() {
        let config = parse(FIXTURE);
        assert_eq!(config.hosts[3].alias(), "kami");
        assert_eq!(config.hosts[3].shadowed_by, Some(1));
        assert_eq!(config.hosts[1].shadowed_by, None);
    }

    #[test]
    fn a_duplicate_block_introducing_a_new_keyword_is_not_shadowed() {
        // OpenSSH takes the first value per keyword, so `User` here still wins.
        let config = parse("Host a\n  Port 22\nHost a\n  User lxb\n");
        assert_eq!(config.hosts[1].shadowed_by, None);
    }

    #[test]
    fn a_wildcard_block_shadows_a_later_literal_block() {
        let config = parse("Host *\n  Port 22\nHost kami\n  Port 2222\n");
        assert_eq!(config.hosts[1].shadowed_by, Some(0));
    }

    #[test]
    fn a_negated_pattern_does_not_shadow() {
        let config = parse("Host * !kami\n  Port 22\nHost kami\n  Port 2222\n");
        assert_eq!(config.hosts[1].shadowed_by, None);
    }

    #[test]
    fn block_end_excludes_trailing_blank_lines() {
        let config = parse(FIXTURE);
        let kami = &config.hosts[1];
        // Header at 5, directives at 6..=8, whitespace-only line 9 stays outside.
        assert_eq!((kami.start, kami.end), (5, 9));
    }

    #[test]
    fn a_match_block_fences_off_the_preceding_host() {
        let config = parse("Host a\n  Port 22\nMatch user root\n  Port 23\n");
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].directives.len(), 1);
    }

    #[test]
    fn key_equals_value_is_accepted() {
        let config = parse("Host=a\n  Port=22\n");
        assert_eq!(config.hosts[0].alias(), "a");
        assert_eq!(config.hosts[0].get("Port").unwrap().value, "22");
    }

    #[test]
    fn a_missing_file_parses_as_an_empty_config() {
        let config = SshConfig::load_from(Path::new("/tmp/definitely-not-here")).unwrap();
        assert!(config.hosts.is_empty());
    }

    /// Structural invariants against whatever the machine actually has. Content
    /// is never asserted -- only that no block is malformed or overlapping.
    #[test]
    fn the_real_config_parses_without_malformed_blocks() {
        let Some(path) = SshConfig::default_path() else {
            return;
        };
        if !path.exists() {
            return;
        }
        let config = SshConfig::load_from(&path).unwrap();
        assert_eq!(config.to_text(), std::fs::read_to_string(&path).unwrap());

        let mut previous_end = 0;
        for block in &config.hosts {
            assert!(!block.alias().is_empty());
            assert!(block.start < block.end);
            assert!(block.start >= previous_end, "blocks overlap at {}", block.start);
            previous_end = block.end;
        }

        let gateways = config.hosts.iter().filter(|b| b.gateway().is_some()).count();
        let shadowed = config.hosts.iter().filter(|b| b.shadowed_by.is_some()).count();
        eprintln!(
            "real config: {} hosts, {} with a gateway, {} shadowed, read_only={}",
            config.hosts.len(),
            gateways,
            shadowed,
            config.read_only
        );
    }
}
