use chrono::Utc;

use crate::agent::{Agent, AgentState};
use crate::state::try_with_state;

/// Arguments for the `clamor set-state` primitive.
///
/// The primitive is harness-agnostic: any external hook, script, or tool
/// can call it to update an agent's state. The agent id must be passed
/// explicitly — clamor does not read `CLAMOR_AGENT_ID` from the
/// environment, to keep the contract free of hidden coupling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStateArgs {
    pub agent_id: String,
    pub new_state: Option<AgentState>,
    pub tool: Option<String>,
    pub session_token: Option<String>,
    pub activity_only: bool,
}

/// Run the `set-state` subcommand. Silent on success.
///
/// Uses a non-blocking lock — a blocked lock + hook timeout is worse than a
/// missed update (dashboards detect stopped processes anyway).
pub fn run(args: SetStateArgs) {
    let _ = run_inner(args);
}

fn run_inner(args: SetStateArgs) -> anyhow::Result<()> {
    let debug = debug_enabled();
    if debug {
        debug_log_args(&args);
    }

    try_with_state(|state| {
        let Some(agent) = state.agents.get_mut(&args.agent_id) else {
            if debug {
                debug_log(&format!(
                    "-- agent {} not in state, skipped\n",
                    args.agent_id
                ));
            }
            return;
        };

        let state_before = format!("{:?}", agent.state);
        let token_before = agent.resume_token.clone();

        apply_state(agent, &args);

        if debug {
            debug_log(&format!(
                "-- agent={} state: {state_before} -> {:?} token: {:?} -> {:?}\n",
                args.agent_id, agent.state, token_before, agent.resume_token
            ));
        }
    })?;

    Ok(())
}

/// Pure state mutation. Applied atomically under the state lock.
pub fn apply_state(agent: &mut Agent, args: &SetStateArgs) {
    if let Some(ref token) = args.session_token {
        if agent.resume_token.as_ref() != Some(token) {
            agent.resume_token = Some(token.clone());
        }
    }

    if let Some(ref tool) = args.tool {
        agent.last_tool = Some(tool.clone());
    }

    if args.activity_only {
        agent.last_activity_at = Utc::now();
        return;
    }

    if let Some(ref new_state) = args.new_state {
        let changed = agent.state != *new_state;
        agent.state = new_state.clone();
        if changed || !matches!(new_state, AgentState::Done) {
            agent.last_activity_at = Utc::now();
        }
    }
}

fn debug_enabled() -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    std::path::PathBuf::from(home)
        .join(".clamor")
        .join("debug")
        .exists()
}

fn debug_log_args(args: &SetStateArgs) {
    let ts = Utc::now().to_rfc3339();
    let pid = std::process::id();
    debug_log(&format!(
        "\n>>> {ts} pid={pid} agent={} state={:?} tool={:?} token={:?} activity_only={}\n",
        args.agent_id, args.new_state, args.tool, args.session_token, args.activity_only
    ));
}

fn debug_log(msg: &str) {
    use std::io::Write;
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = std::path::PathBuf::from(home)
        .join(".clamor")
        .join("state.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(msg.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_agent(state: AgentState, token: Option<&str>) -> Agent {
        let now = Utc::now();
        Agent {
            id: "test".into(),
            title: "t".into(),
            folder_id: "f".into(),
            backend_id: "any".into(),
            cwd: "/tmp".into(),
            initial_prompt: None,
            state,
            started_at: now,
            last_activity_at: now,
            last_tool: None,
            resume_token: token.map(String::from),
            metadata: HashMap::new(),
            key: None,
            color_index: 0,
        }
    }

    fn args(new_state: Option<AgentState>) -> SetStateArgs {
        SetStateArgs {
            agent_id: "test".into(),
            new_state,
            tool: None,
            session_token: None,
            activity_only: false,
        }
    }

    #[test]
    fn transitions_working_to_input() {
        let mut a = make_agent(AgentState::Working, None);
        apply_state(&mut a, &args(Some(AgentState::Input)));
        assert_eq!(a.state, AgentState::Input);
    }

    #[test]
    fn stop_lands_on_done() {
        let mut a = make_agent(AgentState::Input, Some("sess-1"));
        apply_state(&mut a, &args(Some(AgentState::Done)));
        assert_eq!(a.state, AgentState::Done);
        assert_eq!(a.resume_token.as_deref(), Some("sess-1"));
    }

    #[test]
    fn session_token_updates_resume_token() {
        let mut a = make_agent(AgentState::Working, None);
        let mut a_args = args(Some(AgentState::Working));
        a_args.session_token = Some("sess-new".into());
        apply_state(&mut a, &a_args);
        assert_eq!(a.resume_token.as_deref(), Some("sess-new"));
    }

    #[test]
    fn activity_only_bumps_timestamp_without_changing_state() {
        let mut a = make_agent(AgentState::Input, None);
        let before = a.last_activity_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut a_args = args(None);
        a_args.activity_only = true;
        apply_state(&mut a, &a_args);
        assert_eq!(a.state, AgentState::Input);
        assert!(a.last_activity_at > before);
    }

    #[test]
    fn tool_label_is_stored_verbatim() {
        let mut a = make_agent(AgentState::Working, None);
        let mut a_args = args(Some(AgentState::Working));
        a_args.tool = Some("Bash: ls".into());
        apply_state(&mut a, &a_args);
        assert_eq!(a.last_tool.as_deref(), Some("Bash: ls"));
    }
}
