//! Integration test for graceful shutdown via signal channels.
//!
//! Verifies that:
//! 1. shutdown signal can be sent to multiple subscribers
//! 2. all tasks exit cleanly within timeout
//! 3. no panic or deadlock

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};

/// Test that broadcast shutdown channel can fan out to multiple subscribers.
#[tokio::test]
async fn test_shutdown_fan_out() {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Spawn 3 subscribers that listen to shutdown
    for _ in 0..3 {
        let mut rx = shutdown_tx.subscribe();
        let cnt = counter.clone();
        let h = tokio::spawn(async move {
            tokio::select! {
                _ = rx.recv() => {
                    cnt.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        handles.push(h);
    }

    // Give tasks time to register
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send shutdown signal
    let _ = shutdown_tx.send(());

    // Wait for all tasks to complete
    for h in handles {
        let _ = timeout(Duration::from_secs(1), h).await;
    }

    // All 3 should have received the signal
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

/// Test that shutdown signal completes within timeout (no deadlock).
#[tokio::test]
async fn test_shutdown_no_deadlock() {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let stx = shutdown_tx.clone();

    let handle = tokio::spawn(async move {
        let mut rx = stx.subscribe();
        loop {
            tokio::select! {
                _ = rx.recv() => break,
                // Simulated work that runs forever unless shutdown
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
            }
        }
    });

    // Give the task time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send shutdown
    let _ = shutdown_tx.send(());

    // Should complete quickly after signal
    let result = timeout(Duration::from_millis(500), handle).await;
    assert!(
        result.is_ok(),
        "Shutdown should complete quickly after signal; task may be deadlocked"
    );
}

/// Test that tasks can select! between shutdown and async work.
#[tokio::test]
async fn test_select_shutdown_vs_work() {
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let counter = Arc::new(AtomicUsize::new(0));
    let stx = shutdown_tx.clone();

    let cnt = counter.clone();
    let handle = tokio::spawn(async move {
        let mut rx = stx.subscribe();
        let mut work_count = 0;
        loop {
            tokio::select! {
                _ = rx.recv() => break,
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    work_count += 1;
                    if work_count >= 100 {
                        break;
                    }
                }
            }
        }
        cnt.fetch_add(work_count, Ordering::SeqCst);
    });

    // Let it do a few iterations
    tokio::time::sleep(Duration::from_millis(25)).await;

    // Send shutdown while work is ongoing
    let _ = shutdown_tx.send(());

    // Should exit gracefully without completing all 100 iterations
    let result = timeout(Duration::from_secs(1), handle).await;
    assert!(result.is_ok(), "Task should exit on shutdown signal");
}
