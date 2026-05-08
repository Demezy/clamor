use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_backend_id() -> String {
    "claude-code".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Input,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    /// Display name shown in dashboard. Renamed from `description`.
    #[serde(alias = "description")]
    pub title: String,
    #[serde(alias = "folder")]
    pub folder_id: String,
    #[serde(default = "default_backend_id")]
    pub backend_id: String,
    pub cwd: String,
    /// Prompt sent to claude. None = interactive (bare `claude`).
    #[serde(default)]
    pub initial_prompt: Option<String>,
    pub state: AgentState,
    pub started_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub last_tool: Option<String>,
    /// Generic backend resume token. Legacy `session_id` still deserializes here.
    #[serde(default)]
    #[serde(alias = "session_id")]
    pub resume_token: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub key: Option<char>,
    #[serde(default)]
    pub color_index: u8,
}

pub fn generate_id(existing: &std::collections::HashSet<String>) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    loop {
        let n: u32 = rng.gen_range(0..0xFFFFFF);
        let id = format!("{:06x}", n);
        if !existing.contains(&id) {
            return id;
        }
    }
}

pub fn next_color_index(existing: &[&Agent]) -> u8 {
    let max = existing.iter().map(|a| a.color_index).max().unwrap_or(0);
    if existing.is_empty() {
        0
    } else {
        max.wrapping_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_fields() {
        let agent: Agent = serde_json::from_str(
            r#"{
                "id": "abc123",
                "title": "task",
                "folder": "work",
                "cwd": "/tmp/work",
                "state": "input",
                "started_at": "2026-03-29T00:00:00Z",
                "last_activity_at": "2026-03-29T00:00:00Z",
                "last_tool": null,
                "session_id": "sess-1",
                "key": "a",
                "color_index": 1
            }"#,
        )
        .unwrap();

        assert_eq!(agent.folder_id, "work");
        assert_eq!(agent.backend_id, "claude-code");
        assert_eq!(agent.resume_token.as_deref(), Some("sess-1"));
    }
}
