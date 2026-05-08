use std::fs::File;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
#[allow(unused_imports)]
use fs2::FileExt as _;
use notify::{EventKind, RecursiveMode, Watcher};

use crate::config::{ClamorConfig, WatchMode};
use crate::state::ClamorState;

/// Reads state.json with a shared lock, returning None on any failure.
fn reload_from_disk(path: &std::path::Path) -> Option<ClamorState> {
    let file = File::open(path).ok()?;
    file.lock_shared().ok()?;
    let mut contents = String::new();
    (&file).read_to_string(&mut contents).ok()?;
    file.unlock().ok()?;

    if contents.trim().is_empty() {
        return Some(ClamorState::default());
    }

    serde_json::from_str(&contents).ok()
}

/// Watches `~/.clamor/state.json` for changes and keeps an in-memory cache.
pub(crate) struct StateWatcher {
    cached: Arc<Mutex<ClamorState>>,
    _watcher: Box<dyn Watcher + Send>,
}

impl StateWatcher {
    fn new(config: &ClamorConfig) -> Result<Self> {
        let initial = ClamorState::load()?;
        let cached = Arc::new(Mutex::new(initial));

        let watch_dir = ClamorConfig::runtime_dir()?;
        let state_path = watch_dir.join("state.json");

        let cached_clone = cached.clone();
        let state_path_clone = state_path.clone();

        let handler = move |res: std::result::Result<notify::Event, notify::Error>| {
            let event = match res {
                Ok(e) => e,
                Err(_) => return,
            };

            let dominated = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));
            let affects_state = event.paths.iter().any(|p| p == &state_path_clone);

            if dominated && affects_state {
                if let Some(new_state) = reload_from_disk(&state_path_clone) {
                    *cached_clone.lock().unwrap() = new_state;
                }
            }
        };

        let watcher: Box<dyn Watcher + Send> = match config.dashboard.watch_mode {
            WatchMode::Poll => {
                let poll_config =
                    notify::Config::default().with_poll_interval(Duration::from_secs(1));
                let mut w = notify::PollWatcher::new(handler, poll_config)
                    .context("Failed to create poll watcher")?;
                w.watch(&watch_dir, RecursiveMode::NonRecursive)
                    .context("Failed to watch clamor directory")?;
                Box::new(w)
            }
            WatchMode::Fsevents => {
                let mut w = notify::RecommendedWatcher::new(handler, notify::Config::default())
                    .context("Failed to create file watcher")?;
                w.watch(&watch_dir, RecursiveMode::NonRecursive)
                    .context("Failed to watch clamor directory")?;
                Box::new(w)
            }
        };

        Ok(Self {
            cached,
            _watcher: watcher,
        })
    }

    fn get(&self) -> ClamorState {
        self.cached.lock().unwrap().clone()
    }

    fn invalidate(&self) {
        if let Ok(state) = ClamorState::load() {
            *self.cached.lock().unwrap() = state;
        }
    }
}

/// Public interface: either a live watcher or a direct-from-disk fallback.
pub enum StateSource {
    Watched(StateWatcher),
    Direct,
}

impl StateSource {
    /// Create a new state source based on config. Falls back to direct
    /// disk reads if the file watcher cannot be created.
    pub fn new(config: &ClamorConfig) -> Self {
        match StateWatcher::new(config) {
            Ok(w) => Self::Watched(w),
            Err(_) => Self::Direct,
        }
    }

    /// Get the current state (from cache or disk).
    pub fn get(&self) -> ClamorState {
        match self {
            Self::Watched(w) => w.get(),
            Self::Direct => ClamorState::load().unwrap_or_default(),
        }
    }

    /// Refresh the cache after a local `with_state` mutation.
    pub fn invalidate(&self) {
        if let Self::Watched(w) = self {
            w.invalidate();
        }
    }
}
