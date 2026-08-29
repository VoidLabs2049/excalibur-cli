use super::sshconfig::SshConfig;
use std::process::{Command, Stdio};

/// What OpenSSH actually resolves for a host, per `ssh -G`.
///
/// This is the only way to see the difference between what a block says and
/// what ssh does with it: values are taken first-match-wins across every block
/// that matches the alias, so a directive can be present and inert.
#[derive(Debug, Default)]
pub struct Effective {
    /// Lowercased keys in the order ssh printed them. A key may repeat --
    /// `identityfile` is listed once per candidate.
    pub values: Vec<(String, String)>,
    /// Whatever ssh wrote to stderr. It refuses to resolve a config containing
    /// an unknown keyword, so this doubles as a syntax check.
    pub error: Option<String>,
}

impl Effective {
    /// Whether `written` is what ssh ends up using for `key`.
    pub fn agrees(&self, key: &str, written: &str) -> bool {
        let key = key.to_ascii_lowercase();
        self.values.iter().any(|(k, v)| *k == key && v == written)
    }

    pub fn first(&self, key: &str) -> Option<&str> {
        let key = key.to_ascii_lowercase();
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Ask ssh to resolve `alias`.
///
/// `ssh -G` is local and takes single-digit milliseconds, so it runs inline --
/// except when the config has a `Match exec` block, which would make ssh run a
/// shell command and could hang the UI. Those are refused with a reason instead.
pub fn resolve(alias: &str, config_lines: &[String]) -> Effective {
    if let Some(line) = match_exec_line(config_lines) {
        return Effective {
            values: Vec::new(),
            error: Some(format!(
                "not run: `Match exec` on line {line} would execute a command"
            )),
        };
    }

    let output = Command::new("ssh")
        .arg("-G")
        .arg(alias)
        // The trailing command is load-bearing: with none, ssh assumes an
        // interactive session, finds stdin is not a tty, and warns on stderr --
        // which would surface here as a config error. It is never run; `-G` only
        // prints the resolved options.
        .arg("true")
        .stdin(Stdio::null())
        .output();

    let output = match output {
        Ok(output) => output,
        Err(e) => {
            return Effective {
                values: Vec::new(),
                error: Some(format!("could not run ssh: {e}")),
            };
        }
    };

    let values = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            Some((key.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Effective {
        values,
        error: (!stderr.is_empty()).then_some(stderr),
    }
}

/// What OpenSSH made of a config that has not been written yet.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// ssh parsed it. Safe to write.
    Ok,
    /// ssh refused it. Writing it would break the file from that line down.
    Rejected(String),
    /// The check could not be run at all. The save may still go ahead, but the
    /// caller has to say so -- a promise of validation that quietly does not
    /// happen is worse than no promise.
    Skipped(String),
}

/// Ask ssh to parse `lines` before they replace the real config.
///
/// This is the only check that catches the failure the editor can actually
/// cause: OpenSSH stops reading a config at the first line it cannot parse, so
/// one bad value silently disables **everything below it** -- and nothing says
/// so until the next connection, long after anyone remembers editing it.
///
/// The candidate goes to a sibling temp file rather than anywhere else so it
/// lands on the same filesystem with the same ownership; the mode is copied
/// from the real config because ssh refuses a config file it considers too
/// open, and that refusal would otherwise be reported here as a syntax error.
pub fn check(config: &SshConfig, lines: &[String], alias: &str) -> Verdict {
    if let Some(line) = match_exec_line(lines) {
        return Verdict::Skipped(format!("`Match exec` on line {line} would run a command"));
    }
    // `write_config` refuses a read-only config anyway, and staging a copy
    // beside it would fail first with a far less useful message.
    if config.read_only {
        return Verdict::Skipped("config is read-only".to_string());
    }
    let temp = {
        let mut name = config.path.file_name().unwrap_or_default().to_os_string();
        name.push(".excalibur.check");
        config.path.with_file_name(name)
    };
    if let Err(e) = std::fs::write(&temp, lines.join("\n")) {
        return Verdict::Skipped(format!("could not stage a copy: {e}"));
    }
    if let Ok(meta) = std::fs::metadata(&config.path) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }

    let output = Command::new("ssh")
        .arg("-F")
        .arg(&temp)
        // Same trailing `true` as `resolve`: without it ssh warns about the
        // missing tty on stderr, which would read as a config error.
        .args(["-G", alias, "true"])
        .stdin(Stdio::null())
        .output();
    std::fs::remove_file(&temp).ok();

    let Ok(output) = output else {
        return Verdict::Skipped("ssh is not available".to_string());
    };
    if output.status.success() {
        return Verdict::Ok;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr
        .lines()
        // The path in ssh's message is the throwaway copy, which would send
        // anyone reading it to a file that no longer exists. Joined onto one
        // line because the notification bar is one row and would otherwise
        // clip the rest away without saying so.
        .map(|line| line.trim().replace(&temp.display().to_string(), "config"))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    Verdict::Rejected(match stderr.is_empty() {
        true => format!("ssh -G exited with {}", output.status),
        false => stderr,
    })
}

fn match_exec_line(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .position(|line| {
            let line = line.trim();
            line.len() >= 5
                && line[..5].eq_ignore_ascii_case("match")
                && line.to_ascii_lowercase().contains("exec")
        })?
        .checked_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn agrees_only_when_the_written_value_is_the_one_in_use() {
        let effective = Effective {
            values: vec![
                ("port".into(), "22".into()),
                ("identityfile".into(), "~/.ssh/id_rsa".into()),
                ("identityfile".into(), "~/.ssh/id_ed25519".into()),
            ],
            error: None,
        };
        assert!(effective.agrees("Port", "22"));
        assert!(!effective.agrees("Port", "9999"));
        // A repeated key agrees with any of its values.
        assert!(effective.agrees("IdentityFile", "~/.ssh/id_ed25519"));
        assert_eq!(effective.first("Port"), Some("22"));
        assert_eq!(effective.first("User"), None);
    }

    #[test]
    fn a_match_exec_block_is_refused_rather_than_run() {
        let result = resolve("anything", &lines("Host a\nMatch exec \"true\"\n  Port 22"));
        assert!(result.values.is_empty());
        assert!(result.error.unwrap().contains("line 2"));
    }

    #[test]
    fn a_plain_match_block_does_not_block_resolution() {
        assert!(match_exec_line(&lines("Host a\nMatch user root\n  Port 22")).is_none());
    }

    /// A config file in its own directory, so nothing here can reach the real
    /// `~/.ssh/config`. Returns `None` when there is no ssh to ask.
    fn staged(name: &str, text: &str) -> Option<(SshConfig, std::path::PathBuf)> {
        if Command::new("ssh").arg("-V").output().is_err() {
            return None;
        }
        let dir =
            std::env::temp_dir().join(format!("excalibur-ssh-check-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, text).unwrap();
        Some((SshConfig::parse(&path, text), dir))
    }

    #[test]
    fn a_value_openssh_cannot_parse_is_refused_before_it_is_written() {
        // The failure the editor can actually cause: ssh stops reading at the
        // first line it rejects, so `Port abc` silently kills every block below
        // it and nothing says so until the next connection.
        let Some((config, dir)) = staged("bad", "Host x\n  Port 22\n") else {
            return;
        };
        let verdict = check(&config, &lines("Host x\n  Port abc\n"), "x");
        std::fs::remove_dir_all(&dir).ok();

        let Verdict::Rejected(why) = verdict else {
            panic!("a config ssh refuses was accepted: {verdict:?}");
        };
        assert!(why.contains("line 2"), "does not point at the line: {why}");
        assert!(
            !why.contains("excalibur.check"),
            "sends the reader to the throwaway copy: {why}"
        );
    }

    #[test]
    fn an_unknown_keyword_is_refused_too() {
        let Some((config, dir)) = staged("unknown", "Host x\n  Port 22\n") else {
            return;
        };
        let verdict = check(&config, &lines("Host x\n  Frobnicate yes\n"), "x");
        std::fs::remove_dir_all(&dir).ok();
        assert!(matches!(verdict, Verdict::Rejected(_)), "got: {verdict:?}");
    }

    #[test]
    fn a_config_openssh_accepts_passes_and_leaves_nothing_behind() {
        let Some((config, dir)) = staged("good", "Host x\n  Port 22\n") else {
            return;
        };
        let verdict = check(&config, &lines("Host x\n  Port 2222\n  User root\n"), "x");
        let leftover = dir.join("config.excalibur.check").exists();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(verdict, Verdict::Ok);
        assert!(!leftover, "the staged copy was left in ~/.ssh");
    }

    #[test]
    fn a_match_exec_config_is_skipped_rather_than_run_or_refused() {
        // Running it would execute a shell command on every save. Refusing it
        // would make such a config unsavable, so the only answer is to say the
        // check did not happen.
        let Some((config, dir)) = staged("exec", "Host x\n  Port 22\n") else {
            return;
        };
        let verdict = check(
            &config,
            &lines("Host x\n  Port 22\nMatch exec \"true\"\n  User root\n"),
            "x",
        );
        std::fs::remove_dir_all(&dir).ok();
        let Verdict::Skipped(why) = verdict else {
            panic!("Match exec was not skipped: {verdict:?}");
        };
        assert!(why.contains("line 3"), "got: {why}");
    }

    /// The whole point of the feature: a second block that sets a keyword the
    /// first already set is inert, and only ssh can tell you so.
    #[test]
    fn a_shadowed_value_disagrees_with_what_is_written() {
        let dir = std::env::temp_dir().join(format!("excalibur-ssh-eff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, "Host x\n  Port 22\nHost x\n  Port 9999\n").unwrap();

        let output = Command::new("ssh")
            .args(["-F"])
            .arg(&path)
            .args(["-G", "x"])
            .stdin(Stdio::null())
            .output();
        std::fs::remove_dir_all(&dir).ok();

        let Ok(output) = output else { return }; // no ssh on this machine
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("port 22"), "ssh resolved: {text}");
        assert!(!text.contains("port 9999"));
    }
}
