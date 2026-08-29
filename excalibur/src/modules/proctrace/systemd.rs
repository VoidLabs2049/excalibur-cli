use color_eyre::Result;
use std::collections::HashMap;
use std::process::Command;

/// Rich systemd metadata from `systemctl show`
#[derive(Debug, Clone)]
pub struct SystemdMetadata {
    pub unit_name: String,
    pub description: Option<String>,
    pub active_state: String, // active, inactive, failed
    pub sub_state: String,    // running, dead, exited
    pub exec_start: Option<String>,
    pub restart_policy: Option<String>,
}

/// Fetch rich metadata from systemctl show
pub fn fetch_systemd_metadata(unit_name: &str) -> Result<SystemdMetadata> {
    let output = Command::new("systemctl")
        .args(["show", unit_name])
        .output()?;

    if !output.status.success() {
        return Err(color_eyre::eyre::eyre!(
            "systemctl show failed for {}",
            unit_name
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let properties = parse_systemctl_output(&stdout);

    Ok(SystemdMetadata {
        unit_name: unit_name.to_string(),
        description: properties.get("Description").cloned(),
        active_state: properties
            .get("ActiveState")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        sub_state: properties
            .get("SubState")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        exec_start: properties.get("ExecStart").cloned(),
        restart_policy: properties.get("Restart").cloned(),
    })
}

/// Parse systemctl show output (KEY=VALUE format)
fn parse_systemctl_output(output: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            props.insert(key.to_string(), value.to_string());
        }
    }

    props
}
