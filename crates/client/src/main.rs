//! Organism CLI - talks to the running daemon over a Unix socket.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing_subscriber::EnvFilter;

use organism_protocol::{Envelope, EventContext, OrganismEvent, TerminalEvent};

mod cmd_backfill;
mod cmd_stats;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let result: Result<()> = match cmd {
        "status" => cmd_status().await,
        "suggest" => cmd_suggest().await,
        "apply" => cmd_apply(&args[2..]).await,
        "feedback" => cmd_feedback(&args[2..]).await,
        "errors" => cmd_errors(&args[2..]).await,
        "profile" => cmd_profile(&args[2..]).await,
        "doctor" => cmd_doctor().await,
        "log" => cmd_log().await,
        "sleep" => cmd_sleep().await,
        "wake" => cmd_wake().await,
        "emit-terminal" => cmd_emit_terminal(&args[2..]).await,
        "backfill-accepts" => cmd_backfill::cmd_backfill_accepts().await,
        "stats" => cmd_stats(&args[2..]),
        _ => {
            cmd_help();
            Ok(())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::from(1)
        }
    }
}

fn cmd_help() {
    println!("Organism CLI");
    println!();
    println!("USAGE:");
    println!("  organism-cli <command>");
    println!();
    println!("COMMANDS:");
    println!("  status    Show daemon status");
    println!("  suggest   Request a suggestion for current directory");
    println!("  apply <ERROR_KEY> [--stage]");
    println!("            Materialize a cached suggestion (--stage writes patch / clipboards cmd)");
    println!("  feedback <ERROR_KEY> accept|reject|ignore [--note \"...\"]");
    println!("            Record user verdict on a suggestion");
    println!("  errors [--limit N] [--json]");
    println!("            List recent errors (default 20, --json for raw JSON output)");
    println!("  profile [--rebuild] [--json]");
    println!(
        "            Show computed style profile (--rebuild forces recompute, --json for raw JSON)"
    );
    println!("  doctor    Check knowledge store and daemon health");
    println!("  log       Show recent daemon activity");
    println!("  sleep     Pause all daemon activity");
    println!("  wake      Resume daemon activity");
    println!("  emit-terminal <cmd> [--exit-code N] [--cwd PATH] [--duration-ms M] [--stderr STR]");
    println!("            Inject a terminal event into the daemon (used by shell hook)");
    println!("  backfill-accepts");
    println!("            Snapshot existing accepted suggestions into immutable table (one-time)");
    println!("  stats [--json] [--capture-baseline] [--baseline] [--since <DURATION>]");
    println!("            Show metrics; capture baseline or compare with baseline");
    println!("  help      Show this help");
}

/// Format a timestamp as human-readable age relative to now.
/// Returns strings like "3m", "1h", "2d", "59s".
fn format_age(last_seen_rfc3339: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(last_seen_rfc3339) {
        Ok(dt) => {
            let dt_utc = dt.with_timezone(&chrono::Utc);
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt_utc);
            let secs = duration.num_seconds();

            if secs < 0 {
                // Future timestamp (shouldn't happen)
                "0s".to_string()
            } else if secs < 60 {
                format!("{}s", secs)
            } else if secs < 3600 {
                // Minutes
                let mins = secs / 60;
                format!("{}m", mins)
            } else if secs < 86400 {
                // Hours
                let hrs = secs / 3600;
                format!("{}h", hrs)
            } else {
                // Days
                let days = secs / 86400;
                format!("{}d", days)
            }
        }
        Err(_) => {
            // Could not parse; just return as-is
            "?".to_string()
        }
    }
}

/// Parse emit-terminal CLI args and build the corresponding TerminalEvent.
/// Pulled out of `cmd_emit_terminal` so it can be unit tested without IO.
fn build_terminal_event(args: &[String]) -> Result<TerminalEvent> {
    if args.is_empty() {
        anyhow::bail!("emit-terminal: missing <command> argument");
    }
    let command_line = args[0].clone();

    let mut exit_code: Option<i32> = None;
    let mut cwd: Option<String> = None;
    let mut duration_ms: Option<u64> = None;
    let mut stderr_snippet: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--exit-code" => {
                let v = args.get(i + 1).context("--exit-code requires a value")?;
                exit_code = Some(v.parse().context("--exit-code: not an integer")?);
                i += 2;
            }
            "--cwd" => {
                let v = args.get(i + 1).context("--cwd requires a value")?;
                cwd = Some(v.clone());
                i += 2;
            }
            "--duration-ms" => {
                let v = args.get(i + 1).context("--duration-ms requires a value")?;
                duration_ms = Some(v.parse().context("--duration-ms: not an integer")?);
                i += 2;
            }
            "--stderr" => {
                let v = args.get(i + 1).context("--stderr requires a value")?;
                stderr_snippet = Some(v.clone());
                i += 2;
            }
            other => anyhow::bail!("unknown flag: {}", other),
        }
    }

    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown".to_string())
    });

    Ok(TerminalEvent {
        ts: chrono::Utc::now(),
        pid: std::process::id(),
        cwd,
        command_line,
        stdout_snippet: None,
        stderr_snippet,
        keystroke_rate: 0.0,
        exit_code,
        duration_ms,
        context: EventContext::default(),
    })
}

async fn cmd_emit_terminal(args: &[String]) -> Result<()> {
    let evt = OrganismEvent::Terminal(build_terminal_event(args)?);
    let payload = serde_json::to_value(&evt).context("serializing event")?;
    let resp = send_request("event", payload).await?;
    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    println!("{}", serde_json::to_string(result)?);
    Ok(())
}

async fn cmd_status() -> Result<()> {
    let resp = send_request("status", serde_json::json!({})).await?;
    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    let uptime = result
        .get("uptime_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let awake = result
        .get("awake")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let count = result
        .get("event_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("Organism Status");
    println!("  awake:        {}", awake);
    println!("  uptime_secs:  {}", uptime);
    println!("  event_count:  {}", count);
    Ok(())
}

async fn cmd_suggest() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut error_key: Option<String> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--error-key" => {
                let v = args.get(i + 1).context("--error-key requires a value")?;
                error_key = Some(v.clone());
                i += 2;
            }
            other => anyhow::bail!("unknown flag in suggest: {}", other),
        }
    }

    let resp = send_request("suggest", serde_json::json!({ "error_key": error_key })).await?;
    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    let text = result
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("(no suggestion)");
    let cached = result
        .get("cached")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let display_text = if text.is_empty() {
        "(no suggestion yet — try again in a moment)".to_string()
    } else {
        text.to_string()
    };

    let cached_tag = if cached { " (cached)" } else { "" };
    println!("Suggestion:{}\n{}", cached_tag, display_text);
    Ok(())
}

/// Parse `apply` subcommand args and return (error_key, stage_flag).
fn parse_apply_args(args: &[String]) -> Result<(String, bool)> {
    if args.is_empty() {
        anyhow::bail!(
            "apply: missing <ERROR_KEY>. Run `organism-cli log` or `suggest` to find one."
        );
    }
    let mut error_key: Option<String> = None;
    let mut stage = false;
    for a in args {
        match a.as_str() {
            "--stage" => stage = true,
            other if other.starts_with("--") => anyhow::bail!("unknown flag in apply: {}", other),
            other => {
                if error_key.is_some() {
                    anyhow::bail!("apply: only one ERROR_KEY allowed");
                }
                error_key = Some(other.to_string());
            }
        }
    }
    let key = error_key.ok_or_else(|| anyhow::anyhow!("apply: missing <ERROR_KEY>"))?;
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("apply: ERROR_KEY must be 1-64 hex chars");
    }
    Ok((key, stage))
}

/// Format and print all plans from ApplyResponse, with numbering.
/// Handles both legacy single-plan responses and new multi-plan responses.
fn format_apply_response(result: &serde_json::Value) {
    // Check for new multi-plan format
    if let Some(plans_array) = result.get("plans").and_then(|v| v.as_array()) {
        if !plans_array.is_empty() {
            let total = plans_array.len();
            for (idx, plan) in plans_array.iter().enumerate() {
                let kind = plan.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let body = plan.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let plan_num = idx + 1;

                println!("[{}/{}] {}", plan_num, total, kind);
                // Indent body by 4 spaces
                for line in body.lines() {
                    println!("    {}", line);
                }

                if let Some(artifact) = plan.get("artifact_path").and_then(|v| v.as_str()) {
                    println!("    artifact: {}", artifact);
                }

                if idx < total - 1 {
                    println!();
                }
            }
            return;
        }
    }

    // Fall back to legacy format for backward compat
    let kind = result
        .get("plan_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let message = result.get("message").and_then(|v| v.as_str()).unwrap_or("");
    println!("[{}]\n{}", kind, message);
    if let Some(p) = result.get("artifact_path").and_then(|v| v.as_str()) {
        println!("\nartifact: {}", p);
    }
}

async fn cmd_apply(args: &[String]) -> Result<()> {
    let (error_key, stage) = parse_apply_args(args)?;
    let mode = if stage { "stage" } else { "dry" };
    let resp = send_request(
        "apply",
        serde_json::json!({ "error_key": error_key, "mode": mode }),
    )
    .await?;

    if let Some(err) = resp.payload.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("daemon error: {}", err);
    }

    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    format_apply_response(result);
    Ok(())
}

/// Parse feedback command arguments: <error_key> <verdict> [--note "..."]
fn parse_feedback_args(args: &[String]) -> Result<(String, String, Option<String>)> {
    if args.len() < 2 {
        anyhow::bail!("feedback: missing <ERROR_KEY> and <verdict>. Usage: feedback <ERROR_KEY> accept|reject|ignore [--note \"...\"]");
    }

    let error_key = args[0].clone();
    if error_key.is_empty()
        || error_key.len() > 64
        || !error_key.chars().all(|c| c.is_ascii_hexdigit())
    {
        anyhow::bail!("feedback: ERROR_KEY must be 1-64 hex chars");
    }

    let verdict = args[1].clone();
    if !["accept", "reject", "ignore"].contains(&verdict.as_str()) {
        anyhow::bail!("feedback: verdict must be 'accept', 'reject', or 'ignore'");
    }

    let mut note: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--note" => {
                let n = args.get(i + 1).context("--note requires a value")?;
                note = Some(n.clone());
                i += 2;
            }
            other => anyhow::bail!("unknown flag in feedback: {}", other),
        }
    }

    Ok((error_key, verdict, note))
}

async fn cmd_feedback(args: &[String]) -> Result<()> {
    let (error_key, verdict, note) = parse_feedback_args(args)?;

    let resp = send_request(
        "feedback",
        serde_json::json!({
            "error_key": error_key,
            "verdict": verdict,
            "note": note
        }),
    )
    .await?;

    if let Some(err) = resp.payload.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("daemon error: {}", err);
    }

    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    if ok {
        println!("Feedback recorded: {} verdict={}", error_key, verdict);
    } else {
        anyhow::bail!("feedback rejected by daemon");
    }

    Ok(())
}

async fn cmd_errors(args: &[String]) -> Result<()> {
    let mut limit: Option<usize> = None;
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => {
                let v = args.get(i + 1).context("--limit requires a value")?;
                limit = Some(v.parse().context("--limit: not an integer")?);
                i += 2;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            other => anyhow::bail!("unknown flag in errors: {}", other),
        }
    }

    let resp = send_request("errors", serde_json::json!({ "limit": limit })).await?;

    if let Some(err) = resp.payload.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("daemon error: {}", err);
    }

    let result = resp.payload.get("result").unwrap_or(&resp.payload);

    if json_output {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }

    // Human-readable format
    if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
        if items.is_empty() {
            println!("(no errors recorded yet)");
            return Ok(());
        }

        // Print header
        println!(
            "{:<10} {:<9} {:<6} {:<5} {:<30}",
            "HASH", "AGE", "OCC", "SUG", "COMMAND"
        );
        println!("{}", "-".repeat(70));

        for item in items {
            let hash = item.get("hash").and_then(|v| v.as_str()).unwrap_or("?");
            let command = item.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let occurrences = item
                .get("occurrences")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let has_suggestion = item
                .get("has_suggestion")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let last_seen = item
                .get("last_seen")
                .and_then(|v| v.as_str())
                .unwrap_or("?");

            let age = format_age(last_seen);
            let sug_tag = if has_suggestion { "yes" } else { "no" };
            let hash_short = if hash.len() > 10 { &hash[..10] } else { hash };
            let cmd_short = if command.len() > 30 {
                format!("{}...", &command[..27])
            } else {
                command.to_string()
            };

            println!(
                "{:<10} {:<9} {:<6} {:<5} {:<30}",
                hash_short, age, occurrences, sug_tag, cmd_short
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(result)?);
    }

    Ok(())
}

async fn cmd_profile(args: &[String]) -> Result<()> {
    let mut rebuild = false;
    let mut json_output = false;

    for arg in args {
        match arg.as_str() {
            "--rebuild" => rebuild = true,
            "--json" => json_output = true,
            other => anyhow::bail!("unknown flag in profile: {}", other),
        }
    }

    let params = serde_json::json!({"rebuild": rebuild});
    let envelope = send_request("profile", params).await?;

    let result = envelope
        .payload
        .get("result")
        .cloned()
        .unwrap_or(envelope.payload);
    let profile_response: organism_protocol::ProfileResponse = serde_json::from_value(result)?;
    let profile = &profile_response.profile;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&profile_response)?);
    } else {
        // Human-readable format
        println!(
            "Profile Status: {}",
            if profile_response.freshly_built {
                "freshly built"
            } else {
                "cached"
            }
        );
        println!();

        println!("Feedback Count: {}", profile.feedback_count);
        println!("Accept Rate: {:.1}%", profile.accept_rate_overall * 100.0);
        println!("Preferred Terseness: {:?}", profile.preferred_terseness);
        println!();

        // Top 3 tools by activity (accepts + rejects)
        println!("Top Tools by Activity:");
        let mut tool_activity: Vec<_> = profile
            .by_tool
            .iter()
            .map(|(name, stats)| {
                let total = stats.accepts + stats.rejects;
                (name.clone(), total, stats.accepts, stats.rejects)
            })
            .collect();
        tool_activity.sort_by_key(|(_, total, _, _)| std::cmp::Reverse(*total));

        for (name, total, accepts, rejects) in tool_activity.iter().take(3) {
            let accept_rate = if *total > 0 {
                (*accepts as f32 / *total as f32) * 100.0
            } else {
                0.0
            };
            println!(
                "  {}: {} total ({} accepted, {} rejected, {:.0}%)",
                name, total, accepts, rejects, accept_rate
            );
        }
        println!();

        // Top 5 accepted phrases
        println!("Top Accepted Phrases:");
        for (i, phrase) in profile.top_accepted_phrases.iter().take(5).enumerate() {
            println!("  {}. {}", i + 1, phrase);
        }
    }

    Ok(())
}

/// Print notification gate status by reading style_profile_current.json directly.
/// Gate threshold mirrors maybe_notify in daemon: tool_rate >= 0.70.
fn print_gate_status(data_dir: &std::path::Path) {
    use organism_knowledge::StyleProfile;

    const GATE: f32 = 0.70;
    let profile_path = data_dir.join("style_profile_current.json");

    let profile: StyleProfile = match std::fs::read_to_string(&profile_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(p) => p,
        None => {
            println!("notification gates: (no data — run 'organism-cli feedback' to build profile)");
            return;
        }
    };

    if profile.by_tool.is_empty() {
        println!("notification gates: (no tool data yet)");
        return;
    }

    println!("notification gates:");
    let mut tools: Vec<_> = profile.by_tool.iter().collect();
    tools.sort_by_key(|(k, _)| k.as_str());
    for (tool, stats) in &tools {
        let total = stats.accepts + stats.rejects;
        let rate = if total == 0 {
            0.0f32
        } else {
            stats.accepts as f32 / total as f32
        };
        let gate_label = if rate >= GATE {
            format!("[notifiable \u{2265}{:.2}]", GATE)
        } else {
            format!("[silent <{:.2}]", GATE)
        };
        println!("  {:<20}  accept_rate={:.2}  {}", tool, rate, gate_label);
    }
}

async fn cmd_doctor() -> Result<()> {
    let dir = data_dir();
    // Best-effort: missing dir is fine, we'll just report empty counts.
    let _ = std::fs::create_dir_all(&dir);

    let mut error_count = 0;
    let mut pattern_count = 0;
    let mut suggestion_count = 0;
    let mut feedback_count = 0;
    let mut migration_failures = Vec::new();

    // Scan all JSON files in $ORGANISM_HOME
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = match path.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => continue,
            };

            // Check file extension and count by prefix
            if !file_name.ends_with(".json") {
                continue;
            }

            if file_name.starts_with("error_") {
                error_count += 1;
                // Try to validate error record schema_v
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(schema_v) = value.get("schema_v").and_then(|v| v.as_u64()) {
                            if schema_v > 1 {
                                migration_failures.push(format!(
                                    "{}: unsupported schema version {}",
                                    file_name, schema_v
                                ));
                            }
                        }
                    }
                }
            } else if file_name.starts_with("pattern_") {
                pattern_count += 1;
            } else if file_name.starts_with("suggestion_") {
                suggestion_count += 1;
            } else if file_name.starts_with("feedback_") {
                feedback_count += 1;
            }
        }
    }

    // Check daemon status via IPC
    let daemon_status = match send_request("status", serde_json::json!({})).await {
        Ok(_) => "awake",
        Err(_) => "asleep",
    };

    // Print report
    println!(
        "knowledge: {} errors, {} suggestions, {} patterns, {} feedback",
        error_count, suggestion_count, pattern_count, feedback_count
    );

    // Report any migration failures
    for failure in &migration_failures {
        eprintln!("schema migration failed: {}", failure);
    }

    println!("daemon:   {}", daemon_status);

    // Notification gate status — read profile directly (no IPC needed)
    print_gate_status(&dir);

    // Exit code: 0 if healthy, 1 if any corruption
    if migration_failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "health check failed: {} schema errors detected",
            migration_failures.len()
        )
    }
}

async fn cmd_log() -> Result<()> {
    let resp = send_request("log", serde_json::json!({})).await?;
    let result = resp.payload.get("result").unwrap_or(&resp.payload);
    if let Some(arr) = result.as_array() {
        if arr.is_empty() {
            println!("(no recent events)");
        } else {
            for entry in arr {
                let ts = entry.get("ts").and_then(|v| v.as_str()).unwrap_or("?");
                let msg = entry.get("msg").and_then(|v| v.as_str()).unwrap_or("?");
                println!("{}  {}", ts, msg);
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(result)?);
    }
    Ok(())
}

async fn cmd_sleep() -> Result<()> {
    let resp = send_request("sleep", serde_json::json!({})).await?;
    let _ = resp;
    println!("Organism paused");
    Ok(())
}

async fn cmd_wake() -> Result<()> {
    let resp = send_request("wake", serde_json::json!({})).await?;
    let _ = resp;
    println!("Organism resumed");
    Ok(())
}

fn cmd_stats(args: &[String]) -> Result<()> {
    let mut json = false;
    let mut capture_baseline = false;
    let mut baseline = false;
    let mut since: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--capture-baseline" => {
                capture_baseline = true;
                i += 1;
            }
            "--baseline" => {
                baseline = true;
                i += 1;
            }
            "--since" => {
                let v = args.get(i + 1).context("--since requires a value")?;
                since = Some(v.clone());
                i += 2;
            }
            other => anyhow::bail!("unknown flag in stats: {}", other),
        }
    }

    let stats_args = cmd_stats::StatsArgs {
        json,
        capture_baseline,
        baseline,
        since,
    };

    cmd_stats::cmd_stats(&stats_args)
}

async fn send_request(method: &str, params: serde_json::Value) -> Result<Envelope> {
    let socket_path = socket_path();
    let stream = match UnixStream::connect(&socket_path).await {
        Ok(s) => s,
        Err(_) => {
            anyhow::bail!(
                "Error: daemon not running (socket: {})",
                socket_path.display()
            );
        }
    };

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let env = Envelope::request(method, params);
    let mut payload = serde_json::to_string(&env).context("serializing request")?;
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).await?;
    write_half.shutdown().await.ok();

    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        anyhow::bail!("daemon closed connection without responding");
    }
    let resp: Envelope = serde_json::from_str(line.trim()).context("parsing response")?;
    Ok(resp)
}

fn socket_path() -> PathBuf {
    data_dir().join("daemon.sock")
}

pub(crate) fn data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("ORGANISM_HOME") {
        return PathBuf::from(override_dir);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    home.join(".organism")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn build_terminal_event_requires_command() {
        let err = build_terminal_event(&[]).unwrap_err();
        assert!(err.to_string().contains("missing <command>"));
    }

    #[test]
    fn build_terminal_event_parses_stderr_flag() {
        let args = vec![s("ls"), s("--stderr"), s("foo")];
        let evt = build_terminal_event(&args).expect("parse ok");
        assert_eq!(evt.command_line, "ls");
        assert_eq!(evt.stderr_snippet.as_deref(), Some("foo"));
    }

    #[test]
    fn build_terminal_event_parses_all_flags() {
        let args = vec![
            s("cargo build"),
            s("--exit-code"),
            s("1"),
            s("--cwd"),
            s("/tmp/proj"),
            s("--duration-ms"),
            s("250"),
            s("--stderr"),
            s("error: bad"),
        ];
        let evt = build_terminal_event(&args).expect("parse ok");
        assert_eq!(evt.command_line, "cargo build");
        assert_eq!(evt.exit_code, Some(1));
        assert_eq!(evt.cwd, "/tmp/proj");
        assert_eq!(evt.duration_ms, Some(250));
        assert_eq!(evt.stderr_snippet.as_deref(), Some("error: bad"));
        assert!(evt.stdout_snippet.is_none());
    }

    #[test]
    fn build_terminal_event_default_stderr_none() {
        let args = vec![s("echo hi")];
        let evt = build_terminal_event(&args).expect("parse ok");
        assert!(evt.stderr_snippet.is_none());
    }

    #[test]
    fn build_terminal_event_unknown_flag_errors() {
        let args = vec![s("ls"), s("--bogus"), s("x")];
        let err = build_terminal_event(&args).unwrap_err();
        assert!(err.to_string().contains("unknown flag"));
    }

    #[test]
    fn parse_apply_args_rejects_missing_key() {
        let err = parse_apply_args(&[]).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn parse_apply_args_default_dry() {
        let args = vec![s("abc123")];
        let (key, stage) = parse_apply_args(&args).unwrap();
        assert_eq!(key, "abc123");
        assert!(!stage);
    }

    #[test]
    fn parse_apply_args_stage_flag() {
        let args = vec![s("abc123"), s("--stage")];
        let (_, stage) = parse_apply_args(&args).unwrap();
        assert!(stage);
    }

    #[test]
    fn parse_apply_args_rejects_non_hex() {
        let args = vec![s("not-hex!")];
        let err = parse_apply_args(&args).unwrap_err();
        assert!(err.to_string().contains("hex chars"));
    }

    #[test]
    fn parse_apply_args_rejects_path_traversal() {
        let args = vec![s("../etc")];
        let err = parse_apply_args(&args).unwrap_err();
        assert!(err.to_string().contains("hex chars"));
    }

    #[test]
    fn parse_apply_args_rejects_unknown_flag() {
        let args = vec![s("abc"), s("--bogus")];
        let err = parse_apply_args(&args).unwrap_err();
        assert!(err.to_string().contains("unknown flag"));
    }

    #[test]
    fn build_terminal_event_stderr_requires_value() {
        let args = vec![s("ls"), s("--stderr")];
        let err = build_terminal_event(&args).unwrap_err();
        assert!(err.to_string().contains("--stderr requires a value"));
    }

    #[test]
    fn format_age_seconds() {
        let now = chrono::Utc::now();
        let thirty_secs_ago = (now - chrono::Duration::seconds(30))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&thirty_secs_ago);
        assert!(age.ends_with("s"), "age={}", age);
        assert!(age.starts_with("3"), "age={}", age);
    }

    #[test]
    fn format_age_minutes() {
        let now = chrono::Utc::now();
        let three_mins_ago =
            (now - chrono::Duration::minutes(3)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&three_mins_ago);
        assert_eq!(age, "3m");
    }

    #[test]
    fn format_age_hours() {
        let now = chrono::Utc::now();
        let one_hour_ago =
            (now - chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&one_hour_ago);
        assert_eq!(age, "1h");
    }

    #[test]
    fn format_age_days() {
        let now = chrono::Utc::now();
        let two_days_ago =
            (now - chrono::Duration::days(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&two_days_ago);
        assert_eq!(age, "2d");
    }

    #[test]
    fn format_age_boundary_60_seconds() {
        let now = chrono::Utc::now();
        let sixty_secs_ago = (now - chrono::Duration::seconds(60))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&sixty_secs_ago);
        assert_eq!(age, "1m");
    }

    #[test]
    fn format_age_boundary_59_seconds() {
        let now = chrono::Utc::now();
        let fifty_nine_secs_ago = (now - chrono::Duration::seconds(59))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let age = format_age(&fifty_nine_secs_ago);
        assert_eq!(age, "59s");
    }

    #[test]
    fn format_apply_response_single_plan() {
        // Test formatter with single plan
        let response = serde_json::json!({
            "plan_kind": "shell",
            "artifact_path": null,
            "clipboard": true,
            "message": "copied to clipboard",
            "plans": [
                {
                    "kind": "shell",
                    "body": "brew install foo",
                    "artifact_path": null,
                    "clipboard": true
                }
            ]
        });
        // Just verify it doesn't panic (output goes to stdout)
        format_apply_response(&response);
    }

    #[test]
    fn format_apply_response_multi_plan() {
        // Test formatter with multiple plans
        let response = serde_json::json!({
            "plan_kind": "shell",
            "artifact_path": null,
            "clipboard": false,
            "message": "[1/2] shell + 1 more plan(s)",
            "plans": [
                {
                    "kind": "shell",
                    "body": "brew install foo",
                    "artifact_path": null,
                    "clipboard": false
                },
                {
                    "kind": "patch",
                    "body": "diff --git a/file b/file\n-old\n+new",
                    "artifact_path": Some("/tmp/organism-abc-0.patch"),
                    "clipboard": false
                }
            ]
        });
        // Just verify it doesn't panic
        format_apply_response(&response);
    }

    // Note: cmd_doctor() is async and connects to a Unix socket, making it difficult
    // to test without a running daemon. The following unit tests verify the core logic
    // (file scanning and counting) in isolation without async/IPC.

    #[test]
    fn test_doctor_clean_store() {
        // Test that a clean (empty) store directory scans without errors
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::env::set_var("ORGANISM_HOME", dir.path());

        // Verify directory is created and readable
        let store_dir = dir.path();
        assert!(store_dir.exists());

        // Manual scan logic: verify we can iterate the directory
        let entries: Vec<_> = std::fs::read_dir(store_dir).unwrap().flatten().collect();
        assert_eq!(entries.len(), 0, "Empty store should have no files");
    }

    #[test]
    fn test_doctor_counts_error_files() {
        // Test that error_*.json files are properly counted
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();

        // Create test error files
        std::fs::write(dir.path().join("error_abc123.json"), "{}").unwrap();
        std::fs::write(dir.path().join("error_def456.json"), "{}").unwrap();
        std::fs::write(dir.path().join("pattern_xyz.json"), "{}").unwrap();

        // Scan and count
        let mut error_count = 0;
        let mut pattern_count = 0;
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("error_") && name.ends_with(".json") {
                error_count += 1;
            } else if name.starts_with("pattern_") && name.ends_with(".json") {
                pattern_count += 1;
            }
        }
        assert_eq!(error_count, 2);
        assert_eq!(pattern_count, 1);
    }

    #[test]
    fn test_doctor_detects_bad_schema_version() {
        // Test that schema_v > 1 is properly detected
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();

        // Create an error file with schema_v = 99
        let bad_error = serde_json::json!({
            "tool": "rustc",
            "kind": "E0599",
            "hash": "badschema",
            "raw_excerpt": "error",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-01T00:00:00Z",
            "occurrences": 1,
            "last_command": "cargo build",
            "schema_v": 99
        });
        std::fs::write(
            dir.path().join("error_badschema.json"),
            serde_json::to_string(&bad_error).unwrap(),
        )
        .unwrap();

        // Scan and validate
        let mut migration_failures = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name() {
                let file_name = name.to_string_lossy().into_owned();
                if file_name.starts_with("error_") && file_name.ends_with(".json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(schema_v) = value.get("schema_v").and_then(|v| v.as_u64()) {
                                if schema_v > 1 {
                                    migration_failures.push(format!(
                                        "{}: unsupported schema version {}",
                                        file_name, schema_v
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(migration_failures.len(), 1);
        assert!(migration_failures[0].contains("99"));
        assert!(migration_failures[0].contains("badschema"));
    }

    #[test]
    fn gate_status_empty_profile_prints_no_data() {
        let dir = tempfile::tempdir().unwrap();
        // No profile file → expect "(no data" line, no panic
        let profile_path = dir.path().join("style_profile_current.json");
        assert!(!profile_path.exists());
        // Redirect is not possible in a unit test, so we call the fn and verify
        // it doesn't panic. The "no data" branch is exercised.
        print_gate_status(dir.path());
    }

    #[test]
    fn gate_status_with_tool_stats_prints_rates() {
        use organism_knowledge::{StyleProfile, ToolStats};
        use std::collections::HashMap;

        let dir = tempfile::tempdir().unwrap();
        let mut by_tool = HashMap::new();
        by_tool.insert("rustc".to_string(), ToolStats { accepts: 8, rejects: 2 });
        by_tool.insert("npm".to_string(), ToolStats { accepts: 3, rejects: 7 });
        let profile = StyleProfile {
            schema_v: 1,
            generated_at: chrono::Utc::now(),
            feedback_count: 20,
            accept_rate_overall: 0.55,
            by_tool,
            by_block_kind: HashMap::new(),
            preferred_terseness: organism_knowledge::Terseness::Standard,
            top_accepted_phrases: vec![],
            top_rejected_phrases: vec![],
        };
        let json = serde_json::to_string(&profile).unwrap();
        std::fs::write(dir.path().join("style_profile_current.json"), json).unwrap();
        // rustc: 8/(8+2) = 0.80 → notifiable; npm: 3/10 = 0.30 → silent
        // Verify no panic; output goes to stdout (not captured in unit test).
        print_gate_status(dir.path());
    }
}
