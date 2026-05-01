//! End-to-end IPC test for the `errors` method.
//!
//! Boots `ipc::serve` against a temp Unix socket, seeds 2 errors with suggestions,
//! calls the errors IPC handler, and asserts 2 items are returned sorted by last_seen DESC.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;

use organism_knowledge::{KnowledgeStore, ErrorRecord};
use organism_protocol::{Envelope, ErrorsRequest, ErrorsResponse};

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
async fn errors_returns_sorted_list() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed 2 errors with different last_seen timestamps and suggestions
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let now = chrono::Utc::now();

        // Error 1: older
        let err1 = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "hash1111".into(),
            raw_excerpt: "error1".into(),
            first_seen: now - chrono::Duration::hours(2),
            last_seen: now - chrono::Duration::hours(2),
            occurrences: 3,
            last_command: "cargo build".into(),
        };

        // Error 2: more recent
        let err2 = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0308".into(),
            hash: "hash2222".into(),
            raw_excerpt: "error2".into(),
            first_seen: now - chrono::Duration::minutes(30),
            last_seen: now - chrono::Duration::minutes(30),
            occurrences: 1,
            last_command: "cargo test".into(),
        };

        store.put_error(&err1).unwrap();
        store.put_error(&err2).unwrap();

        // Add suggestions for both
        store
            .put_suggestion("hash1111", "suggestion for error 1")
            .unwrap();
        store
            .put_suggestion("hash2222", "suggestion for error 2")
            .unwrap();
    }

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));
    let knowledge = Arc::new(RwLock::new(KnowledgeStore::open(tmp.path()).unwrap()));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_knowledge = knowledge.clone();
    let serve_socket = socket_path.clone();
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(serve_state, serve_bus, serve_knowledge, serve_socket).await;
    });

    // Give server time to bind
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send errors request with limit 20
    let req = Envelope::request(
        "errors",
        serde_json::to_value(ErrorsRequest { limit: Some(20) }).unwrap(),
    );
    let resp_env = send(&socket_path, req).await;

    // Parse response
    let resp: ErrorsResponse = serde_json::from_value(resp_env.payload.get("result").unwrap().clone())
        .expect("parse errors response");

    // Assert we have 2 items
    assert_eq!(resp.items.len(), 2);

    // Assert sorted by last_seen DESC (most recent first = hash2222)
    assert_eq!(resp.items[0].hash, "hash2222");
    assert_eq!(resp.items[0].command, "cargo test");
    assert_eq!(resp.items[0].occurrences, 1);
    assert!(resp.items[0].has_suggestion);

    // Second item should be older error
    assert_eq!(resp.items[1].hash, "hash1111");
    assert_eq!(resp.items[1].command, "cargo build");
    assert_eq!(resp.items[1].occurrences, 3);
    assert!(resp.items[1].has_suggestion);

    // Verify last_seen is RFC3339 format
    let last_seen_str = &resp.items[0].last_seen;
    assert!(last_seen_str.contains('T'));
    assert!(last_seen_str.ends_with('Z'));

    // Cleanup
    drop(state);
    drop(bus);
    drop(knowledge);
    server_handle.abort();
}
