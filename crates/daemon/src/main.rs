use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod daemon;
mod error_subscriber;
mod event_bus;
mod ipc;
mod ollama_subscriber;
mod sensors;

use daemon::Daemon;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    info!("Organism daemon starting...");

    // Determine data directory
    let data_dir = organism_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    info!("Data dir: {:?}", data_dir);

    let daemon = Daemon::new(data_dir.clone())?;

    {
        let state = daemon.state.read().await;
        info!(
            trust_level = %state.trust_level,
            sensors = ?state.active_sensors,
            "Daemon ready"
        );
    }

    // Spawn IPC server.
    let socket_path = ipc::socket_path_for(&data_dir);
    let ipc_state = daemon.state.clone();
    let ipc_bus = daemon.bus.clone();
    let ipc_knowledge = daemon.knowledge.clone();
    let ipc_socket = socket_path.clone();
    tokio::spawn(async move {
        if let Err(e) = ipc::serve(ipc_state, ipc_bus, ipc_knowledge, ipc_socket).await {
            tracing::error!(error = %e, "ipc server stopped");
        }
    });

    // Spawn filesystem watcher rooted at the daemon's launch directory.
    let watch_root = std::env::current_dir()?;
    let (file_shutdown_tx, file_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let watch_bus = daemon.bus.clone();
    let watch_state = daemon.state.clone();
    let file_handle = tokio::spawn(async move {
        if let Err(e) =
            sensors::file::watch(watch_bus, watch_state, watch_root, file_shutdown_rx).await
        {
            tracing::error!(error = %e, "file watcher stopped with error");
        }
    });

    // Spawn error subscriber: classifies failed terminal commands and
    // persists ErrorRecord entries.
    let err_bus = daemon.bus.clone();
    let err_knowledge = daemon.knowledge.clone();
    tokio::spawn(async move {
        if let Err(e) = error_subscriber::run(err_bus, err_knowledge).await {
            tracing::error!(error = %e, "error_subscriber stopped with error");
        }
    });

    // Spawn Ollama subscriber: generates LLM suggestions for errors (if enabled).
    let ollama_bus = daemon.bus.clone();
    let ollama_knowledge = daemon.knowledge.clone();
    tokio::spawn(async move {
        if let Err(e) = ollama_subscriber::run(ollama_bus, ollama_knowledge).await {
            tracing::error!(error = %e, "ollama_subscriber stopped with error");
        }
    });

    // Run event loop (keeps daemon alive); exits on ctrl_c.
    daemon.run_event_loop().await;

    // Signal the file watcher to stop and join.
    let _ = file_shutdown_tx.send(());
    let _ = file_handle.await;

    // Best-effort cleanup of socket file on shutdown.
    let _ = std::fs::remove_file(&socket_path);

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
