use color_eyre::Result;
use color_eyre::eyre::bail;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

/// A port something is listening on, on the far side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub port: u16,
    /// The addresses it was seen bound to, joined for display.
    pub address: String,
    /// True only when *every* binding of this port is a loopback address.
    ///
    /// These are the ones worth forwarding: nothing outside that machine can
    /// reach them, which is the entire reason `-L` exists. A port also bound to
    /// `0.0.0.0` is already reachable directly.
    pub loopback: bool,
}

/// Ask `host` what it is listening on.
///
/// `ss` is not on every machine, so the fallback is wrapped in `sh -c` rather
/// than sent as a bare `a || b`: the remote login shell here is fish, whose
/// syntax differs enough that shell one-liners are a standing trap
/// (see ~/.claude/remote-ops.md). `sh -c '<one argument>'` parses identically
/// in both.
const REMOTE: &str = "sh -c 'ss -tlnH 2>/dev/null || netstat -tln'";

pub fn listeners(host: &str) -> Result<Vec<Listener>> {
    let output = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            host,
            REMOTE,
        ])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(match stderr.is_empty() {
            true => format!("ssh {host} exited with {}", output.status),
            false => stderr.lines().next().unwrap_or_default().to_string(),
        });
    }
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

/// Pull the listening ports out of either tool's output.
///
/// Both put the local address in the fourth column, so the difference is only
/// which lines count: `ss -tlnH` emits `LISTEN ...` with no header, `netstat`
/// emits two header lines and rows starting `tcp`/`tcp6`.
///
/// Ports are merged across address families -- `127.0.0.1:631` and `[::1]:631`
/// are one service, and listing them twice would have you forward the same
/// thing twice.
pub fn parse(text: &str) -> Vec<Listener> {
    let mut found: BTreeMap<u16, (Vec<String>, bool)> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = fields.first() else {
            continue;
        };
        if !matches!(*first, "LISTEN" | "tcp" | "tcp6") {
            continue;
        }
        let Some(local) = fields.get(3) else { continue };
        let Some((address, port)) = local.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        let address = address.trim_matches(['[', ']']).to_string();

        let entry = found.entry(port).or_insert((Vec::new(), true));
        // One non-loopback binding is enough to make the port directly
        // reachable, so this can only ever be turned off.
        entry.1 &= is_loopback(&address);
        if !entry.0.contains(&address) {
            entry.0.push(address);
        }
    }

    let mut listeners: Vec<Listener> = found
        .into_iter()
        .map(|(port, (addresses, loopback))| Listener {
            port,
            address: addresses.join(", "),
            loopback,
        })
        .collect();
    // Loopback-only first: they are the ones that cannot be reached any other
    // way, and on this author's machines they are ten of every twelve.
    listeners.sort_by_key(|l| (!l.loopback, l.port));
    listeners
}

fn is_loopback(address: &str) -> bool {
    matches!(address, "127.0.0.1" | "::1" | "localhost") || address.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `ss -tlnH` output, including the `[::1]` form and a port bound on
    /// both families.
    const SS: &str = "\
LISTEN 0      4096       127.0.0.1:8080       0.0.0.0:*
LISTEN 0      511          0.0.0.0:80         0.0.0.0:*
LISTEN 0      128            [::1]:631           [::]:*
LISTEN 0      128        127.0.0.1:631        0.0.0.0:*
LISTEN 0      128             [::]:80            [::]:*
";

    const NETSTAT: &str = "\
Active Internet connections (only servers)
Proto Recv-Q Send-Q Local Address           Foreign Address         State
tcp        0      0 127.0.0.1:8080          0.0.0.0:*               LISTEN
tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN
tcp6       0      0 ::1:631                 :::*                    LISTEN
";

    #[test]
    fn loopback_only_ports_come_first() {
        // They are the point of the feature: nothing outside that machine can
        // reach them, so they are the ones `-L` exists for.
        let ports: Vec<u16> = parse(SS).iter().map(|l| l.port).collect();
        assert_eq!(ports, [631, 8080, 80]);
    }

    #[test]
    fn a_port_bound_on_both_families_is_listed_once() {
        // 631 appears as 127.0.0.1 and [::1]. Listing it twice would have you
        // forward the same service twice.
        let found = parse(SS);
        assert_eq!(found.iter().filter(|l| l.port == 631).count(), 1);
        let cups = found.iter().find(|l| l.port == 631).unwrap();
        assert!(cups.loopback);
        assert!(cups.address.contains("::1"), "got: {}", cups.address);
        assert!(cups.address.contains("127.0.0.1"), "got: {}", cups.address);
    }

    #[test]
    fn one_public_binding_makes_the_whole_port_public() {
        // 80 is on 0.0.0.0 and [::]; either way it is already reachable and
        // forwarding it would be pointless.
        let found = parse(SS);
        assert!(!found.iter().find(|l| l.port == 80).unwrap().loopback);
    }

    #[test]
    fn netstat_output_parses_the_same_way() {
        // The fallback for machines without `ss`. Its header rows must not turn
        // into ports.
        let found = parse(NETSTAT);
        let ports: Vec<u16> = found.iter().map(|l| l.port).collect();
        assert_eq!(ports, [631, 8080, 22]);
        assert!(found[0].loopback && found[1].loopback);
        assert!(!found[2].loopback);
    }

    #[test]
    fn noise_does_not_become_a_port() {
        assert!(parse("").is_empty());
        assert!(parse("bash: ss: command not found\n").is_empty());
        assert!(parse("LISTEN 0 128\n").is_empty(), "a short row was read");
    }
}
