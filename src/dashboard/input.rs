use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Which field is active in the spawn popup.
pub enum PromptField {
    Title,
    Description,
    Backend,
}

/// Actions the dashboard can take in response to keyboard input.
pub enum DashboardAction {
    Attach(String),
    SpawnInline,
    SpawnEditor,
    KillAgent(String),
    PendingKill,
    ReloadAgent(String),
    PendingReload,
    EditAgent(String),
    PendingEdit,
    EditInput(PromptEdit),
    EditSubmitted,
    FolderPicked(usize),
    PromptSubmitted,
    PromptInput(PromptEdit),
    PromptCycleBackend { reverse: bool },
    PromptCycleField { reverse: bool },
    AdoptStart,
    AdoptInput(PromptEdit),
    AdoptSubmitted,
    StartFilter,
    FilterInput(PromptEdit),
    FilterAccept,
    SelectNext,
    SelectPrev,
    SelectFirst,
    SelectLast,
    PendingG,
    AttachSelected,
    ToggleSelect,
    ToggleSelectAll,
    ClearSelection,
    ShowHelp,
    HelpScroll(i32), // positive = down, negative = up
    HelpStartFilter,
    HelpFilterInput(PromptEdit),
    HelpFilterAccept,
    ShowQuitHint,
    ConfirmYes,
    Cancel,
    Quit,
    Refresh,
    JumpInputNext,
    JumpInputPrev,
    RebindStart,
}

/// Edits to a text input.
pub enum PromptEdit {
    Char(char),
    Paste(String),
    Backspace,
    DeleteWord,
    DeleteLine,
    HistoryPrev,
    HistoryNext,
}

/// Why the folder picker is being shown.
pub enum FolderPickReason {
    SpawnInline,
    SpawnEditor,
    Adopt,
}

/// Dashboard input mode.
pub enum InputMode {
    Normal,
    WaitingKill,
    WaitingReload,
    ConfirmReload {
        agent_id: String,
        title: String,
    },
    PickingFolder {
        folder_count: usize,
        reason: FolderPickReason,
    },
    TypingPrompt {
        folder_name: String,
        folder_path: String,
        title: String,
        description: String,
        active_field: PromptField,
    },
    TypingAdopt {
        input: String,
        folder_name: String,
        folder_path: String,
    },
    ConfirmEmptySpawn {
        folder_name: String,
        folder_path: String,
    },
    ConfirmKill {
        agent_id: String,
        title: String,
    },
    ConfirmBatchKill,
    ConfirmRebind,
    ReloadUnavailable {
        reason: String,
    },
    ActionFailed {
        reason: String,
    },
    QuitHint,
    WaitingEdit,
    EditingDescription {
        agent_id: String,
        input: String,
    },
    Filtering {
        query: String,
    },
    Help {
        scroll: usize,
        filter: String,
        filtering: bool, // true when typing in the search field
    },
}

/// Process a keyboard event and return the corresponding action.
pub fn handle_input(
    event: KeyEvent,
    key_map: &HashMap<char, String>,
    mode: &InputMode,
) -> DashboardAction {
    if event.modifiers.contains(KeyModifiers::CONTROL) && matches!(event.code, KeyCode::Char('c')) {
        return match mode {
            InputMode::Normal => DashboardAction::ShowQuitHint,
            _ => DashboardAction::Cancel,
        };
    }

    // Ctrl+G / Ctrl+Shift+G: jump to next/prev agent in Input state.
    // Intercepted only in Normal mode so popups aren't disrupted.
    if matches!(mode, InputMode::Normal)
        && event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(event.code, KeyCode::Char('g') | KeyCode::Char('G'))
    {
        let shifted = event.modifiers.contains(KeyModifiers::SHIFT)
            || matches!(event.code, KeyCode::Char('G'));
        return if shifted {
            DashboardAction::JumpInputPrev
        } else {
            DashboardAction::JumpInputNext
        };
    }

    match mode {
        InputMode::Normal => handle_normal(event, key_map),
        InputMode::WaitingKill => handle_pending_kill(event, key_map),
        InputMode::WaitingReload => handle_pending_reload(event, key_map),
        InputMode::ConfirmReload { .. } => handle_confirm_reload_input(event),
        InputMode::WaitingEdit => handle_pending_edit(event, key_map),
        InputMode::EditingDescription { .. } => handle_edit_input(event),
        InputMode::PickingFolder { folder_count, .. } => handle_folder_pick(event, *folder_count),
        InputMode::TypingPrompt { active_field, .. } => handle_prompt_input(event, active_field),
        InputMode::TypingAdopt { .. } => handle_adopt_input(event),
        InputMode::ConfirmEmptySpawn { .. } => handle_confirm_input(event),
        InputMode::ConfirmKill { .. } => handle_confirm_kill_input(event),
        InputMode::ConfirmBatchKill => handle_confirm_batch_kill(event),
        InputMode::ConfirmRebind => handle_confirm_rebind_input(event),
        InputMode::ReloadUnavailable { .. } => DashboardAction::Cancel,
        InputMode::ActionFailed { .. } => DashboardAction::Cancel,
        InputMode::QuitHint => handle_quit_hint(event),
        InputMode::Filtering { .. } => handle_filter_input(event),
        InputMode::Help { filtering, .. } => handle_help_input(event, *filtering),
    }
}

fn handle_normal(event: KeyEvent, key_map: &HashMap<char, String>) -> DashboardAction {
    match event.code {
        KeyCode::Char('q') => DashboardAction::Quit,
        KeyCode::Char('C') => DashboardAction::SpawnEditor,
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::SHIFT) => {
            DashboardAction::SpawnEditor
        }
        KeyCode::Char('c') => DashboardAction::SpawnInline,
        KeyCode::Char('x') => DashboardAction::PendingKill,
        KeyCode::Char('e') => DashboardAction::PendingEdit,
        KeyCode::Char('r') => DashboardAction::PendingReload,
        KeyCode::Char('R') => DashboardAction::AdoptStart,
        KeyCode::Char('b') => DashboardAction::RebindStart,
        KeyCode::Char('J') | KeyCode::Down => DashboardAction::SelectNext,
        KeyCode::Char('j') if event.modifiers.contains(KeyModifiers::SHIFT) => {
            DashboardAction::SelectNext
        }
        KeyCode::Char('K') | KeyCode::Up => DashboardAction::SelectPrev,
        KeyCode::Char('k') if event.modifiers.contains(KeyModifiers::SHIFT) => {
            DashboardAction::SelectPrev
        }
        KeyCode::Char('g') => DashboardAction::PendingG,
        KeyCode::Char('G') => DashboardAction::SelectLast,
        KeyCode::Enter => DashboardAction::AttachSelected,
        KeyCode::Char('/') => DashboardAction::StartFilter,
        KeyCode::Char('v') => DashboardAction::ToggleSelect,
        KeyCode::Char('V') => DashboardAction::ToggleSelectAll,
        KeyCode::Char('?') => DashboardAction::ShowHelp,
        KeyCode::Char(c) => match key_map.get(&c) {
            Some(agent_id) => DashboardAction::Attach(agent_id.clone()),
            None => DashboardAction::Refresh,
        },
        KeyCode::Esc => DashboardAction::ClearSelection,
        _ => DashboardAction::Refresh,
    }
}

fn handle_pending_kill(event: KeyEvent, key_map: &HashMap<char, String>) -> DashboardAction {
    match event.code {
        KeyCode::Char(c) => match key_map.get(&c) {
            Some(agent_id) => DashboardAction::KillAgent(agent_id.clone()),
            None => DashboardAction::Cancel,
        },
        KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Cancel,
    }
}

fn handle_pending_reload(event: KeyEvent, key_map: &HashMap<char, String>) -> DashboardAction {
    match event.code {
        KeyCode::Char(c) => match key_map.get(&c) {
            Some(agent_id) => DashboardAction::ReloadAgent(agent_id.clone()),
            None => DashboardAction::Cancel,
        },
        KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Cancel,
    }
}

fn handle_confirm_reload_input(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Enter => DashboardAction::ConfirmYes,
        KeyCode::Esc | KeyCode::Char('n') => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_pending_edit(event: KeyEvent, key_map: &HashMap<char, String>) -> DashboardAction {
    match event.code {
        KeyCode::Char(c) => match key_map.get(&c) {
            Some(agent_id) => DashboardAction::EditAgent(agent_id.clone()),
            None => DashboardAction::Cancel,
        },
        KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Cancel,
    }
}

fn handle_edit_input(event: KeyEvent) -> DashboardAction {
    if let Some(edit) = check_text_shortcut(&event) {
        return DashboardAction::EditInput(edit);
    }
    match event.code {
        KeyCode::Enter => DashboardAction::EditSubmitted,
        KeyCode::Esc => DashboardAction::Cancel,
        KeyCode::Backspace => DashboardAction::EditInput(PromptEdit::Backspace),
        KeyCode::Char(c) => DashboardAction::EditInput(PromptEdit::Char(c)),
        _ => DashboardAction::Refresh,
    }
}

fn handle_folder_pick(event: KeyEvent, folder_count: usize) -> DashboardAction {
    match event.code {
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = c.to_digit(10).unwrap() as usize;
            if n >= 1 && n <= folder_count {
                DashboardAction::FolderPicked(n - 1)
            } else {
                DashboardAction::Refresh
            }
        }
        KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_prompt_input(event: KeyEvent, active_field: &PromptField) -> DashboardAction {
    match active_field {
        PromptField::Backend => match event.code {
            KeyCode::Tab => DashboardAction::PromptCycleField { reverse: false },
            KeyCode::BackTab => DashboardAction::PromptCycleField { reverse: true },
            KeyCode::Left => DashboardAction::PromptCycleBackend { reverse: true },
            KeyCode::Right => DashboardAction::PromptCycleBackend { reverse: false },
            KeyCode::Enter => DashboardAction::PromptSubmitted,
            KeyCode::Esc => DashboardAction::Cancel,
            _ => DashboardAction::Refresh,
        },
        PromptField::Title | PromptField::Description => {
            if let Some(edit) = check_text_shortcut(&event) {
                return DashboardAction::PromptInput(edit);
            }
            match event.code {
                // Shift+Enter or Alt+Enter inserts newline in description
                KeyCode::Enter
                    if matches!(active_field, PromptField::Description)
                        && (event.modifiers.contains(KeyModifiers::SHIFT)
                            || event.modifiers.contains(KeyModifiers::ALT)) =>
                {
                    DashboardAction::PromptInput(PromptEdit::Char('\n'))
                }
                KeyCode::Enter => DashboardAction::PromptSubmitted,
                KeyCode::Esc => DashboardAction::Cancel,
                KeyCode::Tab => DashboardAction::PromptCycleField { reverse: false },
                KeyCode::BackTab => DashboardAction::PromptCycleField { reverse: true },
                KeyCode::Up => DashboardAction::PromptInput(PromptEdit::HistoryPrev),
                KeyCode::Down => DashboardAction::PromptInput(PromptEdit::HistoryNext),
                KeyCode::Backspace => DashboardAction::PromptInput(PromptEdit::Backspace),
                KeyCode::Char(c) => DashboardAction::PromptInput(PromptEdit::Char(c)),
                _ => DashboardAction::Refresh,
            }
        }
    }
}

fn handle_adopt_input(event: KeyEvent) -> DashboardAction {
    if let Some(edit) = check_text_shortcut(&event) {
        return DashboardAction::AdoptInput(edit);
    }
    match event.code {
        KeyCode::Enter => DashboardAction::AdoptSubmitted,
        KeyCode::Esc => DashboardAction::Cancel,
        KeyCode::Backspace => DashboardAction::AdoptInput(PromptEdit::Backspace),
        KeyCode::Char(c) => DashboardAction::AdoptInput(PromptEdit::Char(c)),
        _ => DashboardAction::Refresh,
    }
}

fn handle_confirm_input(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Char('y') => DashboardAction::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_confirm_kill_input(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Enter => DashboardAction::ConfirmYes,
        KeyCode::Esc | KeyCode::Char('n') => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_confirm_batch_kill(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Enter => DashboardAction::ConfirmYes,
        KeyCode::Esc => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_confirm_rebind_input(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Enter => DashboardAction::ConfirmYes,
        KeyCode::Esc | KeyCode::Char('n') => DashboardAction::Cancel,
        _ => DashboardAction::Refresh,
    }
}

fn handle_quit_hint(event: KeyEvent) -> DashboardAction {
    match event.code {
        KeyCode::Char('q') => DashboardAction::Quit,
        _ => DashboardAction::Cancel,
    }
}

fn handle_filter_input(event: KeyEvent) -> DashboardAction {
    if let Some(edit) = check_text_shortcut(&event) {
        return DashboardAction::FilterInput(edit);
    }
    match event.code {
        KeyCode::Enter => DashboardAction::FilterAccept,
        KeyCode::Esc => DashboardAction::Cancel,
        KeyCode::Backspace => DashboardAction::FilterInput(PromptEdit::Backspace),
        KeyCode::Char(c) => DashboardAction::FilterInput(PromptEdit::Char(c)),
        _ => DashboardAction::Refresh,
    }
}

fn handle_help_input(event: KeyEvent, filtering: bool) -> DashboardAction {
    if filtering {
        if let Some(edit) = check_text_shortcut(&event) {
            return DashboardAction::HelpFilterInput(edit);
        }
        return match event.code {
            KeyCode::Enter => DashboardAction::HelpFilterAccept,
            KeyCode::Esc => DashboardAction::HelpFilterAccept, // accept and return to browse
            KeyCode::Backspace => DashboardAction::HelpFilterInput(PromptEdit::Backspace),
            KeyCode::Char(c) => DashboardAction::HelpFilterInput(PromptEdit::Char(c)),
            _ => DashboardAction::Refresh,
        };
    }
    match event.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => DashboardAction::Cancel,
        KeyCode::Char('j') | KeyCode::Down => DashboardAction::HelpScroll(1),
        KeyCode::Char('k') | KeyCode::Up => DashboardAction::HelpScroll(-1),
        KeyCode::Char('d') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            DashboardAction::HelpScroll(10)
        }
        KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            DashboardAction::HelpScroll(-10)
        }
        KeyCode::Char('g') => DashboardAction::HelpScroll(i32::MIN), // top
        KeyCode::Char('G') => DashboardAction::HelpScroll(i32::MAX), // bottom
        KeyCode::Char('/') => DashboardAction::HelpStartFilter,
        _ => DashboardAction::Refresh,
    }
}

/// Check for macOS-style text editing shortcuts.
/// Ctrl+W / Alt+Backspace → delete word, Ctrl+U → delete line.
fn check_text_shortcut(event: &KeyEvent) -> Option<PromptEdit> {
    match event.code {
        KeyCode::Char('w') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(PromptEdit::DeleteWord)
        }
        KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(PromptEdit::DeleteLine)
        }
        KeyCode::Backspace if event.modifiers.contains(KeyModifiers::ALT) => {
            Some(PromptEdit::DeleteWord)
        }
        KeyCode::Backspace if event.modifiers.contains(KeyModifiers::SUPER) => {
            Some(PromptEdit::DeleteLine)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn tab_cycles_field_in_spawn_mode() {
        let action = handle_prompt_input(
            key_event(KeyCode::Tab, KeyModifiers::NONE),
            &PromptField::Title,
        );
        assert!(matches!(
            action,
            DashboardAction::PromptCycleField { reverse: false }
        ));

        let action = handle_prompt_input(
            key_event(KeyCode::BackTab, KeyModifiers::SHIFT),
            &PromptField::Title,
        );
        assert!(matches!(
            action,
            DashboardAction::PromptCycleField { reverse: true }
        ));
    }

    #[test]
    fn left_right_cycles_backend_in_backend_field() {
        let action = handle_prompt_input(
            key_event(KeyCode::Left, KeyModifiers::NONE),
            &PromptField::Backend,
        );
        assert!(matches!(
            action,
            DashboardAction::PromptCycleBackend { reverse: true }
        ));

        let action = handle_prompt_input(
            key_event(KeyCode::Right, KeyModifiers::NONE),
            &PromptField::Backend,
        );
        assert!(matches!(
            action,
            DashboardAction::PromptCycleBackend { reverse: false }
        ));
    }

    #[test]
    fn backend_field_ignores_char_input() {
        let action = handle_prompt_input(
            key_event(KeyCode::Char('a'), KeyModifiers::NONE),
            &PromptField::Backend,
        );
        assert!(matches!(action, DashboardAction::Refresh));
    }
}
