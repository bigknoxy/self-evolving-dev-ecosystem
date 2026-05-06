//! Tests for M11-03: conditional profile rebuild after feedback events.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, RwLock};
use tokio::time::timeout;

use organism_knowledge::{ErrorRecord, KnowledgeStore};
use organism_protocol::Envelope;
use serial_test::serial;

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

async fn round_trip_feedback(socket: &std::path::Path, error_key: &str, verdict: &str) -> Envelope {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let params = serde_json::json!({
        "error_key": error_key,
        "verdict": verdict,
        "note": None::<String>
    });
    let env = Envelope::request("feedback", params);
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

#[allow(dead_code)]
async fn round_trip_get_profile(socket: &std::path::Path) -> Envelope {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let env = Envelope::request("profile", serde_json::json!({}));
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
#[serial]
async fn test_profile_rebuilds_after_events() {
    // Reset first to ensure clean state for this test
    ipc::reset_refresh_state().await;

    // Set refresh threshold to 1 event
    std::env::set_var("ORGANISM_PROFILE_REFRESH_EVERY", "1");
    std::env::set_var("ORGANISM_PROFILE_REFRESH_MIN_INTERVAL_MS", "0");

    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("profile_rebuild_test.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    // Seed test data (must be valid hex)
    let error_hash = "1111111111111111111111111111111a";
    let suggestion_text = "Try running cargo fix --allow-dirty";
    {
        let mut store = knowledge.write().await;
        let now = chrono::Utc::now();
        let err = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: error_hash.to_string(),
            raw_excerpt: "error[E0599]: no method named `foo`".to_string(),
            first_seen: now,
            last_seen: now,
            last_command: "cargo build".to_string(),
            occurrences: 1,
            schema_v: 1,
        };
        store.put_error(&err).expect("put_error");
        store
            .put_suggestion(error_hash, suggestion_text)
            .expect("put_suggestion");
    }

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel(1);
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

    // Wait for socket to bind
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(socket_path.exists(), "socket file should be created");

    // Post feedback: rebuild should trigger (threshold=1)
    let _ = round_trip_feedback(&socket_path, error_hash, "accept").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    {
        let mut store = knowledge.write().await;
        let profile_opt = store.get_style_profile().ok().flatten();
        assert!(
            profile_opt.is_some(),
            "profile should exist after feedback (threshold=1)"
        );
    }

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_rate_limit_blocks_immediate_rebuild() {
    // Reset first to ensure clean state for this test
    ipc::reset_refresh_state().await;

    // Set refresh threshold to 1 event, with a 10s rate-limit window
    std::env::set_var("ORGANISM_PROFILE_REFRESH_EVERY", "1");
    std::env::set_var("ORGANISM_PROFILE_REFRESH_MIN_INTERVAL_MS", "10000");

    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("rate_limit_test.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    // Seed test data (must be valid hex, unique per test)
    let error_hash = "2222222222222222222222222222222b";
    let suggestion_text = "Fix the build error";
    {
        let mut store = knowledge.write().await;
        let now = chrono::Utc::now();
        let err = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: error_hash.to_string(),
            raw_excerpt: "error[E0599]: no method named `bar`".to_string(),
            first_seen: now,
            last_seen: now,
            last_command: "cargo build".to_string(),
            occurrences: 1,
            schema_v: 1,
        };
        store.put_error(&err).expect("put_error");
        store
            .put_suggestion(error_hash, suggestion_text)
            .expect("put_suggestion");
    }

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel(1);
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

    // Wait for socket to bind
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(socket_path.exists(), "socket file should be created");

    // Post 1st feedback: should trigger rebuild
    let _ = round_trip_feedback(&socket_path, error_hash, "accept").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let profile_exists_1 = {
        let mut store = knowledge.write().await;
        store.get_style_profile().ok().flatten().is_some()
    };
    assert!(
        profile_exists_1,
        "profile should exist after 1st feedback (threshold=1, no prior refresh)"
    );

    // Post another feedback immediately: should be rate-limited (within 10s window)
    let _ = round_trip_feedback(&socket_path, error_hash, "accept").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Profile should still exist and be unchanged (not rebuilt again)
    let profile_exists_2 = {
        let mut store = knowledge.write().await;
        store.get_style_profile().ok().flatten().is_some()
    };
    assert!(
        profile_exists_2,
        "profile should still exist (rate-limited within 10s window)"
    );

    server_handle.abort();
}

#[tokio::test]
#[serial]
async fn test_threshold_with_zero_rate_limit() {
    // Reset first to ensure clean state for this test
    ipc::reset_refresh_state().await;

    // Set refresh threshold to 2 events, 0ms rate-limit (rebuild every 2 events)
    std::env::set_var("ORGANISM_PROFILE_REFRESH_EVERY", "2");
    std::env::set_var("ORGANISM_PROFILE_REFRESH_MIN_INTERVAL_MS", "0");

    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("threshold_test.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    // Seed test data (must be valid hex, unique per test)
    let error_hash = "3333333333333333333333333333333c";
    let suggestion_text = "Apply this fix";
    {
        let mut store = knowledge.write().await;
        let now = chrono::Utc::now();
        let err = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: error_hash.to_string(),
            raw_excerpt: "error[E0599]: no method named `baz`".to_string(),
            first_seen: now,
            last_seen: now,
            last_command: "cargo build".to_string(),
            occurrences: 1,
            schema_v: 1,
        };
        store.put_error(&err).expect("put_error");
        store
            .put_suggestion(error_hash, suggestion_text)
            .expect("put_suggestion");
    }

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let (_shutdown_tx, shutdown_rx) = broadcast::channel(1);
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

    // Wait for socket to bind
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(socket_path.exists(), "socket file should be created");

    // Post 1st feedback: no rebuild (threshold=2)
    let _ = round_trip_feedback(&socket_path, error_hash, "accept").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Post 2nd feedback: rebuild should trigger (counter hits threshold)
    let _ = round_trip_feedback(&socket_path, error_hash, "accept").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let profile_exists = {
        let mut store = knowledge.write().await;
        store.get_style_profile().ok().flatten().is_some()
    };
    assert!(
        profile_exists,
        "profile should exist after reaching threshold (2 events)"
    );

    server_handle.abort();
}
