---
paths:
  - "excalibur/src/modules/ssh/mod.rs"
  - "excalibur/src/modules/ssh/state.rs"
  - "excalibur/src/modules/ssh/ui.rs"
  - "excalibur/src/modules/ssh/sshconfig.rs"
  - "excalibur/src/modules/ssh/effective.rs"
  - "excalibur/src/modules/ssh/form.rs"
  - "excalibur/src/modules/ssh/tunnels.rs"
  - "excalibur/src/modules/ssh/supervisor.rs"
  - "excalibur/src/modules/ssh/probe.rs"
  - "excalibur/src/modules/ssh/worker.rs"
---

# SSH

Port-forward dashboard plus a structured editor for `~/.ssh/config`. Declare the
tunnels you use, start them, and see three separate layers of connectivity.

Design rationale, the quantitative basis for the field choices, the backlog, and
what was deliberately cut: `docs/ssh.md`. This file is the map; that one is the
argument.

CLI: `cargo run -- ssh`, alias `t` (`s` was taken by settings).

## Key Files

| File | Role |
|------|------|
| `mod.rs` | `SshModule` — implements `Module`, routes keys per `Screen` |
| `state.rs` | `SshState`, `Screen`, the landing `MENU`; polling timers, start/stop, health cache |
| `ui.rs` | Renders all four screens |
| `sshconfig.rs` | Parses `~/.ssh/config` into `HostBlock { patterns, start, end, directives, shadowed_by }` |
| `effective.rs` | Runs `ssh -G <alias>` and diffs it against what the block says |
| `form.rs` | Six-field `HostForm` and `ForwardForm`; `plan()` → line diff; `write_config()` |
| `tunnels.rs` | `Tunnels`/`Profile`/`Forward` serde over `~/.config/excalibur/tunnels.yaml`; rule validation |
| `supervisor.rs` | `parse_argv` / `scan` / `find` / `start` / `stop` |
| `probe.rs` | `Health` — the three lights; `/proc/net/tcp` listen check and end-to-end connect |
| `worker.rs` | `Job`/`Outcome` channels so blocking work stays off the render thread |

## Screens

`Screen::Menu` (landing, preview pane) → `Config` (host list + form) /
`Forward` (tunnel profiles + form) / `Dashboard` (live tunnels).
The cursor starts on `Dashboard`, not the top entry — it is the high-frequency path.

## The three lights

`Health { process, port, path }`, rendered as three symbols (`o` off, `*` ok,
`x` bad, `-` not observable). They are separate because the failures live
*between* them:

| Lights | Means |
|---|---|
| `* x -` | process alive, port never bound — something else holds it |
| `* * x` | tunnel is up but the far-side service is down |

The third layer works because `-L` is lazy: ssh only dials the remote after a
local `accept`, so a failed remote closes the connection immediately.

## Gotchas

These are the ones where getting it wrong produces **no error**, just a
different outcome.

- **`BatchMode=yes` is not optional.** Without it ssh prompts for a passphrase
  when the agent has no key; the TUI owns the terminal, so the prompt is
  invisible and the UI simply hangs. Same for `ExitOnForwardFailure=yes` —
  without it a taken port leaves a live process with no forward.
- **Claim by argv structure, never by substring.** `parse_argv` walks ssh's
  option grammar (`TAKES_VALUE` letters consume the next word) so the host is
  the first bare word left. A `pkill -f`-style match would also hit the process
  doing the matching — see `~/.claude/remote-ops.md`. `stop()` kills by pid for
  the same reason.
- **The form must only rewrite the lines its six fields own.** `plan()` replaces
  values in place, preserving indent, key spelling (`Hostname` vs `HostName`),
  and separator. Rewriting the whole block would silently drop the directives
  the form does not model — one host here has five of them.
- **Only the selected host's `[start, end)` range is written**, which is why no
  lossless CST is needed: every other byte is untouched by construction.
- **`write_config` carries the original mode over.** ssh refuses configs it
  considers too open, and a fresh file would take the umask default instead.
- **`ssh -G` needs a trailing command argument.** `effective::resolve` passes
  `true` to suppress "Pseudo-terminal will not be allocated", which would
  otherwise land in stderr.
- **`-R`'s second light is always `-`.** The port opens on the far side and is
  not observable from here; what is checked instead is the exit — whether the
  service being exposed is alive locally.
- **`probe::listening` matches by port only**, ignoring the bind address.
- **`supervisor::scan()` returns `Vec::new()` off Linux** (it reads `/proc`), so
  the dashboard shows everything as stopped rather than failing to build. Unlike
  proctrace, the module itself is *not* cfg-gated. It also filters to our own
  uid — `/proc/<pid>/cmdline` is world-readable, so without that another user's
  tunnel is listed as unclaimed and offered for a `kill` that can only fail.
- **`Slot` is `(profile_index, forward_index)`** — a position, not an identity.
  Anything cached on it must be dropped when the file reloads.
- **The dashboard cursor runs past the last rule.** `forward_index` indexes
  rules on the forward screen and rules-then-orphans on the dashboard, which is
  what `state::cursor_span()` decides. Use `goto()` to change screen: a cursor
  parked on an orphan row selects nothing on the forward screen, and `n`/`c`/`d`
  then do nothing at all. `selected_slot()` returning `None` is how the mark and
  start keys stay off orphans — that is deliberate, not an oversight.
- **Orphans are `scan()` minus `find()`, never a pattern.** Anything broad
  enough to claim the rest also claims the process doing the claiming.
- **The flow diagram uses `│` only.** `▼`/`→` are East-Asian-ambiguous width;
  a terminal resolving them wide shifts every following line one column.
  `Forward::flow()` decides who listens and who resolves the exit;
  `ui::flow_lines()` only draws.

## Data flow

```
init() → SshConfig::load()  (never cached across entries — edited outside too)
       → Tunnels::load()    (a missing file is an empty set, not an error)

update() → poll_tunnels()
             every 1s   supervisor::scan() → Vec<Running>, drop health for dead slots
             every 10s  worker.submit(Job::Probe)  → Outcome::Probed → health cache
             drain()    Started/Stopped outcomes reset the timers so the UI
                        reflects the change immediately

Enter on dashboard → forward.problem()?  → notify and refuse
                     else worker.submit(Job::Start | Job::Stop)
```

## How to extend

- New form field: add the variant plus `Field::ALL` in `form.rs`, and either give
  it a `keyword()` (the directive it writes one-to-one) or handle it explicitly
  in `plan()` the way `Alias` and `Gateway` are, since neither maps to a single
  keyword.
- New probe layer: extend `Health` and `probe::check_*`; keep it in the worker.
- New tunnel action: add a `Job`/`Outcome` pair in `worker.rs` rather than
  blocking in `state.rs`.

## Testing

137 tests, `cargo test` from `excalibur/`. Parser tests use in-memory fixtures
(`SshConfig::parse`), UI tests render into a `Buffer` and assert on the text —
including a narrow-terminal case, because the right-flushed note is the part
that must survive truncation.

The suite runs against the real machine, so three things must stay true of any
new test — each of them fails by *doing* something, not by erroring:

- **Never call a method that saves.** `save_forward_form`/`delete_forward` write
  the user's real `~/.config/excalibur/tunnels.yaml`. The in-memory halves
  (`apply_forward_form`, `remove_selected_forward`) exist for tests.
- **Never leave a startable rule in a scope you then start.** `start_slots`
  spawns real `ssh`. Give the rule a `running` entry so it counts as already up.
- **Any pid a test may stop must not exist.** `NO_SUCH_PID` in `mod.rs` sits
  above Linux's `pid_max` ceiling of 2^22 for exactly this.
