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
            self.kind.flag().into(),
            self.spec(),
            self.host.clone(),
        ]
    }

    pub fn command_line(&self) -> String {
        format!("ssh {}", self.ssh_args().join(" "))
    }

    /// Where the port opens and where traffic leaves, in that order. The two
    /// swap sides between `-L` and `-R`; saying it outright is the whole point.
    pub fn explain(&self) -> (String, String) {
        match self.kind {
            Kind::Local => (
                format!("listen   this machine   {}", self.bind),
                format!("exit     {}   ->  {}", self.host, self.target),
            ),
            Kind::Remote => (
                format!("listen   {}   {}", self.host, self.bind),
                format!("exit     this machine   ->  {}", self.target),
            ),
        }
    }

    /// One-line summary for the list.
    pub fn summary(&self) -> String {
        format!("{} {} {}", self.kind.flag(), self.spec(), self.host)
    }
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
             -L 29001:0.0.0.0:9001 xx-database-1"
        );
    }

    #[test]
    fn a_remote_forward_uses_the_r_flag() {
        assert!(remote().command_line().contains("-R 8080:localhost:3000"));
    }

    #[test]
    fn the_listening_side_swaps_between_the_two_directions() {
        // The one thing that has to be right: -L opens the port here, -R opens
        // it on the far side.
        let (listen, exit) = local().explain();
        assert!(listen.contains("this machine"));
        assert!(exit.contains("xx-database-1"));

        let (listen, exit) = remote().explain();
        assert!(listen.contains("xx-database-1"));
        assert!(exit.contains("this machine"));
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
