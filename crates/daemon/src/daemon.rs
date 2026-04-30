//! Core daemon: lifecycle management, sensor orchestration, event routing.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::{info, warn};

use organism_knowledge::KnowledgeStore;

use crate::event_bus::EventBus;

const MAX_RECENT_EVENTS: usize = 10;

// Stub variants used at Level 1+ (cortex/effector wiring); kept for protocol stability.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TrustLevel {
    Observer,
    Ask,
    Assist,
    Autonomous,
    Uncaged,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Observer => write!(f, "observer"),
            Self::Ask => write!(f, "ask"),
            Self::Assist => write!(f, "assist"),
            Self::Autonomous => write!(f, "autonomous"),
            Self::Uncaged => write!(f, "uncaged"),
        }
    }
}

/// A short summary of a recent event, retained in a ring buffer for `log` RPC.
#[derive(Debug, Clone)]
pub struct RecentEvent {
    pub ts: DateTime<Utc>,
    pub msg: String,
}

pub struct DaemonState {
    pub trust_level: TrustLevel,
    pub event_count: u64,
    pub started_at: Instant,
    // DateTime form of start_time, available for future status payloads.
    #[allow(dead_code)]
    pub start_time: DateTime<Utc>,
    pub awake: bool,
    pub active_sensors: Vec<String>,
    pub recent_events: VecDeque<RecentEvent>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            trust_level: TrustLevel::Ask,
            event_count: 0,
            started_at: Instant::now(),
            start_time: Utc::now(),
            awake: true,
            active_sensors: vec!["terminal".to_string(), "filesystem".to_string()],
            recent_events: VecDeque::with_capacity(MAX_RECENT_EVENTS),
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn record_event(&mut self, msg: String) {
        self.event_count += 1;
        if self.recent_events.len() >= MAX_RECENT_EVENTS {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(RecentEvent {
            ts: Utc::now(),
            msg,
        });
    }
}

pub struct Daemon {
    pub bus: Arc<EventBus>,
    pub state: Arc<RwLock<DaemonState>>,
    // Consumed by cortex pattern engine at Level 1+ (event loop will read/write fixes).
    #[allow(dead_code)]
    pub knowledge: Arc<RwLock<KnowledgeStore>>,
}

impl Daemon {
    pub fn new(knowledge_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        let knowledge = KnowledgeStore::open(&knowledge_dir)?;
        Ok(Self {
            bus: Arc::new(EventBus::new(1024)),
            state: Arc::new(RwLock::new(DaemonState::new())),
            knowledge: Arc::new(RwLock::new(knowledge)),
        })
    }

    /// Main event processing loop: subscribe to bus, process events.
    /// Exits on SIGINT (ctrl_c) or when the bus is closed.
    pub async fn run_event_loop(&self) {
        let mut rx = self.bus.subscribe();
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("ctrl_c received, shutting down event loop");
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(event) => {
                            // Recording happens on the producer side (ipc/sensors).
                            // This loop is the consumer hook for cortex/effector at L1+.
                            tracing::debug!(?event, "event received");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Event bus lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Event bus closed, stopping event loop");
                            break;
                        }
                    }
                }
            }
        }
    }
}
