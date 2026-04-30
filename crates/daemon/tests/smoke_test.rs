//! Level 0 end-to-end smoke test:
//! EventBus -> consumer task -> pattern engine -> knowledge store roundtrip.
//!
//! The daemon is a binary crate, so we replicate EventBus inline here
//! (matching the pattern used by integration_test.rs).

use chrono::Utc;
use organism_cortex::pattern_engine::{detect_patterns, EventRecord};
use organism_knowledge::KnowledgeStore;
use organism_protocol::{EventContext, OrganismEvent, TerminalEvent};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::timeout;

mod event_bus {
    use organism_protocol::OrganismEvent;
    use tokio::sync::broadcast;

    pub struct EventBus {
        sender: broadcast::Sender<OrganismEvent>,
    }

    impl EventBus {
        pub fn new(capacity: usize) -> Self {
            let (sender, _) = broadcast::channel(capacity);
            Self { sender }
        }
        pub fn publish(&self, event: OrganismEvent) -> usize {
            self.sender.send(event).unwrap_or(0)
        }
        pub fn subscribe(&self) -> broadcast::Receiver<OrganismEvent> {
            self.sender.subscribe()
        }
    }
}

use event_bus::EventBus;

fn term(cmd: &str) -> OrganismEvent {
    OrganismEvent::Terminal(TerminalEvent {
        ts: Utc::now(),
        pid: 1234,
        cwd: "/tmp/proj".to_string(),
        command_line: cmd.to_string(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        exit_code: None,
        duration_ms: None,
        context: EventContext::default(),
    })
}

#[tokio::test]
async fn smoke_level0_end_to_end() {
    // 1. TempDir for knowledge store.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let store_path = tmp.path().to_path_buf();

    // 2. EventBus + consumer task.
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();

    // Sequence: 3x "cargo build" then 3x "cargo test".
    // This produces (build->build) x2, (build->test) x1, (test->test) x2 windows.
    // Both build->build and test->test reach frequency=2.
    let commands = [
        "cargo build",
        "cargo build",
        "cargo build",
        "cargo test",
        "cargo test",
        "cargo test",
    ];
    let expected = commands.len();

    let consumer = tokio::spawn(async move {
        let mut collected: Vec<String> = Vec::new();
        loop {
            let res = timeout(Duration::from_secs(5), rx.recv()).await;
            match res {
                Ok(Ok(OrganismEvent::Terminal(t))) => {
                    collected.push(t.command_line);
                    if collected.len() >= expected {
                        break;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => panic!("consumer timed out waiting for events"),
            }
        }
        collected
    });

    // 3. Publish.
    for c in &commands {
        let n = bus.publish(term(c));
        assert!(n >= 1, "no subscribers received event");
    }

    let collected = consumer.await.expect("consumer task join");
    assert_eq!(collected.len(), expected);

    // 4. Feed into pattern engine. Trigger == previous command, action == next command.
    let event_records: Vec<EventRecord> = collected
        .iter()
        .map(|cmd| EventRecord {
            project_id: "smoke".to_string(),
            event_type: cmd.clone(),
            description: cmd.clone(),
        })
        .collect();

    let patterns = detect_patterns(&event_records, 2);
    assert!(
        !patterns.is_empty(),
        "expected at least one pattern with frequency >= 2"
    );

    // 5. Persist + read back.
    let mut store = KnowledgeStore::open(&store_path).expect("open store");
    for p in &patterns {
        store.put_pattern(p).expect("put pattern");
    }

    let keys = store.list_patterns().expect("list patterns");
    assert!(!keys.is_empty(), "no patterns persisted");

    let mut found_freq_ge_2 = false;
    for p in &patterns {
        let got = store
            .get_pattern(&p.id)
            .expect("get pattern")
            .expect("pattern missing from store");
        if got.frequency >= 2 {
            found_freq_ge_2 = true;
        }
    }
    assert!(
        found_freq_ge_2,
        "no persisted PatternRecord had frequency >= 2"
    );

    // 6. Drop bus -> remaining subscribers exit cleanly.
    drop(bus);
    // Consumer already exited above; make a fresh subscriber would fail because
    // bus is moved. Instead, verify by spawning a second subscriber BEFORE drop
    // in a controlled way:
    let bus2 = EventBus::new(4);
    let mut rx2 = bus2.subscribe();
    drop(bus2);
    let closed = timeout(Duration::from_secs(5), rx2.recv())
        .await
        .expect("subscriber should not hang after bus drop");
    assert!(
        matches!(closed, Err(broadcast::error::RecvError::Closed)),
        "expected Closed after bus drop, got {:?}",
        closed
    );
}
