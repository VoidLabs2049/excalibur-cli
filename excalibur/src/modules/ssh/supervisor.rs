use super::tunnels::{Forward, Kind};
use color_eyre::Result;
use color_eyre::eyre::bail;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
/// Runs once a second on the render thread, which is why Linux keeps its own
/// implementation instead of sharing the portable one: over ~650 processes
/// `/proc` costs about 5ms and `sysinfo` 45-85ms, and the slow one drops frames
/// every second on a screen whose whole point is that it stays live.
///
/// Both are restricted to our own uid, because argv is world-readable: without
/// that another user's tunnel is listed as unclaimed and offered for stopping,
/// and the `kill` behind that offer can only fail.
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
    scan_portable()
}

/// [`scan`] for everything without `/proc`, macOS included.
///
/// `sysinfo::Process::cmd` is real argv there (it reads `KERN_PROCARGS2`), which
/// is what [`parse_argv`] needs: re-splitting the space-joined line that `ps`
/// prints would quietly mis-parse any argument containing a space, and claiming
/// by argv structure is the one thing this module will not trade away.
///
/// Compiled on Linux too, but only for the test below -- otherwise the macOS
/// path would first be built on a Mac, which is where it is hardest to fix.
#[cfg(any(not(target_os = "linux"), test))]
fn scan_portable() -> Vec<Running> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    let me = sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid))
        .and_then(|process| process.user_id().cloned());
    system
        .processes()
        .values()
        .filter(|process| me.is_none() || process.user_id() == me.as_ref())
        .filter_map(|process| {
            let argv: Vec<String> = process
                .cmd()
                .iter()
                .map(|word| word.to_string_lossy().into_owned())
                .collect();
            let (kind, spec, host) = parse_argv(&argv)?;
            Some(Running {
                pid: process.pid().as_u32(),
                kind,
                spec,
                host,
            })
        })
        .collect()
}

/// What one tunnel process has been doing, as opposed to what it is.
///
/// Kept apart from [`Running`] on purpose: `Running` answers "which rule is this
/// process", and nothing here may be read until that answer exists. Measuring
/// first and identifying afterwards is how you end up carefully charting a
/// process that was never the one you meant (see ~/.claude/remote-ops.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub uptime: Duration,
    /// Bytes the process has read, where that can be counted at all.
    ///
    /// A forwarder reads each payload byte once on its way in and writes it once
    /// on its way out, in both directions, so `rchar` alone already tracks
    /// everything through the tunnel -- adding `wchar` counts the same bytes
    /// twice and silently doubles every rate on screen.
    ///
    /// `None` off Linux, and it has to stay `None` all the way to the screen: a
    /// stand-in of `0` would come back out of the delta as a rate of `Some(0.0)`
    /// and print `0B/s`, which is the reading for a live tunnel nobody is using.
    pub read: Option<u64>,
}

/// Measure a process that has *already* been confirmed to be the tunnel's.
#[cfg(target_os = "linux")]
pub fn usage(pid: u32) -> Option<Usage> {
    let process = procfs::process::Process::new(i32::try_from(pid).ok()?).ok()?;
    let started = procfs::boot_time_secs().ok()?
        + process.stat().ok()?.starttime / procfs::ticks_per_second();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Usage {
        uptime: Duration::from_secs(now.saturating_sub(started)),
        read: Some(process.io().ok()?.rchar),
    })
}

/// Uptime carries over; the byte counter does not. `/proc/<pid>/io` has no
/// macOS counterpart that counts socket traffic -- `proc_pid_rusage` counts
/// disk -- so the traffic column stays blank rather than showing a number that
/// measures something else.
///
/// This is also why uptime is the column drawn first: on macOS it is the only
/// one left.
#[cfg(not(target_os = "linux"))]
pub fn usage(pid: u32) -> Option<Usage> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let target = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let started = system.process(target)?.start_time();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Usage {
        uptime: Duration::from_secs(now.saturating_sub(started)),
        read: None,
    })
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
    use crate::modules::ssh::tunnels::Protocol;

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
    fn the_portable_scanner_agrees_with_the_one_this_platform_uses() {
        // What this really buys on Linux is that the macOS path is compiled and
        // run at all -- most of the time both sides are empty, and the sameness
        // only gets tested when a tunnel happens to be up. Discovering that the
        // portable scanner does not build is worth much more on this machine
        // than on the Mac where it would otherwise happen first.
        let mut mine = scan();
        let mut portable = scan_portable();
        mine.sort_by_key(|r| r.pid);
        portable.sort_by_key(|r| r.pid);
        assert_eq!(mine, portable);
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
            protocol: Protocol::Tcp,
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
