use crate::events::OrganismEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Protocol version. Increment when breaking changes are made.
pub const PROTOCOL_VERSION: u8 = 1;

/// Versioned envelope wrapping all messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub id: String,
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn new(msg_type: MessageType, payload: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            msg_type,
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            payload,
        }
    }

    pub fn request(method: &str, params: serde_json::Value) -> Self {
        Self::new(
            MessageType::Request,
            serde_json::json!({ "method": method, "params": params }),
        )
    }

    pub fn ok_response(request_id: &str, result: serde_json::Value) -> Self {
        let mut env = Self::new(
            MessageType::Response,
            serde_json::json!({ "result": result }),
        );
        env.id = request_id.to_string();
        env
    }

    pub fn error_response(request_id: &str, message: &str) -> Self {
        let mut env = Self::new(MessageType::Error, serde_json::json!({ "error": message }));
        env.id = request_id.to_string();
        env
    }

    pub fn event(event: &OrganismEvent) -> anyhow::Result<Self> {
        Ok(Self::new(MessageType::Event, serde_json::to_value(event)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum MessageType {
    Request,
    Response,
    Error,
    Event,
    Heartbeat,
}

/// Status response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub uptime_s: u64,
    pub trust_level: String,
    pub active_sensors: Vec<String>,
    pub events_processed: u64,
}

/// Suggest request payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestRequest {
    pub error_key: Option<String>,
    /// When true, regenerate suggestion even if cached
    #[serde(default)]
    pub force: bool,
}

/// Suggest response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestResponse {
    pub text: String,
    pub cached: bool,
}

/// Apply request payload — turn a cached suggestion into an actionable artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub error_key: String,
    pub mode: ApplyMode,
}

/// Apply execution mode. `Dry` is preview-only and the safe default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    /// Print the plan to stdout. Side-effect free.
    Dry,
    /// Materialize the plan: write patch to a tempfile, or copy shell cmd to clipboard.
    Stage,
}

/// Apply response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResponse {
    /// "patch" | "shell" | "note"
    pub plan_kind: String,
    /// Path to a staged patch file, if any.
    pub artifact_path: Option<String>,
    /// True when a shell command was successfully copied to the clipboard.
    pub clipboard: bool,
    /// Human-readable summary for the CLI to print.
    pub message: String,
}

/// Errors listing request payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorsRequest {
    pub limit: Option<usize>,
}

/// Wire format for a single error summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSummaryWire {
    pub hash: String,
    pub command: String,
    pub occurrences: u32,
    pub last_seen: String, // RFC3339 format
    pub has_suggestion: bool,
}

/// Errors listing response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorsResponse {
    pub items: Vec<ErrorSummaryWire>,
}

/// Feedback request: user verdict on a suggestion
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackRequest {
    pub error_key: String,
    pub verdict: String,
    pub note: Option<String>,
}

/// Feedback response: acknowledgement from daemon
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackResponse {
    pub ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug + PartialEq,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value);
    }

    #[test]
    fn apply_mode_dry_roundtrip() {
        roundtrip(&ApplyMode::Dry);
    }

    #[test]
    fn apply_mode_stage_roundtrip() {
        roundtrip(&ApplyMode::Stage);
    }

    #[test]
    fn apply_request_roundtrip() {
        let json = serde_json::to_string(&ApplyRequest {
            error_key: "abc123".into(),
            mode: ApplyMode::Stage,
        })
        .unwrap();
        let back: ApplyRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.error_key, "abc123");
        assert_eq!(back.mode, ApplyMode::Stage);
    }

    #[test]
    fn apply_response_roundtrip() {
        let resp = ApplyResponse {
            plan_kind: "patch".into(),
            artifact_path: Some("/tmp/x.patch".into()),
            clipboard: false,
            message: "ok".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApplyResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.plan_kind, resp.plan_kind);
        assert_eq!(back.artifact_path, resp.artifact_path);
        assert_eq!(back.clipboard, resp.clipboard);
        assert_eq!(back.message, resp.message);
    }

    #[test]
    fn errors_response_empty_roundtrip() {
        let resp = ErrorsResponse { items: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ErrorsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.items.len(), 0);
    }

    #[test]
    fn errors_response_with_items_roundtrip() {
        let resp = ErrorsResponse {
            items: vec![
                ErrorSummaryWire {
                    hash: "deadbeef".into(),
                    command: "cargo build --workspace".into(),
                    occurrences: 4,
                    last_seen: "2026-04-30T10:30:00Z".into(),
                    has_suggestion: true,
                },
                ErrorSummaryWire {
                    hash: "cafef00d".into(),
                    command: "pnpm test".into(),
                    occurrences: 1,
                    last_seen: "2026-04-30T09:15:00Z".into(),
                    has_suggestion: false,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ErrorsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.items.len(), 2);
        assert_eq!(back.items[0].hash, "deadbeef");
        assert_eq!(back.items[1].hash, "cafef00d");
        assert!(back.items[0].has_suggestion);
        assert!(!back.items[1].has_suggestion);
    }

    #[test]
    fn test_suggest_request_default_force() {
        let req = SuggestRequest {
            error_key: Some("hash123".to_string()),
            force: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SuggestRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.error_key, Some("hash123".to_string()));
        assert!(!back.force);
    }

    #[test]
    fn test_suggest_request_force_true() {
        let req = SuggestRequest {
            error_key: Some("hash456".to_string()),
            force: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SuggestRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.error_key, Some("hash456".to_string()));
        assert!(back.force);
    }

    #[test]
    fn test_suggest_request_backwards_compat() {
        // Old JSON without force field should default to false
        let json = r#"{"error_key":"hash789"}"#;
        let req: SuggestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.error_key, Some("hash789".to_string()));
        assert!(!req.force);
    }

    #[test]
    fn test_feedback_request_accept_roundtrip() {
        roundtrip(&FeedbackRequest {
            error_key: "test_error".to_string(),
            verdict: "accept".to_string(),
            note: Some("Good fix".to_string()),
        });
    }

    #[test]
    fn test_feedback_request_reject_roundtrip() {
        roundtrip(&FeedbackRequest {
            error_key: "test_error".to_string(),
            verdict: "reject".to_string(),
            note: None,
        });
    }

    #[test]
    fn test_feedback_response_roundtrip() {
        roundtrip(&FeedbackResponse { ok: true });
    }
}
