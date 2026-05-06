//! Subscribes to the event bus, classifies failed terminal commands,
//! and persists ErrorRecord entries in the knowledge store.
//!
//! Spawned from `main.rs` after the daemon is constructed. Runs until the
//! event bus is closed.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, Utc};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, warn};

use organism_cortex::classify;
use organism_knowledge::{ErrorRecord, KnowledgeStore};
use organism_protocol::{ErrorClassifiedEvent, OrganismEvent};

use crate::event_bus::EventBus;

pub async fn run(
    bus: Arc<EventBus>,
    knowledge: Arc<RwLock<KnowledgeStore>>,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    let mut rx = bus.subscribe();
    let mut window_state: HashMap<String, chrono::DateTime<Utc>> = HashMap::new();
    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                debug!("error_subscriber received shutdown signal");
                break;
            }
            msg = rx.recv() => {
        match msg {
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

                // Clone sig fields before consuming in ErrorRecord construction
                let sig_hash = sig.hash.clone();
                let sig_tool = sig.tool.clone();
                let sig_kind = sig.kind.clone();

                match store.get_error(&sig_hash) {
                    Ok(Some(mut existing)) => {
                        existing.occurrences = existing.occurrences.saturating_add(1);
                        existing.last_seen = now;
                        existing.last_command = term.command_line.clone();
                        if let Err(e) = store.put_error(&existing) {
                            warn!(error = %e, "failed to update ErrorRecord");
                        }

                        // Check 60-second window: is this the first occurrence in the window?
                        let is_first_in_window = match window_state.get(&sig_hash) {
                            Some(last_seen) => {
                                let elapsed = now.signed_duration_since(*last_seen);
                                elapsed > Duration::seconds(60)
                            }
                            None => true,
                        };

                        // Update window state
                        window_state.insert(sig_hash.clone(), now);

                        // Emit ErrorClassified event after update
                        let _ = bus.publish(OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
                            ts: now,
                            hash: sig_hash,
                            tool: sig_tool,
                            error_kind: sig_kind,
                            command: term.command_line.clone(),
                            is_first_in_window,
                        }));
                    }
                    Ok(None) => {
                        let rec = ErrorRecord {
                            tool: sig_tool.clone(),
                            kind: sig_kind.clone(),
                            hash: sig_hash.clone(),
                            raw_excerpt: sig.raw_excerpt,
                            first_seen: now,
                            last_seen: now,
                            occurrences: 1,
                            last_command: term.command_line.clone(),
                            schema_v: 1,
                        };
                        if let Err(e) = store.put_error(&rec) {
                            warn!(error = %e, "failed to insert ErrorRecord");
                        }

                        // First occurrence is always first in window
                        window_state.insert(sig_hash.clone(), now);

                        // Emit ErrorClassified event after insert
                        let _ = bus.publish(OrganismEvent::ErrorClassified(ErrorClassifiedEvent {
                            ts: now,
                            hash: sig_hash,
                            tool: sig_tool,
                            error_kind: sig_kind,
                            command: term.command_line.clone(),
                            is_first_in_window: true,
                        }));
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
        }
    }
    debug!("error_subscriber stopped");
    Ok(())
}
