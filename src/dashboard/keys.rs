use crate::agent::Agent;

const KEY_POOL: &[char] = &[
    'a', 's', 'd', 'f', 'j', 'k', 'l', 'h', // homerow (g reserved for SelectFirst)
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', // overflow
];

/// Find the next available key from the pool that isn't already assigned.
pub fn next_available_key(agents: &[&Agent]) -> Option<char> {
    let used: std::collections::HashSet<char> = agents.iter().filter_map(|a| a.key).collect();
    KEY_POOL.iter().copied().find(|k| !used.contains(k))
}

pub fn key_pool() -> &'static [char] {
    KEY_POOL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentState;
    use chrono::Utc;

    fn make_agent(key: Option<char>) -> Agent {
        let now = Utc::now();
        Agent {
            id: String::new(),
            title: String::new(),
            folder_id: String::new(),
            backend_id: "claude-code".to_string(),
            cwd: String::new(),
            initial_prompt: None,
            state: AgentState::Working,
            started_at: now,
            last_activity_at: now,
            last_tool: None,
            resume_token: None,
            metadata: std::collections::HashMap::new(),
            key,
            color_index: 0,
        }
    }

    #[test]
    fn first_key_is_a() {
        let agents: Vec<&Agent> = vec![];
        assert_eq!(next_available_key(&agents), Some('a'));
    }

    #[test]
    fn skips_used_keys() {
        let a1 = make_agent(Some('a'));
        let a2 = make_agent(Some('s'));
        let agents: Vec<&Agent> = vec![&a1, &a2];
        assert_eq!(next_available_key(&agents), Some('d'));
    }

    #[test]
    fn no_key_pool_conflicts_with_dashboard_bindings() {
        use super::super::input::{handle_input, DashboardAction, InputMode};
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        // Build a key_map where every KEY_POOL char maps to a sentinel agent ID.
        let mut key_map: std::collections::HashMap<char, String> = std::collections::HashMap::new();
        for &k in KEY_POOL {
            key_map.insert(k, format!("agent-{k}"));
        }

        let mode = InputMode::Normal;

        for &k in KEY_POOL {
            let event = KeyEvent {
                code: KeyCode::Char(k),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            };
            let action = handle_input(event, &key_map, &mode);
            assert!(
                matches!(action, DashboardAction::Attach(_)),
                "KEY_POOL char '{k}' is intercepted by a dashboard keybinding \
                 instead of reaching the Attach catch-all. \
                 Remove '{k}' from KEY_POOL or change the keybinding."
            );
        }
    }

    #[test]
    fn returns_none_when_pool_exhausted() {
        let all_agents: Vec<Agent> = KEY_POOL.iter().map(|&k| make_agent(Some(k))).collect();
        let refs: Vec<&Agent> = all_agents.iter().collect();
        assert_eq!(next_available_key(&refs), None);
    }
}
