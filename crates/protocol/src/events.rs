use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Context attached to every event for project identification
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventContext {
    pub project_id: Option<String>,
    pub detected_stack: Option<Vec<String>>,
    pub last_error_signature: Option<String>,
}

/// A command run in a terminal session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalEvent {
    pub ts: DateTime<Utc>,
    pub pid: u32,
    pub cwd: String,
    pub command_line: String,
    pub stdout_snippet: Option<String>,
    pub stderr_snippet: Option<String>,
    /// Keystrokes per minute (0 if not measured)
    pub keystroke_rate: f32,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub context: EventContext,
}

/// A file system change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    pub ts: DateTime<Utc>,
    pub path: String,
    pub event_type: FileEventType,
    pub size_bytes: u64,
    pub context: EventContext,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FileEventType {
    Create,
    Modify,
    Delete,
    Rename,
}

/// A git repository event (commit, branch switch, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitEvent {
    pub ts: DateTime<Utc>,
    pub repo_path: String,
    pub branch: String,
    pub head_sha: String,
    pub commit_msg: Option<String>,
    pub author: Option<String>,
    pub context: EventContext,
}

/// A process lifecycle event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEvent {
    pub ts: DateTime<Utc>,
    pub pid: u32,
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub cpu_ms: u64,
    pub mem_kb: u64,
    pub context: EventContext,
}

/// Union of all event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrganismEvent {
    Terminal(TerminalEvent),
    File(FileEvent),
    Git(GitEvent),
    Process(ProcessEvent),
}

impl OrganismEvent {
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::Terminal(e) => e.ts,
            Self::File(e) => e.ts,
            Self::Git(e) => e.ts,
            Self::Process(e) => e.ts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_event_with_exit_code_roundtrip() {
        let evt = TerminalEvent {
            ts: Utc::now(),
            pid: 4242,
            cwd: "/tmp/proj".to_string(),
            command_line: "false".to_string(),
            stdout_snippet: None,
            stderr_snippet: None,
            keystroke_rate: 0.0,
            exit_code: Some(1),
            duration_ms: Some(120),
            context: EventContext::default(),
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: TerminalEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.exit_code, Some(1));
        assert_eq!(back.duration_ms, Some(120));
        assert_eq!(back.command_line, "false");
        assert_eq!(back.pid, 4242);
    }

    #[test]
    fn test_terminal_event_back_compat() {
        // Old envelope missing exit_code and duration_ms must still parse.
        let json = r#"{
            "ts": "2026-04-29T00:00:00Z",
            "pid": 1,
            "cwd": "/",
            "command_line": "ls",
            "stdout_snippet": null,
            "stderr_snippet": null,
            "keystroke_rate": 0.0,
            "context": {}
        }"#;
        let evt: TerminalEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.exit_code, None);
        assert_eq!(evt.duration_ms, None);
    }
}
