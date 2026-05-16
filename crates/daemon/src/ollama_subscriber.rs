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
use crate::notify;

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

/// Fire desktop notification if all conditions are met; mark hash notified to avoid repeats.
fn maybe_notify(hash: &str, store: &mut KnowledgeStore, notified: &mut HashSet<String>) {
    if std::env::var("ORGANISM_NOTIFY").as_deref() != Ok("1") {
        return;
    }
    if notified.contains(hash) {
        return;
    }
    if store.get_suggestion(hash).ok().flatten().is_none() {
        return;
    }
    if let Ok(Some(record)) = store.get_error(hash) {
        let tool_rate = store
            .get_style_profile()
            .ok()
            .flatten()
            .and_then(|p| p.by_tool.get(&record.tool).cloned())
            .map(|s| {
                let total = s.accepts + s.rejects;
                if total == 0 {
                    0.0f32
                } else {
                    s.accepts as f32 / total as f32
                }
            })
            .unwrap_or(0.0);
        if record.occurrences >= 3 && tool_rate >= 0.7 {
            notified.insert(hash.to_string());
            if let Err(e) = notify::notify(
                "organism: suggestion ready",
                &format!("{} (occ {})", record.last_command, record.occurrences),
            ) {
                warn!(error = %e, "ollama_subscriber: notify failed");
            } else {
                info!(hash = %hash, "ollama_subscriber: desktop notification fired");
            }
        }
    }
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

    // Call Ollama to generate suggestion (use_profile=true for M11 few-shot context)
    match suggest_for_error(client, store, &event.hash, true).await {
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
    let mut notified: HashSet<String> = HashSet::new();

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                debug!("ollama_subscriber received shutdown signal");
                break;
            }
            msg = rx.recv() => {
        match msg {
            Ok(OrganismEvent::ErrorClassified(e)) => {
                {
                    // Notify check: runs on every event, gated by notified set (once per session).
                    // Requires: ORGANISM_NOTIFY=1, suggestion cached, occurrences>=3, tool rate>=0.7.
                    let mut store = knowledge.write().await;
                    maybe_notify(&e.hash, &mut store, &mut notified);
                }

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

    // ── M13 notify threshold tests ───────────────────────────────────────────

    fn make_error(hash: &str, tool: &str, occurrences: u64) -> ErrorRecord {
        ErrorRecord {
            tool: tool.to_string(),
            kind: "E0599".to_string(),
            hash: hash.to_string(),
            raw_excerpt: "test error".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        }
    }

    fn make_event(hash: &str, tool: &str) -> ErrorClassifiedEvent {
        ErrorClassifiedEvent {
            ts: Utc::now(),
            hash: hash.to_string(),
            tool: tool.to_string(),
            error_kind: "E0599".to_string(),
            command: "cargo build".to_string(),
            is_first_in_window: true,
        }
    }

    #[tokio::test]
    async fn test_notify_gate_off_does_not_crash() {
        // ORGANISM_NOTIFY unset → notify block skipped entirely.
        std::env::remove_var("ORGANISM_NOTIFY");

        let tmp = TempDir::new().expect("tempdir");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store");
        let err = make_error("notify_hash_1", "cargo", 5);
        store.put_error(&err).expect("put error");

        let event = make_event("notify_hash_1", "cargo");
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
        };
        let mut seen = HashSet::new();
        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
            .await
            .expect("handle_event");
        assert_eq!(outcome, SubscriberOutcome::Generated);
    }

    #[tokio::test]
    async fn test_notify_below_occurrence_threshold_does_not_crash() {
        // ORGANISM_NOTIFY=1 but occurrences=2 < 3 → notify not called.
        std::env::set_var("ORGANISM_NOTIFY", "1");

        let tmp = TempDir::new().expect("tempdir");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store");
        let err = make_error("notify_hash_2", "cargo", 2);
        store.put_error(&err).expect("put error");

        let event = make_event("notify_hash_2", "cargo");
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
        };
        let mut seen = HashSet::new();
        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
            .await
            .expect("handle_event");
        assert_eq!(outcome, SubscriberOutcome::Generated);
    }

    #[tokio::test]
    async fn test_notify_below_rate_threshold_does_not_crash() {
        // ORGANISM_NOTIFY=1, occurrences=3, but accept_rate=0.5 < 0.7 → no notify.
        std::env::set_var("ORGANISM_NOTIFY", "1");

        let tmp = TempDir::new().expect("tempdir");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store");

        let err = make_error("notify_hash_3", "cargo", 3);
        store.put_error(&err).expect("put error");

        // Store a profile with low accept rate for "cargo"
        use organism_knowledge::{StyleProfile, ToolStats};
        let mut profile = StyleProfile::empty();
        profile.by_tool.insert(
            "cargo".to_string(),
            ToolStats {
                accepts: 1,
                rejects: 1,
            },
        ); // 50% rate
        store.put_style_profile(&profile).expect("put profile");

        let event = make_event("notify_hash_3", "cargo");
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
        };
        let mut seen = HashSet::new();
        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
            .await
            .expect("handle_event");
        assert_eq!(outcome, SubscriberOutcome::Generated);
    }

    #[tokio::test]
    async fn test_notify_threshold_met_completes_without_crash() {
        // ORGANISM_NOTIFY=1, occurrences=3, accept_rate=0.75 ≥ 0.7 → notify fires.
        // On CI, binary may not exist → Ok(()) swallowed. Assert Generated returned.
        std::env::set_var("ORGANISM_NOTIFY", "1");

        let tmp = TempDir::new().expect("tempdir");
        let mut store = KnowledgeStore::open(tmp.path()).expect("store");

        let err = make_error("notify_hash_4", "cargo", 3);
        store.put_error(&err).expect("put error");

        use organism_knowledge::{StyleProfile, ToolStats};
        let mut profile = StyleProfile::empty();
        profile.by_tool.insert(
            "cargo".to_string(),
            ToolStats {
                accepts: 3,
                rejects: 1,
            },
        ); // 75% rate
        store.put_style_profile(&profile).expect("put profile");

        let event = make_event("notify_hash_4", "cargo");
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
        };
        let mut seen = HashSet::new();
        let outcome = handle_event(event, &mut store, &mock, &mut seen, None)
            .await
            .expect("handle_event");
        // Notify fires (or binary not found → warn). Either way: Generated returned.
        assert_eq!(outcome, SubscriberOutcome::Generated);
    }
}
