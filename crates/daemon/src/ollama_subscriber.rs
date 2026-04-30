//! Subscribes to errors and generates LLM suggestions using Ollama.
//!
//! Gated by `OLLAMA_ENABLED` environment variable (default: 0/disabled).
//! When enabled, calls suggest_for_error on newly classified errors and persists
//! suggestions. Best-effort: never crashes the daemon, only logs warnings on error.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use organism_cortex::suggest_for_error;
use organism_knowledge::KnowledgeStore;
use organism_ollama::OllamaClient;
use organism_protocol::OrganismEvent;

use crate::event_bus::EventBus;

pub async fn run(bus: Arc<EventBus>, knowledge: Arc<RwLock<KnowledgeStore>>) -> Result<()> {
    // Check if Ollama integration is enabled
    let ollama_enabled = std::env::var("OLLAMA_ENABLED")
        .unwrap_or_else(|_| "0".to_string())
        .trim()
        == "1";

    if !ollama_enabled {
        debug!("ollama_subscriber: OLLAMA_ENABLED=0, disabling");
        return Ok(());
    }

    let client = OllamaClient::new();
    debug!(
        "ollama_subscriber: starting with base_url={}, model={}",
        client.base_url, client.model
    );

    let mut rx = bus.subscribe();
    let mut seen: HashSet<String> = HashSet::new();

    loop {
        match rx.recv().await {
            Ok(OrganismEvent::ErrorClassified(e)) => {
                // Deduplicate: skip if we've already processed this error
                if seen.contains(&e.hash) {
                    debug!(hash = %e.hash, "ollama_subscriber: skipping duplicate error");
                    continue;
                }
                seen.insert(e.hash.clone());

                // Check if suggestion is already cached
                {
                    let mut store = knowledge.write().await;
                    match store.get_suggestion(&e.hash) {
                        Ok(Some(_)) => {
                            debug!(hash = %e.hash, "ollama_subscriber: suggestion already cached");
                            continue;
                        }
                        Err(err) => {
                            warn!(error = %err, hash = %e.hash, "ollama_subscriber: failed to check cache");
                        }
                        Ok(None) => {}
                    }
                }

                // Call Ollama to generate suggestion
                let mut store = knowledge.write().await;
                match suggest_for_error(&client, &mut store, &e.hash).await {
                    Ok(text) => {
                        // Persist the suggestion
                        if let Err(e2) = store.put_suggestion(&e.hash, &text) {
                            warn!(error = %e2, hash = %e.hash, "ollama_subscriber: failed to persist suggestion");
                        } else {
                            info!(hash = %e.hash, "ollama_subscriber: suggestion generated and cached");
                        }
                    }
                    Err(e2) => {
                        warn!(error = %e2, hash = %e.hash, "ollama_subscriber: suggest_for_error failed");
                    }
                }
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
    Ok(())
}
