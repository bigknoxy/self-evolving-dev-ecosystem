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

use organism_protocol::{
    ApplyMode, ApplyRequest, ApplyResponse, Envelope, ErrorSummaryWire, ErrorsRequest,
    ErrorsResponse, OrganismEvent, SuggestRequest, SuggestResponse,
};

use crate::clipboard;
use crate::daemon::DaemonState;
use crate::event_bus::EventBus;
use organism_cortex::apply::{extract_plan, ApplyPlan};
use organism_knowledge::KnowledgeStore;
use tokio::sync::RwLock;

fn is_safe_error_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_hexdigit())
}

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
                serde_json::from_value(params).unwrap_or(SuggestRequest {
                    error_key: None,
                    force: false,
                });
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
        "errors" => {
            let params = req
                .payload
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let req_data: ErrorsRequest =
                serde_json::from_value(params).unwrap_or(ErrorsRequest { limit: None });

            let store = knowledge.read().await;
            let limit = req_data.limit.unwrap_or(20);
            match store.list_errors_summary(limit) {
                Ok(summaries) => {
                    let items: Vec<ErrorSummaryWire> = summaries
                        .into_iter()
                        .map(|s| ErrorSummaryWire {
                            hash: s.hash,
                            command: s.last_command,
                            occurrences: s.occurrences,
                            last_seen: s
                                .last_seen
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            has_suggestion: s.has_suggestion,
                        })
                        .collect();
                    let resp = ErrorsResponse { items };
                    match serde_json::to_value(resp) {
                        Ok(v) => Envelope::ok_response(&req.id, v),
                        Err(e) => Envelope::error_response(
                            &req.id,
                            &format!("response serialize failed: {}", e),
                        ),
                    }
                }
                Err(e) => {
                    Envelope::error_response(&req.id, &format!("failed to list errors: {}", e))
                }
            }
        }
        "apply" => {
            let params = req
                .payload
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let req_data: ApplyRequest = match serde_json::from_value(params) {
                Ok(r) => r,
                Err(e) => {
                    return Envelope::error_response(
                        &req.id,
                        &format!("invalid apply request: {}", e),
                    );
                }
            };

            if !is_safe_error_key(&req_data.error_key) {
                return Envelope::error_response(
                    &req.id,
                    "invalid error_key (must be 1-64 hex chars)",
                );
            }

            let mut store = knowledge.write().await;
            let suggestion = match store.get_suggestion(&req_data.error_key) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    return Envelope::error_response(
                        &req.id,
                        "no cached suggestion for that error_key",
                    );
                }
                Err(e) => {
                    return Envelope::error_response(
                        &req.id,
                        &format!("knowledge read failed: {}", e),
                    );
                }
            };
            drop(store);

            let plan = extract_plan(&suggestion);
            let resp = build_apply_response(&plan, req_data.mode, &req_data.error_key);
            match serde_json::to_value(resp) {
                Ok(v) => Envelope::ok_response(&req.id, v),
                Err(e) => {
                    Envelope::error_response(&req.id, &format!("response serialize failed: {}", e))
                }
            }
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

fn build_apply_response(plan: &ApplyPlan, mode: ApplyMode, error_key: &str) -> ApplyResponse {
    match plan {
        ApplyPlan::Note { text } => ApplyResponse {
            plan_kind: "note".into(),
            artifact_path: None,
            clipboard: false,
            message: text.clone(),
        },
        ApplyPlan::Patch { diff } => match mode {
            ApplyMode::Dry => ApplyResponse {
                plan_kind: "patch".into(),
                artifact_path: None,
                clipboard: false,
                message: format!("diff (dry-run):\n\n{}", diff),
            },
            ApplyMode::Stage => {
                let path = std::env::temp_dir().join(format!("organism-{}.patch", error_key));
                match std::fs::write(&path, diff) {
                    Ok(_) => ApplyResponse {
                        plan_kind: "patch".into(),
                        artifact_path: Some(path.to_string_lossy().into_owned()),
                        clipboard: false,
                        message: format!("patch written. apply with: git apply {}", path.display()),
                    },
                    Err(e) => ApplyResponse {
                        plan_kind: "patch".into(),
                        artifact_path: None,
                        clipboard: false,
                        message: format!("failed to write patch: {}\n\n{}", e, diff),
                    },
                }
            }
        },
        ApplyPlan::Shell { command } => match mode {
            ApplyMode::Dry => ApplyResponse {
                plan_kind: "shell".into(),
                artifact_path: None,
                clipboard: false,
                message: format!("would run:\n{}", command),
            },
            ApplyMode::Stage => {
                let copied = clipboard::copy(command).unwrap_or(false);
                let message = if copied {
                    format!("copied to clipboard:\n{}", command)
                } else {
                    format!("clipboard unavailable. run manually:\n{}", command)
                };
                ApplyResponse {
                    plan_kind: "shell".into(),
                    artifact_path: None,
                    clipboard: copied,
                    message,
                }
            }
        },
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

    #[test]
    fn safe_error_key_accepts_hex() {
        assert!(is_safe_error_key("abc123"));
        assert!(is_safe_error_key("0123456789abcdef"));
    }

    #[test]
    fn safe_error_key_rejects_traversal_and_garbage() {
        assert!(!is_safe_error_key(""));
        assert!(!is_safe_error_key("../etc/passwd"));
        assert!(!is_safe_error_key("abc/def"));
        assert!(!is_safe_error_key("nothex!"));
        assert!(!is_safe_error_key(&"a".repeat(65)));
    }

    #[test]
    fn build_apply_response_note() {
        let plan = ApplyPlan::Note { text: "hi".into() };
        let r = build_apply_response(&plan, ApplyMode::Dry, "abc");
        assert_eq!(r.plan_kind, "note");
        assert_eq!(r.message, "hi");
    }

    #[test]
    fn build_apply_response_patch_dry() {
        let plan = ApplyPlan::Patch {
            diff: "-a\n+b\n".into(),
        };
        let r = build_apply_response(&plan, ApplyMode::Dry, "abc");
        assert_eq!(r.plan_kind, "patch");
        assert!(r.artifact_path.is_none());
        assert!(r.message.contains("-a"));
    }

    #[test]
    fn build_apply_response_patch_stage_writes_file() {
        let plan = ApplyPlan::Patch {
            diff: "-a\n+b\n".into(),
        };
        let key = format!("test{}", std::process::id());
        let r = build_apply_response(&plan, ApplyMode::Stage, &key);
        assert_eq!(r.plan_kind, "patch");
        let p = r.artifact_path.expect("artifact_path set");
        let content = std::fs::read_to_string(&p).expect("patch file");
        assert_eq!(content, "-a\n+b\n");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn build_apply_response_shell_dry() {
        let plan = ApplyPlan::Shell {
            command: "echo hi".into(),
        };
        let r = build_apply_response(&plan, ApplyMode::Dry, "abc");
        assert_eq!(r.plan_kind, "shell");
        assert!(!r.clipboard);
        assert!(r.message.contains("echo hi"));
    }
}
