//! Subscribes to errors and generates LLM suggestions using Ollama.
//!
//! Gated by `OLLAMA_ENABLED` environment variable (default: 0/disabled).
//! When enabled, calls suggest_for_error on newly classified errors and persists
//! suggestions. Best-effort: never crashes the daemon, only logs warnings on error.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use organism_cortex::suggest_for_error;
use organism_knowledge::KnowledgeStore;
use organism_ollama::{LlmClient, OllamaClient};
use organism_protocol::{ErrorClassifiedEvent, OrganismEvent};

use crate::event_bus::EventBus;
use crate::metrics::SharedMetrics;

/// Outcome of processing an error event for suggestion generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriberOutcome {
    /// Suggestion was already cached; skipped generation.
    SkippedCached,
    /// Suggestion was generated and persisted.
    Generated,
    /// Skipped for a reason (error message).
    Skipped(String),
}

/// Handle a single ErrorClassifiedEvent: check cache and generate suggestion if needed.
///
/// This function is testable and can be called with a mock LlmClient.
/// Maintains in-memory seen set to avoid duplicate processing.
pub async fn handle_event<C: LlmClient>(
    event: ErrorClassifiedEvent,
    store: &mut KnowledgeStore,
    client: &C,
    seen: &mut HashSet<String>,
    metrics: Option<&SharedMetrics>,
) -> Result<SubscriberOutcome> {
    // Deduplicate: skip if we've already processed this error in this session
    if seen.contains(&event.hash) {
        debug!(hash = %event.hash, "ollama_subscriber: skipping duplicate error");
        return Ok(SubscriberOutcome::Skipped(
            "duplicate in session".to_string(),
        ));
    }
    seen.insert(event.hash.clone());

    // Check if suggestion is already cached
    match store.get_suggestion(&event.hash) {
        Ok(Some(_)) => {
            debug!(hash = %event.hash, "ollama_subscriber: suggestion cached for hash");
            // Bump cached counter
            if let Some(m) = metrics {
                m.write().await.suggestions_cached += 1;
            }
            return Ok(SubscriberOutcome::SkippedCached);
        }
        Err(err) => {
            return Ok(SubscriberOutcome::Skipped(format!(
                "cache check failed: {}",
                err
            )));
        }
        Ok(None) => {}
    }

    // Call Ollama to generate suggestion
    match suggest_for_error(client, store, &event.hash).await {
        Ok(text) => {
            // Persist the suggestion
            if let Err(e) = store.put_suggestion(&event.hash, &text) {
                warn!(error = %e, hash = %event.hash, "ollama_subscriber: failed to persist suggestion");
                return Ok(SubscriberOutcome::Skipped(format!("persist failed: {}", e)));
            }
            // Bump total counter
            if let Some(m) = metrics {
                m.write().await.suggestions_total += 1;
            }
            info!(hash = %event.hash, "ollama_subscriber: suggestion generated and cached");
            Ok(SubscriberOutcome::Generated)
        }
        Err(e) => {
            warn!(error = %e, hash = %event.hash, "ollama_subscriber: suggest_for_error failed");
            Ok(SubscriberOutcome::Skipped(format!(
                "generation failed: {}",
                e
            )))
        }
    }
}

pub async fn run(
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
    metrics: SharedMetrics,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    // Check if Ollama integration is enabled
    let ollama_enabled = std::env::var("OLLAMA_ENABLED")
        .unwrap_or_else(|_| "0".to_string())
        .trim()
        == "1";

    if !ollama_enabled {
        debug!("ollama_subscriber: OLLAMA_ENABLED=0, disabling");
        return Ok(());
    }

    let client = match OllamaClient::new() {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "ollama_subscriber: failed to initialize OllamaClient: {}",
                e
            );
            return Ok(());
        }
    };
    debug!(
        "ollama_subscriber: starting with base_url={}, model={}",
        client.base_url, client.model
    );

    let mut rx = bus.subscribe();
    let mut seen: HashSet<String> = HashSet::new();

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                debug!("ollama_subscriber received shutdown signal");
                break;
            }
            msg = rx.recv() => {
        match msg {
            Ok(OrganismEvent::ErrorClassified(e)) => {
                // Only process if this is the first occurrence in the 60-second window
                if !e.is_first_in_window {
                    debug!(hash = %e.hash, "ollama_subscriber: skipping duplicate within window");
                    continue;
                }
                let mut store = knowledge.write().await;
                let _ = handle_event(e, &mut store, &client, &mut seen, Some(&metrics)).await;
            }
            Ok(_) => {
                debug!("ollama_subscriber: ignoring non-error-classified event");
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("ollama_subscriber lagged by {} messages", n);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                debug!("event bus closed, ollama_subscriber exiting");
                break;
            }
        }
            }
        }
    }
    debug!("ollama_subscriber stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use organism_knowledge::ErrorRecord;
    use tempfile::TempDir;

    struct MockLlmClient {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlmClient {
        async fn generate(&self, _prompt: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingLlmClient;

    #[async_trait::async_trait]
    impl LlmClient for FailingLlmClient {
        async fn generate(&self, _prompt: &str) -> Result<String> {
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
            schema_v: 1,
        };
        store.put_error(&error).expect("error stored");
        store
            .put_suggestion("test_hash", "Try implementing the trait.")
            .expect("suggestion stored");

        let event = ErrorClassifiedEvent {
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

        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
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
            schema_v: 1,
        };
        store.put_error(&error).expect("error stored");

        let event = ErrorClassifiedEvent {
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

        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
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
            schema_v: 1,
        };
        store.put_error(&error).expect("error stored");

        let event = ErrorClassifiedEvent {
            ts: Utc::now(),
            hash: "test_hash3".to_string(),
            tool: "cargo".to_string(),
            error_kind: "E0599".to_string(),
            command: "cargo build".to_string(),
            is_first_in_window: true,
        };

        let mut seen = HashSet::new();

        let outcome = handle_event(event, &mut store, &FailingLlmClient, &mut seen, None)
            .await
            .expect("outcome");
        match outcome {
            SubscriberOutcome::Skipped(ref reason) => {
                assert!(reason.contains("generation failed"));
            }
            _ => panic!("Expected Skipped outcome, got {:?}", outcome),
        }
    }

    #[tokio::test]
    async fn test_handle_event_bumps_counters_on_cache_hit() {
        use crate::metrics;

        let tmp = TempDir::new().expect("tempdir created");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store opened");

        // Pre-populate suggestion in cache
        store
            .put_suggestion("test_hash4", "Cached suggestion")
            .expect("suggestion stored");

        let event = ErrorClassifiedEvent {
            ts: Utc::now(),
            hash: "test_hash4".to_string(),
            tool: "cargo".to_string(),
            error_kind: "E0599".to_string(),
            command: "cargo build".to_string(),
            is_first_in_window: true,
        };

        let mock = MockLlmClient {
            response: "Should not be called".to_string(),
        };
        let mut seen = HashSet::new();

        // Create shared metrics
        let metrics = metrics::new_shared();

        let outcome = handle_event(event, &mut store, &mock, &mut seen, Some(&metrics))
            .await
            .expect("outcome");
        assert_eq!(outcome, SubscriberOutcome::SkippedCached);

        // Verify counter was bumped
        let m = metrics.read().await;
        assert_eq!(m.suggestions_cached, 1);
    }

    #[tokio::test]
    async fn test_handle_event_bumps_counters_on_generation() {
        use crate::metrics;

        let tmp = TempDir::new().expect("tempdir created");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store opened");

        let error = ErrorRecord {
            tool: "cargo".to_string(),
            kind: "E0599".to_string(),
            hash: "test_hash5".to_string(),
            raw_excerpt: "no method".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        store.put_error(&error).expect("error stored");

        let event = ErrorClassifiedEvent {
            ts: Utc::now(),
            hash: "test_hash5".to_string(),
            tool: "cargo".to_string(),
            error_kind: "E0599".to_string(),
            command: "cargo build".to_string(),
            is_first_in_window: true,
        };

        let mock = MockLlmClient {
            response: "Generated suggestion".to_string(),
        };
        let mut seen = HashSet::new();

        // Create shared metrics
        let metrics = metrics::new_shared();

        let outcome = handle_event(event, &mut store, &mock, &mut seen, Some(&metrics))
            .await
            .expect("outcome");
        assert_eq!(outcome, SubscriberOutcome::Generated);

        // Verify counter was bumped
        let m = metrics.read().await;
        assert_eq!(m.suggestions_total, 1);
    }
}
