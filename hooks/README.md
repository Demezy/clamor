# Clamor Hook (Claude Code reference)

Reports agent state changes to the clamor daemon by invoking the generic `clamor set-state` primitive from each Claude Code hook event.

Triggered on: `UserPromptSubmit`, `Notification`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `Stop`.

`Stop` maps to `input` — semantically, Claude finishing a turn means it's waiting for the next user message, not truly done. The `done` state is reserved for real process exit, which clamor's daemon detects directly from the PTY.

`SessionStart` is intentionally not wired: clamor already sets `input` after resume/adopt, and a `working` hook firing immediately after would race and win. Letting the agent sit at the prompt until `UserPromptSubmit` is the correct signal.

Each hook runs a one-liner that extracts the relevant payload fields (via `jq`) and calls `clamor set-state <state> --agent "$CLAMOR_AGENT_ID" [flags]`. `CLAMOR_AGENT_ID` is injected into the agent's environment by clamor at spawn time. `jq` must be on `$PATH` for session-token and tool-label extraction.

Configure by merging `hooks.json` into `.claude/settings.json`. Exits silently if clamor isn't installed.

This file is a minimal reference. Harnesses that spawn parallel subagents (e.g. Claude Code's `Task` tool) need extra counter logic so `Stop` doesn't fire `input` while subagents are still running — see `agentic-kit/hooks/clamor/` for such a wrapper.
