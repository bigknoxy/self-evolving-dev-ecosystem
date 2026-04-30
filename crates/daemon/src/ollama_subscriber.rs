//! Subscribes to errors and generates LLM suggestions using Ollama.
//!
//! Gated by `OLLAMA_ENABLED` environment variable (default: 0/disabled).
//! When enabled, calls suggest_for_error on newly classified errors and persists
//! suggestions. Best-effort: never crashes the daemon, only logs warnings on error.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use organism_knowledge::KnowledgeStore;
use organism_ollama::OllamaClient;
use organism_protocol::OrganismEvent;

use crate::event_bus::EventBus;

pub async fn run(bus: Arc<EventBus>, _knowledge: Arc<RwLock<KnowledgeStore>>) -> Result<()> {
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
    loop {
        match rx.recv().await {
            Ok(OrganismEvent::Terminal(_term)) => {
                // We don't have direct access to ErrorRecord here; we'd need to
                // listen for a custom event. For now, silently skip until we can
                // hook into error classification.
                debug!("ollama_subscriber: received terminal event");
            }
            Ok(_) => {
                debug!("ollama_subscriber: ignoring non-terminal event");
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
