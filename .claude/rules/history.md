---
paths:
  - "excalibur/src/modules/history/mod.rs"
  - "excalibur/src/modules/history/parser.rs"
  - "excalibur/src/modules/history/state.rs"
  - "excalibur/src/modules/history/clipboard.rs"
  - "excalibur/src/modules/history/ui.rs"
---

# History

Fish shell command history browser with search, sort, clipboard copy, and shell integration (output selected command to stdout).

## Key Files

| File | Role |
|------|------|
| `mod.rs` | `HistoryModule` — implements `Module` trait, routes Normal/Search input modes |
| `parser.rs` | `FishHistoryParser` — parses `~/.local/share/fish/fish_history` (YAML-like), aggregates by command, computes stats |
| `state.rs` | `HistoryState` — filtered indices, sort modes (UsageCount/Timestamp/Alphabetical), selection, notifications |
| `clipboard.rs` | `ClipboardManager` — wraps `arboard::Clipboard` with graceful fallback |
| `ui.rs` | Renders header, search bar, virtual-scrolled table, details panel, status bar, notification popup |

## Architecture / Data Flow

```
FishHistoryParser::parse()
  → Vec<CommandEntry> (aggregated: cmd, count, timestamp, paths)
  → HistoryState::new(commands, stats)

User input:
  Normal mode: navigate (j/k/g/G), sort (s), copy (y), select (Enter), execute (Ctrl+O)
  Search mode: type to filter → apply_filters() → re-sort filtered_indices
```

## Design Patterns

- **Preloaded data**: History is parsed once in `HistoryModule::new()`, not on each `init()`. Only UI state resets on re-entry.
- **Virtual scrolling**: `ui.rs` only renders visible rows (max 30), computing a scroll window around `selected_index`
- **Index indirection**: `filtered_indices: Vec<usize>` indexes into `commands`, allowing sort/filter without cloning

## How to Extend

- Add new sort mode: add variant to `SortMode`, implement in `apply_sort()`, update `display()` and `next()`
- Support other shells: create a new parser implementing the same `CommandEntry`/`HistoryStats` output

## Testing

- **No tests in this module.** `cargo test` (from `excalibur/`) passes 169, but every one of them is in `ssh`.

## Dependencies

- chrono 0.4 — timestamp formatting
- arboard 3.4 — clipboard access
- dirs 5.0 — locate Fish history file
- serde — `RawEntry` carries a `Deserialize` derive, but `parse_raw_entries()` reads the file line by line; nothing goes through serde here
