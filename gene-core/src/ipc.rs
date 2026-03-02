use crate::SharedState;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Commands accepted from gene-ctl over the Unix socket.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd")]
pub enum IpcCommand {
    Status,
    Signals,
    Symbols,
    Identity,
    Inject { signal_id: u32, delta: f64 },
    Pause,
    Resume,
    Checkpoint,
    Expressions { n: Option<u32> },
}

/// Responses sent back to gene-ctl.
#[derive(Debug, Serialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub data: serde_json::Value,
}

pub async fn serve(socket_path: PathBuf, state: Arc<Mutex<SharedState>>) -> Result<()> {
    // Remove stale socket
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("IPC listening on {:?}", socket_path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, state).await {
                        tracing::debug!("IPC client error: {}", e);
                    }
                });
            }
            Err(e) => {
                tracing::warn!("IPC accept error: {}", e);
            }
        }
    }
}

async fn handle_client(mut stream: UnixStream, state: Arc<Mutex<SharedState>>) -> Result<()> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let line = line.trim();

    let cmd: IpcCommand = match serde_json::from_str(line) {
        Ok(c) => c,
        Err(e) => {
            let resp = IpcResponse {
                ok: false,
                data: serde_json::json!({ "error": format!("parse error: {}", e) }),
            };
            let s = serde_json::to_string(&resp)? + "\n";
            writer.write_all(s.as_bytes()).await?;
            return Ok(());
        }
    };

    let resp = {
        let mut st = state.lock().unwrap();
        match cmd {
            IpcCommand::Status => IpcResponse {
                ok: true,
                data: serde_json::json!({
                    "tick": st.tick,
                    "imbalance": st.imbalance,
                    "last_action": st.last_action,
                    "pattern_count": st.pattern_count,
                    "symbol_count": st.symbol_count,
                    "composite_count": st.composite_count,
                    "action_count": st.action_count,
                    "confidence": st.confidence,
                    "paused": st.pause_requested,
                }),
            },
            IpcCommand::Symbols => IpcResponse {
                ok: true,
                data: serde_json::json!({
                    "active_symbols": st.active_symbols,
                }),
            },
            IpcCommand::Identity => IpcResponse {
                ok: true,
                data: serde_json::json!({ "identity": st.identity }),
            },
            IpcCommand::Inject { signal_id, delta } => {
                st.pending_inject = Some((crate::signal::types::SignalId(signal_id), delta));
                IpcResponse {
                    ok: true,
                    data: serde_json::json!({ "queued": true }),
                }
            }
            IpcCommand::Pause => {
                st.pause_requested = true;
                IpcResponse { ok: true, data: serde_json::json!({ "paused": true }) }
            }
            IpcCommand::Resume => {
                st.pause_requested = false;
                IpcResponse { ok: true, data: serde_json::json!({ "resumed": true }) }
            }
            IpcCommand::Checkpoint => IpcResponse {
                ok: true,
                data: serde_json::json!({ "checkpoint": "will occur at next scheduled tick" }),
            },
            IpcCommand::Signals => IpcResponse {
                ok: true,
                data: serde_json::json!({
                    "note": "use gene-ctl status for signal summary; full signal dump requires direct ledger read",
                }),
            },
            IpcCommand::Expressions { n } => {
                let n = n.unwrap_or(20) as usize;
                let exprs: Vec<_> = st.recent_expressions.iter().rev().take(n).cloned().collect::<Vec<_>>()
                    .into_iter().rev().collect();
                IpcResponse {
                    ok: true,
                    data: serde_json::json!({ "expressions": exprs }),
                }
            }
        }
    };

    let s = serde_json::to_string(&resp)? + "\n";
    writer.write_all(s.as_bytes()).await?;
    Ok(())
}
