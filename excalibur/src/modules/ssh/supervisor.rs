use super::tunnels::{Forward, Kind};
use color_eyre::Result;
use color_eyre::eyre::bail;
use std::process::{Command, Stdio};

/// A tunnel process found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    pub kind: Kind,
    /// The `-L`/`-R` argument, normalised out of wherever it sat in argv.
    pub spec: String,
    pub host: String,
}

impl Running {
    /// The same shape as `Forward::summary`, so a process listed next to the
    /// rules reads in the same column.
    pub fn summary(&self) -> String {
        format!("{} {} {}", self.kind.flag(), self.spec, self.host)
    }
}

/// ssh options that consume the following word. Everything else in a `-abc`
/// cluster is a boolean, so the destination is the first bare word that is not
/// one of these values -- which is how the host is told apart from a port.
const TAKES_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";

/// Tunnels started here detach with `-f`, so they outlive the TUI and have to be
/// recognised again on the next run. Recognition is structural -- parse argv and
/// compare fields -- rather than a substring search, which would also match the
/// shell that launched the search (see ~/.claude/remote-ops.md).
pub fn parse_argv(argv: &[String]) -> Option<(Kind, String, String)> {
    let program = argv.first()?;
    if program.rsplit('/').next()? != "ssh" {
        return None;
    }

    let mut found: Option<(Kind, String)> = None;
    let mut host: Option<String> = None;
    let mut i = 1;

    while i < argv.len() {
        let arg = &argv[i];
        match arg.strip_prefix('-') {
            Some(cluster) if !cluster.is_empty() => {
                for (pos, c) in cluster.char_indices() {
                    if !TAKES_VALUE.contains(c) {
                        continue;
                    }
                    let attached = &cluster[pos + c.len_utf8()..];
                    let value = if attached.is_empty() {
                        i += 1;
                        argv.get(i)?.clone()
                    } else {
                        attached.to_string()
                    };
                    if c == 'L' || c == 'R' {
                        let kind = if c == 'L' { Kind::Local } else { Kind::Remote };
                        found.get_or_insert((kind, value));
                    }
                    break; // the rest of the cluster was the value
                }
            }
            // A bare `-` or a plain word: the first such word is the destination,
            // anything after it is the remote command.
            _ if host.is_none() => host = Some(arg.clone()),
            _ => {}
        }
        i += 1;
    }

    let (kind, spec) = found?;
    Some((kind, spec, host?))
}

/// Every ssh forward running under this user.
///
/// Linux-only: it reads `/proc`. Elsewhere the dashboard shows everything as
/// stopped rather than failing to build.
///
/// Restricted to our own uid because `/proc/<pid>/cmdline` is world-readable:
/// without it another user's tunnel is listed as unclaimed and offered for
/// stopping, and the `kill` behind that offer can only fail.
#[cfg(target_os = "linux")]
pub fn scan() -> Vec<Running> {
    let Ok(processes) = procfs::process::all_processes() else {
        return Vec::new();
    };
    let me = procfs::process::Process::myself()
        .ok()
        .and_then(|process| process.uid().ok());
    processes
        .filter_map(|process| {
            let process = process.ok()?;
            if me.is_some_and(|uid| process.uid().ok() != Some(uid)) {
                return None;
            }
            let pid = u32::try_from(process.pid()).ok()?;
            let (kind, spec, host) = parse_argv(&process.cmdline().ok()?)?;
            Some(Running {
                pid,
                kind,
                spec,
                host,
            })
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub fn scan() -> Vec<Running> {
    Vec::new()
}

/// The process serving `forward`, if one is up.
pub fn find<'a>(running: &'a [Running], forward: &Forward) -> Option<&'a Running> {
    let spec = forward.spec();
    running
        .iter()
        .find(|r| r.kind == forward.kind && r.spec == spec && r.host == forward.host)
}

/// Launch the tunnel and wait for ssh to background itself.
///
/// `-f` makes ssh fork once the forward is established, so this returns quickly
/// and a bind failure still comes back as a non-zero exit rather than a process
/// that is alive with nothing listening.
pub fn start(forward: &Forward) -> Result<()> {
    let output = Command::new("ssh")
        .args(forward.ssh_args())
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        bail!("ssh exited with {}", output.status);
    }
    bail!("{stderr}");
}

/// Stop by pid, never by pattern: a pattern would match the process doing the
/// matching (see ~/.claude/remote-ops.md).
pub fn stop(pid: u32) -> Result<()> {
    let status = Command::new("kill").arg(pid.to_string()).status()?;
    if status.success() {
        return Ok(());
    }
    bail!("kill {pid} exited with {status}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn reads_back_a_tunnel_this_tool_started() {
        // Exactly the cmdline observed for a live tunnel.
        let parsed = parse_argv(&argv(
            "ssh -f -N -o BatchMode=yes -o ExitOnForwardFailure=yes \
             -L 39997:127.0.0.1:22 kami",
        ));
        assert_eq!(
            parsed,
            Some((Kind::Local, "39997:127.0.0.1:22".into(), "kami".into()))
        );
    }

    #[test]
    fn reads_back_a_hand_written_tunnel_with_the_flags_in_another_order() {
        // The shape from the author's shell history: host before `-N`.
        let parsed = parse_argv(&argv("ssh -L 29001:0.0.0.0:9001 xx-database-1 -N"));
        assert_eq!(
            parsed,
            Some((
                Kind::Local,
                "29001:0.0.0.0:9001".into(),
                "xx-database-1".into()
            ))
        );
    }

    #[test]
    fn accepts_a_value_attached_to_its_flag() {
        let parsed = parse_argv(&argv("ssh -N -L8080:localhost:80 kami"));
        assert_eq!(
            parsed,
            Some((Kind::Local, "8080:localhost:80".into(), "kami".into()))
        );
    }

    #[test]
    fn accepts_a_cluster_of_booleans_before_the_flag() {
        let parsed = parse_argv(&argv("ssh -fN -R 8080:localhost:3000 kami"));
        assert_eq!(
            parsed,
            Some((Kind::Remote, "8080:localhost:3000".into(), "kami".into()))
        );
    }

    #[test]
    fn an_option_value_is_not_mistaken_for_the_host() {
        // `-p 2222` and `-o Foo=bar` both consume the next word; the host is the
        // first bare word that is left.
        let parsed = parse_argv(&argv(
            "ssh -p 2222 -o ServerAliveInterval=30 -L 1:2:3 realhost",
        ));
        assert_eq!(parsed.unwrap().2, "realhost");
    }

    #[test]
    fn a_proxycommand_helper_is_not_a_tunnel() {
        // Every jump host spawns one of these; matching it would invent tunnels.
        assert_eq!(
            parse_argv(&argv("/run/current-system/sw/bin/ssh lxb@kami -W %h:%p")),
            None
        );
    }

    #[test]
    fn a_plain_session_is_not_a_tunnel() {
        assert_eq!(parse_argv(&argv("ssh kami")), None);
    }

    #[test]
    fn a_dynamic_forward_is_not_claimed() {
        // SOCKS is out of scope, so `-D` must not be reported as a forward.
        assert_eq!(parse_argv(&argv("ssh -N -D 1080 kami")), None);
    }

    #[test]
    fn another_program_is_ignored_even_with_matching_flags() {
        assert_eq!(parse_argv(&argv("sshpass -L 1:2:3 kami")), None);
    }

    #[test]
    fn an_absolute_ssh_path_is_still_ssh() {
        let parsed = parse_argv(&argv("/usr/bin/ssh -N -L 1:h:2 kami"));
        assert_eq!(parsed.unwrap().0, Kind::Local);
    }

    #[test]
    fn a_flag_with_no_value_at_the_end_is_not_a_tunnel() {
        assert_eq!(parse_argv(&argv("ssh -N kami -L")), None);
    }

    #[test]
    fn find_matches_only_on_all_three_fields() {
        let running = vec![Running {
            pid: 1,
            kind: Kind::Local,
            spec: "29001:0.0.0.0:9001".into(),
            host: "db".into(),
        }];
        let mut forward = Forward {
            host: "db".into(),
            kind: Kind::Local,
            bind: "29001".into(),
            target: "0.0.0.0:9001".into(),
            note: String::new(),
        };
        assert!(find(&running, &forward).is_some());

        forward.kind = Kind::Remote;
        assert!(find(&running, &forward).is_none(), "direction ignored");

        forward.kind = Kind::Local;
        forward.target = "0.0.0.0:9002".into();
        assert!(find(&running, &forward).is_none(), "target ignored");

        forward.target = "0.0.0.0:9001".into();
        forward.host = "other".into();
        assert!(find(&running, &forward).is_none(), "host ignored");
    }
}
