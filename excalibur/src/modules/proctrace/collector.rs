use color_eyre::Result;
use procfs::process::{Process, all_processes};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Supervisor type for a process
#[derive(Debug, Clone, PartialEq)]
pub enum Supervisor {
    Systemd { unit: String },
    Docker { container_id: String },
    Shell,
    Unknown,
}

/// Warning types for processes
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessWarning {
    RunningAsRoot,
    HighCpu { percent: f32 },
    HighMemory { gb: f64 },
    LongUptime { days: u64 },
}

impl ProcessWarning {
    /// Get warning symbol
    pub fn symbol(&self) -> &str {
        match self {
            ProcessWarning::RunningAsRoot => "⚠ ROOT",
            ProcessWarning::HighCpu { .. } => "⚠ HIGH_CPU",
            ProcessWarning::HighMemory { .. } => "⚠ HIGH_MEM",
            ProcessWarning::LongUptime { .. } => "⚠ LONG_UPTIME",
        }
    }

    /// Get warning description
    pub fn description(&self) -> String {
        match self {
            ProcessWarning::RunningAsRoot => "Running as root".to_string(),
            ProcessWarning::HighCpu { percent } => format!("High CPU usage: {:.1}%", percent),
            ProcessWarning::HighMemory { gb } => format!("High memory usage: {:.1} GB", gb),
            ProcessWarning::LongUptime { days } => format!("Long uptime: {} days", days),
        }
    }

    /// Get warning color (for ratatui)
    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            ProcessWarning::RunningAsRoot => Color::Red,
            ProcessWarning::HighCpu { .. } => Color::Yellow,
            ProcessWarning::HighMemory { .. } => Color::Yellow,
            ProcessWarning::LongUptime { .. } => Color::Cyan,
        }
    }
}

/// Process information
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub cmdline: Vec<String>,
    pub user: String,
    pub cpu_percent: f32,
    pub memory_rss: u64, // bytes
    pub start_time: u64, // timestamp (seconds since epoch)
    pub supervisor: Supervisor,
    pub warnings: Vec<ProcessWarning>,
}

impl ProcessInfo {
    /// Get formatted memory string (e.g., "45.2 MB")
    pub fn memory_str(&self) -> String {
        let kb = self.memory_rss / 1024;
        if kb < 1024 {
            format!("{} KB", kb)
        } else {
            let mb = kb as f64 / 1024.0;
            if mb < 1024.0 {
                format!("{:.1} MB", mb)
            } else {
                format!("{:.1} GB", mb / 1024.0)
            }
        }
    }

    /// Get uptime duration in human-readable format
    pub fn uptime_str(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let uptime_secs = now.saturating_sub(self.start_time);

        let days = uptime_secs / 86400;
        let hours = (uptime_secs % 86400) / 3600;
        let minutes = (uptime_secs % 3600) / 60;

        if days > 0 {
            format!("{}d {}h {}m", days, hours, minutes)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        }
    }
}

/// CPU stats for calculating percentage
#[derive(Debug, Clone)]
struct CpuStats {
    utime: u64,
    stime: u64,
    timestamp: std::time::Instant,
}

/// Process collector with CPU tracking
#[derive(Debug)]
pub struct ProcessCollector {
    last_cpu_stats: HashMap<u32, CpuStats>,
}

impl ProcessCollector {
    pub fn new() -> Self {
        Self {
            last_cpu_stats: HashMap::new(),
        }
    }

    /// Collect all processes
    pub fn collect(&mut self) -> Result<Vec<ProcessInfo>> {
        let mut processes = Vec::new();
        let all_procs = all_processes()?;

        for proc_result in all_procs {
            if let Ok(process) = proc_result {
                if let Ok(info) = self.collect_process_info(&process) {
                    processes.push(info);
                }
            }
        }

        Ok(processes)
    }

    /// Collect information for a single process
    fn collect_process_info(&mut self, process: &Process) -> Result<ProcessInfo> {
        let pid = process.pid as u32;
        let stat = process.stat()?;
        let status = process.status().ok();

        // Get process name
        let name = stat.comm.clone();

        // Get parent PID
        let ppid = stat.ppid as u32;

        // Get command line
        let cmdline = process
            .cmdline()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<String>>();

        // Get user (UID)
        let user = if let Some(ref s) = status {
            s.ruid.to_string()
        } else {
            "?".to_string()
        };

        // Get memory (RSS in pages, convert to bytes)
        let page_size = procfs::page_size();
        let memory_rss = stat.rss * page_size;

        // Get start time (in clock ticks since boot, need to convert)
        let boot_time = procfs::boot_time_secs().unwrap_or(0);
        let ticks_per_second = procfs::ticks_per_second();
        let start_time = boot_time + (stat.starttime / ticks_per_second);

        // Calculate CPU percentage
        let cpu_percent = self.calculate_cpu_percent(pid, stat.utime, stat.stime);

        // Detect supervisor
        let supervisor = detect_supervisor(pid);

        // Create info struct
        let mut info = ProcessInfo {
            pid,
            ppid,
            name,
            cmdline,
            user,
            cpu_percent,
            memory_rss,
            start_time,
            supervisor,
            warnings: Vec::new(),
        };

        // Detect warnings
        info.warnings = detect_warnings(&info);

        Ok(info)
    }

    /// Calculate CPU percentage based on delta
    fn calculate_cpu_percent(&mut self, pid: u32, utime: u64, stime: u64) -> f32 {
        let now = std::time::Instant::now();
        let total_time = utime + stime;

        if let Some(last_stats) = self.last_cpu_stats.get(&pid) {
            let time_delta = now.duration_since(last_stats.timestamp).as_secs_f32();
            if time_delta > 0.0 {
                let cpu_delta = (total_time - (last_stats.utime + last_stats.stime)) as f32;
                let ticks_per_second = procfs::ticks_per_second() as f32;
                let cpu_percent = (cpu_delta / ticks_per_second) / time_delta * 100.0;

                // Update stats
                self.last_cpu_stats.insert(
                    pid,
                    CpuStats {
                        utime,
                        stime,
                        timestamp: now,
                    },
                );

                return cpu_percent.min(100.0 * num_cpus::get() as f32);
            }
        }

        // First time seeing this process, just store stats
        self.last_cpu_stats.insert(
            pid,
            CpuStats {
                utime,
                stime,
                timestamp: now,
            },
        );

        0.0
    }
}

/// Detect supervisor for a process
pub fn detect_supervisor(pid: u32) -> Supervisor {
    // Read cgroup file
    if let Ok(cgroup_content) = std::fs::read_to_string(format!("/proc/{}/cgroup", pid)) {
        // Check for systemd unit
        for line in cgroup_content.lines() {
            if line.contains(".service") {
                // Extract unit name
                // Format: 12:pids:/system.slice/nginx.service
                if let Some(unit_part) = line.split('/').last() {
                    return Supervisor::Systemd {
                        unit: unit_part.to_string(),
                    };
                }
            }

            // Check for Docker container
            if line.contains("/docker/") {
                // Extract container ID
                // Format: 11:cpuset:/docker/abc123def456...
                if let Some(container_part) = line.split("/docker/").nth(1) {
                    let container_id = container_part.trim_end_matches('\n').to_string();
                    let short_id = if container_id.len() > 12 {
                        &container_id[..12]
                    } else {
                        &container_id
                    };
                    return Supervisor::Docker {
                        container_id: short_id.to_string(),
                    };
                }
            }
        }
    }

    // Fallback: check if parent is systemd
    if pid == 1 {
        return Supervisor::Unknown;
    }

    // Check parent process
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
        // Parse PPID from stat
        if let Some(ppid_str) = stat.split_whitespace().nth(3) {
            if let Ok(ppid) = ppid_str.parse::<u32>() {
                if ppid == 1 {
                    return Supervisor::Systemd {
                        unit: "direct".to_string(),
                    };
                }
            }
        }
    }

    Supervisor::Shell
}

/// Detect warnings for a process
fn detect_warnings(info: &ProcessInfo) -> Vec<ProcessWarning> {
    let mut warnings = Vec::new();

    // Check root privileges (UID 0)
    if info.user == "0" {
        warnings.push(ProcessWarning::RunningAsRoot);
    }

    // Check high CPU (> 80%)
    if info.cpu_percent > 80.0 {
        warnings.push(ProcessWarning::HighCpu {
            percent: info.cpu_percent,
        });
    }

    // Check high memory (> 1GB = 1073741824 bytes)
    let memory_gb = info.memory_rss as f64 / 1073741824.0;
    if memory_gb > 1.0 {
        warnings.push(ProcessWarning::HighMemory { gb: memory_gb });
    }

    // Check long uptime (> 90 days)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let uptime_days = (now.saturating_sub(info.start_time)) / 86400;
    if uptime_days > 90 {
        warnings.push(ProcessWarning::LongUptime { days: uptime_days });
    }

    warnings
}

/// Read single process by PID (for query mode)
pub fn read_process(pid: u32) -> Result<ProcessInfo> {
    let process = Process::new(pid as i32)?;
    let stat = process.stat()?;
    let status = process.status().ok();

    // Get process name
    let name = stat.comm.clone();

    // Get parent PID
    let ppid = stat.ppid as u32;

    // Get command line
    let cmdline = process
        .cmdline()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<String>>();

    // Get user (UID)
    let user = if let Some(ref s) = status {
        s.ruid.to_string()
    } else {
        "?".to_string()
    };

    // Get memory (RSS in pages, convert to bytes)
    let page_size = procfs::page_size();
    let memory_rss = stat.rss * page_size;

    // Get start time
    let boot_time = procfs::boot_time_secs().unwrap_or(0);
    let ticks_per_second = procfs::ticks_per_second();
    let start_time = boot_time + (stat.starttime / ticks_per_second);

    // Detect supervisor
    let supervisor = detect_supervisor(pid);

    // No CPU calculation for single-process reads
    let cpu_percent = 0.0;

    // Create process info
    let mut info = ProcessInfo {
        pid,
        ppid,
        name,
        cmdline,
        user,
        cpu_percent,
        memory_rss,
        start_time,
        supervisor,
        warnings: Vec::new(),
    };

    // Detect warnings
    info.warnings = detect_warnings(&info);

    Ok(info)
}

/// Read working directory from /proc/[pid]/cwd
pub fn read_working_directory(pid: u32) -> Result<String> {
    let cwd_path = format!("/proc/{}/cwd", pid);
    let cwd = std::fs::read_link(&cwd_path)?;
    Ok(cwd.display().to_string())
}

/// Read environment variables from /proc/[pid]/environ
pub fn read_environment(pid: u32) -> Result<HashMap<String, String>> {
    let environ_path = format!("/proc/{}/environ", pid);
    let content = std::fs::read_to_string(&environ_path)?;

    let mut env_map = HashMap::new();
    for entry in content.split('\0') {
        if !entry.is_empty() {
            if let Some((key, value)) = entry.split_once('=') {
                env_map.insert(key.to_string(), value.to_string());
            }
        }
    }

    Ok(env_map)
}
