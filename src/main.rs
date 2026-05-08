mod agent;
mod cli;
mod client;
mod config;
mod daemon;
mod dashboard;
mod mock_agent;
mod pane;
mod picker;
mod protocol;
mod set_state;
mod spawn;
mod state;
mod watcher;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, ConfigCommand, SetStateArg};
use config::ClamorConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let _ = config::prompt_to_migrate_legacy_config()?;
            let config = ClamorConfig::load()?;
            if config.folders.is_empty() {
                eprintln!(
                    "Error: No folders configured. Run `clamor config` to edit folders or `clamor config print-example` for a starter template."
                );
                std::process::exit(1);
            }
            dashboard::run(&config, None).await?;
        }
        Some(Command::Ls) => {
            spawn::list_agents()?;
        }
        Some(Command::Attach { agent_ref }) => {
            let config = ClamorConfig::load()?;
            let state = state::ClamorState::load()?;
            let agent = spawn::resolve_agent(&state, &agent_ref)?;
            dashboard::run(&config, Some(agent.id.clone())).await?;
        }
        Some(Command::New {
            title,
            description,
            folder,
        }) => {
            let effective_desc = match (title, description) {
                (Some(t), Some(d)) => Some(format!("{t}\n\n{d}")),
                (Some(t), None) => Some(t),
                (None, _) => None,
            };
            spawn::spawn_agent(effective_desc, folder, false).await?;
        }
        Some(Command::Adopt {
            session_id,
            description,
            folder,
        }) => {
            spawn::adopt_session(&session_id, description, folder).await?;
        }
        Some(Command::Edit {
            agent_ref,
            description,
        }) => {
            spawn::edit_agent(&agent_ref, description).await?;
        }
        Some(Command::Kill { all: true, .. }) => {
            spawn::kill_all_agents().await?;
        }
        Some(Command::Kill {
            agent_ref: Some(r), ..
        }) => {
            spawn::kill_agent(&r).await?;
        }
        Some(Command::Kill { .. }) => {
            unreachable!("clap enforces ref or --all");
        }
        Some(Command::Clean) => {
            spawn::clean_agents()?;
        }
        Some(Command::Config { command: None }) => {
            spawn::open_config()?;
        }
        Some(Command::Config {
            command: Some(ConfigCommand::Init),
        }) => {
            let path = config::init_config()?;
            println!("Initialized config at {}", path.display());
        }
        Some(Command::Config {
            command: Some(ConfigCommand::Migrate),
        }) => {
            let outcome = config::migrate_legacy_config()?;
            println!(
                "Migrated {} to {}",
                outcome.from.display(),
                outcome.to.display()
            );
            println!("Left the legacy file in place for safety.");
        }
        Some(Command::Config {
            command: Some(ConfigCommand::PrintExample),
        }) => {
            print!(
                "{}",
                config::serialize_config_yaml(&config::example_config())?
            );
        }
        Some(Command::DefaultTheme) => {
            let theme = config::ThemeConfig::default();
            println!("{}", serde_json::to_string_pretty(&theme)?);
        }
        Some(Command::SetState {
            state,
            agent,
            tool,
            session_token,
            activity_only,
        }) => {
            use agent::AgentState;
            let new_state = Some(match state {
                SetStateArg::Working => AgentState::Working,
                SetStateArg::Input => AgentState::Input,
                SetStateArg::Done => AgentState::Done,
            });
            set_state::run(set_state::SetStateArgs {
                agent_id: agent,
                new_state,
                tool,
                session_token,
                activity_only,
            });
        }
        Some(Command::PreUpgrade) => {
            if !spawn::pre_upgrade().await? {
                std::process::exit(1);
            }
        }
        Some(Command::Resume) => {
            spawn::resume_agents().await?;
        }
        Some(Command::Stop) => {
            let mut client = client::DaemonClient::connect().await?;
            client.shutdown().await?;
            println!("Clamor daemon stopped");
        }
        Some(Command::Daemon) => {
            daemon::run_daemon().await?;
        }
        Some(Command::MockAgent {
            description,
            duration,
        }) => {
            mock_agent::run(&description, duration);
        }
    }

    Ok(())
}
