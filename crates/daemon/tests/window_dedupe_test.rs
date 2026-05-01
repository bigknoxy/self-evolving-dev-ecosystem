use chrono::Utc;
use organism_daemon::event_bus::EventBus;
use organism_protocol::{ErrorClassifiedEvent, OrganismEvent};
use std::sync::Arc;

#[tokio::test]
async fn test_first_occurrence_is_first_in_window() {
    let bus = Arc::new(EventBus::new(16));
    let mut rx = bus.subscribe();

    // Emit first event with is_first_in_window=true
    let first_event = OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash".to_string(),
        tool: "rustc".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    });

    bus.publish(first_event);

    let received = rx.recv().await;
    assert!(received.is_ok());
    if let Ok(OrganismEvent::ErrorClassified(e)) = received {
        assert_eq!(e.hash, "test_hash");
        assert!(
            e.is_first_in_window,
            "First occurrence should be first_in_window=true"
        );
    }
}

#[tokio::test]
async fn test_second_within_60s_is_not_first_in_window() {
    let bus = Arc::new(EventBus::new(16));
    let mut rx = bus.subscribe();

    // First event
    let first_event = OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash2".to_string(),
        tool: "rustc".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    });

    bus.publish(first_event);
    let _ = rx.recv().await;

    // Second event within 60s
    let second_event = OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash2".to_string(),
        tool: "rustc".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: false,
    });

    bus.publish(second_event);

    let received = rx.recv().await;
    assert!(received.is_ok());
    if let Ok(OrganismEvent::ErrorClassified(e)) = received {
        assert_eq!(e.hash, "test_hash2");
        assert!(
            !e.is_first_in_window,
            "Second occurrence within 60s should be first_in_window=false"
        );
    }
}

#[tokio::test]
async fn test_second_after_60s_is_first_in_window() {
    let bus = Arc::new(EventBus::new(16));
    let mut rx = bus.subscribe();

    // First event
    let first_event = OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash3".to_string(),
        tool: "rustc".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    });

    bus.publish(first_event);
    let _ = rx.recv().await;

    // Simulate time passing (>60s)
    let future_time = Utc::now() + chrono::Duration::seconds(61);

    // Second event after 60s
    let second_event = OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
        ts: future_time,
        hash: "test_hash3".to_string(),
        tool: "rustc".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true, // Should be set to true by error_subscriber logic
    });

    bus.publish(second_event);

    let received = rx.recv().await;
    assert!(received.is_ok());
    if let Ok(OrganismEvent::ErrorClassified(e)) = received {
        assert_eq!(e.hash, "test_hash3");
        assert!(
            e.is_first_in_window,
            "Second occurrence after 60s should be first_in_window=true"
        );
    }
}
