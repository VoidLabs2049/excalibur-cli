---
paths:
  - "excalibur/src/main.rs"
  - "excalibur/src/app.rs"
  - "excalibur/src/event.rs"
  - "excalibur/src/view.rs"
  - "excalibur/src/ui.rs"
  - "excalibur/src/modules/mod.rs"
  - "excalibur/src/modules/manager.rs"
---

# Core

Application framework: TUI shell, event loop, module system, and main menu rendering.

## Key Files

| File | Role |
|------|------|
| `excalibur/src/main.rs` | CLI entry point (clap), terminal setup, optional direct module entry via subcommands (`history`/`h`, `process-tracer`/`pt`, `ssh`/`t`) |
| `excalibur/src/app.rs` | `App` struct — main event loop, key dispatch, `ModuleAction` handling (Output/OutputAndExecute exit codes) |
| `excalibur/src/event.rs` | `EventHandler` — mpsc channel, background thread emitting `Tick` (30 FPS) and `Crossterm` events, plus `AppEvent` queue |
| `excalibur/src/view.rs` | `View` enum: `MainMenu` or `Module(ModuleId)` |
| `excalibur/src/ui.rs` | `Widget` impl for `&App` — renders main menu (module list with shortcuts) or delegates to active module |
| `excalibur/src/modules/mod.rs` | `Module` trait, `ModuleId` enum, `ModuleAction` enum, `ModuleMetadata` struct |
| `excalibur/src/modules/manager.rs` | `ModuleManager` — registers modules, routes activate/deactivate/key/update/render to active module |

## Architecture / Data Flow

```
main.rs (clap parse → terminal setup)
  └─ App::new() / App::new_with_module()
       └─ Event Loop:
            EventHandler (background thread)
              → Tick / Crossterm / AppEvent
            App::handle_events()
              → MainMenu keys → AppEvent::EnterModule
              → Module keys → ModuleAction → AppEvent::ModuleAction
            ModuleManager
              → activate(id) → module.init()
              → handle_key_event() → ModuleAction
              → update() (on tick)
              → render() (delegated)
```

## Design Patterns

- **Module trait**: All modules implement `Module` (metadata, init, handle_key_event, update, render, cleanup)
- **Event channel**: mpsc sender/receiver decouples input thread from app logic; `AppEvent` allows deferred actions
- **Exit codes for shell integration**: `ModuleAction::Output` → exit(0), `OutputAndExecute` → exit(10); stdout carries the command, TUI uses `/dev/tty`. The fish side of this protocol lives in **one** place — `install/ex.fish` (`ex <module>`); `exh` is a thin alias over it, so a new module needs no new fish function.
- **Platform gating**: the `proctrace` module is `#[cfg(target_os = "linux")]`-only — the `ModuleId::ProcessTracer` variant, its `manager.rs` registration (module map *and* `order`), and the `main.rs` subcommand all compile out off Linux

## How to Extend

1. Add variant to `ModuleId` in `modules/mod.rs`
2. Implement `Module` trait for your new module
3. Register it in `ModuleManager::new()` in `manager.rs` — push onto `order` too, or the module never shows up in the menu
4. Add CLI subcommand in `main.rs` `Commands` enum and map to `ModuleId`

No fish work is needed — `ex <subcommand>` picks up the new module automatically.

## Testing

- **No tests for the framework itself.** `cargo test` (from `excalibur/`) reports 169 — all of them live in the `ssh` module; see [ssh.md](ssh.md) for the rules its suite runs under.

## Dependencies

- ratatui 0.29 — TUI framework
- crossterm 0.28 — terminal backend
- clap 4.5 — CLI argument parsing
- color-eyre 0.6 — error handling
