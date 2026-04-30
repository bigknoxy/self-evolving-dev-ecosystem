//! Async pub/sub event bus for the daemon.
//! Producers push events; subscribers receive them via broadcast channels.

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

    /// Publish an event to all subscribers.
    /// Returns the number of receivers that got the message.
    // Used by sensor tasks at Level 1+ (terminal/file watchers will call publish).
    #[allow(dead_code)]
    pub fn publish(&self, event: OrganismEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<OrganismEvent> {
        self.sender.subscribe()
    }
}
