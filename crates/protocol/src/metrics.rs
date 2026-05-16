use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolMetrics {
    pub accepts: u64,
    pub rejects: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub suggestions_total: u64,
    pub suggestions_cached: u64,
    pub feedback_accept: u64,
    pub feedback_reject: u64,
    #[serde(default)]
    pub feedback_applied: u64,
    pub by_tool: HashMap<String, ToolMetrics>,
    pub since: DateTime<Utc>,
    pub prompt_version: String, // e.g. "m11-fewshot-v1" for cohort tracking
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            suggestions_total: 0,
            suggestions_cached: 0,
            feedback_accept: 0,
            feedback_reject: 0,
            feedback_applied: 0,
            by_tool: HashMap::new(),
            since: Utc::now(),
            prompt_version: "v1".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_roundtrip() {
        let mut metrics = Metrics {
            suggestions_total: 42,
            feedback_accept: 10,
            ..Metrics::default()
        };
        metrics.by_tool.insert(
            "rustfmt".to_string(),
            ToolMetrics {
                accepts: 5,
                rejects: 2,
            },
        );

        let json = serde_json::to_string(&metrics).unwrap();
        let deserialized: Metrics = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.suggestions_total, 42);
        assert_eq!(deserialized.feedback_accept, 10);
        assert_eq!(deserialized.by_tool["rustfmt"].accepts, 5);
        assert_eq!(deserialized.prompt_version, "v1");
    }
}
