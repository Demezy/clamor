use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::UnixStream;

use crate::protocol::{
    recv_message_async, send_message_async, ClientMessage, DaemonAgent, DaemonMessage,
};

pub struct SubscribeResult {
    pub catch_up: Vec<u8>,
    pub buffered: Vec<DaemonMessage>,
}

pub struct DaemonClient {
    stream: UnixStream,
}

impl DaemonClient {
    pub async fn connect() -> Result<Self> {
        let path = crate::daemon::daemon_socket_path()?;
        let stream = UnixStream::connect(&path)
            .await
            .with_context(|| format!("connecting to daemon at {}", path.display()))?;
        Ok(Self { stream })
    }

    pub async fn spawn_agent(
        &mut self,
        id: &str,
        cwd: &str,
        cmd: &[String],
        env: &[(String, String)],
        rows: u16,
        cols: u16,
    ) -> Result<()> {
        self.send(ClientMessage::Spawn {
            id: id.to_string(),
            cwd: cwd.to_string(),
            cmd: cmd.to_vec(),
            env: env.to_vec(),
            rows,
            cols,
        })
        .await?;
        self.expect_ok().await
    }

    pub async fn kill_agent(&mut self, id: &str) -> Result<()> {
        self.send(ClientMessage::Kill { id: id.to_string() })
            .await?;
        self.expect_ok().await
    }

    pub async fn send_sigint(&mut self, id: &str) -> Result<()> {
        self.send(ClientMessage::Sigint { id: id.to_string() })
            .await?;
        self.expect_ok().await
    }

    pub async fn send_input(&mut self, id: &str, data: &[u8]) -> Result<()> {
        self.send(ClientMessage::Input {
            id: id.to_string(),
            data: data.to_vec(),
        })
        .await
    }

    pub async fn resize(&mut self, id: &str, rows: u16, cols: u16) -> Result<()> {
        self.send(ClientMessage::Resize {
            id: id.to_string(),
            rows,
            cols,
        })
        .await?;
        self.expect_ok().await
    }

    /// Like resize(), but returns any in-flight Output/Exited messages that
    /// arrived while waiting for the OK response, instead of discarding them.
    pub async fn resize_buffered(
        &mut self,
        id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<Vec<DaemonMessage>> {
        self.send(ClientMessage::Resize {
            id: id.to_string(),
            rows,
            cols,
        })
        .await?;
        self.expect_ok_buffered().await
    }

    pub async fn subscribe(&mut self, id: &str) -> Result<Vec<u8>> {
        let result = self.subscribe_buffered(id).await?;
        Ok(result.catch_up)
    }

    /// Like subscribe(), but returns any in-flight Output/Exited messages that
    /// arrived while waiting for the CatchUp response, instead of discarding them.
    pub async fn subscribe_buffered(&mut self, id: &str) -> Result<SubscribeResult> {
        self.send(ClientMessage::Subscribe { id: id.to_string() })
            .await?;
        let mut buffered = Vec::new();
        loop {
            let msg: DaemonMessage =
                tokio::time::timeout(Duration::from_secs(5), recv_message_async(&mut self.stream))
                    .await
                    .context("subscribe timed out")??;

            match msg {
                DaemonMessage::CatchUp { data, .. } => {
                    return Ok(SubscribeResult {
                        catch_up: data,
                        buffered,
                    });
                }
                DaemonMessage::Error { message } => {
                    anyhow::bail!("subscribe failed: {message}")
                }
                DaemonMessage::Output { .. } | DaemonMessage::Exited { .. } => {
                    buffered.push(msg);
                }
                DaemonMessage::Heartbeat => continue,
                other => {
                    anyhow::bail!("unexpected response: {other:?}")
                }
            }
        }
    }

    /// Ask daemon to rebuild its parser from the ring buffer and send fresh
    /// catch-up data. Fixes accumulated rendering issues. Also ensures the
    /// subscription is active (same as subscribe).
    pub async fn refresh_parser_buffered(&mut self, id: &str) -> Result<SubscribeResult> {
        self.send(ClientMessage::RefreshParser { id: id.to_string() })
            .await?;
        let mut buffered = Vec::new();
        loop {
            let msg: DaemonMessage =
                tokio::time::timeout(Duration::from_secs(5), recv_message_async(&mut self.stream))
                    .await
                    .context("refresh_parser timed out")??;

            match msg {
                DaemonMessage::CatchUp { data, .. } => {
                    return Ok(SubscribeResult {
                        catch_up: data,
                        buffered,
                    });
                }
                DaemonMessage::Error { message } => {
                    anyhow::bail!("refresh_parser failed: {message}")
                }
                DaemonMessage::Output { .. } | DaemonMessage::Exited { .. } => {
                    buffered.push(msg);
                }
                DaemonMessage::Heartbeat => continue,
                other => {
                    anyhow::bail!("unexpected response: {other:?}")
                }
            }
        }
    }

    pub async fn unsubscribe(&mut self, id: &str) -> Result<()> {
        self.send(ClientMessage::Unsubscribe { id: id.to_string() })
            .await?;
        self.expect_ok().await
    }

    pub async fn list_agents(&mut self) -> Result<Vec<DaemonAgent>> {
        self.send(ClientMessage::List).await?;
        loop {
            let msg: DaemonMessage =
                tokio::time::timeout(Duration::from_secs(5), recv_message_async(&mut self.stream))
                    .await
                    .context("list timed out")??;

            match msg {
                DaemonMessage::AgentList { agents } => return Ok(agents),
                DaemonMessage::Error { message } => {
                    anyhow::bail!("list failed: {message}")
                }
                DaemonMessage::Output { .. }
                | DaemonMessage::Exited { .. }
                | DaemonMessage::Heartbeat => continue,
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.send(ClientMessage::Shutdown).await?;
        self.expect_ok().await
    }

    pub async fn recv(&mut self) -> Result<DaemonMessage> {
        recv_message_async(&mut self.stream).await
    }

    pub async fn pong(&mut self) -> Result<()> {
        self.send(ClientMessage::Pong).await
    }

    async fn send(&mut self, msg: ClientMessage) -> Result<()> {
        send_message_async(&mut self.stream, &msg).await
    }

    async fn expect_ok(&mut self) -> Result<()> {
        let _ = self.expect_ok_buffered().await?;
        Ok(())
    }

    async fn expect_ok_buffered(&mut self) -> Result<Vec<DaemonMessage>> {
        let mut buffered = Vec::new();
        loop {
            let msg: DaemonMessage =
                tokio::time::timeout(Duration::from_secs(5), recv_message_async(&mut self.stream))
                    .await
                    .context("expect_ok timed out")??;

            match msg {
                DaemonMessage::Ok => return Ok(buffered),
                DaemonMessage::Error { message } => anyhow::bail!("{message}"),
                DaemonMessage::Output { .. } | DaemonMessage::Exited { .. } => {
                    buffered.push(msg);
                }
                DaemonMessage::Heartbeat => continue,
                other => anyhow::bail!("unexpected response: {other:?}"),
            }
        }
    }
}
