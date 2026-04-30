//! Unix-socket IPC server for the organism daemon.
//!
//! Wire format: newline-delimited JSON Envelopes.
//! One request per connection; daemon writes one response Envelope and closes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

use organism_protocol::{Envelope, OrganismEvent, SuggestRequest, SuggestResponse};

use crate::daemon::DaemonState;
use crate::event_bus::EventBus;
use organism_knowledge::KnowledgeStore;
use tokio::sync::RwLock;

/// Bind a Unix socket at `socket_path` and serve incoming RPC requests.
/// Cleans up any stale socket file before binding.
pub async fn serve(
    state: Arc<RwLock<DaemonState>>,
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
    socket_path: PathBuf,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating socket parent dir {:?}", parent))?;
    }

    // Remove stale socket if it exists.
    if socket_path.exists() {
        let _ = tokio::fs::remove_file(&socket_path).await;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding unix socket at {:?}", socket_path))?;
    info!(socket = ?socket_path, "IPC listener bound");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state_clone = state.clone();
                let bus_clone = bus.clone();
                let knowledge_clone = knowledge.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(state_clone, bus_clone, knowledge_clone, stream).await
                    {
                        warn!(error = %e, "ipc connection error");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "accept failed");
            }
        }
    }
}

async fn handle_connection(
    state: Arc<RwLock<DaemonState>>,
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
    stream: UnixStream,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(());
    }

    debug!(line = %line.trim(), "ipc request");

    let response = match serde_json::from_str::<Envelope>(line.trim()) {
        Ok(env) => dispatch(state, bus, knowledge, env).await,
        Err(e) => Envelope::error_response("0", &format!("invalid envelope: {}", e)),
    };

    let mut payload = serde_json::to_string(&response)?;
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).await?;
    write_half.shutdown().await.ok();
    Ok(())
}

async fn dispatch(
    state: Arc<RwLock<DaemonState>>,
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
    req: Envelope,
) -> Envelope {
    // Extract method from request payload (Envelope::request format).
    let method = req
        .payload
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    match method.as_str() {
        "status" => {
            let s = state.read().await;
            let body = serde_json::json!({
                "uptime_secs": s.uptime_secs(),
                "awake": s.awake,
                "event_count": s.event_count,
            });
            Envelope::ok_response(&req.id, body)
        }
        "sleep" => {
            let mut s = state.write().await;
            s.awake = false;
            Envelope::ok_response(&req.id, serde_json::json!({"awake": false}))
        }
        "wake" => {
            let mut s = state.write().await;
            s.awake = true;
            Envelope::ok_response(&req.id, serde_json::json!({"awake": true}))
        }
        "log" => {
            let s = state.read().await;
            let arr: Vec<serde_json::Value> = s
                .recent_events
                .iter()
                .map(|e| serde_json::json!({"ts": e.ts, "msg": e.msg}))
                .collect();
            Envelope::ok_response(&req.id, serde_json::Value::Array(arr))
        }
        "suggest" => {
            let params = req
                .payload
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let req_data: SuggestRequest =
                serde_json::from_value(params).unwrap_or(SuggestRequest { error_key: None });
            let mut store = knowledge.write().await;

            // Resolve error key: explicit, or most-recent error by last_seen
            let key = match req_data.error_key {
                Some(k) => Some(k),
                None => store
                    .list_errors()
                    .ok()
                    .and_then(|errs| errs.into_iter().max_by_key(|e| e.last_seen).map(|e| e.hash)),
            };

            let Some(key) = key else {
                return Envelope::ok_response(
                    &req.id,
                    serde_json::to_value(SuggestResponse {
                        text: "(no errors recorded yet)".to_string(),
                        cached: false,
                    })
                    .unwrap(),
                );
            };

            let (text, cached) = match store.get_suggestion(&key) {
                Ok(Some(t)) => (t, true),
                _ => (String::new(), false),
            };

            Envelope::ok_response(
                &req.id,
                serde_json::to_value(SuggestResponse { text, cached }).unwrap(),
            )
        }
        "event" => {
            // params should be a serialized OrganismEvent.
            let params = req
                .payload
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let evt: OrganismEvent = match serde_json::from_value(params) {
                Ok(e) => e,
                Err(e) => {
                    return Envelope::error_response(
                        &req.id,
                        &format!("invalid event payload: {}", e),
                    );
                }
            };

            // Honor sleep state: ack but skip recording.
            let awake = { state.read().await.awake };
            if !awake {
                return Envelope::ok_response(
                    &req.id,
                    serde_json::json!({"ok": true, "recorded": false}),
                );
            }

            // Record on the producer side, then publish for downstream processors.
            {
                let mut s = state.write().await;
                s.record_event(format!("{:?}", evt));
            }
            let _ = bus.publish(evt);
            Envelope::ok_response(&req.id, serde_json::json!({"ok": true, "recorded": true}))
        }
        _ => Envelope::error_response(
            &req.id,
            &format!(
                "unknown method: {}",
                if method.is_empty() { "<none>" } else { &method }
            ),
        ),
    }
}

/// Resolve the on-disk socket path under the organism data directory.
pub fn socket_path_for(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_appends_filename() {
        let p = socket_path_for(Path::new("/tmp/x"));
        assert_eq!(p, PathBuf::from("/tmp/x/daemon.sock"));
    }
}
