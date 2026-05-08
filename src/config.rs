use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClamorConfig {
    #[serde(default, deserialize_with = "deserialize_folders")]
    pub folders: HashMap<String, FolderConfig>,
    #[serde(default)]
    pub backends: HashMap<String, BackendConfig>,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    XdgYaml(PathBuf),
    LegacyJson(PathBuf),
}

impl ConfigSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::XdgYaml(path) | Self::LegacyJson(path) => path.as_path(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: ClamorConfig,
    #[allow(dead_code)]
    pub source: ConfigSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPaths {
    pub legacy_path: PathBuf,
    pub xdg_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderConfig {
    pub path: String,
    #[serde(default)]
    pub backends: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BackendConfig {
    pub display_name: String,
    #[serde(default)]
    pub spawn: BackendCommandConfig,
    #[serde(default)]
    pub resume: Option<BackendCommandConfig>,
    #[serde(default)]
    pub capabilities: BackendCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BackendCommandConfig {
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub title_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BackendCapabilities {
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub sync_output_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum FolderConfigDef {
    LegacyPath(String),
    Structured(FolderConfig),
}

fn validate_config(config: &ClamorConfig) -> anyhow::Result<()> {
    for (folder_id, folder) in &config.folders {
        if folder.backends.is_empty() {
            bail!("Folder '{folder_id}' must define at least one backend");
        }

        for backend_id in &folder.backends {
            if !config.backends.contains_key(backend_id) {
                bail!("Folder '{folder_id}' references unknown backend '{backend_id}'");
            }
        }
    }

    Ok(())
}

fn write_config_file(path: &Path, config: &ClamorConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
    }

    let yaml = serialize_config_yaml(config)?;
    std::fs::write(path, yaml)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

fn load_config_from_path(path: &Path, source: &ConfigSource) -> anyhow::Result<ClamorConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;

    let config: ClamorConfig = match source {
        ConfigSource::XdgYaml(_) => serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?,
        ConfigSource::LegacyJson(_) => serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?,
    };

    validate_config(&config)?;
    Ok(config)
}

pub fn starter_config() -> ClamorConfig {
    ClamorConfig::default()
}

pub fn example_config() -> ClamorConfig {
    let claude_code = BackendConfig {
        display_name: "Claude".to_string(),
        spawn: BackendCommandConfig {
            cmd: vec!["claude".to_string(), "{{prompt}}".to_string()],
            env: HashMap::new(),
            title_template: Some("{{title}}".to_string()),
        },
        resume: Some(BackendCommandConfig {
            cmd: vec![
                "claude".to_string(),
                "--resume".to_string(),
                "{{resume_token}}".to_string(),
            ],
            env: HashMap::new(),
            title_template: Some("{{title}}".to_string()),
        }),
        capabilities: BackendCapabilities {
            resume: true,
            sync_output_mode: true,
        },
    };
    let open_code = BackendConfig {
        display_name: "OpenCode".to_string(),
        spawn: BackendCommandConfig {
            cmd: vec![
                "opencode".to_string(),
                "--prompt".to_string(),
                "{{prompt}}".to_string(),
            ],
            env: HashMap::new(),
            title_template: Some("{{title}}".to_string()),
        },
        ..BackendConfig::default()
    };

    ClamorConfig {
        folders: HashMap::from([(
            "work".to_string(),
            FolderConfig {
                path: "~/Vault/Projects/work".to_string(),
                backends: vec!["claude-code".to_string(), "open-code".to_string()],
            },
        )]),
        backends: HashMap::from([
            ("claude-code".to_string(), claude_code),
            ("open-code".to_string(), open_code),
        ]),
        dashboard: DashboardConfig::default(),
        terminal: TerminalConfig::default(),
        theme: ThemeConfig::default(),
    }
}

pub fn serialize_config_yaml(config: &ClamorConfig) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct ConfigForOutput {
        backends: BTreeMap<String, BackendConfig>,
        folders: BTreeMap<String, FolderConfig>,
        dashboard: DashboardConfig,
        terminal: TerminalConfig,
        theme: ThemeConfig,
    }

    let output = ConfigForOutput {
        backends: config.backends.clone().into_iter().collect(),
        folders: config.folders.clone().into_iter().collect(),
        dashboard: config.dashboard.clone(),
        terminal: config.terminal.clone(),
        theme: config.theme.clone(),
    };

    serde_yaml::to_string(&output).context("Failed to serialize config")
}

fn init_config_at_paths(xdg_path: &Path, legacy_path: &Path) -> anyhow::Result<PathBuf> {
    if xdg_path.exists() {
        bail!("Config already exists at {}", xdg_path.display());
    }

    if legacy_path.exists() {
        bail!(
            "Legacy config exists at {}. Run `clamor config migrate` instead of `clamor config init`.",
            legacy_path.display()
        );
    }

    write_config_file(xdg_path, &starter_config())?;
    Ok(xdg_path.to_path_buf())
}

pub fn init_config() -> anyhow::Result<PathBuf> {
    let xdg_path = ClamorConfig::config_path()?;
    let legacy_path = ClamorConfig::legacy_config_path()?;
    init_config_at_paths(&xdg_path, &legacy_path)
}

pub fn detect_explicit_migration() -> anyhow::Result<Option<MigrationPaths>> {
    let xdg_path = ClamorConfig::config_path()?;
    let legacy_path = ClamorConfig::legacy_config_path()?;

    if xdg_path.exists() || !legacy_path.exists() {
        return Ok(None);
    }

    Ok(Some(MigrationPaths {
        legacy_path,
        xdg_path,
    }))
}

fn migrate_legacy_config_at_paths(paths: &MigrationPaths) -> anyhow::Result<MigrationOutcome> {
    let config = load_config_from_path(
        &paths.legacy_path,
        &ConfigSource::LegacyJson(paths.legacy_path.clone()),
    )?;
    write_config_file(&paths.xdg_path, &config)?;

    Ok(MigrationOutcome {
        from: paths.legacy_path.clone(),
        to: paths.xdg_path.clone(),
    })
}

pub fn migrate_legacy_config() -> anyhow::Result<MigrationOutcome> {
    let paths = detect_explicit_migration()?.context("No legacy config to migrate")?;
    migrate_legacy_config_at_paths(&paths)
}

fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(io::stdin().as_raw_fd()) == 1 }
}

fn prompt_to_migrate_legacy_config_at_paths<R: BufRead, W: Write>(
    paths: Option<MigrationPaths>,
    stdin_is_tty: bool,
    input: &mut R,
    error: &mut W,
) -> anyhow::Result<bool> {
    let Some(paths) = paths else {
        return Ok(false);
    };

    writeln!(
        error,
        "Legacy config detected at {}.",
        paths.legacy_path.display()
    )
    .ok();

    if !stdin_is_tty {
        writeln!(
            error,
            "Non-interactive stdin detected; continuing with the legacy config. Run `clamor config migrate` when you're ready."
        )
        .ok();
        return Ok(false);
    }

    writeln!(
        error,
        "Migrate it to {} now? [Y/n]",
        paths.xdg_path.display()
    )
    .ok();
    error.flush().ok();

    let mut answer = String::new();
    let bytes_read = input
        .read_line(&mut answer)
        .context("Failed to read migration confirmation")?;

    if bytes_read == 0 || (!answer.trim().is_empty() && !answer.trim().eq_ignore_ascii_case("y")) {
        writeln!(
            error,
            "Continuing with the legacy config for now. Run `clamor config migrate` when you're ready."
        )
        .ok();
        return Ok(false);
    }

    let outcome = migrate_legacy_config_at_paths(&paths)?;
    writeln!(
        error,
        "Migrated config to {}. The legacy file was left in place for safety.",
        outcome.to.display()
    )
    .ok();
    Ok(true)
}

pub fn prompt_to_migrate_legacy_config() -> anyhow::Result<bool> {
    let mut input = io::BufReader::new(io::stdin().lock());
    let mut error = io::stderr().lock();
    prompt_to_migrate_legacy_config_at_paths(
        detect_explicit_migration()?,
        stdin_is_tty(),
        &mut input,
        &mut error,
    )
}

fn config_path_for_editing_at_paths<R: BufRead, W: Write>(
    xdg_path: PathBuf,
    legacy_path: PathBuf,
    stdin_is_tty: bool,
    input: &mut R,
    error: &mut W,
) -> anyhow::Result<PathBuf> {
    let migration = if xdg_path.exists() || !legacy_path.exists() {
        None
    } else {
        Some(MigrationPaths {
            legacy_path: legacy_path.clone(),
            xdg_path: xdg_path.clone(),
        })
    };

    let _ = prompt_to_migrate_legacy_config_at_paths(migration, stdin_is_tty, input, error)?;

    if legacy_path.exists() && !xdg_path.exists() {
        return Ok(legacy_path);
    }

    if !xdg_path.exists() && !legacy_path.exists() {
        if let Some(parent) = xdg_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
        }
        write_config_file(&xdg_path, &starter_config())?;
    }

    Ok(detect_config_source_from_paths(xdg_path, legacy_path)
        .path()
        .to_path_buf())
}

pub fn config_path_for_editing() -> anyhow::Result<PathBuf> {
    let mut input = io::BufReader::new(io::stdin().lock());
    let mut error = io::stderr().lock();
    config_path_for_editing_at_paths(
        ClamorConfig::config_path()?,
        ClamorConfig::legacy_config_path()?,
        stdin_is_tty(),
        &mut input,
        &mut error,
    )
}

fn detect_config_source_from_paths(config_path: PathBuf, legacy_path: PathBuf) -> ConfigSource {
    if config_path.exists() {
        ConfigSource::XdgYaml(config_path)
    } else if legacy_path.exists() {
        ConfigSource::LegacyJson(legacy_path)
    } else {
        ConfigSource::XdgYaml(config_path)
    }
}

fn deserialize_folders<'de, D>(deserializer: D) -> Result<HashMap<String, FolderConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = HashMap::<String, FolderConfigDef>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(name, entry)| {
            let folder = match entry {
                FolderConfigDef::LegacyPath(path) => FolderConfig {
                    path,
                    backends: Vec::new(),
                },
                FolderConfigDef::Structured(folder) => folder,
            };
            (name, folder)
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WatchMode {
    #[default]
    Fsevents,
    Poll,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalAction {
    /// Detach from the agent and return to the dashboard.
    Detach,
    /// Send SIGINT to the agent (via the daemon, not raw 0x03).
    Sigint,
    /// Jump to the next agent in `Input` state.
    JumpInputNext,
    /// Jump to the previous agent in `Input` state.
    JumpInputPrev,
    /// Snap the live view to the bottom of the scrollback.
    SnapToBottom,
    /// Enter vim-style copy mode.
    EnterCopyMode,
    /// Rebuild the daemon parser from the ring buffer.
    RefreshParser,
}

/// Bindings for the attached-terminal mode. Any `Ctrl+<key>` (and similar)
/// combo not present in `bindings` is forwarded to the agent's PTY.
///
/// Absence of the entire `terminal:` section yields an empty map; the keymap
/// resolver then falls back to the built-in defaults. Presence of `bindings:`
/// (even an empty map) means "the user is taking over" — only listed combos
/// are intercepted.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TerminalConfig {
    #[serde(default)]
    pub bindings: HashMap<String, TerminalAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: f64,
    #[serde(default)]
    pub watch_mode: WatchMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Background for selected rows. Tune to be slightly lighter than your terminal bg.
    #[serde(default = "defaults::highlight")]
    pub highlight: RgbColor,
    /// Accent color: keybinding hints, borders, selection marker.
    #[serde(default = "defaults::accent")]
    pub accent: RgbColor,
    /// Status: working agents.
    #[serde(default = "defaults::status_working")]
    pub status_working: RgbColor,
    /// Status: agents waiting for input.
    #[serde(default = "defaults::status_input")]
    pub status_input: RgbColor,
    /// Status: finished agents.
    #[serde(default = "defaults::status_done")]
    pub status_done: RgbColor,
    /// Secondary text: durations, metadata.
    #[serde(default = "defaults::dimmed")]
    pub dimmed: RgbColor,
    /// Batch selection marker color.
    #[serde(default = "defaults::batch_marker")]
    pub batch_marker: RgbColor,
    /// Background for kill-targeted agent row.
    #[serde(default = "defaults::kill_highlight")]
    pub kill_highlight: RgbColor,
}

impl RgbColor {
    pub fn to_ratatui(self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(self.0[0], self.0[1], self.0[2])
    }
}

/// Tokyo Night color palette defaults.
mod defaults {
    use super::RgbColor;
    pub fn highlight() -> RgbColor {
        RgbColor([0x28, 0x34, 0x57])
    } // #283457
    pub fn accent() -> RgbColor {
        RgbColor([0x7d, 0xcf, 0xff])
    } // #7dcfff
    pub fn status_working() -> RgbColor {
        RgbColor([0x9e, 0xce, 0x6a])
    } // #9ece6a
    pub fn status_input() -> RgbColor {
        RgbColor([0xe0, 0xaf, 0x68])
    } // #e0af68
    pub fn status_done() -> RgbColor {
        RgbColor([0x56, 0x5f, 0x89])
    } // #565f89
    pub fn dimmed() -> RgbColor {
        RgbColor([0x56, 0x5f, 0x89])
    } // #565f89
    pub fn batch_marker() -> RgbColor {
        RgbColor([0xe0, 0xaf, 0x68])
    } // #e0af68
    pub fn kill_highlight() -> RgbColor {
        RgbColor([0x3b, 0x20, 0x30])
    } // #3b2030
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            highlight: defaults::highlight(),
            accent: defaults::accent(),
            status_working: defaults::status_working(),
            status_input: defaults::status_input(),
            status_done: defaults::status_done(),
            dimmed: defaults::dimmed(),
            batch_marker: defaults::batch_marker(),
            kill_highlight: defaults::kill_highlight(),
        }
    }
}

fn default_refresh_interval() -> f64 {
    1.0
}

/// RGB color that deserializes from either "#rrggbb" or [r, g, b].
#[derive(Debug, Clone, Copy)]
pub struct RgbColor(pub [u8; 3]);

impl Serialize for RgbColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!(
            "#{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2]
        ))
    }
}

impl<'de> Deserialize<'de> for RgbColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = RgbColor;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "hex color \"#rrggbb\" or RGB array [r, g, b]")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<RgbColor, E> {
                let hex = v.strip_prefix('#').unwrap_or(v);
                if hex.len() != 6 {
                    return Err(E::custom(format!(
                        "expected 6 hex digits, got {}",
                        hex.len()
                    )));
                }
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(E::custom)?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(E::custom)?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(E::custom)?;
                Ok(RgbColor([r, g, b]))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<RgbColor, A::Error> {
                let r = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &"3"))?;
                let g = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"3"))?;
                let b = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &"3"))?;
                Ok(RgbColor([r, g, b]))
            }
        }

        deserializer.deserialize_any(ColorVisitor)
    }
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            watch_mode: WatchMode::default(),
        }
    }
}

impl ClamorConfig {
    /// Returns `~/.config/clamor/` (or `$XDG_CONFIG_HOME/clamor/`).
    pub fn config_dir() -> anyhow::Result<PathBuf> {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(xdg).join("clamor"));
        }
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config").join("clamor"))
    }

    /// Returns the legacy runtime dir `~/.clamor/`.
    pub fn runtime_dir() -> anyhow::Result<PathBuf> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".clamor"))
    }

    /// Returns the primary YAML config path.
    pub fn config_path() -> anyhow::Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.yaml"))
    }

    /// Returns the legacy JSON config path.
    pub fn legacy_config_path() -> anyhow::Result<PathBuf> {
        Ok(Self::runtime_dir()?.join("config.json"))
    }

    /// Creates the runtime dir if it doesn't exist.
    pub fn ensure_runtime_dir() -> anyhow::Result<()> {
        let dir = Self::runtime_dir()?;
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create runtime dir: {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn folder_path(&self, folder_id: &str) -> Option<&str> {
        self.folders
            .get(folder_id)
            .map(|folder| folder.path.as_str())
    }

    pub fn folder_backends(&self, folder_id: &str) -> Option<&[String]> {
        self.folders
            .get(folder_id)
            .map(|folder| folder.backends.as_slice())
    }

    pub fn backend_display_name<'a>(&'a self, backend_id: &'a str) -> &'a str {
        self.backends
            .get(backend_id)
            .and_then(|backend| {
                if backend.display_name.is_empty() {
                    None
                } else {
                    Some(backend.display_name.as_str())
                }
            })
            .unwrap_or(backend_id)
    }

    pub fn ordered_folders(&self) -> Vec<(String, String)> {
        let mut folders: Vec<(String, String)> = self
            .folders
            .iter()
            .map(|(id, folder)| (id.clone(), folder.path.clone()))
            .collect();
        folders.sort_by(|a, b| a.0.cmp(&b.0));
        folders
    }

    pub fn load_with_source() -> anyhow::Result<LoadedConfig> {
        let config_path = Self::config_path()?;
        let legacy_path = Self::legacy_config_path()?;
        let source = detect_config_source_from_paths(config_path, legacy_path);

        let config = match &source {
            ConfigSource::XdgYaml(path) if path.exists() => load_config_from_path(path, &source)?,
            ConfigSource::LegacyJson(path) => load_config_from_path(path, &source)?,
            ConfigSource::XdgYaml(path) => {
                let config = starter_config();
                validate_config(&config)?;
                write_config_file(path, &config)?;
                config
            }
        };

        Ok(LoadedConfig { config, source })
    }

    /// Loads config from XDG YAML first, then legacy JSON.
    /// Creates a default config file if it doesn't exist.
    pub fn load() -> anyhow::Result<Self> {
        Ok(Self::load_with_source()?.config)
    }
}

/// Resolve a folder path, expanding `~`.
pub fn resolve_path(path: &str) -> PathBuf {
    if path.starts_with('~') {
        std::env::var("HOME")
            .map(|home| PathBuf::from(path.replacen('~', &home, 1)))
            .unwrap_or_else(|_| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let unique = format!(
            "clamor-config-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(unique).join(name)
    }

    #[test]
    fn parses_terminal_bindings_from_yaml() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
terminal:
  bindings:
    ctrl-f: detach
    ctrl-c: sigint
    ctrl-r: refresh-parser
"#,
        )
        .unwrap();

        let bindings = &config.terminal.bindings;
        assert_eq!(bindings.get("ctrl-f"), Some(&TerminalAction::Detach));
        assert_eq!(bindings.get("ctrl-c"), Some(&TerminalAction::Sigint));
        assert_eq!(
            bindings.get("ctrl-r"),
            Some(&TerminalAction::RefreshParser)
        );
    }

    #[test]
    fn terminal_config_defaults_to_empty_bindings_map() {
        let config: ClamorConfig = serde_yaml::from_str("{}").unwrap();
        assert!(
            config.terminal.bindings.is_empty(),
            "absence of `terminal:` should yield an empty bindings map; \
             defaults are applied at keymap-resolution time"
        );
    }

    #[test]
    fn terminal_action_round_trips_via_yaml() {
        for (s, expected) in [
            ("detach", TerminalAction::Detach),
            ("sigint", TerminalAction::Sigint),
            ("jump-input-next", TerminalAction::JumpInputNext),
            ("jump-input-prev", TerminalAction::JumpInputPrev),
            ("snap-to-bottom", TerminalAction::SnapToBottom),
            ("enter-copy-mode", TerminalAction::EnterCopyMode),
            ("refresh-parser", TerminalAction::RefreshParser),
        ] {
            let parsed: TerminalAction =
                serde_yaml::from_str(s).unwrap_or_else(|e| panic!("parse {s}: {e}"));
            assert_eq!(parsed, expected, "string `{s}` should parse to {expected:?}");
        }
    }

    #[test]
    fn parses_legacy_folder_map_with_empty_backends() {
        let config: ClamorConfig = serde_json::from_str(
            r#"{
                "folders": {
                    "work": "~/work"
                }
            }"#,
        )
        .unwrap();

        let folder = config.folders.get("work").unwrap();
        assert_eq!(folder.path, "~/work");
        assert!(folder.backends.is_empty());
    }

    #[test]
    fn parses_backend_registry_and_structured_folders() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
backends:
  claude-code:
    display_name: Claude
    spawn:
      cmd: [claude, "{{prompt}}"]
    capabilities:
      resume: true
folders:
  work:
    path: ~/work
    backends: [claude-code]
"#,
        )
        .unwrap();
        validate_config(&config).unwrap();

        assert_eq!(config.folders["work"].backends, vec!["claude-code"]);
        assert_eq!(config.backends["claude-code"].display_name, "Claude");
        assert_eq!(
            config.backends["claude-code"].spawn.cmd,
            vec!["claude", "{{prompt}}"]
        );
    }

    #[test]
    fn rejects_unknown_folder_backend_ids() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
folders:
  work:
    path: ~/work
    backends: [missing]
"#,
        )
        .unwrap();

        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("unknown backend 'missing'"));
    }

    #[test]
    fn rejects_folder_with_empty_backends() {
        let config: ClamorConfig = serde_yaml::from_str(
            r#"
folders:
  work:
    path: ~/work
"#,
        )
        .unwrap();

        let err = validate_config(&config).unwrap_err();
        assert!(err.to_string().contains("must define at least one backend"));
    }

    #[test]
    fn prefers_legacy_source_when_xdg_missing() {
        let source = detect_config_source_from_paths(
            PathBuf::from("/tmp/test-config.yaml"),
            PathBuf::from("/tmp/test-legacy.json"),
        );

        assert_eq!(
            source,
            ConfigSource::XdgYaml(PathBuf::from("/tmp/test-config.yaml"))
        );
    }

    #[test]
    fn chooses_legacy_when_only_legacy_exists() {
        let temp_root = temp_path("root");
        std::fs::create_dir_all(&temp_root).unwrap();
        let config_path = temp_root.join("config.yaml");
        let legacy_path = temp_root.join("config.json");
        std::fs::write(&legacy_path, "{}").unwrap();

        let source = detect_config_source_from_paths(config_path.clone(), legacy_path.clone());
        assert_eq!(source, ConfigSource::LegacyJson(legacy_path));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn init_writes_empty_starter_config() {
        let temp_root = temp_path("init");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");
        let legacy_path = temp_root.join(".clamor").join("config.json");

        let written_path = init_config_at_paths(&xdg_path, &legacy_path).unwrap();
        assert_eq!(written_path, xdg_path);

        let contents = std::fs::read_to_string(&written_path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&contents).unwrap();
        assert!(parsed["backends"].as_mapping().unwrap().is_empty());
        assert!(parsed["folders"].as_mapping().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }

    #[test]
    fn legacy_config_with_non_interactive_input_does_not_migrate_implicitly() {
        let temp_root = temp_path("noninteractive");
        let legacy_path = temp_root.join(".clamor").join("config.json");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(&legacy_path, r#"{"folders":{"work":"~/work"}}"#).unwrap();

        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut error = Vec::new();
        let migrated = prompt_to_migrate_legacy_config_at_paths(
            Some(MigrationPaths {
                legacy_path: legacy_path.clone(),
                xdg_path: xdg_path.clone(),
            }),
            false,
            &mut input,
            &mut error,
        )
        .unwrap();

        assert!(!migrated);
        assert!(legacy_path.exists());
        assert!(!xdg_path.exists());

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }

    #[test]
    fn config_path_for_editing_prefers_legacy_when_xdg_missing() {
        let temp_root = temp_path("config-open-legacy");
        let legacy_path = temp_root.join(".clamor").join("config.json");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(&legacy_path, r#"{"folders":{"work":"~/work"}}"#).unwrap();

        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut error = Vec::new();
        let selected = config_path_for_editing_at_paths(
            xdg_path.clone(),
            legacy_path.clone(),
            false,
            &mut input,
            &mut error,
        )
        .unwrap();

        assert_eq!(selected, legacy_path);
        assert!(!xdg_path.exists());

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }

    #[test]
    fn config_path_for_editing_prefers_xdg_when_present() {
        let temp_root = temp_path("config-open-xdg");
        let legacy_path = temp_root.join(".clamor").join("config.json");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(xdg_path.parent().unwrap()).unwrap();
        std::fs::write(&legacy_path, r#"{"folders":{"legacy":"~/legacy"}}"#).unwrap();
        std::fs::write(&xdg_path, "folders: {}\nbackends: {}\n").unwrap();

        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut error = Vec::new();
        let selected = config_path_for_editing_at_paths(
            xdg_path.clone(),
            legacy_path.clone(),
            false,
            &mut input,
            &mut error,
        )
        .unwrap();

        assert_eq!(selected, xdg_path);

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }

    #[test]
    fn config_path_for_editing_materializes_empty_starter_for_clean_home() {
        let temp_root = temp_path("config-open-clean");
        let legacy_path = temp_root.join(".clamor").join("config.json");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");

        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut error = Vec::new();
        let selected = config_path_for_editing_at_paths(
            xdg_path.clone(),
            legacy_path.clone(),
            false,
            &mut input,
            &mut error,
        )
        .unwrap();

        assert_eq!(selected, xdg_path.clone());
        assert!(xdg_path.exists());
        assert!(!legacy_path.exists());

        let contents = std::fs::read_to_string(&xdg_path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&contents).unwrap();
        assert!(parsed["backends"].as_mapping().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }

    #[test]
    fn example_config_has_inline_backends() {
        let yaml = serialize_config_yaml(&example_config()).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();

        assert!(parsed["backends"]["claude-code"].is_mapping());
        assert!(parsed["backends"]["open-code"].is_mapping());
        assert_eq!(
            parsed["folders"]["work"]["backends"],
            serde_yaml::to_value(vec!["claude-code", "open-code"] as Vec<&str>).unwrap()
        );
    }

    #[test]
    fn migrate_preserves_explicit_backends_and_rejects_folders_without_backends() {
        let temp_root = temp_path("migration");
        let legacy_path = temp_root.join(".clamor").join("config.json");
        let xdg_path = temp_root.join(".config").join("clamor").join("config.yaml");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy_path,
            r#"{
                "folders": {}
            }"#,
        )
        .unwrap();

        let outcome = migrate_legacy_config_at_paths(&MigrationPaths {
            legacy_path: legacy_path.clone(),
            xdg_path: xdg_path.clone(),
        })
        .unwrap();

        assert_eq!(outcome.from, legacy_path);
        assert_eq!(outcome.to, xdg_path.clone());
        assert!(xdg_path.exists());
        assert!(legacy_path.exists());

        let migrated = std::fs::read_to_string(&xdg_path).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&migrated).unwrap();
        assert!(parsed["folders"].as_mapping().unwrap().is_empty());
        assert!(parsed["backends"].as_mapping().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(temp_root.parent().unwrap_or(&temp_root));
    }
}
