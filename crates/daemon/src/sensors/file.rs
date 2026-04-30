//! Filesystem watcher sensor.
//!
//! Watches a root directory recursively, debounces noisy events, filters out
//! ignored paths (target/, .git/, dotfiles), and publishes `OrganismEvent::File`
//! to the bus. Recording into `DaemonState` happens on the producer side.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use notify::{recommended_watcher, EventKind, RecursiveMode, Watcher};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use organism_protocol::{EventContext, FileEvent, FileEventType, OrganismEvent};

use crate::daemon::DaemonState;
use crate::event_bus::EventBus;

const DEBOUNCE_WINDOW_MS: u64 = 200;

/// Returns true if any component of `path` *relative to* `root` is
/// `target`, `.git`, or a dotfile/dotdir. Components in/above the root
/// (e.g. `/private/var/folders/.../.tmpXYZ`) are not considered, so
/// tempdir prefixes don't cause false positives. Compares canonicalized
/// paths so symlink resolution (e.g. /var → /private/var on macOS) lines up.
fn should_ignore(path: &Path, root: &Path) -> bool {
    let canon_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let rel = match canon_path.strip_prefix(&canon_root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    for comp in rel.components() {
        if let std::path::Component::Normal(os) = comp {
            let s = os.to_string_lossy();
            if s == "target" || s == ".git" || s.starts_with('.') {
                return true;
            }
        }
    }
    false
}

fn map_kind(kind: &EventKind) -> Option<FileEventType> {
    match kind {
        EventKind::Create(_) => Some(FileEventType::Create),
        EventKind::Modify(_) => Some(FileEventType::Modify),
        EventKind::Remove(_) => Some(FileEventType::Delete),
        _ => None,
    }
}

/// Watch `root` for filesystem changes, publishing debounced events to `bus`.
/// Records each emitted event into `state` before publishing.
/// Exits cleanly when `shutdown` resolves.
pub async fn watch(
    bus: Arc<EventBus>,
    state: Arc<RwLock<DaemonState>>,
    root: PathBuf,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    info!(?root, "file watcher starting");

    // notify uses std mpsc; bridge to tokio mpsc via spawn_blocking.
    let (std_tx, std_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let (tokio_tx, mut tokio_rx) = mpsc::channel::<notify::Event>(1024);

    let mut watcher = recommended_watcher(move |res: notify::Result<notify::Event>| {
        let _ = std_tx.send(res);
    })
    .context("failed to create filesystem watcher")?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {:?}", root))?;

    // Bridge std mpsc -> tokio mpsc on a blocking thread.
    let bridge = tokio::task::spawn_blocking(move || {
        while let Ok(res) = std_rx.recv() {
            match res {
                Ok(ev) => {
                    if tokio_tx.blocking_send(ev).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "notify watcher error");
                }
            }
        }
    });

    // Debounce buffer keyed by (path, kind).
    let mut pending: HashMap<(String, FileEventType), ()> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(DEBOUNCE_WINDOW_MS));
    tick.tick().await; // skip immediate first tick

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!("file watcher shutdown signal received");
                break;
            }
            maybe_ev = tokio_rx.recv() => {
                match maybe_ev {
                    Some(ev) => {
                        let Some(kind) = map_kind(&ev.kind) else { continue };
                        for path in ev.paths {
                            if should_ignore(&path, &root) {
                                debug!(?path, "ignored path");
                                continue;
                            }
                            let path_str = path.to_string_lossy().into_owned();
                            pending.insert((path_str, kind), ());
                        }
                    }
                    None => {
                        debug!("notify channel closed");
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if pending.is_empty() { continue; }
                let drained: Vec<_> = pending.drain().collect();
                for ((path, kind), _) in drained {
                    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let evt = FileEvent {
                        ts: Utc::now(),
                        path: path.clone(),
                        event_type: kind,
                        size_bytes,
                        context: EventContext::default(),
                    };
                    let organism_evt = OrganismEvent::File(evt);
                    {
                        let mut s = state.write().await;
                        s.record_event(format!("{:?}", organism_evt));
                    }
                    let _ = bus.publish(organism_evt);
                }
            }
        }
    }

    // Drop watcher first so the std channel closes; then wait for bridge.
    drop(watcher);
    let _ = bridge.await;
    info!("file watcher stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ignores_target_path() {
        let root = PathBuf::from("/proj");
        assert!(should_ignore(&PathBuf::from("/proj/target/foo"), &root));
        assert!(should_ignore(&PathBuf::from("/proj/.git/HEAD"), &root));
        assert!(should_ignore(&PathBuf::from("/proj/.hidden"), &root));
        assert!(!should_ignore(&PathBuf::from("/proj/src/main.rs"), &root));
        // Tempdir-style root with leading dot must not poison children.
        let dot_root = PathBuf::from("/var/.tmpXYZ");
        assert!(!should_ignore(
            &PathBuf::from("/var/.tmpXYZ/hello.txt"),
            &dot_root
        ));
    }

    #[test]
    fn maps_event_kinds() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind};
        assert_eq!(
            map_kind(&EventKind::Create(CreateKind::File)),
            Some(FileEventType::Create)
        );
        assert_eq!(
            map_kind(&EventKind::Modify(ModifyKind::Any)),
            Some(FileEventType::Modify)
        );
        assert_eq!(
            map_kind(&EventKind::Remove(RemoveKind::File)),
            Some(FileEventType::Delete)
        );
        assert_eq!(map_kind(&EventKind::Any), None);
    }
}
