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
