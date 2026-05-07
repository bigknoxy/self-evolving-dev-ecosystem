use std::path::PathBuf;
use tokio::sync::broadcast;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod clipboard;
mod daemon;
mod error_subscriber;
mod event_bus;
mod ipc;
mod metrics;
mod ollama_subscriber;
mod sensors;

use daemon::Daemon;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Determine data directory FIRST for logging setup
    let data_dir = organism_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    // Initialize file logging before any tokio::spawn
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;

    // Create rolling file appender with daily rotation and max 7 files
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("daemon.log")
        .build(&log_dir)?;

    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);

    // Initialize structured logging with file output
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".parse().unwrap()),
        )
        .json()
        .init();

    info!("Organism daemon starting...");
    info!(data_dir = ?data_dir, "Data dir initialized");

    // NOTE: Dropping _log_guard truncates pending log writes.
    // The guard must be held for the entire daemon lifetime (main() scope).

    // Create shutdown broadcast channel (capacity 1)
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Spawn signal handler (Unix only)
    #[cfg(unix)]
    {
        let signal_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to create SIGTERM handler");
            let mut sigint =
                signal(SignalKind::interrupt()).expect("failed to create SIGINT handler");
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("SIGTERM received");
                }
                _ = sigint.recv() => {
                    info!("SIGINT received");
                }
            }
            let _ = signal_tx.send(());
        });
    }

    let daemon = Daemon::new(data_dir.clone())?;

    // Load metrics from snapshot; wrap in Arc<RwLock>
    let loaded_metrics = metrics::load_or_default(&data_dir);
    let shared_metrics = std::sync::Arc::new(tokio::sync::RwLock::new(loaded_metrics));

    {
        let state = daemon.state.read().await;
        info!(
            trust_level = %state.trust_level,
            sensors = ?state.active_sensors,
            "Daemon ready"
        );
    }

    // Spawn hourly snapshot task
    let snapshot_metrics = shared_metrics.clone();
    let snapshot_dir = data_dir.clone();
    tokio::spawn(async move {
        // Honor env override for testability (default 3600 seconds)
        let interval_secs = std::env::var("ORGANISM_METRICS_SNAPSHOT_INTERVAL_SECS")
            .unwrap_or_else(|_| "3600".to_string())
            .parse::<u64>()
            .unwrap_or(3600);

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            if let Err(e) = metrics::snapshot(&snapshot_metrics, &snapshot_dir).await {
                tracing::warn!(error = %e, "failed to snapshot metrics");
            }
        }
    });

    // Spawn IPC server with shutdown channel
    let socket_path = ipc::socket_path_for(&data_dir);
    let ipc_state = daemon.state.clone();
    let ipc_bus = daemon.bus.clone();
    let ipc_knowledge = daemon.knowledge.clone();
    let ipc_metrics = shared_metrics.clone();
    let ipc_socket = socket_path.clone();
    let ipc_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) = ipc::serve(
            ipc_state,
            ipc_bus,
            ipc_knowledge,
            ipc_metrics,
            ipc_socket,
            ipc_shutdown,
        )
        .await
        {
            tracing::error!(error = %e, "ipc server stopped");
        }
    });

    // Spawn filesystem watcher rooted at the daemon's launch directory
    let watch_root = std::env::current_dir()?;
    let watch_bus = daemon.bus.clone();
    let watch_state = daemon.state.clone();
    let file_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) =
            sensors::file::watch(watch_bus, watch_state, watch_root, file_shutdown).await
        {
            tracing::error!(error = %e, "file watcher stopped with error");
        }
    });

    // Spawn error subscriber: classifies failed terminal commands and
    // persists ErrorRecord entries.
    let err_bus = daemon.bus.clone();
    let err_knowledge = daemon.knowledge.clone();
    let err_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) = error_subscriber::run(err_bus, err_knowledge, err_shutdown).await {
            tracing::error!(error = %e, "error_subscriber stopped with error");
        }
    });

    // Spawn Ollama subscriber: generates LLM suggestions for errors (if enabled).
    let ollama_bus = daemon.bus.clone();
    let ollama_knowledge = daemon.knowledge.clone();
    let ollama_metrics = shared_metrics.clone();
    let ollama_shutdown = shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) = ollama_subscriber::run(
            ollama_bus,
            ollama_knowledge,
            ollama_metrics,
            ollama_shutdown,
        )
        .await
        {
            tracing::error!(error = %e, "ollama_subscriber stopped with error");
        }
    });

    // Run event loop (keeps daemon alive); exits on ctrl_c.
    daemon.run_event_loop().await;

    // Best-effort cleanup of socket file on shutdown.
    let _ = std::fs::remove_file(&socket_path);

    info!("Organism daemon stopped");
    Ok(())
}

/// Resolve the organism data directory.
/// Honors `ORGANISM_HOME` if set; otherwise `$HOME/.organism`.
fn organism_data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("ORGANISM_HOME") {
        return PathBuf::from(override_dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    home.join(".organism")
}
