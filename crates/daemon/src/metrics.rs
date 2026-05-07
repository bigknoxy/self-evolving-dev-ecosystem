use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(unused_imports)]
pub use organism_protocol::{Metrics, ToolMetrics};

pub type SharedMetrics = Arc<RwLock<Metrics>>;

#[allow(dead_code)]
pub fn new_shared() -> SharedMetrics {
    Arc::new(RwLock::new(Metrics::default()))
}

/// Load metrics snapshot from `<dir>/metrics_snapshot.json`. Returns Default if missing or corrupt.
pub fn load_or_default(dir: &Path) -> Metrics {
    let path = dir.join("metrics_snapshot.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Metrics>(&content) {
            Ok(metrics) => metrics,
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    path = ?path,
                    "Failed to parse metrics snapshot, using default"
                );
                Metrics::default()
            }
        },
        Err(_) => {
            // File doesn't exist or couldn't be read; use default
            Metrics::default()
        }
    }
}

/// Atomic write: write tempfile in same dir, then rename. Never corrupt on partial failure.
pub async fn snapshot(metrics: &SharedMetrics, dir: &Path) -> Result<()> {
    // Read current metrics
    let metrics_data = metrics.read().await;

    // Pretty-print JSON
    let json = serde_json::to_string_pretty(&*metrics_data)?;
    drop(metrics_data); // Release read lock early

    // Write to temp file in same directory
    let target_path = dir.join("metrics_snapshot.json");
    let tmp_path = dir.join("metrics_snapshot.json.tmp");

    tokio::fs::write(&tmp_path, json).await?;

    // Atomic rename
    tokio::fs::rename(&tmp_path, &target_path).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_increments_then_snapshot_roundtrip() {
        // Create temp directory
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        // Create shared metrics and modify
        let metrics = new_shared();
        {
            let mut m = metrics.write().await;
            m.suggestions_total = 5;
            m.feedback_accept = 3;
            m.feedback_reject = 2;
            m.by_tool.insert(
                "rustfmt".to_string(),
                ToolMetrics {
                    accepts: 10,
                    rejects: 1,
                },
            );
        }

        // Snapshot to disk
        snapshot(&metrics, dir_path).await.unwrap();

        // Load from disk
        let loaded = load_or_default(dir_path);

        // Verify all fields match
        assert_eq!(loaded.suggestions_total, 5);
        assert_eq!(loaded.feedback_accept, 3);
        assert_eq!(loaded.feedback_reject, 2);
        assert_eq!(loaded.by_tool.len(), 1);
        assert_eq!(loaded.by_tool["rustfmt"].accepts, 10);
        assert_eq!(loaded.by_tool["rustfmt"].rejects, 1);
    }

    #[test]
    fn test_load_missing_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        // Load from empty directory
        let loaded = load_or_default(dir_path);

        // Verify defaults
        assert_eq!(loaded.suggestions_total, 0);
        assert_eq!(loaded.suggestions_cached, 0);
        assert_eq!(loaded.feedback_accept, 0);
        assert_eq!(loaded.feedback_reject, 0);
        assert!(loaded.by_tool.is_empty());
        assert_eq!(loaded.prompt_version, "v1");
    }

    #[test]
    fn test_load_corrupt_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        // Write garbage to metrics file
        let metrics_path = dir_path.join("metrics_snapshot.json");
        std::fs::write(&metrics_path, "{ garbage json }").unwrap();

        // Load should return default without panicking
        let loaded = load_or_default(dir_path);

        // Verify defaults
        assert_eq!(loaded.suggestions_total, 0);
        assert_eq!(loaded.suggestions_cached, 0);
        assert!(loaded.by_tool.is_empty());
    }
}
