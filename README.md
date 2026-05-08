<p align="center">
  <img src="banner.png" alt="Clamor" width="100%">
</p>

<h3 align="center"><i>ORCHESTRATE THE NOISE</i></h3>

<p align="center"> Tmux for agents. Because one isn't loud enough. </p>

<p align="center">A terminal multiplexer for managing parallel coding agent sessions.</p>

<!-- demo gif placeholder -->

## Why

You're running five coding agent sessions in parallel. Tab names blur together. You can't tell which agent needs input and which is still working. Switching between them means hunting through tmux windows or terminal tabs. Spawning a new one is a ritual of `cd`, naming, and arranging.

Clamor fixes this without replacing your workflow. It's *not* a new terminal, *not* a new editor, *not* an IDE that swallows everything. It's a single tool that does one thing well: manage parallel agent sessions. It runs inside your existing terminal, alongside tmux, inside whatever setup you already have. Unix philosophy — small, composable, stays out of the way.

## Features

**Persistent sessions** — Agents live in a background daemon, not your terminal. Close the dashboard, reopen it — everything's still running. Terminal crash? SSH disconnect? Doesn't matter.

**Jump keys** — Each agent gets a home-row key (`a`/`s`/`d`/`f`/`j`/`k`/`l`/`h`, overflow to `1`–`0`). One keypress to attach, `Ctrl+F` to detach. Switching between agents is instant.

**Live state tracking** — The dashboard shows each agent's actual state (working/waiting/done). Spot stalled agents immediately.

**Color-coded title bar** — When attached, a title bar shows the project name and a distinct color so you always know where you are. No squinting at tab names.

**Auto-resume** — The daemon automatically resumes agent sessions on restart. Terminal crash, SSH disconnect, daemon restart — agents pick up where they left off.

**Session adoption** — Already have agent sessions from before? Press `R` in the dashboard to adopt an existing session into Clamor.

**Batch operations** — Select multiple agents with `v`, select all with `V`, then act on the selection. Filter agents by name with `/`.

**Non-blocking hooks** — State tracking uses non-blocking file locks. Clamor never slows down your agent sessions.

## Quick start

### Install

```bash
cargo install clamor
```

Or with Nix (flakes enabled):

```bash
nix profile install github:amenocturne/clamor
```

Run without installing: `nix run github:amenocturne/clamor`. Hack on it: `nix develop`.

Or build from source:

```bash
git clone https://github.com/amenocturne/clamor.git
cd clamor
cargo build --release
cp target/release/clamor ~/.local/bin/
```

### Hook setup

Clamor tracks agent state (working/waiting/done) through a generic CLI primitive:

```
clamor set-state <working|input|done> --agent "$CLAMOR_AGENT_ID" [--tool LABEL] [--session-token TOKEN] [--activity-only]
```

Each backend wires its own hook system to invoke this command. `CLAMOR_AGENT_ID` is set automatically in the spawned agent's environment; pass it through explicitly — clamor does not read it from the environment.

For Claude Code, copy `hooks/hooks.json` from this repo into `~/.claude/settings.json` (or merge the `hooks` section). The template uses `jq` to extract `session_id` from the hook payload, so install jq if you want resume-token tracking.

Want to add a new backend? Declare its spawn/resume commands in `config.yaml` and wire its hooks (or equivalent) to call `clamor set-state`. No Rust changes needed.

### Configure folders

```bash
clamor config init    # Write an empty starter config
clamor config         # Open config in $EDITOR
```

Config lives at `~/.config/clamor/config.yaml`. Legacy `~/.clamor/config.json` is auto-detected — run `clamor config migrate` to convert. Clamor ships no built-in backends; you must declare at least one before it can spawn anything. Run `clamor config print-example` for a ready-to-edit template.

#### Folders

Each folder is a project directory where agents can be spawned:

```yaml
folders:
  my-app:
    path: ~/projects/my-app
    backends: [claude-code, open-code]  # Which backends are available here
```

Every folder must list at least one backend id that is declared under `backends:`. When a folder lists multiple backends, the spawn prompt shows a backend selector you can cycle through with arrow keys. The selection is persisted per folder.

#### Backends

Backends define how to spawn and resume agent sessions. Clamor has no knowledge of specific harnesses — declare whatever you use:

```yaml
backends:
  claude-code:
    display_name: Claude
    spawn:
      cmd: [claude, "{{prompt}}"]
      env:                  # Optional extra environment variables
        CLAUDE_CODE_MAX_TURNS: "50"
    resume:
      cmd: [claude, --resume, "{{resume_token}}"]
    capabilities:
      resume: true          # Can resume sessions after daemon restart
      sync_output_mode: true  # Uses synchronized output (BSU/ESU)

  my-custom-agent:          # Define your own
    display_name: Custom
    spawn:
      cmd: [my-agent, --prompt, "{{prompt}}"]
```

**Template variables** available in `cmd`:

| Variable | Description |
| --- | --- |
| `{{prompt}}` | User's description text (optional — omitted if empty) |
| `{{title}}` | Agent title |
| `{{folder_id}}` | Folder ID from config |
| `{{folder_path}}` | Expanded folder path |
| `{{cwd}}` | Working directory |
| `{{backend_id}}` | Backend ID |
| `{{resume_token}}` | Session ID for resume (resume commands only) |

**Capabilities**:

| Capability | Default | Description |
| --- | --- | --- |
| `resume` | `false` | Agent sessions can be resumed after daemon restart |
| `sync_output_mode` | `false` | Backend uses synchronized output markers |

Use `clamor config print-example` to see a full config.

### Launch

```bash
clamor
```

## Usage

```
clamor                  Open the dashboard (starts daemon if needed)
clamor ls               List all agents
clamor new <title>      Spawn a new agent
clamor attach <ref>     Attach to an agent (by ID or jump key)
clamor adopt <id>       Adopt an existing agent session
clamor edit <ref>       Update agent description
clamor kill <ref>       Terminate an agent
clamor kill --all       Terminate all agents
clamor clean            Remove finished agents
clamor config           Open config in $EDITOR
clamor config init      Write a starter XDG config
clamor config migrate   Copy legacy JSON config to XDG YAML
clamor config print-example
                        Print a full example config
clamor stop             Stop the daemon
```

### Dashboard keys

| Key            | Action                           |
| -------------- | -------------------------------- |
| `a`–`h`, `1`–`0` | Jump to agent                 |
| `J` / `K`      | Navigate agent list (vim-style)  |
| `g` / `G`      | Jump to first / last agent       |
| `Enter`        | Attach to selected agent         |
| `c`            | Create agent (inline prompt)     |
| `C`            | Create agent ($EDITOR prompt)    |
| `x` + key      | Kill agent (with confirmation)   |
| `e` + key      | Edit agent description           |
| `v`            | Toggle select agent              |
| `V`            | Select / deselect all            |
| `/`            | Filter agents by name            |
| `R`            | Adopt existing agent session     |
| `?`            | Help popup                       |
| `Esc`          | Clear selection                  |
| `q`            | Quit dashboard                   |

### Spawn prompt keys

| Key              | Action                                |
| ---------------- | ------------------------------------- |
| `Tab` / `Shift+Tab` | Cycle fields (title → description → backend) |
| `←` / `→`       | Select backend (when backend field active) |
| `Shift+Enter`    | New line in description               |
| `↑` / `↓`       | Prompt history                        |
| `Ctrl+W`         | Delete word                           |
| `Ctrl+U`         | Delete line                           |
| `Enter`          | Spawn agent (empty = interactive)     |

### Terminal keys

| Key       | Action                                   |
| --------- | ---------------------------------------- |
| `Ctrl+F`  | Detach (back to dashboard)               |
| `Ctrl+C`  | Send SIGINT to agent                     |
| `Ctrl+G`  | Jump to next agent waiting for input     |
| `Ctrl+Shift+G` | Jump to previous agent waiting for input |
| `Ctrl+J`  | Snap to bottom (live view)               |
| `Ctrl+R`  | Refresh terminal (rebuild from buffer)   |
| `Ctrl+S`  | Enter copy mode                          |

All terminal keys are user-configurable. Any combo not listed below is
forwarded directly to the agent — so wrapped tools like Claude Code receive
their own `Ctrl+R`, `Ctrl+S`, etc. as expected.

#### Customizing terminal keys

Add a `terminal:` section to `~/.config/clamor/config.yaml`. Listing
`bindings:` opts you out of the defaults entirely — list every chord you
want clamor to intercept; everything else passes through to the agent.

```yaml
terminal:
  bindings:
    ctrl-f: detach          # required — without this you cannot escape the agent
    ctrl-c: sigint
    ctrl-g: jump-input-next
    ctrl-shift-g: jump-input-prev
    # ctrl-j: snap-to-bottom    # commented out → Ctrl+J reaches Claude Code as a newline
    # ctrl-s: enter-copy-mode   # commented out → Ctrl+S reaches Claude Code's search
    # ctrl-r: refresh-parser    # commented out → Ctrl+R reaches Claude Code's verbose toggle
    alt-r: refresh-parser   # remap: refresh now lives on Alt+R
```

Available actions: `detach`, `sigint`, `jump-input-next`, `jump-input-prev`,
`snap-to-bottom`, `enter-copy-mode`, `refresh-parser`. Chord syntax is
`[ctrl-][shift-][alt-]<letter>` (modifier order does not matter; uppercase
letters are equivalent to `shift-<lowercase>`). Clamor refuses to start if
the resolved bindings do not include `detach`.

## Architecture

Clamor uses a daemon-client architecture, similar to tmux:

```
┌──────────────┐    Unix socket    ┌────────────────────────┐
│              │◄─────────────────►│         Daemon         │
│   Dashboard  │  length-prefixed  │                        │
│  (TUI client)│       JSON        │  ┌───────┐ ┌───────┐   │
│              │                   │  │  PTY  │ │  PTY  │   │
└──────────────┘                   │  │ agent │ │ agent │…  │
                                   │  └───────┘ └───────┘   │
                                   └────────────────────────┘
```

- The **daemon** runs in the background, owns all agent PTY processes, and persists across dashboard restarts
- The **dashboard** connects over a Unix socket (`~/.clamor/clamor.sock`), subscribes to output streams, and renders the TUI
- **State** is tracked in `~/.clamor/state.json` with file-locked reads/writes, updated in real time via `clamor set-state` calls from harness hooks

## Troubleshooting

**Agent terminal looks garbled after attaching** — Double `Ctrl+F` fixes it. The first detaches to the dashboard, the second re-attaches, resetting the terminal state.

**Daemon won't start** — Check if a stale socket exists: `rm ~/.clamor/clamor.sock` and try again.

**Hooks not updating state** — Verify that `clamor` is in your `PATH` and the hook entries are in your `.claude/settings.json`.

## License

[MIT](LICENSE)
