use chrono::Utc;
use organism_daemon::ollama_subscriber::{handle_event, SubscriberOutcome};
use organism_knowledge::{ErrorRecord, KnowledgeStore};
use organism_ollama::LlmClient;
use std::collections::HashSet;
use tempfile::TempDir;

struct MockLlmClient {
    response: String,
}

#[async_trait::async_trait]
impl LlmClient for MockLlmClient {
    async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
        Ok(self.response.clone())
    }
}

struct FailingLlmClient;

#[async_trait::async_trait]
impl LlmClient for FailingLlmClient {
    async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!("LLM unavailable"))
    }
}

#[tokio::test]
async fn test_handle_event_cached_suggestion() {
    let tmp = TempDir::new().expect("tempdir created");
    let mut store = KnowledgeStore::open(tmp.path()).expect("store opened");

    let error = ErrorRecord {
        tool: "cargo".to_string(),
        kind: "E0599".to_string(),
        hash: "test_hash".to_string(),
        raw_excerpt: "no method named `foo`".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 1,
        last_command: "cargo build".to_string(),
    };
    store.put_error(&error).expect("error stored");
    store
        .put_suggestion("test_hash", "Try implementing the trait.")
        .expect("suggestion stored");

    let event = organism_protocol::ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash".to_string(),
        tool: "cargo".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    };

    let mock = MockLlmClient {
        response: "Should not be called".to_string(),
    };
    let mut seen = HashSet::new();

    let outcome = handle_event(event, &mut store, &mock, &mut seen)
        .await
        .expect("outcome");
    assert_eq!(outcome, SubscriberOutcome::SkippedCached);
}

#[tokio::test]
async fn test_handle_event_generates_suggestion() {
    let tmp = TempDir::new().expect("tempdir created");
    let mut store = KnowledgeStore::open(tmp.path()).expect("store opened");

    let error = ErrorRecord {
        tool: "cargo".to_string(),
        kind: "E0599".to_string(),
        hash: "test_hash2".to_string(),
        raw_excerpt: "no method".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 1,
        last_command: "cargo build".to_string(),
    };
    store.put_error(&error).expect("error stored");

    let event = organism_protocol::ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash2".to_string(),
        tool: "cargo".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    };

    let mock = MockLlmClient {
        response: "Generated suggestion".to_string(),
    };
    let mut seen = HashSet::new();

    let outcome = handle_event(event, &mut store, &mock, &mut seen)
        .await
        .expect("outcome");
    assert_eq!(outcome, SubscriberOutcome::Generated);

    // Verify suggestion was persisted
    let stored = store.get_suggestion("test_hash2").expect("get call");
    assert!(stored.is_some());
}

#[tokio::test]
async fn test_handle_event_llm_error() {
    let tmp = TempDir::new().expect("tempdir created");
    let mut store = KnowledgeStore::open(tmp.path()).expect("store opened");

    let error = ErrorRecord {
        tool: "cargo".to_string(),
        kind: "E0599".to_string(),
        hash: "test_hash3".to_string(),
        raw_excerpt: "no method".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 1,
        last_command: "cargo build".to_string(),
    };
    store.put_error(&error).expect("error stored");

    let event = organism_protocol::ErrorClassifiedEvent {
        ts: Utc::now(),
        hash: "test_hash3".to_string(),
        tool: "cargo".to_string(),
        error_kind: "E0599".to_string(),
        command: "cargo build".to_string(),
        is_first_in_window: true,
    };

    let mut seen = HashSet::new();

    let outcome = handle_event(event, &mut store, &FailingLlmClient, &mut seen)
        .await
        .expect("outcome");
    match outcome {
        SubscriberOutcome::Skipped(ref reason) => {
            assert!(reason.contains("generation failed"));
        }
        _ => panic!("Expected Skipped outcome, got {:?}", outcome),
    }
}
