//! Integration test for the error subscriber: publishes failed terminal
//! events on the bus and verifies that ErrorRecord entries are persisted in
//! the knowledge store, with occurrences incrementing on duplicates.

use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::RwLock;

use organism_knowledge::KnowledgeStore;
use organism_protocol::{EventContext, OrganismEvent, TerminalEvent};

#[allow(dead_code)]
#[path = "../src/event_bus.rs"]
mod event_bus;

#[allow(dead_code)]
#[path = "../src/error_subscriber.rs"]
mod error_subscriber;

use event_bus::EventBus;

fn make_failed_event(stderr: &str) -> OrganismEvent {
    OrganismEvent::Terminal(TerminalEvent {
        ts: chrono::Utc::now(),
        pid: 4242,
        cwd: "/tmp/proj".to_string(),
        command_line: "cargo build".to_string(),
        stdout_snippet: None,
        stderr_snippet: Some(stderr.to_string()),
        keystroke_rate: 0.0,
        exit_code: Some(101),
        duration_ms: Some(120),
        context: EventContext::default(),
    })
}

#[tokio::test]
async fn error_subscriber_persists_and_increments() {
    let tmp = TempDir::new().expect("tempdir");
    let store = KnowledgeStore::open(tmp.path()).expect("open store");
    let knowledge = Arc::new(RwLock::new(store));
    let bus = Arc::new(EventBus::new(64));

    // Spawn subscriber.
    let sub_bus = bus.clone();
    let sub_knowledge = knowledge.clone();
    let handle = tokio::spawn(async move {
        let _ = error_subscriber::run(sub_bus, sub_knowledge).await;
    });

    // Give the subscriber a moment to subscribe.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stderr = "error[E0599]: no method named `foo` found for type `Bar`";
    bus.publish(make_failed_event(stderr));

    // Wait up to 1s for persistence.
    let mut attempts = 0;
    let mut listed;
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        listed = knowledge.write().await.list_errors().expect("list");
        if !listed.is_empty() || attempts > 20 {
            break;
        }
        attempts += 1;
    }
    assert_eq!(listed.len(), 1, "expected 1 error record");
    assert_eq!(listed[0].tool, "rustc");
    assert_eq!(listed[0].kind, "E0599");
    assert_eq!(listed[0].occurrences, 1);

    // Publish same again -> occurrences should become 2.
    bus.publish(make_failed_event(stderr));

    let mut attempts = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let listed = knowledge.write().await.list_errors().expect("list");
        if listed.first().map(|r| r.occurrences).unwrap_or(0) >= 2 || attempts > 20 {
            assert_eq!(listed.len(), 1, "still only one record");
            assert_eq!(listed[0].occurrences, 2, "occurrences should increment");
            break;
        }
        attempts += 1;
    }

    handle.abort();
}
