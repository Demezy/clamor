# Clamor UX Improvements (inspired by lazygit)

## Navigation & Discoverability

? help popup — context-aware keybindings overlay
  Shows all available keys for the current mode (dashboard vs terminal vs kill-pending).
  Lazygit's strongest UX feature — users discover bindings organically.

/ filter mode — filter agent list by typing
  Useful when you have 10+ agents. Substring match on agent title/folder.
  Press / to enter filter, type to narrow, Esc to clear.

g/G jump to first/last agent
  Standard vim top/bottom navigation. Tiny addition, nice polish.

## Visual Feedback

Spinner for agent status — animate "work" state
  Replace static "work" text with a rotating spinner (⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏).
  Makes it obvious which agents are actively processing vs stuck.

Selection indicator — left border marker instead of just background tint
  Add ▎ or ▌ character on the left edge of the selected row.
  More visible than subtle background color, especially on varied terminal themes.

Active panel border color — differentiate focused vs unfocused
  In terminal mode, make the title bar brighter when focused.
  Already partially done, but could be more pronounced.

## Layout & Views

Split preview mode — see agent output without fully attaching
  Half the screen shows agent list, other half shows live terminal output
  of the selected agent. Navigate with J/K to preview different agents.
  Enter to go full-screen. Like lazygit's diff preview panel.

Scroll margin for cursor navigation
  When moving J/K through the agent list, keep a 2-line margin from
  viewport edges. Prevents the selected item from touching the top/bottom
  edge — you always see context above and below.

## Agent Management

Batch operations via range select
  Press v to start range selection, J/K to extend, then X to kill all
  selected agents at once. Only contiguous ranges (lazygit's design choice).

Quick restart — kill + respawn in one action
  Select an agent, press r to kill it and immediately respawn with the
  same folder + prompt. Useful for retrying failed tasks.

## Input & Editing

Inline prompt history — cycle through previous prompts
  When in the create prompt popup, Up/Down cycles through previously
  used titles/descriptions. Like shell history for agent prompts.

Word-level cursor movement in text fields
  Alt+Left/Right to jump by word in title/description fields.
  Currently only supports character-level backspace and delete-word.

## Terminal Mode

Scroll position indicator
  Show "line X/Y" or a percentage when scrolled up in terminal mode.
  Currently no indication of where you are in the scrollback.

Copy mode with keyboard
  Enter a copy mode (like tmux) where you can move a cursor with
  vim keys and select text without mouse. Useful for SSH sessions
  where mouse might not work well.

## Status & Monitoring

Agent activity timeline — show last tool used + time
  Already have last_tool in state. Could show it in the dashboard
  as a subtle third line or tooltip: "last: Read file.rs (2m ago)"

Notification on state change
  Terminal bell or visual flash when an agent transitions to "input"
  state (waiting for user). Helps when you're working on something
  else and need to know when an agent needs attention.

Desktop notification integration
  Send macOS notification when agent finishes or needs input.
  Especially useful when clamor is in a background terminal tab.
