# Clamor UX Issues Found by Review

## Silent failures (high impact)

Selection index not re-clamped after filter changes
  User navigates to index 5, applies filter that shows 2 agents.
  selected_index stays at 5. Press Enter -> nothing happens silently.
  Fix: re-clamp selected_index whenever filter_query changes.

Empty agent list with stale selection -> Enter silently fails
  User kills last agent, selected_index is still Some(0).
  Press Enter -> ordered_agent_ids returns empty, get() returns None, no-op.
  Fix: auto-clear selected_index when list becomes empty.

Filter clears selection, no auto-select on single result
  User filters to 1 agent, presses Enter to accept filter.
  selected_index is None. Must press J then Enter (2 extra steps).
  Fix: auto-select first agent when filter narrows to results.

Empty title in prompt popup -> no error feedback
  User presses Enter with empty title. Code silently switches back
  to Title field with no message. User thinks app froze.
  Fix: show brief red "Title required" text in the popup.

Empty edit description -> silently rejected
  User edits agent, clears title, presses Enter. Silently ignored.
  Fix: show error feedback, same as above.

Attaching to Lost agent -> silent no-op
  User presses jump key or Enter on a Lost agent. Nothing happens.
  No message explaining why. User thinks key is broken.
  Fix: show message like "Agent lost (no session to resume)".

Ctrl+F re-attach to Lost agent -> silent fail
  Last attached agent became Lost. Ctrl+F tries to switch but
  the code only checks contains_key, not the Lost state.
  Fix: also check agent.state != Lost before re-attaching.

## UX friction (medium impact)

No visual indicator that filter is active
  After filtering with / and pressing Enter, the agent list is
  filtered but there's no visible badge or marker showing that
  a filter is applied. User thinks agents disappeared.
  Fix: show "Filter: <query>" in header or above the agent list.

Adopt goes to first folder without choice
  When multiple folders configured, R -> type session -> Enter
  always adopts into sorted_folders[0]. No folder picker shown.
  Fix: show folder picker before or after session ID input.

Can't navigate J/K while filter input is open
  While typing a filter query, J/K are consumed as text input.
  User can't preview which agents match while typing.
  Fix: this is actually correct behavior for a text field.
  Alternative: show filtered results live as user types (already done).

Selection indicator not accounted in width calculation
  The left border marker adds 1 char but render_agent_line doesn't
  know about it. On narrow terminals, selected rows may overflow.
  Fix: reduce desc_width by 1 when selected, or always reserve the space.

Kill gives no visual confirmation
  User presses X + agent key. Agent row shows "killed" state with
  3-second linger, which IS visual feedback. But if daemon is down,
  kill silently fails and user sees nothing change.
  Fix: check kill result and show error if it fails.

Empty description auto-spawns interactive without confirm
  User types title, leaves description empty, presses Enter.
  Spawns bare `claude` (interactive) immediately. No confirmation.
  Fix: show ConfirmEmptySpawn dialog when description is empty.

## Edge cases (lower impact)

History stash not initialized on prompt open
  If user presses Up immediately (before typing anything),
  history_stash is set to empty strings. Then pressing Down
  restores empty state. Works correctly but edge-case-y.

Folder path comparison is string-based in draft restoration
  Draft uses exact path match. ~/projects != /Users/x/projects.
  Could fail if paths are resolved differently.

No "daemon disconnected" feedback
  If daemon crashes, dashboard appears frozen with no explanation.
  All actions silently fail until daemon is restarted.

Scrollback offset can accumulate drift on very long sessions
  When vt100 buffer is full (10K lines), scroll_offset adjustment
  in process_output may drift slightly. Clamped per call but
  could accumulate over thousands of updates.

No visual feedback when agent transitions to Done
  Agent finishes -> state label changes from "work" to "done"
  but no flash, highlight, or notification draws attention to it.

Ctrl+C quits from any dashboard mode
  Could be surprising if user expects Ctrl+C to cancel current
  action. But q also quits, so Ctrl+C is a fallback. Standard
  terminal behavior, probably fine.
