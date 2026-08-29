use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which way the forward runs.
///
/// The two are mirror images and the argument order does not say which is which,
/// so everything here is phrased as "where it listens" and "where it exits".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `-L`: listens here, exits on the far side.
    Local,
    /// `-R`: listens on the far side, exits here.
    Remote,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Local, Kind::Remote];

    pub fn flag(self) -> &'static str {
        match self {
            Kind::Local => "-L",
            Kind::Remote => "-R",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Local => "local",
            Kind::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forward {
    /// An alias from `~/.ssh/config`.
    pub host: String,
    pub kind: Kind,
    /// The listening side: `29001`, or `0.0.0.0:29001` to accept from elsewhere.
    pub bind: String,
    /// Where traffic comes out: `host:port`.
    pub target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Forward {
    /// The `-L`/`-R` argument: `bind:target`.
    pub fn spec(&self) -> String {
        format!("{}:{}", self.bind, self.target)
    }

    /// Arguments for the tunnel process.
    ///
    /// `BatchMode` is not optional: without it ssh prompts for a passphrase when
    /// the agent has no key, and with the TUI holding the terminal that prompt
    /// is invisible and the UI simply hangs. `ExitOnForwardFailure` is what turns
    /// "the port was already taken" from a live process with no forward into a
    /// visible failure.
    pub fn ssh_args(&self) -> Vec<String> {
        vec![
            "-f".into(),
            "-N".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ExitOnForwardFailure=yes".into(),
            // Bounds how long an unreachable host can hold the worker; without
            // it ssh waits out the full TCP connect timeout.
            "-o".into(),
            "ConnectTimeout=10".into(),
            self.kind.flag().into(),
            self.spec(),
            self.host.clone(),
        ]
    }

    pub fn command_line(&self) -> String {
        format!("ssh {}", self.ssh_args().join(" "))
    }

    /// Where the port opens and where traffic leaves, in that order.
    ///
    /// Both the side that listens and the side the exit is *resolved from* swap
    /// between `-L` and `-R`, and the second is the part the syntax hides: in
    /// `-L 29001:10.0.0.5:9001 kami` it is kami that connects to 10.0.0.5, so
    /// the exit may name anything kami can reach and nothing this machine can.
    pub fn explain(&self) -> (String, String) {
        match self.kind {
            Kind::Local => (
                format!("listen   here             {}", self.bind),
                format!("exit     from {}   {}", self.host, self.annotated_target()),
            ),
            Kind::Remote => (
                format!("listen   on {}      {}", self.host, self.bind),
                format!("exit     from here       {}", self.annotated_target()),
            ),
        }
    }

    /// `localhost` in the exit is the *far* side's localhost for `-L`, which is
    /// the single most misread part of a forward -- so it is spelled out.
    fn annotated_target(&self) -> String {
        let host = self.target.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        if !matches!(host, "localhost" | "127.0.0.1" | "::1") {
            return self.target.clone();
        }
        match self.kind {
            Kind::Local => format!("{}   (= {} itself)", self.target, self.host),
            Kind::Remote => format!("{}   (= this machine)", self.target),
        }
    }

    /// The side that resolves the exit address.
    pub fn exit_resolved_from(&self) -> String {
        match self.kind {
            Kind::Local => self.host.clone(),
            Kind::Remote => "this machine".to_string(),
        }
    }

    /// One-line summary for the list.
    pub fn summary(&self) -> String {
        format!("{} {} {}", self.kind.flag(), self.spec(), self.host)
    }

    /// Why ssh would reject this rule, if it would.
    ///
    /// The exit is the trap: `-L` takes `port:host:hostport`, so an exit written
    /// as a bare port produces `-L 22:6022`, which ssh refuses. Nothing about
    /// the field says it needs two parts, so it is checked here instead.
    pub fn problem(&self) -> Option<String> {
        if self.host.trim().is_empty() {
            return Some("host is required".into());
        }
        if self.bind.trim().is_empty() {
            return Some("listen side is required".into());
        }
        if port_of(&self.bind).is_none() {
            return Some(format!(
                "`{}` is not a port -- use `8080` or `0.0.0.0:8080`",
                self.bind
            ));
        }
        let target = self.target.trim();
        if target.is_empty() {
            return Some("exit side is required".into());
        }
        if !target.contains(':') {
            return Some(format!(
                "exit needs a host too: `localhost:{target}` for {} itself, or any \
                 address {} can reach",
                self.exit_resolved_from(),
                self.exit_resolved_from()
            ));
        }
        if port_of(target).is_none() {
            return Some(format!("`{target}` does not end in a port"));
        }
        None
    }

    /// A bare port typed into the exit field, expanded to the host ssh needs.
    /// Applied on commit so the field visibly changes rather than the command
    /// quietly differing from what was typed.
    pub fn normalise_target(value: &str) -> String {
        let value = value.trim();
        match value.parse::<u16>() {
            Ok(port) => format!("localhost:{port}"),
            Err(_) => value.to_string(),
        }
    }

    /// Ports below 1024 need root to bind. Not an error -- excalibur may well be
    /// run as root -- but the failure is otherwise a bare "permission denied".
    pub fn privileged_bind(&self) -> Option<u16> {
        port_of(&self.bind).filter(|p| *p < 1024 && self.kind == Kind::Local)
    }
}

/// The port at the end of a `port`, `address:port`, or `host:port` spec.
pub fn port_of(spec: &str) -> Option<u16> {
    spec.rsplit(':').next()?.trim().parse().ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub forwards: Vec<Forward>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tunnels {
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

impl Tunnels {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("excalibur").join("tunnels.yaml"))
    }

    /// A missing file is an empty set of profiles, not an error.
    pub fn load() -> Result<Self> {
        let Some(path) = Tunnels::path() else {
            return Ok(Tunnels::default());
        };
        if !path.exists() {
            return Ok(Tunnels::default());
        }
        Ok(serde_yaml::from_str(&std::fs::read_to_string(&path)?)?)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Tunnels::path()
            .ok_or_else(|| color_eyre::eyre::eyre!("cannot resolve the config directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_yaml::to_string(self)?)?;
        std::fs::rename(&temp, &path)?;
        Ok(path)
    }

    /// Every forward with the profile it belongs to, in display order.
    pub fn all(&self) -> Vec<(usize, usize)> {
        self.profiles
            .iter()
            .enumerate()
            .flat_map(|(p, profile)| (0..profile.forwards.len()).map(move |f| (p, f)))
            .collect()
    }

    pub fn get(&self, profile: usize, forward: usize) -> Option<&Forward> {
        self.profiles.get(profile)?.forwards.get(forward)
    }

    pub fn count(&self) -> usize {
        self.profiles.iter().map(|p| p.forwards.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Forward {
        Forward {
            host: "xx-database-1".into(),
            kind: Kind::Local,
            bind: "29001".into(),
            target: "0.0.0.0:9001".into(),
            note: "minio console".into(),
        }
    }

    fn remote() -> Forward {
        Forward {
            host: "xx-database-1".into(),
            kind: Kind::Remote,
            bind: "8080".into(),
            target: "localhost:3000".into(),
            note: String::new(),
        }
    }

    #[test]
    fn a_local_forward_matches_what_the_shell_history_shows() {
        assert_eq!(
            local().command_line(),
            "ssh -f -N -o BatchMode=yes -o ExitOnForwardFailure=yes \
             -o ConnectTimeout=10 -L 29001:0.0.0.0:9001 xx-database-1"
        );
    }

    #[test]
    fn a_remote_forward_uses_the_r_flag() {
        assert!(remote().command_line().contains("-R 8080:localhost:3000"));
    }

    #[test]
    fn both_the_listening_side_and_the_exit_side_swap() {
        // -L opens the port here and the far side resolves the exit; -R is the
        // mirror. The second half is what the flag syntax hides: in
        // `-L 29001:10.0.0.5:9001 kami` it is kami that connects to 10.0.0.5.
        let (listen, exit) = local().explain();
        assert!(listen.contains("here"), "got: {listen}");
        assert!(exit.contains("from xx-database-1"), "got: {exit}");
        assert_eq!(local().exit_resolved_from(), "xx-database-1");

        let (listen, exit) = remote().explain();
        assert!(listen.contains("on xx-database-1"), "got: {listen}");
        assert!(exit.contains("from here"), "got: {exit}");
        assert_eq!(remote().exit_resolved_from(), "this machine");
    }

    #[test]
    fn an_exit_the_far_side_can_reach_is_a_normal_rule() {
        // Using kami as a jump host to reach something only kami can see.
        let through_a_jump_host = Forward {
            host: "kami".into(),
            kind: Kind::Local,
            bind: "29001".into(),
            target: "10.0.0.5:9001".into(),
            note: String::new(),
        };
        assert_eq!(through_a_jump_host.problem(), None);
        assert!(
            through_a_jump_host
                .command_line()
                .contains("-L 29001:10.0.0.5:9001 kami")
        );
    }

    #[test]
    fn the_missing_host_message_names_the_side_that_resolves_it() {
        let rule = Forward {
            host: "kami".into(),
            kind: Kind::Local,
            bind: "29001".into(),
            target: "9001".into(),
            note: String::new(),
        };
        let problem = rule.problem().unwrap();
        assert!(problem.contains("kami"), "got: {problem}");
    }

    #[test]
    fn round_trips_through_yaml() {
        let tunnels = Tunnels {
            profiles: vec![Profile {
                name: "daily".into(),
                forwards: vec![local(), remote()],
            }],
        };
        let text = serde_yaml::to_string(&tunnels).unwrap();
        let back: Tunnels = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.profiles[0].forwards, tunnels.profiles[0].forwards);
        assert!(text.contains("kind: local"), "got: {text}");
    }

    #[test]
    fn an_empty_note_is_left_out_of_the_file() {
        let text = serde_yaml::to_string(&remote()).unwrap();
        assert!(!text.contains("note"), "got: {text}");
    }

    #[test]
    fn a_file_without_notes_still_parses() {
        let text = "profiles:\n  - name: daily\n    forwards:\n      \
                    - host: kami\n        kind: remote\n        bind: '8080'\n        \
                    target: localhost:3000\n";
        let tunnels: Tunnels = serde_yaml::from_str(text).unwrap();
        assert_eq!(tunnels.count(), 1);
        assert_eq!(tunnels.get(0, 0).unwrap().kind, Kind::Remote);
        assert_eq!(tunnels.get(0, 0).unwrap().note, "");
    }

    #[test]
    fn all_walks_every_profile() {
        let tunnels = Tunnels {
            profiles: vec![
                Profile {
                    name: "a".into(),
                    forwards: vec![local()],
                },
                Profile {
                    name: "b".into(),
                    forwards: vec![local(), remote()],
                },
            ],
        };
        assert_eq!(tunnels.all(), [(0, 0), (1, 0), (1, 1)]);
        assert_eq!(tunnels.count(), 3);
    }
}

#[cfg(test)]
mod validation {
    use super::*;

    fn rule(bind: &str, target: &str) -> Forward {
        Forward {
            host: "kami".into(),
            kind: Kind::Local,
            bind: bind.into(),
            target: target.into(),
            note: String::new(),
        }
    }

    #[test]
    fn a_bare_port_as_the_exit_is_rejected_with_the_fix_spelled_out() {
        // The rule the author actually wrote: `-L 22:6022 kami`, which ssh
        // refuses because the exit has no host.
        let problem = rule("22", "6022").problem().unwrap();
        assert!(problem.contains("localhost:6022"), "got: {problem}");
    }

    #[test]
    fn a_complete_rule_has_no_problem() {
        assert_eq!(rule("29001", "0.0.0.0:9001").problem(), None);
        assert_eq!(rule("0.0.0.0:29001", "localhost:9001").problem(), None);
    }

    #[test]
    fn missing_pieces_are_named_individually() {
        assert!(
            rule("", "localhost:1")
                .problem()
                .unwrap()
                .contains("listen")
        );
        assert!(rule("1", "").problem().unwrap().contains("exit"));

        let mut hostless = rule("1", "localhost:1");
        hostless.host = String::new();
        assert!(hostless.problem().unwrap().contains("host"));
    }

    #[test]
    fn a_non_numeric_port_is_caught_on_either_side() {
        assert!(rule("http", "localhost:1").problem().is_some());
        assert!(rule("1", "localhost:http").problem().is_some());
    }

    #[test]
    fn a_bare_port_expands_to_localhost() {
        assert_eq!(Forward::normalise_target("6022"), "localhost:6022");
        assert_eq!(Forward::normalise_target(" 9001 "), "localhost:9001");
    }

    #[test]
    fn an_exit_that_already_names_a_host_is_left_alone() {
        assert_eq!(Forward::normalise_target("0.0.0.0:9001"), "0.0.0.0:9001");
        assert_eq!(
            Forward::normalise_target("db.internal:5432"),
            "db.internal:5432"
        );
    }

    #[test]
    fn a_privileged_local_bind_is_flagged() {
        assert_eq!(rule("22", "localhost:6022").privileged_bind(), Some(22));
        assert_eq!(rule("29001", "localhost:6022").privileged_bind(), None);
    }

    #[test]
    fn a_remote_bind_below_1024_is_not_our_problem() {
        // The far side binds it, so local privileges do not decide.
        let mut remote = rule("80", "localhost:8080");
        remote.kind = Kind::Remote;
        assert_eq!(remote.privileged_bind(), None);
    }
}

#[cfg(test)]
mod whose_localhost {
    use super::*;

    fn rule(kind: Kind, target: &str) -> Forward {
        Forward {
            host: "kami".into(),
            kind,
            bind: "6022".into(),
            target: target.into(),
            note: String::new(),
        }
    }

    #[test]
    fn a_local_forward_says_localhost_means_the_far_side() {
        // `-L 6022:localhost:22 kami` lands on kami's sshd, not this machine's.
        // Reading the rule as "my localhost" is the usual mistake.
        let (_, exit) = rule(Kind::Local, "localhost:22").explain();
        assert!(exit.contains("= kami itself"), "got: {exit}");
    }

    #[test]
    fn a_remote_forward_says_localhost_means_this_machine() {
        let (_, exit) = rule(Kind::Remote, "127.0.0.1:3000").explain();
        assert!(exit.contains("= this machine"), "got: {exit}");
    }

    #[test]
    fn a_named_exit_host_needs_no_annotation() {
        let (_, exit) = rule(Kind::Local, "10.0.0.5:9001").explain();
        assert!(!exit.contains("(="), "got: {exit}");
        assert!(exit.contains("10.0.0.5:9001"));
    }

    #[test]
    fn the_annotation_is_display_only() {
        // It must never leak into the command ssh is given.
        let command = rule(Kind::Local, "localhost:22").command_line();
        assert!(
            command.contains("-L 6022:localhost:22 kami"),
            "got: {command}"
        );
        assert!(!command.contains("(="));
    }
}
