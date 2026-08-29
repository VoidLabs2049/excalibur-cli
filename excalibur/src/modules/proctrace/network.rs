use color_eyre::Result;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

/// Network protocol
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Protocol {
    Tcp,
    Udp,
}

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState {
    Listen,
    Established,
    TimeWait,
    CloseWait,
    Unknown,
}

/// Network binding information
#[derive(Debug, Clone)]
pub struct NetworkBinding {
    pub protocol: Protocol,
    pub local_addr: IpAddr,
    pub local_port: u16,
    pub remote_addr: Option<IpAddr>,
    pub remote_port: Option<u16>,
    pub state: ConnectionState,
    pub inode: u64,
}

/// Parse /proc/net/tcp file
pub fn parse_tcp_connections() -> Result<Vec<NetworkBinding>> {
    let content = std::fs::read_to_string("/proc/net/tcp")?;
    parse_connections(&content, Protocol::Tcp)
}

/// Parse /proc/net/udp file
pub fn parse_udp_connections() -> Result<Vec<NetworkBinding>> {
    let content = std::fs::read_to_string("/proc/net/udp")?;
    parse_connections(&content, Protocol::Udp)
}

/// Generic connection parser
fn parse_connections(content: &str, protocol: Protocol) -> Result<Vec<NetworkBinding>> {
    let mut connections = Vec::new();

    // Skip header line
    for line in content.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }

        // Parse addresses: "0100007F:EBF7" → 127.0.0.1:60407
        let local = match parse_address_v4(parts[1]) {
            Ok(addr) => addr,
            Err(_) => continue,
        };

        let remote =
            parse_address_v4(parts[2]).unwrap_or((IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0));

        // Parse state
        let state = parse_state(parts[3]);

        // Parse inode
        let inode: u64 = parts[9].parse().unwrap_or(0);

        connections.push(NetworkBinding {
            protocol,
            local_addr: local.0,
            local_port: local.1,
            remote_addr: Some(remote.0),
            remote_port: Some(remote.1),
            state,
            inode,
        });
    }

    Ok(connections)
}

/// Parse hex address format: "0100007F:EBF7" → (127.0.0.1, 60407)
fn parse_address_v4(hex_str: &str) -> Result<(IpAddr, u16)> {
    let parts: Vec<&str> = hex_str.split(':').collect();
    if parts.len() != 2 {
        return Err(color_eyre::eyre::eyre!("Invalid address format"));
    }

    // Parse little-endian hex IP: "0100007F" → 127.0.0.1
    let ip_hex = u32::from_str_radix(parts[0], 16)?;
    // Convert from little-endian: swap bytes
    let ip_bytes = ip_hex.to_le_bytes();
    let ip = IpAddr::V4(Ipv4Addr::from(ip_bytes));

    // Parse hex port (big-endian)
    let port = u16::from_str_radix(parts[1], 16)?;

    Ok((ip, port))
}

/// Parse connection state hex code
fn parse_state(hex_str: &str) -> ConnectionState {
    match hex_str {
        "0A" => ConnectionState::Listen,
        "01" => ConnectionState::Established,
        "06" => ConnectionState::TimeWait,
        "08" => ConnectionState::CloseWait,
        _ => ConnectionState::Unknown,
    }
}

/// Map socket inodes to the PIDs holding them, by scanning /proc/[pid]/fd
pub fn map_connections_to_pids() -> Result<HashMap<u64, u32>> {
    let mut inode_to_pid = HashMap::new();

    // Get all process PIDs
    let proc_dir = std::fs::read_dir("/proc")?;

    for entry in proc_dir.flatten() {
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Check if it's a numeric directory (PID)
        if let Ok(pid) = file_name_str.parse::<u32>() {
            // Read all file descriptors for this process
            let fd_path = format!("/proc/{}/fd", pid);
            if let Ok(fd_dir) = std::fs::read_dir(&fd_path) {
                for fd_entry in fd_dir.flatten() {
                    // Read symlink target
                    if let Ok(link_target) = std::fs::read_link(fd_entry.path()) {
                        let target_str = link_target.to_string_lossy();

                        // Check if it's a socket: "socket:[934413]"
                        if let Some(stripped) = target_str.strip_prefix("socket:[") {
                            if let Some(inode_str) = stripped.strip_suffix(']') {
                                if let Ok(inode) = inode_str.parse::<u64>() {
                                    inode_to_pid.insert(inode, pid);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(inode_to_pid)
}

/// Find process listening on specific port
pub fn find_process_by_port(port: u16) -> Result<Option<u32>> {
    // Parse all connections
    let mut all_conns = parse_tcp_connections()?;
    all_conns.extend(parse_udp_connections()?);

    // Filter for listening connections on this port
    let listening: Vec<_> = all_conns
        .iter()
        .filter(|c| c.local_port == port && c.state == ConnectionState::Listen)
        .collect();

    if listening.is_empty() {
        return Ok(None);
    }

    // Build inode → PID mapping
    let inode_map = map_connections_to_pids()?;

    // Find first match
    for conn in listening {
        if let Some(&pid) = inode_map.get(&conn.inode) {
            return Ok(Some(pid));
        }
    }

    Ok(None)
}

/// Get all network bindings for a specific process
pub fn get_process_bindings(pid: u32) -> Result<Vec<NetworkBinding>> {
    // Parse all connections
    let mut all_conns = parse_tcp_connections()?;
    all_conns.extend(parse_udp_connections()?);

    // Build inode → PID mapping
    let inode_map = map_connections_to_pids()?;

    // Filter connections for this PID
    let bindings: Vec<_> = all_conns
        .into_iter()
        .filter(|conn| inode_map.get(&conn.inode) == Some(&pid))
        .collect();

    Ok(bindings)
}
