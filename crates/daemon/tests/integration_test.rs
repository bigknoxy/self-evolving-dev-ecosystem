//! Integration tests for the daemon event bus.

use chrono::Utc;
use organism_protocol::{EventContext, OrganismEvent, TerminalEvent};

// We import internal modules by re-exporting them or using #[path]
// For simplicity, we replicate the EventBus here for testing.
// Real integration tests would use the daemon library (convert daemon to lib + bin).

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

#[tokio::test]
async fn test_event_bus_publish_subscribe() {
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();

    let event = OrganismEvent::Terminal(TerminalEvent {
        ts: Utc::now(),
        pid: 9999,
        cwd: "/test".to_string(),
        command_line: "cargo test".to_string(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        exit_code: None,
        duration_ms: None,
        context: EventContext::default(),
    });

    bus.publish(event);

    let received = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("timeout")
        .expect("recv error");

    if let OrganismEvent::Terminal(t) = received {
        assert_eq!(t.command_line, "cargo test");
        assert_eq!(t.pid, 9999);
    } else {
        panic!("Expected terminal event");
    }
}

#[tokio::test]
async fn test_event_bus_multiple_subscribers() {
    let bus = EventBus::new(64);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    let event = OrganismEvent::Terminal(TerminalEvent {
        ts: Utc::now(),
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

    bus.publish(event);

    let r1 = rx1.recv().await.unwrap();
    let r2 = rx2.recv().await.unwrap();

    if let (OrganismEvent::Terminal(t1), OrganismEvent::Terminal(t2)) = (r1, r2) {
        assert_eq!(t1.command_line, t2.command_line);
    }
}

#[tokio::test]
async fn test_knowledge_store_in_tempdir() {
    use chrono::Utc;
    use organism_knowledge::{FixRecord, KnowledgeStore};

    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = KnowledgeStore::open(tmp.path()).unwrap();

    let fix = FixRecord {
        id: "integration-fix".to_string(),
        signature_hash: "hash999".to_string(),
        patch: "apply fix X".to_string(),
        confidence: 0.88,
        applied_count: 2,
        last_applied: Utc::now(),
        source: "learned".to_string(),
    };

    store.put_fix(&fix).unwrap();
    let retrieved = store.get_fix("hash999").unwrap().unwrap();
    assert_eq!(retrieved.id, "integration-fix");
    assert!((retrieved.confidence - 0.88).abs() < 0.001);
}
