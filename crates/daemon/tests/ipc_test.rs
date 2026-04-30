//! IPC server integration tests.
//!
//! Spawns the daemon's `ipc::serve` against a temp Unix socket and exercises
//! the Status / Sleep / Wake / Log methods.
//!
//! The daemon is a binary crate, so we mount its modules with #[path].

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;

use organism_protocol::Envelope;

// Modules are mounted from the binary crate's src/ via #[path]. Some items
// (Daemon::new, run_event_loop, knowledge field, etc.) are not exercised by
// this test and would otherwise trip dead_code lints.
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

fn awake(env: &Envelope) -> bool {
    env.payload
        .get("result")
        .and_then(|r| r.get("awake"))
        .and_then(|v| v.as_bool())
        .expect("awake field")
}

#[tokio::test]
async fn ipc_status_sleep_wake_log_flow() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    let state = Arc::new(RwLock::new(DaemonState::new()));
    let bus = Arc::new(EventBus::new(64));

    let serve_state = state.clone();
    let serve_bus = bus.clone();
    let serve_socket = socket_path.clone();
    let server_handle = tokio::spawn(async move {
        let _ = ipc::serve(serve_state, serve_bus, serve_socket).await;
    });

    // Wait briefly for the listener to bind.
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(socket_path.exists(), "socket file should be created");

    // 1. Status -> awake == true
    let resp = round_trip(&socket_path, "status").await;
    assert!(awake(&resp), "expected awake=true initially");

    // 2. Sleep
    let _ = round_trip(&socket_path, "sleep").await;

    // 3. Status -> awake == false
    let resp = round_trip(&socket_path, "status").await;
    assert!(!awake(&resp), "expected awake=false after sleep");

    // 4. Wake
    let _ = round_trip(&socket_path, "wake").await;
    let resp = round_trip(&socket_path, "status").await;
    assert!(awake(&resp), "expected awake=true after wake");

    // 5. Log returns an array payload
    let resp = round_trip(&socket_path, "log").await;
    let result = resp.payload.get("result").expect("result field");
    assert!(
        result.is_array(),
        "log result should be an array, got {:?}",
        result
    );

    server_handle.abort();
}
