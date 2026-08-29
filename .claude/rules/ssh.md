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
  - "excalibur/src/modules/ssh/discover.rs"
---

# SSH

Port-forward dashboard plus a structured editor for `~/.ssh/config`. Declare the
tunnels you use, start them, and see three separate layers of connectivity.

Design rationale, the quantitative basis for the field choices, the backlog, and
what was deliberately cut: `docs/ssh.md`. This file is the map; that one is the
argument.

CLI: `cargo run -- ssh`, alias `t`.

## Key Files

| File | Role |
|------|------|
| `mod.rs` | `SshModule` — implements `Module`, routes keys per `Screen` |
| `state.rs` | `SshState`, `Screen`, the landing `MENU`; polling timers, start/stop, health cache |
| `ui.rs` | Renders all five screens |
| `sshconfig.rs` | Parses `~/.ssh/config` into `HostBlock { patterns, start, end, directives, shadowed_by }` |
| `effective.rs` | `resolve()` diffs a block against `ssh -G`; `check()` gates every save |
| `form.rs` | Six-field `HostForm`/`ForwardForm`, `BlockEdit` raw-text; `plan()` → line diff; `write_config()` |
| `tunnels.rs` | `Tunnels`/`Profile`/`Forward` serde over `~/.config/excalibur/tunnels.yaml`; rule validation |
| `supervisor.rs` | `parse_argv` / `scan` / `find` / `start` / `stop` |
| `probe.rs` | `Health` — the three lights; the listen check (`/proc/net/tcp` or `netstat`) and end-to-end connect |
| `worker.rs` | `Job`/`Outcome` channels so blocking work stays off the render thread |
| `discover.rs` | `ss -tlnH` / GNU `netstat -tln` / BSD `netstat -an -p tcp` over ssh, parsed into `Listener`s |

## Screens

`Screen::Menu` (landing, preview pane) → `Config` (host list + form) /
`Forward` (tunnel profiles + form) / `Dashboard` (live tunnels).
`Discover` (a host's listening ports) hangs off `Config` via `d`, not off the menu.
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

## Platforms

Linux and macOS. Three functions have two implementations; everything else —
`parse_argv`, `find`, `start`, `stop`, the end-to-end probe, and every screen —
is platform-independent.

| | Linux | macOS |
|---|---|---|
| `supervisor::scan` | `procfs` | `scan_portable`, via `sysinfo` |
| `supervisor::usage` | uptime + `rchar` | uptime only, `read: None` |
| `probe::listening` | `/proc/net/tcp` | `netstat -an -p tcp` |

- **`scan()` keeps a separate Linux implementation for speed, not portability.**
  It runs once a second on the render thread; over ~650 processes `procfs` costs
  ~5ms and `sysinfo` 45–85ms. The slow one would drop frames every second on the
  one screen whose value is that it stays live. Do not "simplify" the two into
  one without re-measuring.
- **The macOS halves are compiled on Linux too** — `scan_portable` and
  `listens_in_netstat` are `#[cfg(any(<their platform>, test))]`, and
  `listens_on` is cfg'd the same way in reverse. Without that, a change here
  first fails to build on a Mac, which is the worst place to find out.
  `the_portable_scanner_agrees_with_the_one_this_platform_uses` then runs both
  scanners and compares; when a tunnel is up that is a real cross-check, and
  when none is it still proves the thing builds and does not panic.
- **The traffic rate is Linux-only, and must read as absent rather than zero.**
  `Usage::read` is `Option<u64>` for that reason: a stand-in `0` would come back
  out of the delta as `Some(0.0)` and print `0B/s`, which is the reading for a
  live tunnel nobody is using. Uptime is drawn before the rate, so the column
  macOS keeps is the one that survives a narrow terminal.
- **`Tunnels::path()` is `$XDG_CONFIG_HOME`-or-`~/.config`, not
  `dirs::config_dir()`.** The latter is `~/Library/Application Support` on
  macOS, which would put the rules somewhere other than every doc says.

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
- **`scan()` filters to our own uid.** argv is world-readable, so without that
  another user's tunnel is listed as unclaimed and offered for a `kill` that can
  only fail. Both implementations have to keep doing it.
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
- **`usage()` reads `rchar`, not `rchar + wchar`.** A forwarder reads each
  payload byte once and writes it once, so `rchar` alone already covers the
  traffic; adding `wchar` counts the same bytes twice and doubles every rate.
  There is no macOS equivalent that counts socket traffic — `proc_pid_rusage`
  counts disk — so the field is `Option` and stays `None` there.
- **`sample_meters()` may only read pids that came out of `scan()`.** Identity
  by argv first, measurement second — otherwise the numbers describe a process
  that was never the one meant, and nothing about them looks wrong.
- **A discovery answer must be matched to the host still on screen.** An ssh
  round trip can outlive the screen that asked; `apply_discovery` drops any
  answer whose host is not the open one. The remote command is wrapped in
  `sh -c '...'` because the login shell over there is fish, where a bare
  `a || b` is a standing trap (`~/.claude/remote-ops.md`).
- **`discover::parse` must filter BSD rows by the state column.** The macOS
  fallback is `netstat -an -p tcp`, which lists *every* connection rather than
  only listeners; without the check, the far port of each outbound connection is
  offered as a port to forward. Its addresses are also dotted (`::1.6022`), so
  `split_endpoint` tries `:` first and falls back to `.` only when the port
  fails to parse. Change the remote command and this parser together — a
  mismatch returns an empty list, not an error.
- **`exits_on(host)` filters by host.** kami's 8080 and thor's 8080 are
  different services; a bare port match reports one as already forwarded when
  nothing forwards it.
- **`HostForm::creating` switches `plan()` to append a whole block.**
  `host_index` then points one past the end and is only good for excluding
  "self" from candidate lists. A new alias that duplicates an existing one is
  refused at save: OpenSSH takes the first match, so the block would be written,
  appear in the list, and do nothing.
- **`BlockEdit` holds its own `start`/`end`.** The config is reloaded after any
  save, and a range re-derived afterwards would overwrite different lines.
  While it is open every key belongs to the text — the host-list bindings must
  not fire underneath it.
- **A rate of `None` is not a rate of zero.** `None` is "no second sample yet";
  `Some(0.0)` is a live tunnel carrying nothing, which is the interesting one.
- **`effective::check` runs before every config write** and returns three
  states, not a bool. `Skipped` still saves — a `Match exec` config cannot be
  validated (running it would execute a shell command on every save) and must
  not therefore become unsavable — so the caller has to *say* it was unchecked.
  Its staged copy goes beside the real config and inherits its mode: ssh
  refuses a config file it considers too open, and that refusal would surface
  here as a bogus syntax error.
- **`pristine` is captured on the first successful load only.** `load_config()`
  runs again after every save; re-snapshotting there leaves `U` looking
  available and doing nothing. `undo_config` deliberately skips
  `effective::check` — restoring what was already on disk cannot be worse than
  what is there now, and a config broken before the session started would
  otherwise be unreachable.

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

175 tests, `cargo test` from `excalibur/`. Parser tests use in-memory fixtures
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
