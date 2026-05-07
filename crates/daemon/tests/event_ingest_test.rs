//! Integration test for the IPC `event` ingest path.
//!
//! Sends an `event` envelope carrying a `TerminalEvent`, then verifies:
//!   * the response is ok with `recorded: true`
//!   * the daemon's `status` reports `event_count >= 1`
//!   * the daemon's `log` array contains the original command line

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, RwLock};
use tokio::time::timeout;

use organism_knowledge::KnowledgeStore;
use organism_protocol::{Envelope, EventContext, OrganismEvent, TerminalEvent};

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

#[allow(dead_code)]
#[path = "../src/metrics.rs"]
mod metrics;

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
async fn event_ingest_records_and_appears_in_log() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let metrics = Arc::new(RwLock::new(metrics::Metrics::default()));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_metrics = metrics.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(
            serve_state,
            serve_bus,
            serve_knowledge,
            serve_metrics,
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

    // Send an `event` envelope carrying a TerminalEvent.
    let cmd = "cargo build --workspace".to_string();
    let evt = OrganismEvent::Terminal(TerminalEvent {
        ts: chrono::Utc::now(),
        pid: 4242,
        cwd: "/tmp/proj".to_string(),
        command_line: cmd.clone(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        exit_code: Some(0),
        duration_ms: Some(12),
        context: EventContext::default(),
    });
    let payload = serde_json::to_value(&evt).unwrap();
    let req = Envelope::request("event", payload);
    let resp = send(&socket_path, req).await;

    let result = resp.payload.get("result").expect("result field");
    assert_eq!(
        result.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "expected ok=true, got {:?}",
        result
    );
    assert_eq!(
        result.get("recorded").and_then(|v| v.as_bool()),
        Some(true),
        "expected recorded=true, got {:?}",
        result
    );

    // status -> event_count >= 1
    let status_resp = send(
        &socket_path,
        Envelope::request("status", serde_json::json!({})),
    )
    .await;
    let count = status_resp
        .payload
        .get("result")
        .and_then(|r| r.get("event_count"))
        .and_then(|v| v.as_u64())
        .expect("event_count field");
    assert!(count >= 1, "expected event_count >= 1, got {}", count);

    // log -> array contains the command line
    let log_resp = send(
        &socket_path,
        Envelope::request("log", serde_json::json!({})),
    )
    .await;
    let arr = log_resp
        .payload
        .get("result")
        .and_then(|r| r.as_array())
        .expect("log result should be an array");
    let joined: String = arr
        .iter()
        .filter_map(|e| e.get("msg").and_then(|m| m.as_str()))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        joined.contains(&cmd),
        "expected log entries to contain {:?}, got {:?}",
        cmd,
        joined
    );

    let _ = shutdown_tx.send(());
    server_handle.abort();
}

#[tokio::test]
async fn event_ingest_when_asleep_acks_but_does_not_record() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let metrics = Arc::new(RwLock::new(metrics::Metrics::default()));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_metrics = metrics.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(
            serve_state,
            serve_bus,
            serve_knowledge,
            serve_metrics,
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

    // Put the daemon to sleep.
    let _ = send(
        &socket_path,
        Envelope::request("sleep", serde_json::json!({})),
    )
    .await;

    let evt = OrganismEvent::Terminal(TerminalEvent {
        ts: chrono::Utc::now(),
        pid: 1,
        cwd: "/".to_string(),
        command_line: "ls".to_string(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        exit_code: None,
        duration_ms: None,
        context: EventContext::default(),
    });
    let resp = send(
        &socket_path,
        Envelope::request("event", serde_json::to_value(&evt).unwrap()),
    )
    .await;
    let result = resp.payload.get("result").expect("result");
    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        result.get("recorded").and_then(|v| v.as_bool()),
        Some(false),
        "asleep daemon must not record"
    );

    // event_count stays 0.
    let status_resp = send(
        &socket_path,
        Envelope::request("status", serde_json::json!({})),
    )
    .await;
    let count = status_resp
        .payload
        .get("result")
        .and_then(|r| r.get("event_count"))
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(count, 0, "asleep daemon should keep event_count at 0");

    let _ = shutdown_tx.send(());
    server_handle.abort();
}
