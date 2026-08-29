---
paths:
  - "excalibur/src/modules/settings/mod.rs"
  - "excalibur/src/modules/settings/state.rs"
  - "excalibur/src/modules/settings/ui.rs"
  - "excalibur/install/excc.fish"
---

# Settings

Claude Code settings profile manager. Lists `~/.claude/settings*.json` files, previews JSON content, swaps the active `settings.json` with a selected profile, and supports copy / rename / delete plus an in-place JSON key-value editor.

This module emits no commands — every action is a file operation it performs itself, so there is nothing to hand back to the shell. It still routes through `ex s`, which simply repaints when a module emits nothing.

## Key Files

| File | Role |
|------|------|
| `mod.rs` | `SettingsModule` — implements `Module`, dispatches the 8 `InputMode`s to per-mode handlers, executes swap/copy/rename/delete file operations |
| `state.rs` | `SettingsState` — profile scanning, selection, JSON preview, rename input buffer (with cursor), and the flattened key-value editor (`edit_entries`, `edit_value_buf`, `edit_cursor`) |
| `ui.rs` | Vertical layout: header / (profile list + preview-or-edit panel) / action bar / status bar; preview switches to the key-value edit panel in `EditKeys`/`EditValue` modes; notification overlay |
| `install/excc.fish` | Fish function `excc` — a thin alias for `ex s` (see `install/ex.fish`) |

## Input Modes (`InputMode`)

| Mode | Entered by | Action bar / behavior |
|------|-----------|-----------------------|
| `SelectProfile` | default | `[Enter]` switch · `[c]` copy · `[r]` rename · `[d]` delete · `[e]` edit · `j/k` navigate |
| `ConfirmSwap` | `Enter` on inactive profile | `[Enter]` switch (deletes old) · `[b]` backup first · `[Esc]` cancel |
| `BackupRename` | `b` in `ConfirmSwap` | type a backup filename, `[Enter]` renames active → backup then copies in selected |
| `InputCopyName` | `c` | type new filename, `[Enter]` copies selected profile |
| `InputRenameName` | `r` (blocked on active) | type new filename, `[Enter]` renames selected profile |
| `ConfirmDelete` | `d` (blocked on active) | `[Enter]` deletes selected profile |
| `EditKeys` | `e` | navigate flattened keys with `j/k`, `[Enter]` to edit the selected value |
| `EditValue` | `Enter` in `EditKeys` | text input with cursor (`←/→`, `Ctrl+⌫` clears), `[Enter]` saves back to disk |

## Architecture / Data Flow

```
init() -> load_profiles() -> scan ~/.claude/settings*.json
  -> Vec<ProfileEntry> { name, path, is_active }   (active first, then alphabetical)
  -> update_preview() pretty-prints selected file's JSON

Switch (SelectProfile -> ConfirmSwap):
  [Enter] execute_swap(false): remove active settings.json, copy selected -> settings.json
  [b]     BackupRename -> execute_swap(true): rename active -> backup, then copy selected in

Copy / Rename / Delete:
  execute_copy   — std::fs::copy selected -> new name
  execute_rename — std::fs::rename selected -> new name (refuses active)
  execute_delete — std::fs::remove_file selected (refuses active)

Edit (EditKeys -> EditValue):
  parse_json_entries() -> flatten_json() builds Vec<(key_path, value)>
  save_edit() reparses key paths, infers value type, writes nested JSON back, refreshes preview
```

## Design Patterns

- **Active profile is protected**: rename and delete are refused on `settings.json`; switching to it is a no-op ("Already the active profile").
- **Backup-or-replace swap**: `execute_swap(backup)` either renames the current `settings.json` to a user-named backup or deletes it before copying the selected profile in.
- **Flattened JSON editor**: nested objects are flattened to single rows; on save, `save_edit()` infers each value's type — `true`/`false`/`null`, then `i64`, then `f64`, else plain `String` — and writes the nested structure back via `serde_json`.
- **Notifications**: transient messages set via `set_notification()`, auto-cleared after 3s in `update()`.

## Gotchas

- **Key separator is `\0`, not `.`** — `flatten_json()` joins nested keys with `SettingsState::KEY_SEP` (`'\0'`) so that real keys containing dots (e.g. `env."FOO.BAR"`) are not mis-split. The UI renders `\0` back as `.`; `save_edit()` splits on `\0`. (This is why `KEY_SEP` exists — see commit fixing dotted-key splitting.)
- Only object roots are editable; a non-object `settings.json` yields no `edit_entries` ("No editable keys found").

## How to Extend

- Add a new file operation: add an `InputMode` variant, a `handle_*` method in `mod.rs`, an `execute_*` helper, and action/status-bar arms in `ui.rs`.
- Change value-type inference: edit the parse cascade in `save_edit()` (`state.rs`).

## Dependencies

- dirs 5.0 — resolve home directory (`~/.claude`)
- serde_json 1.0 — pretty-print preview, flatten/edit/serialize settings
