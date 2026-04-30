//! Subscribes to the event bus, classifies failed terminal commands,
//! and persists ErrorRecord entries in the knowledge store.
//!
//! Spawned from `main.rs` after the daemon is constructed. Runs until the
//! event bus is closed.

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use organism_cortex::classify;
use organism_knowledge::{ErrorRecord, KnowledgeStore};
use organism_protocol::OrganismEvent;

use crate::event_bus::EventBus;

pub async fn run(
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
) -> Result<()> {
    let mut rx = bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(OrganismEvent::Terminal(term)) => {
                if term.exit_code == Some(0) {
                    continue;
                }
                let sig = classify(
                    &term.command_line,
                    term.exit_code,
                    term.stderr_snippet.as_deref(),
                );
                let Some(sig) = sig else { continue };
                let mut store = knowledge.write().await;
                let now = Utc::now();
                match store.get_error(&sig.hash) {
                    Ok(Some(mut existing)) => {
                        existing.occurrences = existing.occurrences.saturating_add(1);
                        existing.last_seen = now;
                        existing.last_command = term.command_line.clone();
                        if let Err(e) = store.put_error(&existing) {
                            warn!(error = %e, "failed to update ErrorRecord");
                        }
                    }
                    Ok(None) => {
                        let rec = ErrorRecord {
                            tool: sig.tool,
                            kind: sig.kind,
                            hash: sig.hash,
                            raw_excerpt: sig.raw_excerpt,
                            first_seen: now,
                            last_seen: now,
                            occurrences: 1,
                            last_command: term.command_line.clone(),
                        };
                        if let Err(e) = store.put_error(&rec) {
                            warn!(error = %e, "failed to insert ErrorRecord");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to read ErrorRecord");
                    }
                }
            }
            Ok(_) => {
                debug!("error_subscriber: ignoring non-terminal event");
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("error_subscriber lagged by {} messages", n);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                debug!("event bus closed, error_subscriber exiting");
                break;
            }
        }
    }
    Ok(())
}
