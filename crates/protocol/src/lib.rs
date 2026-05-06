pub mod events;
pub mod messages;

pub use events::*;
pub use messages::{
    ApplyMode, ApplyRequest, ApplyResponse, Envelope, ErrorSummaryWire, ErrorsRequest,
    ErrorsResponse, FeedbackRequest, FeedbackResponse, MessageType, PlanItemWire, ProfileRequest,
    ProfileResponse, StatusResponse, SuggestRequest, SuggestResponse,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::PROTOCOL_VERSION;

    #[test]
    fn test_envelope_request_roundtrip() {
        let env = Envelope::request("status.get", serde_json::json!({}));
        let serialized = serde_json::to_string(&env).unwrap();
        let deserialized: Envelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.v, PROTOCOL_VERSION);
        assert_eq!(deserialized.msg_type, MessageType::Request);
    }

    #[test]
    fn test_terminal_event_serializes() {
        use chrono::Utc;
        let event = OrganismEvent::Terminal(TerminalEvent {
            ts: Utc::now(),
            pid: 1234,
            cwd: "/home/user/projects".to_string(),
            command_line: "cargo build".to_string(),
            stdout_snippet: None,
            stderr_snippet: None,
            keystroke_rate: 0.0,
            exit_code: None,
            duration_ms: None,
            context: EventContext::default(),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("terminal"));
        assert!(json.contains("cargo build"));
    }

    #[test]
    fn test_file_event_roundtrip() {
        use chrono::Utc;
        let event = FileEvent {
            ts: Utc::now(),
            path: "/home/user/src/main.rs".to_string(),
            event_type: FileEventType::Modify,
            size_bytes: 1024,
            context: EventContext::default(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/home/user/src/main.rs");
        assert_eq!(back.event_type, FileEventType::Modify);
    }

    #[test]
    fn test_envelope_event_from_organism_event() {
        use chrono::Utc;
        let event = OrganismEvent::Git(GitEvent {
            ts: Utc::now(),
            repo_path: "/home/user/projects/app".to_string(),
            branch: "main".to_string(),
            head_sha: "abc123".to_string(),
            commit_msg: Some("feat: add feature".to_string()),
            author: Some("dev".to_string()),
            context: EventContext::default(),
        });
        let envelope = Envelope::event(&event).unwrap();
        assert_eq!(envelope.msg_type, MessageType::Event);
    }

    #[test]
    fn test_ok_response_preserves_request_id() {
        let env = Envelope::ok_response("req-123", serde_json::json!({"status": "ok"}));
        assert_eq!(env.id, "req-123");
        assert_eq!(env.msg_type, MessageType::Response);
    }

    #[test]
    fn test_suggest_request_roundtrip() {
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
    fn test_suggest_response_roundtrip() {
        let resp = SuggestResponse {
            text: "Try running cargo fix".to_string(),
            cached: false,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: SuggestResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, "Try running cargo fix");
        assert!(!back.cached);
    }

    #[test]
    fn test_suggest_response_cached() {
        let resp = SuggestResponse {
            text: "Solution from cache".to_string(),
            cached: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"cached\":true"));
        let back: SuggestResponse = serde_json::from_str(&json).unwrap();
        assert!(back.cached);
    }
}
