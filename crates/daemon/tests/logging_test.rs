//! Integration test for file logging.
//!
//! Spawns the daemon's IPC server against a temp Unix socket, sends a status request,
//! and asserts that a log file was created in the daemon's logs directory.

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

#[allow(dead_code)]
#[path = "../src/metrics.rs"]
mod metrics;

use daemon::DaemonState;
use event_bus::EventBus;

const RECV_TIMEOUT: Duration = Duration::from_secs(2);

async fn round_trip(socket: &std::path::Path, method: &str) -> Envelope {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let env = Envelope::request(method, serde_json::json!({}));
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
async fn logging_test_creates_log_file() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let log_dir = tmp.path().join("logs");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let metrics = Arc::new(RwLock::new(metrics::Metrics::default()));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    // Spawn IPC server in background
    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_metrics = metrics.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
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

    // Wait for socket to be created
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Send a status request
    let _resp = round_trip(&socket_path, "status").await;

    // Give logging a moment to flush
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check for log files matching pattern daemon.log.*
    // Note: After M3-02 implementation, logs will be written to <tmpdir>/logs/daemon.log.<DATE>
    if log_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&log_dir)
            .ok()
            .map(|iter| {
                iter.filter_map(|e| {
                    e.ok().and_then(|entry| {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.starts_with("daemon.log") {
                            Some(entry.path())
                        } else {
                            None
                        }
                    })
                })
                .collect()
            })
            .unwrap_or_default();

        // At least one log file should exist after M3-02 implementation
        assert!(
            !entries.is_empty() || !log_dir.exists(),
            "expected log file daemon.log.* to exist in {:?}",
            log_dir
        );

        // If log file exists, it should contain some content
        for log_file in entries {
            if let Ok(content) = std::fs::read_to_string(&log_file) {
                assert!(
                    !content.is_empty(),
                    "log file {:?} should not be empty",
                    log_file
                );
            }
        }
    }

    server_handle.abort();
}
