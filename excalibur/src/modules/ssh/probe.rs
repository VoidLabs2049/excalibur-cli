use super::tunnels::{Forward, Kind};
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// How long to wait for the TCP handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1200);
/// How long to wait for the far side to drop a connection it could not complete.
const SETTLE_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// Nothing running.
    Off,
    Ok,
    Bad,
    /// Cannot be answered from this machine.
    Unknown,
}

impl Light {
    pub fn symbol(self) -> &'static str {
        match self {
            Light::Off => "o",
            Light::Ok => "*",
            Light::Bad => "x",
            Light::Unknown => "-",
        }
    }
}

/// What can be observed about one rule, layer by layer.
///
/// The layers are separated because the interesting failures live between them:
/// a live process with nothing listening is a forward that never took effect,
/// and a bound port whose path fails is a tunnel to a service that is down.
#[derive(Debug, Clone)]
pub struct Health {
    /// Is the ssh process up.
    pub process: Light,
    /// Is the port actually listening. Only observable for `-L`: a `-R` rule
    /// opens its port on the far side, which nothing here can see.
    pub port: Light,
    /// For `-L`, whether a connection through the tunnel survives; for `-R`,
    /// whether the thing being exposed is up on this machine.
    pub path: Light,
    pub detail: String,
}

impl Health {
    /// A rule whose process is up but whose measurement has not landed yet.
    pub fn measuring() -> Self {
        Health {
            process: Light::Ok,
            port: Light::Unknown,
            path: Light::Unknown,
            detail: String::new(),
        }
    }

    pub fn stopped() -> Self {
        Health {
            process: Light::Off,
            port: Light::Off,
            path: Light::Off,
            detail: String::new(),
        }
    }

    pub fn lights(&self) -> String {
        format!(
            "{} {} {}",
            self.process.symbol(),
            self.port.symbol(),
            self.path.symbol()
        )
    }
}

/// Measure a rule that has a live process behind it.
pub fn check(forward: &Forward) -> Health {
    match forward.kind {
        Kind::Local => check_local(forward),
        Kind::Remote => check_remote(forward),
    }
}

fn check_local(forward: &Forward) -> Health {
    let Some(port) = bind_port(&forward.bind) else {
        return Health {
            process: Light::Ok,
            port: Light::Unknown,
            path: Light::Unknown,
            detail: format!("cannot read a port out of `{}`", forward.bind),
        };
    };

    let bound = listening(port);
    if bound == Some(false) {
        return Health {
            process: Light::Ok,
            port: Light::Bad,
            path: Light::Off,
            // The failure ExitOnForwardFailure is meant to prevent, seen from
            // the other side: process up, nothing listening.
            detail: format!("running but nothing is listening on {port}"),
        };
    }

    let (path, detail) = match reachable(&format!("127.0.0.1:{port}")) {
        Reach::Open => (Light::Ok, String::new()),
        Reach::Closed => (
            Light::Bad,
            format!("tunnel is up but {} refused it", forward.target),
        ),
        Reach::Refused => (
            Light::Bad,
            format!("nothing accepted a connection on {port}"),
        ),
    };
    Health {
        process: Light::Ok,
        port: bound.map(|_| Light::Ok).unwrap_or(Light::Unknown),
        path,
        detail,
    }
}

fn check_remote(forward: &Forward) -> Health {
    // The listening port is on the far side. What is checkable here is the exit:
    // whether the service being exposed is actually up.
    let (path, detail) = match reachable(&forward.target) {
        Reach::Open => (
            Light::Ok,
            "remote bind is not visible from here; needs GatewayPorts to reach it \
             from a third machine"
                .to_string(),
        ),
        Reach::Closed | Reach::Refused => (
            Light::Bad,
            format!("nothing is serving {} on this machine", forward.target),
        ),
    };
    Health {
        process: Light::Ok,
        port: Light::Unknown,
        path,
        detail,
    }
}

enum Reach {
    /// Connected and stayed connected.
    Open,
    /// Connected, then the far side hung up -- the tunnel exists but its target
    /// did not answer.
    Closed,
    /// Nothing accepted at all.
    Refused,
}

fn reachable(address: &str) -> Reach {
    let Ok(mut addresses) = address.to_socket_addrs() else {
        return Reach::Refused;
    };
    let Some(addr) = addresses.next() else {
        return Reach::Refused;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return Reach::Refused;
    };

    // ssh accepts locally first and only then opens the channel to the far side,
    // so a forward whose target is down looks like an immediate EOF. A read that
    // times out, or that returns a banner, both mean the far side answered.
    if stream.set_read_timeout(Some(SETTLE_TIMEOUT)).is_err() {
        return Reach::Open;
    }
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => Reach::Closed,
        _ => Reach::Open,
    }
}

/// The port out of a bind spec: `29001` or `0.0.0.0:29001`.
pub fn bind_port(bind: &str) -> Option<u16> {
    bind.rsplit(':').next()?.trim().parse().ok()
}

/// Whether anything is listening on `port`, or `None` where that cannot be read.
#[cfg(target_os = "linux")]
pub fn listening(port: u16) -> Option<bool> {
    let mut readable = false;
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        readable = true;
        if text.lines().skip(1).any(|line| listens_on(line, port)) {
            return Some(true);
        }
    }
    readable.then_some(false)
}

/// macOS has no `/proc/net/tcp`. `netstat` is in the base system and needs no
/// privileges to list sockets it does not own, which `lsof` does.
#[cfg(target_os = "macos")]
pub fn listening(port: u16) -> Option<bool> {
    let output = std::process::Command::new("netstat")
        .args(["-an", "-p", "tcp"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| listens_in_netstat(&String::from_utf8_lossy(&output.stdout), port))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn listening(_port: u16) -> Option<bool> {
    None
}

/// A `/proc/net/tcp` row: field 1 is `HEXADDR:HEXPORT`, field 3 is the state,
/// and `0A` is LISTEN.
///
/// Matched on the port alone, not the address: a tunnel binds loopback and v6
/// separately, and `ExitOnForwardFailure` means a live ssh really does own the
/// port it asked for.
///
/// Cfg'd for the same reason as [`listens_in_netstat`], the other way round:
/// each parser is dead code on the platform that does not use it, and the tests
/// for both should run wherever they are built.
#[cfg(any(target_os = "linux", test))]
fn listens_on(line: &str, port: u16) -> bool {
    let mut fields = line.split_whitespace().skip(1);
    let Some(local) = fields.next() else {
        return false;
    };
    if fields.next().is_none() || fields.next() != Some("0A") {
        return false;
    }
    local
        .rsplit(':')
        .next()
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        == Some(port)
}

/// Whether a BSD `netstat -an -p tcp` listing has `port` in LISTEN.
///
/// Two things differ from the Linux side and both fail silently if missed: the
/// address separator is `.`, not `:`, and this listing includes every
/// connection rather than only listeners, so an established connection to the
/// same port would otherwise read as a bound one.
///
/// Compiled on Linux only for its test -- the parsing is the part that can be
/// wrong, and it should not need a Mac to find that out.
#[cfg(any(target_os = "macos", test))]
fn listens_in_netstat(text: &str, port: u16) -> bool {
    text.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.first().is_some_and(|proto| proto.starts_with("tcp"))
            && fields.last() == Some(&"LISTEN")
            && fields
                .get(3)
                .and_then(|local| local.rsplit_once('.'))
                .and_then(|(_, port)| port.parse::<u16>().ok())
                == Some(port)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn a_bind_spec_yields_its_port_with_or_without_an_address() {
        assert_eq!(bind_port("29001"), Some(29001));
        assert_eq!(bind_port("0.0.0.0:29001"), Some(29001));
        assert_eq!(bind_port("127.0.0.1:8080"), Some(8080));
        assert_eq!(bind_port("not-a-port"), None);
    }

    #[test]
    fn a_listen_row_is_recognised_by_port_and_state() {
        // Copied from /proc/net/tcp with a live tunnel on 39995 (0x9C3B).
        let row = "  20: 0100007F:9C3B 00000000:0000 0A 00000000:00000000 \
                   00:00000000 00000000  1002        0 8910121 1";
        assert!(listens_on(row, 39995));
        assert!(!listens_on(row, 39996), "matched the wrong port");
    }

    #[test]
    fn an_established_row_is_not_a_listener() {
        // Same port, state 01 (ESTABLISHED). Counting it would report a forward
        // as bound because something merely connected to it once.
        let row = "  20: 0100007F:9C3B 0100007F:1234 01 00000000:00000000 \
                   00:00000000 00000000  1002        0 8910121 1";
        assert!(!listens_on(row, 39995));
    }

    /// Real `netstat -an -p tcp` output from macOS, with a tunnel on 6022 bound
    /// on both families and an established connection to 443.
    const NETSTAT: &str = "\
Active Internet connections (including servers)
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)
tcp4       0      0  127.0.0.1.6022         *.*                    LISTEN
tcp6       0      0  ::1.6022               *.*                    LISTEN
tcp46      0      0  *.22                   *.*                    LISTEN
tcp4       0      0  192.168.1.5.52134      17.253.144.10.443      ESTABLISHED
";

    #[test]
    fn a_bsd_listen_row_is_recognised_through_its_dotted_port() {
        assert!(listens_in_netstat(NETSTAT, 6022));
        assert!(
            listens_in_netstat(NETSTAT, 22),
            "the wildcard row was missed"
        );
        assert!(!listens_in_netstat(NETSTAT, 60), "matched a partial port");
    }

    #[test]
    fn a_bsd_established_row_is_not_a_listener() {
        // `netstat -an` lists every connection, not only listeners. Without the
        // state column the far port of this row reports 443 as bound here.
        assert!(!listens_in_netstat(NETSTAT, 443));
        assert!(!listens_in_netstat(NETSTAT, 52134));
    }

    #[test]
    fn bsd_headers_are_not_listeners() {
        assert!(!listens_in_netstat(NETSTAT, 0));
        assert!(!listens_in_netstat("", 6022));
    }

    #[test]
    fn a_port_this_test_is_holding_reads_as_listening() {
        // Only the positive direction is asserted: the match is by port number
        // alone, so another process listening on the same port on a different
        // address would make a "now it is free" assertion flaky. That is
        // acceptable here -- ExitOnForwardFailure means a live ssh owns its
        // port, so a match on the port is a match on the tunnel.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(listening(port), Some(true));
    }

    #[test]
    fn a_port_nobody_serves_is_not_reachable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(matches!(
            reachable(&format!("127.0.0.1:{port}")),
            Reach::Refused
        ));
    }

    #[test]
    fn a_listener_that_holds_the_connection_reads_as_open() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let accepted = listener.accept();
            std::thread::sleep(Duration::from_millis(600));
            drop(accepted);
        });
        assert!(matches!(
            reachable(&format!("127.0.0.1:{port}")),
            Reach::Open
        ));
        handle.join().unwrap();
    }

    #[test]
    fn a_listener_that_hangs_up_immediately_reads_as_closed() {
        // This is what a `-L` tunnel looks like when its far-side target is down:
        // ssh accepts, fails to open the channel, and drops the connection.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });
        assert!(matches!(
            reachable(&format!("127.0.0.1:{port}")),
            Reach::Closed
        ));
        handle.join().unwrap();
    }

    #[test]
    fn a_stopped_rule_shows_three_dark_lights() {
        assert_eq!(Health::stopped().lights(), "o o o");
    }
}
