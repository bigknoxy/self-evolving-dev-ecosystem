use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::data_dir;

// Local Metrics struct for client-side use
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
    pub by_tool: HashMap<String, ToolMetrics>,
    pub since: DateTime<Utc>,
    pub prompt_version: String,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            suggestions_total: 0,
            suggestions_cached: 0,
            feedback_accept: 0,
            feedback_reject: 0,
            by_tool: HashMap::new(),
            since: Utc::now(),
            prompt_version: "v1".to_string(),
        }
    }
}

/// Parse duration strings like "7d", "24h", "30m" into seconds.
pub fn parse_duration(s: &str) -> Result<u64> {
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }

    // Extract the numeric part and unit suffix
    let (num_part, unit) = if let Some(pos) = s.chars().position(|c| !c.is_ascii_digit()) {
        s.split_at(pos)
    } else {
        anyhow::bail!(
            "duration string '{}' has no unit suffix (expected 7d, 24h, 30m, etc.)",
            s
        );
    };

    let num: u64 = num_part
        .parse()
        .context(format!("invalid numeric part in duration '{}'", s))?;

    let secs = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => {
            anyhow::bail!(
                "unknown duration unit '{}' in '{}' (expected s, m, h, or d)",
                unit,
                s
            );
        }
    };

    Ok(secs)
}

/// Delta between two Metrics structs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetricsDelta {
    pub suggestions_total: u64,
    pub suggestions_cached: u64,
    pub feedback_accept: u64,
    pub feedback_reject: u64,
    pub by_tool: HashMap<String, ToolDelta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDelta {
    pub accepts: u64,
    pub rejects: u64,
}

/// Compute delta using saturating subtraction.
pub fn compute_delta(current: &Metrics, baseline: &Metrics) -> MetricsDelta {
    let mut by_tool = HashMap::new();

    // Process all tools from current
    for (tool_name, tool_metrics) in &current.by_tool {
        let baseline_tool = baseline.by_tool.get(tool_name);
        let base_accepts = baseline_tool.map(|t| t.accepts).unwrap_or(0);
        let base_rejects = baseline_tool.map(|t| t.rejects).unwrap_or(0);

        by_tool.insert(
            tool_name.clone(),
            ToolDelta {
                accepts: tool_metrics.accepts.saturating_sub(base_accepts),
                rejects: tool_metrics.rejects.saturating_sub(base_rejects),
            },
        );
    }

    MetricsDelta {
        suggestions_total: current
            .suggestions_total
            .saturating_sub(baseline.suggestions_total),
        suggestions_cached: current
            .suggestions_cached
            .saturating_sub(baseline.suggestions_cached),
        feedback_accept: current
            .feedback_accept
            .saturating_sub(baseline.feedback_accept),
        feedback_reject: current
            .feedback_reject
            .saturating_sub(baseline.feedback_reject),
        by_tool,
    }
}

/// Format acceptance ratio with optional baseline delta.
fn format_acceptance_ratio(accepts: u64, rejects: u64) -> String {
    let total = accepts + rejects;
    if total == 0 {
        "n/a".to_string()
    } else {
        let pct = (accepts as f64 / total as f64) * 100.0;
        format!("{}/{} = {:.1}%", accepts, total, pct)
    }
}

/// Pretty-print Metrics struct (human-readable format).
fn format_metrics_human(metrics: &Metrics, delta: Option<&MetricsDelta>) -> String {
    let mut output = String::new();

    // Header with timestamp
    output.push_str("Metrics\n");
    output.push_str(&format!("  since: {}\n", metrics.since));
    output.push_str(&format!("  prompt version: {}\n", metrics.prompt_version));
    output.push('\n');

    // Current metrics
    output.push_str("Current:\n");
    output.push_str(&format!(
        "  suggestions total: {}\n",
        metrics.suggestions_total
    ));
    output.push_str(&format!(
        "  suggestions cached: {}\n",
        metrics.suggestions_cached
    ));
    output.push_str(&format!("  feedback accept: {}\n", metrics.feedback_accept));
    output.push_str(&format!("  feedback reject: {}\n", metrics.feedback_reject));

    let acceptance = format_acceptance_ratio(metrics.feedback_accept, metrics.feedback_reject);
    output.push_str(&format!("  acceptance: {}\n", acceptance));

    // By-tool breakdown
    if !metrics.by_tool.is_empty() {
        output.push_str("\nBy tool:\n");
        let mut tools: Vec<_> = metrics.by_tool.iter().collect();
        tools.sort_by_key(|(name, _)| *name);
        for (tool_name, tool_metrics) in tools {
            let tool_acceptance =
                format_acceptance_ratio(tool_metrics.accepts, tool_metrics.rejects);
            output.push_str(&format!(
                "  {}: {} accepts, {} rejects ({})\n",
                tool_name, tool_metrics.accepts, tool_metrics.rejects, tool_acceptance
            ));
        }
    }

    // Delta section (if baseline provided)
    if let Some(delta) = delta {
        output.push_str("\nDelta vs baseline:\n");
        output.push_str(&format!(
            "  suggestions total: +{}\n",
            delta.suggestions_total
        ));
        output.push_str(&format!(
            "  suggestions cached: +{}\n",
            delta.suggestions_cached
        ));
        output.push_str(&format!("  feedback accept: +{}\n", delta.feedback_accept));
        output.push_str(&format!("  feedback reject: +{}\n", delta.feedback_reject));

        let delta_acceptance =
            format_acceptance_ratio(delta.feedback_accept, delta.feedback_reject);
        output.push_str(&format!("  acceptance: {}\n", delta_acceptance));

        if !delta.by_tool.is_empty() {
            output.push_str("\nBy tool (delta):\n");
            let mut tools: Vec<_> = delta.by_tool.iter().collect();
            tools.sort_by_key(|(name, _)| *name);
            for (tool_name, tool_delta) in tools {
                if tool_delta.accepts > 0 || tool_delta.rejects > 0 {
                    output.push_str(&format!(
                        "  {}: +{} accepts, +{} rejects\n",
                        tool_name, tool_delta.accepts, tool_delta.rejects
                    ));
                }
            }
        }
    }

    output
}

pub struct StatsArgs {
    pub json: bool,
    pub capture_baseline: bool,
    pub baseline: bool,
    pub since: Option<String>,
}

pub fn cmd_stats(args: &StatsArgs) -> Result<()> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).context("creating data directory")?;

    // Handle capture-baseline mode
    if args.capture_baseline {
        let snapshot_path = dir.join("metrics_snapshot.json");
        let baseline_path = dir.join("metrics_baseline.json");

        if !snapshot_path.exists() {
            anyhow::bail!(
                "no metrics snapshot yet (expected at {})",
                snapshot_path.display()
            );
        }

        let content = std::fs::read(&snapshot_path).context("reading metrics_snapshot.json")?;
        std::fs::write(&baseline_path, content).context("writing metrics_baseline.json")?;

        let now = chrono::Utc::now().to_rfc3339();
        println!("baseline captured at {}", now);
        return Ok(());
    }

    // Load current metrics
    let current = load_metrics_from_file(&dir.join("metrics_snapshot.json"))?;

    // Load baseline if it exists and relevant
    let baseline = if args.baseline || args.json {
        load_baseline(&dir)
    } else {
        None
    };

    // Parse --since duration if provided
    let since_display = if let Some(since_str) = &args.since {
        parse_duration(since_str).context(format!("invalid --since value '{}'", since_str))?;
        format!(", showing metrics since {}", since_str)
    } else {
        String::new()
    };

    // JSON output
    if args.json {
        if let Some(baseline) = baseline {
            let delta = compute_delta(&current, &baseline);
            let output = serde_json::json!({
                "current": current,
                "baseline": baseline,
                "delta": delta,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&current)?);
        }
        return Ok(());
    }

    // Human-readable output
    let delta = baseline.as_ref().map(|b| compute_delta(&current, b));
    let formatted = format_metrics_human(&current, delta.as_ref());
    println!("{}{}", formatted, since_display);

    Ok(())
}

fn load_metrics_from_file(path: &Path) -> Result<Metrics> {
    if !path.exists() {
        // Return a sensible default message
        return Ok(Metrics {
            prompt_version: "(no data)".to_string(),
            ..Default::default()
        });
    }

    let content = std::fs::read_to_string(path).context("reading metrics file")?;
    let metrics: Metrics = serde_json::from_str(&content).context("parsing metrics JSON")?;
    Ok(metrics)
}

fn load_baseline(dir: &Path) -> Option<Metrics> {
    let baseline_path = dir.join("metrics_baseline.json");
    if !baseline_path.exists() {
        return None;
    }

    std::fs::read_to_string(&baseline_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Metrics>(&content).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_format_no_data() {
        let dir = TempDir::new().unwrap();
        let args = StatsArgs {
            json: false,
            capture_baseline: false,
            baseline: false,
            since: None,
        };

        // Point to empty dir
        std::env::set_var("ORGANISM_HOME", dir.path());
        let result = cmd_stats(&args);
        assert!(result.is_ok(), "should handle missing metrics gracefully");
    }

    #[test]
    fn test_delta_basic() {
        let mut current = Metrics::default();
        current.feedback_accept = 5;
        current.feedback_reject = 0;

        let mut baseline = Metrics::default();
        baseline.feedback_accept = 2;
        baseline.feedback_reject = 0;

        let delta = compute_delta(&current, &baseline);
        assert_eq!(delta.feedback_accept, 3);
        assert_eq!(delta.feedback_reject, 0);
    }

    #[test]
    fn test_delta_saturating() {
        let mut current = Metrics::default();
        current.feedback_accept = 2;

        let mut baseline = Metrics::default();
        baseline.feedback_accept = 5;

        let delta = compute_delta(&current, &baseline);
        // saturating_sub should return 0, not panic
        assert_eq!(delta.feedback_accept, 0);
    }

    #[test]
    fn test_delta_by_tool() {
        let mut current = Metrics::default();
        current.by_tool.insert(
            "rustfmt".to_string(),
            ToolMetrics {
                accepts: 10,
                rejects: 1,
            },
        );

        let mut baseline = Metrics::default();
        baseline.by_tool.insert(
            "rustfmt".to_string(),
            ToolMetrics {
                accepts: 6,
                rejects: 1,
            },
        );

        let delta = compute_delta(&current, &baseline);
        assert_eq!(delta.by_tool["rustfmt"].accepts, 4);
        assert_eq!(delta.by_tool["rustfmt"].rejects, 0);
    }

    #[test]
    fn test_capture_baseline_copies_file() {
        let dir = TempDir::new().unwrap();
        let snapshot_path = dir.path().join("metrics_snapshot.json");
        let baseline_path = dir.path().join("metrics_baseline.json");

        // Write metrics_snapshot.json
        let metrics = Metrics::default();
        let json = serde_json::to_string_pretty(&metrics).unwrap();
        std::fs::write(&snapshot_path, &json).unwrap();

        std::env::set_var("ORGANISM_HOME", dir.path());

        let args = StatsArgs {
            json: false,
            capture_baseline: true,
            baseline: false,
            since: None,
        };

        let result = cmd_stats(&args);
        assert!(result.is_ok(), "capture should succeed");
        assert!(baseline_path.exists(), "baseline file should exist");

        let baseline_content = std::fs::read_to_string(&baseline_path).unwrap();
        assert_eq!(baseline_content, json, "baseline should match snapshot");
    }

    #[test]
    fn test_capture_baseline_errors_without_snapshot() {
        let dir = TempDir::new().unwrap();
        std::env::set_var("ORGANISM_HOME", dir.path());

        let args = StatsArgs {
            json: false,
            capture_baseline: true,
            baseline: false,
            since: None,
        };

        let result = cmd_stats(&args);
        assert!(
            result.is_err(),
            "capture should error when no snapshot exists"
        );
    }

    #[test]
    fn test_parse_since_durations() {
        assert_eq!(parse_duration("7d").unwrap(), 7 * 86400);
        assert_eq!(parse_duration("24h").unwrap(), 24 * 3600);
        assert_eq!(parse_duration("30m").unwrap(), 30 * 60);
        assert_eq!(parse_duration("60s").unwrap(), 60);
    }

    #[test]
    fn test_parse_since_invalid() {
        assert!(parse_duration("bogus").is_err());
        assert!(parse_duration("7x").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_format_acceptance_ratio_zero() {
        let ratio = format_acceptance_ratio(0, 0);
        assert_eq!(ratio, "n/a");
    }

    #[test]
    fn test_format_acceptance_ratio_perfect() {
        let ratio = format_acceptance_ratio(10, 0);
        assert!(ratio.contains("100.0%"));
    }

    #[test]
    fn test_format_acceptance_ratio_half() {
        let ratio = format_acceptance_ratio(5, 5);
        assert!(ratio.contains("50.0%"));
    }
}
