//! Organism CLI - talks to the running daemon over a Unix socket.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing_subscriber::EnvFilter;

use organism_protocol::{Envelope, EventContext, OrganismEvent, TerminalEvent};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let result = match cmd {
        "status" => cmd_status().await,
        "suggest" => cmd_suggest().await,
        "log" => cmd_log().await,
        "sleep" => cmd_sleep().await,
        "wake" => cmd_wake().await,
        "emit-terminal" => cmd_emit_terminal(&args[2..]).await,
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
    println!("  log       Show recent daemon activity");
    println!("  sleep     Pause all daemon activity");
    println!("  wake      Resume daemon activity");
    println!("  emit-terminal <cmd> [--exit-code N] [--cwd PATH] [--duration-ms M] [--stderr STR]");
    println!("            Inject a terminal event into the daemon (used by shell hook)");
    println!("  help      Show this help");
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

fn data_dir() -> PathBuf {
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
    fn build_terminal_event_stderr_requires_value() {
        let args = vec![s("ls"), s("--stderr")];
        let err = build_terminal_event(&args).unwrap_err();
        assert!(err.to_string().contains("--stderr requires a value"));
    }
}
