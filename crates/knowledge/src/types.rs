use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}
