//! End-to-end IPC test for M7 multi-block plan parsing.
//!
//! Seeds a suggestion with both bash and diff fences, applies with --stage,
//! and asserts that both shell and patch artifacts are created.

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
async fn apply_stage_multi_block_writes_artifacts() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed: error + suggestion with bash and diff blocks.
    let hash = "abc123def456";
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let suggestion = "Run this first:\n```bash\nbrew install foo\n```\n\nThen apply patch:\n```diff\n-old\n+new\n```\n";
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

    // Apply with --stage mode
    let req = Envelope::request(
        "apply",
        serde_json::json!({
            "error_key": hash,
            "mode": "stage"
        }),
    );

    let resp = send(&socket_path, req).await;

    // Check the response
    let result = &resp.payload["result"];

    // Should have plans array with 2 items (shell + patch)
    let plans = result
        .get("plans")
        .and_then(|p| p.as_array())
        .expect("plans array");
    assert_eq!(plans.len(), 2, "should have 2 plans (shell + patch)");

    // First plan: shell
    let shell_plan = &plans[0];
    assert_eq!(shell_plan["kind"], "shell");
    assert_eq!(shell_plan["body"], "brew install foo\n");
    // Multi-block shell should write to .sh file
    let shell_artifact = shell_plan.get("artifact_path").and_then(|p| p.as_str());
    assert!(
        shell_artifact.is_some(),
        "shell plan should have artifact_path"
    );

    // Second plan: patch
    let patch_plan = &plans[1];
    assert_eq!(patch_plan["kind"], "patch");
    assert_eq!(patch_plan["body"], "-old\n+new\n");
    let patch_artifact = patch_plan
        .get("artifact_path")
        .and_then(|p| p.as_str())
        .expect("patch should have artifact_path");

    // Verify files actually exist on disk
    if let Some(shell_path) = shell_artifact {
        assert!(
            std::path::Path::new(shell_path).exists(),
            "shell script file should exist at {}",
            shell_path
        );
        let contents = std::fs::read_to_string(shell_path).expect("read shell script");
        assert_eq!(contents, "brew install foo\n");
    }

    assert!(
        std::path::Path::new(patch_artifact).exists(),
        "patch file should exist at {}",
        patch_artifact
    );
    let patch_contents = std::fs::read_to_string(patch_artifact).expect("read patch");
    assert_eq!(patch_contents, "-old\n+new\n");

    // Shutdown
    let _ = shutdown_tx.send(());
    let _ = timeout(RECV_TIMEOUT, server_handle).await;
}

#[tokio::test]
async fn apply_stage_three_blocks_preserves_order() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed: suggestion with three blocks
    let hash = "def789abc123";
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let suggestion = "First:\n```bash\necho 1\n```\nSecond:\n```bash\necho 2\n```\nThird:\n```patch\n-x\n+y\n```\n";
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
        serde_json::json!({
            "error_key": hash,
            "mode": "stage"
        }),
    );

    let resp = send(&socket_path, req).await;
    let result = resp.payload.get("result").unwrap_or(&resp.payload);

    let plans = result
        .get("plans")
        .and_then(|p| p.as_array())
        .expect("plans array");
    assert_eq!(plans.len(), 3, "should have 3 plans");

    // Verify order is preserved
    assert_eq!(plans[0]["kind"], "shell");
    assert_eq!(plans[0]["body"], "echo 1\n");

    assert_eq!(plans[1]["kind"], "shell");
    assert_eq!(plans[1]["body"], "echo 2\n");

    assert_eq!(plans[2]["kind"], "patch");
    assert_eq!(plans[2]["body"], "-x\n+y\n");

    // Shutdown
    let _ = shutdown_tx.send(());
    let _ = timeout(RECV_TIMEOUT, server_handle).await;
}

#[tokio::test]
async fn apply_stage_single_shell_message() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed: error + suggestion with only a single bash block
    let hash = "abcd0001";
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let suggestion = "Run this:\n```bash\necho hello\n```\n";
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

    // Apply with --stage mode for single shell
    let req = Envelope::request(
        "apply",
        serde_json::json!({
            "error_key": hash,
            "mode": "stage"
        }),
    );

    let resp = send(&socket_path, req).await;
    let result = &resp.payload["result"];

    // Single shell plan should exist
    let plans = result
        .get("plans")
        .and_then(|p| p.as_array())
        .expect("plans array");
    assert_eq!(plans.len(), 1, "should have 1 plan");

    let shell_plan = &plans[0];
    assert_eq!(shell_plan["kind"], "shell");
    assert_eq!(shell_plan["body"], "echo hello\n");

    // Check the message: should be either "copied to clipboard:" or "command:"
    // depending on clipboard flag
    let message = result["message"].as_str().expect("message");
    let clipboard = result["clipboard"].as_bool().expect("clipboard bool");

    if clipboard {
        assert!(
            message.starts_with("copied to clipboard:"),
            "message should mention clipboard when clipboard=true, got: {}",
            message
        );
    } else {
        assert!(
            message.starts_with("command:"),
            "message should say 'command:' when clipboard=false, got: {}",
            message
        );
    }

    // Shutdown
    let _ = shutdown_tx.send(());
    let _ = timeout(RECV_TIMEOUT, server_handle).await;
}

#[tokio::test]
async fn apply_stage_single_patch_message() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");

    // Seed: error + suggestion with only a single diff block
    let hash = "def00001";
    {
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let suggestion = "Apply this patch:\n```diff\n-old line\n+new line\n```\n";
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

    // Apply with --stage mode for single patch
    let req = Envelope::request(
        "apply",
        serde_json::json!({
            "error_key": hash,
            "mode": "stage"
        }),
    );

    let resp = send(&socket_path, req).await;
    let result = &resp.payload["result"];

    // Single patch plan should exist
    let plans = result
        .get("plans")
        .and_then(|p| p.as_array())
        .expect("plans array");
    assert_eq!(plans.len(), 1, "should have 1 plan");

    let patch_plan = &plans[0];
    assert_eq!(patch_plan["kind"], "patch");
    assert_eq!(patch_plan["body"], "-old line\n+new line\n");

    // Check the message: should mention "patch written" and include artifact_path
    let message = result["message"].as_str().expect("message");
    assert!(
        message.contains("patch written"),
        "message should mention 'patch written' in stage mode, got: {}",
        message
    );
    assert!(
        message.contains("git apply"),
        "message should contain 'git apply' instruction, got: {}",
        message
    );

    // artifact_path should be populated
    let artifact_path = result
        .get("artifact_path")
        .and_then(|a| a.as_str())
        .expect("artifact_path");
    assert!(
        !artifact_path.is_empty(),
        "artifact_path should be populated for stage patch"
    );

    // Shutdown
    let _ = shutdown_tx.send(());
    let _ = timeout(RECV_TIMEOUT, server_handle).await;
}
