# Excalibur CLI

A unified TUI command-line interface built in Rust with ratatui, integrating multiple tools via a module system.

## Build & Run

```bash
cd excalibur
cargo build --release
cargo run                    # main menu
cargo run -- history         # direct module entry
cargo run -- process-tracer  # direct module entry
cargo run -- ssh             # direct module entry
```

## Architecture

```
excalibur/
├── src/
│   ├── main.rs          # CLI entry (clap), terminal setup
│   ├── app.rs           # Event loop, key dispatch, ModuleAction handling
│   ├── event.rs         # Background event thread (Tick/Crossterm/AppEvent)
│   ├── view.rs          # View enum (MainMenu / Module)
│   ├── ui.rs            # Main menu rendering
│   └── modules/
│       ├── mod.rs       # Module trait, ModuleId, ModuleAction
│       ├── manager.rs   # ModuleManager (registry, routing)
│       ├── history/     # Fish shell history browser
│       ├── proctrace/   # Process tracer/analyzer (Linux-only, cfg-gated)
│       └── ssh/         # Tunnel dashboard + ssh config editor
└── install/             # Fish shell integration (ex.fish + exh alias)
```

`install/ex.fish` defines `ex <module>` and is the single owner of the exit-code
protocol (exit 0 → insert into the command line, exit 10 → insert and execute).
`exh` (bound to Ctrl+R) is a thin alias over it, so a new module needs no new
fish function.

`proctrace` is gated behind `#[cfg(target_os = "linux")]` — the module, its `ModuleId` variant, manager registration, and the `process-tracer` subcommand all compile out off Linux. `ssh` runs on Linux **and macOS**: `supervisor::scan`/`usage` and `probe::listening` each have two implementations, and the macOS pair is compiled on Linux too (under `cfg(any(…, test))`) so it cannot rot unnoticed on a machine nobody builds on. The only capability that does not carry over is the per-tunnel traffic rate. See `.claude/rules/ssh.md` for what differs and why.

## Modules

| Module | Rules file | Description |
|--------|-----------|-------------|
| core | `.claude/rules/core.md` | App framework: event loop, module system, main menu |
| history | `.claude/rules/history.md` | Fish shell history browser with search, sort, clipboard |
| proctrace | `.claude/rules/proctrace.md` | Query-driven process inspector (name/PID/port), Linux-only; emits kill/journalctl/systemctl/cd |
| ssh | `.claude/rules/ssh.md` | Port-forward dashboard (three connectivity layers) + structured `~/.ssh/config` editor |

## Design docs

`.claude/rules/*.md` is the map — what the files are and what bites you. The
reasoning behind a design lives separately:

| File | Contents |
|------|----------|
| `docs/README.md` | Project positioning, the absorption criteria, the quantitative basis, the roadmap |
| `docs/ssh.md` | SSH module: design rationale, what shipped, the backlog, and what was deliberately cut |

## Adding a New Module

1. Add variant to `ModuleId` in `excalibur/src/modules/mod.rs`
2. Create `excalibur/src/modules/<name>/` with `mod.rs` implementing the `Module` trait
3. Register in `ModuleManager::new()` in `excalibur/src/modules/manager.rs` — insert into `modules` **and** push onto `order`, which is what the main menu iterates
4. Add CLI subcommand in `main.rs` `Commands` enum
5. Create `.claude/rules/<module>.md` with `paths:` frontmatter listing all related source files
6. Add an entry to the Modules table above
