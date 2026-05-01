//! End-to-end IPC test for the `apply` method.
//!
//! Boots `ipc::serve` against a temp Unix socket, seeds an error + suggestion
//! containing a fenced diff block, then asserts the dry-run response carries
//! the patch.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, RwLock};
use tokio::time::timeout;

use organism_knowledge::KnowledgeStore;
use organism_protocol::Envelope;

#[allow(dead_code)]
#[path = "../src/clipboard.rs"]
mod clipboard;

#[allow(dead_code)]
#[path = "../src/event_bus.rs"]
mod event_bus;

#[allow(dead_code)]
#[path = "../src/daemon.rs"]
mod daemon;

#[allow(dead_code)]
#[path = "../src/ipc.rs"]
mod ipc;

use daemon::DaemonState;
use event_bus::EventBus;

const RECV_TIMEOUT: Duration = Duration::from_secs(2);

async fn send(socket: &std::path::Path, env: Envelope) -> Envelope {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut buf = serde_json::to_string(&env).unwrap();
    buf.push('\n');
    write_half.write_all(buf.as_bytes()).await.unwrap();
    write_half.shutdown().await.ok();

    let mut line = String::new();
    timeout(RECV_TIMEOUT, reader.read_line(&mut line))
        .await
        .expect("recv timeout")
        .expect("read");
    serde_json::from_str(line.trim()).expect("parse envelope")
}

#[tokio::test]
async fn apply_dry_returns_patch_message() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed: error + suggestion with a diff block.
    let hash = "deadbeef";
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let suggestion = "Try this:\n```diff\n-old\n+new\n```\n";
        store.put_suggestion(hash, suggestion).unwrap();
    }

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(
            serve_state,
            serve_bus,
            serve_knowledge,
            serve_socket,
            shutdown_rx,
        )
        .await;
    });

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(socket_path.exists(), "socket file should be created");

    let req = Envelope::request(
        "apply",
        serde_json::json!({ "error_key": hash, "mode": "dry" }),
    );
    let resp = send(&socket_path, req).await;
    let result = resp.payload.get("result").expect("result field");
    assert_eq!(
        result.get("plan_kind").and_then(|v| v.as_str()),
        Some("patch")
    );
    let msg = result
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(msg.contains("-old"), "message should contain diff: {}", msg);
    assert!(msg.contains("+new"), "message should contain diff: {}", msg);

    let _ = shutdown_tx.send(());
    server_handle.abort();
}

#[tokio::test]
async fn apply_unknown_key_errors() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(
            serve_state,
            serve_bus,
            serve_knowledge,
            serve_socket,
            shutdown_rx,
        )
        .await;
    });

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let req = Envelope::request(
        "apply",
        serde_json::json!({ "error_key": "abc123", "mode": "dry" }),
    );
    let resp = send(&socket_path, req).await;
    let err = resp.payload.get("error").and_then(|v| v.as_str());
    assert!(err.is_some(), "expected error response, got {:?}", resp);
    assert!(err.unwrap().contains("no cached suggestion"));

    let _ = shutdown_tx.send(());
    server_handle.abort();
}

#[tokio::test]
async fn apply_rejects_path_traversal_key() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(
            serve_state,
            serve_bus,
            serve_knowledge,
            serve_socket,
            shutdown_rx,
        )
        .await;
    });

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let req = Envelope::request(
        "apply",
        serde_json::json!({ "error_key": "../etc/passwd", "mode": "dry" }),
    );
    let resp = send(&socket_path, req).await;
    let err = resp.payload.get("error").and_then(|v| v.as_str());
    assert!(err.is_some(), "expected error response, got {:?}", resp);
    assert!(err.unwrap().contains("invalid error_key"));

    let _ = shutdown_tx.send(());
    server_handle.abort();
}
