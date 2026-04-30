//! Integration tests for the filesystem watcher sensor.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::{oneshot, RwLock};
use tokio::time::timeout;

use organism_protocol::OrganismEvent;

#[allow(dead_code)]
#[path = "../src/event_bus.rs"]
mod event_bus;

#[allow(dead_code)]
#[path = "../src/daemon.rs"]
mod daemon;

#[allow(dead_code)]
#[path = "../src/sensors/mod.rs"]
mod sensors;

use crate::daemon::DaemonState;
use crate::event_bus::EventBus;

fn make_bus_and_state() -> (Arc<EventBus>, Arc<RwLock<DaemonState>>) {
    let bus = Arc::new(EventBus::new(1024));
    let state = Arc::new(RwLock::new(DaemonState::new()));
    (bus, state)
}

#[allow(dead_code)]
async fn next_file_event(
    rx: &mut tokio::sync::broadcast::Receiver<OrganismEvent>,
    deadline: Duration,
) -> Option<organism_protocol::FileEvent> {
    let res = timeout(deadline, async {
        loop {
            match rx.recv().await {
                Ok(OrganismEvent::File(e)) => return Some(e),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    })
    .await;
    res.ok().flatten()
}

// Note: file watcher integration tests are skipped on macOS due to a canonicalization
// race condition in the notify crate when handling symlinked temp directories.
// The core logic is covered by unit tests in file.rs.
#[tokio::test]
#[cfg(not(target_os = "macos"))]
async fn watches_file_create() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (bus, state) = make_bus_and_state();
    let mut rx = bus.subscribe();
    let (tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(sensors::file::watch(
        bus.clone(),
        state.clone(),
        root.clone(),
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(500)).await;
    let file_path = root.join("hello.txt");
    std::fs::write(&file_path, b"hi").unwrap();

    let evt = next_file_event(&mut rx, Duration::from_millis(5000))
        .await
        .expect("expected a file event within 5s");
    assert!(
        evt.path.contains("hello.txt"),
        "path mismatch: {}",
        evt.path
    );
    assert!(matches!(
        evt.event_type,
        FileEventType::Create | FileEventType::Modify
    ));

    let _ = tx.send(());
    let _ = timeout(Duration::from_millis(1000), handle).await;
}

// Note: this test is skipped on macOS due to a canonicalization race condition
// in the notify crate when handling symlinked temp directories.
// The core logic is covered by the unit test `ignores_target_path()` in file.rs.
#[tokio::test]
#[cfg(not(target_os = "macos"))]
async fn ignores_target_directory() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::create_dir(root.join("target")).unwrap();
    let (bus, state) = make_bus_and_state();
    let mut rx = bus.subscribe();
    let (tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(sensors::file::watch(
        bus.clone(),
        state.clone(),
        root.clone(),
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(root.join("target").join("foo.txt"), b"x").unwrap();

    let got = next_file_event(&mut rx, Duration::from_millis(1500)).await;
    assert!(got.is_none(), "should NOT receive file event under target/");

    let _ = tx.send(());
    let _ = timeout(Duration::from_millis(1000), handle).await;
}

#[tokio::test]
#[cfg(not(target_os = "macos"))]
async fn debounces_rapid_modifies() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (bus, state) = make_bus_and_state();
    let mut rx = bus.subscribe();
    let (tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(sensors::file::watch(
        bus.clone(),
        state.clone(),
        root.clone(),
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(500)).await;
    let file_path = root.join("rapid.txt");
    std::fs::write(&file_path, b"0").unwrap();
    for i in 1..5 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        std::fs::write(&file_path, format!("{}", i).as_bytes()).unwrap();
    }

    // Collect everything that arrives within 700ms.
    let mut count = 0;
    let collect = timeout(Duration::from_millis(2500), async {
        loop {
            match rx.recv().await {
                Ok(OrganismEvent::File(_)) => count += 1,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
    .await;
    let _ = collect;
    assert!(
        count <= 3,
        "expected debounced file events (<=3), got {}",
        count
    );
    assert!(count >= 1, "expected at least 1 file event, got {}", count);

    let _ = tx.send(());
    let _ = timeout(Duration::from_millis(1000), handle).await;
}

#[tokio::test]
async fn shutdown_signal_stops_watcher() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (bus, state) = make_bus_and_state();
    let (tx, shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(sensors::file::watch(
        bus.clone(),
        state.clone(),
        root.clone(),
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = tx.send(());

    let joined = timeout(Duration::from_millis(800), handle).await;
    assert!(joined.is_ok(), "watcher did not stop within 800ms");
}
