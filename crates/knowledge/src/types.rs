use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Returns the default schema version for new records.
pub fn default_schema_v() -> u32 {
    1
}

/// A fix record: known error → patch solution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixRecord {
    pub id: String,
    /// sha256 of the error snippet that triggers this fix
    pub signature_hash: String,
    /// Unified diff or description of the fix
    pub patch: String,
    /// 0.0 - 1.0 confidence based on success rate
    pub confidence: f64,
    pub applied_count: u32,
    pub last_applied: DateTime<Utc>,
    /// "learned" | "manual" | "imported"
    pub source: String,
}

/// A detected coding pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRecord {
    pub id: String,
    pub trigger: String,
    pub action: String,
    pub frequency: u32,
    pub confidence: f64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub examples: Vec<String>,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

/// A project metadata record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub path: String,
    pub name: String,
    pub detected_stack: Vec<String>,
    pub primary_language: Option<String>,
    pub last_accessed: DateTime<Utc>,
    pub session_count: u32,
}

/// A classified error occurrence record.
/// Signature fields (tool/kind/hash) are inlined to avoid a knowledge → cortex
/// dependency cycle; cortex constructs the values via `cortex::classify`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRecord {
    pub tool: String,
    pub kind: String,
    pub hash: String,
    pub raw_excerpt: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub occurrences: u64,
    pub last_command: String,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

/// An LLM-generated suggestion for an error, cached on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionRecord {
    pub text: String,
    pub ts: DateTime<Utc>,
}

/// User verdict on a suggestion
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Verdict {
    Accepted,
    Rejected,
    Ignored,
}

/// A record of user feedback on a suggestion
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackRecord {
    pub error_hash: String,
    pub suggestion_hash: String,
    pub verdict: Verdict,
    pub note: Option<String>,
    pub ts: DateTime<Utc>,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

/// An accepted suggestion snapshot — immutable copy of text at acceptance time
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptedSuggestion {
    pub suggestion_hash: String,
    pub error_hash: String,
    pub text: String,
    pub ts: DateTime<Utc>,
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
}

impl AcceptedSuggestion {
    pub fn from_feedback(fb: &FeedbackRecord, text: String) -> Self {
        Self {
            suggestion_hash: fb.suggestion_hash.clone(),
            error_hash: fb.error_hash.clone(),
            text,
            ts: fb.ts,
            schema_v: 1,
        }
    }
}

/// Key prefixes for the key-value store
pub mod keys {
    pub const FIX_PREFIX: &str = "fix:";
    pub const PATTERN_PREFIX: &str = "pat:";
    pub const PROJECT_PREFIX: &str = "proj:";
    pub const STATS_PREFIX: &str = "stats:";
    pub const ERROR_PREFIX: &str = "error:";
    pub const SUGGESTION_PREFIX: &str = "suggestion:";
    pub const FEEDBACK_PREFIX: &str = "feedback:";
    pub const ACCEPTED_PREFIX: &str = "accepted:";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_error_write_has_schema_v_1() {
        let rec = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "abc123".to_string(),
            raw_excerpt: "error[E0599]".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_fresh_pattern_write_has_schema_v_1() {
        let rec = PatternRecord {
            id: "pat1".to_string(),
            trigger: "test".to_string(),
            action: "act".to_string(),
            frequency: 5,
            confidence: 0.8,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            examples: vec![],
            schema_v: 1,
        };
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_fresh_feedback_write_has_schema_v_1() {
        let rec = FeedbackRecord {
            error_hash: "hash1".to_string(),
            suggestion_hash: "sugg1".to_string(),
            verdict: Verdict::Accepted,
            note: None,
            ts: Utc::now(),
            schema_v: 1,
        };
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_old_error_json_without_field_deserializes_to_1() {
        let old_json = r#"{
            "tool": "rustc",
            "kind": "E0599",
            "hash": "abc123",
            "raw_excerpt": "error[E0599]",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-01T00:00:00Z",
            "occurrences": 1,
            "last_command": "cargo build"
        }"#;
        let rec: ErrorRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_old_pattern_json_without_field_deserializes_to_1() {
        let old_json = r#"{
            "id": "pat1",
            "trigger": "test",
            "action": "act",
            "frequency": 5,
            "confidence": 0.8,
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-01T00:00:00Z",
            "examples": []
        }"#;
        let rec: PatternRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_old_feedback_json_without_field_deserializes_to_1() {
        let old_json = r#"{
            "error_hash": "hash1",
            "suggestion_hash": "sugg1",
            "verdict": "Accepted",
            "note": null,
            "ts": "2023-01-01T00:00:00Z"
        }"#;
        let rec: FeedbackRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(rec.schema_v, 1);
    }

    #[test]
    fn test_error_explicit_schema_v_preserved() {
        let json = r#"{
            "tool": "rustc",
            "kind": "E0599",
            "hash": "abc123",
            "raw_excerpt": "error[E0599]",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-01T00:00:00Z",
            "occurrences": 1,
            "last_command": "cargo build",
            "schema_v": 99
        }"#;
        let rec: ErrorRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.schema_v, 99);
    }

    #[test]
    fn test_pattern_explicit_schema_v_preserved() {
        let json = r#"{
            "id": "pat1",
            "trigger": "test",
            "action": "act",
            "frequency": 5,
            "confidence": 0.8,
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-01T00:00:00Z",
            "examples": [],
            "schema_v": 42
        }"#;
        let rec: PatternRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.schema_v, 42);
    }

    #[test]
    fn test_feedback_explicit_schema_v_preserved() {
        let json = r#"{
            "error_hash": "hash1",
            "suggestion_hash": "sugg1",
            "verdict": "Accepted",
            "note": null,
            "ts": "2023-01-01T00:00:00Z",
            "schema_v": 7
        }"#;
        let rec: FeedbackRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.schema_v, 7);
    }

    #[test]
    fn test_accepted_suggestion_roundtrip() {
        let acc = AcceptedSuggestion {
            suggestion_hash: "sugg_hash_001".to_string(),
            error_hash: "err_hash_001".to_string(),
            text: "Try adding `derive(Clone)` to the struct definition.".to_string(),
            ts: Utc::now(),
            schema_v: 1,
        };
        let json = serde_json::to_string(&acc).unwrap();
        let roundtrip: AcceptedSuggestion = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, acc);
    }

    #[test]
    fn test_accepted_suggestion_old_json_deserializes_to_1() {
        let old_json = r#"{
            "suggestion_hash": "sugg_hash_001",
            "error_hash": "err_hash_001",
            "text": "Try adding `derive(Clone)` to the struct definition.",
            "ts": "2023-01-01T00:00:00Z"
        }"#;
        let rec: AcceptedSuggestion = serde_json::from_str(old_json).unwrap();
        assert_eq!(rec.schema_v, 1);
    }
}
