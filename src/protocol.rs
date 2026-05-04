use std::io::{Read, Write};

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn default_rows() -> u16 {
    24
}
fn default_cols() -> u16 {
    80
}

/// Messages sent from client to daemon over the Unix domain socket.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Spawn a new PTY process
    Spawn {
        id: String,
        cwd: String,
        cmd: Vec<String>,
        env: Vec<(String, String)>,
        #[serde(default = "default_rows")]
        rows: u16,
        #[serde(default = "default_cols")]
        cols: u16,
    },
    /// Kill a PTY process
    Kill { id: String },
    /// Send SIGINT to the foreground process group
    Sigint { id: String },
    /// Send raw input bytes to PTY
    Input { id: String, data: Vec<u8> },
    /// Resize a PTY
    Resize { id: String, rows: u16, cols: u16 },
    /// Subscribe to PTY output for an agent
    Subscribe { id: String },
    /// Unsubscribe from PTY output
    Unsubscribe { id: String },
    /// Rebuild daemon-side parser from ring buffer and send fresh catch-up.
    /// Fixes accumulated rendering issues without restarting the session.
    RefreshParser { id: String },
    /// List all managed PTYs and their status
    List,
    /// Shut down the daemon
    Shutdown,
    /// Version handshake on connect
    Hello { version: String },
    /// Response to daemon heartbeat
    Pong,
}

/// Messages sent from daemon to client over the Unix domain socket.
#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonMessage {
    /// PTY output bytes
    Output { id: String, data: Vec<u8> },
    /// PTY process exited
    Exited { id: String },
    /// Response to List
    AgentList { agents: Vec<DaemonAgent> },
    /// Success response
    Ok,
    /// Error response
    Error { message: String },
    /// Catch-up buffer sent when a client first subscribes to an agent
    CatchUp { id: String, data: Vec<u8> },
    /// Version handshake response
    Hello { version: String },
    /// Liveness check — client should respond with Pong
    Heartbeat,
}

/// Minimal agent info tracked by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonAgent {
    pub id: String,
    pub alive: bool,
}

/// Send a length-prefixed JSON message over a writer.
///
/// Wire format: 4-byte big-endian length prefix followed by JSON bytes.
#[allow(dead_code)]
pub fn send_message<W: Write>(writer: &mut W, msg: &impl Serialize) -> Result<()> {
    let json = serde_json::to_vec(msg).context("serializing message")?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).context("writing length prefix")?;
    writer.write_all(&json).context("writing message body")?;
    writer.flush().context("flushing message")?;
    Ok(())
}

/// Read a length-prefixed JSON message from a reader.
///
/// Wire format: 4-byte big-endian length prefix followed by JSON bytes.
#[allow(dead_code)]
pub fn recv_message<T: DeserializeOwned, R: Read>(reader: &mut R) -> Result<T> {
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf) {
        return Err(map_recv_error(e, "length prefix"));
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    if let Err(e) = reader.read_exact(&mut buf) {
        return Err(map_recv_error(e, "message body"));
    }

    serde_json::from_slice(&buf).context("deserializing message")
}

/// Async variant of `send_message`.
pub async fn send_message_async<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &impl Serialize,
) -> Result<()> {
    let json = serde_json::to_vec(msg).context("serializing message")?;
    let len = (json.len() as u32).to_be_bytes();
    writer
        .write_all(&len)
        .await
        .context("writing length prefix")?;
    writer
        .write_all(&json)
        .await
        .context("writing message body")?;
    writer.flush().await.context("flushing message")?;
    Ok(())
}

/// Async variant of `recv_message`.
pub async fn recv_message_async<T: DeserializeOwned, R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<T> {
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf).await {
        return Err(map_recv_error(e, "length prefix"));
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    if let Err(e) = reader.read_exact(&mut buf).await {
        return Err(map_recv_error(e, "message body"));
    }

    serde_json::from_slice(&buf).context("deserializing message")
}

/// Map an io error from a recv into an actionable anyhow error.
///
/// `UnexpectedEof` here almost always means the daemon dropped our connection —
/// either because another clamor process connected (the daemon only holds one
/// client at a time) or because the daemon exited. Surface that explicitly so
/// users don't have to puzzle out a bare "early eof".
fn map_recv_error(e: std::io::Error, stage: &'static str) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::UnexpectedEof {
        return anyhow::anyhow!(
            "daemon connection closed mid-message (reading {stage}). \
             Likely another clamor process connected, or the daemon exited. \
             See ~/.clamor/daemon.log."
        );
    }
    anyhow::Error::new(e).context(format!("reading {stage}"))
}
