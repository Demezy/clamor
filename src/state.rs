use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use anyhow::Context;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::agent::Agent;
use crate::config::ClamorConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptHistoryEntry {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderState {
    #[serde(default)]
    pub selected_backend: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClamorState {
    #[serde(default)]
    pub agents: HashMap<String, Agent>,
    #[serde(default)]
    pub folder_state: HashMap<String, FolderState>,
    #[serde(default)]
    pub prompt_history: Vec<PromptHistoryEntry>,
}

fn allowed_backends_for_folder<'a>(
    config: &'a ClamorConfig,
    folder_id: &str,
) -> Option<Vec<&'a str>> {
    let allowed: Vec<&str> = config
        .folder_backends(folder_id)?
        .iter()
        .map(String::as_str)
        .filter(|backend_id| config.backends.contains_key(*backend_id))
        .collect();
    if allowed.is_empty() {
        None
    } else {
        Some(allowed)
    }
}

impl ClamorState {
    fn state_path() -> anyhow::Result<std::path::PathBuf> {
        Ok(ClamorConfig::runtime_dir()?.join("state.json"))
    }

    /// Reads state from `~/.clamor/state.json` with a shared (read) lock.
    /// Returns empty state if the file doesn't exist.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::state_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let file = File::open(&path)
            .with_context(|| format!("Failed to open state file: {}", path.display()))?;

        file.lock_shared()
            .context("Failed to acquire shared lock on state file")?;

        let mut contents = String::new();
        (&file)
            .read_to_string(&mut contents)
            .with_context(|| format!("Failed to read state file: {}", path.display()))?;

        file.unlock().context("Failed to unlock state file")?;

        if contents.trim().is_empty() {
            return Ok(Self::default());
        }

        serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse state file: {}", path.display()))
    }

    /// Writes state to `~/.clamor/state.json` with an exclusive file lock.
    #[allow(dead_code)]
    pub fn save(&self) -> anyhow::Result<()> {
        ClamorConfig::ensure_runtime_dir()?;
        let path = Self::state_path()?;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .with_context(|| {
                format!("Failed to open state file for writing: {}", path.display())
            })?;

        file.lock_exclusive()
            .context("Failed to acquire lock on state file")?;

        let json = serde_json::to_string_pretty(self).context("Failed to serialize state")?;

        (&file)
            .write_all(json.as_bytes())
            .with_context(|| format!("Failed to write state file: {}", path.display()))?;

        file.unlock().context("Failed to unlock state file")?;

        Ok(())
    }
}

pub fn selected_backend_for_folder(
    config: &ClamorConfig,
    state: &ClamorState,
    folder_id: &str,
) -> Option<String> {
    let allowed = allowed_backends_for_folder(config, folder_id)?;

    let selected = state
        .folder_state
        .get(folder_id)
        .and_then(|folder| folder.selected_backend.as_deref());

    match selected {
        Some(selected) if allowed.contains(&selected) => Some(selected.to_string()),
        _ => Some(allowed.first()?.to_string()),
    }
}

pub fn cycle_backend_for_folder(
    config: &ClamorConfig,
    state: &mut ClamorState,
    folder_id: &str,
    reverse: bool,
) -> Option<String> {
    let allowed = allowed_backends_for_folder(config, folder_id)?;
    let current = selected_backend_for_folder(config, state, folder_id)?;
    let current_idx = allowed
        .iter()
        .position(|backend_id| *backend_id == current)?;
    let next_idx = if reverse {
        current_idx.checked_sub(1).unwrap_or(allowed.len() - 1)
    } else {
        (current_idx + 1) % allowed.len()
    };
    let next = allowed[next_idx].to_string();

    state
        .folder_state
        .entry(folder_id.to_string())
        .or_default()
        .selected_backend = Some(next.clone());

    Some(next)
}

pub fn reconcile_folder_backend_selections(config: &ClamorConfig, state: &mut ClamorState) -> bool {
    let mut changed = false;

    for folder_id in config.folders.keys() {
        let selected = selected_backend_for_folder(config, state, folder_id);
        let entry = state.folder_state.entry(folder_id.clone()).or_default();
        if entry.selected_backend != selected {
            entry.selected_backend = selected;
            changed = true;
        }
    }

    changed
}

/// Atomic read-modify-write on the state file.
///
/// Acquires an exclusive lock, reads current state, applies `f`, writes back,
/// and releases the lock. Returns whatever `f` returns.
pub fn with_state<F, R>(f: F) -> anyhow::Result<R>
where
    F: FnOnce(&mut ClamorState) -> R,
{
    with_state_inner(f, true)
}

/// Like `with_state`, but non-blocking: returns Ok(None) if the lock can't be acquired.
/// Used by hooks to avoid blocking Claude Code.
pub fn try_with_state<F, R>(f: F) -> anyhow::Result<Option<R>>
where
    F: FnOnce(&mut ClamorState) -> R,
{
    match with_state_inner(f, false) {
        Ok(r) => Ok(Some(r)),
        Err(e) => {
            if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    return Ok(None);
                }
            }
            Err(e)
        }
    }
}

/// Async variant of `with_state` — runs the file-locked read-modify-write
/// on the tokio blocking thread pool to avoid stalling the async runtime.
#[allow(dead_code)]
pub async fn with_state_async<F, R>(f: F) -> anyhow::Result<R>
where
    F: FnOnce(&mut ClamorState) -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(move || with_state(f)).await?
}

fn with_state_inner<F, R>(f: F, blocking: bool) -> anyhow::Result<R>
where
    F: FnOnce(&mut ClamorState) -> R,
{
    ClamorConfig::ensure_runtime_dir()?;
    let path = ClamorConfig::runtime_dir()?.join("state.json");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("Failed to open state file: {}", path.display()))?;

    if blocking {
        file.lock_exclusive()
            .context("Failed to acquire lock on state file")?;
    } else {
        file.try_lock_exclusive()
            .context("Failed to acquire lock on state file (non-blocking)")?;
    }

    let mut contents = String::new();
    (&file)
        .read_to_string(&mut contents)
        .with_context(|| format!("Failed to read state file: {}", path.display()))?;

    let mut state: ClamorState = if contents.trim().is_empty() {
        ClamorState::default()
    } else {
        serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse state file: {}", path.display()))?
    };

    let result = f(&mut state);

    let json = serde_json::to_string_pretty(&state).context("Failed to serialize state")?;

    use std::io::Seek;
    (&file)
        .seek(std::io::SeekFrom::Start(0))
        .context("Failed to seek state file")?;
    file.set_len(0).context("Failed to truncate state file")?;

    (&file)
        .write_all(json.as_bytes())
        .with_context(|| format!("Failed to write state file: {}", path.display()))?;

    file.unlock().context("Failed to unlock state file")?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_first_configured_backend() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
backends:
  claude-code:
    display_name: Claude
  open-code:
    display_name: OpenCode
folders:
  work:
    path: ~/work
    backends: [claude-code, open-code]
"#,
        )
        .unwrap();
        let mut state = ClamorState::default();
        state.folder_state.insert(
            "work".to_string(),
            FolderState {
                selected_backend: Some("missing".to_string()),
            },
        );

        assert_eq!(
            selected_backend_for_folder(&config, &state, "work"),
            Some("claude-code".to_string())
        );
        assert!(reconcile_folder_backend_selections(&config, &mut state));
        assert_eq!(
            state.folder_state["work"].selected_backend.as_deref(),
            Some("claude-code")
        );
    }

    #[test]
    fn keeps_valid_selected_backend() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
backends:
  claude-code:
    display_name: Claude
  open-code:
    display_name: OpenCode
folders:
  work:
    path: ~/work
    backends: [claude-code, open-code]
"#,
        )
        .unwrap();
        let mut state = ClamorState::default();
        state.folder_state.insert(
            "work".to_string(),
            FolderState {
                selected_backend: Some("open-code".to_string()),
            },
        );

        assert_eq!(
            selected_backend_for_folder(&config, &state, "work"),
            Some("open-code".to_string())
        );
        assert!(!reconcile_folder_backend_selections(&config, &mut state));
    }

    #[test]
    fn deserializes_legacy_state_without_folder_state() {
        let state: ClamorState =
            serde_json::from_str(r#"{"agents": {}, "prompt_history": []}"#).unwrap();
        assert!(state.folder_state.is_empty());
    }

    #[test]
    fn cycles_backend_selection_forward_and_persists_it() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
backends:
  claude-code:
    display_name: Claude
  open-code:
    display_name: OpenCode
  pi:
    display_name: Pi
folders:
  work:
    path: ~/work
    backends: [claude-code, open-code, pi]
"#,
        )
        .unwrap();
        let mut state = ClamorState::default();

        assert_eq!(
            cycle_backend_for_folder(&config, &mut state, "work", false),
            Some("open-code".to_string())
        );
        assert_eq!(
            state.folder_state["work"].selected_backend.as_deref(),
            Some("open-code")
        );
    }

    #[test]
    fn cycles_backend_selection_backward_and_wraps() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
backends:
  claude-code:
    display_name: Claude
  open-code:
    display_name: OpenCode
  pi:
    display_name: Pi
folders:
  work:
    path: ~/work
    backends: [claude-code, open-code, pi]
"#,
        )
        .unwrap();
        let mut state = ClamorState::default();

        assert_eq!(
            cycle_backend_for_folder(&config, &mut state, "work", true),
            Some("pi".to_string())
        );
        assert_eq!(
            cycle_backend_for_folder(&config, &mut state, "work", false),
            Some("claude-code".to_string())
        );
    }
}
