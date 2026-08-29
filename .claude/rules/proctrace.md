---
paths:
  - "excalibur/src/modules/proctrace/mod.rs"
  - "excalibur/src/modules/proctrace/collector.rs"
  - "excalibur/src/modules/proctrace/network.rs"
  - "excalibur/src/modules/proctrace/query.rs"
  - "excalibur/src/modules/proctrace/state.rs"
  - "excalibur/src/modules/proctrace/systemd.rs"
  - "excalibur/src/modules/proctrace/ui.rs"
---

# Process Tracer

Query-driven process inspector ("Why is this running?"). Queries by name, PID, or port; shows ancestor chain, network bindings, systemd metadata, environment, and warnings — then hands a ready-made command back to the shell.

**Linux-only**: the entire module is gated behind `#[cfg(target_os = "linux")]` — the `proctrace` module, the `ModuleId::ProcessTracer` variant, its `ModuleManager` registration, and the `process-tracer` CLI subcommand all compile out on non-Linux targets (it reads `/proc`). See [core.md](core.md).

## Key Files

| File | Role |
|------|------|
| `mod.rs` | `ProcessTracerModule` — implements `Module`, routes Query/ViewResults input modes |
| `collector.rs` | `ProcessCollector` — reads `/proc` via `procfs` crate, detects supervisor (systemd/docker/shell), computes CPU%, generates warnings |
| `network.rs` | Parses `/proc/net/tcp` and `/proc/net/udp`, maps socket inodes to PIDs via `/proc/[pid]/fd` symlinks |
| `query.rs` | `QueryEngine` — executes `QueryType` (ByName/ByPid/ByPort), builds `QueryResult` with ancestor chain, env, network, systemd metadata |
| `state.rs` | `ProcessTracerState` — query input, history navigation, result selection, scroll offset |
| `systemd.rs` | Runs `systemctl show <unit>` and parses KEY=VALUE output into `SystemdMetadata` |
| `ui.rs` | Two-mode rendering: query input with help, or results list + scrollable details panel with scrollbar |

## Architecture / Data Flow

```
User input (query string)
  → state.parse_query() → QueryType (ByName | ByPid | ByPort)
  → QueryEngine::execute()
      ByName → ProcessCollector::collect() → filter by name
      ByPid  → read_process(pid)
      ByPort → parse /proc/net/tcp+udp → inode→PID map → read_process
    → build_query_result():
        ancestor_chain (walk PPID → PID 1)
        read_working_directory (/proc/[pid]/cwd)
        read_environment (/proc/[pid]/environ)
        get_process_bindings (network)
        fetch_systemd_metadata (if systemd supervisor)
  → Vec<QueryResult> → state.query_results
```

## Emitting commands (`ViewResults` mode)

The query already resolved the parts that are hard to recall — PID, systemd unit,
working directory — so `emit_from_selection()` in `mod.rs` is pure formatting.
Run via `ex pt` for the emitted command to reach the command line.

| Key | Emits | Requires |
|-----|-------|----------|
| `l` | `journalctl -u <unit> -f` | `Supervisor::Systemd` |
| `r` | `systemctl restart <unit>` | `Supervisor::Systemd` |
| `x` | `kill <pid>` | — |
| `c` | `cd <cwd>` | `working_directory` |

All four use `ModuleAction::Output` (exit 0, editable), never `OutputAndExecute`:
they kill or restart things, so the target is worth a glance first. When the
precondition is missing, the module sets a notification instead of emitting.

## Design Patterns

- **Query-driven**: No auto-refresh; data collected only on explicit query execution
- **Supervisor detection**: Reads `/proc/[pid]/cgroup` for `.service` (systemd) or `/docker/` (container), falls back to PPID==1 check
- **Warning system**: `ProcessWarning` enum with `symbol()`, `description()`, `color()` methods for consistent UI rendering
- **CPU delta tracking**: `ProcessCollector` stores previous utime/stime per PID to compute CPU% across calls

## Gotchas

- Port queries for root-owned processes require root/sudo (inode→PID mapping reads `/proc/[pid]/fd`)
- `read_process()` is a standalone function (no CPU tracking) vs `ProcessCollector::collect_process_info()` (with CPU delta)
- Network parsing only handles IPv4 (`/proc/net/tcp`, `/proc/net/udp`); IPv6 (`tcp6`/`udp6`) not yet supported

## How to Extend

- Add new query type: add variant to `QueryType`, implement in `QueryEngine`, update `state.parse_query()` parsing
- Add IPv6 support: parse `/proc/net/tcp6` and `/proc/net/udp6` in `network.rs`
- Add new detail section: extend `QueryResult` fields, update `render_detailed_analysis()` in `ui.rs`

## Testing

- **Framework**: No tests currently
- **Run**: `cargo test` (from `excalibur/`)

## Dependencies

- procfs 0.17 — read `/proc` filesystem
- num_cpus 1.16 — cap CPU% calculation
